use super::super::{Effect, PendingReplyKind};
use crate::app::{AppState, ops, query};
use crate::features::planning::planning_conversation_prompt;

pub(crate) fn submit_plan_acceptance(state: &mut AppState) -> Option<Effect> {
    if query::has_pending_reply(state) || !query::plan_review_selection_active(state) {
        return None;
    }

    let visible_prompt = accepted_plan_prompt().to_string();
    let prompt = accepted_plan_implementation_prompt(state);
    ops::composer::record_submitted_input(state, &visible_prompt);
    ops::planning::accept_plan_review_for_implementation(state);
    ops::transcript::push_user_message(state, visible_prompt);
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Normal);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: Vec::new(),
        history_model_name: None,
    })
}

pub(super) fn submit_plan_review_selection(state: &mut AppState) -> Option<Effect> {
    match query::selected_plan_review_index(state).unwrap_or(0) {
        0 => submit_plan_acceptance(state),
        1 => {
            ops::planning::begin_plan_review_feedback(state);
            None
        }
        _ => None,
    }
}

pub(super) fn submit_plan_revision_feedback(
    state: &mut AppState,
    submitted: &str,
) -> Option<Effect> {
    if query::has_pending_reply(state)
        || submitted.is_empty()
        || !query::plan_review_feedback_active(state)
    {
        return None;
    }

    let prompt = format!(
        "Revise the proposed plan based on these comments. Respond with an updated <proposed_plan> block and do not begin implementation yet. Do not use subagents for this revision.\n\n{}",
        submitted
    );
    ops::composer::record_submitted_input(state, submitted);
    ops::planning::clear_plan_review(state);
    ops::transcript::push_user_message(state, prompt.clone());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
    })
}

pub(super) fn submit_planning_draft(state: &mut AppState, submitted: &str) -> Option<Effect> {
    if query::has_pending_reply(state) || submitted.is_empty() {
        return None;
    }

    ops::planning::consume_planning_draft_mode(state);
    ops::composer::record_submitted_input(state, submitted);
    ops::transcript::push_user_message(state, submitted.to_string());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt: planning_conversation_prompt(submitted),
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
    })
}

pub(super) fn submit_planning_turn(state: &mut AppState, submitted: &str) -> Option<Effect> {
    if query::has_pending_reply(state) || submitted.is_empty() {
        return None;
    }

    ops::composer::record_submitted_input(state, submitted);
    ops::transcript::push_user_message(state, submitted.to_string());
    ops::history::resume_history_follow(state);
    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted.to_string(),
        history: state.session.session_history.to_vec(),
        history_model_name: state.session.last_history_model_name.clone(),
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
