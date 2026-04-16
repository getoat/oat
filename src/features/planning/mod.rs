mod executor;
mod protocol;
mod state;
mod transitions;

pub use protocol::PlanningAgentConfig;

pub(crate) use executor::{PlanningSynthesisFuture, run_planning_workflow};
pub(crate) use protocol::{
    PlanningConfig, accepted_plan_implementation_prompt, default_planning_reasoning,
    parse_planning_reply, pending_plain_text_is_visible, planning_conversation_prompt,
    planning_conversation_prompt_headless, planning_finalization_prompt_headless,
    planning_reply_visible_text, sanitize_planning_agents,
};
pub(crate) use state::{
    PlanReviewState, PlanningBrief, PlanningFeatureState, PlanningReply, PlanningStage,
    ProposedPlan,
};
pub(crate) use transitions::{
    accept_brief_and_start_fanout, accept_review_for_implementation, cancel_draft, clear_planning,
    enter_draft, show_review, start_conversation, start_finalization,
};
