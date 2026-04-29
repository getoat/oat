mod partial;
mod paths;
#[cfg(test)]
mod tests;
mod types;
mod updates;
mod validation;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};

use crate::{
    features::planning::{PlanningAgentConfig, sanitize_planning_agents},
    model_registry,
};
use partial::PartialAppConfig;
pub(crate) use paths::default_config_locations;
use paths::{default_config_update_path, default_home_config_path};
#[allow(unused_imports)]
pub(crate) use types::CriticConfig;
#[cfg(test)]
use types::default_api_version;
pub(crate) use types::{
    AppConfig, CodexAuthMode, CodexConfig, HistoryMode, KimiThinkingMode, MemoryConfig,
    MemoryExtractionConfig, RawReasoningSetting, ReasoningEffort, WebSearchMode,
};
#[cfg(test)]
pub(crate) use types::{
    AzureConfig, HistoryConfig, OllamaConfig, OpenRouterConfig, OpencodeConfig, SafetyConfig,
    SubagentConfig, ToolConfig, UiConfig,
};
pub use types::{ModelSelectionConfig, ReasoningSetting};
use updates::{write_codex_auth_updates_at_path, write_config_updates_at_path};

#[derive(Debug, Clone, Default)]
pub struct RuntimeConfigOverrides {
    pub model_selection: Option<ModelSelectionConfig>,
    pub planning_agents: Option<Vec<PlanningAgentConfig>>,
}

const DEFAULT_CONFIG_PATH: &str = "config.toml";
const HOME_CONFIG_RELATIVE_PATH: &str = ".config/oat/config.toml";
const DEFAULT_API_VERSION: &str = "2025-01-01-preview";
const DEFAULT_MODEL_NAME: &str = "gpt-5.4-mini";
const DEFAULT_OLLAMA_MODEL_NAME: &str = "glm-5.1:cloud";
const DEFAULT_OPENCODE_MODEL_NAME: &str = "opencode-go/glm-5.1";
const DEFAULT_OPENROUTER_MODEL_NAME: &str = "openai/gpt-5.4-mini";
const DEFAULT_CHUTES_MODEL_NAME: &str = "zai-org/GLM-5-TEE";
const DEFAULT_CODEX_MODEL_NAME: &str = "codex/gpt-5.4-mini";
const DEFAULT_MODEL_REASONING: ReasoningSetting = ReasoningSetting::Gpt(ReasoningEffort::Medium);

impl<'de> Deserialize<'de> for AppConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let partial = PartialAppConfig::deserialize(deserializer)?;
        partial.finalize().map_err(serde::de::Error::custom)
    }
}

impl AppConfig {
    pub fn load_from_default_path() -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        Self::load_from_paths(home_path.as_deref(), Some(cwd_path.as_path()))
    }

    pub fn load_from_paths(home_path: Option<&Path>, cwd_path: Option<&Path>) -> Result<Self> {
        let mut config = PartialAppConfig::default();
        let mut loaded_any = false;
        let default_fallback = (
            DEFAULT_MODEL_NAME.to_string(),
            default_model_reasoning_string(),
        );
        let mut fallback = default_fallback.clone();

        if let Some(path) = home_path.filter(|path| path.exists()) {
            config.merge(Self::load_partial_from_path(
                path,
                &fallback.0,
                &fallback.1,
            )?);
            fallback = config
                .model_selection_hint()
                .unwrap_or_else(|| default_fallback.clone());
            loaded_any = true;
        }

        if let Some(path) = cwd_path.filter(|path| path.exists()) {
            config.merge(Self::load_partial_from_path(
                path,
                &fallback.0,
                &fallback.1,
            )?);
            loaded_any = true;
        }

        if !loaded_any {
            bail!(
                "failed to read config. Create {} from config.example.toml",
                default_config_locations(home_path, cwd_path).join(" or ")
            );
        }

        let config = config.finalize()?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        let fallback_reasoning = default_model_reasoning_string();
        let config = Self::load_partial_from_path(path, DEFAULT_MODEL_NAME, &fallback_reasoning)?
            .finalize()?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_from_runtime_path(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(path) => Self::load_from_path(path),
            None => Self::load_from_default_path(),
        }
    }

    fn load_partial_from_path(
        path: &Path,
        fallback_model_name: &str,
        fallback_reasoning: &str,
    ) -> Result<PartialAppConfig> {
        let raw = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read {}. Create it from config.example.toml",
                path.display()
            )
        })?;

        let mut value: toml::Value =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        let sanitized =
            sanitize_unknown_model_references(&mut value, fallback_model_name, fallback_reasoning);
        let source = if sanitized {
            toml::to_string(&value)
                .with_context(|| format!("failed to normalize {}", path.display()))?
        } else {
            raw
        };

        toml::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
    }
}

