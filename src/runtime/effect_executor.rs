use std::io::Write;
use std::sync::Arc;

use anyhow::Result;
use tokio::{runtime::Runtime, sync::mpsc};

use crate::{
    Tui,
    agent::AgentContext,
    app::{self, App, Effect, query},
    background_terminals::{
        BackgroundTerminalInspectRequest, BackgroundTerminalManager,
        format_terminal_inspect_message, format_terminal_list_message,
    },
    config::AppConfig,
    debug_log::log_debug,
    features::planning::run_planning_workflow,
    features::planning::sanitize_planning_agents,
    llm::{
        AskUserController, LlmService, WriteApprovalController, history_from_rig, history_into_rig,
    },
    memory::MemoryService,
    model_registry::{self, ModelProvider},
    session_store::SessionStore,
    stats::StatsStore,
    subagents::SubagentManager,
    ui,
    web::WebService,
};

use super::side_channel_task_manager::SideChannelTaskManager;
use super::{RuntimeEvent, clipboard::osc52_copy_sequence, reply_driver::ReplyDriver};

pub(crate) struct EffectExecutor<'a> {
    pub(crate) runtime: &'a Runtime,
    pub(crate) terminal: &'a mut Tui,
    pub(crate) reply_driver: &'a mut ReplyDriver,
    pub(crate) side_channel_task_manager: &'a mut SideChannelTaskManager,
    pub(crate) llm: &'a mut LlmService,
    pub(crate) config: &'a mut AppConfig,
    pub(crate) app: &'a mut App,
    pub(crate) stats: &'a StatsStore,
    pub(crate) session_store: &'a mut SessionStore,
    pub(crate) memory: &'a MemoryService,
    pub(crate) stream_tx: mpsc::UnboundedSender<RuntimeEvent>,
    pub(crate) subagents: &'a SubagentManager,
    pub(crate) terminals: &'a BackgroundTerminalManager,
}

fn rebuild_main_llm(
    runtime: &Runtime,
    config: &AppConfig,
    llm: &LlmService,
    access_mode: app::AccessMode,
    subagents: &SubagentManager,
    terminals: &BackgroundTerminalManager,
    memory: &MemoryService,
    web: WebService,
) -> Result<LlmService> {
    let _guard = runtime.enter();
    LlmService::from_config_with_controllers(
        config,
        AgentContext::main(access_mode),
        llm.approvals(),
        llm.shell_approvals(),
        llm.ask_user_controller(),
        llm.todo_available(),
        Some(memory.clone()),
        Some(subagents.clone()),
        Some(terminals.clone()),
        web,
    )
}

fn build_fresh_main_llm(
    runtime: &Runtime,
    config: &AppConfig,
    access_mode: app::AccessMode,
    approval_mode: app::ApprovalMode,
    subagents: &SubagentManager,
    terminals: &BackgroundTerminalManager,
    memory: &MemoryService,
    web: WebService,
) -> Result<LlmService> {
    let _guard = runtime.enter();
    LlmService::from_config(
        config,
        AgentContext::main(access_mode),
        WriteApprovalController::new(approval_mode),
        Some(AskUserController::default()),
        true,
        Some(memory.clone()),
        Some(subagents.clone()),
        Some(terminals.clone()),
        web,
    )
}

fn sync_main_llm_access_mode(
    runtime: &Runtime,
    config: &AppConfig,
    llm: &mut LlmService,
    access_mode: app::AccessMode,
    subagents: &SubagentManager,
    terminals: &BackgroundTerminalManager,
    memory: &MemoryService,
    web: WebService,
) -> Result<bool> {
    if llm.access_mode == access_mode {
        return Ok(false);
    }

    *llm = rebuild_main_llm(
        runtime,
        config,
        llm,
        access_mode,
        subagents,
        terminals,
        memory,
        web,
    )?;
    Ok(true)
}

