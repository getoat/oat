pub mod app;
pub mod config;
pub mod input;
pub mod llm;
pub mod tools;
pub mod ui;

use std::{
    error::Error,
    io::{self, Stdout},
    time::{Duration, Instant},
};

use app::{Action, App, Effect};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{runtime::Runtime, sync::mpsc};

use crate::{config::AppConfig, llm::LlmService};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn run(terminal: &mut Tui, config: AppConfig) -> Result<(), Box<dyn Error>> {
    let runtime = Runtime::new()?;
    let mut config = config;
    let mut llm = {
        let _guard = runtime.enter();
        LlmService::from_config(&config)?
    };
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
    let mut app = App::new(
        config.ui.show_thinking,
        config.ui.show_tool_output,
        config.azure.model_name.clone(),
        config.azure.reasoning_effort,
    );
    let tick_rate = Duration::from_millis(125);
    let mut last_tick = Instant::now();

    while !app.should_quit() {
        while let Ok((reply_id, event)) = stream_rx.try_recv() {
            app.apply(Action::StreamEvent { reply_id, event });
        }

        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Some(action) = input::map_event(event::read()?) {
                if let Some(effect) = app.apply(action) {
                    if let Err(error) = run_effect(
                        &runtime,
                        &mut llm,
                        &mut config,
                        &mut app,
                        stream_tx.clone(),
                        effect,
                    ) {
                        app.push_error_message(format!("Command failed: {error}"));
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.apply(app::Action::Tick);
            last_tick = Instant::now();
        }
    }

    Ok(())
}

pub fn setup_terminal() -> Result<Tui, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore_terminal(terminal: &mut Tui) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_effect(
    runtime: &Runtime,
    llm: &mut LlmService,
    config: &mut AppConfig,
    app: &mut App,
    stream_tx: mpsc::UnboundedSender<(u64, llm::StreamEvent)>,
    effect: Effect,
) -> anyhow::Result<()> {
    match effect {
        Effect::PromptModel {
            reply_id,
            prompt,
            history,
        } => {
            let llm = llm.clone();
            runtime.spawn(async move {
                llm.stream_prompt(reply_id, prompt, history, stream_tx)
                    .await;
            });
            Ok(())
        }
        Effect::SetReasoningEffort { reasoning_effort } => {
            let updated_config = AppConfig::set_default_reasoning_effort(reasoning_effort)?;
            let rebuilt = {
                let _guard = runtime.enter();
                LlmService::from_config(&updated_config)?
            };
            *config = updated_config;
            *llm = rebuilt;
            app.set_reasoning_effort(reasoning_effort);
            app.push_agent_message(format!(
                "Reasoning effort set to `{}` for model `{}` and saved to `config.toml`.",
                reasoning_effort.as_str(),
                app.model_name()
            ));
            Ok(())
        }
    }
}
