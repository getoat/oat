pub mod session;
pub mod ui;

mod reducer_context;
mod shell;

pub(crate) use reducer_context::ReducerContext;
pub(crate) use session::compatible_reasoning_effort;
pub use session::{
    AccessMode, Action, ApprovalMode, ChatMessage, CommandRisk, EditorInput, EditorKey, Effect,
    MessageStyle, PendingAskUser, PendingReply, PendingReplyKind, PendingReplyReplaySeed,
    PendingShellApproval, PendingWriteApproval, SessionHistoryMessage, SessionState,
    ShellApprovalDecision, SlashCommand, Speaker, StreamEvent, SubagentDisplayState,
    SubagentStatusEntry, SubagentStatusKind, ToolCall, ToolResultEntry, TranscriptEntry,
    WriteApprovalDecision,
};
pub use shell::App;
pub use shell::App as AppShell;
pub use ui::{
    ModelPickerTab, PickerSelection, ReasoningPickerTarget, SelectionPicker, ShellApprovalEditMode,
    UiState,
};

pub type AppAction = Action;
pub type AppEffect = Effect;
