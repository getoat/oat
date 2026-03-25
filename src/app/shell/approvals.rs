use super::*;

impl AppShell {
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
        let preview = mutation_preview(&tool_name, &arguments, &self.session.workspace_root);
        let source_context = source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        let approval = PendingWriteApproval {
            request_id,
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            summary: write_approval_summary(&tool_name, &arguments, &self.session.workspace_root),
            target: preview.as_ref().map(|preview| preview.target.clone()),
            source_label,
        };
        self.push_agent_message(format!(
            "Write approval required for `{}`{}.",
            approval.tool_name, source_context
        ));
        self.session.pending_write_approvals.push_back(approval);
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
        let pending = PendingShellApproval::new(
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
            source_label,
        );
        self.push_agent_message(format!(
            "{} risk shell approval required{}.",
            pending.risk.label(),
            source_context
        ));
        self.session.pending_shell_approvals.push_back(pending);
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

    pub(crate) fn apply_shell_approval_input(&mut self, input: EditorInput) {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.input(crate::app::ui::textarea_input(&input));
    }

    pub(crate) fn paste_into_shell_approval_detail(&mut self, text: &str) {
        let Some(pending) = self.ui.pending_shell_approval.as_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.insert_str(crate::app::ui::normalize_pasted_line_endings(text));
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

    pub(crate) fn shell_approval_session(&self) -> Option<&PendingShellApproval> {
        self.session.pending_shell_approvals.front()
    }

    pub(crate) fn shell_approval_ui(&self) -> Option<&ShellApprovalUiState> {
        self.ui.pending_shell_approval.as_ref()
    }
}
