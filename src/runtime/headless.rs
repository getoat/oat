use std::{
    error::Error,
    sync::{Arc, Mutex},
};

use anyhow::{Context, anyhow};

use crate::{
    StartupOptions,
    agent::AgentContext,
    app::{SessionHistoryMessage, SessionProfile, StreamEvent, TurnEndReason},
    config::AppConfig,
    features::planning::{
        PlanningReply, accepted_plan_implementation_prompt, parse_planning_reply,
        planning_conversation_prompt_headless, planning_finalization_prompt_headless,
        run_planning_workflow,
    },
    llm::{
        EventCallback, LlmService, WriteApprovalController, history_from_rig, history_into_rig,
        run_internal_plain_prompt,
    },
    model_registry::{self, ModelProvider},
};

use super::bootstrap::{HeadlessBootstrap, bootstrap_headless};

pub(crate) fn run_headless(
    config: AppConfig,
    startup: StartupOptions,
    prompt: String,
) -> Result<String, Box<dyn Error>> {
    ensure_headless_codex_auth(
        &config,
        [
            config.model.model_name.as_str(),
            config.critic.model_name.as_str(),
        ],
    )?;

    let runtime = bootstrap_headless(&config, startup)?;
    let result = runtime
        .runtime
        .block_on(async {
            run_headless_with_critic_loop(&runtime, startup, prompt, Vec::new(), None).await
        })
        .context("headless request failed")?;
    shutdown_headless(runtime)?;
    Ok(result)
}

async fn run_headless_with_critic_loop(
    runtime: &HeadlessBootstrap,
    startup: StartupOptions,
    initial_prompt: String,
    initial_history: Vec<SessionHistoryMessage>,
    history_model_name: Option<String>,
) -> anyhow::Result<String> {
    let mut task_state = HeadlessTaskState::default();
    let mut prompt = initial_prompt.clone();
    let mut history = initial_history;
    let mut history_model = history_model_name;
    #[allow(unused_assignments)]
    let mut last_output = String::new();
    let max_retries = runtime.config.critic.max_retries;
    let critic_enabled = runtime.config.critic.enabled;
    loop {
        let reply = run_prompt_and_collect(
            &runtime.llm,
            &runtime.stats,
            1,
            prompt.clone(),
            history.clone(),
            history_model.clone(),
        )
        .await?;
        task_state.apply_updates(&reply.task_updates);
        if !critic_enabled {
            return Ok(reply.output);
        }
        if task_state.current_task.is_none() {
            task_state.current_task = Some(synthetic_headless_task(&initial_prompt));
        }
        let Some(task) = task_state.current_task.clone() else {
            return Ok(reply.output);
        };
        if task.criteria.is_empty() {
            return Ok(reply.output);
        }
        if task_state.retry_count >= max_retries {
            return Ok(reply.output);
        }
        let critic_llm = build_headless_critic_llm(runtime, startup)?;
        let emit: EventCallback = Arc::new(|_, _| true);
        let verdict = crate::llm::run_agentic_critic(
            &critic_llm,
            2,
            &task,
            runtime.config.critic.max_tool_steps,
            runtime
                .stats
                .hook_for_model(runtime.config.critic.model_name.clone()),
            emit,
        )
        .await;
        match verdict {
            Ok(crate::llm::CriticVerdict::Done) => return Ok(reply.output),
            Ok(crate::llm::CriticVerdict::NotDone { feedback }) => {
                task_state.retry_count = task_state.retry_count.saturating_add(1);
                history = reply.history.clone().unwrap_or_else(|| history.clone());
                history_model = Some(runtime.config.model.model_name.clone());
                last_output = reply.output;
                prompt = format!(
                    "[critic feedback, retry {}/{}] {}",
                    task_state.retry_count, max_retries, feedback
                );
            }
            Err(error) => {
                eprintln!("critic call failed: {error}");
                return Ok(reply.output);
            }
        }
        if task_state.retry_count >= max_retries {
            return Ok(last_output);
        }
    }
}

