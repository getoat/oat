use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use globset::Glob;
use rig::{
    agent::{HookAction, MultiTurnStreamItem, PromptHook, StreamingError, ToolCallHookAction},
    client::CompletionClient,
    completion::{
        CompletionModel, Message as RigMessage, PromptError, TypedPrompt,
        message::{AssistantContent, ToolResult, ToolResultContent, UserContent},
    },
    providers::openai,
    streaming::{
        StreamedAssistantContent, StreamedUserContent, StreamingChat, ToolCallDeltaContent,
    },
    tool::Tool,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    agent::{AgentContext, AgentRole},
    app::{
        AccessMode, ApprovalMode, CommandRisk, PendingReplyReplaySeed, ShellApprovalDecision,
        WriteApprovalDecision,
    },
    ask_user::{AskUserRequest, AskUserResponse, validate_request},
    completion_request::{
        CompletionRequestSnapshot, estimated_history_context_tokens, estimated_message_tokens,
    },
    config::{AppConfig, ReasoningEffort},
    model_registry,
    stats::StatsHook,
    subagents::SubagentManager,
    tools::{
        AskUserTool, CommentaryArgs, CommentaryTool, RUN_SHELL_SCRIPT_TOOL_NAME,
        RunShellScriptArgs, ToolContext, display_requested_shell_cwd, display_shell_command,
        is_mutation_tool, tool_names_for_context, tools_for_context,
    },
};

