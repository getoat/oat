use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use serde::Serialize;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
    time::{Instant, sleep_until},
};

use crate::{
    agent::AgentContext,
    app::{AccessMode, CommandRisk},
    completion_request::CompletionRequestSnapshot,
    config::AppConfig,
    llm::{CompletionCapture, LlmService, PromptRunResult, StreamEvent, WriteApprovalController},
    stats::StatsStore,
    token_counting::count_text_tokens,
};

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
const SUBAGENT_FAILURE_LOG_DIR_RELATIVE_PATH: &str = ".config/oat/subagent_failures";
const SUBAGENT_FAILURE_LOG_SCHEMA_VERSION: u32 = 2;

#[derive(Clone)]
pub struct SubagentManager {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<State>,
    notify_tx: watch::Sender<u64>,
    ui_tx: mpsc::UnboundedSender<SubagentUiEvent>,
    stats: StatsStore,
}

struct State {
    next_id: u64,
    max_concurrent: usize,
    generation: u64,
    records: HashMap<String, SubagentRecord>,
    tasks: HashMap<String, JoinHandle<()>>,
}

#[derive(Clone, Debug)]
struct SubagentRecord {
    id: String,
    status: SubagentStatus,
    access_mode: AccessMode,
    #[allow(dead_code)]
    model_name: String,
    latest_tool_name: Option<String>,
    output: Option<String>,
    error: Option<String>,
    failure_log_path: Option<String>,
    last_activity_at: Instant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubagentActivityKind {
    General,
    Planning { model_name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubagentUiEvent {
    Spawned {
        id: String,
        access_mode: AccessMode,
        activity_kind: SubagentActivityKind,
    },
    Updated {
        id: String,
        latest_tool_name: Option<String>,
    },
    Completed {
        id: String,
    },
    Failed {
        id: String,
        error: String,
        log_path: Option<String>,
    },
    Cancelled {
        id: String,
    },
    WriteApprovalRequested {
        id: String,
        request_id: String,
        tool_name: String,
        arguments: String,
    },
    ShellApprovalRequested {
        id: String,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    },
}

#[derive(Clone)]
pub struct SubagentSpawnRequest {
    pub prompt: String,
    pub access_mode: AccessMode,
    pub activity_kind: SubagentActivityKind,
    pub model_name_override: Option<String>,
    pub config: AppConfig,
    pub approvals: WriteApprovalController,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubagentSnapshot {
    pub id: String,
    pub status: SubagentStatus,
    pub access_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_log_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WaitResult {
    pub completed_id: Option<String>,
    pub failed_id: Option<String>,
    pub cancelled_id: Option<String>,
    pub inactive_id: Option<String>,
    pub timed_out_on_inactivity: bool,
    pub subagents: Vec<SubagentSnapshot>,
}

#[derive(Debug, Serialize)]
struct SubagentFailureLog {
    schema_version: u32,
    subagent_id: String,
    failed_at_unix_ms: u64,
    model_name: String,
    access_mode: String,
    prompt: String,
    raw_error: String,
    normalized_error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    failing_request: Option<CompletionRequestSnapshot>,
}

struct SubagentExecutionFailure {
    raw_error: String,
    failing_request: Option<CompletionRequestSnapshot>,
}

impl SubagentManager {
    pub fn new(
        max_concurrent: usize,
        ui_tx: mpsc::UnboundedSender<SubagentUiEvent>,
        stats: StatsStore,
    ) -> Self {
        let (notify_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    next_id: 1,
                    max_concurrent,
                    generation: 0,
                    records: HashMap::new(),
                    tasks: HashMap::new(),
                }),
                notify_tx,
                ui_tx,
                stats,
            }),
        }
    }

    pub async fn spawn(&self, request: SubagentSpawnRequest) -> Result<SubagentSnapshot> {
        let id = self.register_running(
            request.access_mode,
            request
                .model_name_override
                .clone()
                .unwrap_or_else(|| request.config.azure.model_name.clone()),
        )?;

        let _ = self.inner.ui_tx.send(SubagentUiEvent::Spawned {
            id: id.clone(),
            access_mode: request.access_mode,
            activity_kind: request.activity_kind.clone(),
        });

        let manager = self.clone();
        let spawned_id = id.clone();
        let request_for_failure = request.clone();
        let (start_tx, start_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            if start_rx.await.is_err() {
                return;
            }
            let result = manager
                .run_spawned_subagent(spawned_id.clone(), request)
                .await;

            match result {
                Ok(output) => manager.mark_completed(&spawned_id, output),
                Err(error) => manager.fail_subagent(&spawned_id, &request_for_failure, error),
            }
        });
        self.insert_task_handle(&id, handle);
        let _ = start_tx.send(());

        self.inspect(&id)
    }

