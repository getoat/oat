use super::super::{ActivityDisplayState, SubagentStatusKind, TranscriptEntry};
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
                ActivityDisplayState::Running,
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
                ActivityDisplayState::Completed,
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
                ActivityDisplayState::Failed,
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
                ActivityDisplayState::Cancelled,
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

#[cfg(test)]
mod tests {
    use crate::{
        app::{
            Action, ActivityDisplayState, MessageStyle, TranscriptEntry,
            session::test_support::new_app,
        },
        subagents::{SubagentActivityKind, SubagentUiEvent},
    };

    #[test]
    fn subagent_failure_message_includes_log_path_when_available() {
        let mut app = new_app(true);

        app.apply(Action::SubagentEvent(SubagentUiEvent::Failed {
            id: "subagent-1".into(),
            error: "boom".into(),
            log_path: Some("/tmp/subagent-1.json".into()),
        }));

        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry") else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(
            message
                .text
                .contains("Logged request to `/tmp/subagent-1.json`.")
        );
    }

    #[test]
    fn subagent_update_tracks_latest_tool_name() {
        let mut app = new_app(true);
        app.apply(Action::SubagentEvent(SubagentUiEvent::Spawned {
            id: "subagent-1".into(),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: SubagentActivityKind::General,
        }));

        app.apply(Action::SubagentEvent(SubagentUiEvent::Updated {
            id: "subagent-1".into(),
            latest_tool_name: Some("Grep".into()),
        }));

        let TranscriptEntry::SubagentStatus(status) = app.entries().last().expect("status entry")
        else {
            panic!("expected subagent status entry");
        };
        assert_eq!(status.latest_tool_name.as_deref(), Some("Grep"));
    }

    #[test]
    fn cancelled_subagent_event_updates_status_without_error_message() {
        let mut app = new_app(true);
        app.apply(Action::SubagentEvent(SubagentUiEvent::Spawned {
            id: "subagent-1".into(),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: SubagentActivityKind::General,
        }));

        let entry_count_before = app.entries().len();
        app.apply(Action::SubagentEvent(SubagentUiEvent::Cancelled {
            id: "subagent-1".into(),
        }));

        assert_eq!(app.entries().len(), entry_count_before);
        let TranscriptEntry::SubagentStatus(status) = app.entries().last().expect("status entry")
        else {
            panic!("expected subagent status entry");
        };
        assert_eq!(status.state, ActivityDisplayState::Cancelled);
        assert_eq!(status.status_text, "cancelled");
    }
}
