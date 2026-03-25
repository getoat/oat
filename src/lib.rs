pub mod agent;
pub mod app;
pub mod ask_user;
mod command_history;
pub mod completion_request;
mod composer;
pub mod config;
pub mod features;
pub mod input;
pub mod llm;
pub mod model_registry;
mod runtime;
pub mod stats;
pub mod subagents;
pub mod token_counting;
pub mod tool_policy;
pub mod tools;
pub mod ui;

use std::{
    error::Error,
    io::{self, Stdout},
};

use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupOptions {
    pub access_mode: app::AccessMode,
    pub approval_mode: app::ApprovalMode,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            access_mode: app::AccessMode::ReadOnly,
            approval_mode: app::ApprovalMode::Manual,
        }
    }
}

pub fn run(terminal: &mut Tui, config: config::AppConfig) -> Result<(), Box<dyn Error>> {
    run_with_options(terminal, config, StartupOptions::default())
}

pub fn run_with_options(
    terminal: &mut Tui,
    config: config::AppConfig,
    startup: StartupOptions,
) -> Result<(), Box<dyn Error>> {
    runtime::tui::run_with_options(terminal, config, startup)
}

pub fn run_headless(
    config: config::AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    runtime::headless::run_headless(config, startup, prompt)
}

pub fn setup_terminal() -> Result<Tui, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

pub fn restore_terminal(terminal: &mut Tui) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
