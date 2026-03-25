use super::super::Effect;
use crate::app::{AppState, PickerSelection, ops};

pub(super) fn submit_picker_selection(state: &mut AppState) -> Option<Effect> {
    match ops::picker::apply_picker_selection(state)? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Reasoning(reasoning_effort) => {
            Some(Effect::SetReasoningEffort { reasoning_effort })
        }
        PickerSelection::PlanningAgent(_) => Some(Effect::SetPlanningAgents {
            planning_agents: state.session.planning_agents.to_vec(),
        }),
        PickerSelection::SafetySelection {
            model_name,
            reasoning_effort,
        } => Some(Effect::SetSafetySelection {
            model_name,
            reasoning_effort,
        }),
    }
}
