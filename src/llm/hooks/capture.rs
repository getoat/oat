use rig::{
    agent::{HookAction, PromptHook},
    completion::CompletionModel,
};

use super::super::CompletionCapture;

pub(crate) const STEP_BOUNDARY_REASON: &str = "__oat_step_boundary__";

#[derive(Clone, Default)]
pub(crate) struct CompletionCaptureHook {
    pub(crate) capture: Option<CompletionCapture>,
}

#[derive(Clone, Default)]
pub(crate) struct StepBoundaryCapture {
    inner: std::sync::Arc<std::sync::Mutex<Option<StepBoundaryState>>>,
}

#[derive(Clone)]
pub(crate) struct StepBoundaryState {
    pub(crate) next_prompt: rig::completion::Message,
    pub(crate) history: Vec<rig::completion::Message>,
}

#[derive(Clone)]
pub(crate) struct StepBoundaryHook {
    pub(crate) capture: StepBoundaryCapture,
}

impl StepBoundaryCapture {
    pub(crate) fn set(
        &self,
        next_prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) {
        let mut slot = self.inner.lock().expect("step boundary lock");
        *slot = Some(StepBoundaryState {
            next_prompt: next_prompt.clone(),
            history: history.to_vec(),
        });
    }

    pub(crate) fn take(&self) -> Option<StepBoundaryState> {
        self.inner.lock().expect("step boundary lock").take()
    }
}

impl<M> PromptHook<M> for StepBoundaryHook
where
    M: CompletionModel,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        if self.capture.take().is_some() {
            self.capture.set(prompt, history);
            HookAction::Terminate {
                reason: STEP_BOUNDARY_REASON.to_string(),
            }
        } else {
            self.capture.set(prompt, history);
            HookAction::Continue
        }
    }
}

impl<M> PromptHook<M> for CompletionCaptureHook
where
    M: CompletionModel,
{
    async fn on_completion_call(
        &self,
        prompt: &rig::completion::Message,
        history: &[rig::completion::Message],
    ) -> HookAction {
        if let Some(capture) = &self.capture {
            capture.record(prompt, history);
        }
        HookAction::Continue
    }
}
