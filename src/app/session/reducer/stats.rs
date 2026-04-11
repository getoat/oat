use crate::app::{Action, AppState, Effect, ops};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::StatsTabLeft => {
            ops::stats::move_stats_tab(state, -1);
            None
        }
        Action::StatsTabRight => {
            ops::stats::move_stats_tab(state, 1);
            None
        }
        Action::ScrollStatsPageUp => {
            ops::stats::scroll_stats_page_up(state);
            None
        }
        Action::ScrollStatsPageDown => {
            ops::stats::scroll_stats_page_down(state);
            None
        }
        Action::ScrollStatsToTop => {
            ops::stats::scroll_stats_to_top(state);
            None
        }
        Action::ScrollStatsToBottom => {
            ops::stats::scroll_stats_to_bottom(state);
            None
        }
        Action::ScrollStatsUp { lines } => {
            ops::stats::scroll_stats_up(state, lines);
            None
        }
        Action::ScrollStatsDown { lines } => {
            ops::stats::scroll_stats_down(state, lines);
            None
        }
        _ => None,
    }
}
