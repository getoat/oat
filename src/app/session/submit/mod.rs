mod approvals;
mod commands;
mod picker;
mod planning;

use approvals::{submit_ask_user, submit_shell_approval};
use commands::submit_command;
use picker::submit_picker_selection;
pub(super) use planning::submit_plan_acceptance;
use planning::{
    submit_plan_review_selection, submit_plan_revision_feedback, submit_planning_draft,
    submit_planning_turn,
};

use super::{Effect, PendingReplyKind};
use crate::app::ReducerContext;
use crate::features::planning::PlanningStage;

pub(super) fn submit_message(ctx: &mut ReducerContext<'_>) -> Option<Effect> {
    if ctx.has_pending_write_approval() {
        return None;
    }

    if ctx.has_pending_shell_approval() {
        return submit_shell_approval(ctx);
    }

    if ctx.plan_review_selection_active() {
        return submit_plan_review_selection(ctx);
    }

    if ctx.has_pending_ask_user() {
        return submit_ask_user(ctx);
    }

    if ctx.selection_picker_visible() {
        return submit_picker_selection(ctx);
    }

    let submitted = ctx.submitted_composer_text();

    if ctx.plan_review_feedback_active() {
        return submit_plan_revision_feedback(ctx, &submitted);
    }

    if ctx.command_query().is_some() {
        let command_name = ctx.command_name().unwrap_or_default().to_owned();
        let arguments = ctx.command_arguments().unwrap_or_default().to_owned();
        return submit_command(ctx, &command_name, &arguments);
    }

    if ctx.planning_draft_mode() {
        return submit_planning_draft(ctx, &submitted);
    }

    if matches!(
        ctx.planning_session_stage(),
        Some(PlanningStage::Conversation | PlanningStage::Finalizing)
    ) {
        return submit_planning_turn(ctx, &submitted);
    }

    if ctx.has_pending_reply() || submitted.is_empty() {
        return None;
    }

    ctx.record_submitted_input(&submitted);
    ctx.clear_plan_review();
    ctx.push_user_message(submitted.clone());
    ctx.resume_history_follow();
    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Normal);

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted,
        history: ctx.session_history().to_vec(),
        history_model_name: ctx.last_history_model_name().map(str::to_string),
    })
}
