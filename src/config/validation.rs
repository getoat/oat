use anyhow::{Result, bail};

use crate::{features::planning::sanitize_planning_agents, model_registry, tool_policy};

use super::AppConfig;

pub(super) fn validate(config: &AppConfig) -> Result<()> {
    if config.azure.resource_name.trim().is_empty() {
        bail!("azure.resource_name must not be empty");
    }

    if config.azure.api_key.trim().is_empty() {
        bail!("azure.api_key must not be empty");
    }

    if config.azure.model_name.trim().is_empty() {
        bail!("azure.model_name must not be empty");
    }

    if config.safety.model_name.trim().is_empty() {
        bail!("safety.model_name must not be empty");
    }

    if let Some(model) = model_registry::find_model(&config.azure.model_name)
        && !model.supports_reasoning(config.azure.reasoning_effort)
    {
        let supported = model
            .supported_reasoning_levels
            .iter()
            .map(|level| level.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "azure.reasoning_effort `{}` is not supported by model `{}`. Supported values: {supported}",
            config.azure.reasoning_effort.as_str(),
            config.azure.model_name
        );
    }

    if let Some(model) = model_registry::find_model(&config.safety.model_name)
        && !model.supports_reasoning(config.safety.reasoning_effort)
    {
        let supported = model
            .supported_reasoning_levels
            .iter()
            .map(|level| level.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "safety.reasoning_effort `{}` is not supported by model `{}`. Supported values: {supported}",
            config.safety.reasoning_effort.as_str(),
            config.safety.model_name
        );
    }

    if config.subagents.max_concurrent == 0 {
        bail!("subagents.max_concurrent must be at least 1");
    }

    let sanitized = sanitize_planning_agents(&config.azure.model_name, &config.planning.agents);
    if sanitized.len() != config.planning.agents.len() {
        bail!(
            "planning.agents must reference unique registry models other than the current azure.model_name, and each selected reasoning_effort must be supported by that model"
        );
    }

    if config.tools.max_output_tokens == 0 {
        bail!("tools.max_output_tokens must be at least 1");
    }

    tool_policy::SearchPathPolicy::validate_patterns(&config.tools.search_include_patterns)?;

    Ok(())
}
