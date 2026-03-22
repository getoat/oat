use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use futures_util::StreamExt;
use rig::{
    agent::{HookAction, MultiTurnStreamItem, PromptHook, ToolCallHookAction},
    client::CompletionClient,
    completion::{
        CompletionModel, Message as RigMessage,
        message::{ToolResult, ToolResultContent},
    },
    providers::openai,
    streaming::{StreamedAssistantContent, StreamedUserContent, StreamingChat},
};
use serde_json::json;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::{
    app::{AccessMode, WriteApprovalDecision},
    config::AppConfig,
    stats::StatsHook,
    tools::{is_mutation_tool, tool_names_for_mode, tools_for_mode},
};

const MAX_TOOL_STEPS_PER_TURN: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        output: String,
    },
    WriteApprovalRequested {
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    Finished {
        history: Option<Vec<RigMessage>>,
    },
    Failed(String),
}

type LlmAgent = rig::agent::Agent<openai::CompletionModel>;

#[derive(Clone, Default)]
pub struct WriteApprovalController {
    inner: Arc<Mutex<WriteApprovalState>>,
}

#[derive(Default)]
struct WriteApprovalState {
    allow_all_session: bool,
    pending: HashMap<String, oneshot::Sender<WriteApprovalDecision>>,
}

#[derive(Clone)]
struct WriteApprovalHook {
    reply_id: u64,
    events: UnboundedSender<(u64, StreamEvent)>,
    approvals: WriteApprovalController,
}

#[derive(Clone)]
struct CombinedHook<H1, H2> {
    first: H1,
    second: H2,
}

#[derive(Clone)]
pub struct LlmService {
    agent: LlmAgent,
    approvals: WriteApprovalController,
    #[cfg_attr(not(test), allow(dead_code))]
    tool_names: Vec<String>,
    #[cfg_attr(not(test), allow(dead_code))]
    preamble: String,
}

impl LlmService {
    pub fn from_config(
        config: &AppConfig,
        access_mode: AccessMode,
        approvals: WriteApprovalController,
    ) -> Result<Self> {
        let workspace_root = env::current_dir().context("failed to determine workspace root")?;
        let client = openai::CompletionsClient::builder()
            .api_key(&config.azure.api_key)
            .base_url(azure_openai_base_url(config))
            .build()
            .context("failed to build OpenAI-compatible Azure client")?;

        let preamble = mode_preamble(access_mode);
        let tool_names = tool_names_for_mode(access_mode);
        let agent = client
            .agent(config.azure.model_name.clone())
            .preamble(&preamble)
            .additional_params(reasoning_params(config))
            .tools(tools_for_mode(&workspace_root, access_mode))
            .build();

        Ok(Self {
            agent,
            approvals,
            tool_names,
            preamble,
        })
    }

    pub fn approvals(&self) -> WriteApprovalController {
        self.approvals.clone()
    }

    pub fn resolve_write_approval(
        &self,
        request_id: &str,
        decision: WriteApprovalDecision,
    ) -> bool {
        self.approvals.resolve(request_id, decision)
    }

    pub fn reset_write_approvals(&self) {
        self.approvals.reset_session();
    }

    pub async fn stream_prompt(
        &self,
        reply_id: u64,
        prompt: String,
        history: Vec<RigMessage>,
        stats_hook: StatsHook,
        events: UnboundedSender<(u64, StreamEvent)>,
    ) {
        let hook = WriteApprovalHook {
            reply_id,
            events: events.clone(),
            approvals: self.approvals.clone(),
        };
        let hook = CombinedHook {
            first: stats_hook,
            second: hook,
        };
        let mut stream = self
            .agent
            .stream_chat(prompt, history)
            .with_hook(hook)
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
                Ok(MultiTurnStreamItem::FinalResponse(response)) => Some(StreamEvent::Finished {
                    history: response.history().map(ToOwned::to_owned),
                }),
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
    }
}