impl EffectExecutor<'_> {
    pub(crate) fn run(&mut self, effect: Effect) -> Result<()> {
        match effect {
            Effect::PromptModel {
                reply_id,
                prompt,
                history,
                history_model_name,
                session_title_prompt,
            } => {
                let prompt = self.memory.augment_prompt(&prompt)?;
                if let Some(seed) = self
                    .app
                    .state_mut()
                    .session
                    .active_main_request_seed
                    .as_mut()
                {
                    seed.model_prompt = prompt.clone();
                }
                log_debug(
                    "effect_executor",
                    format!(
                        "prompt_model reply_id={reply_id} history_len={} prompt_chars={}",
                        history.len(),
                        prompt.chars().count()
                    ),
                );
                self.refresh_codex_auth_if_needed()?;
                self.sync_llm_access_mode(query::mode(self.app.state()))?;
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
                if let Some(session_title_prompt) = session_title_prompt {
                    app::ops::session::begin_session_title_request(self.app.state_mut(), reply_id);
                    let title_llm = self.llm.clone();
                    let title_stream_tx = self.stream_tx.clone();
                    let title_stats_hook = self
                        .stats
                        .hook_for_model(query::model_name(self.app.state()).to_string());
                    self.runtime.spawn(async move {
                        let title = title_llm
                            .generate_session_title(session_title_prompt, title_stats_hook)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let _ = title_stream_tx.send(RuntimeEvent::MainReply {
                            reply_id,
                            event: crate::app::StreamEvent::SessionTitleGenerated(title),
                        });
                    });
                }
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
            Effect::PromptSideChannel {
                reply_id,
                prompt,
                history,
                history_model_name,
            } => {
                let prompt = self.memory.augment_prompt(&prompt)?;
                self.refresh_codex_auth_if_needed()?;
                self.sync_llm_access_mode(query::mode(self.app.state()))?;
                let model_names = vec![
                    self.config.model.model_name.clone(),
                    self.config.safety.model_name.clone(),
                ];
                self.ensure_codex_auth_for_models(model_names.iter().map(String::as_str))?;
                let llm = self.llm.clone();
                let stats_hook = self
                    .stats
                    .hook_for_model(query::model_name(self.app.state()).to_string());
                let stream_tx = self.stream_tx.clone();
                let task = self.runtime.spawn(async move {
                    llm.stream_side_channel(
                        prompt,
                        reply_id,
                        history,
                        history_model_name,
                        stats_hook,
                        stream_tx,
                    )
                    .await;
                });
                self.side_channel_task_manager.spawn_task(reply_id, task);
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
                let stats_hook = self
                    .stats
                    .hook_for_model(query::model_name(self.app.state()).to_string());
                let stream_tx = self.stream_tx.clone();
                let reply_id = ReplyDriver::require_active_reply_id(self.app)?;
                let task = self.runtime.spawn(async move {
                    let event = match llm
                        .compact_history_for_session(history, history_model_name, stats_hook)
                        .await
                    {
                        Ok(result) => crate::app::StreamEvent::CompactionFinished {
                            history: result.history,
                            model_name: result.model_name,
                        },
                        Err(error) => crate::app::StreamEvent::Failed(error.to_string()),
                    };
                    let _ = stream_tx.send(RuntimeEvent::MainReply { reply_id, event });
                });
                self.reply_driver.spawn_task(reply_id, task);
                Ok(())
            }
            Effect::SearchMemories {
                query,
                include_candidates,
            } => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.search_text(
                        &query,
                        include_candidates,
                        self.config.memory.max_candidate_search_results,
                    )?,
                );
                Ok(())
            }
            Effect::ShowMemory { id } => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.get_text(&id)?,
                );
                Ok(())
            }
            Effect::ListMemoryCandidates => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.list_candidates_text()?,
                );
                Ok(())
            }
            Effect::ShowMemoryStats => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.stats_text()?,
                );
                Ok(())
            }
            Effect::PromoteMemory { id } => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.promote(&id)?,
                );
                Ok(())
            }
            Effect::ArchiveMemory { id } => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.archive(&id)?,
                );
                Ok(())
            }
            Effect::ReplaceMemory { id, text } => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.replace(&id, &text)?,
                );
                Ok(())
            }
            Effect::ClearMemories => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.clear()?,
                );
                Ok(())
            }
            Effect::RebuildMemoryIndexes => {
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    self.memory.rebuild_indexes()?,
                );
                Ok(())
            }
            Effect::ShowStats => {
                app::ops::stats::open_stats_screen(self.app.state_mut(), self.stats.report()?);
                Ok(())
            }
            Effect::OpenSessionPicker => {
                let entries = self
                    .session_store
                    .list_sessions_for_workspace(&self.app.state().session.workspace_root)?
                    .into_iter()
                    .map(|entry| app::SessionPickerEntry {
                        session_id: entry.session_id,
                        title: entry.title,
                        detail: entry.detail,
                        resumable: entry.resumable,
                    })
                    .collect::<Vec<_>>();
                if entries.is_empty() {
                    app::ops::transcript::push_agent_message(
                        self.app.state_mut(),
                        "No saved sessions were found for this workspace.",
                    );
                } else {
                    app::ops::picker::open_session_picker(self.app.state_mut(), entries);
                }
                Ok(())
            }
            Effect::OpenModelPicker => {
                app::ops::picker::open_model_picker(self.app.state_mut());
                Ok(())
            }
            Effect::LoginCodex => self.login_codex(),
            Effect::LogoutCodex => self.logout_codex(),
            Effect::RotateSession => {
                self.side_channel_task_manager.cancel_all();
                self.runtime
                    .block_on(self.subagents.cancel_all_running(false));
                self.terminals.cancel_all_running();
                self.stats.rotate_session()?;
                let rebuilt = build_fresh_main_llm(
                    self.runtime,
                    self.config,
                    query::mode(self.app.state()),
                    query::approval_mode(self.app.state()),
                    self.subagents,
                    self.terminals,
                    self.memory,
                    self.llm.web_service(),
                )?;
                *self.llm = rebuilt;
                self.session_store
                    .rotate_to_new_tui_session(self.app.state(), &self.llm.preamble)?;
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
                self.memory.set_config(updated_config.memory.clone());
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_model_name(model_name.clone());
                self.app.set_reasoning(reasoning);
                self.app
                    .set_safety_model_name(self.config.safety.model_name.clone());
                self.app.set_safety_reasoning(self.config.safety.reasoning);
                self.app
                    .set_memory_model_name(self.config.memory.extraction.model_name.clone());
                self.app
                    .set_memory_reasoning(self.config.memory.extraction.reasoning);
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
                self.memory.set_config(updated_config.memory.clone());
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_reasoning(reasoning);
                self.app
                    .set_safety_model_name(self.config.safety.model_name.clone());
                self.app.set_safety_reasoning(self.config.safety.reasoning);
                self.app
                    .set_memory_model_name(self.config.memory.extraction.model_name.clone());
                self.app
                    .set_memory_reasoning(self.config.memory.extraction.reasoning);
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
                self.memory.set_config(updated_config.memory.clone());
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
            Effect::SetMemorySelection {
                model_name,
                reasoning,
            } => {
                self.ensure_codex_auth_for_model(&model_name)?;
                let updated_config =
                    AppConfig::set_default_memory_selection(&model_name, reasoning)?;
                self.memory.set_config(updated_config.memory.clone());
                *self.config = updated_config;
                self.app.set_memory_model_name(model_name.clone());
                self.app.set_memory_reasoning(reasoning);
                let display_name = crate::codex::display_name(&model_name);
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!(
                        "Memory model set to `{}` with `{}` reasoning and saved to the active config.",
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
                let todo_available = self.llm.todo_available();
                let write_approvals = self.llm.approvals();
                let shell_approvals = self.llm.shell_approvals();
                let web = self.llm.web_service();
                let stream_tx = self.stream_tx.clone();
                let stats = self.stats.clone();
                let task = self.runtime.spawn(async move {
                    let finalization_tx = stream_tx.clone();
                    let on_finalization_started = Arc::new(move || {
                        let _ = finalization_tx.send(RuntimeEvent::MainReply {
                            reply_id,
                            event: crate::app::StreamEvent::PlanningFinalizationStarted,
                        });
                    });
                    let failure_tx = stream_tx.clone();
                    let on_failure = Arc::new(move |message: String| {
                        let _ = failure_tx.send(RuntimeEvent::MainReply {
                            reply_id,
                            event: crate::app::StreamEvent::Failed(message),
                        });
                    });
                    let synth_config = config.clone();
                    let synth_stream_tx = stream_tx.clone();
                    let synth_stats = stats.clone();
                    let workflow_write_approvals = write_approvals.clone();
                    let workflow_shell_approvals = shell_approvals.clone();
                    let planning_web = web.clone();
                    let synthesize = Arc::new(move |prompt, history, history_model_name| {
                        let config = synth_config.clone();
                        let stream_tx = synth_stream_tx.clone();
                        let stats = synth_stats.clone();
                        let ask_user = ask_user.clone();
                        let write_approvals = write_approvals.clone();
                        let shell_approvals = shell_approvals.clone();
                        let web = planning_web.clone();
                        Box::pin(async move {
                            let llm = LlmService::from_config_with_controllers(
                                &config,
                                AgentContext::main(app::AccessMode::ReadOnly),
                                write_approvals,
                                shell_approvals,
                                ask_user,
                                todo_available,
                                None,
                                None,
                                None,
                                web,
                            )
                            .map_err(|error| {
                                format!("Failed to start planning synthesis: {error}")
                            })?;
                            let stats_hook = stats.hook_for_model(config.model.model_name.clone());
                            let emit = Arc::new(move |reply_id, event| {
                                stream_tx
                                    .send(RuntimeEvent::MainReply { reply_id, event })
                                    .is_ok()
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
                        workflow_write_approvals,
                        workflow_shell_approvals,
                        web,
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
            } => {
                self.subagents.clear_waiting_for_approval(&request_id);
                self.reply_driver.resolve_write_approval(
                    self.runtime,
                    self.app,
                    self.stats,
                    self.llm,
                    self.stream_tx.clone(),
                    request_id,
                    decision,
                )
            }
            Effect::ResolveShellApproval {
                request_id,
                decision,
            } => {
                self.subagents.clear_waiting_for_approval(&request_id);
                self.reply_driver.resolve_shell_approval(
                    self.runtime,
                    self.app,
                    self.stats,
                    self.llm,
                    self.stream_tx.clone(),
                    request_id,
                    decision,
                )
            }
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
            Effect::ResumeSession { session_id } => self.resume_session(&session_id),
            Effect::CancelPendingReply => {
                log_debug("effect_executor", "cancel_pending_reply effect");
                self.reply_driver.cancel_active_reply(self.llm);
                self.runtime
                    .block_on(self.subagents.cancel_all_running(true));
                Ok(())
            }
            Effect::ListBackgroundTerminals => {
                let terminals = self.terminals.list();
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format_terminal_list_message(&terminals),
                );
                Ok(())
            }
            Effect::InspectBackgroundTerminal { id } => {
                let result = self.runtime.block_on(self.terminals.inspect(
                    &id,
                    BackgroundTerminalInspectRequest {
                        after_sequence: None,
                        wait_for_change_ms: None,
                    },
                ))?;
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format_terminal_inspect_message(&result),
                );
                Ok(())
            }
            Effect::KillBackgroundTerminal { id } => {
                log_debug(
                    "effect_executor",
                    format!("kill_background_terminal id={id}"),
                );
                let snapshot = self.terminals.kill(&id)?;
                app::ops::transcript::push_agent_message(
                    self.app.state_mut(),
                    format!("Cancelled background terminal `{}`.", snapshot.id),
                );
                Ok(())
            }
        }
    }

    fn rebuild_llm(&self, config: &AppConfig, access_mode: app::AccessMode) -> Result<LlmService> {
        rebuild_main_llm(
            self.runtime,
            config,
            self.llm,
            access_mode,
            self.subagents,
            self.terminals,
            self.memory,
            self.llm.web_service(),
        )
    }

    fn sync_llm_access_mode(&mut self, access_mode: app::AccessMode) -> Result<bool> {
        let web = self.llm.web_service();
        sync_main_llm_access_mode(
            self.runtime,
            self.config,
            self.llm,
            access_mode,
            self.subagents,
            self.terminals,
            self.memory,
            web,
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
        self.memory.set_config(updated_config.memory.clone());
        *self.config = updated_config;
        *self.llm = rebuilt;
        Ok(())
    }

    fn restore_runtime_config_from_snapshot(
        &mut self,
        snapshot: &crate::session_store::PersistedSessionSnapshot,
    ) -> Result<(AppConfig, LlmService)> {
        let mut updated_config = self.config.clone();
        updated_config.model.model_name = snapshot.runtime.model_name.clone();
        updated_config.model.reasoning = snapshot.runtime.reasoning;
        updated_config.safety.model_name = snapshot.runtime.safety_model_name.clone();
        updated_config.safety.reasoning = snapshot.runtime.safety_reasoning;
        updated_config.memory.extraction.model_name = snapshot.runtime.memory_model_name.clone();
        updated_config.memory.extraction.reasoning = snapshot.runtime.memory_reasoning;
        updated_config.planning.agents = sanitize_planning_agents(
            &snapshot.runtime.model_name,
            &snapshot.runtime.planning_agents,
        );
        let rebuilt = build_fresh_main_llm(
            self.runtime,
            &updated_config,
            snapshot.runtime.access_mode,
            snapshot.runtime.approval_mode,
            self.subagents,
            self.terminals,
            self.memory,
            self.llm.web_service(),
        )?;
        Ok((updated_config, rebuilt))
    }

    fn resume_session(&mut self, session_id: &str) -> Result<()> {
        let mut snapshot = self.session_store.load_session(session_id)?;
        if model_registry::find_model(&snapshot.runtime.model_name).is_none()
            || model_registry::find_model(&snapshot.runtime.safety_model_name).is_none()
            || model_registry::find_model(&snapshot.runtime.memory_model_name).is_none()
        {
            app::ops::transcript::push_error_message(
                self.app.state_mut(),
                format!(
                    "Session `{session_id}` cannot be resumed because one or more saved model selections are unavailable."
                ),
            );
            return Ok(());
        }
        snapshot.runtime.planning_agents = sanitize_planning_agents(
            &snapshot.runtime.model_name,
            &snapshot.runtime.planning_agents,
        );

        let (updated_config, rebuilt_llm) = self.restore_runtime_config_from_snapshot(&snapshot)?;
        let initial_mode = self.app.state().session.initial_mode;
        let initial_approval_mode = self.app.state().session.initial_approval_mode;
        let restored_state = snapshot
            .clone()
            .into_app_state(initial_mode, initial_approval_mode);
        let session_label = snapshot
            .runtime
            .title
            .clone()
            .unwrap_or_else(|| snapshot.session_id.clone());

        self.reply_driver.cancel_active_reply(self.llm);
        self.side_channel_task_manager.cancel_all();
        self.runtime
            .block_on(self.subagents.cancel_all_running(true));
        self.terminals.cancel_all_running();

        self.memory.set_config(updated_config.memory.clone());
        *self.config = updated_config;
        *self.llm = rebuilt_llm;
        self.app.replace_state(restored_state);
        self.session_store.attach_resumed_session(snapshot);
        app::ops::transcript::push_agent_message(
            self.app.state_mut(),
            format!("Resumed session `{session_label}`. Interrupted work was not restarted."),
        );
        Ok(())
    }

    fn login_codex(&mut self) -> Result<()> {
        self.run_codex_login_flow()
    }

    fn logout_codex(&mut self) -> Result<()> {
        if self.active_models_use_codex() {
            app::ops::transcript::push_error_message(
                self.app.state_mut(),
                "Switch the main, safety, memory, and planning selections away from Codex before logging out.",
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
            || Self::model_uses_codex(&self.config.memory.extraction.model_name)
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

#[cfg(test)]
mod tests {
    use tokio::{runtime::Runtime, sync::mpsc};

    use super::sync_main_llm_access_mode;
    use crate::{
        agent::AgentContext,
        app::{AccessMode, ApprovalMode},
        background_terminals::BackgroundTerminalManager,
        config::{
            AppConfig, AzureConfig, MemoryConfig, ModelSelectionConfig, ReasoningEffort,
            SafetyConfig, SubagentConfig, ToolConfig, UiConfig,
        },
        features::planning::PlanningConfig,
        llm::{AskUserController, LlmService, WriteApprovalController},
        memory::MemoryService,
        stats::StatsStore,
        subagents::SubagentManager,
        web::WebService,
    };

    fn sample_config() -> AppConfig {
        AppConfig {
            azure: Some(AzureConfig {
                resource_name: "demo-resource".into(),
                api_key: "secret".into(),
                api_version: "2025-01-01-preview".into(),
            }),
            chutes: None,
            codex: None,
            ollama: None,
            opencode: None,
            openrouter: None,
            model: ModelSelectionConfig {
                model_name: "gpt-5.4-mini".into(),
                reasoning: ReasoningEffort::Medium.into(),
            },
            safety: SafetyConfig {
                model_name: "gpt-5.4-mini".into(),
                reasoning: ReasoningEffort::Medium.into(),
            },
            memory: MemoryConfig::default(),
            ui: UiConfig::default(),
            subagents: SubagentConfig::default(),
            planning: PlanningConfig::default(),
            tools: ToolConfig::default(),
        }
    }

    #[test]
    fn syncing_prompt_runtime_mode_rebuilds_llm_with_write_tools() {
        let runtime = Runtime::new().expect("runtime");
        let config = sample_config();
        let (subagent_tx, _) = mpsc::unbounded_channel();
        let subagents = SubagentManager::new(4, subagent_tx, StatsStore::new());
        let (terminal_tx, _) = mpsc::unbounded_channel();
        let terminals = BackgroundTerminalManager::new(terminal_tx);
        let memory =
            MemoryService::new(config.memory.clone(), std::env::current_dir().expect("cwd"))
                .expect("memory");
        let web = WebService::new(config.tools.max_output_tokens).expect("web");
        let _guard = runtime.enter();
        let mut llm = LlmService::from_config(
            &config,
            AgentContext::main(AccessMode::ReadOnly),
            WriteApprovalController::new(ApprovalMode::Manual),
            Some(AskUserController::default()),
            true,
            Some(memory.clone()),
            Some(subagents.clone()),
            Some(terminals.clone()),
            web.clone(),
        )
        .expect("service builds");
        drop(_guard);

        let rebuilt = sync_main_llm_access_mode(
            &runtime,
            &config,
            &mut llm,
            AccessMode::ReadWrite,
            &subagents,
            &terminals,
            &memory,
            web,
        )
        .expect("runtime mode sync succeeds");

        assert!(rebuilt);
        assert_eq!(llm.access_mode, AccessMode::ReadWrite);
        assert!(llm.tool_names.contains(&"ApplyPatches".to_string()));
        assert!(llm.tool_names.contains(&"WriteFile".to_string()));
        assert!(llm.preamble.contains("You are currently in write mode."));
        assert!(
            !llm.preamble
                .contains("You are currently in read-only mode.")
        );
    }
}
