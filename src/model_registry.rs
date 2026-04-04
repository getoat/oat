use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex, RwLock},
};

use crate::config::{KimiThinkingMode, ReasoningEffort, ReasoningSetting};

const GPT_5_4_REASONING_SETTINGS: [ReasoningSetting; 3] = [
    ReasoningSetting::Gpt(ReasoningEffort::Low),
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
];

const GPT_5_2_REASONING_SETTINGS: [ReasoningSetting; 5] = [
    ReasoningSetting::Gpt(ReasoningEffort::None),
    ReasoningSetting::Gpt(ReasoningEffort::Low),
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
    ReasoningSetting::Gpt(ReasoningEffort::XHigh),
];

const GPT_5_CODEX_REASONING_SETTINGS: [ReasoningSetting; 4] = [
    ReasoningSetting::Gpt(ReasoningEffort::Low),
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
    ReasoningSetting::Gpt(ReasoningEffort::XHigh),
];

const GPT_5_1_CODEX_MINI_REASONING_SETTINGS: [ReasoningSetting; 2] = [
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
];

const OPENROUTER_REASONING_SETTINGS: [ReasoningSetting; 6] = [
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
    ReasoningSetting::Gpt(ReasoningEffort::Low),
    ReasoningSetting::Gpt(ReasoningEffort::Minimal),
    ReasoningSetting::Gpt(ReasoningEffort::XHigh),
    ReasoningSetting::Gpt(ReasoningEffort::None),
];

const KIMI_K2_5_REASONING_SETTINGS: [ReasoningSetting; 2] = [
    ReasoningSetting::Kimi(KimiThinkingMode::On),
    ReasoningSetting::Kimi(KimiThinkingMode::Off),
];

const DEFAULT_REASONING_SETTINGS: [ReasoningSetting; 1] = [ReasoningSetting::Default];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    AzureOpenAi,
    ChutesAi,
    Codex,
    OpenRouter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelApiFamily {
    Completions,
    Responses,
}

