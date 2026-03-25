use super::super::{Effect, PendingReplyKind, StreamEvent};
use crate::app::{AppState, MessageStyle, ops, query};
use crate::features::planning::{PlanningReply, PlanningStage, parse_planning_reply};

pub(crate) fn on_stream_event(
    state: &mut AppState,
    reply_id: u64,
    event: StreamEvent,
) -> Option<Effect> {
    if ops::session::active_reply_id(state) != Some(reply_id) {
        return None;
    }

    match event {
        StreamEvent::TextDelta(delta) => {
            ops::transcript::append_pending_stream_message(state, &delta, MessageStyle::Plain);
            None
        }
        StreamEvent::Commentary(message) => {
            ops::transcript::push_agent_commentary(state, message);
            None
        }
        StreamEvent::ReasoningDelta(delta) => {
            if state.session.show_thinking {
                ops::transcript::append_pending_stream_message(
                    state,
                    &delta,
                    MessageStyle::Thinking,
                );
            }
            None
        }
        StreamEvent::ToolCall { name, arguments } => {
            ops::transcript::push_tool_call(state, name, arguments);
            None
        }
        StreamEvent::ToolResult { name, output } => {
            ops::transcript::push_tool_result(state, name, output);
            None
        }
        StreamEvent::AskUserRequested {
            request_id,
            request,
        } => {
            ops::ask_user::begin_ask_user(state, request_id, request);
            None
        }
        StreamEvent::WriteApprovalRequested {
            request_id,
            tool_name,
            arguments,
        } => {
            ops::approvals::begin_write_approval(state, request_id, tool_name, arguments);
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
            ops::approvals::begin_shell_approval(
                state,
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
            ops::planning::begin_planning_finalization(state);
            None
        }
        StreamEvent::CompactionFinished {
            history,
            model_name,
        } => {
            ops::session::replace_session_history(state, history);
            ops::session::set_last_history_model_name(state, Some(model_name));
            ops::ask_user::clear_pending_ask_user(state);
            ops::session::clear_pending_reply_only(state);
            ops::transcript::push_agent_message(state, "Context compacted.");
            None
        }
        StreamEvent::Finished { history } => {
            let pending_kind = ops::session::active_reply_kind(state);
            let planning_stage = query::planning_session_stage(state);
            let final_text = ops::session::pending_reply_replay_seed(state)
                .map(|pending| pending.plain_text)
                .unwrap_or_default();
            if let Some(history) = history {
                ops::session::replace_session_history(state, history);
                let model_name = state.session.model_name.clone();
                ops::session::set_last_history_model_name(state, Some(model_name));
            }
            ops::ask_user::clear_pending_ask_user(state);
            let planning_reply = matches!(pending_kind, Some(PendingReplyKind::Planning))
                .then(|| parse_planning_reply(&final_text));
            if planning_stage == Some(PlanningStage::Conversation) {
                if let Some(PlanningReply::ReadyBrief(brief)) = planning_reply.clone() {
                    ops::planning::set_planning_brief(state, brief.markdown.clone());
                    ops::transcript::discard_pending_text_entry(state);
                    ops::session::clear_pending_reply_only(state);
                    ops::planning::begin_planning_fanout(state);
                    let reply_id = ops::session::next_reply_id(state);
                    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);
                    return Some(Effect::RunPlanningWorkflow {
                        reply_id,
                        description: brief.markdown,
                        history: state.session.session_history.to_vec(),
                        history_model_name: state.session.last_history_model_name.clone(),
                    });
                }
            }
            if let Some(PlanningReply::ProposedPlan(plan)) = planning_reply {
                ops::planning::store_proposed_plan(state, plan);
            }
            ops::session::clear_pending_reply_only(state);
            if pending_kind == Some(PendingReplyKind::Planning)
                && matches!(
                    parse_planning_reply(&final_text),
                    PlanningReply::ProposedPlan(_)
                )
            {
                ops::planning::begin_plan_review(state);
            }
            None
        }
        StreamEvent::Failed(error) => {
            if query::planning_session_stage(state) == Some(PlanningStage::RunningFanout) {
                ops::planning::begin_planning_conversation(state);
            }
            ops::ask_user::clear_pending_ask_user(state);
            ops::session::clear_pending_reply_only(state);
            ops::transcript::push_error_message(state, format!("Request failed: {error}"));
            None
        }
    }
}
