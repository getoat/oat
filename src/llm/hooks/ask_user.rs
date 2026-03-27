use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rig::{
    agent::{PromptHook, ToolCallHookAction},
    completion::CompletionModel,
    tool::Tool,
};
use tokio::sync::oneshot;

use crate::{
    ask_user::{AskUserRequest, AskUserResponse, validate_request},
    completion_request::CompletionRequestSnapshot,
    tools::AskUserTool,
};

use super::super::{
    CompletionCapture, EventCallback, InteractionResolveResult, ResumeOverride, ResumeRequest,
    StreamEvent, resume::ResumeOverrideController,
};

#[derive(Clone)]
pub struct AskUserController {
    pub(crate) inner: Arc<Mutex<AskUserState>>,
}

pub(crate) struct AskUserState {
    pub(crate) pending: HashMap<String, PendingAskUserEntry>,
}

pub(crate) struct PendingAskUserEntry {
    pub(crate) sender: oneshot::Sender<AskUserResponse>,
    pub(crate) snapshot: Option<CompletionRequestSnapshot>,
    pub(crate) request: AskUserRequest,
}

#[derive(Clone)]
pub(crate) struct AskUserHook {
    pub(crate) reply_id: u64,
    pub(crate) emit: EventCallback,
    pub(crate) controller: Option<AskUserController>,
    pub(crate) capture: Option<CompletionCapture>,
    pub(crate) resume: Option<ResumeOverrideController>,
}

impl Default for AskUserController {
    fn default() -> Self {
        Self::new()
    }
}

impl AskUserController {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AskUserState {
                pending: HashMap::new(),
            })),
        }
    }

    pub(crate) fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("ask user state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_input(
        &self,
        reply_id: u64,
        internal_call_id: &str,
        args: &str,
        emit: &EventCallback,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        let request = match serde_json::from_str::<AskUserRequest>(args) {
            Ok(request) => request,
            Err(error) => {
                return ToolCallHookAction::skip(format!(
                    "AskUser request was invalid JSON: {error}"
                ));
            }
        };
        if let Err(error) = validate_request(&request) {
            return ToolCallHookAction::skip(format!("AskUser validation error: {error}"));
        }

        if let Some(response) = resume.and_then(|resume| resume.consume_ask_user(&request)) {
            return ToolCallHookAction::skip(serde_json::to_string(&response).unwrap_or_else(
                |_| "{\"questions\":[],\"error\":\"failed to serialize AskUser response\"}".into(),
            ));
        }

        let rx = {
            let mut state = self.inner.lock().expect("ask user state lock");
            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingAskUserEntry {
                    sender: tx,
                    snapshot,
                    request: request.clone(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            StreamEvent::AskUserRequested {
                request_id: internal_call_id.to_string(),
                request: request.clone(),
            },
        ) {
            let mut state = self.inner.lock().expect("ask user state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "AskUser was cancelled because the interactive UI is unavailable.",
            );
        }

        match rx.await {
            Ok(response) => {
                ToolCallHookAction::skip(serde_json::to_string(&response).unwrap_or_else(|_| {
                    "{\"questions\":[],\"error\":\"failed to serialize AskUser response\"}".into()
                }))
            }
            Err(_) => ToolCallHookAction::skip("AskUser was cancelled before the user answered."),
        }
    }

    pub(crate) fn resolve(
        &self,
        request_id: &str,
        response: AskUserResponse,
    ) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("ask user state lock");
            state.pending.remove(request_id)
        };

        let Some(pending) = pending else {
            return InteractionResolveResult::Missing;
        };

        if pending.sender.send(response.clone()).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = pending.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::AskUser {
                    request: pending.request,
                    response,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    pub(crate) fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("ask user state lock");
        state.pending.clear();
    }
}

impl<M> PromptHook<M> for AskUserHook
where
    M: CompletionModel,
{
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        if tool_name != AskUserTool::NAME {
            return ToolCallHookAction::Continue;
        }

        let Some(controller) = &self.controller else {
            return ToolCallHookAction::skip(
                "AskUser requires the interactive UI and is unavailable in this runtime.",
            );
        };

        controller
            .request_input(
                self.reply_id,
                internal_call_id,
                args,
                &self.emit,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}
