use crate::app::{Action, AppState, Effect, ops};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::ClearComposerOrQuit => {
            if state.session.pending_reply.is_some() {
                ops::session::cancel_pending_reply(state);
                ops::transcript::push_error_message(state, "Request cancelled.");
                Some(Effect::CancelPendingReply)
            } else if ops::composer::composer_has_content(state) {
                ops::composer::clear_composer(state);
                None
            } else {
                ops::session::set_should_quit(state);
                None
            }
        }
        Action::CancelPendingReply => {
            if ops::approvals::cancel_shell_approval_editing(state) {
                None
            } else if state.session.pending_reply.is_some() {
                ops::session::cancel_pending_reply(state);
                ops::transcript::push_error_message(state, "Request cancelled.");
                Some(Effect::CancelPendingReply)
            } else if ops::picker::cancel_picker(state) {
                None
            } else if ops::planning::cancel_planning_draft_mode(state) {
                ops::transcript::push_agent_message(state, "Planning draft cancelled.");
                None
            } else {
                None
            }
        }
        Action::ToggleMode => {
            state.session.mode.toggle();
            Some(Effect::RebuildLlm {
                access_mode: state.session.mode,
            })
        }
        Action::Tick => {
            state.session.tick_count = state.session.tick_count.wrapping_add(1);
            None
        }
        _ => None,
    }
}
