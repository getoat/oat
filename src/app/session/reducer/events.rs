use crate::app::session::events::{
    on_background_terminal_event, on_side_channel_event, on_stream_event, on_subagent_event,
};
use crate::app::{Action, AppState, Effect};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::StreamEvent { reply_id, event } => on_stream_event(state, reply_id, event),
        Action::SideChannelEvent { reply_id, event } => {
            on_side_channel_event(state, reply_id, event)
        }
        Action::SubagentEvent(event) => {
            on_subagent_event(state, event);
            None
        }
        Action::BackgroundTerminalEvent(event) => {
            on_background_terminal_event(state, event);
            None
        }
        _ => None,
    }
}
