use anyhow::{Result, bail};

use std::collections::HashSet;

use crate::{
    features::planning::PlanningAgentConfig,
    model_registry::{self, ModelProvider},
    tool_policy,
};

use super::AppConfig;

pub(super) fn validate(config: &AppConfig) -> Result<()> {
    if config.model.model_name.trim().is_empty() {
        bail!("model.model_name must not be empty");
    }

    if config.safety.model_name.trim().is_empty() {
        bail!("safety.model_name must not be empty");
    }
    if config.memory.extraction.model_name.trim().is_empty() {
        bail!("memory.extraction.model_name must not be empty");
    }

    validate_model_reasoning(
        "model.model_name",
        "model.reasoning",
        &config.model.model_name,
        config.model.reasoning,
        config,
    )?;
    validate_model_reasoning(
        "safety.model_name",
        "safety.reasoning",
        &config.safety.model_name,
        config.safety.reasoning,
        config,
    )?;
    validate_model_reasoning(
        "memory.extraction.model_name",
        "memory.extraction.reasoning",
        &config.memory.extraction.model_name,
        config.memory.extraction.reasoning,
        config,
    )?;

    if config.subagents.max_concurrent == 0 {
        bail!("subagents.max_concurrent must be at least 1");
    }

    if config.memory.auto_inject_token_budget == 0 {
        bail!("memory.auto_inject_token_budget must be at least 1");
    }
    if config.memory.max_auto_results == 0 {
        bail!("memory.max_auto_results must be at least 1");
    }
    if config.memory.max_candidate_search_results == 0 {
        bail!("memory.max_candidate_search_results must be at least 1");
    }
    if config.memory.extraction.max_evidence_tokens == 0 {
        bail!("memory.extraction.max_evidence_tokens must be at least 1");
    }
    if config.memory.extraction.max_related_memories == 0 {
        bail!("memory.extraction.max_related_memories must be at least 1");
    }
    if config.memory.extraction.max_candidates_per_turn == 0 {
        bail!("memory.extraction.max_candidates_per_turn must be at least 1");
    }
    if config.memory.extraction.min_candidate_confidence > 100 {
        bail!("memory.extraction.min_candidate_confidence must be between 0 and 100");
    }
    if config.memory.extraction.min_active_confidence > 100 {
        bail!("memory.extraction.min_active_confidence must be between 0 and 100");
    }
    if config.memory.extraction.min_active_confidence
        < config.memory.extraction.min_candidate_confidence
    {
        bail!(
            "memory.extraction.min_active_confidence must be greater than or equal to memory.extraction.min_candidate_confidence"
        );
    }

    validate_planning_agents(&config.model.model_name, &config.planning.agents, config)?;

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
    config: &AppConfig,
) -> Result<()> {
    let Some(model) = model_registry::find_model(model_name) else {
        bail!(
            "{}",
            model_registry::unknown_model_message(model_field_name, model_name)
        );
    };

    validate_provider_credentials(config, model.provider, model_name)?;

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
    config: &AppConfig,
) -> Result<()> {
    let mut seen = HashSet::new();

    for agent in agents {
        if agent.model_name == current_main_model {
            bail!(
                "planning.agents must not include the current model.model_name `{current_main_model}`"
            );
        }

        validate_model_reasoning(
            "planning.agents[].model_name",
            "planning.agents[].reasoning",
            &agent.model_name,
            agent.reasoning,
            config,
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

fn validate_provider_credentials(
    config: &AppConfig,
    provider: ModelProvider,
    model_name: &str,
) -> Result<()> {
    match provider {
        ModelProvider::AzureOpenAi => {
            let Some(azure) = config.azure.as_ref() else {
                bail!("config is missing the [azure] table required for model `{model_name}`");
            };
            if azure.resource_name.trim().is_empty() {
                bail!("azure.resource_name must not be empty");
            }
            if azure.api_key.trim().is_empty() {
                bail!("azure.api_key must not be empty");
            }
        }
        ModelProvider::ChutesAi => {
            let Some(chutes) = config.chutes.as_ref() else {
                bail!("config is missing the [chutes] table required for model `{model_name}`");
            };
            if chutes.api_key.trim().is_empty() {
                bail!("chutes.api_key must not be empty");
            }
        }
        ModelProvider::Codex => {
            if let Some(codex) = config.codex.as_ref()
                && let Some(mode) = codex.resolved_auth_mode()
            {
                match mode {
                    crate::config::CodexAuthMode::ApiKey => {
                        if codex.auth_token().is_none() {
                            bail!(
                                "codex.OPENAI_API_KEY must not be empty when codex.auth_mode = \"api_key\""
                            );
                        }
                    }
                    crate::config::CodexAuthMode::Chatgpt
                    | crate::config::CodexAuthMode::ChatgptAuthTokens => {
                        if codex.auth_token().is_none() {
                            bail!(
                                "codex.access_token must not be empty when codex.auth_mode uses ChatGPT tokens"
                            );
                        }
                    }
                }
            }
        }
        ModelProvider::OpenRouter => {
            let Some(openrouter) = config.openrouter.as_ref() else {
                bail!("config is missing the [openrouter] table required for model `{model_name}`");
            };
            if openrouter.api_key.trim().is_empty() {
                bail!("openrouter.api_key must not be empty");
            }
        }
    }

    Ok(())
}
