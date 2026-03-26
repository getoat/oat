use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};

use crate::{
    features::planning::{PlanningAgentConfig, PlanningConfig},
    model_registry::{self, ParseReasoningSettingError},
    tool_policy,
};

use super::types::{
    AppConfig, AzureConfig, RawReasoningSetting, SafetyConfig, SubagentConfig, ToolConfig,
    UiConfig, default_api_version, default_command_history_limit, default_max_concurrent_subagents,
    default_show_thinking,
};

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct PartialAppConfig {
    azure: Option<PartialAzureConfig>,
    safety: Option<PartialSafetyConfig>,
    ui: Option<PartialUiConfig>,
    subagents: Option<PartialSubagentConfig>,
    planning: Option<PartialPlanningConfig>,
    tools: Option<PartialToolConfig>,
}

impl PartialAppConfig {
    pub(super) fn merge(&mut self, other: Self) {
        if let Some(azure) = other.azure {
            self.azure
                .get_or_insert_with(PartialAzureConfig::default)
                .merge(azure);
        }

        if let Some(ui) = other.ui {
            self.ui
                .get_or_insert_with(PartialUiConfig::default)
                .merge(ui);
        }

        if let Some(safety) = other.safety {
            self.safety
                .get_or_insert_with(PartialSafetyConfig::default)
                .merge(safety);
        }

        if let Some(subagents) = other.subagents {
            self.subagents
                .get_or_insert_with(PartialSubagentConfig::default)
                .merge(subagents);
        }

        if let Some(planning) = other.planning {
            self.planning
                .get_or_insert_with(PartialPlanningConfig::default)
                .merge(planning);
        }

        if let Some(tools) = other.tools {
            self.tools
                .get_or_insert_with(PartialToolConfig::default)
                .merge(tools);
        }
    }

