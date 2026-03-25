use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::{runtime::Runtime, sync::mpsc};

use crate::{
    StartupOptions,
    agent::AgentContext,
    app::{App, StreamEvent, ops, query},
    command_history::CommandHistoryStore,
    config::AppConfig,
    llm::{AskUserController, LlmService, WriteApprovalController},
    stats::StatsStore,
    subagents::{SubagentManager, SubagentUiEvent},
};

use super::reply_driver::ReplyDriver;

pub(crate) struct TuiBootstrap {
    pub(crate) runtime: Runtime,
    pub(crate) config: AppConfig,
    pub(crate) app: App,
    pub(crate) stats: StatsStore,
    pub(crate) subagents: SubagentManager,
    pub(crate) command_history: CommandHistoryStore,
    pub(crate) llm: LlmService,
    pub(crate) stream_tx: mpsc::UnboundedSender<(u64, StreamEvent)>,
    pub(crate) stream_rx: mpsc::UnboundedReceiver<(u64, StreamEvent)>,
    pub(crate) subagent_rx: mpsc::UnboundedReceiver<SubagentUiEvent>,
    pub(crate) reply_driver: ReplyDriver,
    pub(crate) tick_rate: Duration,
    pub(crate) last_tick: Instant,
}

pub(crate) fn bootstrap_tui(config: AppConfig, startup: StartupOptions) -> Result<TuiBootstrap> {
    let runtime = Runtime::new()?;
    let mut app = App::with_startup(
        config.ui.show_thinking,
        config.ui.show_tool_output,
        config.azure.model_name.clone(),
        config.azure.reasoning_effort,
        config.planning.agents.clone(),
        startup.access_mode,
        startup.approval_mode,
    );
    app.state_mut().session.safety_model_name = config.safety.model_name.clone();
    app.state_mut().session.safety_reasoning_effort = config.safety.reasoning_effort;
    let stats = StatsStore::new();
    let (subagent_tx, subagent_rx) = mpsc::unbounded_channel();
    let subagents =
        SubagentManager::new(config.subagents.max_concurrent, subagent_tx, stats.clone());
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
    let llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            &config,
            AgentContext::main(query::mode(app.state())),
            WriteApprovalController::new(startup.approval_mode),
            Some(AskUserController::default()),
            Some(subagents.clone()),
        )?
    };
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();

    Ok(TuiBootstrap {
        runtime,
        config,
        app,
        stats,
        subagents,
        command_history,
        llm,
        stream_tx,
        stream_rx,
        subagent_rx,
        reply_driver: ReplyDriver::default(),
        tick_rate: Duration::from_millis(125),
        last_tick: Instant::now(),
    })
}

pub(crate) struct HeadlessBootstrap {
    pub(crate) runtime: Runtime,
    pub(crate) stats: StatsStore,
    pub(crate) stream_rx: mpsc::UnboundedReceiver<(u64, StreamEvent)>,
    pub(crate) task: tokio::task::JoinHandle<()>,
}

pub(crate) fn bootstrap_headless(
    config: &AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<HeadlessBootstrap> {
    let runtime = Runtime::new()?;
    let stats = StatsStore::new();
    let llm = {
        let _guard = runtime.enter();
        LlmService::from_config(
            config,
            AgentContext::main(startup.access_mode),
            WriteApprovalController::new(startup.approval_mode),
            None,
            None,
        )?
    };
    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let stats_hook = stats.hook_for_model(config.azure.model_name.clone());
    let task = runtime.spawn({
        let llm = llm.clone();
        async move {
            llm.stream_prompt(1, prompt, Vec::new(), None, stats_hook, stream_tx)
                .await;
        }
    });

    Ok(HeadlessBootstrap {
        runtime,
        stats,
        stream_rx,
        task,
    })
}
