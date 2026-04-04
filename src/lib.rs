mod agent;
mod app;
mod ask_user;
mod background_terminals;
mod codex;
mod command_history;
mod completion_request;
mod composer;
mod config;
mod debug_log;
mod features;
mod input;
mod llm;
mod memory;
mod model_registry;
mod runtime;
mod session_store;
mod stats;
mod subagents;
mod todo;
mod token_counting;
mod tool_policy;
mod tools;
mod ui;
mod web;

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
    access_mode: app::AccessMode,
    approval_mode: app::ApprovalMode,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            access_mode: app::AccessMode::ReadOnly,
            approval_mode: app::ApprovalMode::Manual,
        }
    }
}

impl StartupOptions {
    pub fn dangerous() -> Self {
        Self {
            access_mode: app::AccessMode::ReadWrite,
            approval_mode: app::ApprovalMode::Disabled,
        }
    }
}

pub fn run_default_tui(terminal: &mut Tui, startup: StartupOptions) -> Result<(), Box<dyn Error>> {
    let config = config::AppConfig::refresh_default_codex_auth_if_needed()?;
    runtime::tui::run_with_options(terminal, config, startup)
}

pub fn run_default_headless(
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    let config = config::AppConfig::refresh_default_codex_auth_if_needed()?;
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
