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
mod history_reduction;
mod input;
mod llm;
mod memory;
mod model_registry;
mod runtime;
mod session_store;
mod stats;
mod subagents;
mod task;
mod todo;
mod token_counting;
mod tool_policy;
mod tool_result_status;
mod tools;
mod ui;
mod web;

use std::{
    error::Error,
    io::{self, Stdout},
    path::Path,
};

use anyhow::anyhow;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub use config::{ModelSelectionConfig, ReasoningSetting, RuntimeConfigOverrides};
pub use features::planning::PlanningAgentConfig;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeadlessMode {
    Prompt,
    Plan,
    PlanAndImplement,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeadlessOverrides {
    pub model_name: Option<String>,
    pub reasoning: Option<String>,
    pub planning_agents: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupOptions {
    access_mode: app::AccessMode,
    approval_mode: app::ApprovalMode,
    full_system_access: bool,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            access_mode: app::AccessMode::ReadOnly,
            approval_mode: app::ApprovalMode::Manual,
            full_system_access: false,
        }
    }
}

impl StartupOptions {
    pub fn dangerous() -> Self {
        Self {
            access_mode: app::AccessMode::ReadWrite,
            approval_mode: app::ApprovalMode::Disabled,
            full_system_access: true,
        }
    }

    pub(crate) fn access_mode(self) -> app::AccessMode {
        self.access_mode
    }

    pub(crate) fn full_system_access(self) -> bool {
        self.full_system_access
    }
}

pub fn run_default_tui(terminal: &mut Tui, startup: StartupOptions) -> Result<(), Box<dyn Error>> {
    run_tui_with_config(terminal, startup, None)
}

pub fn run_tui_with_config(
    terminal: &mut Tui,
    startup: StartupOptions,
    config_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    let config = config::AppConfig::refresh_codex_auth_if_needed_at_path(config_path)?;
    runtime::tui::run_with_options(terminal, config, startup)
}

pub fn run_default_headless(
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    run_headless_with_options(
        startup,
        None,
        HeadlessOverrides::default(),
        HeadlessMode::Prompt,
        prompt,
    )
}

pub fn run_headless_with_options(
    startup: StartupOptions,
    config_path: Option<&Path>,
    overrides: HeadlessOverrides,
    mode: HeadlessMode,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    let config = config::AppConfig::refresh_codex_auth_if_needed_at_path(config_path)?;
    let runtime_overrides = resolve_headless_overrides(&config, overrides)?;
    let config = config.with_runtime_overrides(runtime_overrides)?;
    match mode {
        HeadlessMode::Prompt => runtime::headless::run_headless(config, startup, prompt),
        HeadlessMode::Plan => runtime::headless::run_headless_plan(config, startup, prompt, false),
        HeadlessMode::PlanAndImplement => {
            runtime::headless::run_headless_plan(config, startup, prompt, true)
        }
    }
}

fn resolve_headless_overrides(
    config: &config::AppConfig,
    overrides: HeadlessOverrides,
) -> anyhow::Result<RuntimeConfigOverrides> {
    let base_model_name = overrides
        .model_name
        .clone()
        .unwrap_or_else(|| config.model.model_name.clone());
    let model_selection = if overrides.model_name.is_some() || overrides.reasoning.is_some() {
        let reasoning = match overrides.reasoning.as_deref() {
            Some(value) => {
                model_registry::parse_reasoning_setting_for_model(&base_model_name, value)
                    .map_err(|error| anyhow!(error.message("reasoning", &base_model_name, value)))?
            }
            None => model_registry::default_reasoning_setting_for_model(&base_model_name)
                .ok_or_else(|| {
                    anyhow!("No default reasoning is registered for `{base_model_name}`")
                })?,
        };
        Some(ModelSelectionConfig {
            model_name: base_model_name.clone(),
            reasoning,
        })
    } else {
        None
    };
    let planning_agents = if overrides.planning_agents.is_empty() {
        None
    } else {
        Some(
            overrides
                .planning_agents
                .into_iter()
                .map(parse_planning_agent_override)
                .collect::<anyhow::Result<Vec<_>>>()?,
        )
    };

    Ok(RuntimeConfigOverrides {
        model_selection,
        planning_agents,
    })
}

fn parse_planning_agent_override(value: String) -> anyhow::Result<PlanningAgentConfig> {
    let Some((model_name, reasoning_value)) = value.split_once("::") else {
        return Err(anyhow!(
            "Invalid planning agent `{value}`. Use `<model>::<reasoning>`."
        ));
    };
    let reasoning = model_registry::parse_reasoning_setting_for_model(model_name, reasoning_value)
        .map_err(|error| anyhow!(error.message("--planning-agent", model_name, reasoning_value)))?;
    Ok(PlanningAgentConfig {
        model_name: model_name.to_string(),
        reasoning,
    })
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
