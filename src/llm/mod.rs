mod agent_builder;
mod compaction;
mod hooks;
mod responses_search;
mod resume;
mod safety;
mod service;
mod streaming;
mod types;

pub(crate) type OpenAiCompletionsAgent =
    rig::agent::Agent<rig::providers::openai::completion::CompletionModel>;
pub(crate) type ResponsesClient = rig::providers::openai::Client<crate::codex::ResponsesHttpClient>;
pub(crate) type ResponsesAgent = rig::agent::Agent<
    rig::providers::openai::responses_api::ResponsesCompletionModel<
        crate::codex::ResponsesHttpClient,
    >,
>;

pub(crate) use crate::app::StreamEvent;
pub(crate) use hooks::{AskUserController, ShellApprovalController, WriteApprovalController};
pub(crate) use responses_search::{
    OAT_INTERACTION_SCOPE_HEADER, ResponsesHostedToolEvent, ResponsesSearchObserverGuard,
    observer_for_scope as responses_search_observer_for_scope,
};
pub(crate) use service::{LlmService, run_internal_plain_prompt};
pub(crate) use types::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResumeOverride, ResumeRequest, TurnInterruptController, TurnInterruptRequest,
};
pub(crate) use types::{history_from_rig, history_into_rig, history_with_prompt_from_rig};

#[cfg(test)]
mod tests;
