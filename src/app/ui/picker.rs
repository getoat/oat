use crate::{
    config::ReasoningSetting,
    features::planning::PlanningAgentConfig,
    model_registry::{self, ModelInfo, ModelProvider},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPickerTab {
    NormalAgent,
    PlanningAgents,
    SafetyModel,
}

impl ModelPickerTab {
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
pub struct SessionPickerEntry {
    pub session_id: String,
    pub title: String,
    pub detail: String,
    pub resumable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionPicker {
    Model {
        active_tab: ModelPickerTab,
        normal_selected_model: String,
        planning_selected_model: String,
        safety_selected_model: String,
    },
    Session {
        entries: Vec<SessionPickerEntry>,
        selected_index: usize,
    },
    Reasoning {
        target: ReasoningPickerTarget,
        model_name: String,
        options: Vec<ReasoningSetting>,
        selected_index: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModelPickerEntry {
    ProviderHeading(ModelProvider),
    Model(&'static ModelInfo),
}

impl ModelPickerEntry {
    pub fn is_model(&self) -> bool {
        matches!(self, Self::Model(_))
    }
}

pub fn selectable_models_for_tab(
    active_tab: ModelPickerTab,
    current_main_model: &str,
) -> Vec<&'static ModelInfo> {
    model_registry::models()
        .iter()
        .filter(|model| {
            active_tab != ModelPickerTab::PlanningAgents || model.name != current_main_model
        })
        .collect()
}

pub fn display_entries_for_tab(
    active_tab: ModelPickerTab,
    current_main_model: &str,
) -> Vec<ModelPickerEntry> {
    let mut entries = Vec::new();
    let mut current_provider = None;

    for model in selectable_models_for_tab(active_tab, current_main_model) {
        if current_provider != Some(model.provider) {
            entries.push(ModelPickerEntry::ProviderHeading(model.provider));
            current_provider = Some(model.provider);
        }
        entries.push(ModelPickerEntry::Model(model));
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_entries_group_models_under_provider_headings() {
        let entries = display_entries_for_tab(ModelPickerTab::NormalAgent, "gpt-5.4-mini");

        assert!(matches!(
            entries.first(),
            Some(ModelPickerEntry::ProviderHeading(
                ModelProvider::AzureOpenAi
            ))
        ));
        assert!(entries.iter().any(|entry| {
            matches!(
                entry,
                ModelPickerEntry::ProviderHeading(ModelProvider::ChutesAi)
            )
        }));
        assert!(entries.iter().any(|entry| {
            matches!(entry, ModelPickerEntry::Model(model) if model.name == "zai-org/GLM-5-TEE")
        }));
    }

    #[test]
    fn planning_entries_exclude_current_main_model() {
        let entries = display_entries_for_tab(ModelPickerTab::PlanningAgents, "gpt-5.4-mini");

        assert!(!entries.iter().any(|entry| {
            matches!(entry, ModelPickerEntry::Model(model) if model.name == "gpt-5.4-mini")
        }));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerSelection {
    Model(String),
    Session(String),
    Reasoning(ReasoningSetting),
    PlanningAgent(PlanningAgentConfig),
    SafetySelection {
        model_name: String,
        reasoning: ReasoningSetting,
    },
}