pub(crate) fn run_headless_plan(
    config: AppConfig,
    startup: StartupOptions,
    prompt: String,
    auto_accept_plan: bool,
) -> Result<String, Box<dyn Error>> {
    ensure_headless_codex_auth(
        &config,
        std::iter::once(config.model.model_name.as_str())
            .chain(
                config
                    .planning
                    .agents
                    .iter()
                    .map(|agent| agent.model_name.as_str()),
            )
            .chain(std::iter::once(config.critic.model_name.as_str())),
    )?;

    let runtime = bootstrap_headless(&config, startup)?;
    let result = runtime.runtime.block_on(async {
        run_headless_plan_inner(&runtime, startup, prompt, auto_accept_plan).await
    });
    shutdown_headless(runtime)?;
    result.map_err(Into::into)
}

fn shutdown_headless(runtime: HeadlessBootstrap) -> anyhow::Result<()> {
    runtime.terminals.cancel_all_running();
    runtime.runtime.block_on(async {
        runtime.subagents.cancel_all_running(false).await;
        tokio::task::yield_now().await;
    });
    runtime.stats.finalize_current_session()?;
    Ok(())
}

async fn run_headless_plan_inner(
    runtime: &HeadlessBootstrap,
    startup: StartupOptions,
    prompt: String,
    auto_accept_plan: bool,
) -> anyhow::Result<String> {
    let planning_llm = build_headless_llm(runtime, startup, SessionProfile::Planning)
        .context("headless planning llm")?;
    let initial = run_prompt_and_collect(
        &planning_llm,
        &runtime.stats,
        1,
        planning_conversation_prompt_headless(&prompt),
        Vec::new(),
        None,
    )
    .await?;

    let PlanningReply::ReadyBrief(brief) = parse_planning_reply(&initial.output) else {
        return Err(anyhow!(
            "Headless planning did not produce a <planning_ready> block.\nVisible reply:\n{}",
            visible_reply_text(&initial.output)
        ));
    };

    let history = initial
        .history
        .clone()
        .ok_or_else(|| anyhow!("Headless planning did not return session history."))?;
    let planning_result = run_planning_finalization(
        runtime,
        &planning_llm,
        startup,
        brief.markdown,
        history,
        Some(runtime.config.model.model_name.clone()),
    )
    .await?;

    let PlanningReply::ProposedPlan(plan) = parse_planning_reply(&planning_result.output) else {
        return Err(anyhow!(
            "Headless planning finalization did not produce a <proposed_plan> block.\nVisible reply:\n{}",
            visible_reply_text(&planning_result.output)
        ));
    };

    if !auto_accept_plan {
        return Ok(plan.raw_block);
    }

    let implementation_prompt = accepted_plan_implementation_prompt(&plan.raw_block);
    let implementation_history = planning_result.history.clone().unwrap_or_default();
    let output = run_headless_with_critic_loop(
        runtime,
        startup,
        implementation_prompt,
        implementation_history,
        Some(runtime.config.model.model_name.clone()),
    )
    .await?;
    Ok(output)
}

