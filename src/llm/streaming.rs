use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;
use futures_util::StreamExt;
use rig::{
    agent::{MultiTurnStreamItem, StreamingError},
    completion::{Message as RigMessage, PromptError},
    streaming::{
        StreamedAssistantContent, StreamedUserContent, StreamingChat, ToolCallDeltaContent,
    },
    tool::Tool,
};

use crate::{
    app::PendingReplyReplaySeed,
    stats::StatsHook,
    tools::{AskUserTool, CommentaryTool},
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

pub(crate) async fn run_prompt_step(
    service: &LlmService,
    reply_id: u64,
    prompt: RigMessage,
    history: Vec<RigMessage>,
    stats_hook: StatsHook,
    capture: Option<CompletionCapture>,
    emit: EventCallback,
    resume: Option<ResumeOverrideController>,
    replay_seed: Option<PendingReplyReplaySeed>,
    max_tool_steps: usize,
) -> Result<PromptStepOutcome> {
    let write_approval_hook = WriteApprovalHook {
        reply_id,
        emit: emit.clone(),
        approvals: service.approvals.clone(),
        capture: capture.clone(),
        resume: resume.clone(),
    };
    let shell_approval_hook = ShellApprovalHook {
        reply_id,
        emit: emit.clone(),
        access_mode: service.access_mode,
        approvals: service.shell_approvals.clone(),
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
            first: stats_hook,
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
    let mut stream = service
        .agent
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

    while let Some(chunk) = stream.next().await {
        let event = match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
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
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                let delta =
                    reconcile_stream_text(&reasoning.display_text(), &mut reasoning_replay_probe);
                if delta.is_empty() {
                    None
                } else {
                    reasoning_output.push_str(&delta);
                    Some(StreamEvent::ReasoningDelta(delta))
                }
            }
            Ok(MultiTurnStreamItem::StreamAssistantItem(
                StreamedAssistantContent::ReasoningDelta { reasoning, .. },
            )) => {
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                let delta = reconcile_stream_text(&reasoning, &mut reasoning_replay_probe);
                if delta.is_empty() {
                    None
                } else {
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
                plain_replay_probe = (!output.is_empty()).then(|| ReplayProbe::new(&output));
                reasoning_replay_probe =
                    (!reasoning_output.is_empty()).then(|| ReplayProbe::new(&reasoning_output));
                let name = tool_call.function.name.clone();
                let fallback_arguments = format_tool_arguments(&tool_call.function.arguments);
                tool_calls.insert(internal_call_id.clone(), name.clone());
                if name == AskUserTool::NAME {
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
                if name == AskUserTool::NAME {
                    None
                } else if commentary_calls.contains(&internal_call_id) {
                    None
                } else {
                    Some(StreamEvent::ToolResult {
                        name,
                        output: format_tool_result(&tool_result),
                    })
                }
            }
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                let history = response.history().map(ToOwned::to_owned);
                let history = history.map(history_from_rig).transpose()?;
                let event = StreamEvent::Finished {
                    history: history.clone(),
                };
                if !(emit)(reply_id, event) {
                    return Err(anyhow::anyhow!("event sink unavailable"));
                }
                return Ok(PromptStepOutcome::Finished(PromptRunResult { output }));
            }
            Ok(_) => None,
            Err(error) => {
                if let Some(boundary) = step_boundary.take()
                    && is_step_boundary_error(&error)
                {
                    return Ok(PromptStepOutcome::Continue(boundary));
                }
                let message = error.to_string();
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
    let _ = (emit)(reply_id, StreamEvent::Failed(message.clone()));
    Err(anyhow::anyhow!(message))
}

pub(crate) fn format_tool_arguments(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments).unwrap_or_else(|_| arguments.to_string())
}

fn is_step_boundary_error(error: &StreamingError) -> bool {
    matches!(
        error,
        StreamingError::Prompt(prompt_error)
            if matches!(prompt_error.as_ref(), PromptError::PromptCancelled { reason, .. } if reason == STEP_BOUNDARY_REASON)
    )
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
