use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result};
use rig::{completion::Message as RigMessage, providers::openai};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::AgentContext,
    app::{
        AccessMode, HostedToolKind, PendingReplyReplaySeed, SessionHistoryMessage,
        ShellApprovalDecision, SideChannelEvent, TurnEndReason, WriteApprovalDecision,
    },
    ask_user::AskUserResponse,
    background_terminals::BackgroundTerminalManager,
    completion_request::CompletionRequestSnapshot,
    config::{AppConfig, ReasoningSetting, WebSearchMode},
    debug_log::log_debug,
    memory::MemoryService,
    model_registry,
    runtime::RuntimeEvent,
    stats::StatsHook,
    subagents::SubagentManager,
    tools::{ToolContext, tool_names_for_context},
};

use super::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResponsesClient, ResponsesHostedToolEvent, ResponsesSearchObserverGuard,
    ResumeOverride, StreamEvent,
    agent_builder::{
        RequestFeatures, build_agent, http_headers_for_model, mode_preamble,
        openai_base_url_for_model, run_plain_prompt_with_hook,
    },
    compaction::{
        COMPACTION_NOTICE, COMPACTION_PROMPT, compaction_model_for_pre_turn,
        drop_oldest_compaction_source_message, is_retryable_compaction_error,
        rebuild_compacted_history, should_compact_before_follow_up,
    },
    history_from_rig, history_into_rig, history_with_prompt_from_rig,
    hooks::{AskUserController, ShellApprovalController, WriteApprovalController},
    resume::ResumeOverrideController,
    safety::SafetyClassifier,
    streaming::{PromptStepOutcome, run_prompt_step},
};

const MAX_TOOL_STEPS_PER_TURN: usize = 64;
const SESSION_TITLE_PROMPT_PREFIX: &str = concat!(
    "Write a concise title for this session based on the user's first request.\n",
    "Respond with only the title.\n",
    "Maximum 6 words.\n",
    "No quotes.\n",
    "No markdown.\n\n",
    "User request:\n"
);
static NEXT_INTERACTION_SCOPE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
enum AgentVariant {
    Completions(super::OpenAiCompletionsAgent),
    Responses(super::ResponsesAgent),
}

#[derive(Clone)]
enum ClientVariant {
    Completions(openai::CompletionsClient),
    Responses(ResponsesClient),
}

fn uses_responses_api(model_name: &str) -> bool {
    model_registry::uses_responses_api(model_name)
}

fn responses_http_client() -> crate::codex::ResponsesHttpClient {
    crate::codex::ResponsesHttpClient::new(rig::http_client::ReqwestClient::new())
}

fn build_responses_client(
    config: &AppConfig,
    model_name: &str,
    interaction_scope: Option<&str>,
) -> Result<ResponsesClient> {
    let provider_config = config.provider_config_for_model(model_name)?;
    let mut headers = http_headers_for_model(config, model_name)?;
    if let Some(scope) = interaction_scope {
        headers.insert(
            super::OAT_INTERACTION_SCOPE_HEADER,
            rig::http_client::HeaderValue::from_str(scope)?,
        );
    }
    openai::Client::builder()
        .http_client(responses_http_client())
        .api_key(provider_config.auth_token().unwrap_or_default())
        .base_url(openai_base_url_for_model(config, model_name)?)
        .http_headers(headers)
        .build()
        .context("failed to build OpenAI Responses client")
}

fn build_completions_client(
    config: &AppConfig,
    model_name: &str,
) -> Result<openai::CompletionsClient> {
    let provider_config = config.provider_config_for_model(model_name)?;
    openai::CompletionsClient::builder()
        .api_key(provider_config.auth_token().unwrap_or_default())
        .base_url(openai_base_url_for_model(config, model_name)?)
        .http_headers(http_headers_for_model(config, model_name)?)
        .build()
        .context("failed to build OpenAI-compatible client")
}

fn build_client_and_agent(
    config: &AppConfig,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    interactive_features: RequestFeatures,
    interaction_scope: &str,
    tool_context: ToolContext,
) -> Result<(ClientVariant, AgentVariant)> {
    if uses_responses_api(model_name) {
        let client = build_responses_client(config, model_name, Some(interaction_scope))?;
        let agent = build_agent(
            &client,
            model_name,
            preamble,
            reasoning,
            interactive_features,
            Some(tool_context),
        );
        Ok((
            ClientVariant::Responses(client),
            AgentVariant::Responses(agent),
        ))
    } else {
        let client = build_completions_client(config, model_name)?;
        let agent = build_agent(
            &client,
            model_name,
            preamble,
            reasoning,
            RequestFeatures::default(),
            Some(tool_context),
        );
        Ok((
            ClientVariant::Completions(client),
            AgentVariant::Completions(agent),
        ))
    }
}

