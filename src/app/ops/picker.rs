use crate::{
    app::{
        AppState, ModelPickerTab, PickerSelection, ReasoningPickerTarget, SelectionPicker,
        SessionPickerEntry, selectable_models_for_tab,
    },
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
    state.ui.picker = Some(SelectionPicker::Model {
        active_tab: ModelPickerTab::NormalAgent,
        normal_selected_model: state.session.model_name.clone(),
        planning_selected_model: initial_planning_selected_model(state),
        safety_selected_model: state.session.safety_model_name.clone(),
        memory_selected_model: state.session.memory_model_name.clone(),
        critic_selected_model: state.session.critic_model_name.clone(),
    });
}

pub(crate) fn open_session_picker(state: &mut AppState, entries: Vec<SessionPickerEntry>) {
    state.ui.picker = Some(SelectionPicker::Session {
        entries,
        selected_index: 0,
    });
}

pub(crate) fn toggle_picker_selection(state: &mut AppState) -> Option<Vec<PlanningAgentConfig>> {
    let model_name = match state.ui.picker.as_ref()? {
        SelectionPicker::Model {
            active_tab: ModelPickerTab::PlanningAgents,
            planning_selected_model,
            ..
        } if !planning_selected_model.is_empty() => planning_selected_model.clone(),
        _ => return None,
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
            model_name: model_name.clone(),
            reasoning: default_planning_reasoning(&model_name),
        });
    }

    Some(state.session.planning_agents.clone())
}

pub(crate) fn apply_picker_selection(state: &mut AppState) -> Option<PickerSelection> {
    let picker = state.ui.picker.take()?;
    match picker {
        SelectionPicker::Model {
            active_tab,
            normal_selected_model,
            planning_selected_model,
            safety_selected_model,
            memory_selected_model,
            critic_selected_model,
        } => match active_tab {
            ModelPickerTab::NormalAgent => (!normal_selected_model.is_empty())
                .then_some(PickerSelection::Model(normal_selected_model)),
            ModelPickerTab::PlanningAgents => {
                if planning_selected_model.is_empty() {
                    return None;
                }
                open_reasoning_picker_for(
                    state,
                    ReasoningPickerTarget::PlanningAgent,
                    planning_selected_model,
                );
                None
            }
            ModelPickerTab::SafetyModel => {
                if safety_selected_model.is_empty() {
                    return None;
                }
                open_reasoning_picker_for(
                    state,
                    ReasoningPickerTarget::SafetyModel,
                    safety_selected_model,
                );
                None
            }
            ModelPickerTab::MemoryModel => {
                if memory_selected_model.is_empty() {
                    return None;
                }
                open_reasoning_picker_for(
                    state,
                    ReasoningPickerTarget::MemoryModel,
                    memory_selected_model,
                );
                None
            }
            ModelPickerTab::CriticModel => {
                if critic_selected_model.is_empty() {
                    return None;
                }
                open_reasoning_picker_for(
                    state,
                    ReasoningPickerTarget::CriticModel,
                    critic_selected_model,
                );
                None
            }
        },
        SelectionPicker::Session {
            entries,
            selected_index,
        } => entries
            .get(selected_index)
            .filter(|entry| entry.resumable)
            .map(|entry| PickerSelection::Session(entry.session_id.clone())),
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
                ReasoningPickerTarget::MemoryModel => {
                    state.session.memory_model_name = model_name.clone();
                    state.session.memory_reasoning = reasoning_effort;
                    PickerSelection::MemorySelection {
                        model_name,
                        reasoning: reasoning_effort,
                    }
                }
                ReasoningPickerTarget::CriticModel => {
                    state.session.critic_model_name = model_name.clone();
                    state.session.critic_reasoning = reasoning_effort;
                    PickerSelection::CriticSelection {
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
        ReasoningPickerTarget::MemoryModel => options
            .iter()
            .position(|level| {
                model_name == state.session.memory_model_name
                    && *level == state.session.memory_reasoning
            })
            .unwrap_or_else(|| {
                options
                    .iter()
                    .position(|level| *level == default_planning_reasoning(&model_name))
                    .unwrap_or(0)
            }),
        ReasoningPickerTarget::CriticModel => options
            .iter()
            .position(|level| {
                model_name == state.session.critic_model_name
                    && *level == state.session.critic_reasoning
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

fn initial_planning_selected_model(state: &AppState) -> String {
    let selectable =
        selectable_models_for_tab(ModelPickerTab::PlanningAgents, &state.session.model_name);
    state
        .session
        .planning_agents
        .iter()
        .find_map(|agent| {
            selectable
                .iter()
                .find(|model| model.name == agent.model_name)
                .map(|model| model.name.to_string())
        })
        .or_else(|| selectable.first().map(|model| model.name.to_string()))
        .unwrap_or_default()
}

fn move_picker_tab(state: &mut AppState, direction: isize) {
    let Some(SelectionPicker::Model { active_tab, .. }) = state.ui.picker.as_mut() else {
        return;
    };

    active_tab.toggle(direction);
}

fn move_picker_selection(state: &mut AppState, direction: isize) {
    let Some(picker) = state.ui.picker.as_mut() else {
        return;
    };

    match picker {
        SelectionPicker::Model {
            active_tab,
            normal_selected_model,
            planning_selected_model,
            safety_selected_model,
            memory_selected_model,
            critic_selected_model,
        } => {
            let selectable = selectable_models_for_tab(*active_tab, &state.session.model_name);
            if selectable.is_empty() {
                return;
            }
            let current = match active_tab {
                ModelPickerTab::NormalAgent => normal_selected_model,
                ModelPickerTab::PlanningAgents => planning_selected_model,
                ModelPickerTab::SafetyModel => safety_selected_model,
                ModelPickerTab::MemoryModel => memory_selected_model,
                ModelPickerTab::CriticModel => critic_selected_model,
            };
            let current_index = selectable
                .iter()
                .position(|model| model.name == current.as_str())
                .unwrap_or(0);
            let next_index =
                (current_index as isize + direction).rem_euclid(selectable.len() as isize) as usize;
            *current = selectable[next_index].name.to_string();
        }
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
        SelectionPicker::Session {
            entries,
            selected_index,
        } => {
            let len = entries.len();
            if len > 0 {
                *selected_index =
                    (*selected_index as isize + direction).rem_euclid(len as isize) as usize;
            }
        }
    }
}
