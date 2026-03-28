use super::state::{PlanReviewState, PlanningFeatureState, PlanningStage};

pub fn enter_draft(state: &mut PlanningFeatureState) {
    state.stage = PlanningStage::Drafting;
    state.review = None;
    state.normalized_brief = None;
    state.proposed_plan = None;
}

pub fn cancel_draft(state: &mut PlanningFeatureState) {
    if state.stage == PlanningStage::Drafting {
        clear_planning(state);
    }
}

pub fn start_conversation(state: &mut PlanningFeatureState) {
    state.stage = PlanningStage::Conversation;
    state.review = None;
}

pub fn accept_brief_and_start_fanout(state: &mut PlanningFeatureState) {
    state.stage = PlanningStage::RunningFanout;
    state.review = None;
}

pub fn start_finalization(state: &mut PlanningFeatureState) {
    state.stage = PlanningStage::Finalizing;
    state.review = None;
}

pub fn show_review(state: &mut PlanningFeatureState, review: PlanReviewState) {
    state.stage = PlanningStage::Review;
    state.review = Some(review);
}

pub fn accept_review_for_implementation(state: &mut PlanningFeatureState) {
    clear_planning(state);
}

pub fn clear_planning(state: &mut PlanningFeatureState) {
    *state = PlanningFeatureState::default();
}
