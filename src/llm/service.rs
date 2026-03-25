use std::{env, sync::Arc};

use anyhow::{Context, Result};
use rig::{completion::Message as RigMessage, providers::openai};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::AgentContext,
    app::{
        AccessMode, PendingReplyReplaySeed, SessionHistoryMessage, ShellApprovalDecision,
        WriteApprovalDecision,
    },
    ask_user::AskUserResponse,
    completion_request::CompletionRequestSnapshot,
    config::{AppConfig, ReasoningEffort},
    model_registry,
    stats::StatsHook,
    subagents::SubagentManager,
    tools::{ToolContext, tool_names_for_context},
};

use super::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResumeOverride, StreamEvent,
    agent_builder::{azure_openai_base_url, build_agent, mode_preamble, run_plain_prompt},
    compaction::{
        COMPACTION_NOTICE, COMPACTION_PROMPT, compaction_model_for_pre_turn,
        drop_oldest_compaction_source_message, is_retryable_compaction_error,
        rebuild_compacted_history, should_compact_before_follow_up,
    },
    history_from_rig, history_into_rig,
    hooks::{AskUserController, ShellApprovalController, WriteApprovalController},
    resume::ResumeOverrideController,
    safety::SafetyClassifier,
    streaming::{PromptStepOutcome, run_prompt_step},
};

const MAX_TOOL_STEPS_PER_TURN: usize = 64;

#[derive(Clone)]
pub struct LlmService {
    pub(crate) agent: super::LlmAgent,
    client: openai::CompletionsClient,
    model_name: String,
    reasoning_effort: ReasoningEffort,
    pub(crate) access_mode: AccessMode,
    role: crate::agent::AgentRole,
    pub(crate) approvals: WriteApprovalController,
    pub(crate) shell_approvals: ShellApprovalController,
    pub(crate) safety: SafetyClassifier,
    pub(crate) ask_user: Option<AskUserController>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) tool_names: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) preamble: String,
}

impl LlmService {
    pub fn from_config(
        config: &AppConfig,
        context: AgentContext,
        approvals: WriteApprovalController,
        ask_user: Option<AskUserController>,
        subagents: Option<SubagentManager>,
    ) -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let client = openai::CompletionsClient::builder()
            .api_key(&config.azure.api_key)
            .base_url(azure_openai_base_url(config))
            .build()
            .context("failed to build OpenAI-compatible Azure client")?;

        let preamble = mode_preamble(&context);
        let tool_context = ToolContext {
            root: workspace_root,
            agent: context.clone(),
            config: config.clone(),
            approval_mode: approvals.mode(),
            ask_user_available: ask_user.is_some(),
            subagents,
        };
        let tool_names = tool_names_for_context(&tool_context);
        let approval_mode = approvals.mode();
        let model_name = context
            .model_name_override
            .clone()
            .unwrap_or_else(|| config.azure.model_name.clone());
        let agent = build_agent(
            &client,
            &model_name,
            &preamble,
            config.azure.reasoning_effort,
            Some(tool_context),
        );
        let safety = SafetyClassifier::from_client(&client, config);

