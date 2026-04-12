use anyhow::Result;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use rig::completion::{
    Message as RigMessage,
    message::{AssistantContent, UserContent},
};

pub use crate::app::StreamEvent;

use crate::{
    app::{CommandRisk, SessionHistoryMessage, ShellApprovalDecision, WriteApprovalDecision},
    ask_user::{AskUserRequest, AskUserResponse},
    completion_request::{CompletionRequestSnapshot, estimated_message_tokens},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOverride {
    WriteApproval {
        tool_name: String,
        arguments: String,
        decision: WriteApprovalDecision,
    },
    ShellApproval {
        risk: CommandRisk,
        command: String,
        working_directory: String,
        decision: ShellApprovalDecision,
    },
    AskUser {
        request: AskUserRequest,
        response: AskUserResponse,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeRequest {
    pub snapshot: CompletionRequestSnapshot,
    pub override_action: ResumeOverride,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InteractionResolveResult {
    Resolved,
    Resume(ResumeRequest),
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnInterruptRequest {
    AtStepBoundary,
}

#[derive(Clone, Default)]
pub(crate) struct TurnInterruptController {
    inner: Arc<Mutex<Option<TurnInterruptRequest>>>,
}

impl TurnInterruptController {
    pub(crate) fn request(&self, request: TurnInterruptRequest) {
        *self.inner.lock().expect("turn interrupt request lock") = Some(request);
    }

    pub(crate) fn clear(&self) {
        *self.inner.lock().expect("turn interrupt request lock") = None;
    }

    pub(crate) fn take(&self) -> Option<TurnInterruptRequest> {
        self.inner
            .lock()
            .expect("turn interrupt request lock")
            .take()
    }
}

pub type EventCallback = Arc<dyn Fn(u64, StreamEvent) -> bool + Send + Sync>;

pub struct PromptRunResult {
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct HistoryCompactionResult {
    pub history: Vec<SessionHistoryMessage>,
    pub model_name: String,
}

#[derive(Clone, Default)]
pub struct CompletionCapture {
    inner: Arc<Mutex<Option<CompletionRequestSnapshot>>>,
}

impl CompletionCapture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Option<CompletionRequestSnapshot> {
        self.inner.lock().expect("completion capture lock").clone()
    }

    pub(crate) fn record(&self, prompt: &RigMessage, history: &[RigMessage]) {
        let mut snapshot = self.inner.lock().expect("completion capture lock");
        *snapshot = Some(CompletionRequestSnapshot::capture(prompt, history));
    }
}

impl SessionHistoryMessage {
    pub(crate) fn from_rig_message(message: RigMessage) -> Result<Self> {
        Ok(Self {
            estimated_tokens: estimated_message_tokens(&message),
            payload: serde_json::to_value(message)?,
        })
    }

    pub(crate) fn into_rig_message(self) -> Result<RigMessage> {
        Ok(serde_json::from_value(self.payload)?)
    }
}

pub(crate) fn history_from_rig(history: Vec<RigMessage>) -> Result<Vec<SessionHistoryMessage>> {
    sanitize_rig_history(history)
        .into_iter()
        .map(SessionHistoryMessage::from_rig_message)
        .collect()
}

pub(crate) fn history_with_prompt_from_rig(
    mut history: Vec<RigMessage>,
    prompt: RigMessage,
) -> Result<Vec<SessionHistoryMessage>> {
    history.push(prompt);
    history_from_rig(history)
}

pub(crate) fn history_into_rig(history: Vec<SessionHistoryMessage>) -> Result<Vec<RigMessage>> {
    let rig_history = history
        .into_iter()
        .map(SessionHistoryMessage::into_rig_message)
        .collect::<Result<Vec<_>>>()?;
    Ok(sanitize_rig_history(rig_history))
}

pub(crate) fn sanitize_rig_history(history: Vec<RigMessage>) -> Vec<RigMessage> {
    #[derive(Clone)]
    struct PendingToolCall {
        position: (usize, usize),
        id: String,
        call_id: Option<String>,
    }

    let mut pending_calls = HashMap::<(usize, usize), PendingToolCall>::new();
    let mut pending_by_id = HashMap::<String, (usize, usize)>::new();
    let mut pending_by_call_id = HashMap::<String, (usize, usize)>::new();
    let mut matched_positions = HashSet::<(usize, usize)>::new();

    for (message_index, message) in history.iter().enumerate() {
        match message {
            RigMessage::Assistant { content, .. } => {
                for (content_index, part) in content.iter().enumerate() {
                    let AssistantContent::ToolCall(tool_call) = part else {
                        continue;
                    };
                    let position = (message_index, content_index);
                    pending_by_id.insert(tool_call.id.clone(), position);
                    if let Some(call_id) = tool_call.call_id.clone() {
                        pending_by_call_id.insert(call_id.clone(), position);
                    }
                    pending_calls.insert(
                        position,
                        PendingToolCall {
                            position,
                            id: tool_call.id.clone(),
                            call_id: tool_call.call_id.clone(),
                        },
                    );
                }
            }
            RigMessage::User { content } => {
                for (content_index, part) in content.iter().enumerate() {
                    let UserContent::ToolResult(tool_result) = part else {
                        continue;
                    };
                    let matched_position = tool_result
                        .call_id
                        .as_deref()
                        .and_then(|value| pending_by_call_id.remove(value))
                        .or_else(|| pending_by_id.remove(&tool_result.id));

                    let Some(matched_position) = matched_position else {
                        continue;
                    };

                    let Some(pending) = pending_calls.remove(&matched_position) else {
                        continue;
                    };
                    pending_by_id.remove(&pending.id);
                    if let Some(call_id) = pending.call_id {
                        pending_by_call_id.remove(&call_id);
                    }
                    matched_positions.insert(pending.position);
                    matched_positions.insert((message_index, content_index));
                }
            }
            RigMessage::System { .. } => {}
        }
    }

    history
        .into_iter()
        .enumerate()
        .filter_map(|(message_index, message)| {
            sanitize_tool_message(message_index, message, &matched_positions)
        })
        .collect()
}

fn sanitize_tool_message(
    message_index: usize,
    message: RigMessage,
    matched_positions: &HashSet<(usize, usize)>,
) -> Option<RigMessage> {
    match message {
        RigMessage::System { .. } => Some(message),
        RigMessage::Assistant { id, content } => {
            let content = content
                .into_iter()
                .enumerate()
                .filter_map(|(content_index, part)| {
                    let keep = !matches!(part, AssistantContent::ToolCall(_))
                        || matched_positions.contains(&(message_index, content_index));
                    keep.then_some(part)
                })
                .collect::<Vec<_>>();
            rebuild_assistant_message(id, content)
        }
        RigMessage::User { content } => {
            let content = content
                .into_iter()
                .enumerate()
                .filter_map(|(content_index, part)| {
                    let keep = !matches!(part, UserContent::ToolResult(_))
                        || matched_positions.contains(&(message_index, content_index));
                    keep.then_some(part)
                })
                .collect::<Vec<_>>();
            rebuild_user_message(content)
        }
    }
}

fn rebuild_assistant_message(
    id: Option<String>,
    content: Vec<AssistantContent>,
) -> Option<RigMessage> {
    match content.len() {
        0 => None,
        1 => Some(RigMessage::Assistant {
            id,
            content: rig::OneOrMany::one(content.into_iter().next().expect("single item")),
        }),
        _ => Some(RigMessage::Assistant {
            id,
            content: rig::OneOrMany::many(content).expect("multiple assistant content items"),
        }),
    }
}

fn rebuild_user_message(content: Vec<UserContent>) -> Option<RigMessage> {
    match content.len() {
        0 => None,
        1 => Some(RigMessage::User {
            content: rig::OneOrMany::one(content.into_iter().next().expect("single item")),
        }),
        _ => Some(RigMessage::User {
            content: rig::OneOrMany::many(content).expect("multiple user content items"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{
        OneOrMany,
        completion::message::{
            AssistantContent, Text, ToolCall, ToolFunction, ToolResultContent, UserContent,
        },
    };
    use serde_json::json;

    #[test]
    fn history_with_prompt_from_rig_appends_prompt_after_history() {
        let history = vec![
            RigMessage::user("user prompt"),
            RigMessage::assistant("tool call"),
        ];
        let prompt = RigMessage::user("tool result");

        let rebuilt = history_with_prompt_from_rig(history, prompt.clone()).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert_eq!(
            rebuilt_rig,
            vec![
                RigMessage::user("user prompt"),
                RigMessage::assistant("tool call"),
                prompt,
            ]
        );
    }

    #[test]
    fn history_round_trip_drops_orphan_tool_result() {
        let history = vec![
            RigMessage::user("first user"),
            RigMessage::User {
                content: OneOrMany::one(UserContent::tool_result_with_call_id(
                    "tool-1",
                    "call-1".to_string(),
                    OneOrMany::one(ToolResultContent::text("orphaned")),
                )),
            },
            RigMessage::assistant("after orphan"),
        ];

        let rebuilt = history_from_rig(history).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert_eq!(
            rebuilt_rig,
            vec![
                RigMessage::user("first user"),
                RigMessage::assistant("after orphan"),
            ]
        );
    }

    #[test]
    fn history_round_trip_drops_unmatched_tool_call() {
        let history = vec![
            RigMessage::user("first user"),
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                    id: "tool-1".into(),
                    call_id: Some("call-1".into()),
                    function: ToolFunction::new("ReadFile".into(), json!({"path": "src/lib.rs"})),
                    signature: None,
                    additional_params: None,
                })),
            },
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::Text(Text {
                    text: "after unmatched call".into(),
                })),
            },
        ];

        let rebuilt = history_from_rig(history).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert_eq!(
            rebuilt_rig,
            vec![
                RigMessage::user("first user"),
                RigMessage::assistant("after unmatched call"),
            ]
        );
    }

    #[test]
    fn history_round_trip_keeps_complete_tool_pair() {
        let history = vec![
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                    id: "tool-1".into(),
                    call_id: Some("call-1".into()),
                    function: ToolFunction::new("ReadFile".into(), json!({"path": "src/lib.rs"})),
                    signature: None,
                    additional_params: None,
                })),
            },
            RigMessage::User {
                content: OneOrMany::one(UserContent::tool_result_with_call_id(
                    "tool-1",
                    "call-1".to_string(),
                    OneOrMany::one(ToolResultContent::text("1 | hello")),
                )),
            },
        ];

        let rebuilt = history_from_rig(history.clone()).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert_eq!(rebuilt_rig, history);
    }

    #[test]
    fn history_round_trip_drops_unmatched_tool_calls_from_multi_item_message() {
        let history = vec![
            RigMessage::user("first user"),
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::Text(Text {
                        text: "working".into(),
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".into(),
                        call_id: Some("call-1".into()),
                        function: ToolFunction::new(
                            "Commentary".into(),
                            json!({"message":"checking files"}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-2".into(),
                        call_id: Some("call-2".into()),
                        function: ToolFunction::new(
                            "Todo".into(),
                            json!({"operation":"create","tasks":[]}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple assistant content items"),
            },
            RigMessage::assistant("after tool calls"),
        ];

        let rebuilt = history_from_rig(history).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert_eq!(
            rebuilt_rig,
            vec![
                RigMessage::user("first user"),
                RigMessage::assistant("working"),
                RigMessage::assistant("after tool calls"),
            ]
        );
    }

    #[test]
    fn history_round_trip_keeps_only_matched_tool_contents_in_multi_item_messages() {
        let history = vec![
            RigMessage::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".into(),
                        call_id: Some("call-1".into()),
                        function: ToolFunction::new(
                            "ReadFile".into(),
                            json!({"path": "src/lib.rs"}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-2".into(),
                        call_id: Some("call-2".into()),
                        function: ToolFunction::new(
                            "Commentary".into(),
                            json!({"message":"checking"}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple assistant content items"),
            },
            RigMessage::User {
                content: OneOrMany::many(vec![
                    UserContent::tool_result_with_call_id(
                        "tool-1",
                        "call-1".to_string(),
                        OneOrMany::one(ToolResultContent::text("1 | hello")),
                    ),
                    UserContent::Text(Text {
                        text: "plain user context".into(),
                    }),
                ])
                .expect("multiple user content items"),
            },
        ];

        let rebuilt = history_from_rig(history).expect("history");
        let rebuilt_rig = history_into_rig(rebuilt).expect("round trip");

        assert!(matches!(
            &rebuilt_rig[0],
            RigMessage::Assistant { content, .. }
                if content.len() == 1
                    && matches!(
                        content.first_ref(),
                        AssistantContent::ToolCall(tool_call)
                            if tool_call.id == "tool-1"
                                && tool_call.call_id.as_deref() == Some("call-1")
                    )
        ));
        assert!(matches!(
            &rebuilt_rig[1],
            RigMessage::User { content }
                if content.len() == 2
                    && matches!(
                        content.first_ref(),
                        UserContent::ToolResult(tool_result)
                            if tool_result.id == "tool-1"
                                && tool_result.call_id.as_deref() == Some("call-1")
                    )
        ));
    }
}
