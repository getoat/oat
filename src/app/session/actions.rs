use crate::{background_terminals::BackgroundTerminalUiEvent, subagents::SubagentUiEvent};

use super::{EditorInput, SideChannelEvent, StreamEvent};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ClearComposerOrQuit,
    CancelPendingReply,
    ToggleMode,
    SelectPreviousCommand,
    SelectNextCommand,
    ScrollHistoryPageUp,
    ScrollHistoryPageDown,
    ScrollHistoryToTop,
    ScrollHistoryToBottom,
    ScrollHistoryUp {
        lines: usize,
    },
    ScrollHistoryDown {
        lines: usize,
    },
    InsertComposerNewline,
    SubmitMessage,
    TogglePickerSelection,
    PickerTabLeft,
    PickerTabRight,
    StatsTabLeft,
    StatsTabRight,
    AskUserTabLeft,
    AskUserTabRight,
    AskUserToggleDetailEditor,
    ShellApprovalToggleDetailEditor,
    ScrollStatsPageUp,
    ScrollStatsPageDown,
    ScrollStatsToTop,
    ScrollStatsToBottom,
    ScrollStatsUp {
        lines: usize,
    },
    ScrollStatsDown {
        lines: usize,
    },
    ApproveWriteOnce,
    ApproveWriteAllSession,
    DenyWrite,
    AcceptPlanAndImplement,
    SuggestPlanChanges,
    Editor(EditorInput),
    Paste(String),
    StartHistorySelection {
        column: u16,
        row: u16,
    },
    UpdateHistorySelection {
        column: u16,
        row: u16,
    },
    FinishHistorySelection {
        column: u16,
        row: u16,
    },
    StreamEvent {
        reply_id: u64,
        event: StreamEvent,
    },
    SideChannelEvent {
        reply_id: u64,
        event: SideChannelEvent,
    },
    SubagentEvent(SubagentUiEvent),
    BackgroundTerminalEvent(BackgroundTerminalUiEvent),
    Tick,
}
