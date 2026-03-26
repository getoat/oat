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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::new_app;

    #[test]
    fn page_up_pins_history_above_live_tail() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);

        app.apply(Action::ScrollHistoryPageUp);

        assert_eq!(app.state_mut().ui.history.scroll_top, Some(20));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn page_down_clamps_at_bottom_without_resuming_follow() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);
        app.state_mut().ui.history.scroll_top = Some(24);

        app.apply(Action::ScrollHistoryPageDown);

        assert_eq!(app.state_mut().ui.history.scroll_top, Some(25));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn jump_to_bottom_resumes_live_follow() {
        let mut app = new_app(true);
        app.state_mut().ui.history.scroll_top = Some(7);

        app.apply(Action::ScrollHistoryToBottom);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn line_scroll_clamps_to_history_bounds() {
        let mut app = new_app(true);
        app.sync_history_viewport(18, 6);
        app.state_mut().ui.history.scroll_top = Some(2);

        app.apply(Action::ScrollHistoryUp { lines: 10 });
        assert_eq!(app.state_mut().ui.history.scroll_top, Some(0));

        app.apply(Action::ScrollHistoryDown { lines: 20 });
        assert_eq!(app.state_mut().ui.history.scroll_top, Some(12));
    }

    #[test]
    fn finishing_history_selection_returns_copy_effect() {
        let mut app = new_app(true);
        app.update_history_snapshot_for_test(0, 0, 20, 2, vec!["alpha".into(), "beta".into()]);

        assert!(
            app.apply(Action::StartHistorySelection { column: 1, row: 0 })
                .is_none()
        );
        let effect = app.apply(Action::FinishHistorySelection { column: 2, row: 1 });

        assert_eq!(
            effect,
            Some(Effect::CopyToClipboard {
                text: "lpha\nbet".into(),
            })
        );
    }
}
