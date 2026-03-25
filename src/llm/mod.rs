mod agent_builder;
mod compaction;
mod hooks;
mod resume;
mod safety;
mod service;
mod streaming;
mod types;

pub(crate) type LlmAgent = rig::agent::Agent<rig::providers::openai::CompletionModel>;

pub(crate) use crate::app::StreamEvent;
pub(crate) use hooks::{AskUserController, WriteApprovalController};
pub(crate) use service::LlmService;
pub(crate) use types::{
    CompletionCapture, EventCallback, HistoryCompactionResult, InteractionResolveResult,
    PromptRunResult, ResumeOverride, ResumeRequest,
};
pub(crate) use types::{history_from_rig, history_into_rig};

#[cfg(test)]
mod tests;
