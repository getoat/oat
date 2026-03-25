use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rig::{
    agent::{PromptHook, ToolCallHookAction},
    providers::openai,
};
use tokio::sync::oneshot;

use crate::{
    app::{ApprovalMode, WriteApprovalDecision},
    completion_request::CompletionRequestSnapshot,
    tools::is_mutation_tool,
};

use super::super::{
    CompletionCapture, EventCallback, InteractionResolveResult, ResumeOverride, ResumeRequest,
    resume::ResumeOverrideController,
};

#[derive(Clone)]
pub struct WriteApprovalController {
    pub(crate) inner: Arc<Mutex<WriteApprovalState>>,
}

pub(crate) struct WriteApprovalState {
    default_mode: ApprovalMode,
    mode: ApprovalMode,
    pub(crate) pending: HashMap<String, PendingWriteApprovalEntry>,
}

pub(crate) struct PendingWriteApprovalEntry {
    pub(crate) sender: oneshot::Sender<WriteApprovalDecision>,
    pub(crate) snapshot: Option<CompletionRequestSnapshot>,
    pub(crate) tool_name: String,
    pub(crate) arguments: String,
}

#[derive(Clone)]
pub(crate) struct WriteApprovalHook {
    pub(crate) reply_id: u64,
    pub(crate) emit: EventCallback,
    pub(crate) approvals: WriteApprovalController,
    pub(crate) capture: Option<CompletionCapture>,
    pub(crate) resume: Option<ResumeOverrideController>,
}

impl Default for WriteApprovalController {
    fn default() -> Self {
        Self::new(ApprovalMode::Manual)
    }
}

impl WriteApprovalController {
    pub fn new(mode: ApprovalMode) -> Self {
        Self {
            inner: Arc::new(Mutex::new(WriteApprovalState {
                default_mode: mode,
                mode,
                pending: HashMap::new(),
            })),
        }
    }

    pub fn mode(&self) -> ApprovalMode {
        let state = self.inner.lock().expect("approval state lock");
        state.mode
    }

    pub(crate) fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("approval state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_approval(
        &self,
        reply_id: u64,
        tool_name: &str,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        if let Some(decision) = resume.and_then(|resume| resume.consume_write(tool_name, args)) {
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                let mut state = self.inner.lock().expect("approval state lock");
                state.mode = ApprovalMode::Disabled;
            }
            return match decision {
                WriteApprovalDecision::AllowOnce | WriteApprovalDecision::AllowAllSession => {
                    ToolCallHookAction::Continue
                }
                WriteApprovalDecision::Deny => {
                    ToolCallHookAction::skip("Write action denied by user.")
                }
            };
        }

        let rx = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(state.mode, ApprovalMode::Disabled) {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingWriteApprovalEntry {
                    sender: tx,
                    snapshot,
                    tool_name: tool_name.to_string(),
                    arguments: args.to_string(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            super::super::StreamEvent::WriteApprovalRequested {
                request_id: internal_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: args.to_string(),
            },
        ) {
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

    pub(crate) fn resolve(
        &self,
        request_id: &str,
        decision: WriteApprovalDecision,
    ) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("approval state lock");
            if matches!(decision, WriteApprovalDecision::AllowAllSession) {
                state.mode = ApprovalMode::Disabled;
            }
            state.pending.remove(request_id)
        };

        let Some(pending) = pending else {
            return InteractionResolveResult::Missing;
        };

        if pending.sender.send(decision).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = pending.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::WriteApproval {
                    tool_name: pending.tool_name,
                    arguments: pending.arguments,
                    decision,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    pub(crate) fn reset_session(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.mode = state.default_mode;
        for (_, pending) in state.pending.drain() {
            let _ = pending.sender.send(WriteApprovalDecision::Deny);
        }
    }

    pub(crate) fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("approval state lock");
        state.pending.clear();
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
                &self.emit,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}
