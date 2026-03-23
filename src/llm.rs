use std::{
    collections::{HashMap, HashSet},
    env,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use globset::Glob;
use rig::{
    agent::{HookAction, MultiTurnStreamItem, PromptHook, ToolCallHookAction},
    client::CompletionClient,
    completion::{
        Chat, CompletionModel, Message as RigMessage,
        message::{ToolResult, ToolResultContent},
    },
    providers::openai,
    streaming::{
        StreamedAssistantContent, StreamedUserContent, StreamingChat, ToolCallDeltaContent,
    },
    tool::Tool,
};
use serde_json::json;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    agent::{AgentContext, AgentRole},
    app::{AccessMode, ApprovalMode, CommandRisk, ShellApprovalDecision, WriteApprovalDecision},
    ask_user::{AskUserRequest, AskUserResponse, validate_request},
    completion_request::CompletionRequestSnapshot,
    config::{AppConfig, ReasoningEffort},
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
    Finished {
        history: Option<Vec<RigMessage>>,
    },
    Failed(String),
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
    pending: HashMap<String, oneshot::Sender<WriteApprovalDecision>>,
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
    pending: HashMap<String, oneshot::Sender<AskUserResponse>>,
}

#[derive(Clone)]
struct ShellApprovalHook {
    reply_id: u64,
    emit: EventCallback,
    access_mode: AccessMode,
    approvals: ShellApprovalController,
    safety: SafetyClassifier,
}

#[derive(Clone)]
struct WriteApprovalHook {
    reply_id: u64,
    emit: EventCallback,
    approvals: WriteApprovalController,
}

#[derive(Clone)]
struct AskUserHook {
    reply_id: u64,
    emit: EventCallback,
    controller: Option<AskUserController>,
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

impl ReplayProbe {
    fn new(expected: &str) -> Self {
        Self {
            expected: expected.to_string(),
            buffered: String::new(),
        }
    }
}

#[derive(Clone)]
struct CombinedHook<H1, H2> {
    first: H1,
    second: H2,
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

#[derive(Clone)]
pub struct LlmService {
    agent: LlmAgent,
    access_mode: AccessMode,
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

#[derive(Clone, Default)]
pub struct CompletionCapture {
    inner: Arc<Mutex<Option<CompletionRequestSnapshot>>>,
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
        let agent = client
            .agent(
                context
                    .model_name_override
                    .clone()
                    .unwrap_or_else(|| config.azure.model_name.clone()),
            )
            .preamble(&preamble)
            .additional_params(reasoning_params(config.azure.reasoning_effort))
            .tools(tools_for_context(tool_context))
            .build();
        let safety = SafetyClassifier::from_client(&client, config);

