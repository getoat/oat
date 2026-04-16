mod events;
mod executor;
mod failures;
mod store;
mod waiting;

use std::sync::Arc;

use serde::Serialize;

use crate::{
    app::{AccessMode, CommandRisk},
    config::AppConfig,
    llm::{ShellApprovalController, WriteApprovalController},
    token_counting::count_text_tokens,
    web::WebService,
};
pub use failures::normalize_subagent_failure;
pub(crate) use failures::{SubagentFailureLog, persist_subagent_failure_log};
use store::Inner;

const DEFAULT_WAIT_TIMEOUT_MS: u64 = 30_000;
const SUBAGENT_FAILURE_LOG_DIR_RELATIVE_PATH: &str = ".config/oat/subagent_failures";
const SUBAGENT_FAILURE_LOG_SCHEMA_VERSION: u32 = 2;

#[derive(Clone)]
pub struct SubagentManager {
    inner: Arc<Inner>,
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
    pub allow_full_system_access: bool,
    pub activity_kind: SubagentActivityKind,
    pub model_name_override: Option<String>,
    pub config: AppConfig,
    pub write_approvals: WriteApprovalController,
    pub shell_approvals: ShellApprovalController,
    pub web: WebService,
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

pub fn estimate_prompt_tokens(prompt: &str) -> usize {
    count_text_tokens(prompt) as usize
}

#[cfg(test)]
mod tests;
