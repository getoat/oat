use crate::app::{
    AppState, ApprovalMode, CommandRisk, EditorInput, PendingWriteApproval, ShellApprovalDecision,
    ShellApprovalEditMode, WriteApprovalDecision,
};

use super::transcript::{push_agent_message, push_error_message};

pub(crate) fn begin_write_approval(
    state: &mut AppState,
    request_id: String,
    tool_name: String,
    arguments: String,
) {
    enqueue_write_approval(state, None, request_id, tool_name, arguments);
}

pub(crate) fn begin_subagent_write_approval(
    state: &mut AppState,
    subagent_id: String,
    request_id: String,
    tool_name: String,
    arguments: String,
) {
    enqueue_write_approval(state, Some(subagent_id), request_id, tool_name, arguments);
}

pub(crate) fn begin_shell_approval(
    state: &mut AppState,
    request_id: String,
    risk: CommandRisk,
    risk_explanation: String,
    command: String,
    working_directory: String,
    reason: String,
) {
    enqueue_shell_approval(
        state,
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
    state: &mut AppState,
    subagent_id: String,
    request_id: String,
    risk: CommandRisk,
    risk_explanation: String,
    command: String,
    working_directory: String,
    reason: String,
) {
    enqueue_shell_approval(
        state,
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
    state: &mut AppState,
    source_label: Option<String>,
    request_id: String,
    tool_name: String,
    arguments: String,
) {
    let source_context = source_label
        .as_ref()
        .map(|source| format!(" from `{source}`"))
        .unwrap_or_default();
    push_agent_message(
        state,
        format!(
            "Write approval required for `{}`{}.",
            tool_name, source_context
        ),
    );
    state
        .session
        .enqueue_write_approval(source_label, request_id, tool_name, arguments);
}

fn enqueue_shell_approval(
    state: &mut AppState,
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
    push_agent_message(
        state,
        format!(
            "{} risk shell approval required{}.",
            risk.label(),
            source_context
        ),
    );
    state.session.enqueue_shell_approval(
        source_label,
        request_id,
        risk,
        risk_explanation,
        command,
        working_directory,
        reason,
    );
    sync_pending_shell_approval_ui(state);
}

pub(crate) fn resolve_write_approval(
    state: &mut AppState,
    decision: WriteApprovalDecision,
) -> Option<PendingWriteApproval> {
    let pending = state.session.pending_write_approvals.pop_front()?;
    let source_context = pending
        .source_label
        .as_ref()
        .map(|source| format!(" from `{source}`"))
        .unwrap_or_default();
    match decision {
        WriteApprovalDecision::AllowOnce => {
            push_agent_message(
                state,
                format!("Approved `{}` once{}.", pending.tool_name, source_context),
            );
        }
        WriteApprovalDecision::AllowAllSession => {
            state.session.approval_mode = ApprovalMode::Disabled;
            push_agent_message(
                state,
                format!(
                    "Approved `{}` and all future writes for this session{}.",
                    pending.tool_name, source_context
                ),
            );
        }
        WriteApprovalDecision::Deny => {
            push_error_message(
                state,
                format!("Denied `{}`{}.", pending.tool_name, source_context),
            );
        }
    }
    Some(pending)
}

pub(crate) fn move_shell_approval_selection(state: &mut AppState, direction: isize) {
    if let Some(pending) = state.ui.pending_shell_approval.as_mut() {
        pending.move_selection(direction);
    }
}

pub(crate) fn cancel_shell_approval_editing(state: &mut AppState) -> bool {
    let Some(pending) = state.ui.pending_shell_approval.as_mut() else {
        return false;
    };
    if pending.edit_mode != Some(ShellApprovalEditMode::Deny) {
        return false;
    }
    pending.cancel_editing();
    true
}

pub(crate) fn toggle_shell_approval_detail_editing(state: &mut AppState) {
    let Some(pending) = state.ui.pending_shell_approval.as_mut() else {
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

pub(crate) fn apply_shell_approval_input(state: &mut AppState, input: EditorInput) {
    let Some(pending) = state.ui.pending_shell_approval.as_mut() else {
        return;
    };
    let Some(editor) = pending.active_editor_mut() else {
        return;
    };
    editor.input(crate::app::ui::textarea_input(&input));
}

pub(crate) fn paste_into_shell_approval_detail(state: &mut AppState, text: &str) {
    let Some(pending) = state.ui.pending_shell_approval.as_mut() else {
        return;
    };
    let Some(editor) = pending.active_editor_mut() else {
        return;
    };
    editor.insert_str(crate::app::ui::normalize_pasted_line_endings(text));
}

pub(crate) fn submit_shell_approval(
    state: &mut AppState,
) -> Option<(String, ShellApprovalDecision, CommandRisk)> {
    let pending_ui = state.ui.pending_shell_approval.as_mut()?;
    if pending_ui.is_editing() {
        if pending_ui.edit_mode == Some(ShellApprovalEditMode::Pattern)
            && pending_ui.selected_decision().is_none()
        {
            push_error_message(state, "Provide a non-empty shell approval pattern.");
            return None;
        }
        if pending_ui.edit_mode == Some(ShellApprovalEditMode::Deny) {
            pending_ui.cancel_editing();
        }
    } else if pending_ui.selected_index == 1 {
        pending_ui.begin_editing();
        return None;
    }

    let pending = state.session.pending_shell_approvals.pop_front()?;
    let decision = state
        .ui
        .pending_shell_approval
        .as_ref()
        .and_then(crate::app::ui::ShellApprovalUiState::selected_decision)
        .unwrap_or(ShellApprovalDecision::Deny(None));
    state.ui.pending_shell_approval = None;
    sync_pending_shell_approval_ui(state);

    let source_context = pending
        .source_label
        .as_ref()
        .map(|source| format!(" from `{source}`"))
        .unwrap_or_default();
    match &decision {
        ShellApprovalDecision::AllowOnce => push_agent_message(
            state,
            format!(
                "Approved {} risk shell command once{}.",
                pending.risk.as_str(),
                source_context
            ),
        ),
        ShellApprovalDecision::AllowPattern(pattern) => push_agent_message(
            state,
            format!(
                "Approved {} risk shell commands matching `{}`{}.",
                pending.risk.as_str(),
                pattern,
                source_context
            ),
        ),
        ShellApprovalDecision::AllowAllRisk => push_agent_message(
            state,
            format!(
                "Approved all future {} risk shell commands this session{}.",
                pending.risk.as_str(),
                source_context
            ),
        ),
        ShellApprovalDecision::Deny(note) => {
            let suffix = note
                .as_deref()
                .filter(|note| !note.is_empty())
                .map(|note| format!(" ({note})"))
                .unwrap_or_default();
            push_error_message(
                state,
                format!(
                    "Denied {} risk shell command{}{}.",
                    pending.risk.as_str(),
                    source_context,
                    suffix
                ),
            );
        }
    }

    Some((pending.request_id, decision, pending.risk))
}

fn sync_pending_shell_approval_ui(state: &mut AppState) {
    let next_request_id = state
        .session
        .pending_shell_approvals
        .front()
        .map(|pending| pending.request_id.as_str());
    let current_request_id = state
        .ui
        .pending_shell_approval
        .as_ref()
        .map(|pending| pending.request_id.as_str());

    if next_request_id == current_request_id {
        return;
    }

    state.ui.pending_shell_approval = state
        .session
        .pending_shell_approvals
        .front()
        .map(crate::app::ui::ShellApprovalUiState::new);
}
