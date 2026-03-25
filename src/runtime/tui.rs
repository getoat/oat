use std::error::Error;

use crate::{
    Tui,
    app::{Action, StreamEvent, ops, query},
    input, ui,
};

use super::{
    bootstrap::bootstrap_tui, command_history::persist_command_history_if_needed,
    effect_executor::EffectExecutor,
};

pub(crate) fn run_with_options(
    terminal: &mut Tui,
    config: crate::config::AppConfig,
    startup: crate::StartupOptions,
) -> Result<(), Box<dyn Error>> {
    let mut state = bootstrap_tui(config, startup)?;

    while !query::should_quit(state.app.state()) {
        while let Ok((reply_id, event)) = state.stream_rx.try_recv() {
            state.reply_driver.clear_completed_task(reply_id, &event);
            if matches!(&event, StreamEvent::Failed(_))
                && state
                    .reply_driver
                    .should_defer_failed_stream_event(&state.app, &state.llm, reply_id)
            {
                continue;
            }
            if let Some(effect) = state.app.apply(Action::StreamEvent { reply_id, event }) {
                let mut runner = EffectExecutor {
                    runtime: &state.runtime,
                    terminal,
                    reply_driver: &mut state.reply_driver,
                    llm: &mut state.llm,
                    config: &mut state.config,
                    app: &mut state.app,
                    stats: &state.stats,
                    stream_tx: state.stream_tx.clone(),
                    subagents: &state.subagents,
                };
                if let Err(error) = runner.run(effect) {
                    ops::transcript::push_error_message(
                        state.app.state_mut(),
                        format!("Command failed: {error}"),
                    );
                }
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }
        while let Ok(event) = state.subagent_rx.try_recv() {
            state.app.apply(Action::SubagentEvent(event));
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }

        state.app.state_mut().session.session_stats = state.stats.current_totals();
        terminal.draw(|frame| ui::render(frame, &mut state.app))?;

        let timeout = state.tick_rate.saturating_sub(state.last_tick.elapsed());
        if crossterm::event::poll(timeout)?
            && let Some(action) = input::map_event_with_context(
                crossterm::event::read()?,
                query::input_context(state.app.state()),
            )
            && let Some(effect) = state.app.apply(action)
        {
            let mut runner = EffectExecutor {
                runtime: &state.runtime,
                terminal,
                reply_driver: &mut state.reply_driver,
                llm: &mut state.llm,
                config: &mut state.config,
                app: &mut state.app,
                stats: &state.stats,
                stream_tx: state.stream_tx.clone(),
                subagents: &state.subagents,
            };
            if let Err(error) = runner.run(effect) {
                ops::transcript::push_error_message(
                    state.app.state_mut(),
                    format!("Command failed: {error}"),
                );
            }
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        } else {
            persist_command_history_if_needed(&mut state.app, &state.command_history);
        }

        if state.last_tick.elapsed() >= state.tick_rate {
            state.app.apply(crate::app::Action::Tick);
            persist_command_history_if_needed(&mut state.app, &state.command_history);
            state.last_tick = std::time::Instant::now();
        }
    }

    state.stats.finalize_current_session()?;
    Ok(())
}
