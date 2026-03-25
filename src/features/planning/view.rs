use super::state::{PlanReviewState, PlanningFeatureState, PlanningStage};

pub fn stage_label(state: PlanningFeatureState) -> Option<&'static str> {
    match state.stage {
        PlanningStage::Idle => None,
        PlanningStage::Drafting => Some("Planning draft"),
        PlanningStage::Conversation => Some("Planning conversation"),
        PlanningStage::RunningFanout => Some("Planning fanout"),
        PlanningStage::Finalizing => Some("Planning finalization"),
        PlanningStage::Review => Some("Plan review"),
    }
}

pub fn review_mode(state: PlanningFeatureState) -> Option<PlanReviewState> {
    state.review
}
