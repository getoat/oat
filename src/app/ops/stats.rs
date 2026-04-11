use crate::{
    app::{AppState, StatsScreenState},
    stats::StatsReport,
};

pub(crate) fn open_stats_screen(state: &mut AppState, report: StatsReport) {
    state.ui.stats_screen = Some(StatsScreenState::new(report));
}

pub(crate) fn close_stats_screen(state: &mut AppState) -> bool {
    state.ui.stats_screen.take().is_some()
}

pub(crate) fn move_stats_tab(state: &mut AppState, direction: isize) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    screen.active_tab.toggle(direction);
}

pub(crate) fn move_stats_selection(state: &mut AppState, direction: isize) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        table.move_selection(direction);
    }
}

pub(crate) fn scroll_stats_page_up(state: &mut AppState) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        table.page_up();
    }
}

pub(crate) fn scroll_stats_page_down(state: &mut AppState) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        table.page_down();
    }
}

pub(crate) fn scroll_stats_to_top(state: &mut AppState) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        table.scroll_to_top();
    }
}

pub(crate) fn scroll_stats_to_bottom(state: &mut AppState) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        table.scroll_to_bottom();
    }
}

pub(crate) fn scroll_stats_up(state: &mut AppState, lines: usize) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        for _ in 0..lines {
            table.move_selection(-1);
        }
    }
}

pub(crate) fn scroll_stats_down(state: &mut AppState, lines: usize) {
    let Some(screen) = state.ui.stats_screen.as_mut() else {
        return;
    };
    if let Some(table) = screen.active_table_mut() {
        for _ in 0..lines {
            table.move_selection(1);
        }
    }
}