const MAX_TOOL_STEPS_PER_TURN: usize = 64;
const SYSTEM_PROMPT: &str = include_str!("../prompts/system.md");
const COMPACTION_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n- Decision complete plan, if using\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";
const COMPACTION_SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. You have access to the state of the last few tools that were used by that language model, and the last few tokens of user messages to contextualise. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model; use the information in the summary to assist with your own analysis:\n";
const COMPACTION_USER_TOKEN_BUDGET: usize = 10_000;
const COMPACTION_TOOL_TOKEN_BUDGET: usize = 10_000;
const STEP_BOUNDARY_REASON: &str = "__oat_step_boundary__";
const COMPACTION_NOTICE: &str = "Context compacted.";

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    Commentary(String),
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        output: String,
    },
    AskUserRequested {
        request_id: String,
        request: AskUserRequest,
    },
    WriteApprovalRequested {
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    ShellApprovalRequested {
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    },
    PlanningFinalizationStarted,
    CompactionFinished {
        history: Vec<RigMessage>,
        model_name: String,
    },
    Finished {
        history: Option<Vec<RigMessage>>,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOverride {
    WriteApproval {
        tool_name: String,
        arguments: String,
        decision: WriteApprovalDecision,
    },
    ShellApproval {
        risk: CommandRisk,
        command: String,
        working_directory: String,
        decision: ShellApprovalDecision,
    },
    AskUser {
        request: AskUserRequest,
        response: AskUserResponse,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeRequest {
    pub snapshot: CompletionRequestSnapshot,
    pub override_action: ResumeOverride,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InteractionResolveResult {
    Resolved,
    Resume(ResumeRequest),
    Missing,
}

type LlmAgent = rig::agent::Agent<openai::CompletionModel>;
pub type EventCallback = Arc<dyn Fn(u64, StreamEvent) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct WriteApprovalController {
    inner: Arc<Mutex<WriteApprovalState>>,
}

struct WriteApprovalState {
    default_mode: ApprovalMode,
    mode: ApprovalMode,
    pending: HashMap<String, PendingWriteApprovalEntry>,
}

struct PendingWriteApprovalEntry {
    sender: oneshot::Sender<WriteApprovalDecision>,
    snapshot: Option<CompletionRequestSnapshot>,
    tool_name: String,
    arguments: String,
}

#[derive(Clone)]
pub struct ShellApprovalController {
    inner: Arc<Mutex<ShellApprovalState>>,
}

struct ShellApprovalState {
    default_mode: ApprovalMode,
    low: ShellRiskApprovalBucket,
    medium: ShellRiskApprovalBucket,
    high: ShellRiskApprovalBucket,
    pending: HashMap<String, PendingShellApprovalEntry>,
}

struct PendingShellApprovalEntry {
    risk: CommandRisk,
    sender: oneshot::Sender<ShellApprovalDecision>,
    snapshot: Option<CompletionRequestSnapshot>,
    command: String,
    working_directory: String,
}

#[derive(Clone)]
struct ShellRiskApprovalBucket {
    mode: ApprovalMode,
    patterns: Vec<String>,
}

impl ShellApprovalState {
    fn bucket_mut(&mut self, risk: CommandRisk) -> &mut ShellRiskApprovalBucket {
        match risk {
            CommandRisk::Low => &mut self.low,
            CommandRisk::Medium => &mut self.medium,
            CommandRisk::High => &mut self.high,
        }
    }
}

#[derive(Clone)]
pub struct AskUserController {
    inner: Arc<Mutex<AskUserState>>,
}

struct AskUserState {
    pending: HashMap<String, PendingAskUserEntry>,
}

struct PendingAskUserEntry {
    sender: oneshot::Sender<AskUserResponse>,
    snapshot: Option<CompletionRequestSnapshot>,
    request: AskUserRequest,
}

#[derive(Clone)]
struct ShellApprovalHook {
    reply_id: u64,
    emit: EventCallback,
    access_mode: AccessMode,
    approvals: ShellApprovalController,
    safety: SafetyClassifier,
    capture: Option<CompletionCapture>,
    resume: Option<ResumeOverrideController>,
}

#[derive(Clone)]
struct WriteApprovalHook {
    reply_id: u64,
    emit: EventCallback,
    approvals: WriteApprovalController,
    capture: Option<CompletionCapture>,
    resume: Option<ResumeOverrideController>,
}

#[derive(Clone)]
struct AskUserHook {
    reply_id: u64,
    emit: EventCallback,
    controller: Option<AskUserController>,
    capture: Option<CompletionCapture>,
    resume: Option<ResumeOverrideController>,
}

#[derive(Clone, Default)]
struct CompletionCaptureHook {
    capture: Option<CompletionCapture>,
}

#[derive(Default)]
struct PartialToolCall {
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayProbe {
    expected: String,
    buffered: String,
}

#[derive(Clone, Default)]
struct ResumeOverrideController {
    inner: Arc<Mutex<Option<ResumeOverrideState>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeOverrideState {
    override_action: ResumeOverride,
    tool_call_suppressed: bool,
}

impl ReplayProbe {
    fn new(expected: &str) -> Self {
        Self {
            expected: expected.to_string(),
            buffered: String::new(),
        }
    }
}

impl ResumeOverrideController {
    fn new(override_action: ResumeOverride) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(ResumeOverrideState {
                override_action,
                tool_call_suppressed: false,
            }))),
        }
    }

    fn consume_write(&self, tool_name: &str, arguments: &str) -> Option<WriteApprovalDecision> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::WriteApproval {
            tool_name: expected_tool_name,
            arguments: expected_arguments,
            decision: _,
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if expected_tool_name != tool_name || expected_arguments != arguments {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::WriteApproval { decision, .. } = state.override_action else {
            unreachable!("matched write override");
        };
        Some(decision)
    }

    fn consume_shell(
        &self,
        risk: CommandRisk,
        command: &str,
        working_directory: &str,
    ) -> Option<ShellApprovalDecision> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::ShellApproval {
            risk: expected_risk,
            command: expected_command,
            working_directory: expected_working_directory,
            ..
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if *expected_risk != risk
            || expected_command != command
            || expected_working_directory != working_directory
        {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::ShellApproval { decision, .. } = state.override_action else {
            unreachable!("matched shell override");
        };
        Some(decision)
    }

    fn consume_ask_user(&self, request: &AskUserRequest) -> Option<AskUserResponse> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::AskUser {
            request: expected_request,
            ..
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if expected_request != request {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::AskUser { response, .. } = state.override_action else {
            unreachable!("matched ask user override");
        };
        Some(response)
    }

    fn suppress_matching_tool_call(&self, name: &str, arguments: &str) -> bool {
        let mut state = self.inner.lock().expect("resume override lock");
        let Some(state) = state.as_mut() else {
            return false;
        };
        if state.tool_call_suppressed {
            return false;
        }
        if !resume_override_matches_tool_call(&state.override_action, name, arguments) {
            return false;
        }
        state.tool_call_suppressed = true;
        true
    }
}

#[derive(Clone)]
struct CombinedHook<H1, H2> {
    first: H1,
    second: H2,
}

#[derive(Clone)]
struct StepBoundaryHook {
    capture: StepBoundaryCapture,
}

#[derive(Clone)]
struct SafetyClassifier {
    agent: LlmAgent,
}

#[derive(Clone)]
struct SafetyClassification {
    risk: CommandRisk,
    risk_explanation: String,
    reason: String,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
enum SafetyClassifierRiskOutput {
    Low,
    Medium,
    High,
}

impl From<SafetyClassifierRiskOutput> for CommandRisk {
    fn from(value: SafetyClassifierRiskOutput) -> Self {
        match value {
            SafetyClassifierRiskOutput::Low => Self::Low,
            SafetyClassifierRiskOutput::Medium => Self::Medium,
            SafetyClassifierRiskOutput::High => Self::High,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SafetyClassifierOutput {
    risk: SafetyClassifierRiskOutput,
    explanation: String,
}

#[derive(Clone)]
pub struct LlmService {
    agent: LlmAgent,
    client: openai::CompletionsClient,
    model_name: String,
    reasoning_effort: ReasoningEffort,
    access_mode: AccessMode,
    role: AgentRole,
    approvals: WriteApprovalController,
    shell_approvals: ShellApprovalController,
    safety: SafetyClassifier,
    ask_user: Option<AskUserController>,
    #[cfg_attr(not(test), allow(dead_code))]
    tool_names: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    preamble: String,
}

pub struct PromptRunResult {
    pub output: String,
    pub history: Option<Vec<RigMessage>>,
}

#[derive(Clone, Debug)]
pub struct HistoryCompactionResult {
    pub history: Vec<RigMessage>,
    pub model_name: String,
}

#[derive(Clone, Default)]
pub struct CompletionCapture {
    inner: Arc<Mutex<Option<CompletionRequestSnapshot>>>,
}

#[derive(Clone, Default)]
struct StepBoundaryCapture {
    inner: Arc<Mutex<Option<StepBoundaryState>>>,
}

#[derive(Clone)]
struct StepBoundaryState {
    next_prompt: RigMessage,
    history: Vec<RigMessage>,
}

enum PromptStepOutcome {
    Finished(PromptRunResult),
    Continue(StepBoundaryState),
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
            approvals: approvals.clone(),
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
        history: Vec<RigMessage>,
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
        history: Vec<RigMessage>,
        history_model_name: Option<String>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
    ) -> Result<PromptRunResult> {
        let prompt = RigMessage::user(prompt);
        let mut history = history;

        if let Some(compaction_model_name) =
            self.compaction_model_for_pre_turn(&history, history_model_name.as_deref(), &prompt)
        {
            let result = self
                .compact_history(
                    history.clone(),
                    &compaction_model_name,
                    reply_id,
                    emit.clone(),
                    self.role == AgentRole::Main,
                )
                .await?;
            history = result.history;
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

            match self
                .run_prompt_step(
                    reply_id,
                    prompt,
                    history,
                    stats_hook.clone(),
                    capture.clone(),
                    emit.clone(),
                    resume.clone(),
                    replay_seed.take(),
                )
                .await?
            {
                PromptStepOutcome::Finished(result) => {
                    return Ok(result);
                }
                PromptStepOutcome::Continue(next) => {
                    prompt = next.next_prompt;
                    history = next.history;
                    if self.should_compact_before_follow_up(&history, &prompt) {
                        let result = self
                            .compact_history(
                                history.clone(),
                                &self.model_name,
                                reply_id,
                                emit.clone(),
                                self.role == AgentRole::Main,
                            )
                            .await?;
                        history = result.history;
                    }
                }
            }
        }
    }

    pub async fn compact_history_for_session(
        &self,
        history: Vec<RigMessage>,
        history_model_name: Option<String>,
    ) -> Result<HistoryCompactionResult> {
        let model_name = history_model_name.unwrap_or_else(|| self.model_name.clone());
        self.compact_history(history, &model_name, 0, Arc::new(|_, _| true), false)
            .await
    }

    fn compaction_model_for_pre_turn(
        &self,
        history: &[RigMessage],
        history_model_name: Option<&str>,
        prompt: &RigMessage,
    ) -> Option<String> {
        if !self.should_compact_request_for_model(&self.model_name, history, prompt) {
            return None;
        }

        let Some(previous_model_name) = history_model_name else {
            return Some(self.model_name.clone());
        };
        let Some(previous_model) = model_registry::find_model(previous_model_name) else {
            return Some(self.model_name.clone());
        };
        let Some(current_model) = model_registry::find_model(&self.model_name) else {
            return Some(self.model_name.clone());
        };
        if previous_model.context_length > current_model.context_length
            && self.should_compact_request_for_model(
                &self.model_name,
                history,
                &RigMessage::user(""),
            )
        {
            Some(previous_model_name.to_string())
        } else {
            Some(self.model_name.clone())
        }
    }

    fn should_compact_before_follow_up(&self, history: &[RigMessage], prompt: &RigMessage) -> bool {
        self.should_compact_request_for_model(&self.model_name, history, prompt)
    }

    fn should_compact_request_for_model(
        &self,
        model_name: &str,
        history: &[RigMessage],
        prompt: &RigMessage,
    ) -> bool {
        model_registry::find_model(model_name).is_some_and(|model| {
            model.should_compact_for_input_tokens(estimated_request_tokens(history, prompt))
        })
    }

    async fn run_prompt_step(
        &self,
        reply_id: u64,
        prompt: RigMessage,
        history: Vec<RigMessage>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
        resume: Option<ResumeOverrideController>,
        replay_seed: Option<PendingReplyReplaySeed>,
    ) -> Result<PromptStepOutcome> {
        let write_approval_hook = WriteApprovalHook {
            reply_id,
            emit: emit.clone(),
            approvals: self.approvals.clone(),
            capture: capture.clone(),
            resume: resume.clone(),
        };
        let shell_approval_hook = ShellApprovalHook {
            reply_id,
            emit: emit.clone(),
            access_mode: self.access_mode,
            approvals: self.shell_approvals.clone(),
            safety: self.safety.clone(),
            capture: capture.clone(),
            resume: resume.clone(),
        };
        let ask_user_hook = AskUserHook {
            reply_id,
            emit: emit.clone(),
            controller: self.ask_user.clone(),
            capture: capture.clone(),
            resume: resume.clone(),
        };
        let step_boundary = StepBoundaryCapture::default();
        let hook = CombinedHook {
            first: StepBoundaryHook {
                capture: step_boundary.clone(),
            },
            second: CombinedHook {
                first: stats_hook,
                second: CombinedHook {
                    first: CompletionCaptureHook { capture },
                    second: CombinedHook {
                        first: shell_approval_hook,
                        second: CombinedHook {
                            first: write_approval_hook,
                            second: ask_user_hook,
                        },
                    },
                },
            },
        };
        let mut stream = self
            .agent
            .stream_chat(prompt, history)
            .with_hook(hook)
            .multi_turn(MAX_TOOL_STEPS_PER_TURN)
            .await;
        let mut tool_calls = HashMap::<String, String>::new();
        let mut commentary_calls = HashSet::<String>::new();
        let mut partial_tool_calls = HashMap::<String, PartialToolCall>::new();
        let mut output = String::new();
        let mut reasoning_output = String::new();
        let mut plain_replay_probe = replay_seed
            .as_ref()
            .map(|seed| seed.plain_text.clone())
            .filter(|text| !text.is_empty())
            .map(|text| ReplayProbe::new(&text));
        let mut reasoning_replay_probe = replay_seed
            .as_ref()
            .map(|seed| seed.reasoning_text.clone())
            .filter(|text| !text.is_empty())
            .map(|text| ReplayProbe::new(&text));
        let mut commentary_replay_messages = replay_seed
            .map(|seed| seed.commentary_messages.into())
            .filter(|messages: &VecDeque<String>| !messages.is_empty());

        while let Some(chunk) = stream.next().await {
            let event = match chunk {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => {
                    let delta = reconcile_stream_text(&text.text, &mut plain_replay_probe);
                    if delta.is_empty() {
                        None
                    } else {
                        output.push_str(&delta);
                        Some(StreamEvent::TextDelta(delta))
                    }
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::Reasoning(reasoning),
                )) => {
                    plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    let delta = reconcile_stream_text(
                        &reasoning.display_text(),
                        &mut reasoning_replay_probe,
                    );
                    if delta.is_empty() {
                        None
                    } else {
                        reasoning_output.push_str(&delta);
                        Some(StreamEvent::ReasoningDelta(delta))
                    }
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                )) => {
                    plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    let delta = reconcile_stream_text(&reasoning, &mut reasoning_replay_probe);
                    if delta.is_empty() {
                        None
                    } else {
                        reasoning_output.push_str(&delta);
                        Some(StreamEvent::ReasoningDelta(delta))
                    }
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ToolCallDelta {
                        internal_call_id,
                        content,
                        ..
                    },
                )) => {
                    let partial = partial_tool_calls.entry(internal_call_id).or_default();
                    match content {
                        ToolCallDeltaContent::Name(name) => {
                            partial.name = Some(name);
                        }
                        ToolCallDeltaContent::Delta(delta) => {
                            partial.arguments.push_str(&delta);
                        }
                    }
                    None
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    },
                )) => {
                    plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    reasoning_replay_probe =
                        (!reasoning_output.is_empty()).then(|| ReplayProbe::new(&reasoning_output));
                    let name = tool_call.function.name.clone();
                    let fallback_arguments = format_tool_arguments(&tool_call.function.arguments);
                    tool_calls.insert(internal_call_id.clone(), name.clone());
                    if name == AskUserTool::NAME {
                        partial_tool_calls.remove(&internal_call_id);
                        None
                    } else if name == CommentaryTool::NAME {
                        match resolve_commentary_message(
                            &mut partial_tool_calls,
                            &internal_call_id,
                            &fallback_arguments,
                        ) {
                            Ok(message) => {
                                commentary_calls.insert(internal_call_id);
                                if commentary_replay_messages.as_ref().is_some_and(|messages| {
                                    messages
                                        .front()
                                        .is_some_and(|expected| expected == &message)
                                }) {
                                    if let Some(messages) = commentary_replay_messages.as_mut() {
                                        messages.pop_front();
                                        if messages.is_empty() {
                                            commentary_replay_messages = None;
                                        }
                                    }
                                    None
                                } else {
                                    commentary_replay_messages = None;
                                    Some(StreamEvent::Commentary(message))
                                }
                            }
                            Err(_) => Some(StreamEvent::ToolCall {
                                name,
                                arguments: fallback_arguments,
                            }),
                        }
                    } else if resume.as_ref().is_some_and(|resume| {
                        resume.suppress_matching_tool_call(&name, &fallback_arguments)
                    }) {
                        partial_tool_calls.remove(&internal_call_id);
                        None
                    } else {
                        partial_tool_calls.remove(&internal_call_id);
                        Some(StreamEvent::ToolCall {
                            name,
                            arguments: fallback_arguments,
                        })
                    }
                }
                Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                    tool_result,
                    internal_call_id,
                })) => {
                    plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    reasoning_replay_probe =
                        (!reasoning_output.is_empty()).then(|| ReplayProbe::new(&reasoning_output));
                    let name = tool_calls
                        .get(&internal_call_id)
                        .cloned()
                        .unwrap_or_else(|| tool_result.id.clone());
                    if name == AskUserTool::NAME {
                        None
                    } else if commentary_calls.contains(&internal_call_id) {
                        None
                    } else {
                        Some(StreamEvent::ToolResult {
                            name,
                            output: format_tool_result(&tool_result),
                        })
                    }
                }
                Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                    let history = response.history().map(ToOwned::to_owned);
                    let event = StreamEvent::Finished {
                        history: history.clone(),
                    };
                    if !(emit)(reply_id, event) {
                        return Err(anyhow::anyhow!("event sink unavailable"));
                    }
                    return Ok(PromptStepOutcome::Finished(PromptRunResult {
                        output,
                        history,
                    }));
                }
                Ok(_) => None,
                Err(error) => {
                    if let Some(boundary) = step_boundary.take()
                        && is_step_boundary_error(&error)
                    {
                        return Ok(PromptStepOutcome::Continue(boundary));
                    }
                    let message = error.to_string();
                    let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
                    return Err(error.into());
                }
            };

            if let Some(event) = event
                && !(emit)(reply_id, event)
            {
                return Err(anyhow::anyhow!("event sink unavailable"));
            }
        }

        let message = "Request ended before response completed.".to_string();
        let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
        Err(anyhow::anyhow!(message))
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
            let request_tokens =
                estimated_request_tokens(&candidate_history, &RigMessage::user(COMPACTION_PROMPT));
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
                history: rebuilt,
                model_name: model_name.to_string(),
            });
        }
    }
}