impl ModelProvider {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::AzureOpenAi => "Azure OpenAI",
            Self::ChutesAi => "Chutes AI",
            Self::Codex => "Codex",
            Self::OpenRouter => "OpenRouter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    pub input_per_million_tokens: f64,
    pub cache_read_per_million_tokens: f64,
    pub output_per_million_tokens: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LongContextPricing {
    pub input_tokens_threshold: usize,
    pub pricing: ModelPricing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInfo {
    pub name: &'static str,
    pub provider: ModelProvider,
    pub api_family: ModelApiFamily,
    pub context_length: usize,
    pub context_length_display: Option<&'static str>,
    pub compaction_trigger_percent_used: u8,
    pub pricing: ModelPricing,
    pub long_context_pricing: Option<LongContextPricing>,
    pub supported_reasoning_settings: &'static [ReasoningSetting],
    pub supports_search: bool,
}

impl ModelInfo {
    pub fn supports_reasoning(self, reasoning: ReasoningSetting) -> bool {
        self.supported_reasoning_settings.contains(&reasoning)
    }

    pub fn display_context_length(self) -> Option<&'static str> {
        self.context_length_display
    }

    pub fn pricing_for_input_tokens(self, input_tokens: usize) -> ModelPricing {
        self.long_context_pricing
            .filter(|tier| input_tokens > tier.input_tokens_threshold)
            .map(|tier| tier.pricing)
            .unwrap_or(self.pricing)
    }

    pub fn compaction_trigger_percent_used(self) -> u8 {
        self.compaction_trigger_percent_used.min(90)
    }

    pub fn compaction_trigger_tokens(self) -> usize {
        self.context_length * self.compaction_trigger_percent_used() as usize / 100
    }

    pub fn should_compact_for_input_tokens(self, input_tokens: usize) -> bool {
        input_tokens >= self.compaction_trigger_tokens()
    }

    pub fn recommended_prompt_token_budget(self) -> usize {
        let base_limit = self
            .long_context_pricing
            .map(|tier| tier.input_tokens_threshold)
            .unwrap_or(self.context_length / 2);
        base_limit.saturating_sub(32_000).max(8_000)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseReasoningSettingError {
    UnknownModel,
    Unknown,
    UnsupportedForModel {
        supported: &'static [ReasoningSetting],
    },
}

fn supported_reasoning_settings_display(supported: &[ReasoningSetting]) -> String {
    supported
        .iter()
        .map(|setting| setting.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn supported_models_display() -> String {
    models()
        .iter()
        .map(|model| model.name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn unknown_model_message(field_name: &str, model_name: &str) -> String {
    format!(
        "Warning: unknown {field_name} `{model_name}`. Supported models: {}",
        supported_models_display()
    )
}

impl ParseReasoningSettingError {
    pub fn message(self, field_name: &str, model_name: &str, value: &str) -> String {
        match self {
            Self::UnknownModel => unknown_model_message(field_name, model_name),
            Self::Unknown => format!("unknown {field_name} `{value}`"),
            Self::UnsupportedForModel { supported } => format!(
                "{field_name} `{value}` is not supported by model `{model_name}`. Supported values: {}",
                supported_reasoning_settings_display(supported)
            ),
        }
    }
}

const BASE_MODELS: [ModelInfo; 18] = [
    ModelInfo {
        name: "gpt-5.4",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 2.50,
            cache_read_per_million_tokens: 0.25,
            output_per_million_tokens: 15.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_4_REASONING_SETTINGS,
        supports_search: true,
    },
    ModelInfo {
        name: "gpt-5.4-mini",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.75,
            cache_read_per_million_tokens: 0.075,
            output_per_million_tokens: 4.50,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_4_REASONING_SETTINGS,
        supports_search: true,
    },
    ModelInfo {
        name: "gpt-5.4-nano",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.20,
            cache_read_per_million_tokens: 0.02,
            output_per_million_tokens: 1.25,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_4_REASONING_SETTINGS,
        supports_search: true,
    },
    ModelInfo {
        name: "gpt-5.2",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 1.75,
            cache_read_per_million_tokens: 0.175,
            output_per_million_tokens: 14.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_2_REASONING_SETTINGS,
        supports_search: true,
    },
    ModelInfo {
        name: "gpt-5.3-codex",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 1.75,
            cache_read_per_million_tokens: 0.175,
            output_per_million_tokens: 14.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_CODEX_REASONING_SETTINGS,
        supports_search: true,
    },
    ModelInfo {
        name: "kimi-k2.5",
        provider: ModelProvider::AzureOpenAi,
        api_family: ModelApiFamily::Completions,
        context_length: 262_144,
        context_length_display: Some("256K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.60,
            cache_read_per_million_tokens: 0.10,
            output_per_million_tokens: 3.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &KIMI_K2_5_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "zai-org/GLM-5-TEE",
        provider: ModelProvider::ChutesAi,
        api_family: ModelApiFamily::Completions,
        context_length: 200_000,
        context_length_display: Some("200K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.0,
            cache_read_per_million_tokens: 0.0,
            output_per_million_tokens: 0.0,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &DEFAULT_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "MiniMaxAI/MiniMax-M2.5-TEE",
        provider: ModelProvider::ChutesAi,
        api_family: ModelApiFamily::Completions,
        context_length: 200_000,
        context_length_display: Some("200K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.0,
            cache_read_per_million_tokens: 0.0,
            output_per_million_tokens: 0.0,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &DEFAULT_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "openai/gpt-5.4",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 2.50,
            cache_read_per_million_tokens: 0.25,
            output_per_million_tokens: 15.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "openai/gpt-5.4-mini",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.75,
            cache_read_per_million_tokens: 0.075,
            output_per_million_tokens: 4.50,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "openai/gpt-5.4-nano",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.20,
            cache_read_per_million_tokens: 0.02,
            output_per_million_tokens: 1.25,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "openai/gpt-5.2",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 400_000,
        context_length_display: Some("400K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 1.75,
            cache_read_per_million_tokens: 0.175,
            output_per_million_tokens: 14.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_2_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "openai/gpt-5.3-codex",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 400_000,
        context_length_display: Some("400K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 1.75,
            cache_read_per_million_tokens: 0.175,
            output_per_million_tokens: 14.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &GPT_5_CODEX_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "minimax/minimax-m2.7",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 204_800,
        context_length_display: Some("204K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.30,
            cache_read_per_million_tokens: 0.06,
            output_per_million_tokens: 1.20,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "xiaomi/mimo-v2-omni",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 262_144,
        context_length_display: Some("256K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.40,
            cache_read_per_million_tokens: 0.08,
            output_per_million_tokens: 2.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "xiaomi/mimo-v2-pro",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 1_048_576,
        context_length_display: Some("1.05M"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 1.00,
            cache_read_per_million_tokens: 0.20,
            output_per_million_tokens: 3.00,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "xiaomi/mimo-v2-flash",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 262_144,
        context_length_display: Some("256K"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.09,
            cache_read_per_million_tokens: 0.045,
            output_per_million_tokens: 0.29,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
    ModelInfo {
        name: "qwen/qwen3.6-plus:free",
        provider: ModelProvider::OpenRouter,
        api_family: ModelApiFamily::Completions,
        context_length: 1_000_000,
        context_length_display: Some("1M"),
        compaction_trigger_percent_used: 90,
        pricing: ModelPricing {
            input_per_million_tokens: 0.0,
            cache_read_per_million_tokens: 0.0,
            output_per_million_tokens: 0.0,
        },
        long_context_pricing: None,
        supported_reasoning_settings: &OPENROUTER_REASONING_SETTINGS,
        supports_search: false,
    },
];

static MODEL_REGISTRY: LazyLock<RwLock<&'static [ModelInfo]>> =
    LazyLock::new(|| RwLock::new(build_model_registry()));
static GENERIC_CODEX_MODELS: LazyLock<Mutex<HashMap<String, &'static ModelInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn build_model_registry() -> &'static [ModelInfo] {
    let mut models = BASE_MODELS.to_vec();
    let mut seen = BASE_MODELS
        .iter()
        .map(|model| model.name.to_string())
        .collect::<HashSet<_>>();

    for bundled in bundled_codex_models() {
        if seen.insert(bundled.name.to_string()) {
            models.push(bundled);
        }
    }

    Box::leak(models.into_boxed_slice())
}

fn bundled_codex_models() -> [ModelInfo; 7] {
    [
        bundled_codex_model(
            "codex/gpt-5.3-codex",
            &GPT_5_CODEX_REASONING_SETTINGS,
            false,
        ),
        bundled_codex_model("codex/gpt-5.4", &GPT_5_4_REASONING_SETTINGS, false),
        bundled_codex_model("codex/gpt-5.4-mini", &GPT_5_4_REASONING_SETTINGS, true),
        bundled_codex_model(
            "codex/gpt-5.2-codex",
            &GPT_5_CODEX_REASONING_SETTINGS,
            false,
        ),
        bundled_codex_model(
            "codex/gpt-5.1-codex-max",
            &GPT_5_CODEX_REASONING_SETTINGS,
            false,
        ),
        bundled_codex_model("codex/gpt-5.2", &GPT_5_2_REASONING_SETTINGS, false),
        bundled_codex_model(
            "codex/gpt-5.1-codex-mini",
            &GPT_5_1_CODEX_MINI_REASONING_SETTINGS,
            true,
        ),
    ]
}

fn bundled_codex_model(
    name: &'static str,
    supported_reasoning_settings: &'static [ReasoningSetting],
    is_mini: bool,
) -> ModelInfo {
    ModelInfo {
        name,
        provider: ModelProvider::Codex,
        api_family: ModelApiFamily::Responses,
        context_length: 272_000,
        context_length_display: None,
        compaction_trigger_percent_used: 90,
        pricing: if is_mini {
            ModelPricing {
                input_per_million_tokens: 0.75,
                cache_read_per_million_tokens: 0.075,
                output_per_million_tokens: 4.50,
            }
        } else {
            ModelPricing {
                input_per_million_tokens: 1.75,
                cache_read_per_million_tokens: 0.175,
                output_per_million_tokens: 14.00,
            }
        },
        long_context_pricing: None,
        supported_reasoning_settings,
        supports_search: true,
    }
}

fn dynamic_codex_model(name: &str) -> ModelInfo {
    let leaked_name: &'static str = Box::leak(name.to_string().into_boxed_str());
    let slug = name.strip_prefix("codex/").unwrap_or(name);

    if let Some(template) = BASE_MODELS.iter().find(|model| {
        model.provider != ModelProvider::Codex
            && (model.name == slug || model.name == format!("openai/{slug}"))
    }) {
        let mut model = *template;
        model.name = leaked_name;
        model.provider = ModelProvider::Codex;
        model.api_family = ModelApiFamily::Responses;
        model.supports_search = true;
        return model;
    }

    let (context_length, context_length_display, pricing, supported_reasoning_settings) =
        if slug == "gpt-5.2" {
            (
                400_000,
                Some("400K"),
                ModelPricing {
                    input_per_million_tokens: 1.75,
                    cache_read_per_million_tokens: 0.175,
                    output_per_million_tokens: 14.00,
                },
                &GPT_5_2_REASONING_SETTINGS[..],
            )
        } else if slug.contains("codex") {
            (
                400_000,
                Some("400K"),
                ModelPricing {
                    input_per_million_tokens: 1.75,
                    cache_read_per_million_tokens: 0.175,
                    output_per_million_tokens: 14.00,
                },
                &GPT_5_CODEX_REASONING_SETTINGS[..],
            )
        } else {
            (
                272_000,
                None,
                ModelPricing {
                    input_per_million_tokens: 2.50,
                    cache_read_per_million_tokens: 0.25,
                    output_per_million_tokens: 15.00,
                },
                &GPT_5_4_REASONING_SETTINGS[..],
            )
        };

    ModelInfo {
        name: leaked_name,
        provider: ModelProvider::Codex,
        api_family: ModelApiFamily::Responses,
        context_length,
        context_length_display,
        compaction_trigger_percent_used: 90,
        pricing,
        long_context_pricing: None,
        supported_reasoning_settings,
        supports_search: true,
    }
}

pub fn models() -> &'static [ModelInfo] {
    *MODEL_REGISTRY.read().expect("model registry lock")
}

pub fn find_model(name: &str) -> Option<&'static ModelInfo> {
    models()
        .iter()
        .find(|model| model.name == name)
        .or_else(|| generic_codex_model(name))
}

fn generic_codex_model(name: &str) -> Option<&'static ModelInfo> {
    if !name.starts_with("codex/") {
        return None;
    }

    let mut models = GENERIC_CODEX_MODELS
        .lock()
        .expect("generic codex model lock");
    if let Some(model) = models.get(name) {
        return Some(*model);
    }

    let model = Box::leak(Box::new(dynamic_codex_model(name)));
    models.insert(name.to_string(), model);
    Some(model)
}

pub fn reasoning_settings_for_model(name: &str) -> Option<&'static [ReasoningSetting]> {
    find_model(name).map(|model| model.supported_reasoning_settings)
}

pub fn uses_responses_api(name: &str) -> bool {
    find_model(name).is_some_and(|model| model.api_family == ModelApiFamily::Responses)
}

pub fn supports_search(name: &str) -> bool {
    find_model(name).is_some_and(|model| model.supports_search)
}

pub fn parse_reasoning_setting_for_model(
    model_name: &str,
    value: &str,
) -> Result<ReasoningSetting, ParseReasoningSettingError> {
    let Some(supported) = reasoning_settings_for_model(model_name) else {
        return Err(ParseReasoningSettingError::UnknownModel);
    };

    if let Some(setting) = ReasoningSetting::parse_from_supported(value, supported) {
        Ok(setting)
    } else if ReasoningSetting::parse_unscoped(value).is_some() {
        Err(ParseReasoningSettingError::UnsupportedForModel { supported })
    } else {
        Err(ParseReasoningSettingError::Unknown)
    }
}

pub fn default_reasoning_setting_for_model(name: &str) -> Option<ReasoningSetting> {
    reasoning_settings_for_model(name).and_then(|settings| settings.first().copied())
}

pub fn recommended_prompt_token_budget(name: &str) -> Option<usize> {
    find_model(name).map(|model| model.recommended_prompt_token_budget())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_models_are_available() {
        assert!(models().len() >= 23);
        assert!(find_model("gpt-5.4").is_some());
        assert!(find_model("gpt-5.4-mini").is_some());
        assert!(find_model("gpt-5.4-nano").is_some());
        assert!(find_model("gpt-5.2").is_some());
        assert!(find_model("gpt-5.3-codex").is_some());
        assert!(find_model("kimi-k2.5").is_some());
        assert!(find_model("zai-org/GLM-5-TEE").is_some());
        assert!(find_model("MiniMaxAI/MiniMax-M2.5-TEE").is_some());
        assert!(find_model("codex/gpt-5.3-codex").is_some());
        assert!(find_model("codex/gpt-5.4").is_some());
        assert!(find_model("codex/gpt-5.4-mini").is_some());
        assert!(find_model("codex/gpt-5.2-codex").is_some());
        assert!(find_model("codex/gpt-5.1-codex-max").is_some());
        assert!(find_model("codex/gpt-5.2").is_some());
        assert!(find_model("codex/gpt-5.1-codex-mini").is_some());
        assert!(find_model("openai/gpt-5.4").is_some());
        assert!(find_model("openai/gpt-5.4-mini").is_some());
        assert!(find_model("openai/gpt-5.4-nano").is_some());
        assert!(find_model("openai/gpt-5.2").is_some());
        assert!(find_model("openai/gpt-5.3-codex").is_some());
        assert!(find_model("minimax/minimax-m2.7").is_some());
        assert!(find_model("xiaomi/mimo-v2-omni").is_some());
        assert!(find_model("xiaomi/mimo-v2-pro").is_some());
        assert!(find_model("xiaomi/mimo-v2-flash").is_some());
        assert!(find_model("qwen/qwen3.6-plus:free").is_some());
    }

    #[test]
    fn registry_exposes_reasoning_settings() {
        let model = find_model("gpt-5.4-mini").expect("registry model");
        assert!(model.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::Medium)));
        assert!(!model.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::Minimal)));
        assert!(!model.supports_reasoning(ReasoningSetting::Kimi(KimiThinkingMode::On)));
    }

    #[test]
    fn gpt_5_2_and_5_3_codex_expose_model_specific_reasoning_levels() {
        let gpt_5_2 = find_model("gpt-5.2").expect("registry model");
        let gpt_5_3_codex = find_model("gpt-5.3-codex").expect("registry model");
        let codex_gpt_5_2 = find_model("codex/gpt-5.2").expect("registry model");
        let codex_gpt_5_1_mini = find_model("codex/gpt-5.1-codex-mini").expect("registry model");
        let openrouter_gpt_5_2 = find_model("openai/gpt-5.2").expect("registry model");
        let openrouter_gpt_5_3_codex = find_model("openai/gpt-5.3-codex").expect("registry model");

        assert!(gpt_5_2.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::None)));
        assert!(gpt_5_2.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::XHigh)));
        assert!(!gpt_5_2.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::Minimal)));
        assert!(!gpt_5_3_codex.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::None)));
        assert!(gpt_5_3_codex.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::XHigh)));
        assert_eq!(
            openrouter_gpt_5_2.supported_reasoning_settings,
            &GPT_5_2_REASONING_SETTINGS
        );
        assert_eq!(
            openrouter_gpt_5_3_codex.supported_reasoning_settings,
            &GPT_5_CODEX_REASONING_SETTINGS
        );
        assert_eq!(
            codex_gpt_5_2.supported_reasoning_settings,
            &GPT_5_2_REASONING_SETTINGS
        );
        assert_eq!(
            codex_gpt_5_1_mini.supported_reasoning_settings,
            &GPT_5_1_CODEX_MINI_REASONING_SETTINGS
        );
    }

    #[test]
    fn parse_reasoning_setting_for_model_rejects_cross_family_values() {
        assert_eq!(
            parse_reasoning_setting_for_model("gpt-5.4-mini", "medium"),
            Ok(ReasoningSetting::Gpt(ReasoningEffort::Medium))
        );
        assert_eq!(
            parse_reasoning_setting_for_model("kimi-k2.5", "medium"),
            Err(ParseReasoningSettingError::UnsupportedForModel {
                supported: &KIMI_K2_5_REASONING_SETTINGS,
            })
        );
    }

    #[test]
    fn parse_reasoning_setting_for_unknown_model_reports_unknown_model() {
        assert_eq!(
            parse_reasoning_setting_for_model("custom-model", "medium"),
            Err(ParseReasoningSettingError::UnknownModel)
        );
    }

    #[test]
    fn chutes_models_use_default_reasoning() {
        let glm = find_model("zai-org/GLM-5-TEE").expect("registry model");
        let minimax = find_model("MiniMaxAI/MiniMax-M2.5-TEE").expect("registry model");

        assert_eq!(glm.provider, ModelProvider::ChutesAi);
        assert_eq!(
            glm.supported_reasoning_settings,
            &DEFAULT_REASONING_SETTINGS
        );
        assert_eq!(minimax.provider, ModelProvider::ChutesAi);
        assert_eq!(
            minimax.supported_reasoning_settings,
            &DEFAULT_REASONING_SETTINGS
        );
        assert_eq!(
            parse_reasoning_setting_for_model("zai-org/GLM-5-TEE", "default"),
            Ok(ReasoningSetting::Default)
        );
    }

    #[test]
    fn kimi_exposes_on_off_reasoning_and_display_context() {
        let model = find_model("kimi-k2.5").expect("registry model");

        assert_eq!(model.context_length, 262_144);
        assert_eq!(model.display_context_length(), Some("256K"));
        assert_eq!(
            model.supported_reasoning_settings,
            &KIMI_K2_5_REASONING_SETTINGS
        );
    }

    #[test]
    fn gpt_5_4_uses_base_pricing_only() {
        let model = find_model("gpt-5.4").expect("registry model");

        assert_eq!(model.long_context_pricing, None);
        assert_eq!(model.pricing_for_input_tokens(272_000), model.pricing);
        assert_eq!(model.pricing_for_input_tokens(272_001), model.pricing);
    }

    #[test]
    fn long_context_pricing_still_applies_for_tiered_models() {
        let model = ModelInfo {
            name: "synthetic-tiered-model",
            provider: ModelProvider::AzureOpenAi,
            api_family: ModelApiFamily::Completions,
            context_length: 272_000,
            context_length_display: None,
            compaction_trigger_percent_used: 95,
            supports_search: false,
            pricing: ModelPricing {
                input_per_million_tokens: 1.0,
                cache_read_per_million_tokens: 0.1,
                output_per_million_tokens: 2.0,
            },
            long_context_pricing: Some(LongContextPricing {
                input_tokens_threshold: 100_000,
                pricing: ModelPricing {
                    input_per_million_tokens: 3.0,
                    cache_read_per_million_tokens: 0.3,
                    output_per_million_tokens: 6.0,
                },
            }),
            supported_reasoning_settings: &GPT_5_4_REASONING_SETTINGS,
        };

        assert_eq!(model.pricing_for_input_tokens(100_000), model.pricing);
        assert_eq!(
            model.pricing_for_input_tokens(100_001),
            model.long_context_pricing.expect("long tier").pricing
        );
    }

    #[test]
    fn recommended_prompt_budget_uses_conservative_headroom() {
        let gpt_54 = find_model("gpt-5.4").expect("registry model");
        let gpt_54_mini = find_model("gpt-5.4-mini").expect("registry model");
        let gpt_54_nano = find_model("gpt-5.4-nano").expect("registry model");
        let gpt_5_2 = find_model("gpt-5.2").expect("registry model");
        let gpt_5_3_codex = find_model("gpt-5.3-codex").expect("registry model");
        let kimi = find_model("kimi-k2.5").expect("registry model");
        let glm = find_model("zai-org/GLM-5-TEE").expect("registry model");
        let openrouter_gpt_54 = find_model("openai/gpt-5.4").expect("registry model");
        let openrouter_gpt_54_mini = find_model("openai/gpt-5.4-mini").expect("registry model");
        let openrouter_gpt_54_nano = find_model("openai/gpt-5.4-nano").expect("registry model");
        let openrouter_gpt_5_2 = find_model("openai/gpt-5.2").expect("registry model");
        let openrouter_gpt_5_3_codex = find_model("openai/gpt-5.3-codex").expect("registry model");
        let minimax = find_model("minimax/minimax-m2.7").expect("registry model");
        let mimo_omni = find_model("xiaomi/mimo-v2-omni").expect("registry model");
        let mimo_pro = find_model("xiaomi/mimo-v2-pro").expect("registry model");
        let mimo_flash = find_model("xiaomi/mimo-v2-flash").expect("registry model");
        let qwen_preview = find_model("qwen/qwen3.6-plus:free").expect("registry model");

        assert_eq!(gpt_54.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_54_mini.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_54_nano.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_5_2.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_5_3_codex.recommended_prompt_token_budget(), 104_000);
        assert_eq!(kimi.recommended_prompt_token_budget(), 99_072);
        assert_eq!(glm.recommended_prompt_token_budget(), 68_000);
        assert_eq!(openrouter_gpt_54.recommended_prompt_token_budget(), 104_000);
        assert_eq!(
            openrouter_gpt_54_mini.recommended_prompt_token_budget(),
            104_000
        );
        assert_eq!(
            openrouter_gpt_54_nano.recommended_prompt_token_budget(),
            104_000
        );
        assert_eq!(
            openrouter_gpt_5_2.recommended_prompt_token_budget(),
            168_000
        );
        assert_eq!(
            openrouter_gpt_5_3_codex.recommended_prompt_token_budget(),
            168_000
        );
        assert_eq!(minimax.recommended_prompt_token_budget(), 70_400);
        assert_eq!(mimo_omni.recommended_prompt_token_budget(), 99_072);
        assert_eq!(mimo_pro.recommended_prompt_token_budget(), 492_288);
        assert_eq!(mimo_flash.recommended_prompt_token_budget(), 99_072);
        assert_eq!(qwen_preview.recommended_prompt_token_budget(), 468_000);
    }

    #[test]
    fn openrouter_models_use_effort_reasoning_and_expected_pricing() {
        let gpt_54 = find_model("openai/gpt-5.4").expect("registry model");
        let gpt_54_mini = find_model("openai/gpt-5.4-mini").expect("registry model");
        let gpt_54_nano = find_model("openai/gpt-5.4-nano").expect("registry model");
        let gpt_5_2 = find_model("openai/gpt-5.2").expect("registry model");
        let gpt_5_3_codex = find_model("openai/gpt-5.3-codex").expect("registry model");
        let minimax = find_model("minimax/minimax-m2.7").expect("registry model");
        let mimo_omni = find_model("xiaomi/mimo-v2-omni").expect("registry model");
        let mimo_pro = find_model("xiaomi/mimo-v2-pro").expect("registry model");
        let mimo_flash = find_model("xiaomi/mimo-v2-flash").expect("registry model");
        let qwen_preview = find_model("qwen/qwen3.6-plus:free").expect("registry model");

        assert_eq!(gpt_54.provider, ModelProvider::OpenRouter);
        assert_eq!(gpt_54_mini.provider, ModelProvider::OpenRouter);
        assert_eq!(gpt_54_nano.provider, ModelProvider::OpenRouter);
        assert_eq!(gpt_5_2.provider, ModelProvider::OpenRouter);
        assert_eq!(gpt_5_3_codex.provider, ModelProvider::OpenRouter);
        assert_eq!(minimax.provider, ModelProvider::OpenRouter);
        assert_eq!(mimo_omni.provider, ModelProvider::OpenRouter);
        assert_eq!(mimo_pro.provider, ModelProvider::OpenRouter);
        assert_eq!(mimo_flash.provider, ModelProvider::OpenRouter);
        assert_eq!(qwen_preview.provider, ModelProvider::OpenRouter);
        assert_eq!(
            gpt_54.supported_reasoning_settings,
            &OPENROUTER_REASONING_SETTINGS
        );
        assert_eq!(gpt_54.pricing.input_per_million_tokens, 2.50);
        assert_eq!(gpt_54_mini.pricing.cache_read_per_million_tokens, 0.075);
        assert_eq!(gpt_54_nano.pricing.output_per_million_tokens, 1.25);
        assert_eq!(gpt_5_2.context_length, 400_000);
        assert_eq!(gpt_5_2.pricing.input_per_million_tokens, 1.75);
        assert_eq!(gpt_5_3_codex.context_length, 400_000);
        assert_eq!(gpt_5_3_codex.pricing.cache_read_per_million_tokens, 0.175);
        assert_eq!(gpt_5_3_codex.pricing.output_per_million_tokens, 14.00);
        assert_eq!(minimax.pricing.input_per_million_tokens, 0.30);
        assert_eq!(minimax.pricing.cache_read_per_million_tokens, 0.06);
        assert_eq!(mimo_omni.pricing.output_per_million_tokens, 2.00);
        assert_eq!(mimo_pro.context_length, 1_048_576);
        assert_eq!(mimo_flash.pricing.input_per_million_tokens, 0.09);
        assert_eq!(mimo_flash.pricing.cache_read_per_million_tokens, 0.045);
        assert_eq!(mimo_flash.pricing.output_per_million_tokens, 0.29);
        assert_eq!(qwen_preview.context_length, 1_000_000);
        assert_eq!(qwen_preview.display_context_length(), Some("1M"));
        assert_eq!(
            qwen_preview.supported_reasoning_settings,
            &OPENROUTER_REASONING_SETTINGS
        );
        assert_eq!(qwen_preview.pricing.input_per_million_tokens, 0.0);
        assert_eq!(qwen_preview.pricing.output_per_million_tokens, 0.0);
    }

    #[test]
    fn compaction_trigger_never_compacts_later_than_ninety_percent() {
        let model = ModelInfo {
            name: "synthetic-late-model",
            provider: ModelProvider::AzureOpenAi,
            api_family: ModelApiFamily::Completions,
            context_length: 100,
            context_length_display: None,
            compaction_trigger_percent_used: 99,
            supports_search: false,
            pricing: ModelPricing {
                input_per_million_tokens: 1.0,
                cache_read_per_million_tokens: 0.1,
                output_per_million_tokens: 2.0,
            },
            long_context_pricing: None,
            supported_reasoning_settings: &GPT_5_4_REASONING_SETTINGS,
        };

        assert_eq!(model.compaction_trigger_percent_used(), 90);
        assert_eq!(model.compaction_trigger_tokens(), 90);
        assert!(model.should_compact_for_input_tokens(90));
        assert!(!model.should_compact_for_input_tokens(89));
    }
}