    pub async fn cancel_all_running(&self, emit_ui_events: bool) -> Vec<String> {
        let (cancelled_ids, tasks) = {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            let cancelled_ids = state
                .records
                .values()
                .filter(|record| record.status == SubagentStatus::Running)
                .map(|record| record.id.clone())
                .collect::<Vec<_>>();
            let mut cancelled_ids = cancelled_ids;
            cancelled_ids.sort();

            if cancelled_ids.is_empty() {
                return Vec::new();
            }

            for id in &cancelled_ids {
                if let Some(record) = state.records.get_mut(id) {
                    record.status = SubagentStatus::Cancelled;
                    record.output = None;
                    record.error = None;
                    record.failure_log_path = None;
                    record.last_activity_at = Instant::now();
                }
            }

            let tasks = cancelled_ids
                .iter()
                .filter_map(|id| state.tasks.remove(id).map(|handle| (id.clone(), handle)))
                .collect::<Vec<_>>();

            self.bump_generation(&mut state);
            (cancelled_ids, tasks)
        };

        if emit_ui_events {
            for id in &cancelled_ids {
                let _ = self
                    .inner
                    .ui_tx
                    .send(SubagentUiEvent::Cancelled { id: id.clone() });
            }
        }

        for (_, handle) in &tasks {
            handle.abort();
        }

        for (_, handle) in tasks {
            let _ = handle.await;
        }

        cancelled_ids
    }

    pub fn inspect(&self, id: &str) -> Result<SubagentSnapshot> {
        let state = self.inner.state.lock().expect("subagent state lock");
        let record = state
            .records
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Unknown subagent `{id}`."))?;
        Ok(snapshot_from_record(record))
    }

    pub async fn wait(&self, ids: &[String], timeout: Option<Duration>) -> Result<WaitResult> {
        if ids.is_empty() {
            bail!("ids must contain at least one subagent id");
        }

        let inactivity_timeout =
            timeout.unwrap_or_else(|| Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS));
        if inactivity_timeout.is_zero() {
            bail!("timeout_ms must be greater than 0");
        }

