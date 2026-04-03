mod actions;
mod approvals;
mod ask_user;
mod effects;
pub(crate) mod events;
mod models;
mod reducer;
mod selectors;
mod state;
pub(crate) mod submit;
#[cfg(test)]
pub(crate) mod test_support;
mod transcript;

pub use actions::Action;
pub use approvals::{
    CommandRisk, PendingShellApproval, PendingWriteApproval, ShellApprovalDecision,
    WriteApprovalDecision, default_shell_approval_pattern,
};
pub use ask_user::PendingAskUser;
pub use effects::Effect;
pub(crate) use models::compatible_reasoning_setting;
pub use models::{
    AccessMode, ApprovalMode, EditorInput, EditorKey, SessionHistoryMessage, SideChannelEvent,
    SlashCommand, Speaker, StreamEvent, TurnEndReason,
};
pub(crate) use reducer::apply;
#[cfg(test)]
pub(crate) use selectors::has_visible_pending_content;
pub(crate) use selectors::{
    history_pending_status_label, next_request_context_percent, should_show_history_busy_indicator,
    shows_startup_banner, supported_reasoning_settings,
};
pub use state::SessionState;
pub use transcript::{
    ActivityDisplayState, BackgroundTerminalStatusEntry, ChatMessage, HostedToolKind,
    HostedToolStatusEntry, MainRequestSeed, MessageStyle, PendingReply, PendingReplyActivity,
    PendingReplyKind, PendingReplyReplaySeed, PendingSideReply, ProposedPlanEntry, SideChannelKind,
    SubagentStatusEntry, SubagentStatusKind, ToolCall, ToolResultEntry, TranscriptEntry,
    pending_stream_text_is_visible, startup_banner_message,
};