fn sanitize_unknown_model_references(
    value: &mut toml::Value,
    fallback_model_name: &str,
    fallback_reasoning: &str,
) -> bool {
    let Some(root) = value.as_table_mut() else {
        return false;
    };

    let fallback_model_name = fallback_model_name.to_string();
    let fallback_reasoning = fallback_reasoning.to_string();

    let model_table_fallback = root
        .get("model")
        .and_then(toml::Value::as_table)
        .and_then(|table| {
            table_string(table, "model_name")
                .filter(|model_name| model_registry::find_model(model_name).is_none())
                .map(|model_name| {
                    resolve_unknown_model_fallback(
                        root,
                        &fallback_model_name,
                        &fallback_reasoning,
                        &model_name,
                    )
                })
        })
        .unwrap_or_else(|| (fallback_model_name.clone(), fallback_reasoning.clone()));
    let azure_table_fallback = root
        .get("azure")
        .and_then(toml::Value::as_table)
        .and_then(|table| {
            table_string(table, "model_name")
                .filter(|model_name| model_registry::find_model(model_name).is_none())
                .map(|model_name| {
                    resolve_unknown_model_fallback(
                        root,
                        &fallback_model_name,
                        &fallback_reasoning,
                        &model_name,
                    )
                })
        })
        .unwrap_or_else(|| (fallback_model_name.clone(), fallback_reasoning.clone()));

    let (current_model_name, current_reasoning, mut changed) = if let Some(model_table) =
        root.get_mut("model").and_then(toml::Value::as_table_mut)
    {
        let changed = sanitize_model_selection_table(
            model_table,
            &model_table_fallback.0,
            &model_table_fallback.1,
        );
        (
            table_string(model_table, "model_name")
                .unwrap_or_else(|| model_table_fallback.0.clone()),
            table_reasoning_string(model_table).unwrap_or_else(|| model_table_fallback.1.clone()),
            changed,
        )
    } else if let Some(azure_table) = root.get_mut("azure").and_then(toml::Value::as_table_mut) {
        let changed = sanitize_model_selection_table(
            azure_table,
            &azure_table_fallback.0,
            &azure_table_fallback.1,
        );
        (
            table_string(azure_table, "model_name")
                .unwrap_or_else(|| azure_table_fallback.0.clone()),
            table_reasoning_string(azure_table).unwrap_or_else(|| azure_table_fallback.1.clone()),
            changed,
        )
    } else {
        (
            fallback_model_name.clone(),
            fallback_reasoning.clone(),
            false,
        )
    };

    if let Some(safety_table) = root.get_mut("safety").and_then(toml::Value::as_table_mut) {
        changed |=
            sanitize_model_selection_table(safety_table, &current_model_name, &current_reasoning);
    }

    if let Some(extraction_table) = root
        .get_mut("memory")
        .and_then(toml::Value::as_table_mut)
        .and_then(|memory| memory.get_mut("extraction"))
        .and_then(toml::Value::as_table_mut)
    {
        changed |= sanitize_model_selection_table(
            extraction_table,
            &current_model_name,
            &current_reasoning,
        );
    }

    if let Some(agents) = root
        .get_mut("planning")
        .and_then(toml::Value::as_table_mut)
        .and_then(|planning| planning.get_mut("agents"))
        .and_then(toml::Value::as_array_mut)
    {
        changed |= sanitize_planning_agents_value(agents, &current_model_name);
    }

    changed
}

fn sanitize_model_selection_table(
    table: &mut toml::value::Table,
    fallback_model_name: &str,
    fallback_reasoning: &str,
) -> bool {
    let Some(model_name) = table_string(table, "model_name") else {
        return false;
    };

    if model_registry::find_model(&model_name).is_none() {
        set_model_selection(table, fallback_model_name, fallback_reasoning);
        return true;
    }

    let default_reasoning = preferred_reasoning_string(&model_name);
    match table_reasoning_string(table) {
        Some(reasoning)
            if model_registry::parse_reasoning_setting_for_model(&model_name, &reasoning)
                .is_ok() =>
        {
            false
        }
        _ => {
            set_reasoning(table, &default_reasoning);
            true
        }
    }
}

