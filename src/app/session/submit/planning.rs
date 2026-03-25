use super::super::{Effect, PendingReplyKind};
use crate::app::ReducerContext;
use crate::features::planning::planning_conversation_prompt;

pub(in crate::app::session) fn submit_plan_acceptance(
    ctx: &mut ReducerContext<'_>,
) -> Option<Effect> {
    if ctx.has_pending_reply() || !ctx.plan_review_selection_active() {
        return None;
    }

    let visible_prompt = accepted_plan_prompt().to_string();
    let prompt = accepted_plan_implementation_prompt(ctx);
    ctx.record_submitted_input(&visible_prompt);
    ctx.accept_plan_review_for_implementation();
    ctx.push_user_message(visible_prompt);
    ctx.resume_history_follow();
    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Normal);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: Vec::new(),
        history_model_name: None,
    })
}

pub(super) fn submit_plan_review_selection(ctx: &mut ReducerContext<'_>) -> Option<Effect> {
    match ctx.selected_plan_review_index().unwrap_or(0) {
        0 => submit_plan_acceptance(ctx),
        1 => {
            ctx.begin_plan_review_feedback();
            None
        }
        _ => None,
    }
}

pub(super) fn submit_plan_revision_feedback(
    ctx: &mut ReducerContext<'_>,
    submitted: &str,
) -> Option<Effect> {
    if ctx.has_pending_reply() || submitted.is_empty() || !ctx.plan_review_feedback_active() {
        return None;
    }

    let prompt = format!(
        "Revise the proposed plan based on these comments. Respond with an updated <proposed_plan> block and do not begin implementation yet. Do not use subagents for this revision.\n\n{}",
        submitted
    );
    ctx.record_submitted_input(submitted);
    ctx.clear_plan_review();
    ctx.push_user_message(prompt.clone());
    ctx.resume_history_follow();
    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: ctx.session_history().to_vec(),
        history_model_name: ctx.last_history_model_name().map(str::to_string),
    })
}

pub(super) fn submit_planning_draft(
    ctx: &mut ReducerContext<'_>,
    submitted: &str,
) -> Option<Effect> {
    if ctx.has_pending_reply() || submitted.is_empty() {
        return None;
    }

    ctx.consume_planning_draft_mode();
    ctx.record_submitted_input(submitted);
    ctx.push_user_message(submitted.to_string());
    ctx.resume_history_follow();
    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt: planning_conversation_prompt(submitted),
        history: ctx.session_history().to_vec(),
        history_model_name: ctx.last_history_model_name().map(str::to_string),
    })
}

pub(super) fn submit_planning_turn(
    ctx: &mut ReducerContext<'_>,
    submitted: &str,
) -> Option<Effect> {
    if ctx.has_pending_reply() || submitted.is_empty() {
        return None;
    }

    ctx.record_submitted_input(submitted);
    ctx.push_user_message(submitted.to_string());
    ctx.resume_history_follow();
    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Planning);

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted.to_string(),
        history: ctx.session_history().to_vec(),
        history_model_name: ctx.last_history_model_name().map(str::to_string),
    })
}

fn accepted_plan_prompt() -> &'static str {
    "I accept this plan. Begin implementation now."
}

fn accepted_plan_implementation_prompt(ctx: &ReducerContext<'_>) -> String {
    let accepted_plan = ctx.latest_proposed_plan_message().unwrap_or(
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
