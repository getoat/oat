use super::super::Effect;
use crate::app::{AppState, PickerSelection, ops};

pub(super) fn submit_picker_selection(state: &mut AppState) -> Option<Effect> {
    match ops::picker::apply_picker_selection(state)? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Session(session_id) => Some(Effect::ResumeSession { session_id }),
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
    use crate::app::{
        Action, SelectionPicker, SessionPickerEntry, session::test_support::registry_app,
    };

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

    #[test]
    fn submitting_session_picker_returns_resume_effect() {
        let mut app = registry_app(true);
        app.state_mut().ui.picker = Some(SelectionPicker::Session {
            entries: vec![
                SessionPickerEntry {
                    session_id: "session-1".into(),
                    title: "First".into(),
                    detail: "older".into(),
                    resumable: true,
                },
                SessionPickerEntry {
                    session_id: "session-2".into(),
                    title: "Second".into(),
                    detail: "newer".into(),
                    resumable: true,
                },
            ],
            selected_index: 0,
        });
        app.apply(Action::SelectNextCommand);

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::ResumeSession {
                session_id: "session-2".into(),
            })
        );
        assert!(!app.selection_picker_visible());
    }

    #[test]
    fn submitting_non_resumable_session_picker_returns_none() {
        let mut app = registry_app(true);
        app.state_mut().ui.picker = Some(SelectionPicker::Session {
            entries: vec![SessionPickerEntry {
                session_id: "session-1".into(),
                title: "Unavailable".into(),
                detail: "selection unavailable".into(),
                resumable: false,
            }],
            selected_index: 0,
        });

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(!app.selection_picker_visible());
    }
}