async fn run_planning_finalization(
    runtime: &HeadlessBootstrap,
    planning_llm: &LlmService,
    startup: StartupOptions,
    description: String,
    history: Vec<SessionHistoryMessage>,
    history_model_name: Option<String>,
) -> anyhow::Result<CollectedHeadlessReply> {
    if runtime.config.planning.agents.is_empty() {
        let output = run_internal_plain_prompt(
            &runtime.config,
            &runtime.config.model.model_name,
            &planning_llm.preamble,
            runtime.config.model.reasoning,
            planning_finalization_prompt_headless(&description, &[], &[]),
            runtime
                .stats
                .hook_for_model(runtime.config.model.model_name.clone()),
        )
        .await?;
        return Ok(CollectedHeadlessReply {
            output,
            ..CollectedHeadlessReply::default()
        });
    }

    let history = history_into_rig(history)?;
    let collector = Arc::new(Mutex::new(CollectedHeadlessReply::default()));
    let failure = Arc::new(Mutex::new(None::<String>));
    let synthesize = {
        let llm = planning_llm.clone();
        let stats = runtime.stats.clone();
        let collector = collector.clone();
        Arc::new(move |prompt, history, history_model_name| {
            let llm = llm.clone();
            let stats = stats.clone();
            let collector = collector.clone();
            Box::pin(async move {
                let emit = collector_callback(2, collector);
                llm.run_prompt(
                    2,
                    prompt,
                    history_from_rig(history).map_err(|error| error.to_string())?,
                    history_model_name,
                    stats.hook_for_model(llm.model_name().to_string()),
                    None,
                    emit,
                )
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
            }) as crate::features::planning::PlanningSynthesisFuture
        })
    };

    let failure_for_callback = failure.clone();
    run_planning_workflow(
        2,
        description,
        history,
        history_model_name,
        startup.access_mode(),
        startup.full_system_access(),
        false,
        runtime.config.clone(),
        runtime.subagents.clone(),
        runtime.llm.approvals(),
        runtime.llm.shell_approvals(),
        runtime.llm.web_service(),
        Arc::new(|| {}),
        Arc::new(move |message| {
            *failure_for_callback
                .lock()
                .expect("planning failure callback lock") = Some(message);
        }),
        synthesize,
    )
    .await;

    if let Some(message) = failure.lock().expect("planning failure lock").clone() {
        return Err(anyhow!(message));
    }

    let collected = collector.lock().expect("planning collector lock").clone();
    if let Some(error) = collected.failure_message() {
        return Err(anyhow!(error));
    }
    Ok(collected)
}

async fn run_prompt_and_collect(
    llm: &LlmService,
    stats: &crate::stats::StatsStore,
    reply_id: u64,
    prompt: String,
    history: Vec<SessionHistoryMessage>,
    history_model_name: Option<String>,
) -> anyhow::Result<CollectedHeadlessReply> {
    run_prompt_and_collect_async(llm, stats, reply_id, prompt, history, history_model_name).await
}

async fn run_prompt_and_collect_async(
    llm: &LlmService,
    stats: &crate::stats::StatsStore,
    reply_id: u64,
    prompt: String,
    history: Vec<SessionHistoryMessage>,
    history_model_name: Option<String>,
) -> anyhow::Result<CollectedHeadlessReply> {
    let collector = Arc::new(Mutex::new(CollectedHeadlessReply::default()));
    let emit = collector_callback(reply_id, collector.clone());
    let stats_hook = stats.hook_for_model(llm.model_name().to_string());
    let result = llm
        .run_prompt(
            reply_id,
            prompt,
            history,
            history_model_name,
            stats_hook,
            None,
            emit,
        )
        .await;

    let mut collected = collector.lock().expect("headless collector lock").clone();
    if let Some(error) = collected.failure_message() {
        return Err(anyhow!(error));
    }

    match result {
        Ok(result) => {
            if collected.output.is_empty() {
                collected.output = result.output;
            }
            Ok(collected)
        }
        Err(error) => Err(anyhow!(error)),
    }
}

fn build_headless_llm(
    runtime: &HeadlessBootstrap,
    startup: StartupOptions,
    session_profile: SessionProfile,
) -> anyhow::Result<LlmService> {
    let _guard = runtime.runtime.enter();
    LlmService::from_config(
        &runtime.config,
        AgentContext::main_with_full_system_access(
            startup.access_mode(),
            startup.full_system_access(),
        ),
        session_profile,
        WriteApprovalController::new(startup.approval_mode),
        None,
        true,
        runtime
            .config
            .memory
            .enabled
            .then_some(runtime.memory.clone()),
        Some(runtime.subagents.clone()),
        Some(runtime.terminals.clone()),
        runtime.llm.web_service(),
    )
}

