use std::path::PathBuf;

use rig::tool::Tool;
use rig::tool::ToolDyn;

use crate::{
    agent::{AgentContext, AgentRole},
    app::{AccessMode, ApprovalMode},
    config::AppConfig,
    subagents::SubagentManager,
    tool_policy::{SearchPathPolicy, ToolOutputPolicy},
};

use super::{
    ApplyPatchesTool, AskUserTool, CommentaryTool, DeletePathTool, GrepTool,
    INSPECT_SUBAGENT_TOOL_NAME, InspectSubagentTool, ListTool, ReadFileTool, ReadFilesTool,
    RunShellScriptTool, SPAWN_SUBAGENT_TOOL_NAME, SpawnSubagentTool, WAIT_SUBAGENT_TOOL_NAME,
    WaitSubagentTool, WriteFileTool, output_limit::OutputLimitedTool,
};

#[derive(Clone)]
pub struct ToolContext {
    pub root: PathBuf,
    pub agent: AgentContext,
    pub config: AppConfig,
    pub approval_mode: ApprovalMode,
    pub ask_user_available: bool,
    pub subagents: Option<SubagentManager>,
}

impl ToolContext {
    fn search_policy(&self) -> SearchPathPolicy {
        SearchPathPolicy::new(&self.config.tools.search_include_patterns)
            .expect("config validation ensures valid search include patterns")
    }
}

#[derive(Clone, Copy)]
struct ToolDescriptor {
    name: &'static str,
    access_mode: ToolAccess,
    role_scope: ToolRoleScope,
    constructor: fn(ToolContext) -> Box<dyn ToolDyn>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ToolAccess {
    ReadOnly,
    Mutation,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ToolRoleScope {
    Any,
    MainOnly,
    MainWithManager,
}

const TOOL_DESCRIPTORS: [ToolDescriptor; 13] = [
    ToolDescriptor::read_only(AskUserTool::NAME, ToolRoleScope::MainOnly, |_context| {
        Box::new(AskUserTool)
    }),
    ToolDescriptor::read_only(CommentaryTool::NAME, ToolRoleScope::Any, |_context| {
        Box::new(CommentaryTool)
    }),
    ToolDescriptor::read_only(ListTool::NAME, ToolRoleScope::Any, |context| {
        let search_policy = context.search_policy();
        Box::new(ListTool::new(context.root, search_policy))
    }),
    ToolDescriptor::read_only(ReadFileTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(ReadFileTool::new(context.root))
    }),
    ToolDescriptor::read_only(ReadFilesTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(ReadFilesTool::new(context.root))
    }),
    ToolDescriptor::read_only(GrepTool::NAME, ToolRoleScope::Any, |context| {
        let search_policy = context.search_policy();
        Box::new(GrepTool::new(context.root, search_policy))
    }),
    ToolDescriptor::read_only(RunShellScriptTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(RunShellScriptTool::new(context.root))
    }),
    ToolDescriptor::mutation(ApplyPatchesTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(ApplyPatchesTool::new(context.root))
    }),
    ToolDescriptor::mutation(WriteFileTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(WriteFileTool::new(context.root))
    }),
    ToolDescriptor::mutation(DeletePathTool::NAME, ToolRoleScope::Any, |context| {
        Box::new(DeletePathTool::new(context.root))
    }),
    ToolDescriptor::read_only(
        SPAWN_SUBAGENT_TOOL_NAME,
        ToolRoleScope::MainWithManager,
        |context| {
            Box::new(SpawnSubagentTool::new(
                context
                    .subagents
                    .expect("main agent subagent tools require a manager"),
                context.config,
                context.agent.access_mode,
                context.approval_mode,
            ))
        },
    ),
    ToolDescriptor::read_only(
        WAIT_SUBAGENT_TOOL_NAME,
        ToolRoleScope::MainWithManager,
        |context| {
            Box::new(WaitSubagentTool::new(
                context
                    .subagents
                    .expect("main agent subagent tools require a manager"),
            ))
        },
    ),
    ToolDescriptor::read_only(
        INSPECT_SUBAGENT_TOOL_NAME,
        ToolRoleScope::MainWithManager,
        |context| {
            Box::new(InspectSubagentTool::new(
                context
                    .subagents
                    .expect("main agent subagent tools require a manager"),
            ))
        },
    ),
];

impl ToolDescriptor {
    const fn read_only(
        name: &'static str,
        role_scope: ToolRoleScope,
        constructor: fn(ToolContext) -> Box<dyn ToolDyn>,
    ) -> Self {
        Self {
            name,
            access_mode: ToolAccess::ReadOnly,
            role_scope,
            constructor,
        }
    }

    const fn mutation(
        name: &'static str,
        role_scope: ToolRoleScope,
        constructor: fn(ToolContext) -> Box<dyn ToolDyn>,
    ) -> Self {
        Self {
            name,
            access_mode: ToolAccess::Mutation,
            role_scope,
            constructor,
        }
    }

    fn is_enabled(self, context: &ToolContext) -> bool {
        if self.name == AskUserTool::NAME && !context.ask_user_available {
            return false;
        }
        let access_enabled = self.access_mode == ToolAccess::ReadOnly
            || context.agent.access_mode == AccessMode::ReadWrite;
        let role_enabled = match self.role_scope {
            ToolRoleScope::Any => true,
            ToolRoleScope::MainOnly => context.agent.role == AgentRole::Main,
            ToolRoleScope::MainWithManager => {
                context.agent.role == AgentRole::Main && context.subagents.is_some()
            }
        };

        access_enabled && role_enabled
    }
}

