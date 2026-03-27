use anyhow::Result;
use futures_util::StreamExt;
use rig::{
    agent::MultiTurnStreamItem,
    client::CompletionClient,
    completion::Message as RigMessage,
    providers::openai,
    streaming::{StreamedAssistantContent, StreamingChat},
};
use serde_json::json;

use crate::{
    agent::{AgentContext, AgentRole},
    app::AccessMode,
    config::{AppConfig, KimiThinkingMode, ReasoningSetting},
    tools::{ToolContext, tools_for_context},
};

use super::LlmAgent;

const SYSTEM_PROMPT: &str = include_str!("../../prompts/system.md");

pub(crate) fn reasoning_params(model_name: &str, reasoning: ReasoningSetting) -> serde_json::Value {
    match reasoning {
        ReasoningSetting::Default => json!({}),
        ReasoningSetting::Gpt(reasoning_effort) => json!({
            "reasoning_effort": reasoning_effort.as_str()
        }),
        ReasoningSetting::Kimi(KimiThinkingMode::On) => json!({}),
        ReasoningSetting::Kimi(KimiThinkingMode::Off) if model_name == "kimi-k2.5" => json!({
            "thinking": {
                "type": "disabled"
            }
        }),
        ReasoningSetting::Kimi(KimiThinkingMode::Off) => json!({}),
    }
}

pub(crate) fn openai_base_url_for_model(
    config: &AppConfig,
    model_name: &str,
) -> anyhow::Result<String> {
    Ok(config.provider_config_for_model(model_name)?.base_url())
}

fn execution_mode_label(access_mode: AccessMode) -> &'static str {
    match access_mode {
        AccessMode::ReadOnly => "read-only mode",
        AccessMode::ReadWrite => "write mode",
    }
}

pub(crate) fn mode_preamble(context: &AgentContext) -> String {
    let mut preamble = SYSTEM_PROMPT.trim().replace(
        "{{EXECUTION_MODE}}",
        execution_mode_label(context.access_mode),
    );

    match context.role {
        AgentRole::Main => {
            preamble.push_str(
                "\n\nYou can delegate bounded parallel tasks to subagents when that will help you cover more ground. Give them enough local context to work independently. While subagents are running, normally treat that as a handoff: do not continue doing the same delegated work in the main agent unless the user explicitly wants redundancy or there is a clear independent task you can do without overlap. Prefer to wait on the subagents or inspect their status/results instead of duplicating their work or assuming they completed.",
            );
        }
        AgentRole::Subagent => {
            preamble.push_str(
                "\n\nYou are running as a subagent on behalf of the main agent. You start with fresh context, so rely on the delegated prompt and your own tool exploration. Focus tightly on the delegated task and return a concise result that the main agent can use directly. You cannot spawn subagents of your own.",
            );
        }
    }

    match context.access_mode {
        AccessMode::ReadOnly => {
            preamble.push_str(
                "\n\nYou are currently in read-only mode. Use the provided readonly workspace tools when they are useful. A shell tool may also be available, but only low-risk inspection commands can be approved in read-only mode; anything medium or high risk requires write mode. If the user asks you to edit, create, or delete files, explain that you are in read-only mode and the user must switch to write mode before you can modify the workspace. Do not print large amounts of code in read-only mode unless the user explicitly asks for it.",
            );
        }
        AccessMode::ReadWrite => {
            preamble.push_str(
                "\n\nYou are currently in write mode. Use the provided workspace tools when useful. Shell commands may still require per-command approval depending on risk. If the user asks you to write code, they usually mean to file (either as a new file, or to edit an existing one), rather than just printing it in their terminal, unless they explicitly ask for it.",
            );
        }
    }

    preamble
}

pub(crate) fn build_agent(
    client: &openai::CompletionsClient,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    tool_context: Option<ToolContext>,
) -> LlmAgent {
    let builder = client
        .agent(model_name.to_string())
        .preamble(preamble)
        .additional_params(reasoning_params(model_name, reasoning));
    match tool_context {
        Some(tool_context) => builder.tools(tools_for_context(tool_context)).build(),
        None => builder.build(),
    }
}

pub(crate) async fn run_plain_prompt(
    agent: &LlmAgent,
    prompt: String,
    history: Vec<RigMessage>,
) -> Result<String> {
    let mut stream = agent.stream_chat(prompt, history).multi_turn(0).await;
    let mut output = String::new();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(text))) => {
                output.push_str(&text.text);
            }
            Ok(MultiTurnStreamItem::FinalResponse(response)) => {
                if !response.response().is_empty() {
                    return Ok(response.response().to_string());
                }
                return Ok(output);
            }
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow::anyhow!("Request ended before response completed."))
}
