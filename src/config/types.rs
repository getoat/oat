use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};

use std::{fmt, sync::LazyLock};

use crate::{features::planning::PlanningConfig, model_registry, tool_policy};

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub azure: Option<AzureConfig>,
    pub chutes: Option<ChutesConfig>,
    pub codex: Option<CodexConfig>,
    pub ollama: Option<OllamaConfig>,
    pub opencode: Option<OpencodeConfig>,
    pub openrouter: Option<OpenRouterConfig>,
    pub model: ModelSelectionConfig,
    pub safety: SafetyConfig,
    pub ui: UiConfig,
    pub subagents: SubagentConfig,
    pub planning: PlanningConfig,
    pub memory: MemoryConfig,
    pub tools: ToolConfig,
}

impl AppConfig {
    pub fn provider_config_for_model(&self, model_name: &str) -> Result<ProviderConfigRef<'_>> {
        let model = model_registry::find_model(model_name).ok_or_else(|| {
            anyhow!(model_registry::unknown_model_message(
                "model.model_name",
                model_name
            ))
        })?;

        match model.provider {
            model_registry::ModelProvider::AzureOpenAi => self
                .azure
                .as_ref()
                .map(ProviderConfigRef::Azure)
                .ok_or_else(|| {
                    anyhow!("config is missing the [azure] table required for model `{model_name}`")
                }),
            model_registry::ModelProvider::ChutesAi => self
                .chutes
                .as_ref()
                .map(ProviderConfigRef::Chutes)
                .ok_or_else(|| {
                    anyhow!(
                        "config is missing the [chutes] table required for model `{model_name}`"
                    )
                }),
            model_registry::ModelProvider::Codex => Ok(ProviderConfigRef::Codex(
                self.codex.as_ref().unwrap_or(&EMPTY_CODEX_CONFIG),
            )),
            model_registry::ModelProvider::Ollama => self
                .ollama
                .as_ref()
                .map(ProviderConfigRef::Ollama)
                .ok_or_else(|| {
                    anyhow!(
                        "config is missing the [ollama] table required for model `{model_name}`"
                    )
                }),
            model_registry::ModelProvider::OpencodeGo => self
                .opencode
                .as_ref()
                .map(ProviderConfigRef::Opencode)
                .ok_or_else(|| {
                    anyhow!(
                        "config is missing the [opencode] table required for model `{model_name}`"
                    )
                }),
            model_registry::ModelProvider::OpenRouter => self
                .openrouter
                .as_ref()
                .map(ProviderConfigRef::OpenRouter)
                .ok_or_else(|| {
                    anyhow!(
                        "config is missing the [openrouter] table required for model `{model_name}`"
                    )
                }),
        }
    }

    pub fn base_url_for_model(&self, model_name: &str) -> Result<String> {
        let model = model_registry::find_model(model_name).ok_or_else(|| {
            anyhow!(model_registry::unknown_model_message(
                "model.model_name",
                model_name
            ))
        })?;
        Ok(self
            .provider_config_for_model(model_name)?
            .base_url(model.api_family))
    }
}

static EMPTY_CODEX_CONFIG: LazyLock<CodexConfig> = LazyLock::new(CodexConfig::default);

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AzureConfig {
    pub resource_name: String,
    pub api_key: String,
    #[serde(default = "default_api_version")]
    pub api_version: String,
}