fn sanitize_planning_agents_value(agents: &mut Vec<toml::Value>, current_main_model: &str) -> bool {
    let original_len = agents.len();
    let mut changed = false;
    let mut seen = HashSet::new();
    let mut sanitized = Vec::with_capacity(agents.len());

    for mut agent in std::mem::take(agents) {
        let Some(table) = agent.as_table_mut() else {
            changed = true;
            continue;
        };
        let Some(model_name) = table_string(table, "model_name") else {
            changed = true;
            continue;
        };
        if model_name == current_main_model
            || model_registry::find_model(&model_name).is_none()
            || !seen.insert(model_name.clone())
        {
            changed = true;
            continue;
        }

        let default_reasoning = preferred_reasoning_string(&model_name);
        match table_reasoning_string(table) {
            Some(reasoning)
                if model_registry::parse_reasoning_setting_for_model(&model_name, &reasoning)
                    .is_ok() => {}
            _ => {
                set_reasoning(table, &default_reasoning);
                changed = true;
            }
        }

        sanitized.push(agent);
    }

    if sanitized.len() != original_len {
        changed = true;
    }
    *agents = sanitized;
    changed
}

fn table_string(table: &toml::value::Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(ToString::to_string)
}

fn table_reasoning_string(table: &toml::value::Table) -> Option<String> {
    table_string(table, "reasoning").or_else(|| table_string(table, "reasoning_effort"))
}

fn set_model_selection(table: &mut toml::value::Table, model_name: &str, reasoning: &str) {
    table.insert(
        "model_name".into(),
        toml::Value::String(model_name.to_string()),
    );
    set_reasoning(table, reasoning);
}

fn set_reasoning(table: &mut toml::value::Table, reasoning: &str) {
    table.insert(
        "reasoning".into(),
        toml::Value::String(reasoning.to_string()),
    );
    table.remove("reasoning_effort");
}

fn default_model_reasoning_string() -> String {
    DEFAULT_MODEL_REASONING.as_str().to_string()
}

fn preferred_reasoning_string(model_name: &str) -> String {
    model_registry::reasoning_settings_for_model(model_name)
        .and_then(|settings| {
            settings
                .iter()
                .copied()
                .find(|setting| *setting == DEFAULT_MODEL_REASONING)
                .or_else(|| settings.first().copied())
        })
        .unwrap_or(DEFAULT_MODEL_REASONING)
        .as_str()
        .to_string()
}

fn resolve_unknown_model_fallback(
    root: &toml::value::Table,
    inherited_model_name: &str,
    inherited_reasoning: &str,
    unknown_model_name: &str,
) -> (String, String) {
    if let Some(provider) = infer_provider_from_model_name(unknown_model_name) {
        if provider_table_present(root, provider) {
            return default_selection_for_provider(provider);
        }
    }

    if let Some(provider) = infer_provider_from_model_name(inherited_model_name) {
        if provider_table_present(root, provider) {
            return default_selection_for_provider(provider);
        }
    }

    for provider in [
        model_registry::ModelProvider::AzureOpenAi,
        model_registry::ModelProvider::Codex,
        model_registry::ModelProvider::Ollama,
        model_registry::ModelProvider::OpencodeGo,
        model_registry::ModelProvider::OpenRouter,
        model_registry::ModelProvider::ChutesAi,
    ] {
        if provider_table_present(root, provider) {
            return default_selection_for_provider(provider);
        }
    }

    (
        inherited_model_name.to_string(),
        inherited_reasoning.to_string(),
    )
}

fn infer_provider_from_model_name(model_name: &str) -> Option<model_registry::ModelProvider> {
    if let Some(model) = model_registry::find_model(model_name) {
        return Some(model.provider);
    }

    if model_name.starts_with("codex/") {
        Some(model_registry::ModelProvider::Codex)
    } else if model_name.ends_with(":cloud") {
        Some(model_registry::ModelProvider::Ollama)
    } else if model_name.starts_with("opencode-go/") {
        Some(model_registry::ModelProvider::OpencodeGo)
    } else if model_name.starts_with("gpt-") || model_name.starts_with("kimi-") {
        Some(model_registry::ModelProvider::AzureOpenAi)
    } else if model_name.starts_with("zai-org/") || model_name.starts_with("MiniMaxAI/") {
        Some(model_registry::ModelProvider::ChutesAi)
    } else if model_name.contains('/') {
        Some(model_registry::ModelProvider::OpenRouter)
    } else {
        None
    }
}

fn provider_table_present(
    root: &toml::value::Table,
    provider: model_registry::ModelProvider,
) -> bool {
    let key = match provider {
        model_registry::ModelProvider::AzureOpenAi => "azure",
        model_registry::ModelProvider::ChutesAi => "chutes",
        model_registry::ModelProvider::Codex => "codex",
        model_registry::ModelProvider::Ollama => "ollama",
        model_registry::ModelProvider::OpencodeGo => "opencode",
        model_registry::ModelProvider::OpenRouter => "openrouter",
    };
    root.get(key).and_then(toml::Value::as_table).is_some()
}

