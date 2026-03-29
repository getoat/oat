use tokio::{runtime::Runtime, sync::mpsc};

use crate::{
    Tui,
    app::{Action, App, Effect, query},
    background_terminals::{BackgroundTerminalManager, BackgroundTerminalUiEvent},
    config::AppConfig,
    debug_log::log_debug,
    llm::{LlmService, TurnInterruptRequest},
    stats::StatsStore,
    subagents::{SubagentManager, SubagentUiEvent},
};

use super::side_channel_task_manager::SideChannelTaskManager;
use super::{
    RuntimeEvent, bootstrap::TuiBootstrap, effect_executor::EffectExecutor,
    reply_driver::ReplyDriver,
};

pub(crate) struct TurnController<'a> {
    runtime: &'a Runtime,
    terminal: &'a mut Tui,
    reply_driver: &'a mut ReplyDriver,
    side_channel_task_manager: &'a mut SideChannelTaskManager,
    llm: &'a mut LlmService,
    config: &'a mut AppConfig,
    app: &'a mut App,
    stats: &'a StatsStore,
    stream_tx: mpsc::UnboundedSender<RuntimeEvent>,
    subagents: &'a SubagentManager,
    terminals: &'a BackgroundTerminalManager,
}

impl<'a> TurnController<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        runtime: &'a Runtime,
        terminal: &'a mut Tui,
        reply_driver: &'a mut ReplyDriver,
        side_channel_task_manager: &'a mut SideChannelTaskManager,
        llm: &'a mut LlmService,
        config: &'a mut AppConfig,
        app: &'a mut App,
        stats: &'a StatsStore,
        stream_tx: mpsc::UnboundedSender<RuntimeEvent>,
        subagents: &'a SubagentManager,
        terminals: &'a BackgroundTerminalManager,
    ) -> Self {
        Self {
            runtime,
            terminal,
            reply_driver,
            side_channel_task_manager,
            llm,
            config,
            app,
            stats,
            stream_tx,
            subagents,
            terminals,
        }
    }

    pub(crate) fn from_bootstrap(terminal: &'a mut Tui, state: &'a mut TuiBootstrap) -> Self {
        Self::new(
            &state.runtime,
            terminal,
            &mut state.reply_driver,
            &mut state.side_channel_task_manager,
            &mut state.llm,
            &mut state.config,
            &mut state.app,
            &state.stats,
            state.stream_tx.clone(),
            &state.subagents,
            &state.terminals,
        )
    }

    pub(crate) fn handle_runtime_event(&mut self, runtime_event: RuntimeEvent) {
        log_debug(
            "turn_controller",
            format!(
                "handle_runtime_event event={} pending_before={} active_reply_before={:?}",
                runtime_event_label(&runtime_event),
                query::has_pending_reply(self.app.state()),
                query::active_reply_id(self.app.state())
            ),
        );
        let effect = match runtime_event {
            RuntimeEvent::MainReply { reply_id, event } => {
                self.reply_driver.clear_completed_task(reply_id, &event);
                if matches!(&event, crate::app::StreamEvent::Failed(_))
                    && self
                        .reply_driver
                        .should_defer_failed_stream_event(self.app, self.llm, reply_id)
                {
                    return;
                }

                self.app.apply(Action::StreamEvent { reply_id, event })
            }
            RuntimeEvent::SideChannel { reply_id, event } => {
                self.side_channel_task_manager
                    .clear_completed_task(reply_id);
                self.app.apply(Action::SideChannelEvent { reply_id, event })
            }
        };
        self.process_follow_ups(effect);
        log_debug(
            "turn_controller",
            format!(
                "handle_runtime_event_done pending_after={} active_reply_after={:?}",
                query::has_pending_reply(self.app.state()),
                query::active_reply_id(self.app.state())
            ),
        );
    }

    pub(crate) fn handle_subagent_event(&mut self, event: SubagentUiEvent) {
        let effect = self.app.apply(Action::SubagentEvent(event));
        self.process_follow_ups(effect);
    }

    pub(crate) fn handle_background_terminal_event(&mut self, event: BackgroundTerminalUiEvent) {
        let effect = self.app.apply(Action::BackgroundTerminalEvent(event));
        self.process_follow_ups(effect);
    }

    pub(crate) fn handle_action(&mut self, action: Action) {
        let effect = self.app.apply(action);
        self.process_follow_ups(effect);
    }

    fn process_follow_ups(&mut self, initial_effect: Option<Effect>) {
        let mut next_effect = initial_effect;
        loop {
            if let Some(effect) = next_effect.take() {
                if let Err(error) = self.run_effect(effect) {
                    crate::app::ops::transcript::push_error_message(
                        self.app.state_mut(),
                        format!("Command failed: {error}"),
                    );
                    break;
                }
                continue;
            }

            next_effect = self.reconcile_turn_progress();
            if next_effect.is_none() {
                break;
            }
        }
    }

    fn reconcile_turn_progress(&mut self) -> Option<Effect> {
        self.sync_turn_interrupt_policy();
        crate::app::session::submit::dispatch_next_queued_message_if_ready(self.app.state_mut())
    }

    fn sync_turn_interrupt_policy(&mut self) {
        match desired_turn_interrupt_request(self.app) {
            Some(request) => self.llm.request_turn_interrupt(request),
            None => self.llm.clear_turn_interrupt_request(),
        }
    }

    fn run_effect(&mut self, effect: Effect) -> anyhow::Result<()> {
        let mut runner = EffectExecutor {
            runtime: self.runtime,
            terminal: self.terminal,
            reply_driver: self.reply_driver,
            side_channel_task_manager: self.side_channel_task_manager,
            llm: self.llm,
            config: self.config,
            app: self.app,
            stats: self.stats,
            stream_tx: self.stream_tx.clone(),
            subagents: self.subagents,
            terminals: self.terminals,
        };
        runner.run(effect)
    }
}

