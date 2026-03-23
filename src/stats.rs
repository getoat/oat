use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use rig::{
    agent::{HookAction, PromptHook},
    completion::{CompletionModel, GetTokenUsage, Message, Usage},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model_registry;

const STATS_DIR_RELATIVE_PATH: &str = ".config/oat/stats";
const SCHEMA_VERSION: u32 = 2;
const TOOL_CALL_ERROR_PREFIX: &str = "ToolCallError:";
const NANOS_PER_USD: u64 = 1_000_000_000;
const TOKENS_PER_MILLION: u64 = 1_000_000;

#[derive(Debug)]
struct StatsState {
    stats_dir: Option<PathBuf>,
    current: SessionStats,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatsTotals {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost_nanos_usd: u64,
    pub request_count: u64,
    pub tool_call_count: u64,
    pub tool_success_count: u64,
    pub tool_failure_count: u64,
}

impl StatsTotals {
    fn add_session(&mut self, session: &SessionStats) {
        self.input_tokens += session.input_tokens;
        self.cached_input_tokens += session.cached_input_tokens;
        self.output_tokens += session.output_tokens;
        self.estimated_cost_nanos_usd += session.estimated_cost_nanos_usd;
        self.request_count += session.request_count;
        self.tool_call_count += session.tool_call_count;
        self.tool_success_count += session.tool_success_count;
        self.tool_failure_count += session.tool_failure_count;
    }

    fn cached_input_percent(self) -> f64 {
        if self.input_tokens == 0 {
            0.0
        } else {
            (self.cached_input_tokens as f64 / self.input_tokens as f64) * 100.0
        }
    }

    fn tool_success_rate(self) -> f64 {
        if self.tool_call_count == 0 {
            0.0
        } else {
            (self.tool_success_count as f64 / self.tool_call_count as f64) * 100.0
        }
    }

    pub fn estimated_cost_usd(self) -> f64 {
        self.estimated_cost_nanos_usd as f64 / NANOS_PER_USD as f64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsReport {
    pub current: StatsTotals,
    pub historical: StatsTotals,
    pub historical_session_count: usize,
}

impl StatsReport {
    pub fn render(&self) -> String {
        format!(
            "Current session\n\n{}\n\nHistorical sessions ({})\n\n{}",
            render_totals(self.current),
            self.historical_session_count,
            render_totals(self.historical),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionStats {
    pub schema_version: u32,
    pub session_id: String,
    pub started_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub estimated_cost_nanos_usd: u64,
    pub request_count: u64,
    pub tool_call_count: u64,
    pub tool_success_count: u64,
    pub tool_failure_count: u64,
}

impl SessionStats {
    fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            session_id: Uuid::now_v7().to_string(),
            started_at_unix_ms: unix_timestamp_ms(),
            finished_at_unix_ms: None,
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            estimated_cost_nanos_usd: 0,
            request_count: 0,
            tool_call_count: 0,
            tool_success_count: 0,
            tool_failure_count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.cached_input_tokens == 0
            && self.output_tokens == 0
            && self.estimated_cost_nanos_usd == 0
            && self.request_count == 0
            && self.tool_call_count == 0
            && self.tool_success_count == 0
            && self.tool_failure_count == 0
    }

    fn finalize(&mut self) {
        self.finished_at_unix_ms = Some(unix_timestamp_ms());
    }

    fn totals(&self) -> StatsTotals {
        let mut totals = StatsTotals::default();
        totals.add_session(self);
        totals
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

    pub fn hook(&self) -> StatsHook {
        StatsHook {
            state: Arc::clone(&self.state),
            model_name: None,
        }
    }

    pub fn hook_for_model(&self, model_name: impl Into<String>) -> StatsHook {
        StatsHook {
            state: Arc::clone(&self.state),
            model_name: Some(model_name.into()),
        }
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

        let (historical, historical_session_count) =
            load_historical_totals(stats_dir.as_deref(), &current.session_id)?;

        Ok(StatsReport {
            current: current.totals(),
            historical,
            historical_session_count,
        })
    }

    pub fn current_totals(&self) -> StatsTotals {
        let state = self.state.lock().expect("stats state lock");
        state.current.totals()
    }
}

impl Drop for StatsStore {
    fn drop(&mut self) {
        let _ = self.finalize_current_session();
    }
}

#[derive(Clone)]
pub struct StatsHook {
    state: Arc<Mutex<StatsState>>,
    model_name: Option<String>,
}

impl StatsHook {
    fn record_request(&self) {
        let _ = update_and_persist(&self.state, |current| {
            current.request_count += 1;
        });
    }

    fn record_tool_result(&self, result: &str) {
        let normalized = normalize_tool_result(result);
        let is_failure = normalized.starts_with(TOOL_CALL_ERROR_PREFIX);
        let _ = update_and_persist(&self.state, |current| {
            current.tool_call_count += 1;
            if is_failure {
                current.tool_failure_count += 1;
            } else {
                current.tool_success_count += 1;
            }
        });
    }

    fn record_usage(&self, usage: Usage) {
        let estimated_cost_nanos_usd = self
            .model_name
            .as_deref()
            .map(|model_name| estimate_request_cost_nanos_usd(model_name, usage))
            .unwrap_or(0);
        let _ = update_and_persist(&self.state, |current| {
            current.input_tokens += usage.input_tokens;
            current.cached_input_tokens += usage.cached_input_tokens;
            current.output_tokens += usage.output_tokens;
            current.estimated_cost_nanos_usd += estimated_cost_nanos_usd;
        });
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
        self.record_usage(response.token_usage().unwrap_or_default());
        HookAction::cont()
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

fn load_historical_totals(
    stats_dir: Option<&Path>,
    current_session_id: &str,
) -> Result<(StatsTotals, usize)> {
    let Some(stats_dir) = stats_dir else {
        return Ok((StatsTotals::default(), 0));
    };

    if !stats_dir.exists() {
        return Ok((StatsTotals::default(), 0));
    }

    let mut totals = StatsTotals::default();
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
        if session.session_id == current_session_id || session.is_empty() {
            continue;
        }

        totals.add_session(&session);
        session_count += 1;
    }

    Ok((totals, session_count))
}

fn session_path(stats_dir: &Path, session_id: &str) -> PathBuf {
    stats_dir.join(format!("{session_id}.json"))
}

fn normalize_tool_result(result: &str) -> String {
    serde_json::from_str::<String>(result).unwrap_or_else(|_| result.to_string())
}

fn render_totals(totals: StatsTotals) -> String {
    format!(
        "- Input tokens: {}\n- Cached input tokens: {} ({:.1}%)\n- Output tokens: {}\n- Estimated cost: ${:.6}\n- Requests: {}\n- Tool calls: {} ({} success, {} fail, {:.1}% success)",
        totals.input_tokens,
        totals.cached_input_tokens,
        totals.cached_input_percent(),
        totals.output_tokens,
        totals.estimated_cost_usd(),
        totals.request_count,
        totals.tool_call_count,
        totals.tool_success_count,
        totals.tool_failure_count,
        totals.tool_success_rate(),
    )
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn report_formats_expected_labels() {
        let report = StatsReport {
            current: StatsTotals {
                input_tokens: 10,
                cached_input_tokens: 2,
                output_tokens: 4,
                estimated_cost_nanos_usd: 12_345_678,
                request_count: 3,
                tool_call_count: 5,
                tool_success_count: 4,
                tool_failure_count: 1,
            },
            historical: StatsTotals::default(),
            historical_session_count: 0,
        };

        let rendered = report.render();

        assert!(rendered.contains("Current session"));
        assert!(rendered.contains("Historical sessions (0)"));
        assert!(rendered.contains("- Cached input tokens: 2 (20.0%)"));
        assert!(rendered.contains("- Estimated cost: $0.012346"));
        assert!(rendered.contains("- Tool calls: 5 (4 success, 1 fail, 80.0% success)"));
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

        hook.record_tool_result(r#""ToolCallError: missing field `filename`""#);

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
        hook.record_usage(Usage {
            input_tokens: 12,
            cached_input_tokens: 3,
            output_tokens: 6,
            total_tokens: 18,
        });
        hook.record_tool_result("ok");
        store.rotate_session().expect("rotate session");

        let report = store.report().expect("load stats report");
        assert_eq!(report.current, StatsTotals::default());
        assert_eq!(report.historical_session_count, 1);
        assert_eq!(report.historical.input_tokens, 12);
        assert_eq!(report.historical.cached_input_tokens, 3);
        assert_eq!(report.historical.output_tokens, 6);
        assert_eq!(report.historical.estimated_cost_nanos_usd, 33_975);
        assert_eq!(report.historical.request_count, 1);
        assert_eq!(report.historical.tool_call_count, 1);
        assert_eq!(report.historical.tool_success_count, 1);
        assert_eq!(report.historical.tool_failure_count, 0);

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
    fn historical_totals_sum_multiple_sessions() {
        let dir = unique_temp_dir("aggregate");
        fs::create_dir_all(&dir).expect("create stats dir");

        let mut first = SessionStats::new();
        first.request_count = 1;
        first.input_tokens = 10;
        first.cached_input_tokens = 2;
        first.output_tokens = 5;
        first.estimated_cost_nanos_usd = 10_000;
        first.tool_call_count = 1;
        first.tool_success_count = 1;
        first.finalize();

        let mut second = SessionStats::new();
        second.request_count = 2;
        second.input_tokens = 20;
        second.cached_input_tokens = 4;
        second.output_tokens = 8;
        second.estimated_cost_nanos_usd = 20_000;
        second.tool_call_count = 3;
        second.tool_success_count = 2;
        second.tool_failure_count = 1;
        second.finalize();

        persist_session(Some(&dir), &first).expect("persist first");
        persist_session(Some(&dir), &second).expect("persist second");

        let (totals, count) =
            load_historical_totals(Some(&dir), "current-session").expect("load historical stats");
        assert_eq!(count, 2);
        assert_eq!(totals.input_tokens, 30);
        assert_eq!(totals.cached_input_tokens, 6);
        assert_eq!(totals.output_tokens, 13);
        assert_eq!(totals.estimated_cost_nanos_usd, 30_000);
        assert_eq!(totals.request_count, 3);
        assert_eq!(totals.tool_call_count, 4);
        assert_eq!(totals.tool_success_count, 3);
        assert_eq!(totals.tool_failure_count, 1);

        fs::remove_dir_all(dir).expect("remove temp dir");
    }

    #[test]
    fn historical_totals_accept_legacy_sessions_without_cost_field() {
        let dir = unique_temp_dir("legacy-historical");
        fs::create_dir_all(&dir).expect("create stats dir");

        let path = dir.join("legacy.json");
        fs::write(
            &path,
            r#"{
  "schema_version": 1,
  "session_id": "legacy-session",
  "started_at_unix_ms": 1,
  "finished_at_unix_ms": 2,
  "input_tokens": 100,
  "cached_input_tokens": 20,
  "output_tokens": 10,
  "request_count": 1,
  "tool_call_count": 0,
  "tool_success_count": 0,
  "tool_failure_count": 0
}"#,
        )
        .expect("write legacy stats");

        let (totals, count) =
            load_historical_totals(Some(&dir), "current-session").expect("load historical stats");

        assert_eq!(count, 1);
        assert_eq!(totals.input_tokens, 100);
        assert_eq!(totals.cached_input_tokens, 20);
        assert_eq!(totals.output_tokens, 10);
        assert_eq!(totals.estimated_cost_nanos_usd, 0);

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
        mini_hook.record_usage(Usage {
            input_tokens: 1_000,
            cached_input_tokens: 200,
            output_tokens: 500,
            total_tokens: 1_500,
        });

        let main_hook = store.hook_for_model("gpt-5.4");
        main_hook.record_request();
        main_hook.record_usage(Usage {
            input_tokens: 300_000,
            cached_input_tokens: 50_000,
            output_tokens: 1_000,
            total_tokens: 301_000,
        });

        let report = store.report().expect("load stats report");
        assert_eq!(report.current.request_count, 2);
        assert_eq!(report.current.input_tokens, 301_000);
        assert_eq!(report.current.cached_input_tokens, 50_200);
        assert_eq!(report.current.output_tokens, 1_500);
        assert_eq!(report.current.estimated_cost_nanos_usd, 655_365_000);

        drop(mini_hook);
        drop(main_hook);
        drop(store);
        fs::remove_dir_all(dir).expect("remove temp dir");
    }
}
