use super::*;

impl AppShell {
    pub(crate) fn move_command_selection_up(&mut self) {
        self.move_command_selection(-1);
    }

    pub(crate) fn move_command_selection_down(&mut self) {
        self.move_command_selection(1);
    }

    pub(crate) fn open_model_picker(&mut self) {
        let normal_selected_index = model_registry::models()
            .iter()
            .position(|model| model.name == self.session.model_name)
            .unwrap_or(0);
        let safety_selected_index = model_registry::models()
            .iter()
            .position(|model| model.name == self.session.safety_model_name)
            .unwrap_or(0);
        self.ui.picker = Some(SelectionPicker::Model {
            active_tab: ModelPickerTab::NormalAgent,
            normal_selected_index,
            planning_selected_index: 0,
            safety_selected_index,
        });
    }

    pub(crate) fn open_reasoning_picker(&mut self) {
        self.open_reasoning_picker_for(
            ReasoningPickerTarget::NormalAgent,
            self.session.model_name.clone(),
        );
    }

    pub(crate) fn open_reasoning_picker_for(
        &mut self,
        target: ReasoningPickerTarget,
        model_name: String,
    ) {
        let Some(options) = model_registry::reasoning_levels_for_model(&model_name) else {
            self.ui.picker = None;
            return;
        };

        let selected_index = match target {
            ReasoningPickerTarget::NormalAgent => options
                .iter()
                .position(|level| *level == self.session.reasoning_effort)
                .unwrap_or(0),
            ReasoningPickerTarget::PlanningAgent => options
                .iter()
                .position(|level| {
                    self.session
                        .planning_agents
                        .iter()
                        .find(|agent| agent.model_name == model_name)
                        .map(|agent| *level == agent.reasoning_effort)
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
                    model_name == self.session.safety_model_name
                        && *level == self.session.safety_reasoning_effort
                })
                .unwrap_or_else(|| {
                    options
                        .iter()
                        .position(|level| *level == default_planning_reasoning(&model_name))
                        .unwrap_or(0)
                }),
        };
        self.ui.picker = Some(SelectionPicker::Reasoning {
            target,
            model_name,
            options: options.to_vec(),
            selected_index,
        });
    }

    pub(crate) fn cancel_picker(&mut self) -> bool {
        self.ui.picker.take().is_some()
    }

    pub(crate) fn move_picker_selection_up(&mut self) {
        self.move_picker_selection(-1);
    }

    pub(crate) fn move_picker_selection_down(&mut self) {
        self.move_picker_selection(1);
    }

    pub(crate) fn move_picker_tab_left(&mut self) {
        self.move_picker_tab(-1);
    }

    pub(crate) fn move_picker_tab_right(&mut self) {
        self.move_picker_tab(1);
    }

    pub(crate) fn toggle_picker_selection(&mut self) -> Option<Vec<PlanningAgentConfig>> {
        let planning_selected_index = match self.ui.picker.as_ref()? {
            SelectionPicker::Model {
                active_tab: ModelPickerTab::PlanningAgents,
                planning_selected_index,
                ..
            } => *planning_selected_index,
            _ => return None,
        };
        let model_name = match self.planning_models().get(planning_selected_index) {
            Some(model) => model.name.to_string(),
            None => return None,
        };

        if let Some(existing_index) = self
            .session
            .planning_agents
            .iter()
            .position(|agent| agent.model_name == model_name)
        {
            self.session.planning_agents.remove(existing_index);
        } else {
            self.session.planning_agents.push(PlanningAgentConfig {
                model_name,
                reasoning_effort: default_planning_reasoning(
                    self.planning_models()
                        .get(planning_selected_index)
                        .map(|model| model.name)
                        .unwrap_or_default(),
                ),
            });
        }

        Some(self.session.planning_agents.clone())
    }

    pub(crate) fn apply_picker_selection(&mut self) -> Option<PickerSelection> {
        let picker = self.ui.picker.take()?;
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
                    let model_name = self
                        .planning_models()
                        .get(planning_selected_index)
                        .map(|model| model.name.to_string())?;
                    self.open_reasoning_picker_for(
                        ReasoningPickerTarget::PlanningAgent,
                        model_name,
                    );
                    None
                }
                ModelPickerTab::SafetyModel => {
                    let model_name = model_registry::models()
                        .get(safety_selected_index)
                        .map(|model| model.name.to_string())?;
                    self.open_reasoning_picker_for(ReasoningPickerTarget::SafetyModel, model_name);
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
                    ReasoningPickerTarget::NormalAgent => {
                        PickerSelection::Reasoning(reasoning_effort)
                    }
                    ReasoningPickerTarget::PlanningAgent => {
                        let planning_agent = PlanningAgentConfig {
                            model_name,
                            reasoning_effort,
                        };
                        if let Some(existing) = self
                            .session
                            .planning_agents
                            .iter_mut()
                            .find(|agent| agent.model_name == planning_agent.model_name)
                        {
                            *existing = planning_agent.clone();
                        } else {
                            self.session.planning_agents.push(planning_agent.clone());
                        }
                        PickerSelection::PlanningAgent(planning_agent)
                    }
                    ReasoningPickerTarget::SafetyModel => {
                        self.session.safety_model_name = model_name.clone();
                        self.session.safety_reasoning_effort = reasoning_effort;
                        PickerSelection::SafetySelection {
                            model_name,
                            reasoning_effort,
                        }
                    }
                }),
        }
    }

    fn move_command_selection(&mut self, direction: isize) {
        let commands = self.filtered_commands();
        if commands.is_empty() {
            return;
        }

        let current_index = commands
            .iter()
            .position(|command| *command == self.ui.selected_command)
            .unwrap_or(0);
        let next_index = (current_index as isize + direction).rem_euclid(commands.len() as isize);
        self.ui.selected_command = commands[next_index as usize];
    }

    pub(crate) fn sync_command_selection(&mut self) {
        let commands = self.filtered_commands();
        if let Some(command) = commands.first().copied()
            && !commands.contains(&self.ui.selected_command)
        {
            self.ui.selected_command = command;
        }
    }

    fn planning_models(&self) -> Vec<&'static model_registry::ModelInfo> {
        model_registry::models()
            .iter()
            .filter(|model| model.name != self.session.model_name)
            .collect()
    }

    fn move_picker_tab(&mut self, direction: isize) {
        let Some(SelectionPicker::Model { active_tab, .. }) = self.ui.picker.as_mut() else {
            return;
        };

        active_tab.toggle(direction);
    }

    fn move_picker_selection(&mut self, direction: isize) {
        let planning_len = self.planning_models().len();
        let Some(picker) = self.ui.picker.as_mut() else {
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
                    if len == 0 {
                        return;
                    }
                    *normal_selected_index = (*normal_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
                ModelPickerTab::PlanningAgents => {
                    if planning_len == 0 {
                        return;
                    }
                    *planning_selected_index = (*planning_selected_index as isize + direction)
                        .rem_euclid(planning_len as isize)
                        as usize;
                }
                ModelPickerTab::SafetyModel => {
                    let len = model_registry::models().len();
                    if len == 0 {
                        return;
                    }
                    *safety_selected_index = (*safety_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
            },
            SelectionPicker::Reasoning {
                options,
                selected_index,
                ..
            } => {
                let len = options.len();
                if len == 0 {
                    return;
                }
                *selected_index =
                    (*selected_index as isize + direction).rem_euclid(len as isize) as usize;
            }
        }
    }
}
