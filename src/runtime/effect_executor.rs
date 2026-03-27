use std::io::Write;
use std::sync::Arc;

use anyhow::Result;
use tokio::{runtime::Runtime, sync::mpsc};

use crate::{
    Tui,
    agent::AgentContext,
    app::{self, App, Effect, StreamEvent, query},
    config::AppConfig,
    features::planning::run_planning_workflow,
    features::planning::sanitize_planning_agents,
    llm::{LlmService, WriteApprovalController, history_from_rig, history_into_rig},
    model_registry::{self, ModelProvider},
    stats::StatsStore,
    subagents::SubagentManager,
    ui,
};

use super::{clipboard::osc52_copy_sequence, reply_driver::ReplyDriver};

pub(crate) struct EffectExecutor<'a> {
    pub(crate) runtime: &'a Runtime,
    pub(crate) terminal: &'a mut Tui,
    pub(crate) reply_driver: &'a mut ReplyDriver,
    pub(crate) llm: &'a mut LlmService,
    pub(crate) config: &'a mut AppConfig,
    pub(crate) app: &'a mut App,
    pub(crate) stats: &'a StatsStore,
    pub(crate) stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
    pub(crate) subagents: &'a SubagentManager,
}

impl EffectExecutor<'_> {
    pub(crate) fn run(&mut self, effect: Effect) -> Result<()> {
        match effect {
            Effect::PromptModel {
                reply_id,
                prompt,
                history,
                history_model_name,
            } => {
                self.refresh_codex_auth_if_needed()?;
                let model_names = vec![
                    self.config.model.model_name.clone(),
                    self.config.safety.model_name.clone(),
                ];
                self.ensure_codex_auth_for_models(model_names.iter().map(String::as_str))?;
                self.reply_driver.cancel_active_reply(self.llm);
                let llm = self.llm.clone();
                let stats_hook = self
                    .stats
                    .hook_for_model(query::model_name(self.app.state()).to_string());
                let stream_tx = self.stream_tx.clone();
                let task = self.runtime.spawn(async move {
                    llm.stream_prompt(
                        reply_id,
                        prompt,
                        history,
                        history_model_name,
                        stats_hook,
                        stream_tx,
                    )
                    .await;
                });
                self.reply_driver.spawn_task(reply_id, task);
                Ok(())
            }
            Effect::CompactHistory => {
                self.refresh_codex_auth_if_needed()?;
                let model_names = vec![
                    self.config.model.model_name.clone(),
                    self.config.safety.model_name.clone(),
                ];
                self.ensure_codex_auth_for_models(model_names.iter().map(String::as_str))?;
                self.reply_driver.cancel_active_reply(self.llm);
                let llm = self.llm.clone();
                let history = query::session_history(self.app.state()).to_vec();
                let history_model_name =
                    query::last_history_model_name(self.app.state()).map(str::to_string);
                let stream_tx = self.stream_tx.clone();
                let reply_id = ReplyDriver::require_active_reply_id(self.app)?;
                let task = self.runtime.spawn(async move {
                    let event = match llm
                        .compact_history_for_session(history, history_model_name)
                        .await
                    {
                        Ok(result) => StreamEvent::CompactionFinished {
                            history: result.history,
                            model_name: result.model_name,
                        },
                        Err(error) => StreamEvent::Failed(error.to_string()),
                    };
                    let _ = stream_tx.send((reply_id, event));
                });
                self.reply_driver.spawn_task(reply_id, task);
                Ok(())
            }
            Effect::ShowStats => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.stats.report()?.render(),
                );
                Ok(())
            }
            Effect::OpenModelPicker => {
                app::ops::picker::open_model_picker(self.app.state_mut());
                Ok(())
            }
            Effect::LoginCodex => self.login_codex(),
            Effect::LogoutCodex => self.logout_codex(),
            Effect::RotateSession => {
                self.runtime
                    .block_on(self.subagents.cancel_all_running(false));
                self.stats.rotate_session()?;
                self.llm.reset_write_approvals();
                Ok(())
            }
            Effect::SetModelSelection { model_name } => {
                self.ensure_codex_auth_for_model(&model_name)?;
                let reasoning = app::compatible_reasoning_setting(
                    &model_name,
                    query::reasoning(self.app.state()),
                );
                let planning_agents =
                    sanitize_planning_agents(&model_name, query::planning_agents(self.app.state()));
                let updated_config = AppConfig::set_default_model_selection_with_planning(
                    &model_name,
                    reasoning,
                    &planning_agents,
                )?;
                let rebuilt = self.rebuild_llm(&updated_config, query::mode(self.app.state()))?;
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_model_name(model_name.clone());
                self.app.set_reasoning(reasoning);
                self.app
                    .set_safety_model_name(self.config.safety.model_name.clone());
                self.app.set_safety_reasoning(self.config.safety.reasoning);
                self.app.set_planning_agents(planning_agents);
                app::ops::picker::open_reasoning_picker(self.app.state_mut());
                let display_name = crate::codex::display_name(&model_name);
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Model set to `{}` and saved to the active config. Select a reasoning setting.",
                        display_name
                    ),
                );
                Ok(())
            }
            Effect::SetReasoning { reasoning } => {
                let updated_config = AppConfig::set_default_reasoning(reasoning)?;
                let rebuilt = self.rebuild_llm(&updated_config, query::mode(self.app.state()))?;
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_reasoning(reasoning);
                self.app
                    .set_safety_model_name(self.config.safety.model_name.clone());
                self.app.set_safety_reasoning(self.config.safety.reasoning);
                let model_name = query::model_name(self.app.state()).to_string();
                let display_name = crate::codex::display_name(&model_name);
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Reasoning set to `{}` for model `{}` and saved to the active config.",
                        reasoning.as_str(),
                        display_name
                    ),
                );
                Ok(())
            }
            Effect::SetPlanningAgents { planning_agents } => {
                self.ensure_codex_auth_for_models(
                    planning_agents
                        .iter()
                        .map(|agent| agent.model_name.as_str()),
                )?;
                let updated_config = AppConfig::set_default_planning_agents(&planning_agents)?;
                *self.config = updated_config;
                self.app.set_planning_agents(planning_agents.clone());
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Saved {} planning agent{} to the active config.",
                        planning_agents.len(),
                        if planning_agents.len() == 1 { "" } else { "s" }
                    ),
                );
                Ok(())
            }
            Effect::SetSafetySelection {
                model_name,
                reasoning,
            } => {
                self.ensure_codex_auth_for_model(&model_name)?;
                let updated_config =
                    AppConfig::set_default_safety_selection(&model_name, reasoning)?;
                let rebuilt = self.rebuild_llm(&updated_config, query::mode(self.app.state()))?;
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_safety_model_name(model_name.clone());
                self.app.set_safety_reasoning(reasoning);
                let display_name = crate::codex::display_name(&model_name);
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Safety model set to `{}` with `{}` reasoning and saved to the active config.",
                        display_name,
                        reasoning.as_str()
                    ),
                );
                Ok(())
            }
            Effect::RunPlanningWorkflow {
                reply_id,
                description,
                history,
                history_model_name,
            } => {
                self.refresh_codex_auth_if_needed()?;
                let mut model_names = vec![
                    self.config.model.model_name.clone(),
                    self.config.safety.model_name.clone(),
                ];
                model_names.extend(
                    self.config
                        .planning
                        .agents
                        .iter()
                        .map(|agent| agent.model_name.clone()),
                );
                self.ensure_codex_auth_for_models(model_names.iter().map(String::as_str))?;
                self.reply_driver.cancel_active_reply(self.llm);
                let history = history_into_rig(history)?;
                let config = self.config.clone();
                let subagents = self.subagents.clone();
                let ask_user = self.llm.ask_user_controller();
                let stream_tx = self.stream_tx.clone();
                let stats = self.stats.clone();
                let task = self.runtime.spawn(async move {
                    let finalization_tx = stream_tx.clone();
                    let on_finalization_started = Arc::new(move || {
                        let _ = finalization_tx
                            .send((reply_id, StreamEvent::PlanningFinalizationStarted));
                    });
                    let failure_tx = stream_tx.clone();
                    let on_failure = Arc::new(move |message: String| {
                        let _ = failure_tx.send((reply_id, StreamEvent::Failed(message)));
                    });
                    let synth_config = config.clone();
                    let synth_stream_tx = stream_tx.clone();
                    let synth_stats = stats.clone();
                    let synthesize = Arc::new(move |prompt, history, history_model_name| {
                        let config = synth_config.clone();
                        let stream_tx = synth_stream_tx.clone();
                        let stats = synth_stats.clone();
                        let ask_user = ask_user.clone();
                        Box::pin(async move {
                            let llm = LlmService::from_config(
                                &config,
                                AgentContext::main(app::AccessMode::ReadOnly),
                                WriteApprovalController::new(app::ApprovalMode::Manual),
                                ask_user,
                                None,
                            )
                            .map_err(|error| {
                                format!("Failed to start planning synthesis: {error}")
                            })?;
                            let stats_hook = stats.hook_for_model(config.model.model_name.clone());
                            let emit = Arc::new(move |reply_id, event| {
                                stream_tx.send((reply_id, event)).is_ok()
                            });
                            llm.run_prompt(
                                reply_id,
                                prompt,
                                history_from_rig(history).map_err(|error| error.to_string())?,
                                history_model_name,
                                stats_hook,
                                None,
                                emit,
                            )
                            .await
                            .map(|_| ())
                            .map_err(|error| error.to_string())
                        })
                            as crate::features::planning::PlanningSynthesisFuture
                    });

                    run_planning_workflow(
                        reply_id,
                        description,
                        history,
                        history_model_name,
                        config,
                        subagents,
                        on_finalization_started,
                        on_failure,
                        synthesize,
                    )
                    .await;
                });
                self.reply_driver.spawn_task(reply_id, task);
                Ok(())
            }
            Effect::RebuildLlm { access_mode } => {
                let rebuilt = self.rebuild_llm(self.config, access_mode)?;
                *self.llm = rebuilt;
                Ok(())
            }
            Effect::ResolveWriteApproval {
                request_id,
                decision,
            } => self.reply_driver.resolve_write_approval(
                self.runtime,
                self.app,
                self.stats,
                self.llm,
                self.stream_tx.clone(),
                request_id,
                decision,
            ),
            Effect::ResolveShellApproval {
                request_id,
                decision,
            } => self.reply_driver.resolve_shell_approval(
                self.runtime,
                self.app,
                self.stats,
                self.llm,
                self.stream_tx.clone(),
                request_id,
                decision,
            ),
            Effect::ResolveAskUser {
                request_id,
                response,
            } => self.reply_driver.resolve_ask_user(
                self.runtime,
                self.app,
                self.stats,
                self.llm,
                self.stream_tx.clone(),
                request_id,
                response,
            ),
            Effect::CopyToClipboard { text } => {
                write!(
                    self.terminal.backend_mut(),
                    "{}",
                    osc52_copy_sequence(&text)
                )?;
                self.terminal.backend_mut().flush()?;
                let line_count = text.lines().count().max(1);
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Copied {line_count} line{} to the terminal clipboard.",
                        if line_count == 1 { "" } else { "s" }
                    ),
                );
                Ok(())
            }
            Effect::CancelPendingReply => {
                self.reply_driver.cancel_active_reply(self.llm);
                self.runtime
                    .block_on(self.subagents.cancel_all_running(true));
                Ok(())
            }
        }
    }

    fn rebuild_llm(&self, config: &AppConfig, access_mode: app::AccessMode) -> Result<LlmService> {
        let _guard = self.runtime.enter();
        LlmService::from_config(
            config,
            AgentContext::main(access_mode),
            self.llm.approvals(),
            self.llm.ask_user_controller(),
            Some(self.subagents.clone()),
        )
    }

    fn refresh_codex_auth_if_needed(&mut self) -> Result<()> {
        if !self
            .config
            .codex
            .as_ref()
            .is_some_and(crate::codex::should_refresh)
        {
            return Ok(());
        }

        let updated_config = AppConfig::refresh_default_codex_auth_if_needed()?;
        if updated_config != *self.config {
            self.sync_runtime_config(updated_config)?;
        }
        Ok(())
    }

    fn sync_runtime_config(&mut self, updated_config: AppConfig) -> Result<()> {
        let rebuilt = self.rebuild_llm(&updated_config, query::mode(self.app.state()))?;
        *self.config = updated_config;
        *self.llm = rebuilt;
        Ok(())
    }

    fn login_codex(&mut self) -> Result<()> {
        self.run_codex_login_flow()
    }

    fn logout_codex(&mut self) -> Result<()> {
        if self.active_models_use_codex() {
            app::ops::transcript::push_error_message(
                self.app.state_mut(),
                "Switch the main, safety, and planning selections away from Codex before logging out.",
            );
            return Ok(());
        }

        let updated_config = AppConfig::set_default_codex_auth(None)?;
        self.sync_runtime_config(updated_config)?;
        app::ops::transcript::push_agent_message(
            self.app.state_mut(),
            "Cleared stored Codex credentials from config.toml.",
        );
        Ok(())
    }

    fn run_codex_login_flow(&mut self) -> Result<()> {
        let session = self
            .runtime
            .block_on(crate::codex::begin_device_code_login())?;
        let prompt = session.prompt().clone();
        app::ops::transcript::push_agent_message(
            self.app.state_mut(),
            format!(
                "Open {} and enter code `{}`. Waiting for Codex device login to complete.",
                prompt.verification_url, prompt.user_code
            ),
        );
        self.draw_ui()?;

        let codex = self
            .runtime
            .block_on(crate::codex::complete_device_code_login(session))?;
        let updated_config = AppConfig::set_default_codex_auth(Some(&codex))?;
        self.sync_runtime_config(updated_config)?;
        app::ops::transcript::push_agent_message(
            self.app.state_mut(),
            "Codex login complete. Credentials were saved to config.toml.",
        );
        let codex_model_count = self.codex_model_count();
        app::ops::transcript::push_agent_message(
            self.app.state_mut(),
            format!(
                "Loaded {} bundled Codex model{} for the picker.",
                codex_model_count,
                if codex_model_count == 1 { "" } else { "s" }
            ),
        );
        Ok(())
    }

    fn codex_model_count(&self) -> usize {
        model_registry::models()
            .iter()
            .filter(|model| model.provider == ModelProvider::Codex)
            .count()
    }

    fn ensure_codex_auth_for_model(&mut self, model_name: &str) -> Result<()> {
        self.ensure_codex_auth_for_models(std::iter::once(model_name))
    }

    fn ensure_codex_auth_for_models<'a, I>(&mut self, model_names: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let needs_codex_auth = model_names.into_iter().any(Self::model_uses_codex);
        let has_auth = self
            .config
            .codex
            .as_ref()
            .is_some_and(|config| config.is_authenticated());
        if needs_codex_auth && !has_auth {
            self.run_codex_login_flow()?;
        }
        Ok(())
    }

    fn active_models_use_codex(&self) -> bool {
        Self::model_uses_codex(&self.config.model.model_name)
            || Self::model_uses_codex(&self.config.safety.model_name)
            || self
                .config
                .planning
                .agents
                .iter()
                .any(|agent| Self::model_uses_codex(&agent.model_name))
    }

    fn model_uses_codex(model_name: &str) -> bool {
        matches!(
            model_registry::find_model(model_name).map(|model| model.provider),
            Some(ModelProvider::Codex)
        )
    }

    fn draw_ui(&mut self) -> Result<()> {
        self.terminal.draw(|frame| ui::render(frame, self.app))?;
        Ok(())
    }
}
