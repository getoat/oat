use crate::{
    app::{ActivityDisplayState, AppState, ops},
    background_terminals::BackgroundTerminalUiEvent,
};

pub(crate) fn on_background_terminal_event(state: &mut AppState, event: BackgroundTerminalUiEvent) {
    match event {
        BackgroundTerminalUiEvent::Spawned {
            id,
            label,
            cwd,
            pid,
        } => {
            let detail_text = pid.map(|pid| format!("cwd: {cwd}  pid: {pid}"));
            let detail_text = detail_text.or_else(|| Some(format!("cwd: {cwd}")));
            ops::transcript::upsert_background_terminal_status(
                state,
                id,
                label,
                ActivityDisplayState::Running,
                "running".into(),
                detail_text,
            );
        }
        BackgroundTerminalUiEvent::StateChanged {
            id,
            label,
            state: display_state,
            status_text,
            detail_text,
        } => {
            ops::transcript::upsert_background_terminal_status(
                state,
                id,
                label,
                display_state,
                status_text,
                detail_text,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{
            Action, ActivityDisplayState, TranscriptEntry, query, session::test_support::new_app,
        },
        background_terminals::BackgroundTerminalUiEvent,
    };

    #[test]
    fn spawned_terminal_event_creates_status_entry() {
        let mut app = new_app(true);

        app.apply(Action::BackgroundTerminalEvent(
            BackgroundTerminalUiEvent::Spawned {
                id: "terminal-1".into(),
                label: "dev server".into(),
                cwd: "src".into(),
                pid: Some(42),
            },
        ));

        let TranscriptEntry::BackgroundTerminalStatus(status) =
            app.entries().last().expect("status entry")
        else {
            panic!("expected background terminal status");
        };
        assert_eq!(status.display_label, "dev server");
        assert_eq!(status.state, ActivityDisplayState::Running);
        assert!(
            status
                .detail_text
                .as_deref()
                .is_some_and(|text| text.contains("pid: 42"))
        );
        assert_eq!(query::active_background_terminal_count(app.state()), 1);
    }

    #[test]
    fn cancelled_terminal_event_removes_terminal_from_active_count() {
        let mut app = new_app(true);

        app.apply(Action::BackgroundTerminalEvent(
            BackgroundTerminalUiEvent::Spawned {
                id: "terminal-1".into(),
                label: "dev server".into(),
                cwd: "src".into(),
                pid: Some(42),
            },
        ));
        assert_eq!(query::active_background_terminal_count(app.state()), 1);

        app.apply(Action::BackgroundTerminalEvent(
            BackgroundTerminalUiEvent::StateChanged {
                id: "terminal-1".into(),
                label: "dev server".into(),
                state: ActivityDisplayState::Cancelled,
                status_text: "cancelled".into(),
                detail_text: Some("cwd: src".into()),
            },
        ));

        assert_eq!(query::active_background_terminal_count(app.state()), 0);
    }
}
