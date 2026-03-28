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

    if submitted.is_empty() {
        return None;
    }

    if query::has_pending_reply(state) {
        ops::composer::record_submitted_input(state, &submitted);
        ops::session::enqueue_queued_message(state, submitted);
        ops::history::resume_history_follow(state);
        ops::composer::clear_composer(state);
        return None;
    }

    ops::composer::clear_composer(state);
    submit_normal_message_text(state, submitted, true)
}

pub(crate) fn dispatch_next_queued_message_if_ready(state: &mut AppState) -> Option<Effect> {
    if !query::queue_dispatch_ready(state) {
        return None;
    }

    let prompt = ops::session::dequeue_queued_message(state)?;
    submit_normal_message_text(state, prompt, false)
}

fn submit_normal_message_text(
    state: &mut AppState,
    prompt: String,
    record_recall_history: bool,
) -> Option<Effect> {
    if prompt.is_empty() || query::has_pending_reply(state) {
        return None;
    }

    let session_title_prompt = should_request_session_title(state).then(|| prompt.clone());
    if record_recall_history {
        ops::composer::record_submitted_input(state, &prompt);
    }
    ops::planning::clear_plan_review(state);
    ops::transcript::push_user_message(state, prompt.clone());
    ops::history::resume_history_follow(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Normal);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
        session_title_prompt,
    })
}

pub(super) fn should_request_session_title(state: &AppState) -> bool {
    query::shows_startup_banner_state(state)
        && state.session.pending_session_title_reply_id.is_none()
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
                session_title_prompt: Some("hello\nworld".into()),
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
        assert!(!app.composer_has_content());
        assert_eq!(
            app.state().session.queued_messages,
            std::collections::VecDeque::from(["new prompt".to_string()])
        );
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
                session_title_prompt: None,
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
    fn queued_message_dispatches_in_fifo_order() {
        let mut app = new_app(true);
        app.state_mut()
            .session
            .queued_messages
            .push_back("first".into());
        app.state_mut()
            .session
            .queued_messages
            .push_back("second".into());

        let first = dispatch_next_queued_message_if_ready(app.state_mut());
        assert!(matches!(
            first,
            Some(Effect::PromptModel {
                prompt,
                reply_id: 1,
                ..
            }) if prompt == "first"
        ));
        assert_eq!(
            app.state().session.queued_messages,
            std::collections::VecDeque::from(["second".to_string()])
        );

        app.state_mut().session.pending_reply = None;
        let second = dispatch_next_queued_message_if_ready(app.state_mut());
        assert!(matches!(
            second,
            Some(Effect::PromptModel {
                prompt,
                reply_id: 2,
                ..
            }) if prompt == "second"
        ));
        assert!(app.state().session.queued_messages.is_empty());
    }

    #[test]
    fn queued_dispatch_waits_for_plain_composer_context() {
        let mut app = new_app(true);
        app.state_mut()
            .session
            .queued_messages
            .push_back("next".into());
        app.composer_mut().insert_str("/model");
        app.sync_command_selection();

        let effect = dispatch_next_queued_message_if_ready(app.state_mut());

        assert!(effect.is_none());
        assert_eq!(
            app.state().session.queued_messages,
            std::collections::VecDeque::from(["next".to_string()])
        );
    }

    #[test]
    fn queued_dispatch_does_not_double_record_recall_history() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(5, PendingReplyKind::Normal));
        app.composer_mut().insert_str("queued once");

        let queued = app.apply(crate::app::Action::SubmitMessage);
        assert!(queued.is_none());
        assert_eq!(
            app.state().session.queued_messages,
            std::collections::VecDeque::from(["queued once".to_string()])
        );

        app.state_mut().session.pending_reply = None;
        let dispatched = dispatch_next_queued_message_if_ready(app.state_mut());
        assert!(matches!(dispatched, Some(Effect::PromptModel { .. })));
        app.state_mut().session.pending_reply = None;

        app.apply(crate::app::Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["queued once"]);
        app.apply(crate::app::Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["queued once"]);
    }

    #[test]
    fn queued_dispatch_can_start_immediately_after_cancellation() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(3, PendingReplyKind::Normal));
        app.state_mut()
            .session
            .queued_messages
            .push_back("next prompt".into());

        let cancel = app.apply(crate::app::Action::CancelPendingReply);
        assert_eq!(cancel, Some(Effect::CancelPendingReply));

        let dispatch = dispatch_next_queued_message_if_ready(app.state_mut());
        assert!(matches!(
            dispatch,
            Some(Effect::PromptModel { prompt, .. }) if prompt == "next prompt"
        ));
    }

    #[test]
    fn queued_dispatch_does_not_clobber_a_new_composer_draft() {
        let mut app = new_app(true);
        app.state_mut()
            .session
            .queued_messages
            .push_back("queued".into());
        app.composer_mut().insert_str("fresh draft");

        let effect = dispatch_next_queued_message_if_ready(app.state_mut());

        assert!(matches!(
            effect,
            Some(Effect::PromptModel { prompt, .. }) if prompt == "queued"
        ));
        assert_eq!(app.composer().lines(), ["fresh draft"]);
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
