use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use anyhow::Result;
use futures_util::StreamExt;
use rig::{
    agent::{MultiTurnStreamItem, StreamingError},
    completion::{CompletionModel, Message as RigMessage, PromptError},
    streaming::{
        StreamedAssistantContent, StreamedUserContent, StreamingChat, ToolCallDeltaContent,
    },
    tool::Tool,
};

use crate::{
    app::{PendingReplyReplaySeed, TurnEndReason},
    debug_log::log_debug,
    stats::StatsHook,
    todo::parse_snapshot,
    tool_result_status::tool_result_is_failure_text,
    tools::{AskUserTool, CommentaryTool, TodoTool},
};

use super::{
    CompletionCapture, EventCallback, LlmService, PromptRunResult, StreamEvent,
    compaction::format_tool_result,
    history_from_rig,
    hooks::{
        AskUserHook, CombinedHook, CompletionCaptureHook, STEP_BOUNDARY_REASON, ShellApprovalHook,
        StepBoundaryCapture, StepBoundaryHook, StepBoundaryState, WriteApprovalHook,
    },
    resume::{ReplayProbe, ResumeOverrideController, reconcile_stream_text},
};

const NORMAL_STREAM_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(60);
const POST_TOOL_RESULT_PROGRESS_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Default)]
pub(crate) struct PartialToolCall {
    name: Option<String>,
    arguments: String,
}

#[cfg(test)]
impl PartialToolCall {
    pub(crate) fn new(name: Option<String>, arguments: String) -> Self {
        Self { name, arguments }
    }
}

