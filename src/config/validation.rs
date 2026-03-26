use anyhow::{Result, bail};

use std::collections::HashSet;

use crate::{features::planning::PlanningAgentConfig, model_registry, tool_policy};

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

    validate_model_reasoning(
        "azure.model_name",
        "azure.reasoning",
        &config.azure.model_name,
        config.azure.reasoning,
    )?;
    validate_model_reasoning(
        "safety.model_name",
        "safety.reasoning",
        &config.safety.model_name,
        config.safety.reasoning,
    )?;

    if config.subagents.max_concurrent == 0 {
        bail!("subagents.max_concurrent must be at least 1");
    }

    validate_planning_agents(&config.azure.model_name, &config.planning.agents)?;

    if config.tools.max_output_tokens == 0 {
        bail!("tools.max_output_tokens must be at least 1");
    }

    tool_policy::SearchPathPolicy::validate_patterns(&config.tools.search_include_patterns)?;

    Ok(())
}

fn validate_model_reasoning(
    model_field_name: &str,
    reasoning_field_name: &str,
    model_name: &str,
    reasoning: super::ReasoningSetting,
) -> Result<()> {
    let Some(model) = model_registry::find_model(model_name) else {
        bail!(
            "{}",
            model_registry::unknown_model_message(model_field_name, model_name)
        );
    };

    if !model.supports_reasoning(reasoning) {
        bail!(
            "{}",
            model_registry::ParseReasoningSettingError::UnsupportedForModel {
                supported: model.supported_reasoning_settings,
            }
            .message(reasoning_field_name, model_name, reasoning.as_str())
        );
    }

    Ok(())
}

fn validate_planning_agents(
    current_main_model: &str,
    agents: &[PlanningAgentConfig],
) -> Result<()> {
    let mut seen = HashSet::new();

    for agent in agents {
        if agent.model_name == current_main_model {
            bail!(
                "planning.agents must not include the current azure.model_name `{current_main_model}`"
            );
        }

        validate_model_reasoning(
            "planning.agents[].model_name",
            "planning.agents[].reasoning",
            &agent.model_name,
            agent.reasoning,
        )?;

        if !seen.insert(agent.model_name.as_str()) {
            bail!(
                "planning.agents contains duplicate model `{}`; each model may appear at most once",
                agent.model_name
            );
        }
    }

    Ok(())
}