impl StepBoundaryCapture {
    fn set(&self, next_prompt: &RigMessage, history: &[RigMessage]) {
        let mut slot = self.inner.lock().expect("step boundary lock");
        *slot = Some(StepBoundaryState {
            next_prompt: next_prompt.clone(),
            history: history.to_vec(),
        });
    }

    fn take(&self) -> Option<StepBoundaryState> {
        self.inner.lock().expect("step boundary lock").take()
    }
}

impl<M> PromptHook<M> for StepBoundaryHook
where
    M: CompletionModel,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        if self.capture.take().is_some() {
            self.capture.set(prompt, history);
            HookAction::Terminate {
                reason: STEP_BOUNDARY_REASON.to_string(),
            }
        } else {
            self.capture.set(prompt, history);
            HookAction::Continue
        }
    }
}

impl CompletionCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Option<CompletionRequestSnapshot> {
        self.inner.lock().expect("completion capture lock").clone()
    }

    fn record(&self, prompt: &RigMessage, history: &[RigMessage]) {
        let mut snapshot = self.inner.lock().expect("completion capture lock");
        *snapshot = Some(CompletionRequestSnapshot::capture(prompt, history));
    }
}

impl Default for WriteApprovalController {
    fn default() -> Self {
        Self::new(ApprovalMode::Manual)
    }
}

impl Default for ShellApprovalController {
    fn default() -> Self {
        Self::new(ApprovalMode::Manual)
    }
}

impl Default for AskUserController {
    fn default() -> Self {
        Self::new()
    }
}

impl SafetyClassifier {
    fn from_client(client: &openai::CompletionsClient, config: &AppConfig) -> Self {
        let agent = client
            .agent(config.safety.model_name.clone())
            .preamble(safety_classifier_preamble())
            .additional_params(reasoning_params(config.safety.reasoning_effort))
            .build();
        Self { agent }
    }

    async fn classify(
        &self,
        access_mode: AccessMode,
        args: &RunShellScriptArgs,
    ) -> SafetyClassification {
        let command = display_shell_command(&args.script);
        let heuristic = minimum_shell_risk(&command, &args.script);
        let reason = normalize_summary(&args.intent);
        let working_directory = display_requested_shell_cwd(args.cwd.as_deref());
        let prompt =
            safety_classifier_prompt(access_mode, &command, &working_directory, args, heuristic);
        let model_output = self
            .agent
            .prompt_typed::<SafetyClassifierOutput>(prompt)
            .await
            .ok();
        let model_risk = model_output
            .as_ref()
            .map(|output| CommandRisk::from(output.risk))
            .unwrap_or(CommandRisk::High);
        let risk = max_command_risk(model_risk, heuristic.unwrap_or(CommandRisk::Low));
        let risk_explanation = match model_output {
            Some(output) if risk == model_risk => output.explanation,
            Some(_) if risk != model_risk => {
                "Local safety heuristic raised the final risk above the model response.".into()
            }
            _ => "Safety classifier did not provide a usable explanation.".into(),
        };

        SafetyClassification {
            risk,
            risk_explanation,
            reason,
        }
    }
}

impl WriteApprovalController {
    pub fn new(mode: ApprovalMode) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WriteApprovalState {
                default_mode: mode,
                mode,
                pending: HashMap::new(),
            })),
        }
    }

    pub fn mode(&self) -> ApprovalMode {
        let state = self.inner.lock().expect("approval state lock");
        state.mode
    }

    fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("approval state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_approval(
        &self,
        reply_id: u64,
        tool_name: &str,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        if let Some(decision) = resume.and_then(|resume| resume.consume_write(tool_name, args)) {
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                let mut state = self.inner.lock().expect("approval state lock");
                state.mode = ApprovalMode::Disabled;
            }
            return match decision {
                WriteApprovalDecision::AllowOnce | WriteApprovalDecision::AllowAllSession => {
                    ToolCallHookAction::Continue
                }
                WriteApprovalDecision::Deny => {
                    ToolCallHookAction::skip("Write action denied by user.")
                }
            };
        }

        let rx = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(state.mode, ApprovalMode::Disabled) {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingWriteApprovalEntry {
                    sender: tx,
                    snapshot,
                    tool_name: tool_name.to_string(),
                    arguments: args.to_string(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            StreamEvent::WriteApprovalRequested {
                request_id: internal_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: args.to_string(),
            },
        ) {
            let mut state = self.inner.lock().expect("approval state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "Write action cancelled because approval UI is unavailable.",
            );
        }

        match rx.await {
            Ok(WriteApprovalDecision::AllowOnce | WriteApprovalDecision::AllowAllSession) => {
                ToolCallHookAction::Continue
            }
            Ok(WriteApprovalDecision::Deny) => {
                ToolCallHookAction::skip("Write action denied by user.")
            }
            Err(_) => ToolCallHookAction::skip("Write action cancelled before approval."),
        }
    }

    fn resolve(
        &self,
        request_id: &str,
        decision: WriteApprovalDecision,
    ) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                state.mode = ApprovalMode::Disabled;
            }
            state.pending.remove(request_id)
        };

        let Some(pending) = pending else {
            return InteractionResolveResult::Missing;
        };

        if pending.sender.send(decision).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = pending.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::WriteApproval {
                    tool_name: pending.tool_name,
                    arguments: pending.arguments,
                    decision,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    fn reset_session(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.mode = state.default_mode;
        for (_, pending) in state.pending.drain() {
            let _ = pending.sender.send(WriteApprovalDecision::Deny);
        }
    }

    fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.pending.clear();
    }
}

