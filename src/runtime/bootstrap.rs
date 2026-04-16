use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::{runtime::Runtime, sync::mpsc};

use crate::{
    StartupOptions,
    agent::AgentContext,
    app::{App, ops, query},
    background_terminals::{BackgroundTerminalManager, BackgroundTerminalUiEvent},
    command_history::CommandHistoryStore,
    config::AppConfig,
    llm::{AskUserController, LlmService, WriteApprovalController},
    memory::MemoryService,
    session_store::SessionStore,
    stats::StatsStore,
    subagents::{SubagentManager, SubagentUiEvent},
    web::WebService,
};

use super::{
    RuntimeEvent, reply_driver::ReplyDriver, side_channel_task_manager::SideChannelTaskManager,
};

pub(crate) struct TuiBootstrap {
    pub(crate) runtime: Runtime,
    pub(crate) config: AppConfig,
    pub(crate) app: App,
    pub(crate) stats: StatsStore,
    pub(crate) session_store: SessionStore,
    pub(crate) memory: MemoryService,
    pub(crate) subagents: SubagentManager,
    pub(crate) terminals: BackgroundTerminalManager,
    pub(crate) command_history: CommandHistoryStore,
    pub(crate) llm: LlmService,
    pub(crate) stream_tx: mpsc::UnboundedSender<RuntimeEvent>,
    pub(crate) stream_rx: mpsc::UnboundedReceiver<RuntimeEvent>,
    pub(crate) subagent_rx: mpsc::UnboundedReceiver<SubagentUiEvent>,
    pub(crate) terminal_rx: mpsc::UnboundedReceiver<BackgroundTerminalUiEvent>,
    pub(crate) reply_driver: ReplyDriver,
    pub(crate) side_channel_task_manager: SideChannelTaskManager,
    pub(crate) tick_rate: Duration,
    pub(crate) last_tick: Instant,
}

pub(crate) fn bootstrap_tui(config: AppConfig, startup: StartupOptions) -> Result<TuiBootstrap> {
    let runtime = Runtime::new()?;
    let mut app = App::with_startup(
        config.ui.show_thinking,
        config.ui.show_tool_output,
        config.model.model_name.clone(),
        config.model.reasoning,
        config.planning.agents.clone(),
        startup.full_system_access(),
        startup.access_mode,
        startup.approval_mode,
    );
    app.set_safety_model_name(config.safety.model_name.clone());
    app.set_safety_reasoning(config.safety.reasoning);
    app.set_memory_model_name(config.memory.extraction.model_name.clone());
    app.set_memory_reasoning(config.memory.extraction.reasoning);
    app.state_mut().session.history_mode = config.history.mode;
    app.state_mut().session.history_retained_steps = config.history.retained_steps;
    let stats = StatsStore::new();
    let (subagent_tx, subagent_rx) = mpsc::unbounded_channel();
    let subagents =
        SubagentManager::new(config.subagents.max_concurrent, subagent_tx, stats.clone());
    let (terminal_tx, terminal_rx) = mpsc::unbounded_channel();
    let terminals = BackgroundTerminalManager::new(terminal_tx);
    let command_history = CommandHistoryStore::new(config.ui.command_history_limit);
    match command_history.load() {
        Ok(entries) => ops::session::restore_command_history(
            app.state_mut(),
            entries,
            config.ui.command_history_limit,
        ),
        Err(error) => {
            ops::transcript::push_error_message(
                app.state_mut(),
                format!("Failed to load input history: {error}"),
            );
        }
    }
    let memory = MemoryService::new(
        config.memory.clone(),
        app.state().session.workspace_root.clone(),
    )?;
    let llm_memory = config.memory.enabled.then_some(memory.clone());
    let web = WebService::new(config.tools.max_output_tokens)?;
    let llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            &config,
            AgentContext::main_with_full_system_access(
                query::mode(app.state()),
                app.state().session.full_system_access,
            ),
            WriteApprovalController::new(startup.approval_mode),
            Some(AskUserController::default()),
            true,
            llm_memory,
            Some(subagents.clone()),
            Some(terminals.clone()),
            web,
        )?
    };
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let session_store = SessionStore::new_tui(app.state(), &llm.preamble);

    Ok(TuiBootstrap {
        runtime,
        config,
        app,
        stats,
        session_store,
        memory,
        subagents,
        terminals,
        command_history,
        llm,
        stream_tx,
        stream_rx,
        subagent_rx,
        terminal_rx,
        reply_driver: ReplyDriver::default(),
        side_channel_task_manager: SideChannelTaskManager::default(),
        tick_rate: Duration::from_millis(125),
        last_tick: Instant::now(),
    })
}

pub(crate) struct HeadlessBootstrap {
    pub(crate) runtime: Runtime,
    pub(crate) config: AppConfig,
    pub(crate) stats: StatsStore,
    pub(crate) llm: LlmService,
    pub(crate) subagents: SubagentManager,
    #[allow(dead_code)]
    pub(crate) terminals: BackgroundTerminalManager,
}

pub(crate) fn bootstrap_headless(
    config: &AppConfig,
    startup: StartupOptions,
) -> Result<HeadlessBootstrap> {
    let runtime = Runtime::new()?;
    let stats = StatsStore::new();
    let memory = MemoryService::new(config.memory.clone(), std::env::current_dir()?)?;
    let llm_memory = config.memory.enabled.then_some(memory.clone());
    let (subagent_tx, _subagent_rx) = mpsc::unbounded_channel();
    let subagents =
        SubagentManager::new(config.subagents.max_concurrent, subagent_tx, stats.clone());
    let (terminal_tx, _terminal_rx) = mpsc::unbounded_channel();
    let terminals = BackgroundTerminalManager::new(terminal_tx);
    let web = WebService::new(config.tools.max_output_tokens)?;
    let llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            config,
            AgentContext::main_with_full_system_access(
                startup.access_mode,
                startup.full_system_access(),
            ),
            WriteApprovalController::new(startup.approval_mode),
            None,
            true,
            llm_memory,
            Some(subagents.clone()),
            Some(terminals.clone()),
            web,
        )?
    };

    Ok(HeadlessBootstrap {
        runtime,
        config: config.clone(),
        stats,
        llm,
        subagents,
        terminals,
    })
}
