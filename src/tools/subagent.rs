use std::time::Duration;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    app::AccessMode,
    config::AppConfig,
    llm::WriteApprovalController,
    model_registry,
    subagents::{SubagentManager, SubagentSpawnRequest, estimate_prompt_tokens},
};

use super::common::ToolExecError;

pub const SPAWN_SUBAGENT_TOOL_NAME: &str = "SpawnSubagent";
pub const WAIT_SUBAGENT_TOOL_NAME: &str = "WaitSubagent";
pub const INSPECT_SUBAGENT_TOOL_NAME: &str = "InspectSubagent";

#[derive(Clone)]
pub struct SpawnSubagentTool {
    manager: SubagentManager,
    config: AppConfig,
    main_access_mode: AccessMode,
    approvals: WriteApprovalController,
}

#[derive(Clone)]
pub struct WaitSubagentTool {
    manager: SubagentManager,
}

#[derive(Clone)]
pub struct InspectSubagentTool {
    manager: SubagentManager,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubagentAccessModeArg {
    ReadOnly,
    Write,
}

impl SubagentAccessModeArg {
    fn into_access_mode(self) -> AccessMode {
        match self {
            Self::ReadOnly => AccessMode::ReadOnly,
            Self::Write => AccessMode::ReadWrite,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SpawnSubagentArgs {
    pub prompt: String,
    pub access_mode: SubagentAccessModeArg,
}

#[derive(Debug, Deserialize)]
pub struct WaitSubagentArgs {
    pub ids: Vec<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct InspectSubagentArgs {
    pub id: String,
}

impl SpawnSubagentTool {
    pub fn new(
        manager: SubagentManager,
        config: AppConfig,
        main_access_mode: AccessMode,
        approvals: WriteApprovalController,
    ) -> Self {
        Self {
            manager,
            config,
            main_access_mode,
            approvals,
        }
    }
}

impl WaitSubagentTool {
    pub fn new(manager: SubagentManager) -> Self {
        Self { manager }
    }
}

impl InspectSubagentTool {
    pub fn new(manager: SubagentManager) -> Self {
        Self { manager }
    }
}

impl Tool for SpawnSubagentTool {
    const NAME: &'static str = SPAWN_SUBAGENT_TOOL_NAME;
    type Error = ToolExecError;
    type Args = SpawnSubagentArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Start an independent subagent to work in parallel on a delegated task. Subagents start with fresh context, can explore the workspace, and return their own final output for later inspection. After delegating a task to a subagent, prefer to wait on it or inspect it rather than continuing the same task in the main agent. Keep the delegated prompt concise and point the subagent at workspace paths instead of pasting large code blocks or tool output.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The delegated task for the subagent. Include just enough repo and task context for it to work independently. Prefer file paths, symbol names, and acceptance criteria over large pasted content."
                    },
                    "access_mode": {
                        "type": "string",
                        "enum": ["read_only", "write"],
                        "description": "Use read_only for exploration tasks. Use write only when the parent agent is already in write mode."
                    }
                },
                "required": ["prompt", "access_mode"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let access_mode = args.access_mode.into_access_mode();
        if access_mode == AccessMode::ReadWrite && self.main_access_mode != AccessMode::ReadWrite {
            return Err(ToolExecError::new(
                "Cannot start a write-capable subagent while the main agent is in read-only mode",
            ));
        }

        let model_name = &self.config.azure.model_name;
        if let Some(budget) = model_registry::recommended_prompt_token_budget(model_name) {
            let estimated_tokens = estimate_prompt_tokens(&args.prompt);
            if estimated_tokens > budget {
                return Err(ToolExecError::new(format!(
                    "Delegated prompt is too large for subagent model `{model_name}` (estimated {estimated_tokens} tokens, recommended budget {budget}). Pass a concise task and workspace paths instead of large pasted code or tool output."
                )));
            }
        }

        let snapshot = self
            .manager
            .spawn(SubagentSpawnRequest {
                prompt: args.prompt,
                access_mode,
                activity_kind: crate::subagents::SubagentActivityKind::General,
                model_name_override: None,
                config: self.config.clone(),
                approvals: self.approvals.clone(),
            })
            .await
            .map_err(|error| ToolExecError::new(error.to_string()))?;

        render_json(&snapshot)
    }
}

impl Tool for WaitSubagentTool {
    const NAME: &'static str = WAIT_SUBAGENT_TOOL_NAME;
    type Error = ToolExecError;
    type Args = WaitSubagentArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Wait for one or more subagents. Use this after delegating work instead of duplicating the delegated task in the main agent. This waits until a subagent finishes, is cancelled, fails, or has no activity for timeout_ms. Active subagents reset the timeout whenever they emit new activity.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "description": "The subagent IDs to wait on."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Inactivity timeout in milliseconds. Defaults to 30000."
                    }
                },
                "required": ["ids"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let timeout = args.timeout_ms.map(Duration::from_millis);
        let result = self
            .manager
            .wait(&args.ids, timeout)
            .await
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        render_json(&result)
    }
}

impl Tool for InspectSubagentTool {
    const NAME: &'static str = INSPECT_SUBAGENT_TOOL_NAME;
    type Error = ToolExecError;
    type Args = InspectSubagentArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Inspect the current status of a subagent and return its final output if it has completed.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The subagent ID to inspect."
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let snapshot = self
            .manager
            .inspect(&args.id)
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        render_json(&snapshot)
    }
}