pub(crate) enum PromptStepOutcome {
    Finished(PromptRunResult),
    Continue(StepBoundaryState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamWaitState {
    Normal,
    AwaitingToolResult,
    AwaitingPostToolProgress,
    AwaitingPostToolFailureProgress,
}

pub(crate) async fn run_prompt_step<M>(
    service: &LlmService,
    agent: &rig::agent::Agent<M>,
    reply_id: u64,
    prompt: RigMessage,
    history: Vec<RigMessage>,
    stats_hook: StatsHook,
    capture: Option<CompletionCapture>,
    emit: EventCallback,
    resume: Option<ResumeOverrideController>,
    replay_seed: Option<PendingReplyReplaySeed>,
    max_tool_steps: usize,
) -> Result<PromptStepOutcome>
where
    M: CompletionModel + 'static,
{
    let write_approval_hook = WriteApprovalHook {
        reply_id,
        emit: emit.clone(),
        approvals: service.approvals.clone(),
        request_id_prefix: service.interaction_scope().to_string(),
        capture: capture.clone(),
        resume: resume.clone(),
    };
    let shell_approval_hook = ShellApprovalHook {
        reply_id,
        emit: emit.clone(),
        access_mode: service.access_mode,
        approvals: service.shell_approvals.clone(),
        request_id_prefix: service.interaction_scope().to_string(),
        safety: service.safety.clone(),
        capture: capture.clone(),
        resume: resume.clone(),
    };
    let ask_user_hook = AskUserHook {
        reply_id,
        emit: emit.clone(),
        controller: service.ask_user.clone(),
        capture: capture.clone(),
        resume: resume.clone(),
    };
    let step_boundary = StepBoundaryCapture::default();
    let hook = CombinedHook {
        first: StepBoundaryHook {
            capture: step_boundary.clone(),
        },
        second: CombinedHook {
            first: stats_hook.clone(),
            second: CombinedHook {
                first: CompletionCaptureHook { capture },
                second: CombinedHook {
                    first: shell_approval_hook,
                    second: CombinedHook {
                        first: write_approval_hook,
                        second: ask_user_hook,
                    },
                },
            },
        },
    };
    let mut stream = agent
        .stream_chat(prompt, history)
        .with_hook(hook)
        .multi_turn(max_tool_steps)
        .await;
    let mut tool_calls = HashMap::<String, String>::new();
    let mut commentary_calls = HashSet::<String>::new();
    let mut partial_tool_calls = HashMap::<String, PartialToolCall>::new();
    let mut output = String::new();
    let mut reasoning_output = String::new();
    let mut plain_replay_probe = replay_seed
        .as_ref()
        .map(|seed| seed.plain_text.clone())
        .filter(|text| !text.is_empty())
        .map(|text| ReplayProbe::new(&text));
    let mut reasoning_replay_probe = replay_seed
        .as_ref()
        .map(|seed| seed.reasoning_text.clone())
        .filter(|text| !text.is_empty())
        .map(|text| ReplayProbe::new(&text));
    let mut commentary_replay_messages = replay_seed
        .map(|seed| seed.commentary_messages.into())
        .filter(|messages: &VecDeque<String>| !messages.is_empty());
    let mut wait_state = StreamWaitState::Normal;

    while let Some(chunk) = next_stream_item_with_timeout(&mut stream, wait_state, None).await? {
        wait_state = StreamWaitState::Normal;
        let event = match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                stats_hook.record_response_progress();
                let delta = reconcile_stream_text(&text.text, &mut plain_replay_probe);
                if delta.is_empty() {
                    None
                } else {
                    output.push_str(&delta);
                    Some(StreamEvent::TextDelta(delta))
                }
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Reasoning(
                reasoning,
            ))) => {
                stats_hook.record_response_progress();
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                let delta = reconcile_completed_reasoning_text(
                    &reasoning.display_text(),
                    &reasoning_output,
                    &mut reasoning_replay_probe,
                );
                if delta.is_empty() {
                    None
                } else {
                    stats_hook.record_reasoning_progress(&delta);
                    reasoning_output.push_str(&delta);
                    Some(StreamEvent::ReasoningDelta(delta))
                }
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            )) => {
                stats_hook.record_response_progress();
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                let delta = reconcile_stream_text(&reasoning, &mut reasoning_replay_probe);
                if delta.is_empty() {
                    None
                } else {
                    stats_hook.record_reasoning_progress(&delta);
                    reasoning_output.push_str(&delta);
                    Some(StreamEvent::ReasoningDelta(delta))
                }
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ToolCallDelta {
                    internal_call_id,
                    content,
                    ..
                },
            )) => {
                stats_hook.record_response_progress();
                let partial = partial_tool_calls.entry(internal_call_id).or_default();
                match content {
                    ToolCallDeltaContent::Name(name) => {
                        partial.name = Some(name);
                    }
                    ToolCallDeltaContent::Delta(delta) => {
                        partial.arguments.push_str(&delta);
                    }
                }
                None
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            })) => {
                stats_hook.record_response_progress();
                wait_state = StreamWaitState::AwaitingToolResult;
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                reasoning_replay_probe =
                    (!reasoning_output.is_empty()).then(|| ReplayProbe::new(&reasoning_output));
                let name = tool_call.function.name.clone();
                log_debug(
                    "llm_stream",
                    format!("tool_call reply_id={reply_id} name={name}"),
                );
                let fallback_arguments = format_tool_arguments(&tool_call.function.arguments);
                tool_calls.insert(internal_call_id.clone(), name.clone());
                if name == AskUserTool::NAME {
                    partial_tool_calls.remove(&internal_call_id);
                    None
                } else if name == TodoTool::NAME {
                    partial_tool_calls.remove(&internal_call_id);
                    None
                } else if name == CommentaryTool::NAME {
                    match resolve_commentary_message(
                        &mut partial_tool_calls,
                        &internal_call_id,
                        &fallback_arguments,
                    ) {
                        Ok(message) => {
                            commentary_calls.insert(internal_call_id);
                            if commentary_replay_messages.as_ref().is_some_and(|messages| {
                                messages
                                    .front()
                                    .is_some_and(|expected| expected == &message)
                            }) {
                                if let Some(messages) = commentary_replay_messages.as_mut() {
                                    messages.pop_front();
                                    if messages.is_empty() {
                                        commentary_replay_messages = None;
                                    }
                                }
                                None
                            } else {
                                commentary_replay_messages = None;
                                Some(StreamEvent::Commentary(message))
                            }
                        }
                        Err(_) => Some(StreamEvent::ToolCall {
                            name,
                            arguments: fallback_arguments,
                        }),
                    }
                } else if resume.as_ref().is_some_and(|resume| {
                    resume.suppress_matching_tool_call(&name, &fallback_arguments)
                }) {
                    partial_tool_calls.remove(&internal_call_id);
                    None
                } else {
                    partial_tool_calls.remove(&internal_call_id);
                    Some(StreamEvent::ToolCall {
                        name,
                        arguments: fallback_arguments,
                    })
                }
            }
            Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                tool_result,
                internal_call_id,
            })) => {
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                reasoning_replay_probe =
                    (!reasoning_output.is_empty()).then(|| ReplayProbe::new(&reasoning_output));
                let name = tool_calls
                    .get(&internal_call_id)
                    .cloned()
                    .unwrap_or_else(|| tool_result.id.clone());
                log_debug(
                    "llm_stream",
                    format!("tool_result reply_id={reply_id} name={name}"),
                );
                let output = format_tool_result(&tool_result);
                wait_state = if tool_result_is_failure(&output) {
                    StreamWaitState::AwaitingPostToolFailureProgress
                } else {
                    StreamWaitState::AwaitingPostToolProgress
                };
                if name == AskUserTool::NAME {
                    None
                } else if name == TodoTool::NAME {
                    match parse_snapshot(&output) {
                        Ok(snapshot) => Some(StreamEvent::TodoSnapshot(snapshot)),
                        Err(_) => Some(StreamEvent::ToolResult { name, output }),
                    }
                } else if commentary_calls.contains(&internal_call_id) {
                    None
                } else {
                    Some(StreamEvent::ToolResult { name, output })
                }
            }
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                stats_hook.record_response_progress();
                log_debug("llm_stream", format!("final_response reply_id={reply_id}"));
                let history = response.history().map(ToOwned::to_owned);
                let history = history.map(history_from_rig).transpose()?;
                let event = StreamEvent::TurnEnded {
                    reason: TurnEndReason::Completed,
                    history,
                };
                if !(emit)(reply_id, event) {
                    return Err(anyhow::anyhow!("event sink unavailable"));
                }
                return Ok(PromptStepOutcome::Finished(PromptRunResult { output }));
            }
            Ok(_) => None,
            Err(error) => {
                log_debug(
                    "llm_stream",
                    format!("stream_error reply_id={reply_id} error={error}"),
                );
                if let Some(boundary) = step_boundary.take()
                    && is_step_boundary_error(&error)
                {
                    stats_hook.finish_request_without_usage();
                    return Ok(PromptStepOutcome::Continue(boundary));
                }
                let message = error.to_string();
                stats_hook.fail_request();
                let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
                return Err(error.into());
            }
        };

        if let Some(event) = event
            && !(emit)(reply_id, event)
        {
            return Err(anyhow::anyhow!("event sink unavailable"));
        }
    }

    let message = "Request ended before response completed.".to_string();
    stats_hook.fail_request();
    let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
    Err(anyhow::anyhow!(message))
}

