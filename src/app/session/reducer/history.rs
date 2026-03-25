use crate::app::{Action, AppState, Effect, ops};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::ScrollHistoryPageUp => {
            ops::history::scroll_history_page_up(state);
            None
        }
        Action::ScrollHistoryPageDown => {
            ops::history::scroll_history_page_down(state);
            None
        }
        Action::ScrollHistoryToTop => {
            ops::history::scroll_history_to_top(state);
            None
        }
        Action::ScrollHistoryToBottom => {
            ops::history::resume_history_follow(state);
            None
        }
        Action::ScrollHistoryUp { lines } => {
            ops::history::scroll_history_up(state, lines);
            None
        }
        Action::ScrollHistoryDown { lines } => {
            ops::history::scroll_history_down(state, lines);
            None
        }
        Action::StartHistorySelection { column, row } => {
            ops::history::start_history_selection(state, column, row);
            None
        }
        Action::UpdateHistorySelection { column, row } => {
            ops::history::update_history_selection(state, column, row);
            None
        }
        Action::FinishHistorySelection { column, row } => {
            ops::history::finish_history_selection(state, column, row)
                .map(|text| Effect::CopyToClipboard { text })
        }
        _ => None,
    }
}
