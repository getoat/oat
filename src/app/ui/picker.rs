use crate::{config::ReasoningEffort, features::planning::PlanningAgentConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPickerTab {
    NormalAgent,
    PlanningAgents,
    SafetyModel,
}

impl ModelPickerTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::NormalAgent => "Normal agent",
            Self::PlanningAgents => "Planning agents",
            Self::SafetyModel => "Safety model",
        }
    }

    pub fn toggle(&mut self, direction: isize) {
        let tabs = [Self::NormalAgent, Self::PlanningAgents, Self::SafetyModel];
        let current = tabs.iter().position(|tab| *tab == *self).unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(tabs.len() as isize) as usize;
        *self = tabs[next];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningPickerTarget {
    NormalAgent,
    PlanningAgent,
    SafetyModel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionPicker {
    Model {
        active_tab: ModelPickerTab,
        normal_selected_index: usize,
        planning_selected_index: usize,
        safety_selected_index: usize,
    },
    Reasoning {
        target: ReasoningPickerTarget,
        model_name: String,
        options: Vec<ReasoningEffort>,
        selected_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerSelection {
    Model(String),
    Reasoning(ReasoningEffort),
    PlanningAgent(PlanningAgentConfig),
    SafetySelection {
        model_name: String,
        reasoning_effort: ReasoningEffort,
    },
}
