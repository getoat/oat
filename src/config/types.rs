use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};

use std::fmt;

use crate::{features::planning::PlanningConfig, tool_policy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub azure: AzureConfig,
    pub safety: SafetyConfig,
    pub ui: UiConfig,
    pub subagents: SubagentConfig,
    pub planning: PlanningConfig,
    pub tools: ToolConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AzureConfig {
    pub resource_name: String,
    pub api_key: String,
    pub model_name: String,
    pub reasoning: ReasoningSetting,
    #[serde(default = "default_api_version")]
    pub api_version: String,
}

impl AzureConfig {
    pub fn endpoint(&self) -> String {
        format!("https://{}.openai.azure.com", self.resource_name.trim())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SafetyConfig {
    pub model_name: String,
    pub reasoning: ReasoningSetting,
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SubagentConfig {
    #[serde(default = "default_max_concurrent_subagents")]
    pub max_concurrent: usize,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent_subagents(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolConfig {
    #[serde(default)]
    pub search_include_patterns: Vec<String>,
    #[serde(default = "tool_policy::default_tool_output_max_tokens")]
    pub max_output_tokens: usize,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            search_include_patterns: Vec::new(),
            max_output_tokens: tool_policy::default_tool_output_max_tokens(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawReasoningSetting {
    pub(crate) reasoning: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
}

impl RawReasoningSetting {
    pub(crate) fn resolve(self) -> Option<String> {
        self.reasoning.or(self.reasoning_effort)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
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
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum KimiThinkingMode {
    On,
    Off,
}

impl KimiThinkingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReasoningSetting {
    Gpt(ReasoningEffort),
    Kimi(KimiThinkingMode),
}

impl ReasoningSetting {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gpt(level) => level.as_str(),
            Self::Kimi(mode) => mode.as_str(),
        }
    }

    pub(crate) fn parse_unscoped(value: &str) -> Option<Self> {
        ReasoningEffort::parse(value)
            .map(Self::Gpt)
            .or_else(|| KimiThinkingMode::parse(value).map(Self::Kimi))
    }

    pub(crate) fn parse_from_supported(value: &str, supported: &[Self]) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        supported
            .iter()
            .copied()
            .find(|setting| setting.as_str() == normalized)
    }
}

impl From<ReasoningEffort> for ReasoningSetting {
    fn from(value: ReasoningEffort) -> Self {
        Self::Gpt(value)
    }
}

impl From<KimiThinkingMode> for ReasoningSetting {
    fn from(value: KimiThinkingMode) -> Self {
        Self::Kimi(value)
    }
}

impl Serialize for ReasoningSetting {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

struct ReasoningSettingVisitor;

impl Visitor<'_> for ReasoningSettingVisitor {
    type Value = ReasoningSetting;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a supported reasoning setting string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        ReasoningSetting::parse_unscoped(value)
            .ok_or_else(|| E::custom(format!("unknown reasoning setting `{value}`")))
    }
}

impl<'de> Deserialize<'de> for ReasoningSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(ReasoningSettingVisitor)
    }
}

pub(super) fn default_show_thinking() -> bool {
    true
}

pub(super) fn default_command_history_limit() -> usize {
    20
}

pub(super) fn default_max_concurrent_subagents() -> usize {
    4
}

pub(super) fn default_api_version() -> String {
    super::DEFAULT_API_VERSION.to_string()
}
