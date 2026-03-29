use std::error::Error;

use crate::{
    Tui,
    app::{Action, query},
    background_terminals::BackgroundTerminalStatus,
    input, ui,
};

use super::{
    bootstrap::bootstrap_tui, command_history::persist_command_history_if_needed,
    turn_controller::TurnController,
};

const MAX_PENDING_EVENTS_PER_LOOP: usize = 128;

pub(crate) fn run_with_options(
    terminal: &mut Tui,
    config: crate::config::AppConfig,
    startup: crate::StartupOptions,
) -> Result<(), Box<dyn Error>> {
    let mut state = bootstrap_tui(config, startup)?;

    while !query::should_quit(state.app.state()) {
        for _ in 0..MAX_PENDING_EVENTS_PER_LOOP {
            let Some(event) = state.stream_rx.try_recv().ok() else {
                break;
            };
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_runtime_event(event);
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }
        for _ in 0..MAX_PENDING_EVENTS_PER_LOOP {
            let Some(event) = state.subagent_rx.try_recv().ok() else {
                break;
            };
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_subagent_event(event);
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }
        for _ in 0..MAX_PENDING_EVENTS_PER_LOOP {
            let Some(event) = state.terminal_rx.try_recv().ok() else {
                break;
            };
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_background_terminal_event(event);
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }

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
        } else {
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }

        if state.last_tick.elapsed() >= state.tick_rate {
            {
                let mut controller = TurnController::from_bootstrap(terminal, &mut state);
                controller.handle_action(Action::Tick);
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
            state.last_tick = std::time::Instant::now();
        }
    }

    state.side_channel_task_manager.cancel_all();
    state.terminals.cancel_all_running();
    state.stats.finalize_current_session()?;
    Ok(())
}
