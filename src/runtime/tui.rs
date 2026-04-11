use std::{
    error::Error,
    time::{Duration, Instant},
};

use crate::{
    Tui,
    app::{Action, StreamEvent, query},
    background_terminals::BackgroundTerminalStatus,
    input, ui,
};

use super::{
    RuntimeEvent, bootstrap::bootstrap_tui, command_history::persist_command_history_if_needed,
    turn_controller::TurnController,
};

const MAX_EVENTS_PER_FRAME: usize = 256;
const FRAME_EVENT_BUDGET: Duration = Duration::from_millis(16);
const SESSION_PERSIST_DEBOUNCE: Duration = Duration::from_millis(500);
const STREAM_DELTA_COALESCE_BYTES: usize = 8 * 1024;

struct SessionPersistenceScheduler {
    dirty: bool,
    force_flush: bool,
    last_flush_at: Instant,
}

impl SessionPersistenceScheduler {
    fn new(now: Instant) -> Self {
        Self {
            dirty: false,
            force_flush: false,
            last_flush_at: now,
        }
    }

    fn mark_dirty(&mut self, force_flush: bool) {
        self.dirty = true;
        self.force_flush |= force_flush;
    }

    fn should_flush(&self, now: Instant) -> bool {
        self.dirty
            && (self.force_flush
                || now.duration_since(self.last_flush_at) >= SESSION_PERSIST_DEBOUNCE)
    }

    fn flushed(&mut self, now: Instant) {
        self.dirty = false;
        self.force_flush = false;
        self.last_flush_at = now;
    }
}

pub(crate) fn run_with_options(
    terminal: &mut Tui,
    config: crate::config::AppConfig,
    startup: crate::StartupOptions,
) -> Result<(), Box<dyn Error>> {
    let mut state = bootstrap_tui(config, startup)?;
    let mut persistence = SessionPersistenceScheduler::new(Instant::now());
    let mut pending_runtime_event = None;
    let mut next_channel = 0_usize;

    while !query::should_quit(state.app.state()) {
        drain_event_frame(
            terminal,
            &mut state,
            &mut persistence,
            &mut pending_runtime_event,
            &mut next_channel,
        );
        flush_pending_persistence_if_needed(&mut state, &mut persistence, false);

        state.app.set_session_stats(state.stats.current_totals());
        state
            .app
            .state_mut()
            .session
            .active_background_terminal_count = state
            .terminals
            .list()
            .into_iter()
            .filter(|terminal| terminal.status == BackgroundTerminalStatus::Running)
            .count();
        terminal.draw(|frame| ui::render(frame, &mut state.app))?;

        let timeout = state.tick_rate.saturating_sub(state.last_tick.elapsed());
        if crossterm::event::poll(timeout)?
            && let Some(action) = input::map_event_with_context(
                crossterm::event::read()?,
                query::input_context(state.app.state()),
            )
        {
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_action(action);
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
            persistence.mark_dirty(false);
            flush_pending_persistence_if_needed(&mut state, &mut persistence, false);
        } else {
            flush_pending_persistence_if_needed(&mut state, &mut persistence, false);
        }

        if state.last_tick.elapsed() >= state.tick_rate {
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_action(Action::Tick);
            }
            persistence.mark_dirty(false);
            flush_pending_persistence_if_needed(&mut state, &mut persistence, false);
            state.last_tick = Instant::now();
        }
    }

    flush_pending_persistence_if_needed(&mut state, &mut persistence, true);
    state.side_channel_task_manager.cancel_all();
    state.subagents.cancel_all_running_now(true);
    state.terminals.cancel_all_running();
    state.session_store.finalize_current_session()?;
    state.stats.finalize_current_session()?;
    Ok(())
}

