mod agent_builder;
mod compaction;
mod hooks;
mod resume;
mod safety;
mod service;
mod streaming;
mod types;

pub(crate) type OpenAiCompletionsAgent =
    rig::agent::Agent<rig::providers::openai::completion::CompletionModel>;
pub(crate) type CodexResponsesClient =
    rig::providers::openai::Client<crate::codex::CodexHttpClient>;
pub(crate) type OpenAiResponsesAgent = rig::agent::Agent<
    rig::providers::openai::responses_api::ResponsesCompletionModel<crate::codex::CodexHttpClient>,
>;

pub(crate) use crate::app::StreamEvent;
pub(crate) use hooks::{AskUserController, ShellApprovalController, WriteApprovalController};
pub(crate) use service::LlmService;
pub(crate) use types::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResumeOverride, ResumeRequest, TurnInterruptController, TurnInterruptRequest,
};
pub(crate) use types::{history_from_rig, history_into_rig, history_with_prompt_from_rig};

#[cfg(test)]
mod tests;
