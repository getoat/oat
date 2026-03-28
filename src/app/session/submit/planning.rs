use super::super::{Effect, PendingReplyKind};
use super::should_request_session_title;
use crate::app::{AccessMode, AppState, ops, query};
use crate::features::planning::planning_conversation_prompt;

pub(crate) fn submit_plan_acceptance(state: &mut AppState) -> Option<Effect> {
    if query::has_pending_reply(state) || !query::plan_review_selection_active(state) {
        return None;
    }

    let visible_prompt = accepted_plan_prompt().to_string();
    let prompt = accepted_plan_implementation_prompt(state);
    ops::composer::record_submitted_input(state, &visible_prompt);
    ops::planning::accept_plan_review_for_implementation(state);
    state.session.mode = AccessMode::ReadWrite;
    ops::transcript::push_user_message(state, visible_prompt);
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Normal);
    ops::session::set_active_main_request_seed(state, Vec::new(), prompt.clone(), None);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: Vec::new(),
        history_model_name: None,
        session_title_prompt: None,
    })
}

pub(super) fn submit_plan_review_selection(state: &mut AppState) -> Option<Effect> {
    match query::selected_plan_review_index(state).unwrap_or(0) {
        0 => submit_plan_acceptance(state),
        1 => {
            ops::planning::begin_plan_discussion(state);
            None
        }
        _ => None,
    }
}

pub(super) fn submit_planning_draft(state: &mut AppState, submitted: &str) -> Option<Effect> {
    if query::has_pending_reply(state) || submitted.is_empty() {
        return None;
    }

    let session_title_prompt = should_request_session_title(state).then(|| submitted.to_string());
    ops::planning::consume_planning_draft_mode(state);
    ops::composer::record_submitted_input(state, submitted);
    ops::transcript::push_user_message(state, submitted.to_string());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);
    let prompt = planning_conversation_prompt(submitted);
    ops::session::set_active_main_request_seed(
        state,
        state.session.session_history.to_vec(),
        prompt.clone(),
        state.session.last_history_model_name.clone(),
    );

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
        session_title_prompt,
    })
}

pub(super) fn submit_planning_turn(state: &mut AppState, submitted: &str) -> Option<Effect> {
    if query::has_pending_reply(state) || submitted.is_empty() {
        return None;
    }

    let session_title_prompt = should_request_session_title(state).then(|| submitted.to_string());
    ops::composer::record_submitted_input(state, submitted);
    ops::transcript::push_user_message(state, submitted.to_string());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);
    ops::session::set_active_main_request_seed(
        state,
        state.session.session_history.to_vec(),
        submitted.to_string(),
        state.session.last_history_model_name.clone(),
    );

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted.to_string(),
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
        session_title_prompt,
    })
}

fn accepted_plan_prompt() -> &'static str {
    "I accept this plan. Begin implementation now."
}