fn drain_event_frame(
    terminal: &mut Tui,
    state: &mut super::bootstrap::TuiBootstrap,
    persistence: &mut SessionPersistenceScheduler,
    pending_runtime_event: &mut Option<RuntimeEvent>,
    next_channel: &mut usize,
) {
    let frame_start = Instant::now();
    let mut processed = 0_usize;

    while processed < MAX_EVENTS_PER_FRAME && frame_start.elapsed() < FRAME_EVENT_BUDGET {
        let mut handled = false;

        for offset in 0..3 {
            let channel = (*next_channel + offset) % 3;
            match channel {
                0 => {
                    let Some(event) =
                        next_runtime_event(&mut state.stream_rx, pending_runtime_event)
                    else {
                        continue;
                    };
                    let force_flush = runtime_event_requires_immediate_persist(&event);
                    {
                        let mut controller = TurnController::from_bootstrap(terminal, state);
                        controller.handle_runtime_event(event);
                    }
                    persist_command_history_if_needed(&mut state.app, &state.command_history);
                    persistence.mark_dirty(force_flush);
                    *next_channel = 1;
                    handled = true;
                    break;
                }
                1 => {
                    let Some(event) = state.subagent_rx.try_recv().ok() else {
                        continue;
                    };
                    {
                        let mut controller = TurnController::from_bootstrap(terminal, state);
                        controller.handle_subagent_event(event);
                    }
                    persistence.mark_dirty(false);
                    *next_channel = 2;
                    handled = true;
                    break;
                }
                _ => {
                    let Some(event) = state.terminal_rx.try_recv().ok() else {
                        continue;
                    };
                    {
                        let mut controller = TurnController::from_bootstrap(terminal, state);
                        controller.handle_background_terminal_event(event);
                    }
                    persistence.mark_dirty(false);
                    *next_channel = 0;
                    handled = true;
                    break;
                }
            }
        }

        if !handled {
            break;
        }
        processed += 1;
        flush_pending_persistence_if_needed(state, persistence, false);
    }
}

fn next_runtime_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<RuntimeEvent>,
    pending: &mut Option<RuntimeEvent>,
) -> Option<RuntimeEvent> {
    let event = pending.take().or_else(|| rx.try_recv().ok())?;
    match event {
        RuntimeEvent::MainReply {
            reply_id,
            event: StreamEvent::TextDelta(mut delta),
        } => {
            while delta.len() < STREAM_DELTA_COALESCE_BYTES {
                let Some(next) = rx.try_recv().ok() else {
                    break;
                };
                match next {
                    RuntimeEvent::MainReply {
                        reply_id: next_reply_id,
                        event: StreamEvent::TextDelta(next_delta),
                    } if next_reply_id == reply_id
                        && delta.len() + next_delta.len() <= STREAM_DELTA_COALESCE_BYTES =>
                    {
                        delta.push_str(&next_delta);
                    }
                    other => {
                        *pending = Some(other);
                        break;
                    }
                }
            }
            Some(RuntimeEvent::MainReply {
                reply_id,
                event: StreamEvent::TextDelta(delta),
            })
        }
        RuntimeEvent::MainReply {
            reply_id,
            event: StreamEvent::ReasoningDelta(mut delta),
        } => {
            while delta.len() < STREAM_DELTA_COALESCE_BYTES {
                let Some(next) = rx.try_recv().ok() else {
                    break;
                };
                match next {
                    RuntimeEvent::MainReply {
                        reply_id: next_reply_id,
                        event: StreamEvent::ReasoningDelta(next_delta),
                    } if next_reply_id == reply_id
                        && delta.len() + next_delta.len() <= STREAM_DELTA_COALESCE_BYTES =>
                    {
                        delta.push_str(&next_delta);
                    }
                    other => {
                        *pending = Some(other);
                        break;
                    }
                }
            }
            Some(RuntimeEvent::MainReply {
                reply_id,
                event: StreamEvent::ReasoningDelta(delta),
            })
        }
        other => Some(other),
    }
}

fn runtime_event_requires_immediate_persist(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::MainReply {
            event: StreamEvent::TurnEnded { .. }
                | StreamEvent::Failed(_)
                | StreamEvent::AskUserRequested { .. }
                | StreamEvent::WriteApprovalRequested { .. }
                | StreamEvent::ShellApprovalRequested { .. },
            ..
        } | RuntimeEvent::CodexLoginCompleted { .. }
    )
}

fn flush_pending_persistence_if_needed(
    state: &mut super::bootstrap::TuiBootstrap,
    scheduler: &mut SessionPersistenceScheduler,
    force: bool,
) {
    if force {
        scheduler.mark_dirty(true);
    }
    let now = Instant::now();
    if !scheduler.should_flush(now) {
        return;
    }

    if let Err(error) = state
        .session_store
        .sync_tui(state.app.state(), &state.llm.preamble)
    {
        crate::app::ops::transcript::push_error_message(
            state.app.state_mut(),
            format!("Failed to persist session state: {error}"),
        );
    } else {
        scheduler.flushed(now);
    }
}
