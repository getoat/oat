use crate::{
    features::planning::pending_plain_text_is_visible, todo::TodoSnapshot, tools::MutationPreview,
};
use std::collections::{HashSet, VecDeque};

use rig::{
    OneOrMany,
    completion::{
        Message as RigMessage,
        message::{AssistantContent, ToolResultContent, UserContent},
    },
};
use serde::{Deserialize, Serialize};

use super::{
    SessionHistoryMessage,
    models::{AccessMode, Speaker},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub speaker: Speaker,
    pub text: String,
    pub style: MessageStyle,
    pub tag: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum MessageStyle {
    Plain,
    Commentary,
    Thinking,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub parameter: String,
    pub preview: Option<MutationPreview>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolResultEntry {
    pub name: String,
    pub output: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum HostedToolKind {
    Search,
    OpenPage,
    FindInPage,
}

impl HostedToolKind {
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Search => "Web search",
            Self::OpenPage => "Open page",
            Self::FindInPage => "Find in page",
        }
    }

    pub fn transcript_prefix(self) -> &'static str {
        match self {
            Self::Search => "● search",
            Self::OpenPage => "● open",
            Self::FindInPage => "● find",
        }
    }

    pub fn action_label(self, state: ActivityDisplayState) -> &'static str {
        match (self, state) {
            (Self::Search, ActivityDisplayState::Running) => "Searching the web",
            (Self::Search, ActivityDisplayState::Completed) => "Searched the web",
            (Self::Search, ActivityDisplayState::Failed) => "Web search failed",
            (Self::Search, ActivityDisplayState::Cancelled) => "Web search cancelled",
            (Self::OpenPage, ActivityDisplayState::Running) => "Opening page",
            (Self::OpenPage, ActivityDisplayState::Completed) => "Opened page",
            (Self::OpenPage, ActivityDisplayState::Failed) => "Page open failed",
            (Self::OpenPage, ActivityDisplayState::Cancelled) => "Page open cancelled",
            (Self::FindInPage, ActivityDisplayState::Running) => "Finding in page",
            (Self::FindInPage, ActivityDisplayState::Completed) => "Found in page",
            (Self::FindInPage, ActivityDisplayState::Failed) => "Find in page failed",
            (Self::FindInPage, ActivityDisplayState::Cancelled) => "Find in page cancelled",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostedToolStatusEntry {
    pub id: String,
    pub kind: HostedToolKind,
    pub state: ActivityDisplayState,
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProposedPlanEntry {
    pub markdown: String,
    pub raw_block: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum SubagentStatusKind {
    Subagent,
    Planning,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum ActivityDisplayState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SubagentStatusEntry {
    pub id: String,
    pub kind: SubagentStatusKind,
    pub display_label: String,
    pub state: ActivityDisplayState,
    pub status_text: String,
    pub latest_tool_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BackgroundTerminalStatusEntry {
    pub id: String,
    pub display_label: String,
    pub state: ActivityDisplayState,
    pub status_text: String,
    pub detail_text: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum TranscriptEntry {
    Message(ChatMessage),
    ProposedPlan(ProposedPlanEntry),
    ToolCall(ToolCall),
    ToolResult(ToolResultEntry),
    HostedToolStatus(HostedToolStatusEntry),
    TodoSnapshot(TodoSnapshot),
    SubagentStatus(SubagentStatusEntry),
    BackgroundTerminalStatus(BackgroundTerminalStatusEntry),
}

#[derive(Debug)]
pub struct PendingReply {
    pub id: u64,
    pub kind: PendingReplyKind,
    pub activity: PendingReplyActivity,
    pub reasoning_entry_index: Option<usize>,
    pub text_entry_index: Option<usize>,
    pub staged_reasoning_text: String,
    pub staged_plain_text: String,
    pub plain_text: String,
    pub reasoning_text: String,
    pub commentary_messages: Vec<String>,
    pub has_visible_content: bool,
    canonical_turn_messages: Vec<RigMessage>,
    pending_tool_calls: VecDeque<PendingToolCallLink>,
    next_tool_call_sequence: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingToolCallLink {
    id: String,
    call_id: String,
    name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MainRequestSeed {
    pub history: Vec<SessionHistoryMessage>,
    pub visible_prompt: String,
    pub model_prompt: String,
    pub history_model_name: Option<String>,
    pub transcript_len_before: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SideChannelKind {
    Btw,
}

impl SideChannelKind {
    pub fn label_prefix(self) -> &'static str {
        match self {
            Self::Btw => "btw",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingSideReply {
    pub kind: SideChannelKind,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingReplyReplaySeed {
    pub plain_text: String,
    pub reasoning_text: String,
    pub commentary_messages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingReplyKind {
    Normal,
    Planning,
    Compacting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingReplyActivity {
    Starting,
    Responding,
    Thinking,
    SearchingWeb,
    WaitingForTool,
    WaitingForApproval,
    WaitingForInput,
}

impl PendingReplyActivity {
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Responding => "Responding",
            Self::Thinking => "thinking",
            Self::SearchingWeb => "Searching the web",
            Self::WaitingForTool => "Waiting for tool",
            Self::WaitingForApproval => "Waiting for approval",
            Self::WaitingForInput => "Waiting for input",
        }
    }
}

impl PendingReply {
    pub fn new(id: u64, kind: PendingReplyKind) -> Self {
        Self {
            id,
            kind,
            activity: PendingReplyActivity::Starting,
            reasoning_entry_index: None,
            text_entry_index: None,
            staged_reasoning_text: String::new(),
            staged_plain_text: String::new(),
            plain_text: String::new(),
            reasoning_text: String::new(),
            commentary_messages: Vec::new(),
            has_visible_content: false,
            canonical_turn_messages: Vec::new(),
            pending_tool_calls: VecDeque::new(),
            next_tool_call_sequence: 1,
        }
    }

    pub fn reset_active_stream_segment(&mut self) {
        self.reasoning_entry_index = None;
        self.text_entry_index = None;
        self.staged_reasoning_text.clear();
        self.staged_plain_text.clear();
    }

    pub fn initialize_canonical_turn(&mut self, visible_prompt: &str) {
        self.canonical_turn_messages = vec![RigMessage::user(visible_prompt)];
        self.pending_tool_calls.clear();
        self.next_tool_call_sequence = 1;
    }

    pub fn canonical_turn_messages(&self) -> &[RigMessage] {
        &self.canonical_turn_messages
    }

    pub fn safe_canonical_turn_messages(&self) -> Vec<RigMessage> {
        let pending_ids = self
            .pending_tool_calls
            .iter()
            .map(|tool_call| tool_call.id.as_str())
            .collect::<HashSet<_>>();
        let pending_call_ids = self
            .pending_tool_calls
            .iter()
            .map(|tool_call| tool_call.call_id.as_str())
            .collect::<HashSet<_>>();

        self.canonical_turn_messages
            .iter()
            .filter(|message| {
                !matches!(
                    message,
                    RigMessage::Assistant { content, .. }
                        if content.len() == 1
                            && matches!(
                                content.first_ref(),
                                AssistantContent::ToolCall(tool_call)
                                    if pending_ids.contains(tool_call.id.as_str())
                                        || tool_call
                                            .call_id
                                            .as_deref()
                                            .is_some_and(|call_id| pending_call_ids.contains(call_id))
                            )
                )
            })
            .cloned()
            .collect()
    }

    pub fn append_canonical_assistant_text(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }

        if let Some(RigMessage::Assistant { content, .. }) = self.canonical_turn_messages.last_mut()
        {
            let is_single_text = content.len() == 1;
            if is_single_text && let AssistantContent::Text(text) = content.first_mut() {
                text.text.push_str(delta);
                return;
            }
        }

        self.canonical_turn_messages
            .push(RigMessage::assistant(delta));
    }

    pub fn push_canonical_tool_call(&mut self, name: &str, arguments: &str) {
        let arguments = serde_json::from_str(arguments)
            .unwrap_or_else(|_| serde_json::Value::String(arguments.to_string()));
        let sequence = self.next_tool_call_sequence;
        self.next_tool_call_sequence += 1;

        let id = format!("oat_tool_call_{sequence}");
        let call_id = format!("oat_call_{sequence}");
        self.pending_tool_calls.push_back(PendingToolCallLink {
            id: id.clone(),
            call_id: call_id.clone(),
            name: name.to_string(),
        });
        self.canonical_turn_messages.push(RigMessage::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::tool_call_with_call_id(
                id,
                call_id,
                name.to_string(),
                arguments,
            )),
        });
    }

    pub fn push_canonical_tool_result(&mut self, name: &str, output: &str) -> bool {
        let Some(tool_call) = self
            .pending_tool_calls
            .iter()
            .position(|tool_call| tool_call.name == name)
            .and_then(|index| self.pending_tool_calls.remove(index))
            .or_else(|| self.pending_tool_calls.pop_front())
        else {
            return false;
        };

        self.canonical_turn_messages.push(RigMessage::User {
            content: OneOrMany::one(UserContent::tool_result_with_call_id(
                tool_call.id,
                tool_call.call_id,
                OneOrMany::one(ToolResultContent::text(output)),
            )),
        });
        true
    }
}

pub fn startup_banner_message(model_name: &str, mode: AccessMode) -> String {
    let _ = mode;
    let provider = crate::model_registry::find_model(model_name)
        .map(|model| model.provider.display_name())
        .unwrap_or("configured");
    let display_name = crate::codex::display_name(model_name);
    format!(
        "Loaded {provider} model `{display_name}` from config. Send a message to start a one-shot response, or type / for commands."
    )
}

pub fn pending_stream_text_is_visible(style: MessageStyle, text: &str) -> bool {
    match style {
        MessageStyle::Plain => pending_plain_text_is_visible(text),
        MessageStyle::Commentary | MessageStyle::Thinking => !text.trim().is_empty(),
        MessageStyle::Error => false,
    }
}