pub(crate) fn format_tool_arguments(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments).unwrap_or_else(|_| arguments.to_string())
}

pub(crate) fn reconcile_completed_reasoning_text(
    incoming: &str,
    existing_reasoning_output: &str,
    replay_probe: &mut Option<ReplayProbe>,
) -> String {
    if replay_probe.is_none() && !existing_reasoning_output.is_empty() {
        *replay_probe = Some(ReplayProbe::new(existing_reasoning_output));
    }
    reconcile_stream_text(incoming, replay_probe)
}

async fn next_stream_item_with_timeout<S, T>(
    stream: &mut S,
    wait_state: StreamWaitState,
    timeout_override: Option<Duration>,
) -> Result<Option<T>>
where
    S: futures_util::Stream<Item = T> + Unpin,
{
    let timeout = timeout_override.or(match wait_state {
        StreamWaitState::Normal => Some(NORMAL_STREAM_INACTIVITY_TIMEOUT),
        // Tool execution and human approval should be controlled by the tool/hook path itself.
        // Do not apply an outer provider inactivity timeout while waiting for a tool result.
        StreamWaitState::AwaitingToolResult => None,
        StreamWaitState::AwaitingPostToolProgress => Some(POST_TOOL_RESULT_PROGRESS_TIMEOUT),
        // If a tool failed, keep waiting for the model to adapt instead of converting the tool
        // failure into a turn-level timeout.
        StreamWaitState::AwaitingPostToolFailureProgress => None,
    });

    if let Some(timeout) = timeout {
        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(item) => Ok(item),
            Err(_) => {
                let (log_label, message) = match wait_state {
                    StreamWaitState::Normal => (
                        "stream_inactivity_timeout",
                        "Request stalled without provider output before the turn completed.",
                    ),
                    StreamWaitState::AwaitingToolResult => {
                        unreachable!("awaiting tool result should not be subject to stream timeout")
                    }
                    StreamWaitState::AwaitingPostToolProgress => (
                        "post_tool_progress_timeout",
                        "Request stalled after tool execution without further model output.",
                    ),
                    StreamWaitState::AwaitingPostToolFailureProgress => unreachable!(
                        "tool failure follow-up should not be subject to stream timeout"
                    ),
                };
                log_debug("llm_stream", log_label);
                Err(anyhow::anyhow!(message))
            }
        }
    } else {
        Ok(stream.next().await)
    }
}

