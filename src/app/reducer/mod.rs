mod approvals;
mod events;
mod history;
mod input;
mod planning;
mod system;

use crate::app::{Action, AppState, Effect};

pub(crate) fn apply(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::ClearComposerOrQuit
        | Action::CancelPendingReply
        | Action::ToggleMode
        | Action::Tick => system::handle(state, action),
        Action::SelectPreviousCommand
        | Action::SelectNextCommand
        | Action::InsertComposerNewline
        | Action::SubmitMessage
        | Action::TogglePickerSelection
        | Action::PickerTabLeft
        | Action::PickerTabRight
        | Action::AskUserTabLeft
        | Action::AskUserTabRight
        | Action::AskUserToggleDetailEditor
        | Action::ShellApprovalToggleDetailEditor
        | Action::Editor(_)
        | Action::Paste(_) => input::handle(state, action),
        Action::ScrollHistoryPageUp
        | Action::ScrollHistoryPageDown
        | Action::ScrollHistoryToTop
        | Action::ScrollHistoryToBottom
        | Action::ScrollHistoryUp { .. }
        | Action::ScrollHistoryDown { .. }
        | Action::StartHistorySelection { .. }
        | Action::UpdateHistorySelection { .. }
        | Action::FinishHistorySelection { .. } => history::handle(state, action),
        Action::ApproveWriteOnce | Action::ApproveWriteAllSession | Action::DenyWrite => {
            approvals::handle(state, action)
        }
        Action::AcceptPlanAndImplement | Action::SuggestPlanChanges => {
            planning::handle(state, action)
        }
        Action::StreamEvent { .. } | Action::SubagentEvent(_) => events::handle(state, action),
    }
}
