mod approvals;
mod ask_user;
mod composer;
mod history;
mod picker;
mod planning;
mod session;
mod transcript;

use std::path::Path;

use ratatui_textarea::TextArea;

use crate::{
    config::ReasoningEffort,
    features::planning::{PlanReviewState, PlanningAgentConfig, PlanningStage},
    model_registry,
    stats::StatsTotals,
};

use super::session::latest_proposed_plan_message;
use super::ui::{ShellApprovalUiState, split_command_query};
use super::{
    AccessMode, ApprovalMode, PendingReplyKind, PendingReplyReplaySeed, PendingWriteApproval,
    SelectionPicker, SessionHistoryMessage, SessionState, SlashCommand, TranscriptEntry, UiState,
};

pub struct App {
    pub session: SessionState,
    pub ui: UiState,
}

impl App {
    pub fn new(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning_effort: ReasoningEffort,
    ) -> Self {
        Self::with_startup(
            show_thinking,
            show_tool_output,
            model_name,
            reasoning_effort,
            Vec::new(),
            AccessMode::ReadOnly,
            ApprovalMode::Manual,
        )
    }

    pub fn with_startup(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning_effort: ReasoningEffort,
        planning_agents: Vec<PlanningAgentConfig>,
        initial_mode: AccessMode,
        initial_approval_mode: ApprovalMode,
    ) -> Self {
        Self {
            session: SessionState::with_startup(
                show_thinking,
                show_tool_output,
                model_name,
                reasoning_effort,
                planning_agents,
                initial_mode,
                initial_approval_mode,
            ),
            ui: UiState::default(),
        }
    }

    pub fn apply(&mut self, action: super::Action) -> Option<super::Effect> {
        crate::app::session::apply(&mut self.session, &mut self.ui, action)
    }

