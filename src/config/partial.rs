use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer};

use crate::{
    features::planning::{PlanningAgentConfig, PlanningConfig},
    model_registry::{self, ParseReasoningSettingError},
    tool_policy,
};

use super::types::{
    AppConfig, AzureConfig, ChutesConfig, CodexConfig, HistoryConfig, HistoryMode, MemoryConfig,
    MemoryExtractionConfig, MemoryRetrievalConfig, MemoryRetrievalModeConfig, ModelSelectionConfig,
    OllamaConfig, OpenRouterConfig, OpencodeConfig, RawReasoningSetting, SafetyConfig,
    SubagentConfig, ToolConfig, ToolWebSearchConfig, UiConfig, WebSearchMode, default_api_version,
    default_command_history_limit, default_history_mode, default_history_retained_steps,
    default_max_concurrent_subagents, default_show_thinking, default_web_search_mode,
};

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct PartialAppConfig {
    azure: Option<PartialAzureConfig>,
    chutes: Option<PartialChutesConfig>,
    codex: Option<PartialCodexConfig>,
    ollama: Option<PartialOllamaConfig>,
    opencode: Option<PartialOpencodeConfig>,
    openrouter: Option<PartialOpenRouterConfig>,
    model: Option<PartialModelSelectionConfig>,
    safety: Option<PartialSafetyConfig>,
    ui: Option<PartialUiConfig>,
    subagents: Option<PartialSubagentConfig>,
    planning: Option<PartialPlanningConfig>,
    memory: Option<PartialMemoryConfig>,
    history: Option<PartialHistoryConfig>,
    tools: Option<PartialToolConfig>,
}