impl ShellApprovalController {
    pub fn new(mode: ApprovalMode) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ShellApprovalState {
                default_mode: mode,
                low: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                medium: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                high: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                pending: HashMap::new(),
            })),
        }
    }

    fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("shell approval state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_approval(
        &self,
        reply_id: u64,
        access_mode: AccessMode,
        internal_call_id: &str,
        args: &RunShellScriptArgs,
        emit: &EventCallback,
        safety: &SafetyClassifier,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        let classification = safety.classify(access_mode, args).await;
        let command = display_shell_command(&args.script);
        let working_directory = display_requested_shell_cwd(args.cwd.as_deref());
        if access_mode == AccessMode::ReadOnly && classification.risk != CommandRisk::Low {
            return ToolCallHookAction::skip(format!(
                "{} risk shell commands require write mode. Switch to write mode before retrying.\nWorking directory: {}\nCommand: {}",
                classification.risk.label(),
                working_directory,
                command
            ));
        }

        if let Some(decision) = resume.and_then(|resume| {
            resume.consume_shell(classification.risk, &command, &working_directory)
        }) {
            {
                let mut state = self.inner.lock().expect("shell approval state lock");
                let bucket = state.bucket_mut(classification.risk);
                match &decision {
                    ShellApprovalDecision::AllowPattern(pattern) => {
                        if !bucket.patterns.iter().any(|existing| existing == pattern) {
                            bucket.patterns.push(pattern.clone());
                        }
                    }
                    ShellApprovalDecision::AllowAllRisk => {
                        bucket.mode = ApprovalMode::Disabled;
                    }
                    ShellApprovalDecision::AllowOnce | ShellApprovalDecision::Deny(_) => {}
                }
            }
            return match decision {
                ShellApprovalDecision::AllowOnce
                | ShellApprovalDecision::AllowPattern(_)
                | ShellApprovalDecision::AllowAllRisk => ToolCallHookAction::Continue,
                ShellApprovalDecision::Deny(note) => ToolCallHookAction::skip(
                    note.unwrap_or_else(|| "Shell command denied by user.".into()),
                ),
            };
        }

        let rx = {
            let mut state = self.inner.lock().expect("shell approval state lock");
            let bucket = state.bucket_mut(classification.risk);
            if matches!(bucket.mode, ApprovalMode::Disabled)
                || bucket
                    .patterns
                    .iter()
                    .any(|pattern| shell_pattern_matches(pattern, &command))
            {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingShellApprovalEntry {
                    risk: classification.risk,
                    sender: tx,
                    snapshot,
                    command: command.clone(),
                    working_directory: working_directory.clone(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            StreamEvent::ShellApprovalRequested {
                request_id: internal_call_id.to_string(),
                risk: classification.risk,
                risk_explanation: classification.risk_explanation.clone(),
                command: command.clone(),
                working_directory: working_directory.clone(),
                reason: classification.reason.clone(),
            },
        ) {
            let mut state = self.inner.lock().expect("shell approval state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "Shell command cancelled because approval UI is unavailable.",
            );
        }

        match rx.await {
            Ok(ShellApprovalDecision::AllowOnce) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::AllowPattern(_)) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::AllowAllRisk) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::Deny(note)) => ToolCallHookAction::skip(
                note.unwrap_or_else(|| "Shell command denied by user.".into()),
            ),
            Err(_) => ToolCallHookAction::skip("Shell command cancelled before approval."),
        }
    }

    fn resolve(
        &self,
        request_id: &str,
        decision: ShellApprovalDecision,
    ) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("shell approval state lock");
            let pending = state.pending.remove(request_id);
            if let Some(entry) = pending.as_ref() {
                let bucket = state.bucket_mut(entry.risk);
                match &decision {
                    ShellApprovalDecision::AllowPattern(pattern) => {
                        if !bucket.patterns.iter().any(|existing| existing == pattern) {
                            bucket.patterns.push(pattern.clone());
                        }
                    }
                    ShellApprovalDecision::AllowAllRisk => {
                        bucket.mode = ApprovalMode::Disabled;
                    }
                    ShellApprovalDecision::AllowOnce | ShellApprovalDecision::Deny(_) => {}
                }
            }
            pending
        };

        let Some(entry) = pending else {
            return InteractionResolveResult::Missing;
        };

        if entry.sender.send(decision.clone()).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = entry.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::ShellApproval {
                    risk: entry.risk,
                    command: entry.command,
                    working_directory: entry.working_directory,
                    decision,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    fn reset_session(&self) {
        let mut state = self.inner.lock().expect("shell approval state lock");
        state.low = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        state.medium = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        state.high = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        for (_, entry) in state.pending.drain() {
            let _ = entry.sender.send(ShellApprovalDecision::Deny(None));
        }
    }

    fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("shell approval state lock");
        state.pending.clear();
    }
}

impl AskUserController {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AskUserState {
                pending: HashMap::new(),
            })),
        }
    }

    fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("ask user state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_input(
        &self,
        reply_id: u64,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        let request = match serde_json::from_str::<AskUserRequest>(args) {
            Ok(request) => request,
            Err(error) => {
                return ToolCallHookAction::skip(format!(
                    "AskUser request was invalid JSON: {error}"
                ));
            }
        };
        if let Err(error) = validate_request(&request) {
            return ToolCallHookAction::skip(format!("AskUser validation error: {error}"));
        }

        if let Some(response) = resume.and_then(|resume| resume.consume_ask_user(&request)) {
            return ToolCallHookAction::skip(serde_json::to_string(&response).unwrap_or_else(
                |_| "{\"questions\":[],\"error\":\"failed to serialize AskUser response\"}".into(),
            ));
        }

        let rx = {
            let mut state = self.inner.lock().expect("ask user state lock");
            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingAskUserEntry {
                    sender: tx,
                    snapshot,
                    request: request.clone(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            StreamEvent::AskUserRequested {
                request_id: internal_call_id.to_string(),
                request: request.clone(),
            },
        ) {
            let mut state = self.inner.lock().expect("ask user state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "AskUser was cancelled because the interactive UI is unavailable.",
            );
        }

        match rx.await {
            Ok(response) => {
                ToolCallHookAction::skip(serde_json::to_string(&response).unwrap_or_else(|_| {
                    "{\"questions\":[],\"error\":\"failed to serialize AskUser response\"}".into()
                }))
            }
            Err(_) => ToolCallHookAction::skip("AskUser was cancelled before the user answered."),
        }
    }

    fn resolve(&self, request_id: &str, response: AskUserResponse) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("ask user state lock");
            state.pending.remove(request_id)
        };

        let Some(pending) = pending else {
            return InteractionResolveResult::Missing;
        };

        if pending.sender.send(response.clone()).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = pending.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::AskUser {
                    request: pending.request,
                    response,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("ask user state lock");
        state.pending.clear();
    }
}

impl PromptHook<openai::CompletionModel> for ShellApprovalHook {
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        if tool_name != RUN_SHELL_SCRIPT_TOOL_NAME {
            return ToolCallHookAction::Continue;
        }

        let args = match serde_json::from_str::<RunShellScriptArgs>(args) {
            Ok(args) => args,
            Err(error) => {
                return ToolCallHookAction::skip(format!(
                    "RunShellScript request was invalid JSON: {error}"
                ));
            }
        };

        self.approvals
            .request_approval(
                self.reply_id,
                self.access_mode,
                internal_call_id,
                &args,
                &self.emit,
                &self.safety,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}

impl PromptHook<openai::CompletionModel> for WriteApprovalHook {
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        if !is_mutation_tool(tool_name) {
            return ToolCallHookAction::Continue;
        }

        self.approvals
            .request_approval(
                self.reply_id,
                tool_name,
                internal_call_id,
                args,
                &self.emit,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}

impl PromptHook<openai::CompletionModel> for AskUserHook {
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        if tool_name != AskUserTool::NAME {
            return ToolCallHookAction::Continue;
        }

        let Some(controller) = &self.controller else {
            return ToolCallHookAction::skip(
                "AskUser requires the interactive UI and is unavailable in this runtime.",
            );
        };

