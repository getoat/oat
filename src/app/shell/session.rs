#[cfg(test)]
use std::path::PathBuf;

use super::*;

impl AppShell {
    pub(crate) fn set_composer_text(&mut self, text: &str) {
        self.set_composer_text_internal(text, true);
    }

    pub(crate) fn restore_command_history(&mut self, entries: Vec<String>, limit: usize) {
        self.session.command_history.restore(entries, limit);
    }

    pub(crate) fn take_command_history_to_persist(&mut self) -> Option<Vec<String>> {
        self.session.command_history.take_dirty_entries()
    }

    pub(crate) fn reset_session(&mut self) {
        let model_name = self.session.model_name.clone();
        let reasoning_effort = self.session.reasoning_effort;
        let planning_agents = self.session.planning_agents.clone();
        let workspace_root = self.session.workspace_root.clone();
        let safety_model_name = self.session.safety_model_name.clone();
        let safety_reasoning_effort = self.session.safety_reasoning_effort;
        let session_stats = self.session.session_stats;
        let next_reply_id = self.session.next_reply_id;
        let mut command_history = std::mem::take(&mut self.session.command_history);
        command_history.reset_navigation();

        self.session = SessionState::with_startup(
            self.session.show_thinking,
            self.session.show_tool_output,
            model_name,
            reasoning_effort,
            planning_agents,
            self.session.initial_mode,
            self.session.initial_approval_mode,
        );
        self.session.workspace_root = workspace_root;
        self.session.safety_model_name = safety_model_name;
        self.session.safety_reasoning_effort = safety_reasoning_effort;
        self.session.session_stats = session_stats;
        self.session.next_reply_id = next_reply_id;
        self.session.command_history = command_history;

        self.ui = UiState::default();
    }

    pub(crate) fn replace_session_history(&mut self, history: Vec<SessionHistoryMessage>) {
        self.session.replace_session_history(history);
    }

    pub(crate) fn set_last_history_model_name(&mut self, model_name: Option<impl Into<String>>) {
        self.session.last_history_model_name = model_name.map(Into::into);
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

    pub(crate) fn cancel_pending_reply(&mut self) {
        self.session.pending_reply = None;
        self.session.pending_write_approvals.clear();
        self.session.pending_shell_approvals.clear();
        self.session.pending_ask_user = None;
        self.ui.pending_shell_approval = None;
        self.ui.pending_ask_user = None;
        if self.session.planning.stage == PlanningStage::RunningFanout {
            start_conversation(&mut self.session.planning);
        }
        self.push_error_message("Request cancelled.");
    }
}
