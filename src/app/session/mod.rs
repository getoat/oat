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
pub(crate) use models::compatible_reasoning_effort;
pub use models::{
    AccessMode, ApprovalMode, EditorInput, EditorKey, SessionHistoryMessage, SlashCommand, Speaker,
    StreamEvent,
};
pub(crate) use reducer::apply;
#[cfg(test)]
pub(crate) use selectors::has_visible_pending_content;
pub(crate) use selectors::{
    current_model_info, history_pending_status_label, next_request_context_percent,
    should_show_history_busy_indicator, shows_startup_banner, supported_reasoning_levels,
};
pub use state::SessionState;
pub use transcript::{
    ChatMessage, MessageStyle, PendingReply, PendingReplyKind, PendingReplyReplaySeed,
    ProposedPlanEntry, SubagentDisplayState, SubagentStatusEntry, SubagentStatusKind, ToolCall,
    ToolResultEntry, TranscriptEntry, pending_stream_text_is_visible, startup_banner_message,
};
