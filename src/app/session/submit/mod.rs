mod approvals;
mod commands;
mod picker;
mod planning;

use approvals::{submit_ask_user, submit_shell_approval};
use commands::submit_command;
use picker::submit_picker_selection;
pub(crate) use planning::submit_plan_acceptance;
use planning::{submit_plan_review_selection, submit_planning_draft, submit_planning_turn};

use super::{Effect, PendingReplyKind};
use crate::app::{AppState, InputContext, ops, query};
use crate::features::planning::PlanningStage;

pub(crate) fn submit_message(state: &mut AppState) -> Option<Effect> {
    match query::input_context(state) {
        InputContext::WriteApproval => return None,
        InputContext::ShellApproval { .. } => {
            return submit_shell_approval(state);
        }
        InputContext::PlanReview => return submit_plan_review_selection(state),
        InputContext::AskUser { .. } => {
            return submit_ask_user(state);
        }
        InputContext::Picker => return submit_picker_selection(state),
        InputContext::Composer | InputContext::CommandPalette => {}
    }

    let submitted = ops::composer::submitted_composer_text(state);

    if ops::composer::command_query(state).is_some() {
        let command_name = ops::composer::command_name(state)
            .unwrap_or_default()
            .to_owned();
        let arguments = ops::composer::command_arguments(state)
            .unwrap_or_default()
            .to_owned();
        return submit_command(state, &command_name, &arguments);
    }

    if query::planning_draft_mode(state) {
        return submit_planning_draft(state, &submitted);
    }

    if matches!(
        query::planning_session_stage(state),
        Some(PlanningStage::Conversation | PlanningStage::Finalizing)
    ) {
        return submit_planning_turn(state, &submitted);
    }

    if query::has_pending_reply(state) || submitted.is_empty() {
        return None;
    }

    ops::composer::record_submitted_input(state, &submitted);
    ops::planning::clear_plan_review(state);
    ops::transcript::push_user_message(state, submitted.clone());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Normal);

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted,
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{
            PendingReply, PendingReplyKind, SessionHistoryMessage, Speaker, TranscriptEntry,
            session::test_support::new_app,
        },
        ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest},
    };

    fn ask_user_request() -> AskUserRequest {
        AskUserRequest {
            title: Some("Clarify implementation".into()),
            questions: vec![AskUserQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                answers: vec![
                    AskUserAnswer {
                        id: "narrow".into(),
                        label: "Narrow".into(),
                    },
                    AskUserAnswer {
                        id: "broad".into(),
                        label: "Broad".into(),
                    },
                ],
            }],
        }
    }

    #[test]
    fn submit_message_ignores_empty_input() {
        let mut app = new_app(true);

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(app.entries().len(), 1);
        assert!(app.state_mut().session.pending_reply.is_none());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_records_prompt_and_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("hello\nworld");

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "hello\nworld".into(),
                history: Vec::new(),
                history_model_name: None,
            })
        );
        assert_eq!(app.entries().len(), 2);
        match &app.entries()[1] {
            TranscriptEntry::Message(message) => {
                assert_eq!(message.speaker, Speaker::User);
                assert_eq!(message.text, "hello\nworld");
            }
            entry => panic!("expected user message, got {entry:?}"),
        }
        assert!(app.state_mut().session.pending_reply.is_some());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn submit_message_resumes_live_history_follow() {
        let mut app = new_app(true);
        app.state_mut().ui.history.scroll_top = Some(3);
        app.composer_mut().insert_str("hello");

        let _ = app.apply(crate::app::Action::SubmitMessage);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn submit_message_does_nothing_while_reply_is_pending() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(5, PendingReplyKind::Normal));
        app.composer_mut().insert_str("new prompt");

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(app.entries().len(), 1);
        assert!(app.composer_has_content());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_clones_canonical_history_into_effect() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.composer_mut().insert_str("next");

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "next".into(),
                history: vec![SessionHistoryMessage::assistant("previous")],
                history_model_name: None,
            })
        );
    }

    #[test]
    fn consecutive_duplicate_messages_are_collapsed_in_recall_history() {
        let mut app = new_app(true);

        app.composer_mut().insert_str("boo");
        let first = app.apply(crate::app::Action::SubmitMessage);
        assert!(matches!(first, Some(Effect::PromptModel { .. })));
        app.state_mut().session.pending_reply = None;

        app.composer_mut().insert_str("boo");
        let second = app.apply(crate::app::Action::SubmitMessage);
        assert!(matches!(second, Some(Effect::PromptModel { .. })));
        app.state_mut().session.pending_reply = None;

        app.apply(crate::app::Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["boo"]);
        app.apply(crate::app::Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["boo"]);
    }

    #[test]
    fn ask_user_review_submission_returns_resolve_effect() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.begin_ask_user("call-1".into(), ask_user_request());

        let first = app.apply(crate::app::Action::SubmitMessage);
        assert!(first.is_none());
        assert_eq!(app.ask_user_ui().map(|ui| ui.active_tab), Some(1));

        let effect = app.apply(crate::app::Action::SubmitMessage);

        match effect {
            Some(Effect::ResolveAskUser {
                request_id,
                response,
            }) => {
                assert_eq!(request_id, "call-1");
                assert_eq!(response.questions.len(), 1);
                assert_eq!(response.questions[0].selected_answer.label, "Narrow");
            }
            other => panic!("expected ResolveAskUser effect, got {other:?}"),
        }
        assert!(!app.has_pending_ask_user());
    }
}
