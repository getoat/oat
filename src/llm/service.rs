use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use anyhow::{Context, Result, anyhow};
use rig::{
    completion::Message as RigMessage,
    providers::{anthropic, openai},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    agent::AgentContext,
    app::{
        AccessMode, HostedToolKind, PendingReplyReplaySeed, SessionHistoryMessage, SessionProfile,
        ShellApprovalDecision, SideChannelEvent, TurnEndReason, WriteApprovalDecision,
    },
    ask_user::AskUserResponse,
    background_terminals::BackgroundTerminalManager,
    completion_request::CompletionRequestSnapshot,
    config::{AppConfig, ReasoningSetting, WebSearchMode},
    debug_log::log_debug,
    history_reduction::reduce_history,
    memory::MemoryService,
    model_registry,
    runtime::RuntimeEvent,
    stats::StatsHook,
    subagents::SubagentManager,
    tools::{ToolContext, tool_names_for_context},
    web::WebService,
};

use super::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResponsesClient, ResponsesHostedToolEvent, ResponsesHostedToolKind,
    ResponsesSearchObserverGuard, ResumeOverride, StreamEvent,
    agent_builder::{
        RequestFeatures, build_agent, build_anthropic_agent, http_headers_for_model, mode_preamble,
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

// rig-core's multi_turn API requires a usize bound and internally checks
// `max_turns + 1`, so `usize::MAX` would overflow. Use the largest safe value
// to avoid imposing an application-level turn cap.
const UNBOUNDED_TOOL_STEPS_PER_TURN: usize = usize::MAX - 1;
const TOOL_STEP_LIMIT_EXCEEDED_ERROR: &str = "tool step limit exceeded";
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
    Anthropic(super::AnthropicCompletionsAgent),
    Completions(super::OpenAiCompletionsAgent),
    Responses(super::ResponsesAgent),
}

#[derive(Clone)]
enum ClientVariant {
    Anthropic(anthropic::Client),
    Completions(openai::CompletionsClient),
    Responses(ResponsesClient),
}

