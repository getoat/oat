use crate::app::StreamEvent;

use super::{SubagentManager, SubagentUiEvent};

impl SubagentManager {
    pub(super) fn handle_stream_event(&self, id: &str, event: StreamEvent) {
        match event {
            StreamEvent::SessionTitleGenerated(_)
            | StreamEvent::TextDelta(_)
            | StreamEvent::Commentary(_)
            | StreamEvent::ReasoningDelta(_)
            | StreamEvent::ToolResult { .. }
            | StreamEvent::AskUserRequested { .. }
            | StreamEvent::PlanningFinalizationStarted
            | StreamEvent::CompactionFinished { .. }
            | StreamEvent::Finished { .. } => {
                self.mark_activity(id);
            }
            StreamEvent::ToolCall { name, .. } => {
                self.record_tool_activity(id, name);
            }
            StreamEvent::WriteApprovalRequested {
                request_id,
                tool_name,
                arguments,
            } => {
                if self.record_approval_wait(id, request_id.clone(), tool_name.clone()) {
                    let _ = self
                        .inner
                        .ui_tx
                        .send(SubagentUiEvent::WriteApprovalRequested {
                            id: id.to_string(),
                            request_id,
                            tool_name,
                            arguments,
                        });
                }
            }
            StreamEvent::ShellApprovalRequested {
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
            } => {
                if self.record_approval_wait(id, request_id.clone(), "RunShellScript".into()) {
                    let _ = self
                        .inner
                        .ui_tx
                        .send(SubagentUiEvent::ShellApprovalRequested {
                            id: id.to_string(),
                            request_id,
                            risk,
                            risk_explanation,
                            command,
                            working_directory,
                            reason,
                        });
                }
            }
            StreamEvent::Failed(_) => {
                self.mark_activity(id);
            }
        }
    }
}
