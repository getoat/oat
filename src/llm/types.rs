use anyhow::Result;
use std::sync::{Arc, Mutex};

use rig::completion::Message as RigMessage;

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
    history
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
    history
        .into_iter()
        .map(SessionHistoryMessage::into_rig_message)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