        Ok(Self {
            agent,
            client,
            model_name,
            reasoning_effort: config.azure.reasoning_effort,
            access_mode: context.access_mode,
            role: context.role,
            approvals,
            shell_approvals: ShellApprovalController::new(approval_mode),
            safety,
            ask_user,
            tool_names,
            preamble,
        })
    }

    pub fn approvals(&self) -> WriteApprovalController {
        self.approvals.clone()
    }

    pub fn ask_user_controller(&self) -> Option<AskUserController> {
        self.ask_user.clone()
    }

    pub fn resolve_write_approval(
        &self,
        request_id: &str,
        decision: WriteApprovalDecision,
    ) -> InteractionResolveResult {
        self.approvals.resolve(request_id, decision)
    }

    pub fn resolve_shell_approval(
        &self,
        request_id: &str,
        decision: ShellApprovalDecision,
    ) -> InteractionResolveResult {
        self.shell_approvals.resolve(request_id, decision)
    }

    pub fn resolve_ask_user(
        &self,
        request_id: &str,
        response: AskUserResponse,
    ) -> InteractionResolveResult {
        self.ask_user
            .as_ref()
            .map(|controller| controller.resolve(request_id, response))
            .unwrap_or(InteractionResolveResult::Missing)
    }

    pub fn can_resolve_write_approval(&self, request_id: &str) -> bool {
        self.approvals.can_resolve(request_id)
    }

    pub fn can_resolve_shell_approval(&self, request_id: &str) -> bool {
        self.shell_approvals.can_resolve(request_id)
    }

    pub fn can_resolve_ask_user(&self, request_id: &str) -> bool {
        self.ask_user
            .as_ref()
            .is_some_and(|controller| controller.can_resolve(request_id))
    }

    pub fn reset_write_approvals(&self) {
        self.approvals.reset_session();
        self.shell_approvals.reset_session();
    }

    pub fn cancel_pending_interactions(&self) {
        self.approvals.cancel_pending();
        self.shell_approvals.cancel_pending();
        if let Some(controller) = &self.ask_user {
            controller.cancel_pending();
        }
    }

    pub async fn stream_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        events: UnboundedSender<(u64, StreamEvent)>,
    ) {
        let emit: EventCallback =
            Arc::new(move |reply_id, event| events.send((reply_id, event)).is_ok());
        let _ = self
            .run_prompt(
                reply_id,
                prompt,
                history,
                history_model_name,
                stats_hook,
                None,
                emit,
            )
            .await;
    }

    pub async fn stream_resumed_prompt(
        &self,
        reply_id: u64,
        snapshot: CompletionRequestSnapshot,
        stats_hook: StatsHook,
        events: UnboundedSender<(u64, StreamEvent)>,
        override_action: ResumeOverride,
        replay_seed: Option<PendingReplyReplaySeed>,
    ) {
        let emit: EventCallback =
            Arc::new(move |reply_id, event| events.send((reply_id, event)).is_ok());
        let _ = self
            .run_prompt_from_state(
                reply_id,
                snapshot.prompt,
                snapshot.history,
                stats_hook,
                None,
                emit,
                Some(ResumeOverrideController::new(override_action)),
                replay_seed,
            )
            .await;
    }

    pub async fn run_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
    ) -> Result<PromptRunResult> {
        let prompt = RigMessage::user(prompt);
        let mut history = history_into_rig(history)?;

        if let Some(compaction_model_name) = compaction_model_for_pre_turn(
            &self.model_name,
            &history,
            history_model_name.as_deref(),
            &prompt,
        ) {
            let result = self
                .compact_history(
                    history.clone(),
                    &compaction_model_name,
                    reply_id,
                    emit.clone(),
                    self.role == crate::agent::AgentRole::Main,
                )
                .await?;
            history = history_into_rig(result.history)?;
        }

        self.run_prompt_from_state(
            reply_id, prompt, history, stats_hook, capture, emit, None, None,
        )
        .await
    }

    async fn run_prompt_from_state(
        &self,
        reply_id: u64,
        mut prompt: RigMessage,
        mut history: Vec<RigMessage>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
        resume: Option<ResumeOverrideController>,
        mut replay_seed: Option<PendingReplyReplaySeed>,
    ) -> Result<PromptRunResult> {
        let mut steps = 0;

        loop {
            steps += 1;
            if steps > MAX_TOOL_STEPS_PER_TURN {
                let message =
                    format!("Request exceeded the turn step limit ({MAX_TOOL_STEPS_PER_TURN}).");
                let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
                return Err(anyhow::anyhow!(message));
            }

            match run_prompt_step(
                self,
                reply_id,
                prompt,
                history,
                stats_hook.clone(),
                capture.clone(),
                emit.clone(),
                resume.clone(),
                replay_seed.take(),
                MAX_TOOL_STEPS_PER_TURN,
            )
            .await?
            {
                PromptStepOutcome::Finished(result) => {
                    return Ok(result);
                }
                PromptStepOutcome::Continue(next) => {
                    prompt = next.next_prompt;
                    history = next.history;
                    if should_compact_before_follow_up(&self.model_name, &history, &prompt) {
                        let result = self
                            .compact_history(
                                history.clone(),
                                &self.model_name,
                                reply_id,
                                emit.clone(),
                                self.role == crate::agent::AgentRole::Main,
                            )
                            .await?;
                        history = history_into_rig(result.history)?;
                    }
                }
            }
        }
    }

    pub async fn compact_history_for_session(
        &self,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
    ) -> Result<HistoryCompactionResult> {
        let model_name = history_model_name.unwrap_or_else(|| self.model_name.clone());
        self.compact_history(
            history_into_rig(history)?,
            &model_name,
            0,
            Arc::new(|_, _| true),
            false,
        )
        .await
    }

    async fn compact_history(
        &self,
        history: Vec<RigMessage>,
        model_name: &str,
        reply_id: u64,
        emit: EventCallback,
        emit_notice: bool,
    ) -> Result<HistoryCompactionResult> {
        let compact_agent = build_agent(
            &self.client,
            model_name,
            &self.preamble,
            self.reasoning_effort,
            None,
        );
        let mut candidate_history = history;

        loop {
            let request_tokens = super::compaction::estimated_request_tokens(
                &candidate_history,
                &RigMessage::user(COMPACTION_PROMPT),
            );
            if model_registry::find_model(model_name)
                .is_some_and(|model| request_tokens > model.context_length)
            {
                if !drop_oldest_compaction_source_message(&mut candidate_history) {
                    return Err(anyhow::anyhow!(
                        "Compaction request exceeded the model context and could not be reduced further."
                    ));
                }
                continue;
            }

            let summary = match run_plain_prompt(
                &compact_agent,
                COMPACTION_PROMPT.to_string(),
                candidate_history.clone(),
            )
            .await
            {
                Ok(summary) => summary,
                Err(error) if is_retryable_compaction_error(&error.to_string()) => {
                    if !drop_oldest_compaction_source_message(&mut candidate_history) {
                        return Err(error);
                    }
                    continue;
                }
                Err(error) => return Err(error),
            };

            let rebuilt = rebuild_compacted_history(&candidate_history, &summary);
            if emit_notice && !(emit)(reply_id, StreamEvent::Commentary(COMPACTION_NOTICE.into())) {
                return Err(anyhow::anyhow!("event sink unavailable"));
            }
            return Ok(HistoryCompactionResult {
                history: history_from_rig(rebuilt)?,
                model_name: model_name.to_string(),
            });
        }
    }
}