fn model_api_family(model_name: &str) -> Result<model_registry::ModelApiFamily> {
    model_registry::api_family_for_model(model_name).ok_or_else(|| {
        anyhow::anyhow!(model_registry::unknown_model_message(
            "model.model_name",
            model_name,
        ))
    })
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

fn build_anthropic_client(config: &AppConfig, model_name: &str) -> Result<anthropic::Client> {
    let provider_config = config.provider_config_for_model(model_name)?;
    anthropic::Client::builder()
        .api_key(provider_config.auth_token().unwrap_or_default())
        .base_url(openai_base_url_for_model(config, model_name)?)
        .http_headers(http_headers_for_model(config, model_name)?)
        .build()
        .context("failed to build Anthropic-compatible client")
}

fn build_client_variant(
    config: &AppConfig,
    model_name: &str,
    interaction_scope: Option<&str>,
) -> Result<ClientVariant> {
    match model_api_family(model_name)? {
        model_registry::ModelApiFamily::Anthropic => Ok(ClientVariant::Anthropic(
            build_anthropic_client(config, model_name)?,
        )),
        model_registry::ModelApiFamily::Completions => Ok(ClientVariant::Completions(
            build_completions_client(config, model_name)?,
        )),
        model_registry::ModelApiFamily::Responses => Ok(ClientVariant::Responses(
            build_responses_client(config, model_name, interaction_scope)?,
        )),
    }
}

fn build_agent_variant(
    client: &ClientVariant,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    features: RequestFeatures,
    tool_context: Option<ToolContext>,
) -> AgentVariant {
    match client {
        ClientVariant::Anthropic(client) => AgentVariant::Anthropic(build_anthropic_agent(
            client,
            model_name,
            preamble,
            reasoning,
            features,
            tool_context,
        )),
        ClientVariant::Completions(client) => AgentVariant::Completions(build_agent(
            client,
            model_name,
            preamble,
            reasoning,
            features,
            tool_context,
        )),
        ClientVariant::Responses(client) => AgentVariant::Responses(build_agent(
            client,
            model_name,
            preamble,
            reasoning,
            features,
            tool_context,
        )),
    }
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
    let client = build_client_variant(config, model_name, Some(interaction_scope))?;
    let agent = build_agent_variant(
        &client,
        model_name,
        preamble,
        reasoning,
        interactive_features,
        Some(tool_context),
    );
    Ok((client, agent))
}

fn build_safety_client(config: &AppConfig) -> Result<super::safety::SafetyClient> {
    let model_name = &config.safety.model_name;
    match build_client_variant(config, model_name, None)? {
        ClientVariant::Anthropic(client) => Ok(super::safety::SafetyClient::Anthropic(client)),
        ClientVariant::Completions(client) => Ok(super::safety::SafetyClient::Completions(client)),
        ClientVariant::Responses(client) => Ok(super::safety::SafetyClient::Responses(client)),
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

async fn run_plain_prompt_with_fresh_client(
    config: &AppConfig,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    prompt: String,
    history: Vec<RigMessage>,
    stats_hook: StatsHook,
) -> Result<String> {
    let client = build_client_variant(config, model_name, None)?;
    let agent = build_agent_variant(
        &client,
        model_name,
        preamble,
        reasoning,
        RequestFeatures::default(),
        None,
    );

    match agent {
        AgentVariant::Anthropic(agent) => {
            run_plain_prompt_with_hook(&agent, prompt.clone(), history.clone(), stats_hook.clone())
                .await
        }
        AgentVariant::Completions(agent) => {
            run_plain_prompt_with_hook(&agent, prompt.clone(), history.clone(), stats_hook.clone())
                .await
        }
        AgentVariant::Responses(agent) => {
            run_plain_prompt_with_hook(&agent, prompt, history, stats_hook).await
        }
    }
}

#[derive(Clone)]
pub struct LlmService {
    config: AppConfig,
    agent: AgentVariant,
    client: ClientVariant,
    model_name: String,
    reasoning: ReasoningSetting,
    interactive_features: RequestFeatures,
    pub(crate) access_mode: AccessMode,
    pub(crate) session_profile: SessionProfile,
    role: crate::agent::AgentRole,
    pub(crate) approvals: WriteApprovalController,
    pub(crate) shell_approvals: ShellApprovalController,
    pub(crate) safety: SafetyClassifier,
    pub(crate) ask_user: Option<AskUserController>,
    todo_available: bool,
    web: WebService,
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
        session_profile: SessionProfile,
        approvals: WriteApprovalController,
        ask_user: Option<AskUserController>,
        todo_available: bool,
        memory: Option<MemoryService>,
        subagents: Option<SubagentManager>,
        terminals: Option<BackgroundTerminalManager>,
        web: WebService,
    ) -> Result<Self> {
        let shell_approvals = ShellApprovalController::new(approvals.mode());
        Self::from_config_with_controllers(
            config,
            context,
            session_profile,
            approvals,
            shell_approvals,
            ask_user,
            todo_available,
            memory,
            subagents,
            terminals,
            web,
        )
    }

    pub fn from_config_with_controllers(
        config: &AppConfig,
        context: AgentContext,
        session_profile: SessionProfile,
        approvals: WriteApprovalController,
        shell_approvals: ShellApprovalController,
        ask_user: Option<AskUserController>,
        todo_available: bool,
        memory: Option<MemoryService>,
        subagents: Option<SubagentManager>,
        terminals: Option<BackgroundTerminalManager>,
        web: WebService,
    ) -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let model_name = context
            .model_name_override
            .clone()
            .unwrap_or_else(|| config.model.model_name.clone());
        let reasoning = context.reasoning_override.unwrap_or(config.model.reasoning);
        let interaction_scope = next_interaction_scope_id();
        let interactive_features = if context.role == crate::agent::AgentRole::Main {
            interactive_request_features(config, &model_name)
        } else {
            RequestFeatures::default()
        };

        let preamble = mode_preamble(&context);
        let tool_context = ToolContext {
            root: workspace_root,
            allow_full_system_access: context.allow_full_system_access,
            agent: context.clone(),
            session_profile,
            config: config.clone(),
            write_approvals: approvals.clone(),
            shell_approvals: shell_approvals.clone(),
            memory,
            ask_user_available: ask_user.is_some(),
            todo_available,
            subagents,
            terminals,
            web: web.clone(),
        };
        let tool_names = tool_names_for_context(&tool_context);
        let (client, agent) = build_client_and_agent(
            config,
            &model_name,
            &preamble,
            reasoning,
            interactive_features,
            &interaction_scope,
            tool_context,
        )?;
        let safety_client = build_safety_client(config)?;
        let safety = SafetyClassifier::from_client(&safety_client, config);

        Ok(Self {
            config: config.clone(),
            agent,
            client,
            model_name,
            reasoning,
            interactive_features,
            access_mode: context.access_mode,
            session_profile,
            role: context.role,
            approvals,
            shell_approvals,
            safety,
            ask_user,
            todo_available,
            web,
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

    pub(crate) fn model_name(&self) -> &str {
        &self.model_name
    }

    #[cfg(test)]
    pub(crate) fn tool_names(&self) -> &[String] {
        &self.tool_names
    }

    pub(crate) fn session_profile(&self) -> SessionProfile {
        self.session_profile
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

    pub(crate) fn web_service(&self) -> WebService {
        self.web.clone()
    }

    async fn run_plain_prompt_for_model(
        &self,
        model_name: &str,
        prompt: String,
        history: Vec<RigMessage>,
        stats_hook: StatsHook,
    ) -> Result<String> {
        run_plain_prompt_with_fresh_client(
            &self.config,
            model_name,
            &self.preamble,
            self.reasoning,
            prompt,
            history,
            stats_hook,
        )
        .await
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
                    ResponsesHostedToolEvent::WebSearchStarted { id, kind, detail } => {
                        StreamEvent::HostedToolStarted {
                            id,
                            kind: match kind {
                                ResponsesHostedToolKind::Search => HostedToolKind::Search,
                                ResponsesHostedToolKind::OpenPage => HostedToolKind::OpenPage,
                                ResponsesHostedToolKind::FindInPage => HostedToolKind::FindInPage,
                            },
                            detail,
                        }
                    }
                    ResponsesHostedToolEvent::WebSearchCompleted { id, kind, detail } => {
                        StreamEvent::HostedToolCompleted {
                            id,
                            kind: match kind {
                                ResponsesHostedToolKind::Search => HostedToolKind::Search,
                                ResponsesHostedToolKind::OpenPage => HostedToolKind::OpenPage,
                                ResponsesHostedToolKind::FindInPage => HostedToolKind::FindInPage,
                            },
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
        let capture = CompletionCapture::new();
        let _search_observer = self.search_observer_guard(reply_id, emit.clone());
        if let Err(error) = self
            .run_prompt(
                reply_id,
                prompt,
                history,
                history_model_name,
                stats_hook,
                Some(capture),
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
        let raw = self
            .run_plain_prompt_for_model(
                &self.model_name,
                prompt,
                Vec::new(),
                stats_hook.with_model(self.model_name.clone()),
            )
            .await?;

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
        let capture = CompletionCapture::new();
        let _search_observer = self.search_observer_guard(reply_id, emit.clone());
        if let Err(error) = self
            .run_prompt_from_state(
                reply_id,
                snapshot.prompt,
                snapshot.history,
                stats_hook,
                Some(capture),
                emit.clone(),
                Some(ResumeOverrideController::new(override_action)),
                replay_seed,
                UNBOUNDED_TOOL_STEPS_PER_TURN,
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
            reply_id,
            prompt,
            history,
            stats_hook,
            capture,
            emit,
            None,
            None,
            UNBOUNDED_TOOL_STEPS_PER_TURN,
        )
        .await
    }

    pub async fn run_prompt_with_tool_step_limit(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
        max_tool_steps: usize,
    ) -> Result<PromptRunResult> {
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

        self.run_prompt_from_state(
            reply_id,
            prompt,
            history,
            stats_hook,
            capture,
            emit,
            None,
            None,
            max_tool_steps,
        )
        .await
    }

    pub(crate) fn is_tool_step_limit_error(error: &anyhow::Error) -> bool {
        error.to_string() == TOOL_STEP_LIMIT_EXCEEDED_ERROR
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
            ClientVariant::Anthropic(client) => {
                let agent = build_anthropic_agent(
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
        max_tool_steps: usize,
    ) -> Result<PromptRunResult> {
        loop {
            let replay_seed = replay_seed.take();
            let outcome = match &self.agent {
                AgentVariant::Anthropic(agent) => {
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
                        max_tool_steps,
                    )
                    .await?
                }
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
                        max_tool_steps,
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
                        max_tool_steps,
                    )
                    .await?
                }
            };

            match outcome {
                PromptStepOutcome::Finished(result) => {
                    return Ok(result);
                }
                PromptStepOutcome::Continue(next) => {
                    if max_tool_steps != UNBOUNDED_TOOL_STEPS_PER_TURN {
                        return Err(anyhow!(TOOL_STEP_LIMIT_EXCEEDED_ERROR));
                    }
                    let reduced_history = reduce_history(
                        &next.history,
                        self.config.history.mode,
                        self.config.history.retained_steps,
                        false,
                    );
                    if let Some(super::TurnInterruptRequest::AtStepBoundary) =
                        self.take_turn_interrupt_request()
                    {
                        let history =
                            history_with_prompt_from_rig(reduced_history, next.next_prompt)?;
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
                    history = reduced_history;
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
        let mut candidate_history = reduce_history(
            &history,
            self.config.history.mode,
            self.config.history.retained_steps,
            true,
        );

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

            let summary = self
                .run_plain_prompt_for_model(
                    model_name,
                    COMPACTION_PROMPT.to_string(),
                    candidate_history.clone(),
                    stats_hook.clone(),
                )
                .await;
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
    run_plain_prompt_with_fresh_client(
        config,
        model_name,
        preamble,
        reasoning,
        prompt,
        Vec::new(),
        stats_hook,
    )
    .await
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
