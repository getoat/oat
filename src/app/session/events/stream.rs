use super::super::{Effect, PendingReplyKind, StreamEvent};
use crate::app::{MessageStyle, ReducerContext};
use crate::features::planning::{
    PlanningStage, contains_proposed_plan, extract_planning_ready_brief,
};

pub(in crate::app::session) fn on_stream_event(
    ctx: &mut ReducerContext<'_>,
    reply_id: u64,
    event: StreamEvent,
) -> Option<Effect> {
    if ctx.active_reply_id() != Some(reply_id) {
        return None;
    }

    match event {
        StreamEvent::TextDelta(delta) => {
            ctx.append_pending_stream_message(&delta, MessageStyle::Plain);
            None
        }
        StreamEvent::Commentary(message) => {
            ctx.push_agent_commentary(message);
            None
        }
        StreamEvent::ReasoningDelta(delta) => {
            if ctx.show_thinking() {
                ctx.append_pending_stream_message(&delta, MessageStyle::Thinking);
            }
            None
        }
        StreamEvent::ToolCall { name, arguments } => {
            ctx.push_tool_call(name, arguments);
            None
        }
        StreamEvent::ToolResult { name, output } => {
            ctx.push_tool_result(name, output);
            None
        }
        StreamEvent::AskUserRequested {
            request_id,
            request,
        } => {
            ctx.begin_ask_user(request_id, request);
            None
        }
        StreamEvent::WriteApprovalRequested {
            request_id,
            tool_name,
            arguments,
        } => {
            ctx.begin_write_approval(request_id, tool_name, arguments);
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
            ctx.begin_shell_approval(
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
            ctx.begin_planning_finalization();
            None
        }
        StreamEvent::CompactionFinished {
            history,
            model_name,
        } => {
            ctx.replace_session_history(history);
            ctx.set_last_history_model_name(Some(model_name));
            ctx.clear_pending_ask_user();
            ctx.clear_pending_reply_only();
            ctx.push_agent_message("Context compacted.");
            None
        }
        StreamEvent::Finished { history } => {
            let pending_kind = ctx.active_reply_kind();
            let planning_stage = ctx.planning_session_stage();
            let final_text = ctx
                .pending_reply_replay_seed()
                .map(|pending| pending.plain_text)
                .unwrap_or_default();
            if let Some(history) = history {
                ctx.replace_session_history(history);
                ctx.set_last_history_model_name(Some(ctx.model_name().to_string()));
            }
            ctx.clear_pending_ask_user();
            ctx.clear_pending_reply_only();
            if planning_stage == Some(PlanningStage::Conversation)
                && let Some(description) = extract_planning_ready_brief(&final_text)
            {
                ctx.begin_planning_fanout();
                let reply_id = ctx.next_reply_id();
                ctx.set_pending_reply(reply_id, PendingReplyKind::Planning);
                return Some(Effect::RunPlanningWorkflow {
                    reply_id,
                    description,
                    history: ctx.session_history().to_vec(),
                    history_model_name: ctx.last_history_model_name().map(str::to_string),
                });
            }
            if pending_kind == Some(PendingReplyKind::Planning)
                && contains_proposed_plan(&final_text)
            {
                ctx.begin_plan_review();
            }
            None
        }
        StreamEvent::Failed(error) => {
            if ctx.planning_session_stage() == Some(PlanningStage::RunningFanout) {
                ctx.begin_planning_conversation();
            }
            ctx.clear_pending_ask_user();
            ctx.clear_pending_reply_only();
            ctx.push_agent_error(format!("Request failed: {error}"));
            None
        }
    }
}
