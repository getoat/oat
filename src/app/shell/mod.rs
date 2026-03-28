use crate::{config::ReasoningSetting, features::planning::PlanningAgentConfig};

use super::{AccessMode, AppState, ApprovalMode, SessionState, UiState};

pub struct App {
    state: AppState,
}

impl App {
    #[cfg(test)]
    pub fn new(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning: impl Into<ReasoningSetting>,
    ) -> Self {
        Self::with_startup(
            show_thinking,
            show_tool_output,
            model_name,
            reasoning,
            Vec::new(),
            AccessMode::ReadOnly,
            ApprovalMode::Manual,
        )
    }

    pub fn with_startup(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning: impl Into<ReasoningSetting>,
        planning_agents: Vec<PlanningAgentConfig>,
        initial_mode: AccessMode,
        initial_approval_mode: ApprovalMode,
    ) -> Self {
        Self {
            state: AppState::new(
                SessionState::with_startup(
                    show_thinking,
                    show_tool_output,
                    model_name,
                    reasoning,
                    planning_agents,
                    initial_mode,
                    initial_approval_mode,
                ),
                UiState::default(),
            ),
        }
    }

    pub fn apply(&mut self, action: super::Action) -> Option<super::Effect> {
        crate::app::session::apply(&mut self.state, action)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    pub(crate) fn set_reasoning(&mut self, reasoning: ReasoningSetting) {
        self.state.session.reasoning = reasoning;
    }

    pub(crate) fn set_safety_reasoning(&mut self, reasoning: ReasoningSetting) {
        self.state.session.safety_reasoning = reasoning;
    }

    pub(crate) fn set_session_stats(&mut self, session_stats: crate::stats::StatsTotals) {
        self.state.session.session_stats = session_stats;
    }

    pub(crate) fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.state.session.model_name = model_name.into();
    }

    pub(crate) fn set_safety_model_name(&mut self, model_name: impl Into<String>) {
        self.state.session.safety_model_name = model_name.into();
    }

    pub(crate) fn set_planning_agents(
        &mut self,
        planning_agents: Vec<crate::features::planning::PlanningAgentConfig>,
    ) {
        self.state.session.planning_agents = planning_agents;
    }

    #[cfg(test)]
    pub(crate) fn set_workspace_root(&mut self, workspace_root: std::path::PathBuf) {
        self.state.session.workspace_root = workspace_root;
    }

    #[cfg(test)]
    pub(crate) fn push_agent_message(&mut self, text: impl Into<String>) {
        crate::app::ops::transcript::push_agent_message(self.state_mut(), text);
    }
}

#[cfg(test)]
#[allow(dead_code)]
impl App {
    pub(crate) fn mode(&self) -> AccessMode {
        self.state.session.mode
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.state.session.should_quit
    }