fn is_step_boundary_error(error: &StreamingError) -> bool {
    matches!(
        error,
        StreamingError::Prompt(prompt_error)
            if matches!(prompt_error.as_ref(), PromptError::PromptCancelled { reason, .. } if reason == STEP_BOUNDARY_REASON)
    )
}

fn tool_result_is_failure(output: &str) -> bool {
    tool_result_is_failure_text(output)
}

#[cfg(test)]
mod tests {
    use futures_util::stream;

    use super::{StreamWaitState, next_stream_item_with_timeout, tool_result_is_failure};

    #[tokio::test]
    async fn post_tool_timeout_returns_next_item_when_progress_resumes() {
        let mut stream = stream::iter([1]);

        let item = next_stream_item_with_timeout(
            &mut stream,
            StreamWaitState::AwaitingPostToolProgress,
            Some(std::time::Duration::from_millis(10)),
        )
        .await
        .expect("stream item");

        assert_eq!(item, Some(1));
    }

    #[tokio::test]
    async fn normal_timeout_fails_when_stream_stops_without_closing() {
        let mut stream = stream::pending::<i32>();

        let error = next_stream_item_with_timeout(
            &mut stream,
            StreamWaitState::Normal,
            Some(std::time::Duration::from_millis(10)),
        )
        .await
        .expect_err("timeout error");

        assert!(
            error
                .to_string()
                .contains("stalled without provider output")
        );
    }

    #[tokio::test]
    async fn post_tool_timeout_fails_when_no_follow_up_arrives() {
        let mut stream = stream::pending::<i32>();

        let error = next_stream_item_with_timeout(
            &mut stream,
            StreamWaitState::AwaitingPostToolProgress,
            Some(std::time::Duration::from_millis(10)),
        )
        .await
        .expect_err("timeout error");

        assert!(
            error
                .to_string()
                .contains("Request stalled after tool execution")
        );
    }

    #[tokio::test]
    async fn tool_wait_without_override_waits_for_result_without_timeout() {
        let mut stream = stream::iter([1]);

        let item =
            next_stream_item_with_timeout(&mut stream, StreamWaitState::AwaitingToolResult, None)
                .await
                .expect("stream item");

        assert_eq!(item, Some(1));
    }

    #[tokio::test]
    async fn post_tool_failure_wait_without_override_waits_for_result_without_timeout() {
        let mut stream = stream::iter([1]);

        let item = next_stream_item_with_timeout(
            &mut stream,
            StreamWaitState::AwaitingPostToolFailureProgress,
            None,
        )
        .await
        .expect("stream item");

        assert_eq!(item, Some(1));
    }

    #[test]
    fn tool_result_failure_detection_tracks_tool_errors() {
        assert!(tool_result_is_failure(
            "ToolCallError: Shell command timed out after 10ms."
        ));
        assert!(tool_result_is_failure(
            "Toolset error: ToolCallError: ToolCallError: patch 1 old_text was not found"
        ));
        assert!(!tool_result_is_failure("Exit code: 0"));
    }
}

pub(crate) fn parse_commentary_message(args: &str) -> Result<String> {
    serde_json::from_str::<crate::tools::CommentaryArgs>(args)?
        .validated_message()
        .map_err(Into::into)
}

pub(crate) fn resolve_commentary_message(
    partial_tool_calls: &mut HashMap<String, PartialToolCall>,
    internal_call_id: &str,
    fallback_arguments: &str,
) -> Result<String> {
    let mut candidates = Vec::new();
    if let Some(partial) = partial_tool_calls.remove(internal_call_id)
        && !partial.arguments.trim().is_empty()
    {
        candidates.push(partial.arguments);
    }
    candidates.push(fallback_arguments.to_string());

    let mut best_message = None;
    let mut last_error = None;
    for candidate in candidates {
        match parse_commentary_message(&candidate) {
            Ok(message) => {
                if best_message.as_ref().is_none_or(|current: &String| {
                    message.chars().count() > current.chars().count()
                }) {
                    best_message = Some(message);
                }
            }
            Err(error) => last_error = Some(error),
        }
    }

    if let Some(message) = best_message {
        Ok(message)
    } else if let Some(error) = last_error {
        Err(error)
    } else {
        parse_commentary_message(fallback_arguments)
    }
}