        let mut rx = self.inner.notify_tx.subscribe();
        loop {
            let snapshot = self.wait_state_snapshot(ids, inactivity_timeout)?;
            if snapshot.is_terminal() {
                return Ok(snapshot.into_result());
            }

            let deadline = snapshot.deadline.expect("deadline for running subagents");
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Ok(self.wait_state_snapshot(ids, inactivity_timeout)?.into_result());
                    }
                }
                _ = sleep_until(deadline) => {
                    let timed_out = self.wait_state_snapshot(ids, inactivity_timeout)?;
                    if let Some(inactive_id) = timed_out.inactive_id.clone() {
                        return Ok(timed_out.into_result_with_inactive(inactive_id));
                    }
                }
            }
        }
    }

    async fn run_spawned_subagent(
        &self,
        id: String,
        request: SubagentSpawnRequest,
    ) -> std::result::Result<String, SubagentExecutionFailure> {
        let context =
            AgentContext::subagent(request.access_mode, request.model_name_override.clone());
        let service = LlmService::from_config(
            &request.config,
            context,
            request.approvals.clone(),
            None,
            Some(self.clone()),
        )
        .map_err(|error| SubagentExecutionFailure {
            raw_error: error.to_string(),
            failing_request: None,
        })?;
        let stats_hook = self.inner.stats.hook_for_model(
            request
                .model_name_override
                .clone()
                .unwrap_or_else(|| request.config.azure.model_name.clone()),
        );
        let subagent_id = id.clone();
        let callback_manager = self.clone();
        let callback = Arc::new(move |_reply_id: u64, event: StreamEvent| {
            callback_manager.handle_stream_event(&subagent_id, event);
            true
        });
        let capture = CompletionCapture::new();

        let result = service
            .run_prompt(
                1,
                request.prompt,
                Vec::new(),
                stats_hook,
                Some(capture.clone()),
                callback,
            )
            .await;

        match result {
            Ok(PromptRunResult { output, .. }) => Ok(output),
            Err(error) => Err(SubagentExecutionFailure {
                raw_error: error.to_string(),
                failing_request: capture.snapshot(),
            }),
        }
    }

    fn handle_stream_event(&self, id: &str, event: StreamEvent) {
        match event {
            StreamEvent::TextDelta(_)
            | StreamEvent::ReasoningDelta(_)
            | StreamEvent::ToolResult { .. }
            | StreamEvent::AskUserRequested { .. }
            | StreamEvent::PlanningFinalizationStarted
            | StreamEvent::Finished { .. } => {
                self.mark_activity(id);
            }
            StreamEvent::ToolCall { name, .. } => {
                self.record_tool_activity(id, name);
            }
            StreamEvent::WriteApprovalRequested {
                request_id,
                tool_name,
                arguments,
            } => {
                if self.record_tool_activity(id, tool_name.clone()) {
                    let _ = self
                        .inner
                        .ui_tx
                        .send(SubagentUiEvent::WriteApprovalRequested {
                            id: id.to_string(),
                            request_id,
                            tool_name,
                            arguments,
                        });
                }
            }
            StreamEvent::ShellApprovalRequested {
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
            } => {
                if self.record_tool_activity(id, "RunShellScript".into()) {
                    let _ = self
                        .inner
                        .ui_tx
                        .send(SubagentUiEvent::ShellApprovalRequested {
                            id: id.to_string(),
                            request_id,
                            risk,
                            risk_explanation,
                            command,
                            working_directory,
                            reason,
                        });
                }
            }
            StreamEvent::Failed(_) => {
                self.mark_activity(id);
            }
        }
    }

    fn register_running(&self, access_mode: AccessMode, model_name: String) -> Result<String> {
        let now = Instant::now();
        let mut state = self.inner.state.lock().expect("subagent state lock");
        let active_count = state
            .records
            .values()
            .filter(|record| record.status == SubagentStatus::Running)
            .count();
        if active_count >= state.max_concurrent {
            bail!(
                "Subagent limit reached: {} running, max {}.",
                active_count,
                state.max_concurrent
            );
        }

        let id = format!("subagent-{}", state.next_id);
        state.next_id += 1;
        state.records.insert(
            id.clone(),
            SubagentRecord {
                id: id.clone(),
                status: SubagentStatus::Running,
                access_mode,
                model_name,
                latest_tool_name: None,
                output: None,
                error: None,
                failure_log_path: None,
                last_activity_at: now,
            },
        );
        self.bump_generation(&mut state);
        Ok(id)
    }

    fn insert_task_handle(&self, id: &str, handle: JoinHandle<()>) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if state
            .records
            .get(id)
            .is_some_and(|record| record.status == SubagentStatus::Running)
        {
            state.tasks.insert(id.to_string(), handle);
        }
    }

    fn mark_activity(&self, id: &str) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id)
            && record.status == SubagentStatus::Running
        {
            record.last_activity_at = Instant::now();
            self.bump_generation(&mut state);
            return true;
        }

        false
    }

    fn record_tool_activity(&self, id: &str, tool_name: String) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id)
            && record.status == SubagentStatus::Running
        {
            record.last_activity_at = Instant::now();
            record.latest_tool_name = Some(tool_name.clone());
            self.bump_generation(&mut state);
            drop(state);
            let _ = self.inner.ui_tx.send(SubagentUiEvent::Updated {
                id: id.to_string(),
                latest_tool_name: Some(tool_name),
            });
            return true;
        }

        false
    }

    fn mark_completed(&self, id: &str, output: String) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id) {
            if record.status != SubagentStatus::Running {
                return;
            }
            record.status = SubagentStatus::Completed;
            record.output = Some(output);
            record.error = None;
            record.failure_log_path = None;
            record.last_activity_at = Instant::now();
            state.tasks.remove(id);
            self.bump_generation(&mut state);
            let _ = self
                .inner
                .ui_tx
                .send(SubagentUiEvent::Completed { id: id.to_string() });
        }
    }

    #[cfg(test)]
    fn mark_failed(&self, id: &str, error: String, failure_log_path: Option<String>) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id) {
            if record.status != SubagentStatus::Running {
                return;
            }
            record.status = SubagentStatus::Failed;
            record.output = None;
            record.error = Some(error.clone());
            record.failure_log_path = failure_log_path.clone();
            record.last_activity_at = Instant::now();
            state.tasks.remove(id);
            self.bump_generation(&mut state);
            let _ = self.inner.ui_tx.send(SubagentUiEvent::Failed {
                id: id.to_string(),
                error,
                log_path: failure_log_path,
            });
        }
    }

    fn fail_subagent(
        &self,
        id: &str,
        request: &SubagentSpawnRequest,
        failure: SubagentExecutionFailure,
    ) {
        let normalized_error = normalize_subagent_failure(&failure.raw_error);
        {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            let Some(record) = state.records.get_mut(id) else {
                return;
            };
            if record.status != SubagentStatus::Running {
                return;
            }

            record.status = SubagentStatus::Failed;
            record.output = None;
            record.error = Some(normalized_error.clone());
            record.failure_log_path = None;
            record.last_activity_at = Instant::now();
            state.tasks.remove(id);
            self.bump_generation(&mut state);
        }

        let failure_log_path = persist_subagent_failure_log(
            default_subagent_failure_log_dir().as_deref(),
            &SubagentFailureLog {
                schema_version: SUBAGENT_FAILURE_LOG_SCHEMA_VERSION,
                subagent_id: id.to_string(),
                failed_at_unix_ms: unix_timestamp_ms(),
                model_name: request
                    .model_name_override
                    .clone()
                    .unwrap_or_else(|| request.config.azure.model_name.clone()),
                access_mode: request.access_mode.label().to_ascii_lowercase(),
                prompt: request.prompt.clone(),
                raw_error: failure.raw_error,
                normalized_error: normalized_error.clone(),
                failing_request: failure.failing_request,
            },
        )
        .ok()
        .flatten()
        .map(|path| path.display().to_string());

        {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            if let Some(record) = state.records.get_mut(id)
                && record.status == SubagentStatus::Failed
            {
                record.failure_log_path = failure_log_path.clone();
            }
        }
        let _ = self.inner.ui_tx.send(SubagentUiEvent::Failed {
            id: id.to_string(),
            error: normalized_error,
            log_path: failure_log_path,
        });
    }

    fn wait_state_snapshot(
        &self,
        ids: &[String],
        inactivity_timeout: Duration,
    ) -> Result<WaitStateSnapshot> {
        let state = self.inner.state.lock().expect("subagent state lock");
        let mut snapshots = Vec::with_capacity(ids.len());
        let now = Instant::now();
        let mut completed_id = None;
        let mut failed_id = None;
        let mut cancelled_id = None;
        let mut inactive_id = None;
        let mut deadline = None;

        for id in ids {
            let record = state
                .records
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow!("Unknown subagent `{id}`."))?;
            match record.status {
                SubagentStatus::Completed if completed_id.is_none() => {
                    completed_id = Some(record.id.clone());
                }
                SubagentStatus::Failed if failed_id.is_none() => {
                    failed_id = Some(record.id.clone());
                }
                SubagentStatus::Cancelled if cancelled_id.is_none() => {
                    cancelled_id = Some(record.id.clone());
                }
                SubagentStatus::Running => {
                    let record_deadline = record.last_activity_at + inactivity_timeout;
                    if record_deadline <= now && inactive_id.is_none() {
                        inactive_id = Some(record.id.clone());
                    }
                    deadline = Some(match deadline {
                        Some(current) if current <= record_deadline => current,
                        _ => record_deadline,
                    });
                }
                _ => {}
            }
            snapshots.push(snapshot_from_record(record));
        }

        Ok(WaitStateSnapshot {
            subagents: snapshots,
            completed_id,
            failed_id,
            cancelled_id,
            inactive_id,
            deadline,
        })
    }

    fn bump_generation(&self, state: &mut State) {
        state.generation = state.generation.wrapping_add(1);
        let _ = self.inner.notify_tx.send(state.generation);
    }

    #[cfg(test)]
    pub(crate) fn register_running_for_test(&self, id: &str, last_activity_ago: Duration) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        state.records.insert(
            id.to_string(),
            SubagentRecord {
                id: id.to_string(),
                status: SubagentStatus::Running,
                access_mode: AccessMode::ReadOnly,
                model_name: "gpt-5.4-mini".into(),
                latest_tool_name: None,
                output: None,
                error: None,
                failure_log_path: None,
                last_activity_at: Instant::now() - last_activity_ago,
            },
        );
        self.bump_generation(&mut state);
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(&self, id: &str, output: &str) {
        self.mark_completed(id, output.to_string());
    }

    #[cfg(test)]
    pub(crate) fn fail_for_test(&self, id: &str, error: &str) {
        self.mark_failed(id, error.to_string(), None);
    }

    #[cfg(test)]
    pub(crate) async fn cancel_all_running_for_test(&self) -> Vec<String> {
        self.cancel_all_running(true).await
    }

    #[cfg(test)]
    pub(crate) fn mark_activity_for_test(&self, id: &str) {
        self.mark_activity(id);
    }
}

