use super::{
    Effect, MessageStyle, PendingReply, PendingReplyKind, SubagentDisplayState, SubagentStatusKind,
    TranscriptEntry, WriteApprovalDecision,
};
use crate::app::AppShell as App;
use crate::features::planning::{
    PlanningStage, contains_proposed_plan, extract_planning_ready_brief,
};
use crate::subagents::{SubagentActivityKind, SubagentUiEvent};

use super::StreamEvent;

pub(super) fn on_stream_event(app: &mut App, reply_id: u64, event: StreamEvent) -> Option<Effect> {
    if app.active_reply_id() != Some(reply_id) {
        return None;
    }

    match event {
        StreamEvent::TextDelta(delta) => {
            app.append_pending_stream_message(&delta, MessageStyle::Plain);
            None
        }
        StreamEvent::Commentary(message) => {
            app.push_agent_commentary(message);
            None
        }
        StreamEvent::ReasoningDelta(delta) => {
            if app.show_thinking() {
                app.append_pending_stream_message(&delta, MessageStyle::Thinking);
            }
            None
        }
        StreamEvent::ToolCall { name, arguments } => {
            app.push_tool_call(name, arguments);
            None
        }
        StreamEvent::ToolResult { name, output } => {
            app.push_tool_result(name, output);
            None
        }
        StreamEvent::AskUserRequested {
            request_id,
            request,
        } => {
            app.begin_ask_user(request_id, request);
            None
        }
        StreamEvent::WriteApprovalRequested {
            request_id,
            tool_name,
            arguments,
        } => {
            app.begin_write_approval(request_id, tool_name, arguments);
            None
        }
        StreamEvent::ShellApprovalRequested {
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        } => {
            app.begin_shell_approval(
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
            );
            None
        }
        StreamEvent::PlanningFinalizationStarted => {
            app.begin_planning_finalization();
            None
        }
        StreamEvent::CompactionFinished {
            history,
            model_name,
        } => {
            app.replace_session_history(history);
            app.set_last_history_model_name(Some(model_name));
            app.clear_pending_ask_user();
            app.session.pending_reply = None;
            app.push_agent_message("Context compacted.");
            None
        }
        StreamEvent::Finished { history } => {
            let pending_kind = app
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind);
            let planning_stage = app.planning_session_stage();
            let final_text = app
                .session
                .pending_reply
                .as_ref()
                .map(|pending| pending.plain_text.clone())
                .unwrap_or_default();
            if let Some(history) = history {
                app.replace_session_history(history);
                app.set_last_history_model_name(Some(app.model_name().to_string()));
            }
            app.clear_pending_ask_user();
            app.session.pending_reply = None;
            if planning_stage == Some(PlanningStage::Conversation)
                && let Some(description) = extract_planning_ready_brief(&final_text)
            {
                app.begin_planning_fanout();
                let reply_id = app.session.next_reply_id();
                app.session.pending_reply =
                    Some(PendingReply::new(reply_id, PendingReplyKind::Planning));
                return Some(Effect::RunPlanningWorkflow {
                    reply_id,
                    description,
                    history: app.session_history().to_vec(),
                    history_model_name: app.last_history_model_name().map(str::to_string),
                });
            }
            if pending_kind == Some(PendingReplyKind::Planning)
                && contains_proposed_plan(&final_text)
            {
                app.begin_plan_review();
            }
            None
        }
        StreamEvent::Failed(error) => {
            if app.planning_session_stage() == Some(PlanningStage::RunningFanout) {
                app.begin_planning_conversation();
            }
            app.clear_pending_ask_user();
            app.session.pending_reply = None;
            app.push_agent_error(format!("Request failed: {error}"));
            None
        }
    }
}

pub(super) fn on_subagent_event(app: &mut App, event: SubagentUiEvent) {
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
            app.upsert_subagent_status(
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
                app.set_subagent_latest_tool(id, latest_tool_name);
            }
        }
        SubagentUiEvent::Completed { id } => {
            let existing = app
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            app.upsert_subagent_status(
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
            let existing = app
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            app.upsert_subagent_status(
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
            app.push_error_message(format!("Subagent `{id}` failed: {error}{suffix}"));
        }
        SubagentUiEvent::Cancelled { id } => {
            let existing = app
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    TranscriptEntry::SubagentStatus(status) if status.id == id => {
                        Some((status.kind, status.display_label.clone()))
                    }
                    _ => None,
                })
                .unwrap_or((SubagentStatusKind::Subagent, id.clone()));
            app.upsert_subagent_status(
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
            app.begin_subagent_write_approval(id, request_id, tool_name, arguments);
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
            app.begin_subagent_shell_approval(
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

pub(super) fn apply_write_approval(
    app: &mut App,
    decision: WriteApprovalDecision,
) -> Option<String> {
    app.resolve_write_approval(decision)
        .map(|pending| pending.request_id)
}

pub(super) fn resolve_write_approval(
    request_id: Option<String>,
    decision: WriteApprovalDecision,
) -> Option<Effect> {
    request_id.map(|request_id| Effect::ResolveWriteApproval {
        request_id,
        decision,
    })
}