fn runtime_event_label(event: &RuntimeEvent) -> &'static str {
    match event {
        RuntimeEvent::MainReply { event, .. } => match event {
            crate::app::StreamEvent::TextDelta(_) => "MainReply:TextDelta",
            crate::app::StreamEvent::Commentary(_) => "MainReply:Commentary",
            crate::app::StreamEvent::ReasoningDelta(_) => "MainReply:ReasoningDelta",
            crate::app::StreamEvent::ToolCall { .. } => "MainReply:ToolCall",
            crate::app::StreamEvent::ToolResult { .. } => "MainReply:ToolResult",
            crate::app::StreamEvent::TodoSnapshot(_) => "MainReply:TodoSnapshot",
            crate::app::StreamEvent::AskUserRequested { .. } => "MainReply:AskUserRequested",
            crate::app::StreamEvent::WriteApprovalRequested { .. } => {
                "MainReply:WriteApprovalRequested"
            }
            crate::app::StreamEvent::ShellApprovalRequested { .. } => {
                "MainReply:ShellApprovalRequested"
            }
            crate::app::StreamEvent::PlanningFinalizationStarted => {
                "MainReply:PlanningFinalizationStarted"
            }
            crate::app::StreamEvent::CompactionFinished { .. } => "MainReply:CompactionFinished",
            crate::app::StreamEvent::TurnEnded { .. } => "MainReply:TurnEnded",
            crate::app::StreamEvent::Failed(_) => "MainReply:Failed",
            crate::app::StreamEvent::SessionTitleGenerated(_) => "MainReply:SessionTitleGenerated",
        },
        RuntimeEvent::SideChannel { .. } => "SideChannel",
    }
}

fn desired_turn_interrupt_request(app: &App) -> Option<TurnInterruptRequest> {
    (query::has_queued_messages(app.state()) && query::has_pending_reply(app.state()))
        .then_some(TurnInterruptRequest::AtStepBoundary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{PendingReply, PendingReplyKind, session::test_support::new_app};

    #[test]
    fn desired_turn_interrupt_request_requires_queued_message_and_active_reply() {
        let mut app = new_app(true);
        assert_eq!(desired_turn_interrupt_request(&app), None);

        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        assert_eq!(desired_turn_interrupt_request(&app), None);

        app.state_mut()
            .session
            .queued_messages
            .push_back("steer the turn".into());
        assert_eq!(
            desired_turn_interrupt_request(&app),
            Some(TurnInterruptRequest::AtStepBoundary)
        );

        app.state_mut().session.pending_reply = None;
        assert_eq!(desired_turn_interrupt_request(&app), None);
    }
}