        controller
            .request_input(
                self.reply_id,
                internal_call_id,
                args,
                &self.emit,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}

impl<M> PromptHook<M> for CompletionCaptureHook
where
    M: CompletionModel,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        if let Some(capture) = &self.capture {
            capture.record(prompt, history);
        }
        HookAction::Continue
    }
}

impl<M, H1, H2> PromptHook<M> for CombinedHook<H1, H2>
where
    M: CompletionModel,
    H1: PromptHook<M>,
    H2: PromptHook<M>,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        let first = self.first.on_completion_call(prompt, history).await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second.on_completion_call(prompt, history).await
    }

    async fn on_tool_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        let first = self
            .first
            .on_tool_call(tool_name, tool_call_id.clone(), internal_call_id, args)
            .await;
        if !matches!(first, ToolCallHookAction::Continue) {
            return first;
        }
        self.second
            .on_tool_call(tool_name, tool_call_id, internal_call_id, args)
            .await
    }

    async fn on_tool_result(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
        result: &str,
    ) -> HookAction {
        let first = self
            .first
            .on_tool_result(
                tool_name,
                tool_call_id.clone(),
                internal_call_id,
                args,
                result,
            )
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_tool_result(tool_name, tool_call_id, internal_call_id, args, result)
            .await
    }

    async fn on_text_delta(&self, text_delta: &str, aggregated_text: &str) -> HookAction {
        let first = self.first.on_text_delta(text_delta, aggregated_text).await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second.on_text_delta(text_delta, aggregated_text).await
    }

    async fn on_tool_call_delta(
        &self,
        tool_call_id: &str,
        internal_call_id: &str,
        tool_name: Option<&str>,
        tool_call_delta: &str,
    ) -> HookAction {
        let first = self
            .first
            .on_tool_call_delta(tool_call_id, internal_call_id, tool_name, tool_call_delta)
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_tool_call_delta(tool_call_id, internal_call_id, tool_name, tool_call_delta)
            .await
    }

    async fn on_stream_completion_response_finish(
        &self,
        prompt: &rig::completion::Message,
        response: &M::StreamingResponse,
    ) -> HookAction {
        let first = self
            .first
            .on_stream_completion_response_finish(prompt, response)
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_stream_completion_response_finish(prompt, response)
            .await
    }
}

fn reasoning_params(reasoning_effort: ReasoningEffort) -> serde_json::Value {
    json!({
        "reasoning_effort": reasoning_effort.as_str()
    })
}

fn safety_classifier_preamble() -> &'static str {
    concat!(
        "You classify shell commands for execution safety.\n",
        "Respond using the structured output schema.\n",
        "Set `risk` to Low, Medium, or High.\n",
        "Set `explanation` to a concise justification.\n",
        "Keep `explanation` to 10 words or fewer when possible.\n",
        "Low: clearly read-only and non-destructive inspection commands.\n",
        "Medium: changes are possible but local, non-destructive, and reversible.\n",
        "High: destructive, irreversible, repository-changing, system-changing, network-changing, package-installing, or otherwise risky commands.\n",
        "Classify based on side effects, not whether the command touches a remote or the network.\n",
        "Git commands are not automatically High. Read-only git inspection commands such as status, diff, log, show, and ls-remote can be Low, including when they inspect remote refs or metadata. Git commands that modify repository state, update refs, transfer objects into the repo, or may discard work are usually High.\n",
        "If unsure, return High."
    )
}

fn safety_classifier_prompt(
    access_mode: AccessMode,
    command: &str,
    working_directory: &str,
    args: &RunShellScriptArgs,
    heuristic: Option<CommandRisk>,
) -> String {
    format!(
        concat!(
            "Access mode: {}\n",
            "Display command: {}\n",
            "Working directory: {}\n",
            "Intent: {}\n",
            "Heuristic minimum risk: {}\n",
            "Script:\n{}\n\n",
            "Return a structured response with `risk` and `explanation`.\n",
            "`risk` must be Low, Medium, or High.\n",
            "`explanation` should be concise: 10 words or fewer when possible.\n",
            "Do not set `risk` below the heuristic minimum risk when one is provided.\n"
        ),
        access_mode.label(),
        command,
        working_directory,
        normalize_summary(&args.intent),
        heuristic.map(CommandRisk::label).unwrap_or("None"),
        args.script
    )
}

fn normalize_summary(summary: &str) -> String {
    let normalized = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "No reason provided for this shell command".into()
    } else {
        normalized
    }
}

fn minimum_shell_risk(command: &str, script: &str) -> Option<CommandRisk> {
    let normalized = format!("{command}\n{script}").to_ascii_lowercase();
    let high_markers = [
        " rm ", "\nrm ", "rm -", "mkfs", "shutdown", "reboot", "kill ", "killall", "sudo ",
        "chmod ", "chown ", "dd ",
    ];
    if high_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(CommandRisk::High);
    }

    let medium_markers = [
        "mkdir ", "touch ", " mv ", "\nmv ", " cp ", "\ncp ", "tee ", ">>", " >", "install ",
        "sed -i", "perl -pi",
    ];
    if medium_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return Some(CommandRisk::Medium);
    }

    None
}

fn max_command_risk(left: CommandRisk, right: CommandRisk) -> CommandRisk {
    use CommandRisk::{High, Low, Medium};
    match (left, right) {
        (High, _) | (_, High) => High,
        (Medium, _) | (_, Medium) => Medium,
        (Low, Low) => Low,
    }
}

fn shell_pattern_matches(pattern: &str, command: &str) -> bool {
    if pattern.contains('*') {
        Glob::new(pattern)
            .ok()
            .is_some_and(|glob| glob.compile_matcher().is_match(command))
    } else {
        command.starts_with(pattern)
    }
}

fn build_agent(
    client: &openai::CompletionsClient,
    model_name: &str,
    preamble: &str,
    reasoning_effort: ReasoningEffort,
    tool_context: Option<ToolContext>,
) -> LlmAgent {
    let builder = client
        .agent(model_name.to_string())
        .preamble(preamble)
        .additional_params(reasoning_params(reasoning_effort));
    match tool_context {
        Some(tool_context) => builder.tools(tools_for_context(tool_context)).build(),
        None => builder.build(),
    }
}

async fn run_plain_prompt(
    agent: &LlmAgent,
    prompt: String,
    history: Vec<RigMessage>,
) -> Result<String> {
    let mut stream = agent.stream_chat(prompt, history).multi_turn(0).await;
    let mut output = String::new();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                output.push_str(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                if !response.response().is_empty() {
                    return Ok(response.response().to_string());
                }
                return Ok(output);
            }
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow::anyhow!("Request ended before response completed."))
}

fn estimated_request_tokens(history: &[RigMessage], prompt: &RigMessage) -> usize {
    (estimated_history_context_tokens(history) + estimated_message_tokens(prompt)) as usize
}

fn is_step_boundary_error(error: &StreamingError) -> bool {
    matches!(
        error,
        StreamingError::Prompt(prompt_error)
            if matches!(prompt_error.as_ref(), PromptError::PromptCancelled { reason, .. } if reason == STEP_BOUNDARY_REASON)
    )
}

fn is_retryable_compaction_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "context_length_exceeded",
        "input tokens exceed",
        "maximum context",
        "too large",
        "max_output_tokens",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn drop_oldest_compaction_source_message(history: &mut Vec<RigMessage>) -> bool {
    if history.is_empty() {
        false
    } else {
        history.remove(0);
        true
    }
}

fn rebuild_compacted_history(history: &[RigMessage], summary: &str) -> Vec<RigMessage> {
    let user_indexes = retain_tail_indexes(
        history,
        COMPACTION_USER_TOKEN_BUDGET,
        |message| matches!(message, RigMessage::User { content } if content.iter().any(is_regular_user_content)),
    );
    let tool_indexes = retain_tail_indexes(history, COMPACTION_TOOL_TOKEN_BUDGET, |message| {
        message_contains_tool_state(message)
    });

    let mut retained = user_indexes
        .into_iter()
        .chain(tool_indexes)
        .collect::<Vec<_>>();
    retained.sort_unstable();
    retained.dedup();

    let mut rebuilt = retained
        .into_iter()
        .map(|index| history[index].clone())
        .collect::<Vec<_>>();
    rebuilt.push(RigMessage::user(format!(
        "{COMPACTION_SUMMARY_PREFIX}{summary}"
    )));
    rebuilt
}

fn retain_tail_indexes(
    history: &[RigMessage],
    token_budget: usize,
    predicate: impl Fn(&RigMessage) -> bool,
) -> Vec<usize> {
    let mut kept = Vec::new();
    let mut used_tokens = 0usize;

    for (index, message) in history.iter().enumerate().rev() {
        if !predicate(message) {
            continue;
        }

        let message_tokens = estimated_message_tokens(message) as usize;
        if !kept.is_empty() && used_tokens + message_tokens > token_budget {
            break;
        }
        kept.push(index);
        used_tokens += message_tokens;
        if used_tokens >= token_budget {
            break;
        }
    }

    kept.reverse();
    kept
}

