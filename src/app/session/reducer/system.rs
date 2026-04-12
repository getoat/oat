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
            } else if ops::stats::close_stats_screen(state) {
                None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        Action, MainRequestSeed, MessageStyle, PendingReply, PendingReplyKind, TranscriptEntry,
        session::test_support::{new_app, registry_app},
    };
    use crate::llm::history_into_rig;
    use crate::stats::StatsReport;
    use rig::completion::{Message as RigMessage, message::AssistantContent};

    #[test]
    fn clear_composer_or_quit_quits_when_composer_is_empty() {
        let mut app = new_app(true);

        app.apply(Action::ClearComposerOrQuit);

        assert!(app.state_mut().session.should_quit);
    }

    #[test]
    fn clear_composer_or_quit_clears_composer_before_quitting() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("draft");

        app.apply(Action::ClearComposerOrQuit);

        assert!(!app.state_mut().session.should_quit);
        assert!(!app.composer_has_content());
    }

    #[test]
    fn clear_composer_or_quit_cancels_pending_reply_instead_of_quitting() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        let effect = app.apply(Action::ClearComposerOrQuit);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(!app.state_mut().session.should_quit);
        assert!(app.state_mut().session.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("cancel message")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
    }

    #[test]
    fn explicit_cancel_pending_reply_adds_cancellation_message() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        let effect = app.apply(Action::CancelPendingReply);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(app.state_mut().session.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("cancel message")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
    }

    #[test]
    fn explicit_cancel_pending_reply_drops_incomplete_tool_calls_from_history() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.state_mut().session.active_main_request_seed = Some(MainRequestSeed {
            history: vec![crate::app::SessionHistoryMessage::assistant("old")],
            visible_prompt: "continue".into(),
            model_prompt: "continue".into(),
            history_model_name: None,
            transcript_len_before: 0,
        });
        crate::app::ops::session::initialize_pending_reply_history(app.state_mut());
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::app::StreamEvent::TextDelta("checking files".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::app::StreamEvent::ToolCall {
                name: "ReadFile".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            },
        });

        let effect = app.apply(Action::CancelPendingReply);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        let rig_history = history_into_rig(app.session_history().to_vec()).expect("rig history");
        assert_eq!(
            rig_history,
            vec![
                RigMessage::assistant("old"),
                RigMessage::user("continue"),
                RigMessage::assistant("checking files"),
            ]
        );
        assert!(
            !rig_history
                .iter()
                .any(|message| matches!(message, RigMessage::Assistant { content, .. } if matches!(content.first_ref(), AssistantContent::ToolCall(_)))),
            "cancelled history should not retain unmatched tool calls"
        );
    }

    #[test]
    fn escape_action_closes_active_picker() {
        let mut app = registry_app(true);
        app.open_model_picker();

        let effect = app.apply(Action::CancelPendingReply);

        assert!(effect.is_none());
        assert!(!app.selection_picker_visible());
    }

    #[test]
    fn escape_action_closes_active_stats_screen() {
        let mut app = registry_app(true);
        crate::app::ops::stats::open_stats_screen(
            app.state_mut(),
            StatsReport {
                current: Default::default(),
                historical: Default::default(),
                current_models: Default::default(),
                historical_models: Default::default(),
                historical_session_count: 0,
            },
        );

        let effect = app.apply(Action::CancelPendingReply);

        assert!(effect.is_none());
        assert!(!crate::app::query::stats_screen_visible(app.state()));
    }
}