pub fn tool_names_for_context(context: &ToolContext) -> Vec<String> {
    TOOL_DESCRIPTORS
        .into_iter()
        .filter(|tool| tool.is_enabled(context))
        .map(|tool| tool.name.to_string())
        .collect()
}

pub fn tools_for_context(context: ToolContext) -> Vec<Box<dyn ToolDyn>> {
    let output_policy = ToolOutputPolicy::new(context.config.tools.max_output_tokens)
        .expect("config validation ensures a usable tool output tokenizer");
    TOOL_DESCRIPTORS
        .into_iter()
        .filter(|tool| tool.is_enabled(&context))
        .map(|tool| {
            Box::new(OutputLimitedTool::new(
                (tool.constructor)(context.clone()),
                output_policy.clone(),
            )) as Box<dyn ToolDyn>
        })
        .collect()
}

pub fn is_mutation_tool(tool_name: &str) -> bool {
    TOOL_DESCRIPTORS.iter().any(|tool| {
        tool.access_mode == ToolAccess::Mutation && tool.name.eq_ignore_ascii_case(tool_name)
    })
}

#[cfg(test)]
fn is_shell_tool(tool_name: &str) -> bool {
    tool_name.eq_ignore_ascii_case(RunShellScriptTool::NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_exposes_only_read_tools() {
        let tool_names = tool_names_for_context(&ToolContext {
            root: PathBuf::from("."),
            agent: AgentContext::main(AccessMode::ReadOnly),
            config: sample_config(),
            approval_mode: ApprovalMode::Manual,
            ask_user_available: true,
            subagents: Some(test_subagent_manager()),
        });

        assert_eq!(
            tool_names,
            vec![
                "AskUser",
                "Commentary",
                "List",
                "ReadFile",
                "ReadFiles",
                "Grep",
                "RunShellScript",
                "SpawnSubagent",
                "WaitSubagent",
                "InspectSubagent"
            ]
        );
    }

    #[test]
    fn read_write_mode_exposes_all_tools() {
        let tool_names = tool_names_for_context(&ToolContext {
            root: PathBuf::from("."),
            agent: AgentContext::main(AccessMode::ReadWrite),
            config: sample_config(),
            approval_mode: ApprovalMode::Manual,
            ask_user_available: true,
            subagents: Some(test_subagent_manager()),
        });

        assert!(tool_names.contains(&"AskUser".to_string()));
        assert!(tool_names.contains(&"Commentary".to_string()));
        assert!(tool_names.contains(&"RunShellScript".to_string()));
        assert!(tool_names.contains(&"ApplyPatches".to_string()));
        assert!(tool_names.contains(&"WriteFile".to_string()));
        assert!(tool_names.contains(&"DeletePath".to_string()));
        assert!(tool_names.contains(&"SpawnSubagent".to_string()));
    }

    #[test]
    fn mutation_classification_matches_write_tools() {
        for tool_name in tool_names_for_context(&ToolContext {
            root: PathBuf::from("."),
            agent: AgentContext::main(AccessMode::ReadOnly),
            config: sample_config(),
            approval_mode: ApprovalMode::Manual,
            ask_user_available: true,
            subagents: Some(test_subagent_manager()),
        }) {
            assert!(
                !is_mutation_tool(&tool_name),
                "{tool_name} should be read-only"
            );
        }

        for tool_name in ["ApplyPatches", "WriteFile", "DeletePath"] {
            assert!(is_mutation_tool(tool_name), "{tool_name} should be mutable");
        }
        assert!(is_shell_tool("RunShellScript"));
    }

    #[test]
    fn subagents_do_not_get_subagent_tools() {
        let tool_names = tool_names_for_context(&ToolContext {
            root: PathBuf::from("."),
            agent: AgentContext::subagent(AccessMode::ReadOnly, None),
            config: sample_config(),
            approval_mode: ApprovalMode::Manual,
            ask_user_available: true,
            subagents: Some(test_subagent_manager()),
        });

        assert!(!tool_names.contains(&"AskUser".to_string()));
        assert!(!tool_names.contains(&"SpawnSubagent".to_string()));
        assert!(!tool_names.contains(&"WaitSubagent".to_string()));
        assert!(!tool_names.contains(&"InspectSubagent".to_string()));
    }

    #[test]
    fn main_agent_without_manager_omits_subagent_tools() {
        let tool_names = tool_names_for_context(&ToolContext {
            root: PathBuf::from("."),
            agent: AgentContext::main(AccessMode::ReadOnly),
            config: sample_config(),
            approval_mode: ApprovalMode::Manual,
            ask_user_available: false,
            subagents: None,
        });

        assert_eq!(
            tool_names,
            vec![
                "Commentary",
                "List",
                "ReadFile",
                "ReadFiles",
                "Grep",
                "RunShellScript"
            ]
        );
    }

    fn sample_config() -> AppConfig {
        AppConfig {
            azure: Some(crate::config::AzureConfig {
                resource_name: "demo-resource".into(),
                api_key: "secret".into(),
                api_version: "2025-01-01-preview".into(),
            }),
            chutes: None,
            model: crate::config::ModelSelectionConfig {
                model_name: "gpt-5.4-mini".into(),
                reasoning: crate::config::ReasoningEffort::Medium.into(),
            },
            safety: crate::config::SafetyConfig {
                model_name: "gpt-5.4-mini".into(),
                reasoning: crate::config::ReasoningEffort::Medium.into(),
            },
            ui: crate::config::UiConfig::default(),
            subagents: crate::config::SubagentConfig { max_concurrent: 4 },
            planning: crate::features::planning::PlanningConfig::default(),
            tools: crate::config::ToolConfig::default(),
        }
    }

    fn test_subagent_manager() -> SubagentManager {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        SubagentManager::new(4, tx, crate::stats::StatsStore::new())
    }
}
