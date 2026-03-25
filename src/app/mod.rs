pub mod session;
pub mod ui;

pub(crate) mod ops;
pub(crate) mod query;
mod reducer;
mod shell;
mod state;

pub use query::InputTarget;
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
pub use state::AppState;
pub use ui::{
    ModelPickerTab, PickerSelection, ReasoningPickerTarget, SelectionPicker, ShellApprovalEditMode,
    UiState,
};

pub type AppAction = Action;
pub type AppEffect = Effect;