    fn reducer_context(&mut self) -> crate::app::ReducerContext<'_> {
        crate::app::ReducerContext::new(&mut self.session, &mut self.ui)
    }

    pub fn mode(&self) -> AccessMode {
        self.session.mode
    }

    pub fn workspace_root(&self) -> &Path {
        self.session.workspace_root()
    }

    pub fn approval_mode(&self) -> ApprovalMode {
        self.session.approval_mode
    }

    pub fn safety_model_name(&self) -> &str {
        &self.session.safety_model_name
    }

    pub fn safety_reasoning_effort(&self) -> ReasoningEffort {
        self.session.safety_reasoning_effort
    }

    pub fn pending_write_approval(&self) -> Option<&PendingWriteApproval> {
        self.session.pending_write_approvals.front()
    }

    pub fn main_pending_write_approval_request_id(&self) -> Option<&str> {
        self.session
            .pending_write_approvals
            .iter()
            .find(|pending| pending.source_label.is_none())
            .map(|pending| pending.request_id.as_str())
    }

    pub fn has_pending_write_approval(&self) -> bool {
        !self.session.pending_write_approvals.is_empty()
    }

    pub fn main_pending_shell_approval_request_id(&self) -> Option<&str> {
        self.session
            .pending_shell_approvals
            .iter()
            .find(|pending| pending.source_label.is_none())
            .map(|pending| pending.request_id.as_str())
    }

    pub fn has_pending_shell_approval(&self) -> bool {
        !self.session.pending_shell_approvals.is_empty()
    }

    pub fn shell_approval_editing(&self) -> bool {
        self.ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::is_editing)
    }

    pub fn shell_approval_editor_can_move_up(&self) -> bool {
        self.ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::editor_can_move_up)
    }

    pub fn shell_approval_editor_can_move_down(&self) -> bool {
        self.ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::editor_can_move_down)
    }

    pub fn has_pending_ask_user(&self) -> bool {
        self.session.pending_ask_user.is_some()
    }

    pub fn ask_user_review_active(&self) -> bool {
        self.session
            .pending_ask_user
            .as_ref()
            .zip(self.ui.pending_ask_user.as_ref())
            .is_some_and(|(pending, ui)| ui.active_tab == pending.questions.len())
    }

    pub fn ask_user_detail_editing(&self) -> bool {
        self.ui
            .pending_ask_user
            .as_ref()
            .is_some_and(|pending| pending.detail_editing)
    }

    pub fn plan_review_selection_active(&self) -> bool {
        self.session.planning.stage == PlanningStage::Review
            && self.session.planning.review == Some(PlanReviewState::Selection)
    }

    pub fn plan_review_feedback_active(&self) -> bool {
        self.session.planning.stage == PlanningStage::Review
            && self.session.planning.review == Some(PlanReviewState::Feedback)
    }

    pub fn planning_session_stage(&self) -> Option<PlanningStage> {
        (self.session.planning.stage != PlanningStage::Idle).then_some(self.session.planning.stage)
    }

    pub fn should_quit(&self) -> bool {
        self.session.should_quit
    }

    pub fn composer(&self) -> &TextArea<'static> {
        &self.ui.composer.composer
    }

    pub fn composer_mut(&mut self) -> &mut TextArea<'static> {
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        &mut self.ui.composer.composer
    }

    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.session.entries
    }

    pub fn latest_proposed_plan_message(&self) -> Option<&str> {
        latest_proposed_plan_message(&self.session)
    }

    pub fn session_history(&self) -> &[SessionHistoryMessage] {
        &self.session.session_history
    }

    pub(crate) fn shows_startup_banner(&self) -> bool {
        crate::app::session::shows_startup_banner(&self.session)
    }

    pub fn has_pending_reply(&self) -> bool {
        self.session.pending_reply.is_some()
    }

    pub fn has_visible_pending_content(&self) -> bool {
        crate::app::session::has_visible_pending_content(&self.session)
    }

    pub fn should_show_history_busy_indicator(&self) -> bool {
        crate::app::session::should_show_history_busy_indicator(&self.session)
    }

    pub fn history_pending_status_label(&self) -> &'static str {
        crate::app::session::history_pending_status_label(&self.session)
    }

    pub fn composer_height(&mut self) -> u16 {
        self.ui.composer_layout().height().saturating_add(2) as u16
    }

    pub fn overlay_height(&self) -> u16 {
        if let Some(picker) = self.selection_picker() {
            return crate::app::ui::picker_height(picker);
        }

        self.command_palette_height()
    }

    pub fn command_palette_height(&self) -> u16 {
        if !self.command_palette_visible() {
            return 0;
        }

        let line_count = self.filtered_commands().len().clamp(1, 4) as u16;
        line_count + 2
    }

    pub fn composer_has_content(&self) -> bool {
        self.ui
            .composer
            .composer
            .lines()
            .iter()
            .any(|line| !line.is_empty())
    }

    pub fn show_thinking(&self) -> bool {
        self.session.show_thinking
    }

    pub fn model_name(&self) -> &str {
        &self.session.model_name
    }

    pub fn reasoning_effort(&self) -> ReasoningEffort {
        self.session.reasoning_effort
    }

    pub fn last_history_model_name(&self) -> Option<&str> {
        self.session.last_history_model_name.as_deref()
    }

    pub fn planning_agents(&self) -> &[PlanningAgentConfig] {
        &self.session.planning_agents
    }

    pub fn planning_draft_mode(&self) -> bool {
        self.session.planning.stage == PlanningStage::Drafting
    }

    pub fn plan_active(&self) -> bool {
        self.session.planning.stage != PlanningStage::Idle
            || self
                .session
                .pending_reply
                .as_ref()
                .is_some_and(|pending| pending.kind == PendingReplyKind::Planning)
    }

    pub fn current_model_info(&self) -> Option<&'static model_registry::ModelInfo> {
        crate::app::session::current_model_info(&self.session)
    }

    pub fn show_tool_output(&self) -> bool {
        self.session.show_tool_output
    }

    pub fn session_stats(&self) -> StatsTotals {
        self.session.session_stats
    }

    pub fn estimated_next_request_context_tokens(&self) -> u64 {
        self.session.estimated_session_history_tokens
    }

    pub fn next_request_context_percent(&self) -> u64 {
        crate::app::session::next_request_context_percent(&self.session)
    }

    pub fn tick_count(&self) -> usize {
        self.session.tick_count
    }

    pub fn command_palette_visible(&self) -> bool {
        self.selection_picker().is_none() && self.command_query().is_some()
    }

    pub fn selection_picker(&self) -> Option<&SelectionPicker> {
        self.ui.picker.as_ref()
    }

    pub fn selection_picker_visible(&self) -> bool {
        self.ui.picker.is_some()
    }

    pub fn history_is_pinned(&self) -> bool {
        self.ui.history.is_pinned()
    }

    pub fn history_status_label(&self) -> &'static str {
        if self.history_is_pinned() {
            "History pinned  End latest"
        } else {
            "History live  PgUp/PgDn scroll"
        }
    }

    pub fn command_query(&self) -> Option<&str> {
        let [line] = self.ui.composer.composer.lines() else {
            return None;
        };

        line.starts_with('/').then_some(line.as_str())
    }

    pub fn command_name(&self) -> Option<&str> {
        self.command_query()
            .map(split_command_query)
            .map(|(name, _)| name)
    }

    pub fn command_arguments(&self) -> Option<&str> {
        self.command_query()
            .map(split_command_query)
            .map(|(_, args)| args)
    }

    pub fn filtered_commands(&self) -> Vec<SlashCommand> {
        self.command_name()
            .map(SlashCommand::filtered)
            .unwrap_or_default()
    }

    pub fn selected_command(&self) -> Option<SlashCommand> {
        let commands = self.filtered_commands();
        commands
            .contains(&self.ui.selected_command)
            .then_some(self.ui.selected_command)
            .or_else(|| commands.first().copied())
    }

    pub fn supported_reasoning_levels(&self) -> Vec<ReasoningEffort> {
        crate::app::session::supported_reasoning_levels(&self.session)
    }

    pub(crate) fn active_reply_id(&self) -> Option<u64> {
        self.session
            .pending_reply
            .as_ref()
            .map(|pending| pending.id)
    }

    pub(crate) fn active_reply_kind(&self) -> Option<PendingReplyKind> {
        self.session
            .pending_reply
            .as_ref()
            .map(|pending| pending.kind)
    }

    pub(crate) fn pending_reply_replay_seed(&self) -> Option<PendingReplyReplaySeed> {
        self.session
            .pending_reply
            .as_ref()
            .map(|pending| PendingReplyReplaySeed {
                plain_text: pending.plain_text.clone(),
                reasoning_text: pending.reasoning_text.clone(),
                commentary_messages: pending.commentary_messages.clone(),
            })
    }

    pub(crate) fn ensure_pending_reply(&mut self, kind: PendingReplyKind) -> u64 {
        self.session.ensure_pending_reply(kind)
    }
}
