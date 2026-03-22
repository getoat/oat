pub mod agent;
pub mod app;
mod command_history;
pub mod completion_request;
pub mod config;
pub mod input;
pub mod llm;
pub mod model_registry;
pub mod stats;
pub mod subagents;
pub mod token_counting;
pub mod tool_policy;
pub mod tools;
pub mod ui;

use std::{
    error::Error,
    io::{self, Stdout, Write},
    time::{Duration, Instant},
};

use anyhow::anyhow;
use app::{Action, App, ApprovalMode, Effect};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

use crate::{
    agent::AgentContext, command_history::CommandHistoryStore, config::AppConfig, llm::LlmService,
    stats::StatsStore, subagents::SubagentManager,
};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupOptions {
    pub access_mode: app::AccessMode,
    pub approval_mode: ApprovalMode,
}

impl Default for StartupOptions {
    fn default() -> Self {
        Self {
            access_mode: app::AccessMode::ReadOnly,
            approval_mode: ApprovalMode::Manual,
        }
    }
}

struct EffectRunner<'a> {
    runtime: &'a Runtime,
    terminal: &'a mut Tui,
    active_reply_task: &'a mut Option<(u64, JoinHandle<()>)>,
    llm: &'a mut LlmService,
    config: &'a mut AppConfig,
    app: &'a mut App,
    stats: &'a StatsStore,
    stream_tx: mpsc::UnboundedSender<(u64, llm::StreamEvent)>,
    subagents: &'a SubagentManager,
}

pub fn run(terminal: &mut Tui, config: AppConfig) -> Result<(), Box<dyn Error>> {
    run_with_options(terminal, config, StartupOptions::default())
}

pub fn run_with_options(
    terminal: &mut Tui,
    config: AppConfig,
    startup: StartupOptions,
) -> Result<(), Box<dyn Error>> {
    let runtime = Runtime::new()?;
    let mut config = config;
    let mut app = App::with_startup(
        config.ui.show_thinking,
        config.ui.show_tool_output,
        config.azure.model_name.clone(),
        config.azure.reasoning_effort,
        startup.access_mode,
        startup.approval_mode,
    );
    let stats = StatsStore::new();
    let (subagent_tx, mut subagent_rx) = mpsc::unbounded_channel();
    let subagents =
        SubagentManager::new(config.subagents.max_concurrent, subagent_tx, stats.clone());
    let command_history = CommandHistoryStore::new(config.ui.command_history_limit);
    match command_history.load() {
        Ok(entries) => app.restore_command_history(entries, config.ui.command_history_limit),
        Err(error) => app.push_error_message(format!("Failed to load input history: {error}")),
    }
    let mut llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            &config,
            AgentContext::main(app.mode()),
            llm::WriteApprovalController::new(startup.approval_mode),
            Some(subagents.clone()),
        )?
    };
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
    let tick_rate = Duration::from_millis(125);
    let mut last_tick = Instant::now();
    let mut active_reply_task: Option<(u64, JoinHandle<()>)> = None;

    while !app.should_quit() {
        while let Ok((reply_id, event)) = stream_rx.try_recv() {
            if matches!(
                event,
                llm::StreamEvent::Finished { .. } | llm::StreamEvent::Failed(_)
            ) && active_reply_task
                .as_ref()
                .is_some_and(|(active_reply_id, _)| *active_reply_id == reply_id)
            {
                active_reply_task = None;
            }
            app.apply(Action::StreamEvent { reply_id, event });
            persist_command_history_if_needed(&mut app, &command_history);
        }
        while let Ok(event) = subagent_rx.try_recv() {
            app.apply(Action::SubagentEvent(event));
            persist_command_history_if_needed(&mut app, &command_history);
        }

        app.set_session_stats(stats.current_totals());
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Some(action) =
                input::map_event_with_state(event::read()?, app.has_pending_write_approval())
            && let Some(effect) = app.apply(action)
        {
            let mut runner = EffectRunner {
                runtime: &runtime,
                terminal,
                active_reply_task: &mut active_reply_task,
                llm: &mut llm,
                config: &mut config,
                app: &mut app,
                stats: &stats,
                stream_tx: stream_tx.clone(),
                subagents: &subagents,
            };
            if let Err(error) = runner.run(effect) {
                app.push_error_message(format!("Command failed: {error}"));
            }
            persist_command_history_if_needed(&mut app, &command_history);
        } else {
            persist_command_history_if_needed(&mut app, &command_history);
        }

        if last_tick.elapsed() >= tick_rate {
            app.apply(app::Action::Tick);
            persist_command_history_if_needed(&mut app, &command_history);
            last_tick = Instant::now();
        }
    }

    stats.finalize_current_session()?;
    Ok(())
}

