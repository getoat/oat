use anyhow::{Result, anyhow};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

use crate::{
    app::{App, PendingReplyKind, StreamEvent},
    llm::{InteractionResolveResult, LlmService, ResumeRequest},
    stats::StatsStore,
};

#[derive(Default)]
pub(crate) struct ReplyDriver {
    active_reply_task: Option<(u64, JoinHandle<()>)>,
}

impl ReplyDriver {
    pub(crate) fn clear_completed_task(&mut self, reply_id: u64, event: &StreamEvent) {
        if matches!(
            event,
            StreamEvent::Finished { .. }
                | StreamEvent::CompactionFinished { .. }
                | StreamEvent::Failed(_)
        ) && self
            .active_reply_task
            .as_ref()
            .is_some_and(|(active_reply_id, _)| *active_reply_id == reply_id)
        {
            self.active_reply_task = None;
        }
    }

    pub(crate) fn should_defer_failed_stream_event(
        &self,
        app: &App,
        llm: &LlmService,
        reply_id: u64,
    ) -> bool {
        if app.active_reply_id() != Some(reply_id) {
            return false;
        }

        app.main_pending_write_approval_request_id()
            .is_some_and(|request_id| llm.can_resolve_write_approval(request_id))
            || app
                .main_pending_shell_approval_request_id()
                .is_some_and(|request_id| llm.can_resolve_shell_approval(request_id))
            || app
                .pending_ask_user()
                .is_some_and(|pending| llm.can_resolve_ask_user(&pending.request_id))
    }

    pub(crate) fn cancel_active_reply(&mut self, llm: &LlmService) {
        if let Some((_, task)) = self.active_reply_task.take() {
            task.abort();
        }
        llm.cancel_pending_interactions();
    }

    pub(crate) fn spawn_task(&mut self, reply_id: u64, task: JoinHandle<()>) {
        self.active_reply_task = Some((reply_id, task));
    }

    pub(crate) fn resume_interrupted_reply(
        &mut self,
        runtime: &Runtime,
        app: &mut App,
        stats: &StatsStore,
        llm: &LlmService,
        stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
        request: ResumeRequest,
    ) -> Result<()> {
        if let Some((_, task)) = self.active_reply_task.take() {
            task.abort();
        }

        let reply_kind = app.active_reply_kind().unwrap_or(PendingReplyKind::Normal);
        let reply_id = app.ensure_pending_reply(reply_kind);
        let replay_seed = app.pending_reply_replay_seed();
        let llm = llm.clone();
        let stats_hook = stats.hook_for_model(app.model_name().to_string());

        let task = runtime.spawn(async move {
            llm.stream_resumed_prompt(
                reply_id,
                request.snapshot,
                stats_hook,
                stream_tx,
                request.override_action,
                replay_seed,
            )
            .await;
        });
        self.spawn_task(reply_id, task);
        Ok(())
    }

    pub(crate) fn resolve_write_approval(
        &mut self,
        runtime: &Runtime,
        app: &mut App,
        stats: &StatsStore,
        llm: &LlmService,
        stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
        request_id: String,
        decision: crate::app::WriteApprovalDecision,
    ) -> Result<()> {
        match llm.resolve_write_approval(&request_id, decision) {
            InteractionResolveResult::Resolved => Ok(()),
            InteractionResolveResult::Resume(request) => {
                self.resume_interrupted_reply(runtime, app, stats, llm, stream_tx, request)
            }
            InteractionResolveResult::Missing => {
                app.push_error_message("Write approval request is no longer active.");
                Ok(())
            }
        }
    }

    pub(crate) fn resolve_shell_approval(
        &mut self,
        runtime: &Runtime,
        app: &mut App,
        stats: &StatsStore,
        llm: &LlmService,
        stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
        request_id: String,
        decision: crate::app::ShellApprovalDecision,
    ) -> Result<()> {
        match llm.resolve_shell_approval(&request_id, decision) {
            InteractionResolveResult::Resolved => Ok(()),
            InteractionResolveResult::Resume(request) => {
                self.resume_interrupted_reply(runtime, app, stats, llm, stream_tx, request)
            }
            InteractionResolveResult::Missing => {
                app.push_error_message("Shell approval request is no longer active.");
                Ok(())
            }
        }
    }

    pub(crate) fn resolve_ask_user(
        &mut self,
        runtime: &Runtime,
        app: &mut App,
        stats: &StatsStore,
        llm: &LlmService,
        stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
        request_id: String,
        response: crate::ask_user::AskUserResponse,
    ) -> Result<()> {
        match llm.resolve_ask_user(&request_id, response) {
            InteractionResolveResult::Resolved => Ok(()),
            InteractionResolveResult::Resume(request) => {
                self.resume_interrupted_reply(runtime, app, stats, llm, stream_tx, request)
            }
            InteractionResolveResult::Missing => {
                app.push_error_message("AskUser request is no longer active.");
                Ok(())
            }
        }
    }

    pub(crate) fn require_active_reply_id(app: &App) -> Result<u64> {
        app.active_reply_id()
            .ok_or_else(|| anyhow!("Compaction requires an active pending reply."))
    }
}
