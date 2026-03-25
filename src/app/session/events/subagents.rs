use super::super::{SubagentDisplayState, SubagentStatusKind, TranscriptEntry};
use crate::app::{AppState, ops};
use crate::subagents::{SubagentActivityKind, SubagentUiEvent};

pub(crate) fn on_subagent_event(state: &mut AppState, event: SubagentUiEvent) {
    match event {
        SubagentUiEvent::Spawned {
            id,
            access_mode,
            activity_kind,
        } => {
            let (kind, display_label) = match activity_kind {
                SubagentActivityKind::General => (SubagentStatusKind::Subagent, id.clone()),
                SubagentActivityKind::Planning { model_name } => (
                    SubagentStatusKind::Planning,
                    format!("Planning with {model_name}"),
                ),
            };
            ops::transcript::upsert_subagent_status(
                state,
                id,
                kind,
                display_label,
                SubagentDisplayState::Running,
                format!(
                    "running in {} mode",
                    access_mode.label().to_ascii_lowercase()
                ),
            );
        }
        SubagentUiEvent::Updated {
            id,
            latest_tool_name,
        } => {
            if let Some(latest_tool_name) = latest_tool_name {
                ops::transcript::set_subagent_latest_tool(state, id, latest_tool_name);
            }
        }
        SubagentUiEvent::Completed { id } => {
            let existing = state
                .session
                .entries
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            ops::transcript::upsert_subagent_status(
                state,
                id,
                existing.0,
                existing.1,
                SubagentDisplayState::Completed,
                "completed".into(),
            );
        }
        SubagentUiEvent::Failed {
            id,
            error,
            log_path,
        } => {
            let existing = state
                .session
                .entries
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            ops::transcript::upsert_subagent_status(
                state,
                id.clone(),
                existing.0,
                existing.1,
                SubagentDisplayState::Failed,
                format!("failed: {error}"),
            );
            let suffix = log_path
                .as_deref()
                .map(|path| format!(" Logged request to `{path}`."))
                .unwrap_or_default();
            ops::transcript::push_error_message(
                state,
                format!("Subagent `{id}` failed: {error}{suffix}"),
            );
        }
        SubagentUiEvent::Cancelled { id } => {
            let existing = state
                .session
                .entries
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            ops::transcript::upsert_subagent_status(
                state,
                id,
                existing.0,
                existing.1,
                SubagentDisplayState::Cancelled,
                "cancelled".into(),
            );
        }
        SubagentUiEvent::WriteApprovalRequested {
            id,
            request_id,
            tool_name,
            arguments,
        } => {
            ops::approvals::begin_subagent_write_approval(
                state, id, request_id, tool_name, arguments,
            );
        }
        SubagentUiEvent::ShellApprovalRequested {
            id,
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        } => {
            ops::approvals::begin_subagent_shell_approval(
                state,
                id,
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
            );
        }
    }
}
