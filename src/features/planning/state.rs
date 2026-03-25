#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanningStage {
    Idle,
    Drafting,
    Conversation,
    RunningFanout,
    Finalizing,
    Review,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanReviewState {
    Selection,
    Feedback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanningFeatureState {
    pub stage: PlanningStage,
    pub review: Option<PlanReviewState>,
}

impl Default for PlanningFeatureState {
    fn default() -> Self {
        Self {
            stage: PlanningStage::Idle,
            review: None,
        }
    }
}
