use super::super::Effect;
use crate::app::{PickerSelection, ReducerContext};

pub(super) fn submit_picker_selection(ctx: &mut ReducerContext<'_>) -> Option<Effect> {
    match ctx.apply_picker_selection()? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Reasoning(reasoning_effort) => {
            Some(Effect::SetReasoningEffort { reasoning_effort })
        }
        PickerSelection::PlanningAgent(_) => Some(Effect::SetPlanningAgents {
            planning_agents: ctx.planning_agents().to_vec(),
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
