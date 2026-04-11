use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rig::{
    agent::{HookAction, PromptHook, ToolCallHookAction},
    completion::{CompletionModel, GetTokenUsage, Message, Usage},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{model_registry, token_counting::count_text_tokens};

const STATS_DIR_RELATIVE_PATH: &str = ".config/oat/stats";
const SCHEMA_VERSION: u32 = 5;
const TOOL_CALL_ERROR_PREFIX: &str = "ToolCallError:";
const NANOS_PER_USD: u64 = 1_000_000_000;
const TOKENS_PER_MILLION: u64 = 1_000_000;

#[derive(Debug)]
struct StatsState {
    stats_dir: Option<PathBuf>,
    current: SessionStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingTokenTotals {
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub available_request_count: u64,
    #[serde(default)]
    pub unavailable_request_count: u64,
    #[serde(default)]
    pub estimated_request_count: u64,
}

impl ThinkingTokenTotals {
    fn add(self, other: Self) -> Self {
        Self {
            tokens: self.tokens + other.tokens,
            available_request_count: self.available_request_count + other.available_request_count,
            unavailable_request_count: self.unavailable_request_count
                + other.unavailable_request_count,
            estimated_request_count: self.estimated_request_count + other.estimated_request_count,
        }
    }

    fn record(&mut self, thinking_tokens: Option<u64>, estimated: bool) {
        match thinking_tokens {
            Some(tokens) => {
                self.tokens += tokens;
                self.available_request_count += 1;
                if estimated {
                    self.estimated_request_count += 1;
                }
            }
            None => {
                self.unavailable_request_count += 1;
            }
        }
    }

    fn value_for_requests(self, request_count: u64) -> Option<u64> {
        if request_count == 0 {
            Some(0)
        } else if self.available_request_count == 0 {
            None
        } else {
            Some(self.tokens)
        }
    }

    fn is_partial_for_requests(self, request_count: u64) -> bool {
        request_count > 0 && self.available_request_count > 0 && self.unavailable_request_count > 0
    }

    fn is_estimated_for_requests(self, request_count: u64) -> bool {
        request_count > 0 && self.estimated_request_count > 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatsTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub thinking_tokens: ThinkingTokenTotals,
    #[serde(default)]
    pub estimated_cost_nanos_usd: u64,
    pub request_count: u64,
    pub tool_call_count: u64,
    pub tool_success_count: u64,
    pub tool_failure_count: u64,
    #[serde(default)]
    pub ttfb_total_millis: u64,
    #[serde(default)]
    pub ttfb_recorded_request_count: u64,
    #[serde(default)]
    pub total_request_millis: u64,
    #[serde(default)]
    pub timed_request_count: u64,
    #[serde(default)]
    pub completed_request_count: u64,
    #[serde(default)]
    pub failed_request_count: u64,
    #[serde(default)]
    pub interrupted_request_count: u64,
    #[serde(default)]
    pub usage_recorded_request_count: u64,
    #[serde(default)]
    pub throughput_output_tokens: u64,
    #[serde(default)]
    pub usage_recorded_request_millis: u64,
}

impl StatsTotals {
    fn add_totals(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.thinking_tokens = self.thinking_tokens.add(other.thinking_tokens);
        self.estimated_cost_nanos_usd += other.estimated_cost_nanos_usd;
        self.request_count += other.request_count;
        self.tool_call_count += other.tool_call_count;
        self.tool_success_count += other.tool_success_count;
        self.tool_failure_count += other.tool_failure_count;
        self.ttfb_total_millis += other.ttfb_total_millis;
        self.ttfb_recorded_request_count += other.ttfb_recorded_request_count;
        self.total_request_millis += other.total_request_millis;
        self.timed_request_count += other.timed_request_count;
        self.completed_request_count += other.completed_request_count;
        self.failed_request_count += other.failed_request_count;
        self.interrupted_request_count += other.interrupted_request_count;
        self.usage_recorded_request_count += other.usage_recorded_request_count;
        self.throughput_output_tokens += other.throughput_output_tokens;
        self.usage_recorded_request_millis += other.usage_recorded_request_millis;
    }

    fn record_request(&mut self) {
        self.request_count += 1;
    }

    fn record_tool_result(&mut self, is_failure: bool) {
        self.tool_call_count += 1;
        if is_failure {
            self.tool_failure_count += 1;
        } else {
            self.tool_success_count += 1;
        }
    }

    fn record_usage(
        &mut self,
        usage: Usage,
        thinking_tokens_estimated: bool,
        estimated_cost_nanos_usd: u64,
        request_millis: u64,
    ) {
        self.input_tokens += usage.input_tokens;
        self.cached_input_tokens += usage.cached_input_tokens;
        self.output_tokens += usage.output_tokens;
        self.thinking_tokens
            .record(usage.thinking_tokens, thinking_tokens_estimated);
        self.estimated_cost_nanos_usd += estimated_cost_nanos_usd;
        self.usage_recorded_request_count += 1;
        self.throughput_output_tokens += usage.output_tokens;
        self.usage_recorded_request_millis += request_millis;
    }

    fn record_usage_missing(&mut self, estimated_thinking_tokens: Option<u64>) {
        self.thinking_tokens.record(
            estimated_thinking_tokens,
            estimated_thinking_tokens.is_some(),
        );
    }

    fn record_timing(&mut self, ttfb_millis: Option<u64>, total_request_millis: u64) {
        if let Some(ttfb_millis) = ttfb_millis {
            self.ttfb_total_millis += ttfb_millis;
            self.ttfb_recorded_request_count += 1;
        }
        self.total_request_millis += total_request_millis;
        self.timed_request_count += 1;
    }

    fn record_request_outcome(&mut self, outcome: RequestOutcome) {
        match outcome {
            RequestOutcome::Completed => {
                self.completed_request_count += 1;
            }
            RequestOutcome::Failed => {
                self.failed_request_count += 1;
            }
            RequestOutcome::Interrupted => {
                self.interrupted_request_count += 1;
            }
        }
    }

    pub fn thinking_tokens_value(self) -> Option<u64> {
        if self.open_request_count() > 0 {
            return None;
        }
        self.thinking_tokens.value_for_requests(self.request_count)
    }

    pub fn thinking_tokens_partial(self) -> bool {
        if self.open_request_count() > 0 {
            return false;
        }
        self.thinking_tokens
            .is_partial_for_requests(self.request_count)
    }

    pub fn thinking_tokens_estimated(self) -> bool {
        if self.open_request_count() > 0 {
            return false;
        }
        self.thinking_tokens
            .is_estimated_for_requests(self.request_count)
    }

    pub fn average_ttfb_millis(self) -> Option<f64> {
        if self.ttfb_recorded_request_count == 0 {
            None
        } else {
            Some(self.ttfb_total_millis as f64 / self.ttfb_recorded_request_count as f64)
        }
    }

    pub fn average_total_request_millis(self) -> Option<f64> {
        if self.timed_request_count == 0 {
            None
        } else {
            Some(self.total_request_millis as f64 / self.timed_request_count as f64)
        }
    }

    pub fn tokens_per_second(self) -> Option<f64> {
        if self.usage_recorded_request_count == 0 || self.usage_recorded_request_millis == 0 {
            None
        } else {
            Some(
                self.throughput_output_tokens as f64
                    / (self.usage_recorded_request_millis as f64 / 1_000.0),
            )
        }
    }

    pub fn estimated_cost_usd(self) -> f64 {
        self.estimated_cost_nanos_usd as f64 / NANOS_PER_USD as f64
    }

    pub fn closed_request_count(self) -> u64 {
        self.completed_request_count + self.failed_request_count + self.interrupted_request_count
    }

    pub fn open_request_count(self) -> u64 {
        self.request_count
            .saturating_sub(self.closed_request_count())
    }

    pub fn requests_without_usage(self) -> u64 {
        self.closed_request_count()
            .saturating_sub(self.usage_recorded_request_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsReport {
    pub current: StatsTotals,
    pub historical: StatsTotals,
    pub current_models: BTreeMap<String, StatsTotals>,
    pub historical_models: BTreeMap<String, StatsTotals>,
    pub historical_session_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionStats {
    pub schema_version: u32,
    pub session_id: String,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    #[serde(flatten)]
    pub totals: StatsTotals,
    #[serde(default)]
    pub per_model: BTreeMap<String, StatsTotals>,
}

impl SessionStats {
    fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id: Uuid::now_v7().to_string(),
            started_at_unix_ms: unix_timestamp_ms(),
            finished_at_unix_ms: None,
            totals: StatsTotals::default(),
            per_model: BTreeMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.totals == StatsTotals::default() && self.per_model.is_empty()
    }

    fn finalize(&mut self) {
        self.finished_at_unix_ms = Some(unix_timestamp_ms());
    }

    fn apply_to_model_and_total(
        &mut self,
        model_name: Option<&str>,
        mut apply: impl FnMut(&mut StatsTotals),
    ) {
        apply(&mut self.totals);
        if let Some(model_name) = model_name {
            apply(self.per_model.entry(model_name.to_string()).or_default());
        }
    }
}

#[derive(Clone)]
pub struct StatsStore {
    state: Arc<Mutex<StatsState>>,
}

impl Default for StatsStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StatsStore {
    pub fn new() -> Self {
        Self::with_stats_dir(default_stats_dir())
    }

    fn with_stats_dir(stats_dir: Option<PathBuf>) -> Self {
        Self {
            state: Arc::new(Mutex::new(StatsState {
                stats_dir,
                current: SessionStats::new(),
            })),
        }
    }

    #[cfg(test)]
    pub fn hook(&self) -> StatsHook {
        StatsHook::new(Arc::clone(&self.state), None)
    }

    pub fn hook_for_model(&self, model_name: impl Into<String>) -> StatsHook {
        StatsHook::new(Arc::clone(&self.state), Some(model_name.into()))
    }

    pub fn rotate_session(&self) -> Result<()> {
        let (snapshot, stats_dir) = {
            let mut state = self.state.lock().expect("stats state lock");
            state.current.finalize();
            let snapshot = state.current.clone();
            let stats_dir = state.stats_dir.clone();
            state.current = SessionStats::new();
            (snapshot, stats_dir)
        };

        persist_session(stats_dir.as_deref(), &snapshot)
    }

    pub fn finalize_current_session(&self) -> Result<()> {
        let (snapshot, stats_dir) = {
            let mut state = self.state.lock().expect("stats state lock");
            state.current.finalize();
            (state.current.clone(), state.stats_dir.clone())
        };

        persist_session(stats_dir.as_deref(), &snapshot)
    }

    pub fn report(&self) -> Result<StatsReport> {
        let (current, stats_dir) = {
            let state = self.state.lock().expect("stats state lock");
            (state.current.clone(), state.stats_dir.clone())
        };

        let (historical, historical_models, historical_session_count) =
            load_historical_report(stats_dir.as_deref(), &current.session_id)?;

        Ok(StatsReport {
            current: current.totals,
            historical,
            current_models: current.per_model,
            historical_models,
            historical_session_count,
        })
    }

    pub fn current_totals(&self) -> StatsTotals {
        let state = self.state.lock().expect("stats state lock");
        state.current.totals
    }
}

impl Drop for StatsStore {
    fn drop(&mut self) {
        let _ = self.finalize_current_session();
    }
}

#[derive(Debug, Default)]
struct RequestLifecycle {
    active: Option<ActiveRequest>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveRequest {
    started_at: Instant,
    first_response_elapsed_millis: Option<u64>,
    model_wait_millis: Option<u64>,
    estimated_thinking_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone)]
pub struct StatsHook {
    state: Arc<Mutex<StatsState>>,
    model_name: Option<String>,
    request: Arc<Mutex<RequestLifecycle>>,
}

impl StatsHook {
    fn new(state: Arc<Mutex<StatsState>>, model_name: Option<String>) -> Self {
        Self {
            state,
            model_name,
            request: Arc::new(Mutex::new(RequestLifecycle::default())),
        }
    }

    pub fn with_model(&self, model_name: impl Into<String>) -> Self {
        Self::new(Arc::clone(&self.state), Some(model_name.into()))
    }

    fn record_request(&self) {
        let model_name = self.model_name.as_deref();
        let _ = update_and_persist(&self.state, |current| {
            current.apply_to_model_and_total(model_name, StatsTotals::record_request);
        });

        let mut request = self.request.lock().expect("stats request lock");
        request.active = Some(ActiveRequest {
            started_at: Instant::now(),
            first_response_elapsed_millis: None,
            model_wait_millis: None,
            estimated_thinking_tokens: 0,
        });
    }

    pub fn record_response_progress(&self) {
        let mut request = self.request.lock().expect("stats request lock");
        let Some(active) = request.active.as_mut() else {
            return;
        };

        if active.first_response_elapsed_millis.is_none() {
            active.first_response_elapsed_millis =
                Some(active.started_at.elapsed().as_millis() as u64);
        }
    }

    pub fn record_reasoning_progress(&self, text: &str) {
        if text.is_empty() {
            return;
        }

        let mut request = self.request.lock().expect("stats request lock");
        let Some(active) = request.active.as_mut() else {
            return;
        };
        active.estimated_thinking_tokens += count_text_tokens(text);
    }

    fn record_tool_result(&self, result: &str) {
        let normalized = normalize_tool_result(result);
        let is_failure = normalized.starts_with(TOOL_CALL_ERROR_PREFIX);
        let model_name = self.model_name.as_deref();
        let _ = update_and_persist(&self.state, |current| {
            current.apply_to_model_and_total(model_name, |totals| {
                totals.record_tool_result(is_failure);
            });
        });
    }

    fn persist_timing_if_needed(&self) {
        let timing = {
            let mut request = self.request.lock().expect("stats request lock");
            let Some(active) = request.active.as_mut() else {
                return;
            };
            if active.model_wait_millis.is_some() {
                return;
            }

            let total_request_millis = active.started_at.elapsed().as_millis() as u64;
            active.model_wait_millis = Some(total_request_millis);
            Some((active.first_response_elapsed_millis, total_request_millis))
        };

        let Some((ttfb_millis, total_request_millis)) = timing else {
            return;
        };

        let model_name = self.model_name.as_deref();
        let _ = update_and_persist(&self.state, |current| {
            current.apply_to_model_and_total(model_name, |totals| {
                totals.record_timing(ttfb_millis, total_request_millis);
            });
        });
    }

    fn pause_request_timing(&self) {
        self.persist_timing_if_needed();
    }

    fn complete_request_with_outcome(&self, usage: Option<Usage>, outcome: RequestOutcome) {
        self.persist_timing_if_needed();

        let active = {
            let mut request = self.request.lock().expect("stats request lock");
            request.active.take()
        };
        let Some(active) = active else {
            return;
        };

        let request_millis = active
            .model_wait_millis
            .unwrap_or_else(|| active.started_at.elapsed().as_millis() as u64);
        let estimated_thinking_tokens = active.estimated_thinking_tokens;
        let model_name = self.model_name.as_deref();
        let (usage, thinking_tokens_estimated, estimated_cost_nanos_usd) = match usage {
            Some(mut usage) => {
                let thinking_tokens_estimated =
                    usage.thinking_tokens.is_none() && estimated_thinking_tokens > 0;
                if thinking_tokens_estimated {
                    usage.thinking_tokens = Some(estimated_thinking_tokens);
                }
                let estimated_cost_nanos_usd = self
                    .model_name
                    .as_deref()
                    .map(|model_name| estimate_request_cost_nanos_usd(model_name, usage))
                    .unwrap_or(0);
                (
                    Some(usage),
                    thinking_tokens_estimated,
                    estimated_cost_nanos_usd,
                )
            }
            None => (None, false, 0),
        };

        let _ = update_and_persist(&self.state, |current| {
            current.apply_to_model_and_total(model_name, |totals| {
                totals.record_request_outcome(outcome);
                if let Some(usage) = usage {
                    totals.record_usage(
                        usage,
                        thinking_tokens_estimated,
                        estimated_cost_nanos_usd,
                        request_millis,
                    );
                } else {
                    totals.record_usage_missing(
                        (estimated_thinking_tokens > 0).then_some(estimated_thinking_tokens),
                    );
                }
            });
        });
    }

    fn complete_request(&self, usage: Option<Usage>) {
        self.complete_request_with_outcome(usage, RequestOutcome::Completed);
    }

    pub fn finish_request_without_usage(&self) {
        self.complete_request_with_outcome(None, RequestOutcome::Completed);
    }

    pub fn fail_request(&self) {
        self.complete_request_with_outcome(None, RequestOutcome::Failed);
    }

    fn interrupt_request(&self) {
        self.complete_request_with_outcome(None, RequestOutcome::Interrupted);
    }
}

impl<M> PromptHook<M> for StatsHook
where
    M: CompletionModel,
    M::StreamingResponse: GetTokenUsage,
{
    async fn on_completion_call(&self, _prompt: &Message, _history: &[Message]) -> HookAction {
        self.record_request();
        HookAction::cont()
    }

    async fn on_text_delta(&self, _text_delta: &str, _aggregated_text: &str) -> HookAction {
        self.record_response_progress();
        HookAction::cont()
    }

    async fn on_tool_call_delta(
        &self,
        _tool_call_id: &str,
        _internal_call_id: &str,
        _tool_name: Option<&str>,
        _tool_call_delta: &str,
    ) -> HookAction {
        self.record_response_progress();
        HookAction::cont()
    }

    async fn on_tool_call(
        &self,
        _tool_name: &str,
        _tool_call_id: Option<String>,
        _internal_call_id: &str,
        _args: &str,
    ) -> ToolCallHookAction {
        self.record_response_progress();
        self.pause_request_timing();
        ToolCallHookAction::Continue
    }

    async fn on_tool_result(
        &self,
        _tool_name: &str,
        _tool_call_id: Option<String>,
        _internal_call_id: &str,
        _args: &str,
        result: &str,
    ) -> HookAction {
        self.record_tool_result(result);
        HookAction::cont()
    }

    async fn on_stream_completion_response_finish(
        &self,
        _prompt: &Message,
        response: &M::StreamingResponse,
    ) -> HookAction {
        self.record_response_progress();
        self.complete_request(response.token_usage());
        HookAction::cont()
    }
}

impl Drop for StatsHook {
    fn drop(&mut self) {
        if Arc::strong_count(&self.request) == 1 {
            self.interrupt_request();
        }
    }
}

fn default_stats_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(STATS_DIR_RELATIVE_PATH))
}

fn update_and_persist(
    state: &Arc<Mutex<StatsState>>,
    mutate: impl FnOnce(&mut SessionStats),
) -> Result<()> {
    let (snapshot, stats_dir) = {
        let mut state = state.lock().expect("stats state lock");
        mutate(&mut state.current);
        (state.current.clone(), state.stats_dir.clone())
    };

    persist_session(stats_dir.as_deref(), &snapshot)
}

fn persist_session(stats_dir: Option<&Path>, session: &SessionStats) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }

    let Some(stats_dir) = stats_dir else {
        return Ok(());
    };

    fs::create_dir_all(stats_dir)
        .with_context(|| format!("failed to create {}", stats_dir.display()))?;

    let path = session_path(stats_dir, &session.session_id);
    let tmp_path = path.with_extension("json.tmp");
    let payload = serde_json::to_string_pretty(session).with_context(|| {
        format!(
            "failed to serialize stats for session {}",
            session.session_id
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
    Ok(())
}

fn load_historical_report(
    stats_dir: Option<&Path>,
    current_session_id: &str,
) -> Result<(StatsTotals, BTreeMap<String, StatsTotals>, usize)> {
    let Some(stats_dir) = stats_dir else {
        return Ok((StatsTotals::default(), BTreeMap::new(), 0));
    };

    if !stats_dir.exists() {
        return Ok((StatsTotals::default(), BTreeMap::new(), 0));
    }

    let mut totals = StatsTotals::default();
    let mut models = BTreeMap::new();
    let mut session_count = 0;

    for entry in fs::read_dir(stats_dir)
        .with_context(|| format!("failed to read {}", stats_dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", stats_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let session: SessionStats = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let session = normalize_legacy_session(session);
        if session.session_id == current_session_id || session.is_empty() {
            continue;
        }

        totals.add_totals(session.totals);
        for (model_name, model_totals) in session.per_model {
            models
                .entry(model_name)
                .or_insert_with(StatsTotals::default)
                .add_totals(model_totals);
        }
        session_count += 1;
    }

    Ok((totals, models, session_count))
}

fn normalize_legacy_session(mut session: SessionStats) -> SessionStats {
    if session.schema_version < 3 {
        if session.totals.request_count > 0
            && session.totals.thinking_tokens.tokens == 0
            && session.totals.thinking_tokens.unavailable_request_count == 0
        {
            session.totals.thinking_tokens.unavailable_request_count = session.totals.request_count;
        }
        for totals in session.per_model.values_mut() {
            if totals.request_count > 0
                && totals.thinking_tokens.tokens == 0
                && totals.thinking_tokens.unavailable_request_count == 0
            {
                totals.thinking_tokens.unavailable_request_count = totals.request_count;
            }
        }
    }

    if session.schema_version < 5 {
        normalize_legacy_thinking_coverage(&mut session.totals);
        for totals in session.per_model.values_mut() {
            normalize_legacy_thinking_coverage(totals);
        }
    }

    if session.schema_version < 4 {
        normalize_legacy_timing(&mut session.totals);
        for totals in session.per_model.values_mut() {
            normalize_legacy_timing(totals);
        }
    }

    session.schema_version = SCHEMA_VERSION;

    session
}

fn normalize_legacy_thinking_coverage(totals: &mut StatsTotals) {
    totals.thinking_tokens.available_request_count = if totals.request_count == 0 {
        0
    } else if totals.thinking_tokens.unavailable_request_count == 0 {
        totals.request_count
    } else if totals.thinking_tokens.tokens > 0 {
        1
    } else {
        0
    };
    totals.thinking_tokens.estimated_request_count = totals
        .thinking_tokens
        .estimated_request_count
        .min(totals.thinking_tokens.available_request_count);
}

fn normalize_legacy_timing(totals: &mut StatsTotals) {
    totals.ttfb_total_millis = 0;
    totals.ttfb_recorded_request_count = 0;
    totals.total_request_millis = 0;
    totals.timed_request_count = 0;
    totals.completed_request_count = totals.request_count;
    totals.failed_request_count = 0;
    totals.interrupted_request_count = 0;
    totals.usage_recorded_request_count = totals.request_count;
    totals.throughput_output_tokens = 0;
    totals.usage_recorded_request_millis = 0;
}

fn session_path(stats_dir: &Path, session_id: &str) -> PathBuf {
    stats_dir.join(format!("{session_id}.json"))
}

fn normalize_tool_result(result: &str) -> String {
    serde_json::from_str::<String>(result).unwrap_or_else(|_| result.to_string())
}

fn estimate_request_cost_nanos_usd(model_name: &str, usage: Usage) -> u64 {
    let Some(model) = model_registry::find_model(model_name) else {
        return 0;
    };

    let pricing = model.pricing_for_input_tokens(usage.input_tokens as usize);
    let uncached_input_tokens = usage.input_tokens.saturating_sub(usage.cached_input_tokens);

    token_cost_nanos(uncached_input_tokens, pricing.input_per_million_tokens)
        + token_cost_nanos(
            usage.cached_input_tokens,
            pricing.cache_read_per_million_tokens,
        )
        + token_cost_nanos(usage.output_tokens, pricing.output_per_million_tokens)
}

fn token_cost_nanos(tokens: u64, dollars_per_million_tokens: f64) -> u64 {
    if tokens == 0 || dollars_per_million_tokens == 0.0 {
        return 0;
    }

    let nanos_per_million_tokens = (dollars_per_million_tokens * NANOS_PER_USD as f64).round();
    ((tokens as f64 * nanos_per_million_tokens) / TOKENS_PER_MILLION as f64).round() as u64
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use futures_util::StreamExt;
    use rig::{
        agent::{AgentBuilder, MultiTurnStreamItem},
        completion::{CompletionError, CompletionRequest, CompletionResponse},
        streaming::{
            RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse, StreamingPrompt,
        },
    };
    use serde::{Deserialize, Serialize};

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oat-stats-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        ))
    }

    fn session_file_paths(dir: &Path) -> Vec<PathBuf> {
        let mut paths = fs::read_dir(dir)
            .expect("read stats dir")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    #[test]
    fn persist_session_skips_empty_sessions() {
        let dir = unique_temp_dir("empty");
        let session = SessionStats::new();

        persist_session(Some(&dir), &session).expect("persist empty session");

        assert!(!dir.exists());
    }

    #[test]
    fn tool_call_error_result_counts_as_failure() {
        let dir = unique_temp_dir("failure");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook();

        hook.record_request();
        hook.record_tool_result(r#""ToolCallError: missing field `filename`""#);
        hook.finish_request_without_usage();

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.tool_call_count, 1);
        assert_eq!(report.current.tool_failure_count, 1);
        assert_eq!(report.current.tool_success_count, 0);

        drop(hook);
        drop(store);
        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn rotate_session_persists_previous_session_and_excludes_current_from_history() {
        let dir = unique_temp_dir("rotate");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook_for_model("gpt-5.4-mini");

        hook.record_request();
        hook.record_response_progress();
        hook.complete_request(Some(Usage {
            input_tokens: 12,
            cached_input_tokens: 3,
            output_tokens: 6,
            total_tokens: 18,
            thinking_tokens: Some(2),
        }));
        hook.record_tool_result("ok");
        store.rotate_session().expect("rotate session");

        let report = store.report().expect("load stats report");
        assert_eq!(report.current, StatsTotals::default());
        assert_eq!(report.historical_session_count, 1);
        assert_eq!(report.historical.input_tokens, 12);
        assert_eq!(report.historical.cached_input_tokens, 3);
        assert_eq!(report.historical.output_tokens, 6);
        assert_eq!(report.historical.thinking_tokens_value(), Some(2));
        assert_eq!(report.historical.request_count, 1);
        assert_eq!(report.historical.tool_call_count, 1);
        assert_eq!(report.historical.tool_success_count, 1);
        assert_eq!(
            report
                .historical_models
                .get("gpt-5.4-mini")
                .expect("historical model")
                .thinking_tokens_value(),
            Some(2)
        );

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn finalize_current_session_marks_finished_at_and_writes_file() {
        let dir = unique_temp_dir("finalize");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook();

        hook.record_request();
        store
            .finalize_current_session()
            .expect("finalize current session");

        let paths = session_file_paths(&dir);
        assert_eq!(paths.len(), 1);
        let raw = fs::read_to_string(&paths[0]).expect("read session file");
        let session: SessionStats = serde_json::from_str(&raw).expect("parse session file");
        assert!(session.finished_at_unix_ms.is_some());

        drop(hook);
        drop(store);
        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn historical_report_sums_multiple_sessions_and_models() {
        let dir = unique_temp_dir("aggregate");
        fs::create_dir_all(&dir).expect("create stats dir");

        let mut first = SessionStats::new();
        first.totals.request_count = 1;
        first.totals.completed_request_count = 1;
        first.totals.usage_recorded_request_count = 1;
        first.totals.input_tokens = 10;
        first.totals.cached_input_tokens = 2;
        first.totals.output_tokens = 5;
        first.totals.thinking_tokens.tokens = 1;
        first.totals.thinking_tokens.available_request_count = 1;
        first.totals.estimated_cost_nanos_usd = 10_000;
        first.per_model.insert(
            "gpt-5.4-mini".into(),
            StatsTotals {
                request_count: 1,
                completed_request_count: 1,
                usage_recorded_request_count: 1,
                input_tokens: 10,
                cached_input_tokens: 2,
                output_tokens: 5,
                thinking_tokens: ThinkingTokenTotals {
                    tokens: 1,
                    available_request_count: 1,
                    unavailable_request_count: 0,
                    estimated_request_count: 0,
                },
                estimated_cost_nanos_usd: 10_000,
                ..StatsTotals::default()
            },
        );
        first.finalize();

        let mut second = SessionStats::new();
        second.totals.request_count = 2;
        second.totals.completed_request_count = 2;
        second.totals.usage_recorded_request_count = 2;
        second.totals.input_tokens = 20;
        second.totals.cached_input_tokens = 4;
        second.totals.output_tokens = 8;
        second.totals.thinking_tokens.unavailable_request_count = 2;
        second.totals.estimated_cost_nanos_usd = 20_000;
        second.totals.tool_call_count = 3;
        second.totals.tool_success_count = 2;
        second.totals.tool_failure_count = 1;
        second.per_model.insert(
            "gpt-5.4".into(),
            StatsTotals {
                request_count: 2,
                completed_request_count: 2,
                usage_recorded_request_count: 2,
                input_tokens: 20,
                cached_input_tokens: 4,
                output_tokens: 8,
                thinking_tokens: ThinkingTokenTotals {
                    tokens: 0,
                    available_request_count: 0,
                    unavailable_request_count: 2,
                    estimated_request_count: 0,
                },
                estimated_cost_nanos_usd: 20_000,
                tool_call_count: 3,
                tool_success_count: 2,
                tool_failure_count: 1,
                ..StatsTotals::default()
            },
        );
        second.finalize();

        persist_session(Some(&dir), &first).expect("persist first");
        persist_session(Some(&dir), &second).expect("persist second");

        let (totals, models, count) =
            load_historical_report(Some(&dir), "current-session").expect("load historical stats");
        assert_eq!(count, 2);
        assert_eq!(totals.input_tokens, 30);
        assert_eq!(totals.cached_input_tokens, 6);
        assert_eq!(totals.output_tokens, 13);
        assert_eq!(totals.estimated_cost_nanos_usd, 30_000);
        assert_eq!(totals.request_count, 3);
        assert_eq!(totals.tool_call_count, 3);
        assert_eq!(totals.tool_success_count, 2);
        assert_eq!(totals.tool_failure_count, 1);
        assert_eq!(totals.thinking_tokens_value(), Some(1));
        assert!(totals.thinking_tokens_partial());
        assert_eq!(models.len(), 2);
        assert_eq!(
            models
                .get("gpt-5.4-mini")
                .expect("mini model")
                .thinking_tokens_value(),
            Some(1)
        );

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn historical_report_marks_legacy_sessions_as_thinking_unavailable() {
        let dir = unique_temp_dir("legacy-historical");
        fs::create_dir_all(&dir).expect("create stats dir");

        let path = dir.join("legacy.json");
        fs::write(
            &path,
            r#"{
  "schema_version": 2,
  "session_id": "legacy-session",
  "started_at_unix_ms": 1,
  "finished_at_unix_ms": 2,
  "input_tokens": 100,
  "cached_input_tokens": 20,
  "output_tokens": 10,
  "estimated_cost_nanos_usd": 0,
  "request_count": 1,
  "tool_call_count": 0,
  "tool_success_count": 0,
  "tool_failure_count": 0
}"#,
        )
        .expect("write legacy stats");

        let (totals, _, count) =
            load_historical_report(Some(&dir), "current-session").expect("load historical stats");

        assert_eq!(count, 1);
        assert_eq!(totals.input_tokens, 100);
        assert_eq!(totals.cached_input_tokens, 20);
        assert_eq!(totals.output_tokens, 10);
        assert_eq!(totals.open_request_count(), 0);
        assert_eq!(totals.requests_without_usage(), 0);
        assert_eq!(totals.thinking_tokens_value(), None);
        assert_eq!(totals.average_ttfb_millis(), None);
        assert_eq!(totals.average_total_request_millis(), None);
        assert_eq!(totals.tokens_per_second(), None);
        assert_eq!(totals.usage_recorded_request_count, 1);
        assert_eq!(totals.throughput_output_tokens, 0);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn average_timing_and_tokens_per_second_are_derived_from_aggregates() {
        let totals = StatsTotals {
            output_tokens: 500,
            throughput_output_tokens: 500,
            ttfb_total_millis: 100,
            ttfb_recorded_request_count: 2,
            total_request_millis: 2_000,
            timed_request_count: 2,
            usage_recorded_request_count: 2,
            usage_recorded_request_millis: 2_000,
            ..StatsTotals::default()
        };

        assert_eq!(totals.average_ttfb_millis(), Some(50.0));
        assert_eq!(totals.average_total_request_millis(), Some(1_000.0));
        assert_eq!(totals.tokens_per_second(), Some(250.0));
    }

    #[test]
    fn pausing_request_timing_excludes_non_model_wait() {
        let dir = unique_temp_dir("pause-timing");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook_for_model("gpt-5.4-mini");

        hook.record_request();
        std::thread::sleep(Duration::from_millis(15));
        hook.record_response_progress();
        hook.pause_request_timing();
        std::thread::sleep(Duration::from_millis(20));
        hook.complete_request(Some(Usage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 5,
            total_tokens: 15,
            thinking_tokens: Some(1),
        }));

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.timed_request_count, 1);
        assert_eq!(report.current.completed_request_count, 1);
        assert_eq!(report.current.usage_recorded_request_count, 1);
        assert!(report.current.total_request_millis < 35);
        assert_eq!(
            report.current.tokens_per_second(),
            Some(
                report.current.throughput_output_tokens as f64
                    / (report.current.usage_recorded_request_millis as f64 / 1_000.0)
            )
        );

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn failed_requests_without_response_do_not_contribute_ttfb() {
        let dir = unique_temp_dir("failed-no-ttfb");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook();

        hook.record_request();
        std::thread::sleep(Duration::from_millis(1));
        hook.fail_request();

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.request_count, 1);
        assert_eq!(report.current.failed_request_count, 1);
        assert_eq!(report.current.average_ttfb_millis(), None);
        assert_eq!(report.current.ttfb_recorded_request_count, 0);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn completed_requests_without_usage_are_counted_explicitly() {
        let dir = unique_temp_dir("missing-usage");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook();

        hook.record_request();
        hook.record_response_progress();
        hook.finish_request_without_usage();

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.completed_request_count, 1);
        assert_eq!(report.current.usage_recorded_request_count, 0);
        assert_eq!(report.current.requests_without_usage(), 1);
        assert_eq!(report.current.thinking_tokens_value(), None);
        assert_eq!(report.current.tokens_per_second(), None);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn reasoning_text_estimates_thinking_tokens_when_usage_omits_them() {
        let dir = unique_temp_dir("estimated-thinking-usage");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook_for_model("kimi-k2.5");

        hook.record_request();
        hook.record_response_progress();
        hook.record_reasoning_progress("Working through the problem step by step.");
        hook.complete_request(Some(Usage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 5,
            total_tokens: 15,
            thinking_tokens: None,
        }));

        let report = store.report().expect("load stats report");
        assert!(report.current.thinking_tokens_value().unwrap_or(0) > 0);
        assert!(report.current.thinking_tokens_estimated());
        assert_eq!(report.current.usage_recorded_request_count, 1);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn reasoning_text_estimates_thinking_tokens_without_usage() {
        let dir = unique_temp_dir("estimated-thinking-no-usage");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook();

        hook.record_request();
        hook.record_response_progress();
        hook.record_reasoning_progress("Reasoning content that should still count.");
        hook.finish_request_without_usage();

        let report = store.report().expect("load stats report");
        assert!(report.current.thinking_tokens_value().unwrap_or(0) > 0);
        assert!(report.current.thinking_tokens_estimated());
        assert_eq!(report.current.requests_without_usage(), 1);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn thinking_tokens_stay_visible_when_coverage_is_partial() {
        let mut totals = StatsTotals {
            request_count: 2,
            completed_request_count: 2,
            usage_recorded_request_count: 2,
            ..StatsTotals::default()
        };
        totals.thinking_tokens.record(Some(12), false);
        totals.thinking_tokens.record(None, false);

        assert_eq!(totals.thinking_tokens_value(), Some(12));
        assert!(totals.thinking_tokens_partial());
    }

    #[test]
    fn thinking_tokens_can_be_marked_estimated() {
        let mut totals = StatsTotals {
            request_count: 1,
            completed_request_count: 1,
            ..StatsTotals::default()
        };
        totals.thinking_tokens.record(Some(9), true);

        assert_eq!(totals.thinking_tokens_value(), Some(9));
        assert!(totals.thinking_tokens_estimated());
    }

    #[test]
    fn dropping_last_hook_with_active_request_marks_it_interrupted() {
        let dir = unique_temp_dir("interrupted-drop");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));

        {
            let hook = store.hook();
            hook.record_request();
            std::thread::sleep(Duration::from_millis(1));
        }

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.request_count, 1);
        assert_eq!(report.current.interrupted_request_count, 1);
        assert_eq!(report.current.timed_request_count, 1);
        assert_eq!(report.current.thinking_tokens_value(), None);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn estimate_request_cost_uses_base_pricing_for_gpt_5_4() {
        let cost = estimate_request_cost_nanos_usd(
            "gpt-5.4",
            Usage {
                input_tokens: 272_001,
                cached_input_tokens: 0,
                output_tokens: 10,
                total_tokens: 272_011,
                thinking_tokens: Some(4),
            },
        );

        assert_eq!(cost, 680_152_500);
    }

    #[test]
    fn report_tracks_total_cost_across_model_switches() {
        let dir = unique_temp_dir("mixed-models");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));

        let mini_hook = store.hook_for_model("gpt-5.4-mini");
        mini_hook.record_request();
        mini_hook.record_response_progress();
        mini_hook.complete_request(Some(Usage {
            input_tokens: 1_000,
            cached_input_tokens: 200,
            output_tokens: 500,
            total_tokens: 1_500,
            thinking_tokens: Some(120),
        }));

        let main_hook = store.hook_for_model("gpt-5.4");
        main_hook.record_request();
        std::thread::sleep(Duration::from_millis(1));
        main_hook.record_response_progress();
        main_hook.complete_request(Some(Usage {
            input_tokens: 300_000,
            cached_input_tokens: 50_000,
            output_tokens: 1_000,
            total_tokens: 301_000,
            thinking_tokens: None,
        }));

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.request_count, 2);
        assert_eq!(report.current.input_tokens, 301_000);
        assert_eq!(report.current.cached_input_tokens, 50_200);
        assert_eq!(report.current.output_tokens, 1_500);
        assert_eq!(report.current.estimated_cost_nanos_usd, 655_365_000);
        assert_eq!(report.current_models.len(), 2);
        assert_eq!(report.current.thinking_tokens_value(), Some(120));
        assert!(report.current.thinking_tokens_partial());
        assert_eq!(
            report
                .current_models
                .get("gpt-5.4-mini")
                .expect("mini")
                .thinking_tokens_value(),
            Some(120)
        );

        drop(mini_hook);
        drop(main_hook);
        drop(store);
        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct MockStreamingResponse {
        usage: Usage,
    }

    impl MockStreamingResponse {
        fn new(input_tokens: u64, output_tokens: u64) -> Self {
            Self {
                usage: Usage {
                    input_tokens,
                    cached_input_tokens: 0,
                    output_tokens,
                    total_tokens: input_tokens + output_tokens,
                    thinking_tokens: Some(output_tokens / 2),
                },
            }
        }
    }

    impl GetTokenUsage for MockStreamingResponse {
        fn token_usage(&self) -> Option<Usage> {
            Some(self.usage)
        }
    }

    #[derive(Clone, Default)]
    struct ToolThenFailureModel {
        turn_counter: Arc<AtomicUsize>,
    }

    #[allow(refining_impl_trait)]
    impl CompletionModel for ToolThenFailureModel {
        type Response = ();
        type StreamingResponse = MockStreamingResponse;
        type Client = ();

        fn make(_: &Self::Client, _: impl Into<String>) -> Self {
            Self::default()
        }

        async fn completion(
            &self,
            _request: CompletionRequest,
        ) -> std::result::Result<CompletionResponse<Self::Response>, CompletionError> {
            Err(CompletionError::ProviderError(
                "completion is unused in this streaming test".to_string(),
            ))
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
        ) -> std::result::Result<
            StreamingCompletionResponse<Self::StreamingResponse>,
            CompletionError,
        > {
            let turn = self.turn_counter.fetch_add(1, Ordering::SeqCst);
            let stream = async_stream::stream! {
                if turn == 0 {
                    yield Ok(RawStreamingChoice::ToolCall(
                        RawStreamingToolCall::new(
                            "tool_call_1".to_string(),
                            "missing_tool".to_string(),
                            serde_json::json!({"input": "value"}),
                        )
                        .with_call_id("call_1".to_string()),
                    ));
                    yield Ok(RawStreamingChoice::FinalResponse(MockStreamingResponse::new(12, 4)));
                } else {
                    yield Err(CompletionError::ProviderError("boom".to_string()));
                }
            };

            Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
        }
    }

    #[tokio::test]
    async fn completed_tool_only_steps_record_usage_before_later_failure() {
        let dir = unique_temp_dir("tool-step-usage");
        let store = StatsStore::with_stats_dir(Some(dir.clone()));
        let hook = store.hook_for_model("gpt-5.4-mini");
        let agent = AgentBuilder::new(ToolThenFailureModel::default()).build();

        let mut stream = agent
            .stream_prompt("do tool work")
            .with_history(Vec::new())
            .with_hook(hook)
            .multi_turn(3)
            .await;

        let mut saw_tool_result = false;
        let mut saw_failure = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(MultiTurnStreamItem::StreamUserItem(_)) => {
                    saw_tool_result = true;
                }
                Ok(MultiTurnStreamItem::FinalResponse(_)) => {
                    panic!("stream should fail before the overall turn completes");
                }
                Err(error) if error.to_string().contains("boom") => {
                    saw_failure = true;
                    break;
                }
                Err(error) => panic!("unexpected stream error: {error:?}"),
                Ok(_) => {}
            }
        }

        assert!(saw_tool_result);
        assert!(saw_failure);

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.request_count, 2);
        assert_eq!(report.current.open_request_count(), 1);
        assert_eq!(report.current.completed_request_count, 1);
        assert_eq!(report.current.tool_call_count, 1);
        assert_eq!(report.current.input_tokens, 12);
        assert_eq!(report.current.output_tokens, 4);
        assert_eq!(report.current.thinking_tokens_value(), None);
        assert_eq!(report.current.estimated_cost_nanos_usd, 27_000);
        // This test drives rig's raw multi-turn stream directly. The follow-up
        // request increments request_count via the hook, but request closure is
        // finalized by oat's streaming wrappers, not by the hook alone.
        assert_eq!(report.current.timed_request_count, 1);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }
}
