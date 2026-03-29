use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct PlanningBrief {
    pub markdown: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ProposedPlan {
    pub markdown: String,
    pub raw_block: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanningReply {
    ConversationText(String),
    ReadyBrief(PlanningBrief),
    ProposedPlan(ProposedPlan),
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum PlanningStage {
    Idle,
    Drafting,
    Conversation,
    RunningFanout,
    Finalizing,
    Review,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub enum PlanReviewState {
    Selection,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct PlanningFeatureState {
    pub stage: PlanningStage,
    pub review: Option<PlanReviewState>,
    pub normalized_brief: Option<PlanningBrief>,
    pub proposed_plan: Option<ProposedPlan>,
}

impl Default for PlanningFeatureState {
    fn default() -> Self {
        Self {
            stage: PlanningStage::Idle,
            review: None,
            normalized_brief: None,
            proposed_plan: None,
        }
    }
}
