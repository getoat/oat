use std::{fs, path::Path};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "config.toml";
const DEFAULT_API_VERSION: &str = "2025-01-01-preview";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AppConfig {
    pub azure: AzureConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

impl AppConfig {
    pub fn load_from_default_path() -> Result<Self> {
        Self::load_from_path(Path::new(DEFAULT_CONFIG_PATH))
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

    pub fn set_default_reasoning_effort(reasoning_effort: ReasoningEffort) -> Result<Self> {
        Self::set_reasoning_effort_at_path(Path::new(DEFAULT_CONFIG_PATH), reasoning_effort)
    }

    pub fn set_reasoning_effort_at_path(
        path: &Path,
        reasoning_effort: ReasoningEffort,
    ) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut value: toml::Value =
            toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
        let azure = value
            .get_mut("azure")
            .and_then(toml::Value::as_table_mut)
            .context("config.toml is missing the [azure] table")?;
        azure.insert(
            "reasoning_effort".into(),
            toml::Value::String(reasoning_effort.as_str().to_string()),
        );

        let formatted = toml::to_string_pretty(&value)
            .with_context(|| format!("failed to serialize {}", path.display()))?;
        fs::write(path, formatted)
            .with_context(|| format!("failed to write {}", path.display()))?;
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
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            show_thinking: default_show_thinking(),
            show_tool_output: false,
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

fn default_api_version() -> String {
    DEFAULT_API_VERSION.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "#,
        )
        .expect("config parses");

        assert_eq!(config.azure.resource_name, "demo-resource");
        assert_eq!(config.azure.reasoning_effort, ReasoningEffort::Medium);
        assert!(!config.ui.show_thinking);
        assert!(config.ui.show_tool_output);
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
}