fn render_json<T: Serialize>(value: &T) -> Result<String, ToolExecError> {
    serde_json::to_string(value).map_err(|error| ToolExecError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        app::ApprovalMode,
        config::{AzureConfig, ReasoningEffort, SubagentConfig, ToolConfig, UiConfig},
        planning::PlanningConfig,
        stats::StatsStore,
    };

    fn sample_config() -> AppConfig {
        AppConfig {
            azure: AzureConfig {
                resource_name: "demo-resource".into(),
                api_key: "secret".into(),
                model_name: "gpt-5.4-mini".into(),
                reasoning_effort: ReasoningEffort::Medium,
                api_version: "2025-01-01-preview".into(),
            },
            ui: UiConfig::default(),
            subagents: SubagentConfig { max_concurrent: 4 },
            planning: PlanningConfig::default(),
            tools: ToolConfig::default(),
        }
    }

    fn manager() -> SubagentManager {
        let (tx, _rx) = mpsc::unbounded_channel();
        SubagentManager::new(4, tx, StatsStore::new())
    }

    #[tokio::test]
    async fn spawn_tool_rejects_write_mode_when_main_agent_is_read_only() {
        let tool = SpawnSubagentTool::new(
            manager(),
            sample_config(),
            AccessMode::ReadOnly,
            WriteApprovalController::new(ApprovalMode::Manual),
        );

        let error = tool
            .call(SpawnSubagentArgs {
                prompt: "do it".into(),
                access_mode: SubagentAccessModeArg::Write,
            })
            .await
            .expect_err("spawn must fail");

        assert!(
            error
                .to_string()
                .contains("write-capable subagent while the main agent is in read-only mode")
        );
    }

    #[tokio::test]
    async fn inspect_tool_returns_json() {
        let manager = manager();
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        let tool = InspectSubagentTool::new(manager);

        let output = tool
            .call(InspectSubagentArgs {
                id: "subagent-1".into(),
            })
            .await
            .expect("inspect succeeds");

        assert!(output.contains("\"id\":\"subagent-1\""));
        assert!(output.contains("\"status\":\"running\""));
    }

    #[tokio::test(start_paused = true)]
    async fn wait_tool_returns_inactivity_result() {
        let manager = manager();
        manager.register_running_for_test("subagent-1", Duration::from_secs(0));
        let tool = WaitSubagentTool::new(manager.clone());

        let task = tokio::spawn(async move {
            tool.call(WaitSubagentArgs {
                ids: vec!["subagent-1".into()],
                timeout_ms: Some(100),
            })
            .await
        });

        tokio::time::advance(Duration::from_millis(101)).await;
        let output = task.await.expect("join").expect("wait succeeds");
        assert!(output.contains("\"inactive_id\":\"subagent-1\""));
        assert!(output.contains("\"timed_out_on_inactivity\":true"));
    }
}
