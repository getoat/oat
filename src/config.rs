use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "config.toml";
const HOME_CONFIG_RELATIVE_PATH: &str = ".config/oat/config.toml";
const DEFAULT_API_VERSION: &str = "2025-01-01-preview";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub azure: AzureConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl AppConfig {
    pub fn load_from_default_path() -> Result<Self> {
        let home_path = default_home_config_path();
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

    pub fn set_default_reasoning_effort(reasoning_effort: ReasoningEffort) -> Result<Self> {
        let home_path = default_home_config_path();
        let cwd_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let target_path = default_reasoning_effort_path(home_path.as_deref(), Some(&cwd_path))?;
        write_reasoning_effort_at_path(&target_path, reasoning_effort)?;
        Self::load_from_default_path()
    }

    pub fn set_reasoning_effort_at_path(
        path: &Path,
        reasoning_effort: ReasoningEffort,
    ) -> Result<Self> {
        write_reasoning_effort_at_path(path, reasoning_effort)?;
        Self::load_from_path(path)
    }

    fn validate(&self) -> Result<()> {
        if self.azure.resource_name.trim().is_empty() {
            bail!("azure.resource_name must not be empty");
        }

        if self.azure.api_key.trim().is_empty() {
            bail!("azure.api_key must not be empty");
        }

        if self.azure.model_name.trim().is_empty() {
            bail!("azure.model_name must not be empty");
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialAppConfig {
    azure: Option<PartialAzureConfig>,
    ui: Option<PartialUiConfig>,
}

impl PartialAppConfig {
    fn merge(&mut self, other: Self) {
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
    }

    fn finalize(self) -> Result<AppConfig> {
        Ok(AppConfig {
            azure: self
                .azure
                .context("config is missing the [azure] table")?
                .finalize()?,
            ui: self.ui.unwrap_or_default().finalize(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialAzureConfig {
    resource_name: Option<String>,
    api_key: Option<String>,
    model_name: Option<String>,
    reasoning_effort: Option<ReasoningEffort>,
    api_version: Option<String>,
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
        if other.reasoning_effort.is_some() {
            self.reasoning_effort = other.reasoning_effort;
        }
        if other.api_version.is_some() {
            self.api_version = other.api_version;
        }
    }

    fn finalize(self) -> Result<AzureConfig> {
        Ok(AzureConfig {
            resource_name: self
                .resource_name
                .context("config is missing azure.resource_name")?,
            api_key: self.api_key.context("config is missing azure.api_key")?,
            model_name: self
                .model_name
                .context("config is missing azure.model_name")?,
            reasoning_effort: self
                .reasoning_effort
                .context("config is missing azure.reasoning_effort")?,
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AzureConfig {
    pub resource_name: String,
    pub api_key: String,
    pub model_name: String,
    pub reasoning_effort: ReasoningEffort,
    #[serde(default = "default_api_version")]
    pub api_version: String,
}

impl AzureConfig {
    pub fn endpoint(&self) -> String {
        format!("https://{}.openai.azure.com", self.resource_name.trim())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct UiConfig {
    #[serde(default = "default_show_thinking")]
    pub show_thinking: bool,
    #[serde(default)]
    pub show_tool_output: bool,
    #[serde(default = "default_command_history_limit")]
    pub command_history_limit: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_thinking: default_show_thinking(),
            show_tool_output: false,
            command_history_limit: default_command_history_limit(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            _ => None,
        }
    }

    pub fn supported_values() -> &'static [&'static str] {
        &["minimal", "low", "medium", "high", "xhigh"]
    }
}

fn default_show_thinking() -> bool {
    true
}

fn default_command_history_limit() -> usize {
    20
}

fn default_api_version() -> String {
    DEFAULT_API_VERSION.to_string()
}

fn default_home_config_path() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(HOME_CONFIG_RELATIVE_PATH))
}

fn default_config_locations(home_path: Option<&Path>, cwd_path: Option<&Path>) -> Vec<String> {
    let mut locations = Vec::new();
    if let Some(path) = home_path {
        locations.push(path.display().to_string());
    }
    if let Some(path) = cwd_path {
        locations.push(path.display().to_string());
    }
    locations
}

fn default_reasoning_effort_path(
    home_path: Option<&Path>,
    cwd_path: Option<&Path>,
) -> Result<PathBuf> {
    if let Some(path) = cwd_path.filter(|path| path.exists()) {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = home_path {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = cwd_path {
        return Ok(path.to_path_buf());
    }

    bail!("failed to determine a config path for reasoning effort updates")
}

fn write_reasoning_effort_at_path(path: &Path, reasoning_effort: ReasoningEffort) -> Result<()> {
    let raw = fs::read_to_string(path).unwrap_or_default();
    let mut value: toml::Value = if raw.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?
    };

    let root = value
        .as_table_mut()
        .context("config root must be a TOML table")?;
    let azure = root
        .entry("azure")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("config azure value must be a TOML table")?;
    azure.insert(
        "reasoning_effort".into(),
        toml::Value::String(reasoning_effort.as_str().to_string()),
    );

    let formatted = toml::to_string_pretty(&value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, formatted).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oat-{name}-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        ))
    }

    #[test]
    fn parses_expected_config_shape() {
        let config: AppConfig = toml::from_str(
            r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "medium"

            [ui]
            show_thinking = false
            show_tool_output = true
            command_history_limit = 42
            "#,
        )
        .expect("config parses");

        assert_eq!(config.azure.resource_name, "demo-resource");
        assert_eq!(config.azure.reasoning_effort, ReasoningEffort::Medium);
        assert!(!config.ui.show_thinking);
        assert!(config.ui.show_tool_output);
        assert_eq!(config.ui.command_history_limit, 42);
        assert_eq!(config.azure.api_version, DEFAULT_API_VERSION);
    }

    #[test]
    fn ui_config_defaults_tool_output_to_hidden() {
        let config: AppConfig = toml::from_str(
            r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "medium"
            "#,
        )
        .expect("config parses");

        assert!(config.ui.show_thinking);
        assert!(!config.ui.show_tool_output);
        assert_eq!(config.ui.command_history_limit, 20);
    }

    #[test]
    fn endpoint_is_derived_from_resource_name() {
        let azure = AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5-mini".into(),
            reasoning_effort: ReasoningEffort::High,
            api_version: default_api_version(),
        };

        assert_eq!(azure.endpoint(), "https://demo-resource.openai.azure.com");
    }

    #[test]
    fn validation_rejects_blank_required_fields() {
        let config = AppConfig {
            azure: AzureConfig {
                resource_name: String::new(),
                api_key: "secret".into(),
                model_name: "gpt-5-mini".into(),
                reasoning_effort: ReasoningEffort::Low,
                api_version: default_api_version(),
            },
            ui: UiConfig::default(),
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn parses_xhigh_reasoning_effort() {
        let config: AppConfig = toml::from_str(
            r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "xhigh"
            "#,
        )
        .expect("config parses");

        assert_eq!(config.azure.reasoning_effort, ReasoningEffort::XHigh);
    }

    #[test]
    fn default_load_merges_home_and_cwd_with_cwd_precedence() {
        let home_path = unique_temp_path("home-config");
        let cwd_path = unique_temp_path("cwd-config");

        fs::write(
            &home_path,
            r#"
            [azure]
            resource_name = "home-resource"
            api_key = "home-secret"
            model_name = "home-model"
            reasoning_effort = "minimal"

            [ui]
            show_thinking = true
            show_tool_output = false
            command_history_limit = 50
            "#,
        )
        .expect("write home config");

        fs::write(
            &cwd_path,
            r#"
            [azure]
            model_name = "cwd-model"
            reasoning_effort = "high"

            [ui]
            show_tool_output = true
            command_history_limit = 7
            "#,
        )
        .expect("write cwd config");

        let config = AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path))
            .expect("merged config loads");

        assert_eq!(config.azure.resource_name, "home-resource");
        assert_eq!(config.azure.api_key, "home-secret");
        assert_eq!(config.azure.model_name, "cwd-model");
        assert_eq!(config.azure.reasoning_effort, ReasoningEffort::High);
        assert!(config.ui.show_thinking);
        assert!(config.ui.show_tool_output);
        assert_eq!(config.ui.command_history_limit, 7);

        fs::remove_file(home_path).expect("remove home config");
        fs::remove_file(cwd_path).expect("remove cwd config");
    }

    #[test]
    fn default_load_accepts_cwd_only_partial_ui_override_when_home_has_required_fields() {
        let home_path = unique_temp_path("home-base");
        let cwd_path = unique_temp_path("cwd-ui");

        fs::write(
            &home_path,
            r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "medium"
            "#,
        )
        .expect("write home config");

        fs::write(
            &cwd_path,
            r#"
            [ui]
            show_thinking = false
            command_history_limit = 9
            "#,
        )
        .expect("write cwd config");

        let config = AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path))
            .expect("merged config loads");

        assert_eq!(config.azure.model_name, "gpt-5-mini");
        assert!(!config.ui.show_thinking);
        assert!(!config.ui.show_tool_output);
        assert_eq!(config.ui.command_history_limit, 9);

        fs::remove_file(home_path).expect("remove home config");
        fs::remove_file(cwd_path).expect("remove cwd config");
    }

    #[test]
    fn set_reasoning_effort_updates_config_file() {
        let path = std::env::temp_dir().join(format!(
            "oat-config-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        ));

        fs::write(
            &path,
            r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "medium"

            [ui]
            show_thinking = true
            show_tool_output = false
            "#,
        )
        .expect("write temp config");

        let updated = AppConfig::set_reasoning_effort_at_path(&path, ReasoningEffort::XHigh)
            .expect("update config");

        assert_eq!(updated.azure.reasoning_effort, ReasoningEffort::XHigh);
        let raw = fs::read_to_string(&path).expect("read updated config");
        assert!(raw.contains("reasoning_effort = \"xhigh\""));

        fs::remove_file(path).expect("remove temp config");
    }

    #[test]
    fn default_reasoning_effort_path_prefers_existing_cwd_config() {
        let home_path = unique_temp_path("home-effort");
        let cwd_path = unique_temp_path("cwd-effort");
        fs::write(&cwd_path, "").expect("write cwd config");

        let selected = default_reasoning_effort_path(Some(&home_path), Some(&cwd_path))
            .expect("select reasoning effort path");

        assert_eq!(selected, cwd_path);

        fs::remove_file(selected).expect("remove cwd config");
    }

    #[test]
    fn default_reasoning_effort_path_uses_home_when_cwd_config_is_missing() {
        let home_path = unique_temp_path("home-fallback");
        let cwd_path = unique_temp_path("cwd-missing");

        let selected = default_reasoning_effort_path(Some(&home_path), Some(&cwd_path))
            .expect("select reasoning effort path");

        assert_eq!(selected, home_path);
    }
}