fn build_headless_critic_llm(
    runtime: &HeadlessBootstrap,
    startup: StartupOptions,
) -> anyhow::Result<LlmService> {
    let _guard = runtime.runtime.enter();
    LlmService::from_config(
        &runtime.config,
        AgentContext::critic_with_full_system_access(
            Some(runtime.config.critic.model_name.clone()),
            Some(runtime.config.critic.reasoning),
            startup.full_system_access(),
        ),
        SessionProfile::Normal,
        WriteApprovalController::new(startup.approval_mode),
        None,
        false,
        None,
        None,
        None,
        runtime.llm.web_service(),
    )
}

fn collector_callback(
    expected_reply_id: u64,
    collector: Arc<Mutex<CollectedHeadlessReply>>,
) -> EventCallback {
    Arc::new(move |reply_id, event| {
        if reply_id != expected_reply_id {
            return true;
        }

        collector
            .lock()
            .expect("headless collector lock")
            .record(event)
    })
}

fn ensure_headless_codex_auth<'a>(
    config: &AppConfig,
    model_names: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<()> {
    let missing_codex_auth = model_names.into_iter().any(|model_name| {
        matches!(
            model_registry::find_model(model_name).map(|model| model.provider),
            Some(ModelProvider::Codex)
        ) && !config
            .codex
            .as_ref()
            .is_some_and(|codex| codex.is_authenticated())
    });

    if missing_codex_auth {
        Err(anyhow!(
            "Headless Codex requests require authenticating from the TUI first with `/login`."
        ))
    } else {
        Ok(())
    }
}

fn visible_reply_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "(empty reply)".into()
    } else {
        trimmed.to_string()
    }
}

#[derive(Debug, Clone, Default)]
struct CollectedHeadlessReply {
    output: String,
    history: Option<Vec<SessionHistoryMessage>>,
    runtime_error: Option<String>,
    task_updates: Vec<crate::tools::TaskUpdate>,
    turn_evidence: crate::task::TurnEvidence,
    tool_calls: Vec<(String, String)>,
}

impl CollectedHeadlessReply {
    fn record(&mut self, event: StreamEvent) -> bool {
        match event {
            StreamEvent::SessionTitleGenerated(_) => true,
            StreamEvent::TextDelta(delta) => {
                self.output.push_str(&delta);
                true
            }
            StreamEvent::Commentary(_)
            | StreamEvent::ReasoningDelta(_)
            | StreamEvent::HostedToolStarted { .. }
            | StreamEvent::HostedToolCompleted { .. }
            | StreamEvent::PlanningFinalizationStarted => true,
            StreamEvent::ToolCall { name, arguments } => {
                self.tool_calls.push((name, arguments));
                true
            }
            StreamEvent::ToolResult { name, output } => {
                self.record_tool_evidence(&name, &output);
                true
            }
            StreamEvent::TaskUpdated { update, .. } => {
                self.task_updates.push(update);
                true
            }
            StreamEvent::TodoSnapshot(_) => true,
            StreamEvent::CompactionFinished { .. } => true,
            StreamEvent::TurnEnded { reason, history } => {
                if reason == TurnEndReason::Completed {
                    self.history = history;
                }
                true
            }
            StreamEvent::Failed(error) => {
                self.runtime_error = Some(format!("Request failed: {error}"));
                true
            }
            StreamEvent::AskUserRequested { .. } => {
                self.runtime_error =
                    Some("Headless mode does not support AskUser interactions.".to_string());
                false
            }
            StreamEvent::WriteApprovalRequested { tool_name, .. } => {
                self.runtime_error = Some(format!(
                    "Headless mode cannot continue because `{tool_name}` requested write approval."
                ));
                false
            }
            StreamEvent::ShellApprovalRequested { command, .. } => {
                self.runtime_error = Some(format!(
                    "Headless mode cannot continue because shell command approval was requested: {command}"
                ));
                false
            }
        }
    }