    pub(crate) fn composer(&self) -> &ratatui_textarea::TextArea<'static> {
        &self.state.ui.composer.composer
    }

    pub(crate) fn composer_mut(&mut self) -> &mut ratatui_textarea::TextArea<'static> {
        self.state.ui.invalidate_composer_layout();
        self.state.ui.composer.visual_column = None;
        &mut self.state.ui.composer.composer
    }

    pub(crate) fn set_composer_wrap_width(&mut self, width: usize) {
        crate::app::ops::composer::set_composer_wrap_width(self.state_mut(), width);
    }

    pub(crate) fn set_composer_cursor(&mut self, row: u16, col: u16) {
        crate::app::ops::composer::set_composer_cursor(self.state_mut(), row, col);
    }

    pub(crate) fn composer_has_content(&self) -> bool {
        crate::app::ops::composer::composer_has_content(&self.state)
    }

    pub(crate) fn entries(&self) -> &[super::TranscriptEntry] {
        &self.state.session.entries
    }

    pub(crate) fn session_history(&self) -> &[super::SessionHistoryMessage] {
        &self.state.session.session_history
    }

    pub(crate) fn has_pending_reply(&self) -> bool {
        crate::app::query::has_pending_reply(&self.state)
    }

    pub(crate) fn has_pending_ask_user(&self) -> bool {
        crate::app::query::has_pending_ask_user(&self.state)
    }

    pub(crate) fn pending_ask_user(&self) -> Option<&super::PendingAskUser> {
        self.state.session.pending_ask_user.as_ref()
    }

    pub(crate) fn ask_user_ui(&self) -> Option<&super::ui::AskUserUiState> {
        self.state.ui.pending_ask_user.as_ref()
    }

    pub(crate) fn planning_session_stage(
        &self,
    ) -> Option<crate::features::planning::PlanningStage> {
        crate::app::query::planning_session_stage(&self.state)
    }

    pub(crate) fn planning_draft_mode(&self) -> bool {
        crate::app::query::planning_draft_mode(&self.state)
    }

    pub(crate) fn plan_active(&self) -> bool {
        crate::app::query::plan_active(&self.state)
    }

    pub(crate) fn plan_review_selection_active(&self) -> bool {
        crate::app::query::plan_review_selection_active(&self.state)
    }

    pub(crate) fn history_is_pinned(&self) -> bool {
        crate::app::query::history_is_pinned(&self.state)
    }

    pub(crate) fn has_visible_pending_content(&self) -> bool {
        crate::app::session::has_visible_pending_content(&self.state.session)
    }

    pub(crate) fn selection_picker_visible(&self) -> bool {
        crate::app::query::selection_picker_visible(&self.state)
    }

    pub(crate) fn selected_command(&self) -> Option<super::SlashCommand> {
        crate::app::query::selected_command(&self.state)
    }

    pub(crate) fn history_pending_status_label(&self) -> &'static str {
        crate::app::query::history_pending_status_label_state(&self.state)
    }

    pub(crate) fn current_todo(&self) -> Option<&crate::todo::TodoSnapshot> {
        crate::app::query::current_todo(&self.state)
    }

    pub(crate) fn sync_history_viewport(
        &mut self,
        total_lines: usize,
        viewport_rows: usize,
    ) -> usize {
        self.state
            .ui
            .history
            .sync_viewport(total_lines, viewport_rows)
    }

    pub(crate) fn update_history_snapshot_for_test(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        lines: Vec<String>,
    ) {
        self.state.ui.history.update_snapshot(
            ratatui::layout::Rect {
                x,
                y,
                width,
                height,
            },
            lines,
        );
    }

    pub(crate) fn open_model_picker(&mut self) {
        crate::app::ops::picker::open_model_picker(self.state_mut());
    }

    pub(crate) fn open_reasoning_picker(&mut self) {
        crate::app::ops::picker::open_reasoning_picker(self.state_mut());
    }

    pub(crate) fn sync_command_selection(&mut self) {
        crate::app::ops::composer::sync_command_selection(self.state_mut());
    }

    pub(crate) fn begin_ask_user(
        &mut self,
        request_id: String,
        request: crate::ask_user::AskUserRequest,
    ) {
        crate::app::ops::ask_user::begin_ask_user(self.state_mut(), request_id, request);
    }

    pub(crate) fn begin_plan_review(&mut self) {
        crate::app::ops::planning::begin_plan_review(self.state_mut());
    }

    pub(crate) fn selected_plan_review_index(&self) -> Option<usize> {
        crate::app::query::selected_plan_review_index(self.state())
    }

    pub(crate) fn enter_planning_draft_mode(&mut self) {
        crate::app::ops::planning::enter_planning_draft_mode(self.state_mut());
    }

    pub(crate) fn begin_planning_conversation(&mut self) {
        crate::app::ops::planning::begin_planning_conversation(self.state_mut());
    }

    pub(crate) fn restore_command_history(&mut self, entries: Vec<String>, limit: usize) {
        crate::app::ops::session::restore_command_history(self.state_mut(), entries, limit);
    }

    pub(crate) fn take_command_history_to_persist(&mut self) -> Option<Vec<String>> {
        crate::app::ops::session::take_command_history_to_persist(self.state_mut())
    }

    pub(crate) fn replace_session_history(&mut self, history: Vec<super::SessionHistoryMessage>) {
        self.state.session.replace_session_history(history);
    }
}
