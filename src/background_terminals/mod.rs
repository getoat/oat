mod buffer;
mod events;
mod format;
mod manager;
mod pty;
mod store;

pub(crate) use events::BackgroundTerminalUiEvent;
pub(crate) use format::{format_terminal_inspect_message, format_terminal_list_message};

use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTerminalStatus {
    Running,
    Exited,
    Cancelled,
    SpawnFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TerminalExitInfo {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackgroundTerminalSnapshot {
    pub id: String,
    pub label: String,
    pub status: BackgroundTerminalStatus,
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub last_activity_at: DateTime<Utc>,
    pub retained_output_tokens: u64,
    pub output_sequence: u64,
    pub output_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_info: Option<TerminalExitInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TerminalOutputSlice {
    pub sequence: u64,
    pub text: String,
    pub output_truncated: bool,
    pub cursor_expired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackgroundTerminalInspectResult {
    pub snapshot: BackgroundTerminalSnapshot,
    pub output: TerminalOutputSlice,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTerminalSpawnRequest {
    pub label: String,
    pub cwd: String,
    pub script: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundTerminalInspectRequest {
    pub after_sequence: Option<u64>,
    pub wait_for_change_ms: Option<u64>,
}

#[derive(Clone)]
pub struct BackgroundTerminalManager {
    inner: std::sync::Arc<store::Inner>,
}

#[cfg(test)]
mod tests;