fn default_selection_for_provider(provider: model_registry::ModelProvider) -> (String, String) {
    let model_name = match provider {
        model_registry::ModelProvider::AzureOpenAi => DEFAULT_MODEL_NAME,
        model_registry::ModelProvider::ChutesAi => DEFAULT_CHUTES_MODEL_NAME,
        model_registry::ModelProvider::Codex => DEFAULT_CODEX_MODEL_NAME,
        model_registry::ModelProvider::Ollama => DEFAULT_OLLAMA_MODEL_NAME,
        model_registry::ModelProvider::OpencodeGo => DEFAULT_OPENCODE_MODEL_NAME,
        model_registry::ModelProvider::OpenRouter => DEFAULT_OPENROUTER_MODEL_NAME,
    };
    (
        model_name.to_string(),
        preferred_reasoning_string(model_name),
    )
}

impl AppConfig {
    pub fn set_default_reasoning(reasoning: ReasoningSetting) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(
            &target_path,
            None,
            Some(reasoning),
            None,
            None,
            None,
            None,
            None,
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_reasoning_at_path(path: &Path, reasoning: ReasoningSetting) -> Result<Self> {
        write_config_updates_at_path(path, None, Some(reasoning), None, None, None, None, None)?;
        Self::load_from_path(path)
    }

    pub fn set_default_model_selection_with_planning(
        model_name: &str,
        reasoning: ReasoningSetting,
        planning_agents: &[PlanningAgentConfig],
    ) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(
            &target_path,
            Some(model_name),
            Some(reasoning),
            Some(planning_agents),
            None,
            None,
            None,
            None,
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_model_selection_at_path(
        path: &Path,
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        write_config_updates_at_path(
            path,
            Some(model_name),
            Some(reasoning),
            None,
            None,
            None,
            None,
            None,
        )?;
        Self::load_from_path(path)
    }

    pub fn set_default_planning_agents(planning_agents: &[PlanningAgentConfig]) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(
            &target_path,
            None,
            None,
            Some(planning_agents),
            None,
            None,
            None,
            None,
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_planning_agents_at_path(
        path: &Path,
        planning_agents: &[PlanningAgentConfig],
    ) -> Result<Self> {
        write_config_updates_at_path(
            path,
            None,
            None,
            Some(planning_agents),
            None,
            None,
            None,
            None,
        )?;
        Self::load_from_path(path)
    }

    pub fn set_default_safety_selection(
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(
            &target_path,
            None,
            None,
            None,
            Some(model_name),
            Some(reasoning),
            None,
            None,
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_safety_selection_at_path(
        path: &Path,
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        write_config_updates_at_path(
            path,
            None,
            None,
            None,
            Some(model_name),
            Some(reasoning),
            None,
            None,
        )?;
        Self::load_from_path(path)
    }

    pub fn set_default_memory_selection(
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(
            &target_path,
            None,
            None,
            None,
            None,
            None,
            Some(model_name),
            Some(reasoning),
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_memory_selection_at_path(
        path: &Path,
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        write_config_updates_at_path(
            path,
            None,
            None,
            None,
            None,
            None,
            Some(model_name),
            Some(reasoning),
        )?;
        Self::load_from_path(path)
    }

    pub fn set_default_codex_auth(codex: Option<&CodexConfig>) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_codex_auth_updates_at_path(&target_path, codex)?;
        Self::load_from_default_path()
    }

    fn set_codex_auth_at_path(path: &Path, codex: Option<&CodexConfig>) -> Result<Self> {
        write_codex_auth_updates_at_path(path, codex)?;
        Self::load_from_path(path)
    }

    pub fn refresh_default_codex_auth_if_needed() -> Result<Self> {
        Self::refresh_codex_auth_if_needed_at_path(None)
    }

    pub fn refresh_codex_auth_if_needed_at_path(path: Option<&Path>) -> Result<Self> {
        let current = Self::load_from_runtime_path(path)?;
        let Some(codex) = current.codex.as_ref() else {
            return Ok(current);
        };
        if !crate::codex::should_refresh(codex) {
            return Ok(current);
        }

        let runtime = tokio::runtime::Runtime::new()?;
        let refreshed = runtime.block_on(crate::codex::refresh_auth(codex))?;
        match path {
            Some(path) => Self::set_codex_auth_at_path(path, Some(&refreshed)),
            None => Self::set_default_codex_auth(Some(&refreshed)),
        }
    }

    pub fn with_runtime_overrides(mut self, overrides: RuntimeConfigOverrides) -> Result<Self> {
        if let Some(model_selection) = overrides.model_selection {
            self.model = model_selection;
            self.planning.agents =
                sanitize_planning_agents(&self.model.model_name, &self.planning.agents);
        }

        if let Some(planning_agents) = overrides.planning_agents {
            self.planning.agents =
                sanitize_planning_agents(&self.model.model_name, &planning_agents);
        }

        self.validate()?;
        Ok(self)
    }

    fn validate(&self) -> Result<()> {
        validation::validate(self)
    }
}
