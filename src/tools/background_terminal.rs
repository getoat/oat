use std::path::PathBuf;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::background_terminals::{
    BackgroundTerminalInspectRequest, BackgroundTerminalManager, BackgroundTerminalSpawnRequest,
};
use crate::debug_log::log_debug;

use super::{
    RUN_SHELL_SCRIPT_TOOL_NAME, common::ToolExecError, shell_command::ShellCommandRequest,
};

pub const START_BACKGROUND_TERMINAL_TOOL_NAME: &str = "StartBackgroundTerminal";
pub const LIST_BACKGROUND_TERMINALS_TOOL_NAME: &str = "ListBackgroundTerminals";
pub const INSPECT_BACKGROUND_TERMINAL_TOOL_NAME: &str = "InspectBackgroundTerminal";
pub const KILL_BACKGROUND_TERMINAL_TOOL_NAME: &str = "KillBackgroundTerminal";

#[derive(Clone)]
pub struct StartBackgroundTerminalTool {
    root: PathBuf,
    manager: BackgroundTerminalManager,
    allow_full_system_access: bool,
}

#[derive(Clone)]
pub struct ListBackgroundTerminalsTool {
    manager: BackgroundTerminalManager,
}

#[derive(Clone)]
pub struct InspectBackgroundTerminalTool {
    manager: BackgroundTerminalManager,
}

#[derive(Clone)]
pub struct KillBackgroundTerminalTool {
    manager: BackgroundTerminalManager,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StartBackgroundTerminalArgs {
    #[serde(flatten)]
    pub command: ShellCommandRequest,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct InspectBackgroundTerminalArgs {
    pub id: String,
    #[serde(default)]
    pub after_sequence: Option<u64>,
    #[serde(default)]
    pub wait_for_change_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct KillBackgroundTerminalArgs {
    pub id: String,
}

impl StartBackgroundTerminalTool {
    pub fn new(root: PathBuf, manager: BackgroundTerminalManager) -> Self {
        Self::new_with_access(root, manager, false)
    }

    pub fn new_with_access(
        root: PathBuf,
        manager: BackgroundTerminalManager,
        allow_full_system_access: bool,
    ) -> Self {
        Self {
            root,
            manager,
            allow_full_system_access,
        }
    }
}

impl ListBackgroundTerminalsTool {
    pub fn new(manager: BackgroundTerminalManager) -> Self {
        Self { manager }
    }
}

impl InspectBackgroundTerminalTool {
    pub fn new(manager: BackgroundTerminalManager) -> Self {
        Self { manager }
    }
}

impl KillBackgroundTerminalTool {
    pub fn new(manager: BackgroundTerminalManager) -> Self {
        Self { manager }
    }
}

impl Tool for StartBackgroundTerminalTool {
    const NAME: &'static str = START_BACKGROUND_TERMINAL_TOOL_NAME;
    type Error = ToolExecError;
    type Args = StartBackgroundTerminalArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Start a long-running shell command in a managed background terminal. Use this instead of `{}` for dev servers, watchers, or scripts whose output you will inspect later.",
                RUN_SHELL_SCRIPT_TOOL_NAME
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "script": { "type": "string" },
                    "cwd": { "type": "string" },
                    "intent": { "type": "string" },
                    "label": {
                        "type": "string",
                        "description": "Optional short label shown in lists and transcript status."
                    }
                },
                "required": ["script", "intent"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let cwd = args
            .command
            .resolve_cwd_with_access(&self.root, self.allow_full_system_access)?;
        let cwd_label = args
            .command
            .cwd_label_with_access(&self.root, self.allow_full_system_access)?;
        let label = args
            .label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| truncate_label(&args.command.display_command()));
        let snapshot = self
            .manager
            .start(BackgroundTerminalSpawnRequest {
                label,
                cwd: cwd.display().to_string(),
                script: args.command.script,
            })
            .await
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        let mut value = serde_json::to_value(&snapshot)
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        value["cwd"] = json!(cwd_label);
        serde_json::to_string(&value).map_err(|error| ToolExecError::new(error.to_string()))
    }
}

impl Tool for ListBackgroundTerminalsTool {
    const NAME: &'static str = LIST_BACKGROUND_TERMINALS_TOOL_NAME;
    type Error = ToolExecError;
    type Args = ();
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List running and recent background terminals.".into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        serde_json::to_string(&self.manager.list())
            .map_err(|error| ToolExecError::new(error.to_string()))
    }
}

impl Tool for InspectBackgroundTerminalTool {
    const NAME: &'static str = INSPECT_BACKGROUND_TERMINAL_TOOL_NAME;
    type Error = ToolExecError;
    type Args = InspectBackgroundTerminalArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Inspect a managed background terminal, including retained output.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "after_sequence": { "type": "integer" },
                    "wait_for_change_ms": { "type": "integer" }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let result = self
            .manager
            .inspect(
                &args.id,
                BackgroundTerminalInspectRequest {
                    after_sequence: args.after_sequence,
                    wait_for_change_ms: args.wait_for_change_ms,
                },
            )
            .await
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        serde_json::to_string(&result).map_err(|error| ToolExecError::new(error.to_string()))
    }
}

impl Tool for KillBackgroundTerminalTool {
    const NAME: &'static str = KILL_BACKGROUND_TERMINAL_TOOL_NAME;
    type Error = ToolExecError;
    type Args = KillBackgroundTerminalArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Cancel a running background terminal and then continue the current task normally. If the user asked for confirmation, report the result. If they need more detail, inspect or list terminals next.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        log_debug(
            "background_terminal_tool",
            format!("kill_call_start id={}", args.id),
        );
        let snapshot = self
            .manager
            .kill(&args.id)
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        log_debug(
            "background_terminal_tool",
            format!(
                "kill_call_done id={} status={:?}",
                snapshot.id, snapshot.status
            ),
        );
        serde_json::to_string(&snapshot).map_err(|error| ToolExecError::new(error.to_string()))
    }
}

fn truncate_label(command: &str) -> String {
    const MAX_CHARS: usize = 48;
    let trimmed = command.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    if MAX_CHARS <= 3 {
        return ".".repeat(MAX_CHARS);
    }
    trimmed.chars().take(MAX_CHARS - 3).collect::<String>() + "..."
}