    fn failure_message(&self) -> Option<String> {
        self.runtime_error.clone()
    }

    fn record_tool_evidence(&mut self, tool_name: &str, output: &str) {
        use crate::task::{
            CommandRun, FileTouch, FileTouchKind, MAX_CAPTURED_COMMAND_BYTES,
            truncate_output_head_tail,
        };
        let latest_args = self
            .tool_calls
            .iter()
            .rev()
            .find(|(name, _)| name == tool_name)
            .map(|(_, args)| args.clone());
        let args_json: Option<serde_json::Value> = latest_args
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok());
        match tool_name {
            crate::tools::WRITE_FILE_TOOL_NAME => {
                if let Some(args) = args_json {
                    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                        self.turn_evidence.record_file_touch(FileTouch {
                            path: path.to_string(),
                            kind: FileTouchKind::Written,
                            size_bytes: std::fs::metadata(path).ok().map(|m| m.len()),
                        });
                    }
                }
            }
            crate::tools::DELETE_PATH_TOOL_NAME => {
                if let Some(args) = args_json {
                    if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                        self.turn_evidence.record_file_touch(FileTouch {
                            path: path.to_string(),
                            kind: FileTouchKind::Deleted,
                            size_bytes: None,
                        });
                    }
                }
            }
            crate::tools::APPLY_PATCH_TOOL_NAME => {
                if let Some(args) = args_json {
                    if let Some(patches) = args.get("patches").and_then(|v| v.as_array()) {
                        for patch in patches {
                            if let Some(path) = patch.get("path").and_then(|v| v.as_str()) {
                                self.turn_evidence.record_file_touch(FileTouch {
                                    path: path.to_string(),
                                    kind: FileTouchKind::Edited,
                                    size_bytes: std::fs::metadata(path).ok().map(|m| m.len()),
                                });
                            }
                        }
                    }
                }
            }
            crate::tools::RUN_SHELL_SCRIPT_TOOL_NAME => {
                let (exit_code, stdout, stderr) = parse_shell_output(output);
                let (stdout_head, stdout_tail) =
                    truncate_output_head_tail(&stdout, MAX_CAPTURED_COMMAND_BYTES);
                let (stderr_head, stderr_tail) =
                    truncate_output_head_tail(&stderr, MAX_CAPTURED_COMMAND_BYTES);
                let command = args_json
                    .as_ref()
                    .and_then(|v| v.get("script").and_then(|s| s.as_str()))
                    .unwrap_or("(script)")
                    .to_string();
                let working_dir = args_json
                    .as_ref()
                    .and_then(|v| v.get("working_directory").and_then(|s| s.as_str()))
                    .map(|s| s.to_string());
                self.turn_evidence.record_command(CommandRun {
                    command,
                    working_dir,
                    exit_code,
                    stdout_head,
                    stdout_tail,
                    stderr_head,
                    stderr_tail,
                });
            }
            _ => {}
        }
    }
}

use crate::task::parse_run_shell_script_output as parse_shell_output;

fn synthetic_headless_task(user_request: &str) -> crate::task::ActiveTask {
    let description = user_request
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(user_request)
        .trim()
        .chars()
        .take(200)
        .collect::<String>();
    let mut task = crate::task::ActiveTask::new(
        if description.is_empty() {
            "Fulfill the user's request".to_string()
        } else {
            description
        },
        vec![user_request.to_string()],
    );
    let id = task.allocate_criterion_id();
    task.criteria.push(crate::task::AcceptanceCriterion {
        id,
        text: "The user's stated goal, as described in the source messages, has been fully accomplished in this turn.".into(),
        verification_hint: "Compare the turn evidence (files touched, commands run with their exit codes and output, and the final assistant reply) against what the user asked for. Flag empty outputs, unfixed environment errors, or claims that are not supported by the command output.".into(),
    });
    task
}

