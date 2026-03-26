use super::super::Effect;
use crate::app::{AppState, PickerSelection, ops};

pub(super) fn submit_picker_selection(state: &mut AppState) -> Option<Effect> {
    match ops::picker::apply_picker_selection(state)? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Reasoning(reasoning) => Some(Effect::SetReasoning { reasoning }),
        PickerSelection::PlanningAgent(_) => Some(Effect::SetPlanningAgents {
            planning_agents: state.session.planning_agents.to_vec(),
        }),
        PickerSelection::SafetySelection {
            model_name,
            reasoning,
        } => Some(Effect::SetSafetySelection {
            model_name,
            reasoning,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, session::test_support::registry_app};

    #[test]
    fn submitting_model_picker_returns_model_selection_effect() {
        let mut app = registry_app(true);
        app.open_model_picker();
        app.apply(Action::SelectNextCommand);

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetModelSelection {
                model_name: "gpt-5.4-nano".into(),
            })
        );
        assert!(!app.selection_picker_visible());
    }
}