fn is_regular_user_content(content: &UserContent) -> bool {
    !matches!(content, UserContent::ToolResult(_))
}

fn message_contains_tool_state(message: &RigMessage) -> bool {
    match message {
        RigMessage::Assistant { content, .. } => content
            .iter()
            .any(|content| matches!(content, AssistantContent::ToolCall(_))),
        RigMessage::User { content } => content
            .iter()
            .any(|content| matches!(content, UserContent::ToolResult(_))),
        RigMessage::System { .. } => false,
    }
}

fn azure_openai_base_url(config: &AppConfig) -> String {
    format!(
        "{}/openai/v1",
        config.azure.endpoint().trim_end_matches('/')
    )
}

fn execution_mode_label(access_mode: AccessMode) -> &'static str {
    match access_mode {
        AccessMode::ReadOnly => "read-only mode",
        AccessMode::ReadWrite => "write mode",
    }
}

fn mode_preamble(context: &AgentContext) -> String {
    let mut preamble = SYSTEM_PROMPT.trim().replace(
        "{{EXECUTION_MODE}}",
        execution_mode_label(context.access_mode),
    );

    match context.role {
        AgentRole::Main => {
            preamble.push_str(
                "\n\nYou can delegate bounded parallel tasks to subagents when that will help you cover more ground. Give them enough local context to work independently. While subagents are running, normally treat that as a handoff: do not continue doing the same delegated work in the main agent unless the user explicitly wants redundancy or there is a clear independent task you can do without overlap. Prefer to wait on the subagents or inspect their status/results instead of duplicating their work or assuming they completed.",
            );
        }
        AgentRole::Subagent => {
            preamble.push_str(
                "\n\nYou are running as a subagent on behalf of the main agent. You start with fresh context, so rely on the delegated prompt and your own tool exploration. Focus tightly on the delegated task and return a concise result that the main agent can use directly. You cannot spawn subagents of your own.",
            );
        }
    }

    match context.access_mode {
        AccessMode::ReadOnly => {
            preamble.push_str(
                "\n\nYou are currently in read-only mode. Use the provided readonly workspace tools when they are useful. A shell tool may also be available, but only low-risk inspection commands can be approved in read-only mode; anything medium or high risk requires write mode. If the user asks you to edit, create, or delete files, explain that you are in read-only mode and the user must switch to write mode before you can modify the workspace. Do not print large amounts of code in read-only mode unless the user explicitly asks for it.",
            );
        }
        AccessMode::ReadWrite => {
            preamble.push_str(
                "\n\nYou are currently in write mode. Use the provided workspace tools when useful. Shell commands may still require per-command approval depending on risk. If the user asks you to write code, they usually mean to file (either as a new file, or to edit an existing one), rather than just printing it in their terminal, unless they explicitly ask for it.",
            );
        }
    }

    preamble
}

fn format_tool_arguments(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments).unwrap_or_else(|_| arguments.to_string())
}

fn resume_override_matches_tool_call(
    override_action: &ResumeOverride,
    name: &str,
    arguments: &str,
) -> bool {
    match override_action {
        ResumeOverride::WriteApproval {
            tool_name,
            arguments: expected_arguments,
            ..
        } => tool_name == name && expected_arguments == arguments,
        ResumeOverride::ShellApproval {
            command,
            working_directory,
            ..
        } => {
            if name != RUN_SHELL_SCRIPT_TOOL_NAME {
                return false;
            }
            let Ok(args) = serde_json::from_str::<RunShellScriptArgs>(arguments) else {
                return false;
            };
            display_shell_command(&args.script) == *command
                && display_requested_shell_cwd(args.cwd.as_deref()) == *working_directory
        }
        ResumeOverride::AskUser { .. } => false,
    }
}

fn format_tool_result(tool_result: &ToolResult) -> String {
    let parts = tool_result
        .content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.clone(),
            ToolResultContent::Image(_) => "[image tool result]".to_string(),
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n")
    }
}

fn parse_commentary_message(args: &str) -> Result<String> {
    serde_json::from_str::<CommentaryArgs>(args)?
        .validated_message()
        .map_err(Into::into)
}

