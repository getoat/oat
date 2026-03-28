use crate::app::session::submit::submit_plan_acceptance;
use crate::app::{Action, AppState, Effect, ops};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::AcceptPlanAndImplement => submit_plan_acceptance(state),
        Action::SuggestPlanChanges => {
            if crate::app::query::plan_review_selection_active(state) {
                ops::planning::begin_plan_discussion(state);
            }
            None
        }
        _ => None,
    }
}