impl WriteApprovalController {
    async fn request_approval(
        &self,
        reply_id: u64,
        tool_name: &str,
        internal_call_id: &str,
        args: &str,
        events: &UnboundedSender<(u64, StreamEvent)>,
    ) -> ToolCallHookAction {
        let rx = {
            let mut state = self.inner.lock().expect("approval state lock");
            if state.allow_all_session {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(internal_call_id.to_string(), tx);
            rx
        };

        if events
            .send((
                reply_id,
                StreamEvent::WriteApprovalRequested {
                    request_id: internal_call_id.to_string(),
                    tool_name: tool_name.to_string(),
                    arguments: args.to_string(),
                },
            ))
            .is_err()
        {
            let mut state = self.inner.lock().expect("approval state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "Write action cancelled because approval UI is unavailable.",
            );
        }

        match rx.await {
            Ok(WriteApprovalDecision::AllowOnce | WriteApprovalDecision::AllowAllSession) => {
                ToolCallHookAction::Continue
            }
            Ok(WriteApprovalDecision::Deny) => {
                ToolCallHookAction::skip("Write action denied by user.")
            }
            Err(_) => ToolCallHookAction::skip("Write action cancelled before approval."),
        }
    }

    fn resolve(&self, request_id: &str, decision: WriteApprovalDecision) -> bool {
        let sender = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                state.allow_all_session = true;
            }
            state.pending.remove(request_id)
        };

        if let Some(sender) = sender {
            sender.send(decision).is_ok()
        } else {
            false
        }
    }

    fn reset_session(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.allow_all_session = false;
        for (_, sender) in state.pending.drain() {
            let _ = sender.send(WriteApprovalDecision::Deny);
        }
    }
}

impl PromptHook<openai::CompletionModel> for WriteApprovalHook {
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        if !is_mutation_tool(tool_name) {
            return ToolCallHookAction::Continue;
        }

        self.approvals
            .request_approval(
                self.reply_id,
                tool_name,
                internal_call_id,
                args,
                &self.events,
            )
            .await
    }
}

impl<M, H1, H2> PromptHook<M> for CombinedHook<H1, H2>
where
    M: CompletionModel,
    H1: PromptHook<M>,
    H2: PromptHook<M>,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        let first = self.first.on_completion_call(prompt, history).await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second.on_completion_call(prompt, history).await
    }

    async fn on_tool_call(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        let first = self
            .first
            .on_tool_call(tool_name, tool_call_id.clone(), internal_call_id, args)
            .await;
        if !matches!(first, ToolCallHookAction::Continue) {
            return first;
        }
        self.second
            .on_tool_call(tool_name, tool_call_id, internal_call_id, args)
            .await
    }

    async fn on_tool_result(
        &self,
        tool_name: &str,
        tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
        result: &str,
    ) -> HookAction {
        let first = self
            .first
            .on_tool_result(
                tool_name,
                tool_call_id.clone(),
                internal_call_id,
                args,
                result,
            )
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_tool_result(tool_name, tool_call_id, internal_call_id, args, result)
            .await
    }

    async fn on_text_delta(&self, text_delta: &str, aggregated_text: &str) -> HookAction {
        let first = self.first.on_text_delta(text_delta, aggregated_text).await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second.on_text_delta(text_delta, aggregated_text).await
    }

    async fn on_tool_call_delta(
        &self,
        tool_call_id: &str,
        internal_call_id: &str,
        tool_name: Option<&str>,
        tool_call_delta: &str,
    ) -> HookAction {
        let first = self
            .first
            .on_tool_call_delta(tool_call_id, internal_call_id, tool_name, tool_call_delta)
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_tool_call_delta(tool_call_id, internal_call_id, tool_name, tool_call_delta)
            .await
    }

    async fn on_stream_completion_response_finish(
        &self,
        prompt: &rig::completion::Message,
        response: &M::StreamingResponse,
    ) -> HookAction {
        let first = self
            .first
            .on_stream_completion_response_finish(prompt, response)
            .await;
        if matches!(first, HookAction::Terminate { .. }) {
            return first;
        }
        self.second
            .on_stream_completion_response_finish(prompt, response)
            .await
    }
}

fn reasoning_params(config: &AppConfig) -> serde_json::Value {
    json!({
        "reasoning_effort": config.azure.reasoning_effort.as_str()
    })
}

fn azure_openai_base_url(config: &AppConfig) -> String {
    format!(
        "{}/openai/v1",
        config.azure.endpoint().trim_end_matches('/')
    )
}

