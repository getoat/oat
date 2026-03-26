use crate::{
    app::{AppState, ModelPickerTab, PickerSelection, ReasoningPickerTarget, SelectionPicker},
    features::planning::{PlanningAgentConfig, default_planning_reasoning},
    model_registry,
};

pub(crate) fn cancel_picker(state: &mut AppState) -> bool {
    state.ui.picker.take().is_some()
}

pub(crate) fn move_picker_selection_up(state: &mut AppState) {
    move_picker_selection(state, -1);
}

pub(crate) fn move_picker_selection_down(state: &mut AppState) {
    move_picker_selection(state, 1);
}

pub(crate) fn move_picker_tab_left(state: &mut AppState) {
    move_picker_tab(state, -1);
}

pub(crate) fn move_picker_tab_right(state: &mut AppState) {
    move_picker_tab(state, 1);
}

pub(crate) fn open_model_picker(state: &mut AppState) {
    let normal_selected_index = model_registry::models()
        .iter()
        .position(|model| model.name == state.session.model_name)
        .unwrap_or(0);
    let safety_selected_index = model_registry::models()
        .iter()
        .position(|model| model.name == state.session.safety_model_name)
        .unwrap_or(0);
    state.ui.picker = Some(SelectionPicker::Model {
        active_tab: ModelPickerTab::NormalAgent,
        normal_selected_index,
        planning_selected_index: 0,
        safety_selected_index,
    });
}

pub(crate) fn toggle_picker_selection(state: &mut AppState) -> Option<Vec<PlanningAgentConfig>> {
    let planning_selected_index = match state.ui.picker.as_ref()? {
        SelectionPicker::Model {
            active_tab: ModelPickerTab::PlanningAgents,
            planning_selected_index,
            ..
        } => *planning_selected_index,
        _ => return None,
    };
    let model_name = match planning_models(&state.session.model_name).get(planning_selected_index) {
        Some(model) => model.name.to_string(),
        None => return None,
    };

    if let Some(existing_index) = state
        .session
        .planning_agents
        .iter()
        .position(|agent| agent.model_name == model_name)
    {
        state.session.planning_agents.remove(existing_index);
    } else {
        state.session.planning_agents.push(PlanningAgentConfig {
            model_name,
            reasoning: default_planning_reasoning(
                planning_models(&state.session.model_name)
                    .get(planning_selected_index)
                    .map(|model| model.name)
                    .unwrap_or_default(),
            ),
        });
    }

    Some(state.session.planning_agents.clone())
}

pub(crate) fn apply_picker_selection(state: &mut AppState) -> Option<PickerSelection> {
    let picker = state.ui.picker.take()?;
    match picker {
        SelectionPicker::Model {
            active_tab,
            normal_selected_index,
            planning_selected_index,
            safety_selected_index,
        } => match active_tab {
            ModelPickerTab::NormalAgent => model_registry::models()
                .get(normal_selected_index)
                .map(|model| PickerSelection::Model(model.name.to_string())),
            ModelPickerTab::PlanningAgents => {
                let model_name = planning_models(&state.session.model_name)
                    .get(planning_selected_index)
                    .map(|model| model.name.to_string())?;
                open_reasoning_picker_for(state, ReasoningPickerTarget::PlanningAgent, model_name);
                None
            }
            ModelPickerTab::SafetyModel => {
                let model_name = model_registry::models()
                    .get(safety_selected_index)
                    .map(|model| model.name.to_string())?;
                open_reasoning_picker_for(state, ReasoningPickerTarget::SafetyModel, model_name);
                None
            }
        },
        SelectionPicker::Reasoning {
            target,
            model_name,
            options,
            selected_index,
        } => options
            .get(selected_index)
            .copied()
            .map(|reasoning_effort| match target {
                ReasoningPickerTarget::NormalAgent => PickerSelection::Reasoning(reasoning_effort),
                ReasoningPickerTarget::PlanningAgent => {
                    let planning_agent = PlanningAgentConfig {
                        model_name,
                        reasoning: reasoning_effort,
                    };
                    if let Some(existing) = state
                        .session
                        .planning_agents
                        .iter_mut()
                        .find(|agent| agent.model_name == planning_agent.model_name)
                    {
                        *existing = planning_agent.clone();
                    } else {
                        state.session.planning_agents.push(planning_agent.clone());
                    }
                    PickerSelection::PlanningAgent(planning_agent)
                }
                ReasoningPickerTarget::SafetyModel => {
                    state.session.safety_model_name = model_name.clone();
                    state.session.safety_reasoning = reasoning_effort;
                    PickerSelection::SafetySelection {
                        model_name,
                        reasoning: reasoning_effort,
                    }
                }
            }),
    }
}

