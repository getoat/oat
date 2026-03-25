#[cfg(test)]
use std::path::PathBuf;

use super::App;
use crate::{config::ReasoningEffort, features::planning::PlanningAgentConfig, stats::StatsTotals};

#[cfg(test)]
use crate::app::SessionHistoryMessage;

impl App {
    pub(crate) fn restore_command_history(&mut self, entries: Vec<String>, limit: usize) {
        self.session.command_history.restore(entries, limit);
    }

    pub(crate) fn take_command_history_to_persist(&mut self) -> Option<Vec<String>> {
        self.session.command_history.take_dirty_entries()
    }

    #[cfg(test)]
    pub(crate) fn replace_session_history(&mut self, history: Vec<SessionHistoryMessage>) {
        self.session.replace_session_history(history);
    }

    pub(crate) fn set_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.session.reasoning_effort = reasoning_effort;
    }

    pub(crate) fn set_safety_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.session.safety_reasoning_effort = reasoning_effort;
    }

    pub(crate) fn set_session_stats(&mut self, session_stats: StatsTotals) {
        self.session.session_stats = session_stats;
    }

    pub(crate) fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.session.model_name = model_name.into();
    }

    pub(crate) fn set_safety_model_name(&mut self, model_name: impl Into<String>) {
        self.session.safety_model_name = model_name.into();
    }

    pub(crate) fn set_planning_agents(&mut self, planning_agents: Vec<PlanningAgentConfig>) {
        self.session.planning_agents = planning_agents;
    }

    #[cfg(test)]
    pub(crate) fn set_workspace_root(&mut self, workspace_root: PathBuf) {
        self.session.workspace_root = workspace_root;
    }
}
