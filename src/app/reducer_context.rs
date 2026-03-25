use ratatui_textarea::{CursorMove, TextArea};

use crate::{
    app::{
        PickerSelection, ReasoningPickerTarget,
        session::{
            AccessMode, ApprovalMode, ChatMessage, CommandRisk, EditorInput, MessageStyle,
            PendingAskUser, PendingReply, PendingReplyKind, PendingReplyReplaySeed,
            PendingWriteApproval, SessionHistoryMessage, SessionState, ShellApprovalDecision,
            SlashCommand, Speaker, SubagentDisplayState, SubagentStatusEntry, SubagentStatusKind,
            ToolCall, ToolResultEntry, TranscriptEntry, WriteApprovalDecision, current_model_info,
            latest_proposed_plan_message, pending_stream_text_is_visible,
            supported_reasoning_levels,
        },
        ui::{
            AskUserUiState, ModelPickerTab, SelectionPicker, ShellApprovalEditMode,
            ShellApprovalUiState, UiState, new_composer_with_text, normalize_pasted_line_endings,
            split_command_query, textarea_input,
        },
    },
    ask_user::{AskUserRequest, AskUserResponse},
    config::ReasoningEffort,
    features::planning::{
        PlanReviewState, PlanningAgentConfig, PlanningStage, accept_brief_and_start_fanout,
        accept_review_for_implementation, cancel_draft, clear_planning, default_planning_reasoning,
        request_review_changes, show_review, start_conversation, start_finalization,
    },
    model_registry,
    tools::mutation_preview,
};

pub(crate) struct ReducerContext<'a> {
    pub(super) session: &'a mut SessionState,
    pub(super) ui: &'a mut UiState,
}

impl<'a> ReducerContext<'a> {
    pub(crate) fn new(session: &'a mut SessionState, ui: &'a mut UiState) -> Self {
        Self { session, ui }
    }

    pub(crate) fn mode(&self) -> AccessMode {
        self.session.mode
    }

    pub(crate) fn has_pending_reply(&self) -> bool {
        self.session.pending_reply.is_some()
    }

    pub(crate) fn has_pending_write_approval(&self) -> bool {
        !self.session.pending_write_approvals.is_empty()
    }

    pub(crate) fn has_pending_shell_approval(&self) -> bool {
        !self.session.pending_shell_approvals.is_empty()
    }

    pub(crate) fn has_pending_ask_user(&self) -> bool {
        self.session.pending_ask_user.is_some()
    }

    pub(crate) fn plan_review_selection_active(&self) -> bool {
        self.session.planning.stage == PlanningStage::Review
            && self.session.planning.review == Some(PlanReviewState::Selection)
    }

    pub(crate) fn plan_review_feedback_active(&self) -> bool {
        self.session.planning.stage == PlanningStage::Review
            && self.session.planning.review == Some(PlanReviewState::Feedback)
    }

    pub(crate) fn planning_session_stage(&self) -> Option<PlanningStage> {
        (self.session.planning.stage != PlanningStage::Idle).then_some(self.session.planning.stage)
    }

    pub(crate) fn planning_draft_mode(&self) -> bool {
        self.session.planning.stage == PlanningStage::Drafting
    }

    pub(crate) fn show_thinking(&self) -> bool {
        self.session.show_thinking
    }

    pub(crate) fn model_name(&self) -> &str {
        &self.session.model_name
    }

    pub(crate) fn reasoning_effort(&self) -> ReasoningEffort {
        self.session.reasoning_effort
    }

    pub(crate) fn session_history(&self) -> &[SessionHistoryMessage] {
        &self.session.session_history
    }

    pub(crate) fn last_history_model_name(&self) -> Option<&str> {
        self.session.last_history_model_name.as_deref()
    }

    pub(crate) fn planning_agents(&self) -> &[PlanningAgentConfig] {
        &self.session.planning_agents
    }