fn resolve_commentary_message(
    partial_tool_calls: &mut HashMap<String, PartialToolCall>,
    internal_call_id: &str,
    fallback_arguments: &str,
) -> Result<String> {
    let mut candidates = Vec::new();
    if let Some(partial) = partial_tool_calls.remove(internal_call_id)
        && !partial.arguments.trim().is_empty()
    {
        candidates.push(partial.arguments);
    }
    candidates.push(fallback_arguments.to_string());

    let mut best_message = None;
    let mut last_error = None;
    for candidate in candidates {
        match parse_commentary_message(&candidate) {
            Ok(message) => {
                if best_message.as_ref().is_none_or(|current: &String| {
                    message.chars().count() > current.chars().count()
                }) {
                    best_message = Some(message);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    if let Some(message) = best_message {
        Ok(message)
    } else if let Some(error) = last_error {
        Err(error)
    } else {
        parse_commentary_message(fallback_arguments)
    }
}

fn reconcile_stream_text(incoming: &str, replay_probe: &mut Option<ReplayProbe>) -> String {
    if incoming.is_empty() {
        return String::new();
    }

    let Some(probe) = replay_probe.as_mut() else {
        return incoming.to_string();
    };

    probe.buffered.push_str(incoming);

    if probe.expected.starts_with(&probe.buffered) {
        if probe.expected.len() == probe.buffered.len() {
            *replay_probe = None;
        }
        return String::new();
    }

    if probe.buffered.starts_with(&probe.expected) {
        let suffix = probe.buffered[probe.expected.len()..].to_string();
        *replay_probe = None;
        return suffix;
    }

    let buffered = probe.buffered.clone();
    *replay_probe = None;
    buffered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::AgentContext,
        config::{
            AzureConfig, ReasoningEffort, SafetyConfig, SubagentConfig, ToolConfig, UiConfig,
        },
        planning::PlanningConfig,
    };
    use rig::{OneOrMany, completion::message::Text};

    fn sample_config() -> AppConfig {
        AppConfig {
            azure: AzureConfig {
                resource_name: "demo-resource".into(),
                api_key: "secret".into(),
                model_name: "gpt-5-mini".into(),
                reasoning_effort: ReasoningEffort::Minimal,
                api_version: "2025-01-01-preview".into(),
            },
            safety: SafetyConfig {
                model_name: "gpt-5-mini".into(),
                reasoning_effort: ReasoningEffort::Low,
            },
            ui: UiConfig {
                show_thinking: true,
                show_tool_output: false,
                command_history_limit: 20,
            },
            subagents: SubagentConfig { max_concurrent: 4 },
            planning: PlanningConfig::default(),
            tools: ToolConfig::default(),
        }
    }

    #[test]
    fn reasoning_params_match_requested_effort() {
        let params = reasoning_params(sample_config().azure.reasoning_effort);
        assert_eq!(params, json!({ "reasoning_effort": "minimal" }));
    }

    #[test]
    fn azure_openai_base_url_targets_v1_endpoint() {
        assert_eq!(
            azure_openai_base_url(&sample_config()),
            "https://demo-resource.openai.azure.com/openai/v1"
        );
    }

    #[test]
    fn format_tool_result_joins_text_parts() {
        let tool_result = ToolResult {
            id: "call_1".into(),
            call_id: None,
            content: OneOrMany::many(vec![
                ToolResultContent::Text(Text {
                    text: "first".into(),
                }),
                ToolResultContent::Text(Text {
                    text: "second".into(),
                }),
            ])
            .expect("non-empty"),
        };

        assert_eq!(format_tool_result(&tool_result), "first\nsecond");
    }

    #[test]
    fn format_tool_arguments_serializes_json_compactly() {
        assert_eq!(
            format_tool_arguments(&json!({ "dir": "src", "recursive": true })),
            r#"{"dir":"src","recursive":true}"#
        );
    }

    #[test]
    fn reconcile_stream_text_passes_through_normal_deltas() {
        let mut replay_probe = None;
        assert_eq!(
            reconcile_stream_text(" and more", &mut replay_probe),
            " and more"
        );
    }

    #[test]
    fn reconcile_stream_text_strips_replayed_prefix_at_new_segment_start() {
        let mut replay_probe = Some(ReplayProbe::new("message 1"));
        assert_eq!(
            reconcile_stream_text("message 1 and more", &mut replay_probe),
            " and more"
        );
        assert_eq!(replay_probe, None);
    }

    #[test]
    fn reconcile_stream_text_suppresses_fully_replayed_chunk() {
        let mut replay_probe = Some(ReplayProbe::new("message 1"));
        assert_eq!(reconcile_stream_text("message 1", &mut replay_probe), "");
        assert_eq!(replay_probe, None);
    }

    #[test]
    fn reconcile_stream_text_does_not_strip_unrelated_segment_start() {
        let mut replay_probe = Some(ReplayProbe::new("message 1"));
        assert_eq!(
            reconcile_stream_text("message 2", &mut replay_probe),
            "message 2"
        );
        assert_eq!(replay_probe, None);
    }

    #[test]
    fn reconcile_stream_text_suppresses_chunked_replay_before_new_suffix() {
        let mut output = "Message 1".to_string();
        let mut replay_probe = Some(ReplayProbe::new(&output));

        let first = reconcile_stream_text("Mess", &mut replay_probe);
        assert_eq!(first, "");
        assert_eq!(
            replay_probe,
            Some(ReplayProbe {
                expected: "Message 1".into(),
                buffered: "Mess".into(),
            })
        );

        let second = reconcile_stream_text("age 1", &mut replay_probe);
        assert_eq!(second, "");
        assert_eq!(replay_probe, None);

        let third = reconcile_stream_text("\n\nMessage 2", &mut replay_probe);
        assert_eq!(third, "\n\nMessage 2");
        output.push_str(&third);
        assert_eq!(output, "Message 1\n\nMessage 2");
    }

    #[test]
    fn reconcile_stream_text_emits_only_new_tail_when_chunk_finishes_replay() {
        let mut replay_probe = Some(ReplayProbe::new("Message 1"));
        assert_eq!(
            reconcile_stream_text("Message 1\n\nMessage 2", &mut replay_probe),
            "\n\nMessage 2"
        );
        assert_eq!(replay_probe, None);
    }

    #[test]
    fn reconcile_stream_text_preserves_shared_prefix_until_divergence() {
        let mut replay_probe = Some(ReplayProbe::new("checking the model registry"));

        let first = reconcile_stream_text("checking the ", &mut replay_probe);
        assert_eq!(first, "");

        let second = reconcile_stream_text("plan in the registry", &mut replay_probe);
        assert_eq!(second, "checking the plan in the registry");
        assert_eq!(replay_probe, None);
    }

    #[test]
    fn parse_commentary_message_extracts_trimmed_message() {
        assert_eq!(
            parse_commentary_message(r#"{"message":"  Checking the logs now.  "}"#)
                .expect("valid commentary"),
            "Checking the logs now."
        );
    }

    #[test]
    fn parse_commentary_message_rejects_invalid_payload() {
        assert!(parse_commentary_message(r#"{"message":"   "}"#).is_err());
        assert!(parse_commentary_message(r#"{"text":"missing"}"#).is_err());
    }

    #[test]
    fn resolve_commentary_message_prefers_longer_valid_payload() {
        let mut partials = HashMap::from([(
            "call-1".to_string(),
            PartialToolCall {
                name: Some("Commentary".into()),
                arguments: r#"{"message":"’ve mapped the registry."}"#.into(),
            },
        )]);

        assert_eq!(
            resolve_commentary_message(
                &mut partials,
                "call-1",
                r#"{"message":"I’ve mapped the registry."}"#
            )
            .expect("commentary resolves"),
            "I’ve mapped the registry."
        );
        assert!(!partials.contains_key("call-1"));
    }

    #[test]
    fn resolve_commentary_message_falls_back_when_no_delta_payload_exists() {
        let mut partials = HashMap::new();

        assert_eq!(
            resolve_commentary_message(&mut partials, "call-1", r#"{"message":"fallback"}"#)
                .expect("fallback commentary"),
            "fallback"
        );
    }

    #[test]
    fn completion_capture_keeps_latest_request_snapshot() {
        let capture = CompletionCapture::new();
        let first_history = vec![RigMessage::system("be concise")];
        let first_prompt = RigMessage::user("inspect src");
        capture.record(&first_prompt, &first_history);

        let second_history = vec![RigMessage::assistant("Working on it.")];
        let second_prompt = RigMessage::user("continue");
        capture.record(&second_prompt, &second_history);

        let snapshot = capture.snapshot().expect("snapshot captured");
        assert_eq!(snapshot.history, second_history);
        assert_eq!(snapshot.prompt, second_prompt);
        assert_eq!(snapshot.message_count, 2);
    }

    #[test]
    fn rebuild_compacted_history_keeps_recent_user_and_tool_state_plus_summary() {
        let history = vec![
            RigMessage::user("first user"),
            RigMessage::assistant("plain assistant"),
            RigMessage::Assistant {
                id: None,
                content: rig::OneOrMany::one(AssistantContent::tool_call(
                    "tool-1",
                    "List",
                    json!({"path":"src"}),
                )),
            },
            RigMessage::User {
                content: rig::OneOrMany::one(UserContent::tool_result(
                    "tool-1",
                    rig::OneOrMany::one(ToolResultContent::text("tool output")),
                )),
            },
            RigMessage::user("latest user"),
        ];

        let rebuilt = rebuild_compacted_history(&history, "summary text");

        assert_eq!(rebuilt.len(), 5);
        assert!(
            matches!(&rebuilt[0], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text() == "first user")))
        );
        assert!(matches!(&rebuilt[1], RigMessage::Assistant { .. }));
        assert!(matches!(&rebuilt[2], RigMessage::User { .. }));
        assert!(
            matches!(&rebuilt[3], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text() == "latest user")))
        );
        assert!(
            matches!(&rebuilt[4], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text().contains(COMPACTION_SUMMARY_PREFIX) && text.text().contains("summary text"))))
        );
    }

    #[test]
    fn message_contains_tool_state_distinguishes_plain_and_tool_messages() {
        assert!(!message_contains_tool_state(&RigMessage::user(
            "plain user"
        )));
        assert!(!message_contains_tool_state(&RigMessage::assistant(
            "plain assistant"
        )));
        assert!(message_contains_tool_state(&RigMessage::Assistant {
            id: None,
            content: rig::OneOrMany::one(AssistantContent::tool_call(
                "tool-1",
                "List",
                json!({"path":"src"}),
            )),
        }));
        assert!(message_contains_tool_state(&RigMessage::User {
            content: rig::OneOrMany::one(UserContent::tool_result(
                "tool-1",
                rig::OneOrMany::one(ToolResultContent::text("tool output")),
            )),
        }));
    }

    #[test]
    fn read_only_mode_preamble_uses_shared_prompt_and_read_only_suffix() {
        let preamble = mode_preamble(&AgentContext::main(AccessMode::ReadOnly));
        assert!(preamble.contains("You are oat: an opinionated agent thing."));
        assert!(preamble.contains("You are a provider-agnostic coding agent."));
        assert!(preamble.contains("You have three modes: read-only, write, and plan mode."));
        assert!(
            preamble.contains(
                "Intermediary updates are provided to the user via the `Commentary` tool."
            )
        );
        assert!(preamble.contains("You are currently in read-only mode."));
        assert!(!preamble.contains("{{EXECUTION_MODE}}"));
        assert!(preamble.contains("You are currently in read-only mode."));
        assert!(preamble.contains("Do not print large amounts of code in read-only mode"));
        assert!(!preamble.contains("You are currently in write mode."));
    }

    #[tokio::test]
    async fn read_write_mode_registers_mutation_tools() {
        let service = LlmService::from_config(
            &sample_config(),
            AgentContext::main(AccessMode::ReadWrite),
            WriteApprovalController::default(),
            Some(AskUserController::default()),
            None,
        )
        .expect("service builds");

        assert!(service.tool_names.contains(&"AskUser".to_string()));
        assert!(service.tool_names.contains(&"ApplyPatches".to_string()));
        assert!(service.tool_names.contains(&"WriteFile".to_string()));
        assert!(service.tool_names.contains(&"DeletePath".to_string()));
        assert!(
            service
                .preamble
                .contains("You are oat: an opinionated agent thing.")
        );
        assert!(
            service
                .preamble
                .contains("You are a provider-agnostic coding agent.")
        );
        assert!(
            service
                .preamble
                .contains("Persist until the task is fully handled end-to-end")
        );
        assert!(
            service
                .preamble
                .contains("You are currently in write mode.")
        );
        assert!(service.preamble.contains("they usually mean to file"));
        assert!(
            service
                .preamble
                .contains("While subagents are running, normally treat that as a handoff")
        );
        assert!(
            !service
                .preamble
                .contains("You are currently in read-only mode.")
        );
    }

    #[tokio::test]
    async fn read_only_mode_omits_mutation_tools() {
        let service = LlmService::from_config(
            &sample_config(),
            AgentContext::main(AccessMode::ReadOnly),
            WriteApprovalController::default(),
            Some(AskUserController::default()),
            None,
        )
        .expect("service builds");

        assert!(service.tool_names.contains(&"AskUser".to_string()));
        assert!(!service.tool_names.contains(&"ApplyPatches".to_string()));
        assert!(!service.tool_names.contains(&"WriteFile".to_string()));
        assert!(!service.tool_names.contains(&"DeletePath".to_string()));
    }

    #[test]
    fn write_approval_controller_reset_is_safe_without_pending_requests() {
        let approvals = WriteApprovalController::default();
        assert_eq!(
            approvals.resolve("missing", WriteApprovalDecision::AllowAllSession),
            InteractionResolveResult::Missing
        );
        approvals.reset_session();
    }

    #[test]
    fn write_approval_controller_returns_resume_request_when_waiter_is_gone() {
        let approvals = WriteApprovalController::default();
        let snapshot = CompletionRequestSnapshot::capture(&RigMessage::user("continue"), &[]);
        let (tx, rx) = oneshot::channel();
        drop(rx);
        approvals
            .inner
            .lock()
            .expect("approval state lock")
            .pending
            .insert(
                "call-1".into(),
                PendingWriteApprovalEntry {
                    sender: tx,
                    snapshot: Some(snapshot.clone()),
                    tool_name: "WriteFile".into(),
                    arguments: "{\"path\":\"src/main.rs\"}".into(),
                },
            );

        let result = approvals.resolve("call-1", WriteApprovalDecision::AllowOnce);

        assert_eq!(
            result,
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::WriteApproval {
                    tool_name: "WriteFile".into(),
                    arguments: "{\"path\":\"src/main.rs\"}".into(),
                    decision: WriteApprovalDecision::AllowOnce,
                },
            })
        );
    }

    #[test]
    fn write_approval_controller_can_resolve_when_waiter_is_live_or_snapshot_exists() {
        let approvals = WriteApprovalController::default();

        let (live_tx, _live_rx) = oneshot::channel();
        approvals
            .inner
            .lock()
            .expect("approval state lock")
            .pending
            .insert(
                "live".into(),
                PendingWriteApprovalEntry {
                    sender: live_tx,
                    snapshot: None,
                    tool_name: "WriteFile".into(),
                    arguments: "{}".into(),
                },
            );

        let (closed_tx, closed_rx) = oneshot::channel();
        drop(closed_rx);
        approvals
            .inner
            .lock()
            .expect("approval state lock")
            .pending
            .insert(
                "resume".into(),
                PendingWriteApprovalEntry {
                    sender: closed_tx,
                    snapshot: Some(CompletionRequestSnapshot::capture(
                        &RigMessage::user("continue"),
                        &[],
                    )),
                    tool_name: "WriteFile".into(),
                    arguments: "{}".into(),
                },
            );

        let (dead_tx, dead_rx) = oneshot::channel();
        drop(dead_rx);
        approvals
            .inner
            .lock()
            .expect("approval state lock")
            .pending
            .insert(
                "dead".into(),
                PendingWriteApprovalEntry {
                    sender: dead_tx,
                    snapshot: None,
                    tool_name: "WriteFile".into(),
                    arguments: "{}".into(),
                },
            );

        assert!(approvals.can_resolve("live"));
        assert!(approvals.can_resolve("resume"));
        assert!(!approvals.can_resolve("dead"));
    }

    #[test]
    fn ask_user_controller_returns_resume_request_when_waiter_is_gone() {
        let controller = AskUserController::default();
        let snapshot = CompletionRequestSnapshot::capture(&RigMessage::user("continue"), &[]);
        let request = AskUserRequest {
            title: Some("Clarify scope".into()),
            questions: vec![crate::ask_user::AskUserQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                answers: vec![crate::ask_user::AskUserAnswer {
                    id: "narrow".into(),
                    label: "Narrow".into(),
                }],
            }],
        };
        let response = AskUserResponse {
            questions: vec![crate::ask_user::AskUserAnsweredQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                selected_answer: crate::ask_user::AskUserSelectedAnswer {
                    id: "narrow".into(),
                    label: "Narrow".into(),
                    is_recommended: true,
                    is_something_else: false,
                },
                details: String::new(),
            }],
        };
        let (tx, rx) = oneshot::channel();
        drop(rx);
        controller
            .inner
            .lock()
            .expect("ask user state lock")
            .pending
            .insert(
                "call-2".into(),
                PendingAskUserEntry {
                    sender: tx,
                    snapshot: Some(snapshot.clone()),
                    request: request.clone(),
                },
            );

        let result = controller.resolve("call-2", response.clone());

        assert_eq!(
            result,
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::AskUser { request, response },
            })
        );
    }

    #[test]
    fn ask_user_controller_can_resolve_when_waiter_is_live_or_snapshot_exists() {
        let controller = AskUserController::default();
        let request = AskUserRequest {
            title: Some("Clarify scope".into()),
            questions: vec![crate::ask_user::AskUserQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                answers: vec![crate::ask_user::AskUserAnswer {
                    id: "narrow".into(),
                    label: "Narrow".into(),
                }],
            }],
        };

        let (live_tx, _live_rx) = oneshot::channel();
        controller
            .inner
            .lock()
            .expect("ask user state lock")
            .pending
            .insert(
                "live".into(),
                PendingAskUserEntry {
                    sender: live_tx,
                    snapshot: None,
                    request: request.clone(),
                },
            );

        let (closed_tx, closed_rx) = oneshot::channel();
        drop(closed_rx);
        controller
            .inner
            .lock()
            .expect("ask user state lock")
            .pending
            .insert(
                "resume".into(),
                PendingAskUserEntry {
                    sender: closed_tx,
                    snapshot: Some(CompletionRequestSnapshot::capture(
                        &RigMessage::user("continue"),
                        &[],
                    )),
                    request: request.clone(),
                },
            );

        let (dead_tx, dead_rx) = oneshot::channel();
        drop(dead_rx);
        controller
            .inner
            .lock()
            .expect("ask user state lock")
            .pending
            .insert(
                "dead".into(),
                PendingAskUserEntry {
                    sender: dead_tx,
                    snapshot: None,
                    request,
                },
            );

        assert!(controller.can_resolve("live"));
        assert!(controller.can_resolve("resume"));
        assert!(!controller.can_resolve("dead"));
    }

    #[tokio::test]
    async fn write_mode_preamble_is_the_same_for_both_approval_modes() {
        let manual = LlmService::from_config(
            &sample_config(),
            AgentContext::main(AccessMode::ReadWrite),
            WriteApprovalController::new(ApprovalMode::Manual),
            Some(AskUserController::default()),
            None,
        )
        .expect("manual service builds")
        .preamble;
        let disabled = LlmService::from_config(
            &sample_config(),
            AgentContext::main(AccessMode::ReadWrite),
            WriteApprovalController::new(ApprovalMode::Disabled),
            Some(AskUserController::default()),
            None,
        )
        .expect("disabled service builds")
        .preamble;

        assert_eq!(manual, disabled);
    }

    #[test]
    fn write_approval_controller_can_start_disabled_and_reset_to_default() {
        let approvals = WriteApprovalController::new(ApprovalMode::Disabled);
        assert_eq!(approvals.mode(), ApprovalMode::Disabled);
        approvals.reset_session();
        assert_eq!(approvals.mode(), ApprovalMode::Disabled);
    }

    #[test]
    fn safety_preamble_allows_read_only_git_commands_to_be_low() {
        let preamble = safety_classifier_preamble();
        assert!(preamble.contains("structured output schema"));
        assert!(preamble.contains("Set `risk` to Low, Medium, or High."));
        assert!(preamble.contains("Set `explanation` to a concise justification."));
        assert!(preamble.contains("10 words or fewer when possible"));
        assert!(preamble.contains("side effects"));
        assert!(preamble.contains("Git commands are not automatically High."));
        assert!(preamble.contains("status, diff, log, show, and ls-remote can be Low"));
    }

    #[test]
    fn minimum_shell_risk_does_not_force_git_status_high() {
        assert_eq!(minimum_shell_risk("git status", "git status"), None);
        assert_eq!(
            minimum_shell_risk("git diff --stat", "git diff --stat"),
            None
        );
        assert_eq!(
            minimum_shell_risk("rm -rf target", "rm -rf target"),
            Some(CommandRisk::High)
        );
    }

    #[test]
    fn safety_classifier_risk_output_converts_to_command_risk() {
        assert_eq!(
            CommandRisk::from(SafetyClassifierRiskOutput::Low),
            CommandRisk::Low
        );
        assert_eq!(
            CommandRisk::from(SafetyClassifierRiskOutput::Medium),
            CommandRisk::Medium
        );
        assert_eq!(
            CommandRisk::from(SafetyClassifierRiskOutput::High),
            CommandRisk::High
        );
    }
}