    pub(super) fn finalize(self) -> Result<AppConfig> {
        let azure = self
            .azure
            .context("config is missing the [azure] table")?
            .finalize()?;
        Ok(AppConfig {
            safety: self.safety.unwrap_or_default().finalize(&azure)?,
            azure,
            ui: self.ui.unwrap_or_default().finalize(),
            subagents: self.subagents.unwrap_or_default().finalize(),
            planning: self.planning.unwrap_or_default().finalize(),
            tools: self.tools.unwrap_or_default().finalize(),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct PartialAzureConfig {
    resource_name: Option<String>,
    api_key: Option<String>,
    model_name: Option<String>,
    reasoning: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPartialAzureConfig {
    resource_name: Option<String>,
    api_key: Option<String>,
    model_name: Option<String>,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
    api_version: Option<String>,
}

impl<'de> Deserialize<'de> for PartialAzureConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPartialAzureConfig::deserialize(deserializer)?;
        Ok(Self {
            resource_name: raw.resource_name,
            api_key: raw.api_key,
            model_name: raw.model_name,
            reasoning: raw.reasoning_fields.resolve(),
            api_version: raw.api_version,
        })
    }
}

impl PartialAzureConfig {
    fn merge(&mut self, other: Self) {
        if other.resource_name.is_some() {
            self.resource_name = other.resource_name;
        }
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
        if other.model_name.is_some() {
            self.model_name = other.model_name;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
        if other.api_version.is_some() {
            self.api_version = other.api_version;
        }
    }

    fn finalize(self) -> Result<AzureConfig> {
        let model_name = self
            .model_name
            .context("config is missing azure.model_name")?;
        let reasoning = parse_reasoning_value(
            "azure.model_name",
            "azure.reasoning",
            &model_name,
            self.reasoning
                .context("config is missing azure.reasoning")?,
        )?;
        Ok(AzureConfig {
            resource_name: self
                .resource_name
                .context("config is missing azure.resource_name")?,
            api_key: self.api_key.context("config is missing azure.api_key")?,
            model_name,
            reasoning,
            api_version: self.api_version.unwrap_or_else(default_api_version),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialUiConfig {
    show_thinking: Option<bool>,
    show_tool_output: Option<bool>,
    command_history_limit: Option<usize>,
}

impl PartialUiConfig {
    fn merge(&mut self, other: Self) {
        if other.show_thinking.is_some() {
            self.show_thinking = other.show_thinking;
        }
        if other.show_tool_output.is_some() {
            self.show_tool_output = other.show_tool_output;
        }
        if other.command_history_limit.is_some() {
            self.command_history_limit = other.command_history_limit;
        }
    }

    fn finalize(self) -> UiConfig {
        UiConfig {
            show_thinking: self.show_thinking.unwrap_or_else(default_show_thinking),
            show_tool_output: self.show_tool_output.unwrap_or(false),
            command_history_limit: self
                .command_history_limit
                .unwrap_or_else(default_command_history_limit),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialSubagentConfig {
    max_concurrent: Option<usize>,
}

impl PartialSubagentConfig {
    fn merge(&mut self, other: Self) {
        if other.max_concurrent.is_some() {
            self.max_concurrent = other.max_concurrent;
        }
    }

    fn finalize(self) -> SubagentConfig {
        SubagentConfig {
            max_concurrent: self
                .max_concurrent
                .unwrap_or_else(default_max_concurrent_subagents),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialPlanningConfig {
    agents: Option<Vec<PlanningAgentConfig>>,
}

impl PartialPlanningConfig {
    fn merge(&mut self, other: Self) {
        if other.agents.is_some() {
            self.agents = other.agents;
        }
    }

    fn finalize(self) -> PlanningConfig {
        PlanningConfig {
            agents: self.agents.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialToolConfig {
    search_include_patterns: Option<Vec<String>>,
    max_output_tokens: Option<usize>,
}

impl PartialToolConfig {
    fn merge(&mut self, other: Self) {
        if other.search_include_patterns.is_some() {
            self.search_include_patterns = other.search_include_patterns;
        }
        if other.max_output_tokens.is_some() {
            self.max_output_tokens = other.max_output_tokens;
        }
    }

    fn finalize(self) -> ToolConfig {
        ToolConfig {
            search_include_patterns: self.search_include_patterns.unwrap_or_default(),
            max_output_tokens: self
                .max_output_tokens
                .unwrap_or_else(tool_policy::default_tool_output_max_tokens),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PartialSafetyConfig {
    model_name: Option<String>,
    reasoning: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPartialSafetyConfig {
    model_name: Option<String>,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
}

impl<'de> Deserialize<'de> for PartialSafetyConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPartialSafetyConfig::deserialize(deserializer)?;
        Ok(Self {
            model_name: raw.model_name,
            reasoning: raw.reasoning_fields.resolve(),
        })
    }
}

impl PartialSafetyConfig {
    fn merge(&mut self, other: Self) {
        if other.model_name.is_some() {
            self.model_name = other.model_name;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
    }

    fn finalize(self, azure: &AzureConfig) -> Result<SafetyConfig> {
        let model_name = self.model_name.unwrap_or_else(|| azure.model_name.clone());
        let reasoning = self
            .reasoning
            .map(|value| {
                parse_reasoning_value("safety.model_name", "safety.reasoning", &model_name, value)
            })
            .transpose()?
            .unwrap_or(azure.reasoning);
        Ok(SafetyConfig {
            model_name,
            reasoning,
        })
    }
}

fn parse_reasoning_value(
    model_field_name: &str,
    field_name: &str,
    model_name: &str,
    value: String,
) -> Result<super::ReasoningSetting> {
    match model_registry::parse_reasoning_setting_for_model(model_name, &value) {
        Ok(reasoning) => Ok(reasoning),
        Err(ParseReasoningSettingError::UnknownModel) => Err(anyhow::anyhow!(
            model_registry::unknown_model_message(model_field_name, model_name,)
        )),
        Err(error) => Err(anyhow::anyhow!(
            error.message(field_name, model_name, &value)
        )),
    }
}