    pub(crate) fn current_model_info(&self) -> Option<&'static model_registry::ModelInfo> {
        current_model_info(self.session)
    }

    pub(crate) fn supported_reasoning_levels(&self) -> Vec<ReasoningEffort> {
        supported_reasoning_levels(self.session)
    }

    pub(crate) fn latest_proposed_plan_message(&self) -> Option<&str> {
        latest_proposed_plan_message(self.session)
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

    pub(crate) fn next_reply_id(&mut self) -> u64 {
        self.session.next_reply_id()
    }

    pub(crate) fn set_pending_reply(&mut self, reply_id: u64, kind: PendingReplyKind) {
        self.session.pending_reply = Some(PendingReply::new(reply_id, kind));
    }

    pub(crate) fn clear_pending_reply_only(&mut self) {
        self.session.pending_reply = None;
    }

    pub(crate) fn replace_session_history(&mut self, history: Vec<SessionHistoryMessage>) {
        self.session.replace_session_history(history);
    }

    pub(crate) fn set_last_history_model_name(&mut self, model_name: Option<impl Into<String>>) {
        self.session.last_history_model_name = model_name.map(Into::into);
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

        *self.session = SessionState::with_startup(
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

        *self.ui = UiState::default();
    }

    pub(crate) fn set_should_quit(&mut self) {
        self.session.should_quit = true;
    }

    pub(crate) fn clear_composer(&mut self) {
        self.set_composer_text_internal("", true);
    }

    pub(crate) fn composer_has_content(&self) -> bool {
        self.ui
            .composer
            .composer
            .lines()
            .iter()
            .any(|line| !line.is_empty())
    }

    pub(crate) fn submitted_composer_text(&self) -> String {
        self.ui
            .composer
            .composer
            .lines()
            .join("\n")
            .trim()
            .to_owned()
    }

    pub(crate) fn set_composer_text(&mut self, text: &str) {
        self.set_composer_text_internal(text, true);
    }

    fn set_composer_text_internal(&mut self, text: &str, reset_command_history: bool) {
        let mut composer = new_composer_with_text(text);
        composer.move_cursor(CursorMove::End);
        self.ui.composer.composer = composer;
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        if reset_command_history {
            self.session.command_history.reset_navigation();
        }
        self.sync_command_selection();
    }

    pub(crate) fn move_composer_cursor_up(&mut self) {
        let current_cursor = self.ui.composer.composer.cursor();
        let target = {
            let Some(cursor) = self.ui.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index == 0 {
                if cursor.visual_col > 0 {
                    Some((cursor.row.line_index, cursor.row.start_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.ui.composer.visual_column.unwrap_or(cursor.visual_col);
                self.ui
                    .composer_layout()
                    .target_cursor_for_row(cursor.row_index - 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.ui
                    .composer
                    .composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.ui.composer.visual_column = desired_col;
            }
            None => {
                self.ui.composer.visual_column = None;
            }
        }
    }

    pub(crate) fn move_composer_cursor_down(&mut self) {
        let current_cursor = self.ui.composer.composer.cursor();
        let target = {
            let Some(cursor) = self.ui.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index + 1 >= cursor.total_rows {
                if current_cursor.1 < cursor.row.end_col {
                    Some((cursor.row.line_index, cursor.row.end_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.ui.composer.visual_column.unwrap_or(cursor.visual_col);
                self.ui
                    .composer_layout()
                    .target_cursor_for_row(cursor.row_index + 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.ui
                    .composer
                    .composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.ui.composer.visual_column = desired_col;
            }
            None => {
                self.ui.composer.visual_column = None;
            }
        }
    }

    pub(crate) fn insert_composer_newline(&mut self) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui.composer.composer.insert_newline();
        self.sync_command_selection();
    }

    pub(crate) fn apply_composer_input(&mut self, input: EditorInput) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui.composer.composer.input(textarea_input(&input));
        self.sync_command_selection();
    }

    pub(crate) fn paste_into_composer(&mut self, text: &str) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui
            .composer
            .composer
            .insert_str(normalize_pasted_line_endings(text));
        self.sync_command_selection();
    }

    pub(crate) fn record_submitted_input(&mut self, text: &str) {
        self.session.command_history.record(text);
    }

    pub(crate) fn should_recall_previous_input(&mut self) -> bool {
        let current_cursor = self.ui.composer.composer.cursor();
        self.ui
            .composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| cursor.row_index == 0 && cursor.visual_col == 0)
    }

    pub(crate) fn should_recall_next_input(&mut self) -> bool {
        let current_cursor = self.ui.composer.composer.cursor();
        self.ui
            .composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| {
                cursor.row_index + 1 >= cursor.total_rows && current_cursor.1 == cursor.row.end_col
            })
    }

    pub(crate) fn recall_previous_input(&mut self) -> bool {
        let current = self.ui.composer.composer.lines().join("\n");
        let Some(previous) = self.session.command_history.previous(&current) else {
            return false;
        };
        self.set_composer_text_internal(&previous, false);
        true
    }

    pub(crate) fn recall_next_input(&mut self) -> bool {
        let Some(next) = self.session.command_history.next() else {
            return false;
        };
        self.set_composer_text_internal(&next, false);
        true
    }

    pub(crate) fn begin_write_approval(
        &mut self,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        self.enqueue_write_approval(None, request_id, tool_name, arguments);
    }

    pub(crate) fn begin_subagent_write_approval(
        &mut self,
        subagent_id: String,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        self.enqueue_write_approval(Some(subagent_id), request_id, tool_name, arguments);
    }

    pub(crate) fn begin_shell_approval(
        &mut self,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        self.enqueue_shell_approval(
            None,
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        );
    }

    pub(crate) fn begin_subagent_shell_approval(
        &mut self,
        subagent_id: String,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        self.enqueue_shell_approval(
            Some(subagent_id),
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        );
    }

    fn enqueue_write_approval(
        &mut self,
        source_label: Option<String>,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        let source_context = source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        self.push_agent_message(format!(
            "Write approval required for `{}`{}.",
            tool_name, source_context
        ));
        self.session
            .enqueue_write_approval(source_label, request_id, tool_name, arguments);
    }

    fn enqueue_shell_approval(
        &mut self,
        source_label: Option<String>,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        let source_context = source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        self.push_agent_message(format!(
            "{} risk shell approval required{}.",
            risk.label(),
            source_context
        ));
        self.session.enqueue_shell_approval(
            source_label,
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        );
        self.sync_pending_shell_approval_ui();
    }

    pub(crate) fn resolve_write_approval(
        &mut self,
        decision: WriteApprovalDecision,
    ) -> Option<PendingWriteApproval> {
        let pending = self.session.pending_write_approvals.pop_front()?;
        let source_context = pending
            .source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        match decision {
            WriteApprovalDecision::AllowOnce => {
                self.push_agent_message(format!(
                    "Approved `{}` once{}.",
                    pending.tool_name, source_context
                ));
            }
            WriteApprovalDecision::AllowAllSession => {
                self.session.approval_mode = ApprovalMode::Disabled;
                self.push_agent_message(format!(
                    "Approved `{}` and all future writes for this session{}.",
                    pending.tool_name, source_context
                ));
            }
            WriteApprovalDecision::Deny => {
                self.push_error_message(format!(
                    "Denied `{}`{}.",
                    pending.tool_name, source_context
                ));
            }
        }
        Some(pending)
    }

    pub(crate) fn move_shell_approval_selection(&mut self, direction: isize) {
        if let Some(pending) = self.ui.pending_shell_approval.as_mut() {
            pending.move_selection(direction);
        }
    }

    pub(crate) fn cancel_shell_approval_editing(&mut self) -> bool {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return false;
        };
        if pending.edit_mode != Some(ShellApprovalEditMode::Deny) {
            return false;
        }
        pending.cancel_editing();
        true
    }

    pub(crate) fn toggle_shell_approval_detail_editing(&mut self) {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return;
        };
        if pending.selected_index != 3 {
            return;
        }
        pending.edit_mode = match pending.edit_mode {
            Some(ShellApprovalEditMode::Deny) => None,
            _ => Some(ShellApprovalEditMode::Deny),
        };
    }

    pub(crate) fn shell_approval_editing(&self) -> bool {
        self.ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::is_editing)
    }

    pub(crate) fn apply_shell_approval_input(&mut self, input: EditorInput) {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.input(textarea_input(&input));
    }

    pub(crate) fn paste_into_shell_approval_detail(&mut self, text: &str) {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.insert_str(normalize_pasted_line_endings(text));
    }

    pub(crate) fn submit_shell_approval(
        &mut self,
    ) -> Option<(String, ShellApprovalDecision, CommandRisk)> {
        let pending_ui = self.ui.pending_shell_approval.as_mut()?;
        if pending_ui.is_editing() {
            if pending_ui.edit_mode == Some(ShellApprovalEditMode::Pattern)
                && pending_ui.selected_decision().is_none()
            {
                self.push_error_message("Provide a non-empty shell approval pattern.");
                return None;
            }
            if pending_ui.edit_mode == Some(ShellApprovalEditMode::Deny) {
                pending_ui.cancel_editing();
            }
        } else if pending_ui.selected_index == 1 {
            pending_ui.begin_editing();
            return None;
        }

        let pending = self.session.pending_shell_approvals.pop_front()?;
        let decision = self
            .ui
            .pending_shell_approval
            .as_ref()
            .and_then(ShellApprovalUiState::selected_decision)
            .unwrap_or(ShellApprovalDecision::Deny(None));
        self.ui.pending_shell_approval = None;
        self.sync_pending_shell_approval_ui();

        let source_context = pending
            .source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        match &decision {
            ShellApprovalDecision::AllowOnce => self.push_agent_message(format!(
                "Approved {} risk shell command once{}.",
                pending.risk.as_str(),
                source_context
            )),
            ShellApprovalDecision::AllowPattern(pattern) => self.push_agent_message(format!(
                "Approved {} risk shell commands matching `{}`{}.",
                pending.risk.as_str(),
                pattern,
                source_context
            )),
            ShellApprovalDecision::AllowAllRisk => self.push_agent_message(format!(
                "Approved all future {} risk shell commands this session{}.",
                pending.risk.as_str(),
                source_context
            )),
            ShellApprovalDecision::Deny(note) => {
                let suffix = note
                    .as_deref()
                    .filter(|note| !note.is_empty())
                    .map(|note| format!(" ({note})"))
                    .unwrap_or_default();
                self.push_error_message(format!(
                    "Denied {} risk shell command{}{}.",
                    pending.risk.as_str(),
                    source_context,
                    suffix
                ));
            }
        }

        Some((pending.request_id, decision, pending.risk))
    }

    fn sync_pending_shell_approval_ui(&mut self) {
        let next_request_id = self
            .session
            .pending_shell_approvals
            .front()
            .map(|pending| pending.request_id.as_str());
        let current_request_id = self
            .ui
            .pending_shell_approval
            .as_ref()
            .map(|pending| pending.request_id.as_str());

        if next_request_id == current_request_id {
            return;
        }

        self.ui.pending_shell_approval = self
            .session
            .pending_shell_approvals
            .front()
            .map(ShellApprovalUiState::new);
    }

    pub(crate) fn move_ask_user_tab_left(&mut self) {
        self.move_ask_user_tab(-1);
    }

    pub(crate) fn move_ask_user_tab_right(&mut self) {
        self.move_ask_user_tab(1);
    }

    pub(crate) fn move_ask_user_answer_up(&mut self) {
        self.move_ask_user_answer(-1);
    }

    pub(crate) fn move_ask_user_answer_down(&mut self) {
        self.move_ask_user_answer(1);
    }

    pub(crate) fn toggle_ask_user_detail_editing(&mut self) {
        let Some(pending) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(session_pending) = self.session.pending_ask_user.as_ref() else {
            return;
        };
        if pending.active_tab >= session_pending.questions.len() {
            return;
        }

        pending.detail_editing = !pending.detail_editing;
    }

    pub(crate) fn ask_user_detail_editing(&self) -> bool {
        self.ui
            .pending_ask_user
            .as_ref()
            .is_some_and(|pending| pending.detail_editing)
    }

    pub(crate) fn apply_ask_user_input(&mut self, input: EditorInput) {
        let Some(question) = self.active_ask_user_detail_input_mut() else {
            return;
        };
        question.input(textarea_input(&input));
    }

    pub(crate) fn paste_into_ask_user_detail(&mut self, text: &str) {
        let Some(question) = self.active_ask_user_detail_input_mut() else {
            return;
        };
        question.insert_str(normalize_pasted_line_endings(text));
    }

    pub(crate) fn submit_ask_user_response(&mut self) -> Option<(String, AskUserResponse, String)> {
        let pending = self.session.pending_ask_user.as_ref()?;
        let ui = self.ui.pending_ask_user.as_ref()?;
        if ui.active_tab != pending.questions.len() {
            return None;
        }
        if !pending.is_complete(|index| ui.detail_text(index)) {
            self.push_error_message("Complete all AskUser questions before submitting.");
            return None;
        }

        let response = pending.response(|index| ui.detail_text(index));
        let request_id = pending.request_id.clone();
        let summary = response.transcript_summary();
        self.session.pending_ask_user = None;
        self.ui.pending_ask_user = None;
        self.push_user_message(summary.clone());
        Some((request_id, response, summary))
    }

    pub(crate) fn advance_ask_user(&mut self) -> Option<(String, AskUserResponse, String)> {
        let Some(pending) = self.session.pending_ask_user.as_ref() else {
            return None;
        };
        let Some(ui) = self.ui.pending_ask_user.as_ref() else {
            return None;
        };
        if ui.active_tab == pending.questions.len() {
            return self.submit_ask_user_response();
        }

        let question = &pending.questions[ui.active_tab];
        if !question.is_complete(ui.detail_text(ui.active_tab)) {
            self.push_error_message("`Something else` requires details before continuing.");
            if let Some(ui) = self.ui.pending_ask_user.as_mut() {
                ui.detail_editing = true;
            }
            return None;
        }

        if let Some(ui) = self.ui.pending_ask_user.as_mut() {
            ui.detail_editing = false;
            ui.active_tab += 1;
        }
        None
    }

    fn active_ask_user_detail_input_mut(&mut self) -> Option<&mut TextArea<'static>> {
        let ui = self.ui.pending_ask_user.as_mut()?;
        let session = self.session.pending_ask_user.as_ref()?;
        if !ui.detail_editing || ui.active_tab >= session.questions.len() {
            return None;
        }
        ui.detail_inputs.get_mut(ui.active_tab)
    }

    fn move_ask_user_tab(&mut self, direction: isize) {
        let Some(ui) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(session) = self.session.pending_ask_user.as_ref() else {
            return;
        };

        let tab_count = session.questions.len() + 1;
        ui.active_tab =
            (ui.active_tab as isize + direction).rem_euclid(tab_count as isize) as usize;
        if ui.active_tab >= session.questions.len() {
            ui.detail_editing = false;
        }
    }

    fn move_ask_user_answer(&mut self, direction: isize) {
        let Some(ui) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(pending) = self.session.pending_ask_user.as_mut() else {
            return;
        };
        if ui.active_tab >= pending.questions.len() {
            return;
        }

        let question = &mut pending.questions[ui.active_tab];
        let len = question.answers.len();
        if len == 0 {
            return;
        }
        question.selected_index =
            (question.selected_index as isize + direction).rem_euclid(len as isize) as usize;
        if question.selected_answer().is_something_else {
            ui.detail_editing = true;
        }
    }

    pub(crate) fn begin_ask_user(&mut self, request_id: String, request: AskUserRequest) {
        self.session.pending_ask_user = Some(PendingAskUser::new(request_id, request));
        self.ui.pending_ask_user = self
            .session
            .pending_ask_user
            .as_ref()
            .map(AskUserUiState::new);
    }

    pub(crate) fn clear_pending_ask_user(&mut self) {
        self.session.pending_ask_user = None;
        self.ui.pending_ask_user = None;
    }

    pub(crate) fn begin_plan_review(&mut self) {
        show_review(&mut self.session.planning, PlanReviewState::Selection);
        self.ui.plan_review_selected_index = 0;
        self.clear_composer();
    }

    pub(crate) fn begin_plan_review_feedback(&mut self) {
        request_review_changes(&mut self.session.planning);
        self.clear_composer();
    }

    pub(crate) fn clear_plan_review(&mut self) {
        clear_planning(&mut self.session.planning);
        self.ui.plan_review_selected_index = 0;
    }

    pub(crate) fn accept_plan_review_for_implementation(&mut self) {
        accept_review_for_implementation(&mut self.session.planning);
        self.ui.plan_review_selected_index = 0;
    }

    pub(crate) fn selected_plan_review_index(&self) -> Option<usize> {
        self.plan_review_selection_active()
            .then_some(self.ui.plan_review_selected_index)
    }

    pub(crate) fn move_plan_review_selection(&mut self, direction: isize) {
        if !self.plan_review_selection_active() {
            return;
        }

        self.ui.plan_review_selected_index =
            (self.ui.plan_review_selected_index as isize + direction).rem_euclid(2) as usize;
    }

    pub(crate) fn enter_planning_draft_mode(&mut self) {
        crate::features::planning::enter_draft(&mut self.session.planning);
        self.clear_composer();
    }

    pub(crate) fn cancel_planning_draft_mode(&mut self) -> bool {
        if self.session.planning.stage != PlanningStage::Drafting {
            return false;
        }

        cancel_draft(&mut self.session.planning);
        self.clear_composer();
        true
    }

    pub(crate) fn consume_planning_draft_mode(&mut self) -> bool {
        let was_active = self.planning_draft_mode();
        if was_active {
            start_conversation(&mut self.session.planning);
        }
        was_active
    }

    pub(crate) fn begin_planning_conversation(&mut self) {
        start_conversation(&mut self.session.planning);
    }

    pub(crate) fn begin_planning_fanout(&mut self) {
        accept_brief_and_start_fanout(&mut self.session.planning);
    }

    pub(crate) fn begin_planning_finalization(&mut self) {
        start_finalization(&mut self.session.planning);
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

    pub(crate) fn cancel_picker(&mut self) -> bool {
        self.ui.picker.take().is_some()
    }

    pub(crate) fn selection_picker_visible(&self) -> bool {
        self.ui.picker.is_some()
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

    pub(crate) fn command_query(&self) -> Option<&str> {
        let [line] = self.ui.composer.composer.lines() else {
            return None;
        };

        line.starts_with('/').then_some(line.as_str())
    }

    pub(crate) fn command_name(&self) -> Option<&str> {
        self.command_query()
            .map(split_command_query)
            .map(|(name, _)| name)
    }

    pub(crate) fn command_arguments(&self) -> Option<&str> {
        self.command_query()
            .map(split_command_query)
            .map(|(_, args)| args)
    }

    pub(crate) fn filtered_commands(&self) -> Vec<SlashCommand> {
        self.command_name()
            .map(SlashCommand::filtered)
            .unwrap_or_default()
    }

    pub(crate) fn selected_command(&self) -> Option<SlashCommand> {
        let commands = self.filtered_commands();
        commands
            .contains(&self.ui.selected_command)
            .then_some(self.ui.selected_command)
            .or_else(|| commands.first().copied())
    }

    pub(crate) fn command_palette_visible(&self) -> bool {
        self.ui.picker.is_none() && self.command_query().is_some()
    }

    pub(crate) fn scroll_history_page_up(&mut self) {
        self.scroll_history_up(self.ui.history.page_rows());
    }

    pub(crate) fn scroll_history_page_down(&mut self) {
        self.scroll_history_down(self.ui.history.page_rows());
    }

    pub(crate) fn scroll_history_up(&mut self, lines: usize) {
        self.ui.history.scroll_up(lines);
    }

    pub(crate) fn scroll_history_down(&mut self, lines: usize) {
        self.ui.history.scroll_down(lines);
    }

    pub(crate) fn scroll_history_to_top(&mut self) {
        self.ui.history.scroll_to_top();
    }

    pub(crate) fn resume_history_follow(&mut self) {
        self.ui.history.resume_follow();
    }

    pub(crate) fn start_history_selection(&mut self, column: u16, row: u16) {
        self.ui.history.start_selection(column, row);
    }

    pub(crate) fn update_history_selection(&mut self, column: u16, row: u16) {
        self.ui.history.update_selection(column, row);
    }

    pub(crate) fn finish_history_selection(&mut self, column: u16, row: u16) -> Option<String> {
        self.ui.history.finish_selection(column, row)
    }

    pub(crate) fn push_agent_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Plain);
    }

    pub(crate) fn push_user_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::User, text, MessageStyle::Plain);
    }

    pub(crate) fn push_error_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Error);
    }

    pub(crate) fn push_agent_error(&mut self, text: impl Into<String>) {
        self.push_error_message(text);
    }

    pub(crate) fn push_agent_commentary(&mut self, text: impl Into<String>) {
        let text = text.into();
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.commentary_messages.push(text.clone());
            pending.has_visible_content = true;
        }
        self.push_message(Speaker::Agent, text, MessageStyle::Commentary);
    }

    pub(crate) fn push_tool_call(&mut self, name: String, parameter: String) {
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.has_visible_content = true;
        }
        self.session
            .entries
            .push(TranscriptEntry::ToolCall(ToolCall {
                preview: mutation_preview(&name, &parameter, &self.session.workspace_root),
                name,
                parameter,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn push_tool_result(&mut self, name: String, output: String) {
        if let Some(pending) = self.session.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            if self.session.show_tool_output {
                pending.has_visible_content = true;
            }
        }
        self.session
            .entries
            .push(TranscriptEntry::ToolResult(ToolResultEntry {
                name,
                output,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn upsert_subagent_status(
        &mut self,
        id: String,
        kind: SubagentStatusKind,
        display_label: String,
        state: SubagentDisplayState,
        status_text: String,
    ) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.kind = kind;
            entry.display_label = display_label;
            entry.state = state;
            entry.status_text = status_text;
            self.bump_transcript_revision();
            return;
        }

        self.session
            .entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                id,
                kind,
                display_label,
                state,
                status_text,
                latest_tool_name: None,
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn set_subagent_latest_tool(&mut self, id: String, latest_tool_name: String) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.latest_tool_name = Some(latest_tool_name);
            self.bump_transcript_revision();
            return;
        }

        self.session
            .entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                display_label: id.clone(),
                id,
                kind: SubagentStatusKind::Subagent,
                state: SubagentDisplayState::Running,
                status_text: "running".into(),
                latest_tool_name: Some(latest_tool_name),
            }));
        self.bump_transcript_revision();
    }

    pub(crate) fn append_pending_stream_message(&mut self, delta: &str, style: MessageStyle) {
        if delta.is_empty() || self.session.pending_reply.is_none() || style == MessageStyle::Error
        {
            return;
        }

        let existing_index = {
            let pending = self
                .session
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            let crossed_style_boundary = match style {
                MessageStyle::Plain => pending.reasoning_entry_index.is_some(),
                MessageStyle::Commentary => true,
                MessageStyle::Thinking => pending.text_entry_index.is_some(),
                MessageStyle::Error => false,
            };
            if crossed_style_boundary {
                pending.reset_active_stream_segment();
            }
            match style {
                MessageStyle::Plain => pending.text_entry_index,
                MessageStyle::Commentary => None,
                MessageStyle::Thinking => pending.reasoning_entry_index,
                MessageStyle::Error => None,
            }
        };

        let Some(existing_index) = existing_index else {
            let mut pending_text = delta.to_string();
            {
                let pending = self
                    .session
                    .pending_reply
                    .as_mut()
                    .expect("pending reply checked above");
                match style {
                    MessageStyle::Plain => {
                        pending.plain_text.push_str(delta);
                        pending.staged_plain_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_plain_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_plain_text);
                    }
                    MessageStyle::Thinking => {
                        pending.reasoning_text.push_str(delta);
                        pending.staged_reasoning_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_reasoning_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_reasoning_text);
                    }
                    MessageStyle::Commentary => {
                        if !pending_stream_text_is_visible(style, delta) {
                            return;
                        }
                    }
                    MessageStyle::Error => return,
                }
            }

            self.push_message(Speaker::Agent, pending_text, style);
            let index = self.session.entries.len() - 1;
            let pending = self
                .session
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            pending.has_visible_content = true;
            match style {
                MessageStyle::Plain => pending.text_entry_index = Some(index),
                MessageStyle::Commentary => {}
                MessageStyle::Thinking => pending.reasoning_entry_index = Some(index),
                MessageStyle::Error => {}
            }
            return;
        };

        if let Some(TranscriptEntry::Message(message)) =
            self.session.entries.get_mut(existing_index)
        {
            message.text.push_str(delta);
            if style == MessageStyle::Plain
                && let Some(pending) = self.session.pending_reply.as_mut()
            {
                pending.plain_text.push_str(delta);
            }
            self.bump_transcript_revision();
        }
    }

    fn push_message(&mut self, speaker: Speaker, text: impl Into<String>, style: MessageStyle) {
        self.session
            .entries
            .push(TranscriptEntry::Message(ChatMessage {
                speaker,
                text: text.into(),
                style,
            }));
        self.bump_transcript_revision();
    }

    fn bump_transcript_revision(&mut self) {
        self.session.transcript_revision = self.session.transcript_revision.wrapping_add(1);
        self.ui.history_render_cache = None;
    }
}