fn build_safety_client(config: &AppConfig) -> Result<super::safety::SafetyClient> {
    let model_name = &config.safety.model_name;
    if uses_responses_api(model_name) {
        Ok(super::safety::SafetyClient::Responses(
            build_responses_client(config, model_name, None)
                .context("failed to build safety OpenAI Responses client")?,
        ))
    } else {
        Ok(super::safety::SafetyClient::Completions(
            build_completions_client(config, model_name)
                .context("failed to build safety OpenAI-compatible client")?,
        ))
    }
}

fn interactive_request_features(config: &AppConfig, model_name: &str) -> RequestFeatures {
    RequestFeatures {
        web_search: model_registry::supports_search(model_name)
            .then_some(config.tools.web_search.mode)
            .and_then(|mode| match mode {
                WebSearchMode::Disabled => None,
                WebSearchMode::Cached | WebSearchMode::Live => Some(mode),
            }),
    }
}

#[derive(Clone)]
pub struct LlmService {
    agent: AgentVariant,
    client: ClientVariant,
    model_name: String,
    reasoning: ReasoningSetting,
    interactive_features: RequestFeatures,
    pub(crate) access_mode: AccessMode,
    role: crate::agent::AgentRole,
    pub(crate) approvals: WriteApprovalController,
    pub(crate) shell_approvals: ShellApprovalController,
    pub(crate) safety: SafetyClassifier,
    pub(crate) ask_user: Option<AskUserController>,
    todo_available: bool,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) tool_names: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) preamble: String,
    interaction_scope: String,
    turn_interrupt_request: super::TurnInterruptController,
}

impl LlmService {
    pub fn from_config(
        config: &AppConfig,
        context: AgentContext,
        approvals: WriteApprovalController,
        ask_user: Option<AskUserController>,
        todo_available: bool,
        memory: Option<MemoryService>,
        subagents: Option<SubagentManager>,
        terminals: Option<BackgroundTerminalManager>,
    ) -> Result<Self> {
        let shell_approvals = ShellApprovalController::new(approvals.mode());
        Self::from_config_with_controllers(
            config,
            context,
            approvals,
            shell_approvals,
            ask_user,
            todo_available,
            memory,
            subagents,
            terminals,
        )
    }

    pub fn from_config_with_controllers(
        config: &AppConfig,
        context: AgentContext,
        approvals: WriteApprovalController,
        shell_approvals: ShellApprovalController,
        ask_user: Option<AskUserController>,
        todo_available: bool,
        memory: Option<MemoryService>,
        subagents: Option<SubagentManager>,
        terminals: Option<BackgroundTerminalManager>,
    ) -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let model_name = context
            .model_name_override
            .clone()
            .unwrap_or_else(|| config.model.model_name.clone());
        let interaction_scope = next_interaction_scope_id();
        let interactive_features = if context.role == crate::agent::AgentRole::Main {
            interactive_request_features(config, &model_name)
        } else {
            RequestFeatures::default()
        };

        let preamble = mode_preamble(&context);
        let tool_context = ToolContext {
            root: workspace_root,
            agent: context.clone(),
            config: config.clone(),
            write_approvals: approvals.clone(),
            shell_approvals: shell_approvals.clone(),
            memory,
            ask_user_available: ask_user.is_some(),
            todo_available,
            subagents,
            terminals,
        };
        let tool_names = tool_names_for_context(&tool_context);
        let (client, agent) = build_client_and_agent(
            config,
            &model_name,
            &preamble,
            config.model.reasoning,
            interactive_features,
            &interaction_scope,
            tool_context,
        )?;
        let safety_client = build_safety_client(config)?;
        let safety = SafetyClassifier::from_client(&safety_client, config);