pub(crate) fn open_reasoning_picker(state: &mut AppState) {
    open_reasoning_picker_for(
        state,
        ReasoningPickerTarget::NormalAgent,
        state.session.model_name.clone(),
    );
}

pub(crate) fn open_reasoning_picker_for(
    state: &mut AppState,
    target: ReasoningPickerTarget,
    model_name: String,
) {
    let Some(options) = model_registry::reasoning_settings_for_model(&model_name) else {
        state.ui.picker = None;
        return;
    };

    let selected_index = match target {
        ReasoningPickerTarget::NormalAgent => options
            .iter()
            .position(|level| *level == state.session.reasoning)
            .unwrap_or(0),
        ReasoningPickerTarget::PlanningAgent => options
            .iter()
            .position(|level| {
                state
                    .session
                    .planning_agents
                    .iter()
                    .find(|agent| agent.model_name == model_name)
                    .map(|agent| *level == agent.reasoning)
                    .unwrap_or(false)
            })
            .unwrap_or_else(|| {
                options
                    .iter()
                    .position(|level| *level == default_planning_reasoning(&model_name))
                    .unwrap_or(0)
            }),
        ReasoningPickerTarget::SafetyModel => options
            .iter()
            .position(|level| {
                model_name == state.session.safety_model_name
                    && *level == state.session.safety_reasoning
            })
            .unwrap_or_else(|| {
                options
                    .iter()
                    .position(|level| *level == default_planning_reasoning(&model_name))
                    .unwrap_or(0)
            }),
    };
    state.ui.picker = Some(SelectionPicker::Reasoning {
        target,
        model_name,
        options: options.to_vec(),
        selected_index,
    });
}

fn planning_models(current_main_model: &str) -> Vec<&'static model_registry::ModelInfo> {
    model_registry::models()
        .iter()
        .filter(|model| model.name != current_main_model)
        .collect()
}

fn move_picker_tab(state: &mut AppState, direction: isize) {
    let Some(SelectionPicker::Model { active_tab, .. }) = state.ui.picker.as_mut() else {
        return;
    };

    active_tab.toggle(direction);
}

fn move_picker_selection(state: &mut AppState, direction: isize) {
    let planning_len = planning_models(&state.session.model_name).len();
    let Some(picker) = state.ui.picker.as_mut() else {
        return;
    };

    match picker {
        SelectionPicker::Model {
            active_tab,
            normal_selected_index,
            planning_selected_index,
            safety_selected_index,
        } => match active_tab {
            ModelPickerTab::NormalAgent => {
                let len = model_registry::models().len();
                if len > 0 {
                    *normal_selected_index = (*normal_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
            }
            ModelPickerTab::PlanningAgents => {
                if planning_len > 0 {
                    *planning_selected_index = (*planning_selected_index as isize + direction)
                        .rem_euclid(planning_len as isize)
                        as usize;
                }
            }
            ModelPickerTab::SafetyModel => {
                let len = model_registry::models().len();
                if len > 0 {
                    *safety_selected_index = (*safety_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
            }
        },
        SelectionPicker::Reasoning {
            selected_index,
            options,
            ..
        } => {
            let len = options.len();
            if len > 0 {
                *selected_index =
                    (*selected_index as isize + direction).rem_euclid(len as isize) as usize;
            }
        }
    }
}
