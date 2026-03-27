mod partial;
mod paths;
#[cfg(test)]
mod tests;
mod types;
mod updates;
mod validation;

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer};

use crate::features::planning::PlanningAgentConfig;
use partial::PartialAppConfig;
pub(crate) use paths::default_config_locations;
use paths::{default_config_update_path, default_home_config_path};
#[cfg(test)]
use types::default_api_version;
pub(crate) use types::{
    AppConfig, CodexAuthMode, CodexConfig, KimiThinkingMode, RawReasoningSetting, ReasoningEffort,
    ReasoningSetting,
};
#[cfg(test)]
pub(crate) use types::{
    AzureConfig, ModelSelectionConfig, OpenRouterConfig, SafetyConfig, SubagentConfig, ToolConfig,
    UiConfig,
};
use updates::{write_codex_auth_updates_at_path, write_config_updates_at_path};

const DEFAULT_CONFIG_PATH: &str = "config.toml";
const HOME_CONFIG_RELATIVE_PATH: &str = ".config/oat/config.toml";
const DEFAULT_API_VERSION: &str = "2025-01-01-preview";

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

        if let Some(path) = home_path.filter(|path| path.exists()) {
            config.merge(Self::load_partial_from_path(path)?);
            loaded_any = true;
        }

        if let Some(path) = cwd_path.filter(|path| path.exists()) {
            config.merge(Self::load_partial_from_path(path)?);
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

    #[cfg(test)]
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read {}. Create it from config.example.toml",
                path.display()
            )
        })?;

        let config: Self =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    fn load_partial_from_path(path: &Path) -> Result<PartialAppConfig> {
        let raw = fs::read_to_string(path).with_context(|| {
            format!(
                "failed to read {}. Create it from config.example.toml",
                path.display()
            )
        })?;

        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
    }

    pub fn set_default_reasoning(reasoning: ReasoningSetting) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(&target_path, None, Some(reasoning), None, None, None)?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_reasoning_at_path(path: &Path, reasoning: ReasoningSetting) -> Result<Self> {
        write_config_updates_at_path(path, None, Some(reasoning), None, None, None)?;
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
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_model_selection_at_path(
        path: &Path,
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        write_config_updates_at_path(path, Some(model_name), Some(reasoning), None, None, None)?;
        Self::load_from_path(path)
    }

    pub fn set_default_planning_agents(planning_agents: &[PlanningAgentConfig]) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_config_updates_at_path(&target_path, None, None, Some(planning_agents), None, None)?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_planning_agents_at_path(
        path: &Path,
        planning_agents: &[PlanningAgentConfig],
    ) -> Result<Self> {
        write_config_updates_at_path(path, None, None, Some(planning_agents), None, None)?;
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
        )?;
        Self::load_from_default_path()
    }

    #[cfg(test)]
    pub fn set_safety_selection_at_path(
        path: &Path,
        model_name: &str,
        reasoning: ReasoningSetting,
    ) -> Result<Self> {
        write_config_updates_at_path(path, None, None, None, Some(model_name), Some(reasoning))?;
        Self::load_from_path(path)
    }

    pub fn set_default_codex_auth(codex: Option<&CodexConfig>) -> Result<Self> {
        let home_path = default_home_config_path(HOME_CONFIG_RELATIVE_PATH);
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_config_update_path(home_path.as_deref(), Some(&cwd_path))?;
        write_codex_auth_updates_at_path(&target_path, codex)?;
        Self::load_from_default_path()
    }

    pub fn refresh_default_codex_auth_if_needed() -> Result<Self> {
        let current = Self::load_from_default_path()?;
        let Some(codex) = current.codex.as_ref() else {
            return Ok(current);
        };
        if !crate::codex::should_refresh(codex) {
            return Ok(current);
        }

        let runtime = tokio::runtime::Runtime::new()?;
        let refreshed = runtime.block_on(crate::codex::refresh_auth(codex))?;
        Self::set_default_codex_auth(Some(&refreshed))
    }

    fn validate(&self) -> Result<()> {
        validation::validate(self)
    }
}
