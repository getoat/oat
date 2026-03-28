use crate::{
    app::session::ProposedPlanEntry,
    app::{AppState, TranscriptEntry},
    features::planning::{
        PlanReviewState, PlanningBrief, PlanningStage, ProposedPlan, accept_brief_and_start_fanout,
        accept_review_for_implementation, cancel_draft, clear_planning, show_review,
        start_conversation, start_finalization,
    },
};

use super::{composer, transcript};

pub(crate) fn begin_plan_review(state: &mut AppState) {
    show_review(&mut state.session.planning, PlanReviewState::Selection);
    state.ui.plan_review_selected_index = 0;
    composer::clear_composer(state);
}

pub(crate) fn begin_plan_discussion(state: &mut AppState) {
    start_finalization(&mut state.session.planning);
    state.ui.plan_review_selected_index = 0;
    composer::clear_composer(state);
}

pub(crate) fn clear_plan_review(state: &mut AppState) {
    clear_planning(&mut state.session.planning);
    state.ui.plan_review_selected_index = 0;
}

pub(crate) fn accept_plan_review_for_implementation(state: &mut AppState) {
    accept_review_for_implementation(&mut state.session.planning);
    state.ui.plan_review_selected_index = 0;
}

pub(crate) fn move_plan_review_selection(state: &mut AppState, direction: isize) {
    if !(state.session.planning.stage == PlanningStage::Review
        && state.session.planning.review == Some(PlanReviewState::Selection))
    {
        return;
    }

    state.ui.plan_review_selected_index =
        (state.ui.plan_review_selected_index as isize + direction).rem_euclid(2) as usize;
}

pub(crate) fn enter_planning_draft_mode(state: &mut AppState) {
    crate::features::planning::enter_draft(&mut state.session.planning);
    composer::clear_composer(state);
}

pub(crate) fn cancel_planning_draft_mode(state: &mut AppState) -> bool {
    if state.session.planning.stage != PlanningStage::Drafting {
        return false;
    }

    cancel_draft(&mut state.session.planning);
    composer::clear_composer(state);
    true
}

pub(crate) fn consume_planning_draft_mode(state: &mut AppState) -> bool {
    let was_active = state.session.planning.stage == PlanningStage::Drafting;
    if was_active {
        start_conversation(&mut state.session.planning);
    }
    was_active
}

pub(crate) fn begin_planning_conversation(state: &mut AppState) {
    start_conversation(&mut state.session.planning);
}

pub(crate) fn begin_planning_fanout(state: &mut AppState) {
    accept_brief_and_start_fanout(&mut state.session.planning);
}

pub(crate) fn set_planning_brief(state: &mut AppState, markdown: String) {
    state.session.planning.normalized_brief = Some(PlanningBrief { markdown });
}

pub(crate) fn begin_planning_finalization(state: &mut AppState) {
    start_finalization(&mut state.session.planning);
}

pub(crate) fn store_proposed_plan(state: &mut AppState, plan: ProposedPlan) {
    state.session.planning.proposed_plan = Some(plan.clone());
    let entry = TranscriptEntry::ProposedPlan(ProposedPlanEntry {
        markdown: plan.markdown,
        raw_block: plan.raw_block,
    });
    let replace_index = state
        .session
        .pending_reply
        .as_ref()
        .and_then(|pending| pending.text_entry_index);
    if let Some(index) = replace_index
        && index < state.session.entries.len()
    {
        state.session.entries[index] = entry;
        transcript::bump_transcript_revision(state);
        return;
    }
    state.session.entries.push(entry);
    transcript::bump_transcript_revision(state);
}
