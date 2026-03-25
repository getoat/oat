mod approvals;
mod commands;
mod picker;
mod planning;

use approvals::{submit_ask_user, submit_shell_approval};
use commands::submit_command;
use picker::submit_picker_selection;
pub(crate) use planning::submit_plan_acceptance;
use planning::{
    submit_plan_review_selection, submit_plan_revision_feedback, submit_planning_draft,
    submit_planning_turn,
};

use super::{Effect, PendingReplyKind};
use crate::app::{AppState, InputTarget, ops, query};
use crate::features::planning::PlanningStage;

pub(crate) fn submit_message(state: &mut AppState) -> Option<Effect> {
    if query::has_pending_write_approval(state) {
        return None;
    }

    match query::active_input_target(state) {
        InputTarget::ShellApprovalSelection | InputTarget::ShellApprovalEditor => {
            return submit_shell_approval(state);
        }
        InputTarget::PlanReviewSelection => return submit_plan_review_selection(state),
        InputTarget::AskUserSelection | InputTarget::AskUserEditor => {
            return submit_ask_user(state);
        }
        InputTarget::Picker => return submit_picker_selection(state),
        InputTarget::Composer | InputTarget::CommandPalette => {}
    }

    let submitted = ops::composer::submitted_composer_text(state);

    if query::plan_review_feedback_active(state) {
        return submit_plan_revision_feedback(state, &submitted);
    }

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