impl PartialAppConfig {
    pub(super) fn merge(&mut self, other: Self) {
        if let Some(azure) = other.azure {
            self.azure
                .get_or_insert_with(PartialAzureConfig::default)
                .merge(azure);
        }

        if let Some(chutes) = other.chutes {
            self.chutes
                .get_or_insert_with(PartialChutesConfig::default)
                .merge(chutes);
        }

        if let Some(codex) = other.codex {
            self.codex
                .get_or_insert_with(PartialCodexConfig::default)
                .merge(codex);
        }

        if let Some(ollama) = other.ollama {
            self.ollama
                .get_or_insert_with(PartialOllamaConfig::default)
                .merge(ollama);
        }

        if let Some(opencode) = other.opencode {
            self.opencode
                .get_or_insert_with(PartialOpencodeConfig::default)
                .merge(opencode);
        }

        if let Some(openrouter) = other.openrouter {
            self.openrouter
                .get_or_insert_with(PartialOpenRouterConfig::default)
                .merge(openrouter);
        }

        if let Some(model) = other.model {
            self.model
                .get_or_insert_with(PartialModelSelectionConfig::default)
                .merge(model);
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

        if let Some(memory) = other.memory {
            self.memory
                .get_or_insert_with(PartialMemoryConfig::default)
                .merge(memory);
        }

        if let Some(history) = other.history {
            self.history
                .get_or_insert_with(PartialHistoryConfig::default)
                .merge(history);
        }

        if let Some(tools) = other.tools {
            self.tools
                .get_or_insert_with(PartialToolConfig::default)
                .merge(tools);
        }
    }

    pub(super) fn model_selection_hint(&self) -> Option<(String, String)> {
        if let Some(model) = self.model.as_ref() {
            let model_name = model.model_name.clone()?;
            let reasoning = match model.reasoning.as_deref() {
                Some(value)
                    if model_registry::parse_reasoning_setting_for_model(&model_name, value)
                        .is_ok() =>
                {
                    value.to_string()
                }
                _ => super::preferred_reasoning_string(&model_name),
            };
            return Some((model_name, reasoning));
        }

        self.azure.as_ref().and_then(|azure| {
            let model_name = azure.legacy_model_name.clone()?;
            let reasoning = match azure.legacy_reasoning.as_deref() {
                Some(value)
                    if model_registry::parse_reasoning_setting_for_model(&model_name, value)
                        .is_ok() =>
                {
                    value.to_string()
                }
                _ => super::preferred_reasoning_string(&model_name),
            };
            Some((model_name, reasoning))
        })
    }

    pub(super) fn finalize(self) -> Result<AppConfig> {
        let Self {
            azure,
            chutes,
            codex,
            ollama,
            opencode,
            openrouter,
            model,
            safety,
            ui,
            subagents,
            planning,
            memory,
            history,
            tools,
        } = self;

        let model = match model {
            Some(model) => model.finalize("model.model_name", "model.reasoning")?,
            None => azure
                .as_ref()
                .context(
                    "config is missing the [model] table or legacy azure.model_name/azure.reasoning settings",
                )?
                .finalize_legacy_model_selection()?,
        };

        Ok(AppConfig {
            azure: azure.map(PartialAzureConfig::finalize).transpose()?,
            chutes: chutes.map(PartialChutesConfig::finalize).transpose()?,
            codex: codex.map(PartialCodexConfig::finalize).transpose()?,
            ollama: ollama.map(PartialOllamaConfig::finalize).transpose()?,
            opencode: opencode.map(PartialOpencodeConfig::finalize).transpose()?,
            openrouter: openrouter
                .map(PartialOpenRouterConfig::finalize)
                .transpose()?,
            model: model.clone(),
            safety: safety.unwrap_or_default().finalize(&model)?,
            ui: ui.unwrap_or_default().finalize(),
            subagents: subagents.unwrap_or_default().finalize(),
            planning: planning.unwrap_or_default().finalize(),
            memory: memory.unwrap_or_default().finalize(&model)?,
            history: history.unwrap_or_default().finalize(),
            tools: tools.unwrap_or_default().finalize(),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct PartialAzureConfig {
    resource_name: Option<String>,
    api_key: Option<String>,
    api_version: Option<String>,
    legacy_model_name: Option<String>,
    legacy_reasoning: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPartialAzureConfig {
    resource_name: Option<String>,
    api_key: Option<String>,
    api_version: Option<String>,
    model_name: Option<String>,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
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
            api_version: raw.api_version,
            legacy_model_name: raw.model_name,
            legacy_reasoning: raw.reasoning_fields.resolve(),
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
        if other.api_version.is_some() {
            self.api_version = other.api_version;
        }
        if other.legacy_model_name.is_some() {
            self.legacy_model_name = other.legacy_model_name;
        }
        if other.legacy_reasoning.is_some() {
            self.legacy_reasoning = other.legacy_reasoning;
        }
    }

    fn finalize(self) -> Result<AzureConfig> {
        Ok(AzureConfig {
            resource_name: self.resource_name.unwrap_or_default(),
            api_key: self.api_key.unwrap_or_default(),
            api_version: self.api_version.unwrap_or_else(default_api_version),
        })
    }

    fn finalize_legacy_model_selection(&self) -> Result<ModelSelectionConfig> {
        let model_name = self
            .legacy_model_name
            .clone()
            .context("config is missing azure.model_name")?;
        let reasoning = parse_reasoning_value(
            "azure.model_name",
            "azure.reasoning",
            &model_name,
            self.legacy_reasoning
                .clone()
                .context("config is missing azure.reasoning")?,
        )?;
        Ok(ModelSelectionConfig {
            model_name,
            reasoning,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialChutesConfig {
    api_key: Option<String>,
}

impl PartialChutesConfig {
    fn merge(&mut self, other: Self) {
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
    }

    fn finalize(self) -> Result<ChutesConfig> {
        Ok(ChutesConfig {
            api_key: self.api_key.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialCodexConfig {
    auth_mode: Option<super::types::CodexAuthMode>,
    #[serde(rename = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    account_id: Option<String>,
    last_refresh: Option<chrono::DateTime<chrono::Utc>>,
}

impl PartialCodexConfig {
    fn merge(&mut self, other: Self) {
        if other.auth_mode.is_some() {
            self.auth_mode = other.auth_mode;
        }
        if other.openai_api_key.is_some() {
            self.openai_api_key = other.openai_api_key;
        }
        if other.access_token.is_some() {
            self.access_token = other.access_token;
        }
        if other.refresh_token.is_some() {
            self.refresh_token = other.refresh_token;
        }
        if other.id_token.is_some() {
            self.id_token = other.id_token;
        }
        if other.account_id.is_some() {
            self.account_id = other.account_id;
        }
        if other.last_refresh.is_some() {
            self.last_refresh = other.last_refresh;
        }
    }

    fn finalize(self) -> Result<CodexConfig> {
        Ok(CodexConfig {
            auth_mode: self.auth_mode,
            openai_api_key: self.openai_api_key.filter(|value| !value.trim().is_empty()),
            access_token: self.access_token.filter(|value| !value.trim().is_empty()),
            refresh_token: self.refresh_token.filter(|value| !value.trim().is_empty()),
            id_token: self.id_token.filter(|value| !value.trim().is_empty()),
            account_id: self.account_id.filter(|value| !value.trim().is_empty()),
            last_refresh: self.last_refresh,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialOllamaConfig {
    api_key: Option<String>,
}

impl PartialOllamaConfig {
    fn merge(&mut self, other: Self) {
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
    }

    fn finalize(self) -> Result<OllamaConfig> {
        Ok(OllamaConfig {
            api_key: self.api_key.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialOpencodeConfig {
    api_key: Option<String>,
}

impl PartialOpencodeConfig {
    fn merge(&mut self, other: Self) {
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
    }

    fn finalize(self) -> Result<OpencodeConfig> {
        Ok(OpencodeConfig {
            api_key: self.api_key.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialOpenRouterConfig {
    api_key: Option<String>,
}

impl PartialOpenRouterConfig {
    fn merge(&mut self, other: Self) {
        if other.api_key.is_some() {
            self.api_key = other.api_key;
        }
    }

    fn finalize(self) -> Result<OpenRouterConfig> {
        Ok(OpenRouterConfig {
            api_key: self.api_key.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct PartialModelSelectionConfig {
    model_name: Option<String>,
    reasoning: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPartialModelSelectionConfig {
    model_name: Option<String>,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
}

impl<'de> Deserialize<'de> for PartialModelSelectionConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPartialModelSelectionConfig::deserialize(deserializer)?;
        Ok(Self {
            model_name: raw.model_name,
            reasoning: raw.reasoning_fields.resolve(),
        })
    }
}

impl PartialModelSelectionConfig {
    fn merge(&mut self, other: Self) {
        if other.model_name.is_some() {
            self.model_name = other.model_name;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
    }

    fn finalize(
        self,
        model_field_name: &str,
        reasoning_field_name: &str,
    ) -> Result<ModelSelectionConfig> {
        let model_name = self
            .model_name
            .context(format!("config is missing {model_field_name}"))?;
        let reasoning = parse_reasoning_value(
            model_field_name,
            reasoning_field_name,
            &model_name,
            self.reasoning
                .context(format!("config is missing {reasoning_field_name}"))?,
        )?;
        Ok(ModelSelectionConfig {
            model_name,
            reasoning,
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
struct PartialMemoryConfig {
    enabled: Option<bool>,
    auto_inject: Option<bool>,
    auto_inject_token_budget: Option<usize>,
    max_auto_results: Option<usize>,
    max_candidate_search_results: Option<usize>,
    retrieval: Option<PartialMemoryRetrievalConfig>,
    extraction: Option<PartialMemoryExtractionConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialMemoryRetrievalConfig {
    search: Option<PartialMemoryRetrievalModeConfig>,
    auto_inject: Option<PartialMemoryRetrievalModeConfig>,
    candidate_linking: Option<PartialMemoryRetrievalModeConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialMemoryRetrievalModeConfig {
    min_total_score: Option<f32>,
    min_semantic_score: Option<f32>,
    min_lexical_score: Option<f32>,
}

#[derive(Debug, Clone, Default)]
struct PartialMemoryExtractionConfig {
    enabled: Option<bool>,
    model_name: Option<String>,
    reasoning: Option<String>,
    max_evidence_tokens: Option<usize>,
    max_related_memories: Option<usize>,
    max_candidates_per_turn: Option<usize>,
    min_candidate_confidence: Option<u8>,
    min_active_confidence: Option<u8>,
    run_in_background: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawPartialMemoryExtractionConfig {
    enabled: Option<bool>,
    model_name: Option<String>,
    #[serde(flatten)]
    reasoning_fields: RawReasoningSetting,
    max_evidence_tokens: Option<usize>,
    max_related_memories: Option<usize>,
    max_candidates_per_turn: Option<usize>,
    min_candidate_confidence: Option<u8>,
    min_active_confidence: Option<u8>,
    run_in_background: Option<bool>,
}

impl<'de> Deserialize<'de> for PartialMemoryExtractionConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawPartialMemoryExtractionConfig::deserialize(deserializer)?;
        Ok(Self {
            enabled: raw.enabled,
            model_name: raw.model_name,
            reasoning: raw.reasoning_fields.resolve(),
            max_evidence_tokens: raw.max_evidence_tokens,
            max_related_memories: raw.max_related_memories,
            max_candidates_per_turn: raw.max_candidates_per_turn,
            min_candidate_confidence: raw.min_candidate_confidence,
            min_active_confidence: raw.min_active_confidence,
            run_in_background: raw.run_in_background,
        })
    }
}

impl PartialMemoryConfig {
    fn merge(&mut self, other: Self) {
        if other.enabled.is_some() {
            self.enabled = other.enabled;
        }
        if other.auto_inject.is_some() {
            self.auto_inject = other.auto_inject;
        }
        if other.auto_inject_token_budget.is_some() {
            self.auto_inject_token_budget = other.auto_inject_token_budget;
        }
        if other.max_auto_results.is_some() {
            self.max_auto_results = other.max_auto_results;
        }
        if other.max_candidate_search_results.is_some() {
            self.max_candidate_search_results = other.max_candidate_search_results;
        }
        if let Some(retrieval) = other.retrieval {
            self.retrieval
                .get_or_insert_with(PartialMemoryRetrievalConfig::default)
                .merge(retrieval);
        }
        if let Some(extraction) = other.extraction {
            self.extraction
                .get_or_insert_with(PartialMemoryExtractionConfig::default)
                .merge(extraction);
        }
    }

    fn finalize(self, model: &ModelSelectionConfig) -> Result<MemoryConfig> {
        let defaults = MemoryConfig::default();
        Ok(MemoryConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            auto_inject: self.auto_inject.unwrap_or(defaults.auto_inject),
            auto_inject_token_budget: self
                .auto_inject_token_budget
                .unwrap_or(defaults.auto_inject_token_budget),
            max_auto_results: self.max_auto_results.unwrap_or(defaults.max_auto_results),
            max_candidate_search_results: self
                .max_candidate_search_results
                .unwrap_or(defaults.max_candidate_search_results),
            retrieval: self.retrieval.unwrap_or_default().finalize(),
            extraction: self.extraction.unwrap_or_default().finalize(model)?,
        })
    }
}

impl PartialMemoryRetrievalConfig {
    fn merge(&mut self, other: Self) {
        if let Some(search) = other.search {
            self.search
                .get_or_insert_with(PartialMemoryRetrievalModeConfig::default)
                .merge(search);
        }
        if let Some(auto_inject) = other.auto_inject {
            self.auto_inject
                .get_or_insert_with(PartialMemoryRetrievalModeConfig::default)
                .merge(auto_inject);
        }
        if let Some(candidate_linking) = other.candidate_linking {
            self.candidate_linking
                .get_or_insert_with(PartialMemoryRetrievalModeConfig::default)
                .merge(candidate_linking);
        }
    }

    fn finalize(self) -> MemoryRetrievalConfig {
        let defaults = MemoryRetrievalConfig::default();
        MemoryRetrievalConfig {
            search: self.search.unwrap_or_default().finalize(defaults.search),
            auto_inject: self
                .auto_inject
                .unwrap_or_default()
                .finalize(defaults.auto_inject),
            candidate_linking: self
                .candidate_linking
                .unwrap_or_default()
                .finalize(defaults.candidate_linking),
        }
    }
}

impl PartialMemoryRetrievalModeConfig {
    fn merge(&mut self, other: Self) {
        if other.min_total_score.is_some() {
            self.min_total_score = other.min_total_score;
        }
        if other.min_semantic_score.is_some() {
            self.min_semantic_score = other.min_semantic_score;
        }
        if other.min_lexical_score.is_some() {
            self.min_lexical_score = other.min_lexical_score;
        }
    }

    fn finalize(self, defaults: MemoryRetrievalModeConfig) -> MemoryRetrievalModeConfig {
        MemoryRetrievalModeConfig {
            min_total_score: self.min_total_score.unwrap_or(defaults.min_total_score),
            min_semantic_score: self
                .min_semantic_score
                .unwrap_or(defaults.min_semantic_score),
            min_lexical_score: self.min_lexical_score.unwrap_or(defaults.min_lexical_score),
        }
    }
}

impl PartialMemoryExtractionConfig {
    fn merge(&mut self, other: Self) {
        if other.enabled.is_some() {
            self.enabled = other.enabled;
        }
        if other.model_name.is_some() {
            self.model_name = other.model_name;
        }
        if other.reasoning.is_some() {
            self.reasoning = other.reasoning;
        }
        if other.max_evidence_tokens.is_some() {
            self.max_evidence_tokens = other.max_evidence_tokens;
        }
        if other.max_related_memories.is_some() {
            self.max_related_memories = other.max_related_memories;
        }
        if other.max_candidates_per_turn.is_some() {
            self.max_candidates_per_turn = other.max_candidates_per_turn;
        }
        if other.min_candidate_confidence.is_some() {
            self.min_candidate_confidence = other.min_candidate_confidence;
        }
        if other.min_active_confidence.is_some() {
            self.min_active_confidence = other.min_active_confidence;
        }
        if other.run_in_background.is_some() {
            self.run_in_background = other.run_in_background;
        }
    }

    fn finalize(self, model: &ModelSelectionConfig) -> Result<MemoryExtractionConfig> {
        let defaults = MemoryExtractionConfig::default();
        let model_name = self.model_name.unwrap_or_else(|| model.model_name.clone());
        let reasoning = self
            .reasoning
            .map(|value| {
                parse_reasoning_value(
                    "memory.extraction.model_name",
                    "memory.extraction.reasoning",
                    &model_name,
                    value,
                )
            })
            .transpose()?
            .unwrap_or(model.reasoning);
        Ok(MemoryExtractionConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            model_name,
            reasoning,
            max_evidence_tokens: self
                .max_evidence_tokens
                .unwrap_or(defaults.max_evidence_tokens),
            max_related_memories: self
                .max_related_memories
                .unwrap_or(defaults.max_related_memories),
            max_candidates_per_turn: self
                .max_candidates_per_turn
                .unwrap_or(defaults.max_candidates_per_turn),
            min_candidate_confidence: self
                .min_candidate_confidence
                .unwrap_or(defaults.min_candidate_confidence),
            min_active_confidence: self
                .min_active_confidence
                .unwrap_or(defaults.min_active_confidence),
            run_in_background: self.run_in_background.unwrap_or(defaults.run_in_background),
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialToolConfig {
    search_include_patterns: Option<Vec<String>>,
    max_output_tokens: Option<usize>,
    web_search: Option<PartialToolWebSearchConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialHistoryConfig {
    mode: Option<HistoryMode>,
    retained_steps: Option<usize>,
}

impl PartialHistoryConfig {
    fn merge(&mut self, other: Self) {
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        if other.retained_steps.is_some() {
            self.retained_steps = other.retained_steps;
        }
    }

    fn finalize(self) -> HistoryConfig {
        HistoryConfig {
            mode: self.mode.unwrap_or_else(default_history_mode),
            retained_steps: self
                .retained_steps
                .unwrap_or_else(default_history_retained_steps),
        }
    }
}

impl PartialToolConfig {
    fn merge(&mut self, other: Self) {
        if other.search_include_patterns.is_some() {
            self.search_include_patterns = other.search_include_patterns;
        }
        if other.max_output_tokens.is_some() {
            self.max_output_tokens = other.max_output_tokens;
        }
        if let Some(web_search) = other.web_search {
            self.web_search
                .get_or_insert_with(PartialToolWebSearchConfig::default)
                .merge(web_search);
        }
    }

    fn finalize(self) -> ToolConfig {
        ToolConfig {
            search_include_patterns: self.search_include_patterns.unwrap_or_default(),
            max_output_tokens: self
                .max_output_tokens
                .unwrap_or_else(tool_policy::default_tool_output_max_tokens),
            web_search: self.web_search.unwrap_or_default().finalize(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PartialToolWebSearchConfig {
    mode: Option<WebSearchMode>,
    enabled: Option<bool>,
}

impl PartialToolWebSearchConfig {
    fn merge(&mut self, other: Self) {
        if other.mode.is_some() {
            self.mode = other.mode;
        }
        if other.enabled.is_some() {
            self.enabled = other.enabled;
        }
    }

    fn finalize(self) -> ToolWebSearchConfig {
        ToolWebSearchConfig {
            mode: self
                .mode
                .or_else(|| self.enabled.map(legacy_web_search_mode))
                .unwrap_or_else(default_web_search_mode),
        }
    }
}

fn legacy_web_search_mode(enabled: bool) -> WebSearchMode {
    if enabled {
        WebSearchMode::Live
    } else {
        WebSearchMode::Disabled
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

    fn finalize(self, model: &ModelSelectionConfig) -> Result<SafetyConfig> {
        let model_name = self.model_name.unwrap_or_else(|| model.model_name.clone());
        let reasoning = self
            .reasoning
            .map(|value| {
                parse_reasoning_value("safety.model_name", "safety.reasoning", &model_name, value)
            })
            .transpose()?
            .unwrap_or(model.reasoning);
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