#[derive(Debug, Clone, Default)]
struct HeadlessTaskState {
    current_task: Option<crate::task::ActiveTask>,
    retry_count: u8,
}

impl HeadlessTaskState {
    fn apply_updates(&mut self, updates: &[crate::tools::TaskUpdate]) {
        for update in updates {
            self.apply(update.clone());
        }
    }

    fn apply(&mut self, update: crate::tools::TaskUpdate) {
        use crate::tools::TaskUpdate;
        match update {
            TaskUpdate::Set {
                description,
                criteria,
            } => {
                let mut task = crate::task::ActiveTask::new(description, Vec::new());
                for draft in criteria {
                    let id = task.allocate_criterion_id();
                    task.criteria.push(draft.into_criterion(id));
                }
                self.current_task = Some(task);
                self.retry_count = 0;
            }
            TaskUpdate::Clear => {
                self.current_task = None;
                self.retry_count = 0;
            }
            TaskUpdate::AddCriterion {
                text,
                verification_hint,
            } => {
                if let Some(task) = self.current_task.as_mut() {
                    let id = task.allocate_criterion_id();
                    task.criteria.push(crate::task::AcceptanceCriterion {
                        id,
                        text,
                        verification_hint,
                    });
                }
            }
            TaskUpdate::UpdateCriterion {
                id,
                text,
                verification_hint,
            } => {
                if let Some(task) = self.current_task.as_mut() {
                    if let Some(criterion) = task.find_mut(id) {
                        if let Some(text) = text {
                            criterion.text = text;
                        }
                        if let Some(hint) = verification_hint {
                            criterion.verification_hint = hint;
                        }
                    }
                }
            }
            TaskUpdate::RemoveCriterion { id } => {
                if let Some(task) = self.current_task.as_mut() {
                    task.remove(id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::{Context, Result, anyhow};

    use crate::{
        StartupOptions,
        app::{HostedToolKind, SessionProfile},
        config::AppConfig,
        runtime::bootstrap::bootstrap_headless,
    };

    use super::{CollectedHeadlessReply, build_headless_llm, collector_callback};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        env::temp_dir().join(format!(
            "oat-headless-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let original = env::var_os(key);
            // SAFETY: test-only scoped environment mutation, restored on drop.
            unsafe { env::set_var(key, value) };
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                // SAFETY: restoring the process environment to its prior value.
                unsafe { env::set_var(self.key, value) };
            } else {
                // SAFETY: removing the temporary test override.
                unsafe { env::remove_var(self.key) };
            }
        }
    }

    fn write_live_search_config(temp_root: &Path) -> Result<PathBuf> {
        fs::create_dir_all(temp_root)
            .with_context(|| format!("failed to create {}", temp_root.display()))?;
        let source =
            fs::read_to_string("config.toml").context("failed to read repo config.toml")?;
        let mut value: toml::Value =
            toml::from_str(&source).context("failed to parse repo config.toml")?;
        let root = value
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected config root table"))?;

        let memory = root
            .entry("memory")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [memory] table"))?;
        memory.insert("enabled".into(), toml::Value::Boolean(false));

        let model = root
            .entry("model")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [model] table"))?;
        model.insert(
            "model_name".into(),
            toml::Value::String("gpt-5.4-mini".into()),
        );
        model.insert("reasoning".into(), toml::Value::String("medium".into()));

        let tools = root
            .entry("tools")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [tools] table"))?;
        let web_search = tools
            .entry("web_search")
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or_else(|| anyhow!("expected [tools.web_search] table"))?;
        web_search.insert("mode".into(), toml::Value::String("live".into()));

        let config_path = temp_root.join("config.toml");
        fs::write(
            &config_path,
            toml::to_string(&value).context("failed to serialize temp config")?,
        )
        .with_context(|| format!("failed to write {}", config_path.display()))?;

        Ok(config_path)
    }

    #[test]
    fn bootstrap_headless_exposes_non_interactive_autonomous_tools() -> Result<()> {
        let config = AppConfig::load_from_path(Path::new("config.example.toml"))?;
        let runtime = bootstrap_headless(&config, StartupOptions::dangerous())?;
        let tool_names = runtime.llm.tool_names();

        assert!(tool_names.iter().any(|name| name == "Todo"));
        assert!(tool_names.iter().any(|name| name == "SpawnSubagent"));
        assert!(
            tool_names
                .iter()
                .any(|name| name == "StartBackgroundTerminal")
        );
        assert!(!tool_names.iter().any(|name| name == "AskUser"));

        Ok(())
    }

    #[test]
    fn headless_planning_profile_hides_task_tools() -> Result<()> {
        let config = AppConfig::load_from_path(Path::new("config.example.toml"))?;
        let runtime = bootstrap_headless(&config, StartupOptions::dangerous())?;
        let planning_llm = build_headless_llm(
            &runtime,
            StartupOptions::dangerous(),
            SessionProfile::Planning,
        )?;
        let tool_names = planning_llm.tool_names();

        assert!(!tool_names.iter().any(|name| name == "SetCurrentTask"));
        assert!(!tool_names.iter().any(|name| name == "AddCriterion"));
        assert!(tool_names.iter().any(|name| name == "Todo"));

        Ok(())
    }

    #[test]
    fn bootstrap_headless_hides_memory_tools_when_memory_disabled() -> Result<()> {
        let mut config = AppConfig::load_from_path(Path::new("config.example.toml"))?;
        config.memory.enabled = false;

        let runtime = bootstrap_headless(&config, StartupOptions::dangerous())?;
        let tool_names = runtime.llm.tool_names();

        assert!(!tool_names.iter().any(|name| name == "SearchMemories"));
        assert!(!tool_names.iter().any(|name| name == "GetMemory"));

        Ok(())
    }

    #[test]
    #[ignore = "manual live test requiring provider credentials and network access"]
    fn live_responses_search_emits_hosted_tool_events() -> Result<()> {
        let temp_root = unique_temp_dir("live-search");
        let config_path = write_live_search_config(&temp_root)?;
        let _home = EnvVarGuard::set("HOME", &temp_root);
        let config = AppConfig::load_from_path(&config_path)?;
        let runtime = bootstrap_headless(&config, StartupOptions::default())?;

        let collector = Arc::new(Mutex::new(Vec::new()));
        let emit = {
            let collector = collector.clone();
            let base =
                collector_callback(1, Arc::new(Mutex::new(CollectedHeadlessReply::default())));
            Arc::new(move |reply_id, event: crate::app::StreamEvent| {
                if reply_id == 1 {
                    collector
                        .lock()
                        .expect("event collector lock")
                        .push(event.clone());
                }
                base(reply_id, event)
            })
        };
        let stats_hook = runtime
            .stats
            .hook_for_model(runtime.llm.model_name().to_string());
        let outcome = runtime.runtime.block_on(async {
            runtime
                .llm
                .run_prompt(
                    1,
                    concat!(
                        "Use web search before answering. After the search completes, ",
                        "reply with one short sentence confirming you finished searching."
                    )
                    .to_string(),
                    Vec::new(),
                    None,
                    stats_hook,
                    None,
                    emit,
                )
                .await
        });

        let result = outcome?;
        let events = collector.lock().expect("event collector lock");
        assert!(
            events.iter().any(|event| matches!(
                event,
                crate::app::StreamEvent::HostedToolStarted {
                    kind: HostedToolKind::Search,
                    ..
                }
            )),
            "no hosted web search start event observed. final response: {}",
            result.output
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                crate::app::StreamEvent::HostedToolCompleted {
                    kind: HostedToolKind::Search,
                    ..
                }
            )),
            "no hosted web search completion event observed. final response: {}",
            result.output
        );
        assert!(
            !result.output.trim().is_empty(),
            "final response was empty after hosted web search"
        );

        Ok(())
    }
}
