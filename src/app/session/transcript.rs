use crate::{
    features::planning::pending_plain_text_is_visible, todo::TodoSnapshot, tools::MutationPreview,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MainRequestSeed {
    pub history: Vec<SessionHistoryMessage>,
    pub visible_prompt: String,
    pub model_prompt: String,
    pub history_model_name: Option<String>,
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
        }
    }

    pub fn reset_active_stream_segment(&mut self) {
        self.reasoning_entry_index = None;
        self.text_entry_index = None;
        self.staged_reasoning_text.clear();
        self.staged_plain_text.clear();
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