fn accepted_plan_implementation_prompt(state: &AppState) -> String {
    let accepted_plan = state
        .session
        .planning
        .proposed_plan
        .as_ref()
        .map(|plan| plan.raw_block.as_str())
        .unwrap_or(
            "<proposed_plan>\nAccepted plan content was not found in transcript.\n</proposed_plan>",
        );
    format!(
        concat!(
            "You are no longer in Plan Mode. The plan has been accepted for implementation.\n",
            "Do not say that you still need a developer or system transition out of plan mode.\n",
            "Use the accepted plan below as the implementation brief, explore the workspace as needed, and begin implementation now.\n\n",
            "Accepted plan:\n",
            "{accepted_plan}\n"
        ),
        accepted_plan = accepted_plan
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        app::{AccessMode, Action, Effect, PendingReplyKind, session::test_support::registry_app},
        features::planning::{PlanningStage, planning_conversation_prompt},
    };

    #[test]
    fn planning_draft_submission_starts_planning_workflow() {
        let mut app = registry_app(true);
        app.enter_planning_draft_mode();
        app.composer_mut().insert_str("Add a planning workflow");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: planning_conversation_prompt("Add a planning workflow"),
                history: Vec::new(),
                history_model_name: None,
                session_title_prompt: Some("Add a planning workflow".into()),
            })
        );
        assert!(!app.planning_draft_mode());
        assert!(app.plan_active());
        assert_eq!(
            app.state_mut()
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Planning)
        );
    }

    #[test]
    fn accepting_plan_starts_normal_prompt_model_turn() {
        let mut app = registry_app(true);
        app.state_mut().session.planning.proposed_plan =
            Some(crate::features::planning::ProposedPlan {
                markdown: "# Test Plan\n\n- step one".into(),
                raw_block: "<proposed_plan>\n# Test Plan\n\n- step one\n</proposed_plan>".into(),
            });
        app.begin_plan_review();

        let effect = app.apply(Action::AcceptPlanAndImplement);

        assert!(matches!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                history,
                prompt,
                ..
            }) if history.is_empty()
                && prompt.contains("You are no longer in Plan Mode")
                && prompt.contains("# Test Plan")
                && prompt.contains("step one")
        ));
        assert!(app.state_mut().session.pending_reply.is_some());
        assert!(!app.plan_review_selection_active());
        assert_eq!(app.state().session.mode, AccessMode::ReadWrite);
        assert_eq!(
            app.state_mut()
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Normal)
        );
    }

    #[test]
    fn accepting_plan_preserves_write_mode_when_already_enabled() {
        let mut app = registry_app(true);
        app.state_mut().session.mode = AccessMode::ReadWrite;
        app.state_mut().session.planning.proposed_plan =
            Some(crate::features::planning::ProposedPlan {
                markdown: "# Test Plan\n\n- step one".into(),
                raw_block: "<proposed_plan>\n# Test Plan\n\n- step one\n</proposed_plan>".into(),
            });
        app.begin_plan_review();

        let effect = app.apply(Action::AcceptPlanAndImplement);

        assert!(matches!(effect, Some(Effect::PromptModel { .. })));
        assert_eq!(app.state().session.mode, AccessMode::ReadWrite);
        assert_eq!(
            app.state()
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Normal)
        );
    }

    #[test]
    fn suggesting_plan_changes_returns_to_planning_finalization() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        let effect = app.apply(Action::SuggestPlanChanges);

        assert!(effect.is_none());
        assert!(!app.plan_review_selection_active());
        assert_eq!(
            app.state().session.planning.stage,
            PlanningStage::Finalizing
        );
        assert_eq!(app.state().session.planning.review, None);
    }

    #[test]
    fn plan_review_arrow_selection_and_enter_can_choose_discussion() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        let move_effect = app.apply(Action::SelectNextCommand);
        assert!(move_effect.is_none());
        assert_eq!(app.selected_plan_review_index(), Some(1));

        let submit_effect = app.apply(Action::SubmitMessage);
        assert!(submit_effect.is_none());
        assert_eq!(
            app.state().session.planning.stage,
            PlanningStage::Finalizing
        );
    }

    #[test]
    fn plan_discussion_submission_uses_raw_user_text() {
        let mut app = registry_app(true);
        app.begin_plan_review();
        app.apply(Action::SuggestPlanChanges);
        app.composer_mut().insert_str("Cover rollback and tests.");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "Cover rollback and tests.".into(),
                history: Vec::new(),
                history_model_name: None,
                session_title_prompt: Some("Cover rollback and tests.".into()),
            })
        );
        assert!(app.state_mut().session.pending_reply.is_some());
        assert_eq!(
            app.state_mut()
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Planning)
        );
        assert_eq!(
            app.entries().last().and_then(|entry| match entry {
                crate::app::TranscriptEntry::Message(message) => Some(message.text.as_str()),
                _ => None,
            }),
            Some("Cover rollback and tests.")
        );
    }
}