struct WaitStateSnapshot {
    subagents: Vec<SubagentSnapshot>,
    completed_id: Option<String>,
    failed_id: Option<String>,
    cancelled_id: Option<String>,
    inactive_id: Option<String>,
    deadline: Option<Instant>,
}

pub fn estimate_prompt_tokens(prompt: &str) -> usize {
    count_text_tokens(prompt) as usize
}

pub fn normalize_subagent_failure(error: &str) -> String {
    if !error.contains("context_length_exceeded") && !error.contains("Input tokens exceed") {
        return error.to_string();
    }

    let limit = Regex::new(r"configured limit of (\d+) tokens")
        .ok()
        .and_then(|regex| regex.captures(error))
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<usize>().ok());
    let actual = Regex::new(r"resulted in (\d+) tokens")
        .ok()
        .and_then(|regex| regex.captures(error))
        .and_then(|captures| captures.get(1))
        .and_then(|value| value.as_str().parse::<usize>().ok());

    match (limit, actual) {
        (Some(limit), Some(actual)) => format!(
            "Subagent request exceeded the model context limit ({actual} tokens > {limit}). The delegated task likely accumulated too much history or tool output. Check the failure log for the captured request."
        ),
        (Some(limit), None) => format!(
            "Subagent request exceeded the model context limit ({limit} token limit). The delegated task likely accumulated too much history or tool output. Check the failure log for the captured request."
        ),
        _ => "Subagent request exceeded the model context limit. The delegated task likely accumulated too much history or tool output. Check the failure log for the captured request.".into(),
    }
}