impl AzureConfig {
    pub fn endpoint(&self) -> String {
        format!("https://{}.openai.azure.com", self.resource_name.trim())
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ChutesConfig {
    pub api_key: String,
}

impl ChutesConfig {
    pub fn base_url(&self) -> &'static str {
        "https://llm.chutes.ai/v1"
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexAuthMode {
    ApiKey,
    Chatgpt,
    ChatgptAuthTokens,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodexConfig {
    pub auth_mode: Option<CodexAuthMode>,
    #[serde(rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub account_id: Option<String>,
    pub last_refresh: Option<DateTime<Utc>>,
}

impl CodexConfig {
    pub fn resolved_auth_mode(&self) -> Option<CodexAuthMode> {
        if let Some(mode) = self.auth_mode {
            return Some(mode);
        }
        if self
            .openai_api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some(CodexAuthMode::ApiKey);
        }
        if self
            .access_token
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some(CodexAuthMode::Chatgpt);
        }
        None
    }

    pub fn auth_token(&self) -> Option<&str> {
        match self.resolved_auth_mode() {
            Some(CodexAuthMode::ApiKey) => self
                .openai_api_key
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            Some(CodexAuthMode::Chatgpt | CodexAuthMode::ChatgptAuthTokens) => self
                .access_token
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            None => None,
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }

    pub fn is_authenticated(&self) -> bool {
        self.auth_token().is_some()
    }

    pub fn base_url(&self) -> &'static str {
        match self.resolved_auth_mode() {
            Some(CodexAuthMode::ApiKey) => "https://api.openai.com/v1",
            _ => "https://chatgpt.com/backend-api/codex",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpenRouterConfig {
    pub api_key: String,
}

impl OpenRouterConfig {
    pub fn base_url(&self) -> &'static str {
        "https://openrouter.ai/api/v1"
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OpencodeConfig {
    pub api_key: String,
}

impl OpencodeConfig {
    pub fn base_url(&self, api_family: model_registry::ModelApiFamily) -> &'static str {
        match api_family {
            model_registry::ModelApiFamily::Anthropic => "https://opencode.ai/zen/go",
            model_registry::ModelApiFamily::Completions
            | model_registry::ModelApiFamily::Responses => "https://opencode.ai/zen/go/v1",
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OllamaConfig {
    pub api_key: String,
}

impl OllamaConfig {
    pub fn base_url(&self) -> &'static str {
        "https://ollama.com/v1"
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelSelectionConfig {
    pub model_name: String,
    pub reasoning: ReasoningSetting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderConfigRef<'a> {
    Azure(&'a AzureConfig),
    Chutes(&'a ChutesConfig),
    Codex(&'a CodexConfig),
    Ollama(&'a OllamaConfig),
    Opencode(&'a OpencodeConfig),
    OpenRouter(&'a OpenRouterConfig),
}

impl ProviderConfigRef<'_> {
    pub fn auth_token(&self) -> Option<&str> {
        match self {
            Self::Azure(config) => Some(&config.api_key),
            Self::Chutes(config) => Some(&config.api_key),
            Self::Codex(config) => config.auth_token(),
            Self::Ollama(config) => Some(&config.api_key),
            Self::Opencode(config) => Some(&config.api_key),
            Self::OpenRouter(config) => Some(&config.api_key),
        }
    }

    pub fn base_url(&self, api_family: model_registry::ModelApiFamily) -> String {
        match self {
            Self::Azure(config) => format!("{}/openai/v1", config.endpoint().trim_end_matches('/')),
            Self::Chutes(config) => config.base_url().to_string(),
            Self::Codex(config) => config.base_url().to_string(),
            Self::Ollama(config) => config.base_url().to_string(),
            Self::Opencode(config) => config.base_url(api_family).to_string(),
            Self::OpenRouter(config) => config.base_url().to_string(),
        }
    }

    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::Codex(config) => config.account_id(),
            Self::Azure(_)
            | Self::Chutes(_)
            | Self::Ollama(_)
            | Self::Opencode(_)
            | Self::OpenRouter(_) => None,
        }
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

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_enabled")]
    pub enabled: bool,
    #[serde(default = "default_memory_auto_inject")]
    pub auto_inject: bool,
    #[serde(default = "default_memory_auto_inject_token_budget")]
    pub auto_inject_token_budget: usize,
    #[serde(default = "default_memory_max_auto_results")]
    pub max_auto_results: usize,
    #[serde(default = "default_memory_max_candidate_search_results")]
    pub max_candidate_search_results: usize,
    #[serde(default)]
    pub retrieval: MemoryRetrievalConfig,
    #[serde(default)]
    pub extraction: MemoryExtractionConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MemoryRetrievalConfig {
    #[serde(default = "default_memory_search_retrieval")]
    pub search: MemoryRetrievalModeConfig,
    #[serde(default = "default_memory_auto_inject_retrieval")]
    pub auto_inject: MemoryRetrievalModeConfig,
    #[serde(default = "default_memory_candidate_linking_retrieval")]
    pub candidate_linking: MemoryRetrievalModeConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct MemoryRetrievalModeConfig {
    #[serde(default = "default_memory_retrieval_min_total_score")]
    pub min_total_score: f32,
    #[serde(default = "default_memory_retrieval_min_semantic_score")]
    pub min_semantic_score: f32,
    #[serde(default = "default_memory_retrieval_min_lexical_score")]
    pub min_lexical_score: f32,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct MemoryExtractionConfig {
    #[serde(default = "default_memory_extraction_enabled")]
    pub enabled: bool,
    pub model_name: String,
    pub reasoning: ReasoningSetting,
    #[serde(default = "default_memory_max_evidence_tokens")]
    pub max_evidence_tokens: usize,
    #[serde(default = "default_memory_max_related_memories")]
    pub max_related_memories: usize,
    #[serde(default = "default_memory_max_candidates_per_turn")]
    pub max_candidates_per_turn: usize,
    #[serde(default = "default_memory_min_candidate_confidence")]
    pub min_candidate_confidence: u8,
    #[serde(default = "default_memory_min_active_confidence")]
    pub min_active_confidence: u8,
    #[serde(default = "default_memory_run_in_background")]
    pub run_in_background: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: default_memory_enabled(),
            auto_inject: default_memory_auto_inject(),
            auto_inject_token_budget: default_memory_auto_inject_token_budget(),
            max_auto_results: default_memory_max_auto_results(),
            max_candidate_search_results: default_memory_max_candidate_search_results(),
            retrieval: MemoryRetrievalConfig::default(),
            extraction: MemoryExtractionConfig::default(),
        }
    }
}

impl Default for MemoryRetrievalConfig {
    fn default() -> Self {
        Self {
            search: default_memory_search_retrieval(),
            auto_inject: default_memory_auto_inject_retrieval(),
            candidate_linking: default_memory_candidate_linking_retrieval(),
        }
    }
}

impl Default for MemoryExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: default_memory_extraction_enabled(),
            model_name: "gpt-5.4-mini".into(),
            reasoning: ReasoningEffort::Medium.into(),
            max_evidence_tokens: default_memory_max_evidence_tokens(),
            max_related_memories: default_memory_max_related_memories(),
            max_candidates_per_turn: default_memory_max_candidates_per_turn(),
            min_candidate_confidence: default_memory_min_candidate_confidence(),
            min_active_confidence: default_memory_min_active_confidence(),
            run_in_background: default_memory_run_in_background(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolConfig {
    #[serde(default)]
    pub search_include_patterns: Vec<String>,
    #[serde(default = "tool_policy::default_tool_output_max_tokens")]
    pub max_output_tokens: usize,
    #[serde(default)]
    pub web_search: ToolWebSearchConfig,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            search_include_patterns: Vec::new(),
            max_output_tokens: tool_policy::default_tool_output_max_tokens(),
            web_search: ToolWebSearchConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolWebSearchConfig {
    #[serde(default = "default_web_search_mode")]
    pub mode: WebSearchMode,
}

impl Default for ToolWebSearchConfig {
    fn default() -> Self {
        Self {
            mode: default_web_search_mode(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchMode {
    Disabled,
    Cached,
    Live,
}

impl Default for WebSearchMode {
    fn default() -> Self {
        default_web_search_mode()
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
    None,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

impl ReasoningEffort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
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
    Default,
    Gpt(ReasoningEffort),
    Kimi(KimiThinkingMode),
}

impl ReasoningSetting {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Gpt(level) => level.as_str(),
            Self::Kimi(mode) => mode.as_str(),
        }
    }

    pub(crate) fn parse_unscoped(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "default" => Some(Self::Default),
            _ => ReasoningEffort::parse(value)
                .map(Self::Gpt)
                .or_else(|| KimiThinkingMode::parse(value).map(Self::Kimi)),
        }
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

pub(super) fn default_web_search_mode() -> WebSearchMode {
    WebSearchMode::Live
}

pub(super) fn default_command_history_limit() -> usize {
    20
}

pub(super) fn default_max_concurrent_subagents() -> usize {
    4
}

pub(super) fn default_memory_enabled() -> bool {
    true
}

pub(super) fn default_memory_auto_inject() -> bool {
    true
}

pub(super) fn default_memory_auto_inject_token_budget() -> usize {
    3_000
}

pub(super) fn default_memory_max_auto_results() -> usize {
    12
}

pub(super) fn default_memory_max_candidate_search_results() -> usize {
    50
}

pub(super) fn default_memory_retrieval_min_total_score() -> f32 {
    0.0
}

pub(super) fn default_memory_retrieval_min_semantic_score() -> f32 {
    0.0
}

pub(super) fn default_memory_retrieval_min_lexical_score() -> f32 {
    0.0
}

pub(super) fn default_memory_search_retrieval() -> MemoryRetrievalModeConfig {
    MemoryRetrievalModeConfig {
        min_total_score: 2.2,
        min_semantic_score: 0.58,
        min_lexical_score: 1.6,
    }
}

pub(super) fn default_memory_auto_inject_retrieval() -> MemoryRetrievalModeConfig {
    MemoryRetrievalModeConfig {
        min_total_score: 3.6,
        min_semantic_score: 0.72,
        min_lexical_score: 2.2,
    }
}

pub(super) fn default_memory_candidate_linking_retrieval() -> MemoryRetrievalModeConfig {
    MemoryRetrievalModeConfig {
        min_total_score: 2.4,
        min_semantic_score: 0.6,
        min_lexical_score: 1.6,
    }
}

pub(super) fn default_memory_extraction_enabled() -> bool {
    true
}

pub(super) fn default_memory_max_evidence_tokens() -> usize {
    12_000
}

pub(super) fn default_memory_max_related_memories() -> usize {
    24
}

pub(super) fn default_memory_max_candidates_per_turn() -> usize {
    10
}

pub(super) fn default_memory_min_candidate_confidence() -> u8 {
    55
}

pub(super) fn default_memory_min_active_confidence() -> u8 {
    85
}

pub(super) fn default_memory_run_in_background() -> bool {
    true
}

pub(super) fn default_api_version() -> String {
    super::DEFAULT_API_VERSION.to_string()
}
