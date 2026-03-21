use std::{collections::HashMap, env};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use rig::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::message::{ToolResult, ToolResultContent},
    providers::azure::{self, AzureOpenAIAuth},
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingPrompt},
};
use serde_json::json;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::AppConfig;
use crate::tools::{GrepTool, ListTool, ReadFileTool};

const MAX_TOOL_STEPS_PER_TURN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCall { name: String, arguments: String },
    ToolResult { name: String, output: String },
    Finished,
    Failed(String),
}

type AzureAgent = rig::agent::Agent<azure::CompletionModel>;

#[derive(Clone)]
pub struct LlmService {
    agent: AzureAgent,
}

impl LlmService {
    pub fn from_config(config: &AppConfig) -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let client = azure::Client::builder()
            .api_key(AzureOpenAIAuth::ApiKey(config.azure.api_key.clone()))
            .azure_endpoint(config.azure.endpoint())
            .api_version(&config.azure.api_version)
            .build()
            .context("failed to build Azure OpenAI client")?;

        let agent = client
            .agent(config.azure.model_name.clone())
            .preamble(
                "You are oat. Answer only the user's most recent message directly and helpfully. \
Use the provided readonly workspace tools when they are useful. Within a single turn, you may \
call tools multiple times and use prior tool calls and tool outputs from that same turn. Do not \
rely on memory from previous turns.",
            )
            .additional_params(reasoning_params(config))
            .tool(ListTool::new(workspace_root.clone()))
            .tool(ReadFileTool::new(workspace_root.clone()))
            .tool(GrepTool::new(workspace_root))
            .build();

        Ok(Self { agent })
    }

    pub async fn stream_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        events: UnboundedSender<(u64, StreamEvent)>,
    ) {
        let mut stream = self
            .agent
            .stream_prompt(prompt)
            .multi_turn(MAX_TOOL_STEPS_PER_TURN)
            .await;
        let mut tool_calls = HashMap::<String, String>::new();

        while let Some(chunk) = stream.next().await {
            let event = match chunk {
                Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(
                    text,
                ))) => Some(StreamEvent::TextDelta(text.text)),
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::Reasoning(reasoning),
                )) => Some(StreamEvent::ReasoningDelta(reasoning.display_text())),
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. },
                )) => Some(StreamEvent::ReasoningDelta(reasoning)),
                Ok(MultiTurnStreamItem::StreamAssistantItem(
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    },
                )) => {
                    let name = tool_call.function.name.clone();
                    let arguments = format_tool_arguments(&tool_call.function.arguments);
                    tool_calls.insert(internal_call_id, name.clone());
                    Some(StreamEvent::ToolCall { name, arguments })
                }
                Ok(MultiTurnStreamItem::StreamUserItem(StreamedUserContent::ToolResult {
                    tool_result,
                    internal_call_id,
                })) => Some(StreamEvent::ToolResult {
                    name: tool_calls
                        .get(&internal_call_id)
                        .cloned()
                        .unwrap_or_else(|| tool_result.id.clone()),
                    output: format_tool_result(&tool_result),
                }),
                Ok(MultiTurnStreamItem::FinalResponse(_)) => None,
                Ok(_) => None,
                Err(error) => {
                    let _ = events.send((reply_id, StreamEvent::Failed(error.to_string())));
                    return;
                }
            };

            if let Some(event) = event
                && events.send((reply_id, event)).is_err()
            {
                return;
            }
        }

        let _ = events.send((reply_id, StreamEvent::Finished));
    }
}

fn reasoning_params(config: &AppConfig) -> serde_json::Value {
    json!({
        "reasoning_effort": config.azure.reasoning_effort.as_str()
    })
}

fn format_tool_arguments(arguments: &serde_json::Value) -> String {
    serde_json::to_string(arguments).unwrap_or_else(|_| arguments.to_string())
}

fn format_tool_result(tool_result: &ToolResult) -> String {
    let parts = tool_result
        .content
        .iter()
        .map(|content| match content {
            ToolResultContent::Text(text) => text.text.clone(),
            ToolResultContent::Image(_) => "[image tool result]".to_string(),
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AzureConfig, ReasoningEffort, UiConfig};
    use rig::{OneOrMany, completion::message::Text};

    fn sample_config() -> AppConfig {
        AppConfig {
            azure: AzureConfig {
                resource_name: "demo-resource".into(),
                api_key: "secret".into(),
                model_name: "gpt-5-mini".into(),
                reasoning_effort: ReasoningEffort::Minimal,
                api_version: "2025-01-01-preview".into(),
            },
            ui: UiConfig {
                show_thinking: true,
                show_tool_output: false,
            },
        }
    }

    #[test]
    fn reasoning_params_match_requested_effort() {
        let params = reasoning_params(&sample_config());
        assert_eq!(params, json!({ "reasoning_effort": "minimal" }));
    }

    #[test]
    fn format_tool_result_joins_text_parts() {
        let tool_result = ToolResult {
            id: "call_1".into(),
            call_id: None,
            content: OneOrMany::many(vec![
                ToolResultContent::Text(Text {
                    text: "first".into(),
                }),
                ToolResultContent::Text(Text {
                    text: "second".into(),
                }),
            ])
            .expect("non-empty"),
        };

        assert_eq!(format_tool_result(&tool_result), "first\nsecond");
    }

    #[test]
    fn format_tool_arguments_serializes_json_compactly() {
        assert_eq!(
            format_tool_arguments(&json!({ "dir": "src", "recursive": true })),
            r#"{"dir":"src","recursive":true}"#
        );
    }

    #[tokio::test]
    async fn service_builds_from_config_without_network_calls() {
        let service = LlmService::from_config(&sample_config()).expect("service builds");
        let _ = service;
    }
}
