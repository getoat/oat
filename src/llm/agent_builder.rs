use anyhow::Result;
use futures_util::StreamExt;
use rig::{
    agent::{MultiTurnStreamItem, PromptHook},
    client::CompletionClient,
    completion::CompletionModel,
    completion::Message as RigMessage,
    http_client::{HeaderMap, HeaderValue},
    streaming::{StreamedAssistantContent, StreamingChat},
};
use serde_json::json;

use crate::{
    agent::{AgentContext, AgentRole},
    app::AccessMode,
    config::{AppConfig, KimiThinkingMode, ReasoningSetting},
    model_registry,
    tools::{ToolContext, tools_for_context},
};
const SYSTEM_PROMPT: &str = include_str!("../../prompts/system.md");
const OPENROUTER_REFERER: &str = "https://getoat.app";
const OPENROUTER_TITLE: &str = "oat";
const OPENAI_BETA_HEADER: &str = "responses=experimental";
const OPENAI_ORIGINATOR_HEADER_VALUE: &str = "codex_cli_rs";

pub(crate) fn reasoning_params(model_name: &str, reasoning: ReasoningSetting) -> serde_json::Value {
    match reasoning {
        ReasoningSetting::Default => json!({}),
        ReasoningSetting::Gpt(reasoning_effort) => {
            match model_registry::find_model(model_name).map(|model| model.provider) {
                Some(model_registry::ModelProvider::OpenRouter) => json!({
                    "reasoning": {
                        "effort": reasoning_effort.as_str()
                    }
                }),
                Some(model_registry::ModelProvider::Codex) => json!({
                    "reasoning": {
                        "effort": reasoning_effort.as_str(),
                        "summary": "auto"
                    },
                    "store": false
                }),
                _ => json!({
                    "reasoning_effort": reasoning_effort.as_str()
                }),
            }
        }
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

pub(crate) fn http_headers_for_model(
    config: &AppConfig,
    model_name: &str,
) -> anyhow::Result<HeaderMap> {
    let provider_config = config.provider_config_for_model(model_name)?;
    let mut headers = HeaderMap::new();

    if matches!(
        model_registry::find_model(model_name).map(|model| model.provider),
        Some(model_registry::ModelProvider::Codex)
    ) {
        headers.insert("OpenAI-Beta", HeaderValue::from_static(OPENAI_BETA_HEADER));
        headers.insert(
            "originator",
            HeaderValue::from_static(OPENAI_ORIGINATOR_HEADER_VALUE),
        );
        if let Some(account_id) = provider_config.account_id() {
            headers.insert("chatgpt-account-id", HeaderValue::from_str(account_id)?);
        }
    }

    if matches!(
        model_registry::find_model(model_name).map(|model| model.provider),
        Some(model_registry::ModelProvider::OpenRouter)
    ) {
        headers.insert("HTTP-Referer", HeaderValue::from_static(OPENROUTER_REFERER));
        headers.insert(
            "X-OpenRouter-Title",
            HeaderValue::from_static(OPENROUTER_TITLE),
        );
    }

    Ok(headers)
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

pub(crate) fn build_agent<C>(
    client: &C,
    model_name: &str,
    preamble: &str,
    reasoning: ReasoningSetting,
    tool_context: Option<ToolContext>,
) -> rig::agent::Agent<C::CompletionModel>
where
    C: CompletionClient,
    C::CompletionModel: CompletionModel + 'static,
{
    let api_model_name = crate::codex::api_model_name(model_name);
    let builder = client
        .agent(api_model_name.to_string())
        .preamble(preamble)
        .additional_params(reasoning_params(model_name, reasoning));
    match tool_context {
        Some(tool_context) => builder.tools(tools_for_context(tool_context)).build(),
        None => builder.build(),
    }
}

pub(crate) async fn run_plain_prompt_with_hook<M, H>(
    agent: &rig::agent::Agent<M>,
    prompt: String,
    history: Vec<RigMessage>,
    hook: H,
) -> Result<String>
where
    M: CompletionModel + 'static,
    H: PromptHook<M> + 'static,
{
    let mut stream = agent
        .stream_chat(prompt, history)
        .with_hook(hook)
        .multi_turn(0)
        .await;
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
