use crate::app::AppState;

pub(crate) fn scroll_history_page_up(state: &mut AppState) {
    scroll_history_up(state, state.ui.history.page_rows());
}

pub(crate) fn scroll_history_page_down(state: &mut AppState) {
    scroll_history_down(state, state.ui.history.page_rows());
}

pub(crate) fn scroll_history_up(state: &mut AppState, lines: usize) {
    state.ui.history.scroll_up(lines);
}

pub(crate) fn scroll_history_down(state: &mut AppState, lines: usize) {
    state.ui.history.scroll_down(lines);
}

pub(crate) fn scroll_history_to_top(state: &mut AppState) {
    state.ui.history.scroll_to_top();
}

pub(crate) fn resume_history_follow(state: &mut AppState) {
    state.ui.history.resume_follow();
}

pub(crate) fn start_history_selection(state: &mut AppState, column: u16, row: u16) {
    state.ui.history.start_selection(column, row);
}

pub(crate) fn update_history_selection(state: &mut AppState, column: u16, row: u16) {
    state.ui.history.update_selection(column, row);
}

pub(crate) fn finish_history_selection(
    state: &mut AppState,
    column: u16,
    row: u16,
) -> Option<String> {
    state.ui.history.finish_selection(column, row)
}
