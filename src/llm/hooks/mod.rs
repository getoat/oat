pub(crate) mod ask_user;
mod capture;
mod shell_approval;
pub(crate) mod write_approval;

use rig::{
    agent::{HookAction, PromptHook, ToolCallHookAction},
    completion::CompletionModel,
};

pub use ask_user::AskUserController;
pub(crate) use ask_user::AskUserHook;
pub(crate) use capture::{
    CompletionCaptureHook, STEP_BOUNDARY_REASON, StepBoundaryCapture, StepBoundaryHook,
    StepBoundaryState,
};
pub use shell_approval::ShellApprovalController;
pub(crate) use shell_approval::ShellApprovalHook;
pub use write_approval::WriteApprovalController;
pub(crate) use write_approval::WriteApprovalHook;

#[derive(Clone)]
pub(crate) struct CombinedHook<H1, H2> {
    pub(crate) first: H1,
    pub(crate) second: H2,
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