        Ok(Self {
            agent,
            access_mode: context.access_mode,
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
    ) -> bool {
        self.approvals.resolve(request_id, decision)
    }

    pub fn resolve_shell_approval(
        &self,
        request_id: &str,
        decision: ShellApprovalDecision,
    ) -> bool {
        self.shell_approvals.resolve(request_id, decision)
    }

    pub fn resolve_ask_user(&self, request_id: &str, response: AskUserResponse) -> bool {
        self.ask_user
            .as_ref()
            .is_some_and(|controller| controller.resolve(request_id, response))
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
        stats_hook: StatsHook,
        events: UnboundedSender<(u64, StreamEvent)>,
    ) {
        let emit: EventCallback =
            Arc::new(move |reply_id, event| events.send((reply_id, event)).is_ok());
        let _ = self
            .run_prompt(reply_id, prompt, history, stats_hook, None, emit)
            .await;
    }

    pub async fn run_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<RigMessage>,
        stats_hook: StatsHook,
        capture: Option<CompletionCapture>,
        emit: EventCallback,
    ) -> Result<PromptRunResult> {
        let write_approval_hook = WriteApprovalHook {
            reply_id,
            emit: emit.clone(),
            approvals: self.approvals.clone(),
        };
        let shell_approval_hook = ShellApprovalHook {
            reply_id,
            emit: emit.clone(),
            access_mode: self.access_mode,
            approvals: self.shell_approvals.clone(),
            safety: self.safety.clone(),
        };
        let ask_user_hook = AskUserHook {
            reply_id,
            emit: emit.clone(),
            controller: self.ask_user.clone(),
        };
        let hook = CombinedHook {
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
        let mut replay_probe = None;

        while let Some(chunk) = stream.next().await {
            let event = match chunk {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => {
                    let delta = reconcile_stream_text(&text.text, &mut replay_probe);
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
                    replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    Some(StreamEvent::ReasoningDelta(reasoning.display_text()))
                }
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                )) => {
                    replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                    Some(StreamEvent::ReasoningDelta(reasoning))
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
                    replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
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
                                Some(StreamEvent::Commentary(message))
                            }
                            Err(_) => Some(StreamEvent::ToolCall {
                                name,
                                arguments: fallback_arguments,
                            }),
                        }
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
                    replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
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
                    return Ok(PromptRunResult { output, history });
                }
                Ok(_) => None,
                Err(error) => {
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
        let model_risk = match self.agent.chat(prompt, Vec::<RigMessage>::new()).await {
            Ok(output) => parse_command_risk(&output).unwrap_or(CommandRisk::High),
            Err(_) => CommandRisk::High,
        };
        let risk = max_command_risk(model_risk, heuristic.unwrap_or(CommandRisk::Low));

        SafetyClassification {
            risk,
            risk_explanation: shell_risk_explanation(risk, &command, &args.script),
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

    async fn request_approval(
        &self,
        reply_id: u64,
        tool_name: &str,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
    ) -> ToolCallHookAction {
        let rx = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(state.mode, ApprovalMode::Disabled) {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(internal_call_id.to_string(), tx);
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

    fn resolve(&self, request_id: &str, decision: WriteApprovalDecision) -> bool {
        let sender = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                state.mode = ApprovalMode::Disabled;
            }
            state.pending.remove(request_id)
        };

        if let Some(sender) = sender {
            sender.send(decision).is_ok()
        } else {
            false
        }
    }

    fn reset_session(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.mode = state.default_mode;
        for (_, sender) in state.pending.drain() {
            let _ = sender.send(WriteApprovalDecision::Deny);
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

    async fn request_approval(
        &self,
        reply_id: u64,
        access_mode: AccessMode,
        internal_call_id: &str,
        args: &RunShellScriptArgs,
        emit: &EventCallback,
        safety: &SafetyClassifier,
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

    fn resolve(&self, request_id: &str, decision: ShellApprovalDecision) -> bool {
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

        if let Some(entry) = pending {
            entry.sender.send(decision).is_ok()
        } else {
            false
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

    async fn request_input(
        &self,
        reply_id: u64,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
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

        let rx = {
            let mut state = self.inner.lock().expect("ask user state lock");
            let (tx, rx) = oneshot::channel();
            state.pending.insert(internal_call_id.to_string(), tx);
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

    fn resolve(&self, request_id: &str, response: AskUserResponse) -> bool {
        let sender = {
            let mut state = self.inner.lock().expect("ask user state lock");
            state.pending.remove(request_id)
        };

        if let Some(sender) = sender {
            sender.send(response).is_ok()
        } else {
            false
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
            .request_approval(self.reply_id, tool_name, internal_call_id, args, &self.emit)
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
            .request_input(self.reply_id, internal_call_id, args, &self.emit)
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
        "Return exactly one word: Low, Medium, or High.\n",
        "Low: clearly read-only and non-destructive inspection commands.\n",
        "Medium: changes are possible but local, non-destructive, and reversible.\n",
        "High: destructive, irreversible, repository-changing, system-changing, network-changing, package-installing, or otherwise risky commands.\n",
        "Git commands are not automatically High. Read-only git inspection commands such as status, diff, log, and show can be Low. Git commands that modify repository state, contact remotes, or may discard work are usually High.\n",
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
            "Script:\n{}\n"
        ),
        access_mode.label(),
        command,
        working_directory,
        normalize_summary(&args.intent),
        heuristic.map(CommandRisk::label).unwrap_or("None"),
        args.script
    )
}

fn parse_command_risk(output: &str) -> Option<CommandRisk> {
    let label = output.trim().lines().next()?.trim().to_ascii_lowercase();
    match label.as_str() {
        "low" => Some(CommandRisk::Low),
        "medium" => Some(CommandRisk::Medium),
        "high" => Some(CommandRisk::High),
        _ => None,
    }
}

fn normalize_summary(summary: &str) -> String {
    let normalized = summary.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        "No reason provided for this shell command".into()
    } else {
        normalized
    }
}

fn shell_risk_explanation(risk: CommandRisk, command: &str, script: &str) -> String {
    let normalized = format!("{command}\n{script}").to_ascii_lowercase();
    match risk {
        CommandRisk::Low => {
            if normalized.contains("cat ")
                || normalized.contains("ls ")
                || normalized.contains("pwd")
                || normalized.contains("rg ")
                || normalized.contains("find ")
            {
                "read-only inspection command with no obvious mutation".into()
            } else {
                "no obvious file, repository, or system mutation".into()
            }
        }
        CommandRisk::Medium => {
            if normalized.contains("mkdir ") || normalized.contains("touch ") {
                "may create workspace files or directories, but appears local and reversible".into()
            } else if normalized.contains("cp ") || normalized.contains("mv ") {
                "may change workspace files, but appears limited and reversible".into()
            } else {
                "may modify local state, but does not look destructive".into()
            }
        }
        CommandRisk::High => {
            if normalized.contains("rm ") || normalized.contains("rm -") {
                "includes removal commands that can destroy data".into()
            } else {
                "could irreversibly change repository, filesystem, or system state".into()
            }
        }
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
        assert!(!approvals.resolve("missing", WriteApprovalDecision::AllowAllSession));
        approvals.reset_session();
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
        assert!(preamble.contains("Git commands are not automatically High."));
        assert!(preamble.contains("status, diff, log, and show can be Low"));
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
}
