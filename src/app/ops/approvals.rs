use crate::app::session::PendingReplyActivity;
use crate::app::{
    AppState, ApprovalMode, CommandRisk, EditorInput, PendingWriteApproval, ShellApprovalDecision,
    ShellApprovalEditMode, WriteApprovalDecision,
};
use crate::tools::{ApprovalPreview, approval_preview};

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
    let ApprovalPreview { summary, target } =
        approval_preview(&tool_name, &arguments, &state.session.workspace_root);
    state
        .session
        .pending_write_approvals
        .push_back(PendingWriteApproval {
            request_id,
            tool_name,
            arguments,
            summary,
            target,
            source_label,
        });
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
    sync_pending_reply_activity_after_approval(state);
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
    sync_pending_reply_activity_after_approval(state);

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

fn sync_pending_reply_activity_after_approval(state: &mut AppState) {
    let activity = if !state.session.pending_write_approvals.is_empty()
        || !state.session.pending_shell_approvals.is_empty()
    {
        PendingReplyActivity::WaitingForApproval
    } else {
        PendingReplyActivity::WaitingForTool
    };
    crate::app::ops::session::set_pending_reply_activity(state, activity);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::session::test_support::new_app;
    use crate::app::{PendingReply, PendingReplyKind};

    #[test]
    fn begin_write_approval_stores_summary_and_target() {
        let mut app = new_app(true);

        begin_write_approval(
            app.state_mut(),
            "call-1".into(),
            "WriteFile".into(),
            r#"{"filename":"notes.txt","content":"hello"}"#.into(),
        );

        let pending = app
            .state()
            .session
            .pending_write_approvals
            .front()
            .expect("pending approval");
        assert_eq!(pending.summary, "No reason provided for creating notes.txt");
        assert_eq!(pending.target.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn begin_subagent_write_approval_preserves_source_label() {
        let mut app = new_app(true);

        begin_subagent_write_approval(
            app.state_mut(),
            "planner-1".into(),
            "call-1".into(),
            "DeletePath".into(),
            r#"{"path":"notes.txt","intent":"remove stale notes"}"#.into(),
        );

        let pending = app
            .state()
            .session
            .pending_write_approvals
            .front()
            .expect("pending approval");
        assert_eq!(pending.summary, "remove stale notes");
        assert_eq!(pending.target.as_deref(), Some("notes.txt"));
        assert_eq!(pending.source_label.as_deref(), Some("planner-1"));
    }

    #[test]
    fn submit_shell_approval_advances_pending_status_after_allow_all() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        crate::app::ops::session::set_pending_reply_activity(
            app.state_mut(),
            PendingReplyActivity::WaitingForApproval,
        );

        begin_shell_approval(
            app.state_mut(),
            "call-1".into(),
            CommandRisk::Medium,
            "writes to workspace".into(),
            "cargo test".into(),
            ".".into(),
            "verify changes".into(),
        );
        app.state_mut()
            .ui
            .pending_shell_approval
            .as_mut()
            .expect("shell approval ui")
            .selected_index = 2;

        let (request_id, decision, risk) =
            submit_shell_approval(app.state_mut()).expect("shell approval submitted");

        assert_eq!(request_id, "call-1");
        assert_eq!(decision, ShellApprovalDecision::AllowAllRisk);
        assert_eq!(risk, CommandRisk::Medium);
        assert!(app.state().session.pending_shell_approvals.is_empty());
        assert_eq!(app.history_pending_status_label(), "Waiting for tool");
    }

    #[test]
    fn submit_shell_approval_keeps_waiting_when_another_request_is_queued() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        crate::app::ops::session::set_pending_reply_activity(
            app.state_mut(),
            PendingReplyActivity::WaitingForApproval,
        );

        begin_shell_approval(
            app.state_mut(),
            "call-1".into(),
            CommandRisk::Medium,
            "writes to workspace".into(),
            "cargo test".into(),
            ".".into(),
            "verify changes".into(),
        );
        begin_shell_approval(
            app.state_mut(),
            "call-2".into(),
            CommandRisk::Medium,
            "writes to workspace".into(),
            "cargo check".into(),
            ".".into(),
            "verify changes".into(),
        );

        let (request_id, decision, risk) =
            submit_shell_approval(app.state_mut()).expect("shell approval submitted");

        assert_eq!(request_id, "call-1");
        assert_eq!(decision, ShellApprovalDecision::AllowOnce);
        assert_eq!(risk, CommandRisk::Medium);
        assert_eq!(app.state().session.pending_shell_approvals.len(), 1);
        assert_eq!(
            app.state()
                .ui
                .pending_shell_approval
                .as_ref()
                .map(|pending| pending.request_id.as_str()),
            Some("call-2")
        );
        assert_eq!(app.history_pending_status_label(), "Waiting for approval");
    }

    #[test]
    fn resolve_write_approval_advances_pending_status_when_queue_is_empty() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        crate::app::ops::session::set_pending_reply_activity(
            app.state_mut(),
            PendingReplyActivity::WaitingForApproval,
        );

        begin_write_approval(
            app.state_mut(),
            "call-1".into(),
            "WriteFile".into(),
            r#"{"filename":"notes.txt","content":"hello"}"#.into(),
        );

        let pending = resolve_write_approval(app.state_mut(), WriteApprovalDecision::AllowOnce)
            .expect("write approval resolved");

        assert_eq!(pending.request_id, "call-1");
        assert!(app.state().session.pending_write_approvals.is_empty());
        assert_eq!(app.history_pending_status_label(), "Waiting for tool");
    }
}