fn mode_preamble(access_mode: AccessMode) -> String {
    match access_mode {
        AccessMode::ReadOnly => "You are oat, an opinionated agent thing. If you refer to yourself, use exactly the name `oat` in lowercase. If the user asks who you are, answer with `oat - an opinionated agent thing` and then briefly describe what you can do in this workspace. Keep that capability summary concise and practical. In read-only mode, emphasize that you can inspect files, explain code, answer questions, and use read-only workspace tools, but cannot modify the workspace. Do not call yourself an AI assistant, and do not describe yourself as helping via an API. Answer only the user's most recent message directly and helpfully. You are currently in read-only mode. Use the provided readonly workspace tools when they are useful. If the user asks you to edit, create, or delete files, explain that oat is in read-only mode and the user must switch to write mode before you can modify the workspace. Within a single turn, you may call tools multiple times and use prior tool calls and tool outputs from that same turn. Do not rely on memory from previous turns.".to_string(),
        AccessMode::ReadWrite => "You are oat, an opinionated agent thing. If you refer to yourself, use exactly the name `oat` in lowercase. If the user asks who you are, answer with `oat - an opinionated agent thing` and then briefly describe what you can do in this workspace. Keep that capability summary concise and practical. In write mode, emphasize that you can inspect files, explain code, and make workspace changes with tool use and user approval for mutations. Do not call yourself an AI assistant, and do not describe yourself as helping via an API. Answer only the user's most recent message directly and helpfully. You are currently in write mode. Read and mutation tools may be available. Any mutation tool call requires user approval before it executes, and the user may deny it. For every mutation tool call, include the required `intent` field as a short sentence explaining why the change is needed for the user. Explain purpose or outcome, not the mechanical edit. If a write is denied, acknowledge that and continue from the current workspace state. Within a single turn, you may call tools multiple times and use prior tool calls and tool outputs from that same turn. Do not rely on memory from previous turns.".to_string(),
    }
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
                command_history_limit: 20,
            },
        }
    }

    #[test]
    fn reasoning_params_match_requested_effort() {
        let params = reasoning_params(&sample_config());
        assert_eq!(params, json!({ "reasoning_effort": "minimal" }));
    }

    #[test]
    fn azure_openai_base_url_targets_v1_endpoint() {
        assert_eq!(
            azure_openai_base_url(&sample_config()),
            "https://demo-resource.openai.azure.com/openai/v1"
        );
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

    #[test]
    fn read_only_mode_preamble_mentions_switching_to_write_mode() {
        let preamble = mode_preamble(AccessMode::ReadOnly);
        assert!(preamble.contains("You are oat, an opinionated agent thing."));
        assert!(preamble.contains("use exactly the name `oat` in lowercase"));
        assert!(preamble.contains("answer with `oat - an opinionated agent thing`"));
        assert!(preamble.contains("Keep that capability summary concise and practical"));
        assert!(preamble.contains("inspect files, explain code, answer questions"));
        assert!(preamble.contains("Do not call yourself an AI assistant"));
        assert!(preamble.contains("read-only mode"));
        assert!(preamble.contains("switch to write mode"));
    }

    #[tokio::test]
    async fn read_write_mode_registers_mutation_tools() {
        let service = LlmService::from_config(
            &sample_config(),
            AccessMode::ReadWrite,
            WriteApprovalController::default(),
        )
        .expect("service builds");

        assert!(service.tool_names.contains(&"ApplyPatches".to_string()));
        assert!(service.tool_names.contains(&"WriteFile".to_string()));
        assert!(service.tool_names.contains(&"DeletePath".to_string()));
        assert!(
            service
                .preamble
                .contains("You are oat, an opinionated agent thing.")
        );
        assert!(
            service
                .preamble
                .contains("use exactly the name `oat` in lowercase")
        );
        assert!(
            service
                .preamble
                .contains("answer with `oat - an opinionated agent thing`")
        );
        assert!(
            service
                .preamble
                .contains("Keep that capability summary concise and practical")
        );
        assert!(
            service
                .preamble
                .contains("make workspace changes with tool use and user approval")
        );
        assert!(
            service
                .preamble
                .contains("Do not call yourself an AI assistant")
        );
        assert!(service.preamble.contains("write mode"));
        assert!(service.preamble.contains("intent"));
        assert!(service.preamble.contains("why"));
    }

    #[tokio::test]
    async fn read_only_mode_omits_mutation_tools() {
        let service = LlmService::from_config(
            &sample_config(),
            AccessMode::ReadOnly,
            WriteApprovalController::default(),
        )
        .expect("service builds");

        assert!(!service.tool_names.contains(&"ApplyPatches".to_string()));
        assert!(!service.tool_names.contains(&"WriteFile".to_string()));
        assert!(!service.tool_names.contains(&"DeletePath".to_string()));
    }

    #[test]
    fn write_approval_controller_reset_is_safe_without_pending_requests() {
        let approvals = WriteApprovalController::default();
        assert!(!approvals.resolve("missing", WriteApprovalDecision::AllowAllSession));
        approvals.reset_session();
    }
}