fn default_subagent_failure_log_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(SUBAGENT_FAILURE_LOG_DIR_RELATIVE_PATH))
}

fn persist_subagent_failure_log(
    log_dir: Option<&Path>,
    entry: &SubagentFailureLog,
) -> Result<Option<PathBuf>> {
    let Some(log_dir) = log_dir else {
        return Ok(None);
    };

    fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;

    let path = log_dir.join(format!(
        "{}-{}.json",
        entry.failed_at_unix_ms, entry.subagent_id
    ));
    let tmp_path = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(entry).with_context(|| {
        format!(
            "failed to serialize subagent failure log for {}",
            entry.subagent_id
        )
    })?;

    fs::write(&tmp_path, payload)
        .with_context(|| format!("failed to write {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "failed to move {} into place at {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(Some(path))
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after epoch")
        .as_millis() as u64
}

impl WaitStateSnapshot {
    fn is_terminal(&self) -> bool {
        self.completed_id.is_some() || self.failed_id.is_some() || self.cancelled_id.is_some()
    }

    fn into_result(self) -> WaitResult {
        WaitResult {
            completed_id: self.completed_id,
            failed_id: self.failed_id,
            cancelled_id: self.cancelled_id,
            inactive_id: None,
            timed_out_on_inactivity: false,
            subagents: self.subagents,
        }
    }

    fn into_result_with_inactive(self, inactive_id: String) -> WaitResult {
        WaitResult {
            completed_id: self.completed_id,
            failed_id: self.failed_id,
            cancelled_id: self.cancelled_id,
            inactive_id: Some(inactive_id),
            timed_out_on_inactivity: true,
            subagents: self.subagents,
        }
    }
}

fn snapshot_from_record(record: SubagentRecord) -> SubagentSnapshot {
    SubagentSnapshot {
        id: record.id,
        status: record.status,
        access_mode: record.access_mode.label().to_ascii_lowercase(),
        latest_tool_name: record.latest_tool_name,
        output: record.output,
        error: record.error,
        failure_log_path: record.failure_log_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::time::advance;

    fn manager(
        max_concurrent: usize,
    ) -> (SubagentManager, mpsc::UnboundedReceiver<SubagentUiEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            SubagentManager::new(max_concurrent, tx, StatsStore::new()),
            rx,
        )
    }

    #[test]
    fn inspect_returns_running_snapshot() {
        let (manager, _rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));

        let snapshot = manager.inspect("subagent-1").expect("inspect succeeds");

        assert_eq!(snapshot.status, SubagentStatus::Running);
        assert_eq!(snapshot.id, "subagent-1");
    }

    #[tokio::test(start_paused = true)]
    async fn wait_returns_completed_subagent_immediately() {
        let (manager, _rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        manager.complete_for_test("subagent-1", "done");

        let result = manager
            .wait(&["subagent-1".into()], Some(Duration::from_secs(30)))
            .await
            .expect("wait succeeds");

        assert_eq!(result.completed_id.as_deref(), Some("subagent-1"));
        assert!(!result.timed_out_on_inactivity);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_times_out_after_inactivity() {
        let (manager, _rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));

        let wait = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .wait(&["subagent-1".into()], Some(Duration::from_millis(100)))
                    .await
            }
        });

        advance(Duration::from_millis(101)).await;
        let result = wait.await.expect("join").expect("wait succeeds");

        assert_eq!(result.inactive_id.as_deref(), Some("subagent-1"));
        assert!(result.timed_out_on_inactivity);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_resets_timeout_on_activity() {
        let (manager, _rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));

        let wait = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .wait(&["subagent-1".into()], Some(Duration::from_millis(100)))
                    .await
            }
        });

        advance(Duration::from_millis(80)).await;
        manager.mark_activity_for_test("subagent-1");
        advance(Duration::from_millis(80)).await;
        assert!(!wait.is_finished());

        manager.fail_for_test("subagent-1", "boom");
        let result = wait.await.expect("join").expect("wait succeeds");
        assert_eq!(result.failed_id.as_deref(), Some("subagent-1"));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_returns_cancelled_subagent_immediately() {
        let (manager, _rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        manager.cancel_all_running_for_test().await;

        let result = manager
            .wait(&["subagent-1".into()], Some(Duration::from_secs(30)))
            .await
            .expect("wait succeeds");

        assert_eq!(result.cancelled_id.as_deref(), Some("subagent-1"));
        assert!(!result.timed_out_on_inactivity);
    }

    #[test]
    fn prompt_token_estimate_uses_tokenizer() {
        assert_eq!(
            estimate_prompt_tokens("Count tokens with the shared tokenizer."),
            count_text_tokens("Count tokens with the shared tokenizer.") as usize
        );
    }

    #[test]
    fn tool_activity_updates_snapshot_and_emits_ui_event() {
        let (manager, mut rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));

        manager.handle_stream_event(
            "subagent-1",
            StreamEvent::ToolCall {
                name: "Grep".into(),
                arguments: "{}".into(),
            },
        );

        assert_eq!(
            rx.try_recv().expect("ui event"),
            SubagentUiEvent::Updated {
                id: "subagent-1".into(),
                latest_tool_name: Some("Grep".into()),
            }
        );
        assert_eq!(
            manager
                .inspect("subagent-1")
                .expect("inspect succeeds")
                .latest_tool_name
                .as_deref(),
            Some("Grep")
        );
    }

    #[tokio::test]
    async fn cancelling_running_subagents_marks_them_cancelled_and_emits_ui_events() {
        let (manager, mut rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        manager.register_running_for_test("subagent-2", Duration::from_secs(0));

        let cancelled = manager.cancel_all_running_for_test().await;

        assert_eq!(
            cancelled,
            vec!["subagent-1".to_string(), "subagent-2".to_string()]
        );
        assert_eq!(
            manager
                .inspect("subagent-1")
                .expect("inspect succeeds")
                .status,
            SubagentStatus::Cancelled
        );
        assert_eq!(
            rx.try_recv().expect("first event"),
            SubagentUiEvent::Cancelled {
                id: "subagent-1".into(),
            }
        );
        assert_eq!(
            rx.try_recv().expect("second event"),
            SubagentUiEvent::Cancelled {
                id: "subagent-2".into(),
            }
        );
    }

    #[tokio::test]
    async fn cancelled_subagents_ignore_late_tool_activity() {
        let (manager, mut rx) = manager(4);
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        manager.cancel_all_running_for_test().await;
        assert_eq!(
            rx.try_recv().expect("cancelled event"),
            SubagentUiEvent::Cancelled {
                id: "subagent-1".into(),
            }
        );

        manager.handle_stream_event(
            "subagent-1",
            StreamEvent::ToolCall {
                name: "Grep".into(),
                arguments: "{}".into(),
            },
        );

        assert!(rx.try_recv().is_err());
        assert!(
            manager
                .inspect("subagent-1")
                .expect("inspect succeeds")
                .latest_tool_name
                .is_none()
        );
    }

    #[test]
    fn context_length_failures_are_normalized() {
        let message = normalize_subagent_failure(
            "CompletionError: ProviderError: Invalid status code 400 Bad Request with message: {\"error\":{\"message\":\"Input tokens exceed the configured limit of 272000 tokens. Your messages resulted in 801427 tokens. Please reduce the length of the messages.\",\"type\":\"invalid_request_error\",\"param\":\"messages\",\"code\":\"context_length_exceeded\"}}",
        );

        assert!(message.contains("801427 tokens > 272000"));
        assert!(message.contains("captured request"));
    }

    #[test]
    fn failure_logs_are_persisted_as_json() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("oat-subagent-failures-{unique}"));
        let entry = SubagentFailureLog {
            schema_version: SUBAGENT_FAILURE_LOG_SCHEMA_VERSION,
            subagent_id: "subagent-7".into(),
            failed_at_unix_ms: 123,
            model_name: "gpt-5.4".into(),
            access_mode: "read-only".into(),
            prompt: "inspect src".into(),
            raw_error: "raw".into(),
            normalized_error: "normalized".into(),
            failing_request: Some(CompletionRequestSnapshot::capture(
                &rig::completion::Message::user("latest prompt"),
                &[rig::completion::Message::assistant("history item")],
            )),
        };

        let path = persist_subagent_failure_log(Some(&dir), &entry)
            .expect("persist succeeds")
            .expect("path returned");
        let payload = fs::read_to_string(&path).expect("payload readable");

        assert!(payload.contains("\"subagent_id\": \"subagent-7\""));
        assert!(payload.contains("\"normalized_error\": \"normalized\""));
        assert!(payload.contains("\"failing_request\""));
        assert!(payload.contains("\"latest prompt\""));

        let _ = fs::remove_dir_all(&dir);
    }
}
