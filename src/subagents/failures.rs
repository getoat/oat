use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use regex::Regex;
use serde::Serialize;

use crate::completion_request::CompletionRequestSnapshot;

#[derive(Debug, Serialize)]
pub(crate) struct SubagentFailureLog {
    pub(crate) schema_version: u32,
    pub(crate) subagent_id: String,
    pub(crate) failed_at_unix_ms: u64,
    pub(crate) model_name: String,
    pub(crate) access_mode: String,
    pub(crate) prompt: String,
    pub(crate) raw_error: String,
    pub(crate) normalized_error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failing_request: Option<CompletionRequestSnapshot>,
}

pub(crate) struct SubagentExecutionFailure {
    pub(crate) raw_error: String,
    pub(crate) failing_request: Option<CompletionRequestSnapshot>,
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

pub(crate) fn default_subagent_failure_log_dir(relative_path: &str) -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(relative_path))
}

pub(crate) fn persist_subagent_failure_log(
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

pub(crate) fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time is after epoch")
        .as_millis() as u64
}
