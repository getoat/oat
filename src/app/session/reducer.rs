mod approvals;
mod events;
mod history;
mod input;
mod planning;
mod system;

use super::{Action, Effect};
use crate::app::AppState;

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
        Action::StreamEvent { .. }
        | Action::SideChannelEvent { .. }
        | Action::SubagentEvent(_)
        | Action::BackgroundTerminalEvent(_) => events::handle(state, action),
    }
}

#[cfg(test)]
mod tests {
    use super::apply;
    use crate::{app::Action, app::session::test_support::new_app};

    #[test]
    fn apply_routes_tick_to_system_handler() {
        let mut app = new_app(true);

        let effect = apply(app.state_mut(), Action::Tick);

        assert!(effect.is_none());
        assert_eq!(app.state().session.tick_count, 1);
    }

    #[test]
    fn apply_routes_history_actions_to_history_handler() {
        let mut app = new_app(true);
        app.state_mut().ui.history.scroll_top = Some(7);

        let effect = apply(app.state_mut(), Action::ScrollHistoryToBottom);

        assert!(effect.is_none());
        assert!(!app.history_is_pinned());
    }
}