pub fn run_headless(
    config: AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    let runtime = Runtime::new()?;
    let stats = StatsStore::new();
    let llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            &config,
            AgentContext::main(startup.access_mode),
            llm::WriteApprovalController::new(startup.approval_mode),
            None,
        )?
    };
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
    let stats_hook = stats.hook_for_model(config.azure.model_name.clone());
    let task = runtime.spawn({
        let llm = llm.clone();
        async move {
            llm.stream_prompt(1, prompt, Vec::new(), stats_hook, stream_tx)
                .await;
        }
    });

    let result = runtime.block_on(async {
        let mut output = String::new();

        while let Some((reply_id, event)) = stream_rx.recv().await {
            if reply_id != 1 {
                continue;
            }

            match event {
                llm::StreamEvent::TextDelta(delta) => output.push_str(&delta),
                llm::StreamEvent::Finished { .. } => return Ok(output),
                llm::StreamEvent::Failed(error) => {
                    return Err(anyhow!("Request failed: {error}"));
                }
                llm::StreamEvent::ReasoningDelta(_)
                | llm::StreamEvent::ToolCall { .. }
                | llm::StreamEvent::ToolResult { .. }
                | llm::StreamEvent::WriteApprovalRequested { .. } => {}
            }
        }

        Err(anyhow!("Request ended before response completed."))
    });

    task.abort();
    stats.finalize_current_session()?;
    result.map_err(Into::into)
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

impl EffectRunner<'_> {
    fn run(&mut self, effect: Effect) -> anyhow::Result<()> {
        match effect {
            Effect::PromptModel {
                reply_id,
                prompt,
                history,
            } => {
                self.cancel_active_reply();
                let llm = self.llm.clone();
                let stats_hook = self.stats.hook_for_model(self.app.model_name().to_string());
                let stream_tx = self.stream_tx.clone();
                let task = self.runtime.spawn(async move {
                    llm.stream_prompt(reply_id, prompt, history, stats_hook, stream_tx)
                        .await;
                });
                *self.active_reply_task = Some((reply_id, task));
                Ok(())
            }
            Effect::ShowStats => {
                self.app.push_agent_message(self.stats.report()?.render());
                Ok(())
            }
            Effect::RotateSession => {
                self.stats.rotate_session()?;
                self.llm.reset_write_approvals();
                Ok(())
            }
            Effect::SetModelSelection { model_name } => {
                let reasoning_effort =
                    app::compatible_reasoning_effort(&model_name, self.app.reasoning_effort());
                let updated_config =
                    AppConfig::set_default_model_selection(&model_name, reasoning_effort)?;
                let rebuilt = self.rebuild_llm(&updated_config, self.app.mode())?;
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_model_name(model_name.clone());
                self.app.set_reasoning_effort(reasoning_effort);
                self.app.open_reasoning_picker();
                self.app.push_agent_message(format!(
                    "Model set to `{}` and saved to the active config. Select a reasoning effort.",
                    model_name
                ));
                Ok(())
            }
            Effect::SetReasoningEffort { reasoning_effort } => {
                let updated_config = AppConfig::set_default_reasoning_effort(reasoning_effort)?;
                let rebuilt = self.rebuild_llm(&updated_config, self.app.mode())?;
                *self.config = updated_config;
                *self.llm = rebuilt;
                self.app.set_reasoning_effort(reasoning_effort);
                self.app.push_agent_message(format!(
                    "Reasoning effort set to `{}` for model `{}` and saved to the active config.",
                    reasoning_effort.as_str(),
                    self.app.model_name()
                ));
                Ok(())
            }
            Effect::RebuildLlm { access_mode } => {
                let rebuilt = self.rebuild_llm(self.config, access_mode)?;
                *self.llm = rebuilt;
                Ok(())
            }
            Effect::ResolveWriteApproval {
                request_id,
                decision,
            } => {
                if !self.llm.resolve_write_approval(&request_id, decision) {
                    self.app
                        .push_error_message("Write approval request is no longer active.");
                }
                Ok(())
            }
            Effect::CopyToClipboard { text } => {
                write!(
                    self.terminal.backend_mut(),
                    "{}",
                    osc52_copy_sequence(&text)
                )?;
                self.terminal.backend_mut().flush()?;
                let line_count = text.lines().count().max(1);
                self.app.push_agent_message(format!(
                    "Copied {line_count} line{} to the terminal clipboard.",
                    if line_count == 1 { "" } else { "s" }
                ));
                Ok(())
            }
            Effect::CancelPendingReply => {
                self.cancel_active_reply();
                Ok(())
            }
        }
    }

    fn rebuild_llm(
        &self,
        config: &AppConfig,
        access_mode: app::AccessMode,
    ) -> anyhow::Result<LlmService> {
        let _guard = self.runtime.enter();
        LlmService::from_config(
            config,
            AgentContext::main(access_mode),
            self.llm.approvals(),
            Some(self.subagents.clone()),
        )
    }

    fn cancel_active_reply(&mut self) {
        if let Some((_, task)) = self.active_reply_task.take() {
            task.abort();
        }
    }
}

fn persist_command_history_if_needed(app: &mut App, store: &CommandHistoryStore) {
    let Some(entries) = app.take_command_history_to_persist() else {
        return;
    };

    if let Err(error) = store.save(&entries) {
        app.push_error_message(format!("Failed to save input history: {error}"));
    }
}

fn osc52_copy_sequence(text: &str) -> String {
    format!("\u{1b}]52;c;{}\u{7}", STANDARD.encode(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_sequence_base64_encodes_selection_text() {
        assert_eq!(
            osc52_copy_sequence("copy me"),
            "\u{1b}]52;c;Y29weSBtZQ==\u{7}"
        );
    }
}