        Ok(Self {
            agent,
            client,
            model_name,
            reasoning: config.model.reasoning,
            interactive_features,
            access_mode: context.access_mode,
            role: context.role,
            approvals,
            shell_approvals,
            safety,
            ask_user,
            todo_available,
            tool_names,
            preamble,
            interaction_scope,
            turn_interrupt_request: super::TurnInterruptController::default(),
        })
    }

    pub fn approvals(&self) -> WriteApprovalController {
        self.approvals.clone()
    }

    pub fn shell_approvals(&self) -> ShellApprovalController {
        self.shell_approvals.clone()
    }

    pub(crate) fn interaction_scope(&self) -> &str {
        &self.interaction_scope
    }

    pub fn ask_user_controller(&self) -> Option<AskUserController> {
        self.ask_user.clone()
    }

    pub fn todo_available(&self) -> bool {
        self.todo_available
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

    pub fn cancel_pending_interactions(&self) {
        self.approvals.cancel_pending();
        self.shell_approvals.cancel_pending();
        if let Some(controller) = &self.ask_user {
            controller.cancel_pending();
        }
    }

    pub fn request_turn_interrupt(&self, request: super::TurnInterruptRequest) {
        self.turn_interrupt_request.request(request);
    }

    pub fn clear_turn_interrupt_request(&self) {
        self.turn_interrupt_request.clear();
    }

    fn take_turn_interrupt_request(&self) -> Option<super::TurnInterruptRequest> {
        self.turn_interrupt_request.take()
    }

    fn search_observer_guard(
        &self,
        reply_id: u64,
        emit: EventCallback,
    ) -> Option<ResponsesSearchObserverGuard> {
        if self.interactive_features.web_search.is_none()
            || !matches!(self.client, ClientVariant::Responses(_))
        {
            return None;
        }

        let scope = self.interaction_scope.clone();
        Some(ResponsesSearchObserverGuard::register(
            scope,
            Arc::new(move |event| {
                let event = match event {
                    ResponsesHostedToolEvent::WebSearchStarted { id, detail } => {
                        StreamEvent::HostedToolStarted {
                            id,
                            kind: HostedToolKind::WebSearch,
                            detail,
                        }
                    }
                    ResponsesHostedToolEvent::WebSearchCompleted { id, detail } => {
                        StreamEvent::HostedToolCompleted {
                            id,
                            kind: HostedToolKind::WebSearch,
                            detail,
                        }
                    }
                };
                let _ = emit(reply_id, event);
            }),
        ))
    }

    pub async fn stream_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        events: UnboundedSender<RuntimeEvent>,
    ) {
        self.clear_turn_interrupt_request();
        log_debug(
            "llm_service",
            format!("stream_prompt_start reply_id={reply_id}"),
        );
        let emit: EventCallback = Arc::new(move |reply_id, event| {
            events
                .send(RuntimeEvent::MainReply { reply_id, event })
                .is_ok()
        });
        let _search_observer = self.search_observer_guard(reply_id, emit.clone());
        if let Err(error) = self
            .run_prompt(
                reply_id,
                prompt,
                history,
                history_model_name,
                stats_hook,
                None,
                emit.clone(),
            )
            .await
        {
            log_debug(
                "llm_service",
                format!("stream_prompt_error reply_id={reply_id} error={error}"),
            );
            emit_failed_reply_event(&emit, reply_id, error);
        } else {
            log_debug(
                "llm_service",
                format!("stream_prompt_done reply_id={reply_id}"),
            );
        }
    }

    pub async fn stream_side_channel(
        &self,
        prompt: String,
        reply_id: u64,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        events: UnboundedSender<RuntimeEvent>,
    ) {
        let event = match self
            .run_side_channel(reply_id, prompt, history, history_model_name, stats_hook)
            .await
        {
            Ok(output) => SideChannelEvent::Finished { output },
            Err(error) => SideChannelEvent::Failed(error.to_string()),
        };
        let _ = events.send(RuntimeEvent::SideChannel { reply_id, event });
    }

    pub async fn generate_session_title(
        &self,
        user_request: String,
        stats_hook: StatsHook,
    ) -> Result<Option<String>> {
        let prompt = format!("{SESSION_TITLE_PROMPT_PREFIX}{}", user_request.trim());
        let raw = match &self.client {
            ClientVariant::Completions(client) => {
                let agent = build_agent(
                    client,
                    &self.model_name,
                    &self.preamble,
                    self.reasoning,
                    RequestFeatures::default(),
                    None,
                );
                run_plain_prompt_with_hook(
                    &agent,
                    prompt,
                    Vec::new(),
                    stats_hook.with_model(self.model_name.clone()),
                )
                .await?
            }
            ClientVariant::Responses(client) => {
                let agent = build_agent(
                    client,
                    &self.model_name,
                    &self.preamble,
                    self.reasoning,
                    RequestFeatures::default(),
                    None,
                );
                run_plain_prompt_with_hook(
                    &agent,
                    prompt,
                    Vec::new(),
                    stats_hook.with_model(self.model_name.clone()),
                )
                .await?
            }
        };

        Ok(sanitize_session_title(&raw))
    }

    pub async fn stream_resumed_prompt(
        &self,
        reply_id: u64,
        snapshot: CompletionRequestSnapshot,
        stats_hook: StatsHook,
        events: UnboundedSender<RuntimeEvent>,
        override_action: ResumeOverride,
        replay_seed: Option<PendingReplyReplaySeed>,
    ) {
        self.clear_turn_interrupt_request();
        log_debug(
            "llm_service",
            format!("stream_resumed_prompt_start reply_id={reply_id}"),
        );
        let emit: EventCallback = Arc::new(move |reply_id, event| {
            events
                .send(RuntimeEvent::MainReply { reply_id, event })
                .is_ok()
        });
        let _search_observer = self.search_observer_guard(reply_id, emit.clone());
        if let Err(error) = self
            .run_prompt_from_state(
                reply_id,
                snapshot.prompt,
                snapshot.history,
                stats_hook,
                None,
                emit.clone(),
                Some(ResumeOverrideController::new(override_action)),
                replay_seed,
            )
            .await
        {
            log_debug(
                "llm_service",
                format!("stream_resumed_prompt_error reply_id={reply_id} error={error}"),
            );
            emit_failed_reply_event(&emit, reply_id, error);
        } else {
            log_debug(
                "llm_service",
                format!("stream_resumed_prompt_done reply_id={reply_id}"),
            );
        }
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
        let (prompt, history) = self
            .prepare_prompt_state(
                reply_id,
                prompt,
                history,
                history_model_name,
                stats_hook.clone(),
                emit.clone(),
                self.role == crate::agent::AgentRole::Main,
            )
            .await?;

        self.run_prompt_from_state(
            reply_id, prompt, history, stats_hook, capture, emit, None, None,
        )
        .await
    }

    async fn run_side_channel(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
    ) -> Result<String> {
        let emit: EventCallback = Arc::new(|_, _| true);
        let (prompt, history) = self
            .prepare_prompt_state(
                reply_id,
                prompt,
                history,
                history_model_name,
                stats_hook.clone(),
                emit.clone(),
                false,
            )
            .await?;

        let outcome = match &self.client {
            ClientVariant::Completions(client) => {
                let agent = build_agent(
                    client,
                    &self.model_name,
                    &self.preamble,
                    self.reasoning,
                    RequestFeatures::default(),
                    None,
                );
                run_prompt_step(
                    self, &agent, reply_id, prompt, history, stats_hook, None, emit, None, None, 0,
                )
                .await?
            }
            ClientVariant::Responses(client) => {
                let agent = build_agent(
                    client,
                    &self.model_name,
                    &self.preamble,
                    self.reasoning,
                    RequestFeatures::default(),
                    None,
                );
                run_prompt_step(
                    self, &agent, reply_id, prompt, history, stats_hook, None, emit, None, None, 0,
                )
                .await?
            }
        };

        match outcome {
            PromptStepOutcome::Finished(result) => Ok(result.output),
            PromptStepOutcome::Continue(_) => Err(anyhow::anyhow!(
                "Background query unexpectedly required another step."
            )),
        }
    }

    async fn prepare_prompt_state(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        emit: EventCallback,
        emit_compaction_notice: bool,
    ) -> Result<(RigMessage, Vec<RigMessage>)> {
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
                    stats_hook.with_model(compaction_model_name.clone()),
                    emit,
                    emit_compaction_notice,
                )
                .await?;
            history = history_into_rig(result.history)?;
        }

        Ok((prompt, history))
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

            let replay_seed = replay_seed.take();
            let outcome = match &self.agent {
                AgentVariant::Completions(agent) => {
                    run_prompt_step(
                        self,
                        agent,
                        reply_id,
                        prompt.clone(),
                        history.clone(),
                        stats_hook.clone(),
                        capture.clone(),
                        emit.clone(),
                        resume.clone(),
                        replay_seed.clone(),
                        MAX_TOOL_STEPS_PER_TURN,
                    )
                    .await?
                }
                AgentVariant::Responses(agent) => {
                    run_prompt_step(
                        self,
                        agent,
                        reply_id,
                        prompt.clone(),
                        history.clone(),
                        stats_hook.clone(),
                        capture.clone(),
                        emit.clone(),
                        resume.clone(),
                        replay_seed,
                        MAX_TOOL_STEPS_PER_TURN,
                    )
                    .await?
                }
            };

            match outcome {
                PromptStepOutcome::Finished(result) => {
                    return Ok(result);
                }
                PromptStepOutcome::Continue(next) => {
                    if let Some(super::TurnInterruptRequest::AtStepBoundary) =
                        self.take_turn_interrupt_request()
                    {
                        let history = history_with_prompt_from_rig(next.history, next.next_prompt)?;
                        if !(emit)(
                            reply_id,
                            StreamEvent::TurnEnded {
                                reason: TurnEndReason::InterruptedAtStepBoundary,
                                history: Some(history),
                            },
                        ) {
                            return Err(anyhow::anyhow!("event sink unavailable"));
                        }
                        return Ok(PromptRunResult {
                            output: String::new(),
                        });
                    }
                    prompt = next.next_prompt;
                    history = next.history;
                    if should_compact_before_follow_up(&self.model_name, &history, &prompt) {
                        let result = self
                            .compact_history(
                                history.clone(),
                                &self.model_name,
                                reply_id,
                                stats_hook.with_model(self.model_name.clone()),
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
        stats_hook: StatsHook,
    ) -> Result<HistoryCompactionResult> {
        let model_name = history_model_name.unwrap_or_else(|| self.model_name.clone());
        self.compact_history(
            history_into_rig(history)?,
            &model_name,
            0,
            stats_hook.with_model(model_name.clone()),
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
        stats_hook: StatsHook,
        emit: EventCallback,
        emit_notice: bool,
    ) -> Result<HistoryCompactionResult> {
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

            let summary = match &self.client {
                ClientVariant::Completions(client) => {
                    let compact_agent = build_agent(
                        client,
                        model_name,
                        &self.preamble,
                        self.reasoning,
                        RequestFeatures::default(),
                        None,
                    );
                    run_plain_prompt_with_hook(
                        &compact_agent,
                        COMPACTION_PROMPT.to_string(),
                        candidate_history.clone(),
                        stats_hook.clone(),
                    )
                    .await
                }
                ClientVariant::Responses(client) => {
                    let compact_agent = build_agent(
                        client,
                        model_name,
                        &self.preamble,
                        self.reasoning,
                        RequestFeatures::default(),
                        None,
                    );
                    run_plain_prompt_with_hook(
                        &compact_agent,
                        COMPACTION_PROMPT.to_string(),
                        candidate_history.clone(),
                        stats_hook.clone(),
                    )
                    .await
                }
            };
            let summary = match summary {
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

pub(crate) async fn run_internal_plain_prompt(
    config: &AppConfig,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    prompt: String,
    stats_hook: StatsHook,
) -> Result<String> {
    if uses_responses_api(model_name) {
        let client = build_responses_client(config, model_name, None)?;
        let agent = build_agent(
            &client,
            model_name,
            preamble,
            reasoning,
            RequestFeatures::default(),
            None,
        );
        run_plain_prompt_with_hook(&agent, prompt, Vec::new(), stats_hook).await
    } else {
        let client = build_completions_client(config, model_name)?;
        let agent = build_agent(
            &client,
            model_name,
            preamble,
            reasoning,
            RequestFeatures::default(),
            None,
        );
        run_plain_prompt_with_hook(&agent, prompt, Vec::new(), stats_hook).await
    }
}

fn next_interaction_scope_id() -> String {
    format!(
        "svc-{}",
        NEXT_INTERACTION_SCOPE_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn sanitize_session_title(raw: &str) -> Option<String> {
    let first_non_empty = raw.lines().find(|line| !line.trim().is_empty())?.trim();
    let trimmed = first_non_empty
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
        .trim_start_matches(|ch: char| matches!(ch, '-' | '*' | '•') || ch.is_whitespace())
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let words = trimmed.split_whitespace().take(6).collect::<Vec<_>>();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

fn emit_failed_reply_event(emit: &EventCallback, reply_id: u64, error: anyhow::Error) {
    let _ = emit(reply_id, StreamEvent::Failed(error.to_string()));
}

#[cfg(test)]
mod session_title_tests {
    use std::sync::{Arc, Mutex};

    use super::{emit_failed_reply_event, sanitize_session_title};
    use crate::app::StreamEvent;

    #[test]
    fn sanitize_session_title_trims_quotes_and_whitespace() {
        assert_eq!(
            sanitize_session_title("  \"Fix planning rejection flow\"  "),
            Some("Fix planning rejection flow".into())
        );
    }

    #[test]
    fn sanitize_session_title_limits_to_six_words() {
        assert_eq!(
            sanitize_session_title("One two three four five six seven"),
            Some("One two three four five six".into())
        );
    }

    #[test]
    fn emit_failed_reply_event_forwards_failure_to_event_sink() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let emit: super::EventCallback = Arc::new(move |reply_id, event| {
            captured
                .lock()
                .expect("events lock")
                .push((reply_id, event));
            true
        });

        emit_failed_reply_event(&emit, 7, anyhow::anyhow!("stream blew up"));

        let events = events.lock().expect("events lock");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, 7);
        assert!(
            matches!(&events[0].1, StreamEvent::Failed(message) if message == "stream blew up")
        );
    }
}
