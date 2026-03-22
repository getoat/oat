use std::time::{SystemTime, UNIX_EPOCH};

use rig::completion::{
    Message as RigMessage,
    message::{
        AssistantContent, DocumentSourceKind, ReasoningContent, ToolResultContent, UserContent,
    },
};
use serde::Serialize;

use crate::token_counting::{count_json_tokens, count_text_tokens, estimate_binary_tokens};

const ESTIMATED_MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const ESTIMATED_CONTENT_OVERHEAD_TOKENS: u64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionRequestMessageSource {
    History,
    Prompt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CompletionRequestMessageTokens {
    pub sequence: usize,
    pub source: CompletionRequestMessageSource,
    pub estimated_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CompletionRequestSnapshot {
    pub captured_at_unix_ms: u64,
    pub message_count: usize,
    pub estimated_prompt_tokens: u64,
    pub estimated_history_tokens: u64,
    pub estimated_total_tokens: u64,
    pub message_token_estimates: Vec<CompletionRequestMessageTokens>,
    pub prompt: RigMessage,
    pub history: Vec<RigMessage>,
}

impl CompletionRequestSnapshot {
    pub fn capture(prompt: &RigMessage, history: &[RigMessage]) -> Self {
        let estimated_prompt_tokens = estimated_message_tokens(prompt);
        let estimated_history_tokens = estimated_history_context_tokens(history);
        let mut message_token_estimates = history
            .iter()
            .enumerate()
            .map(|(index, message)| CompletionRequestMessageTokens {
                sequence: index,
                source: CompletionRequestMessageSource::History,
                estimated_tokens: estimated_message_tokens(message),
            })
            .collect::<Vec<_>>();
        message_token_estimates.push(CompletionRequestMessageTokens {
            sequence: history.len(),
            source: CompletionRequestMessageSource::Prompt,
            estimated_tokens: estimated_prompt_tokens,
        });

        Self {
            captured_at_unix_ms: unix_timestamp_ms(),
            message_count: history.len() + 1,
            estimated_prompt_tokens,
            estimated_history_tokens,
            estimated_total_tokens: estimated_history_tokens + estimated_prompt_tokens,
            message_token_estimates,
            prompt: prompt.clone(),
            history: history.to_vec(),
        }
    }
}

pub fn estimated_history_context_tokens(history: &[RigMessage]) -> u64 {
    history.iter().map(estimated_message_tokens).sum()
}

pub fn estimated_message_tokens(message: &RigMessage) -> u64 {
    match message {
        RigMessage::System { content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS + estimated_text_tokens(content)
        }
        RigMessage::User { content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS
                + content
                    .iter()
                    .map(estimated_user_content_tokens)
                    .sum::<u64>()
        }
        RigMessage::Assistant { id, content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS
                + id.as_deref().map(estimated_text_tokens).unwrap_or(0)
                + content
                    .iter()
                    .map(estimated_assistant_content_tokens)
                    .sum::<u64>()
        }
    }
}

fn estimated_user_content_tokens(content: &UserContent) -> u64 {
    ESTIMATED_CONTENT_OVERHEAD_TOKENS
        + match content {
            UserContent::Text(text) => estimated_text_tokens(text.text()),
            UserContent::ToolResult(tool_result) => {
                estimated_text_tokens(&tool_result.id)
                    + tool_result
                        .call_id
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + tool_result
                        .content
                        .iter()
                        .map(estimated_tool_result_content_tokens)
                        .sum::<u64>()
            }
            UserContent::Image(image) => estimated_document_source_tokens(&image.data),
            UserContent::Audio(audio) => estimated_document_source_tokens(&audio.data),
            UserContent::Video(video) => estimated_document_source_tokens(&video.data),
            UserContent::Document(document) => estimated_document_source_tokens(&document.data),
        }
}

fn estimated_assistant_content_tokens(content: &AssistantContent) -> u64 {
    ESTIMATED_CONTENT_OVERHEAD_TOKENS
        + match content {
            AssistantContent::Text(text) => estimated_text_tokens(text.text()),
            AssistantContent::ToolCall(tool_call) => {
                estimated_text_tokens(&tool_call.id)
                    + tool_call
                        .call_id
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + estimated_text_tokens(&tool_call.function.name)
                    + estimated_json_tokens(&tool_call.function.arguments)
                    + tool_call
                        .signature
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + tool_call
                        .additional_params
                        .as_ref()
                        .map(estimated_json_tokens)
                        .unwrap_or(0)
            }
            AssistantContent::Reasoning(reasoning) => {
                reasoning
                    .id
                    .as_deref()
                    .map(estimated_text_tokens)
                    .unwrap_or(0)
                    + reasoning
                        .content
                        .iter()
                        .map(estimated_reasoning_content_tokens)
                        .sum::<u64>()
            }
            AssistantContent::Image(image) => estimated_document_source_tokens(&image.data),
        }
}

fn estimated_tool_result_content_tokens(content: &ToolResultContent) -> u64 {
    match content {
        ToolResultContent::Text(text) => estimated_text_tokens(text.text()),
        ToolResultContent::Image(image) => estimated_document_source_tokens(&image.data),
    }
}

fn estimated_reasoning_content_tokens(content: &ReasoningContent) -> u64 {
    match content {
        ReasoningContent::Text { text, signature } => {
            estimated_text_tokens(text)
                + signature.as_deref().map(estimated_text_tokens).unwrap_or(0)
        }
        ReasoningContent::Encrypted(_) => 0,
        ReasoningContent::Redacted { data } => estimated_text_tokens(data),
        ReasoningContent::Summary(summary) => estimated_text_tokens(summary),
        _ => {
            debug_assert!(
                false,
                "unhandled reasoning content variant in context estimation: {content:?}"
            );
            0
        }
    }
}

fn estimated_document_source_tokens(source: &DocumentSourceKind) -> u64 {
    match source {
        DocumentSourceKind::Url(value)
        | DocumentSourceKind::Base64(value)
        | DocumentSourceKind::String(value) => estimated_text_tokens(value),
        DocumentSourceKind::Raw(value) => estimate_binary_tokens(value),
        DocumentSourceKind::Unknown => 0,
        _ => {
            debug_assert!(
                false,
                "unhandled document source variant in context estimation: {source:?}"
            );
            0
        }
    }
}

fn estimated_json_tokens(value: &serde_json::Value) -> u64 {
    count_json_tokens(value)
}

fn estimated_text_tokens(text: &str) -> u64 {
    count_text_tokens(text)
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_counting::{count_json_tokens, count_text_tokens};
    use rig::{
        OneOrMany,
        completion::message::{AssistantContent, Text, ToolCall, ToolFunction},
    };
    use serde_json::json;

    #[test]
    fn captures_full_request_with_token_estimates() {
        let history = vec![
            RigMessage::system("system guidance"),
            RigMessage::Assistant {
                id: Some("assistant-1".into()),
                content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                    id: "tool-1".into(),
                    call_id: Some("call-1".into()),
                    function: ToolFunction::new("ReadFile".into(), json!({ "path": "src/lib.rs" })),
                    signature: None,
                    additional_params: None,
                })),
            },
        ];
        let prompt = RigMessage::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: "Why did the subagent fail?".into(),
            })),
        };

        let snapshot = CompletionRequestSnapshot::capture(&prompt, &history);

        assert_eq!(snapshot.message_count, 3);
        assert_eq!(snapshot.history, history);
        assert_eq!(snapshot.prompt, prompt);
        assert_eq!(snapshot.message_token_estimates.len(), 3);
        assert_eq!(
            snapshot
                .message_token_estimates
                .last()
                .map(|entry| entry.source),
            Some(CompletionRequestMessageSource::Prompt)
        );
        assert!(snapshot.estimated_total_tokens >= snapshot.estimated_prompt_tokens);
    }

    #[test]
    fn estimates_use_tokenizer_for_tool_arguments() {
        let message = RigMessage::Assistant {
            id: Some("assistant-1".into()),
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "tool-1".into(),
                call_id: Some("call-1".into()),
                function: ToolFunction::new(
                    "ReadFile".into(),
                    json!({ "path": "src/lib.rs", "offset": 120 }),
                ),
                signature: None,
                additional_params: None,
            })),
        };

        let estimated = estimated_message_tokens(&message);
        let expected = ESTIMATED_MESSAGE_OVERHEAD_TOKENS
            + count_text_tokens("assistant-1")
            + ESTIMATED_CONTENT_OVERHEAD_TOKENS
            + count_text_tokens("tool-1")
            + count_text_tokens("call-1")
            + count_text_tokens("ReadFile")
            + count_json_tokens(&json!({ "path": "src/lib.rs", "offset": 120 }));

        assert_eq!(estimated, expected);
    }

    #[test]
    fn estimates_use_tokenizer_for_plain_text() {
        let message = RigMessage::system("System guidance about tokenizer-backed estimates.");

        assert_eq!(
            estimated_message_tokens(&message),
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS
                + count_text_tokens("System guidance about tokenizer-backed estimates.")
        );
    }
}
