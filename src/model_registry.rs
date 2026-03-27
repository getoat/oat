use crate::config::{KimiThinkingMode, ReasoningEffort, ReasoningSetting};

const GPT_5_4_REASONING_SETTINGS: [ReasoningSetting; 3] = [
    ReasoningSetting::Gpt(ReasoningEffort::Low),
    ReasoningSetting::Gpt(ReasoningEffort::Medium),
    ReasoningSetting::Gpt(ReasoningEffort::High),
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
}

impl ModelProvider {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::AzureOpenAi => "Azure OpenAI",
            Self::ChutesAi => "Chutes AI",
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
    pub context_length: usize,
    pub context_length_display: Option<&'static str>,
    pub compaction_trigger_percent_used: u8,
    pub pricing: ModelPricing,
    pub long_context_pricing: Option<LongContextPricing>,
    pub supported_reasoning_settings: &'static [ReasoningSetting],
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

const MODELS: [ModelInfo; 6] = [
    ModelInfo {
        name: "gpt-5.4",
        provider: ModelProvider::AzureOpenAi,
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
    },
    ModelInfo {
        name: "gpt-5.4-mini",
        provider: ModelProvider::AzureOpenAi,
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
    },
    ModelInfo {
        name: "gpt-5.4-nano",
        provider: ModelProvider::AzureOpenAi,
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
    },
    ModelInfo {
        name: "kimi-k2.5",
        provider: ModelProvider::AzureOpenAi,
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
    },
    ModelInfo {
        name: "zai-org/GLM-5-TEE",
        provider: ModelProvider::ChutesAi,
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
    },
    ModelInfo {
        name: "MiniMaxAI/MiniMax-M2.5-TEE",
        provider: ModelProvider::ChutesAi,
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
    },
];

pub fn models() -> &'static [ModelInfo] {
    &MODELS
}

pub fn find_model(name: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|model| model.name == name)
}

pub fn reasoning_settings_for_model(name: &str) -> Option<&'static [ReasoningSetting]> {
    find_model(name).map(|model| model.supported_reasoning_settings)
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
        assert_eq!(models().len(), 6);
        assert!(find_model("gpt-5.4").is_some());
        assert!(find_model("gpt-5.4-mini").is_some());
        assert!(find_model("gpt-5.4-nano").is_some());
        assert!(find_model("kimi-k2.5").is_some());
        assert!(find_model("zai-org/GLM-5-TEE").is_some());
        assert!(find_model("MiniMaxAI/MiniMax-M2.5-TEE").is_some());
    }

    #[test]
    fn registry_exposes_reasoning_settings() {
        let model = find_model("gpt-5.4-mini").expect("registry model");
        assert!(model.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::Medium)));
        assert!(!model.supports_reasoning(ReasoningSetting::Gpt(ReasoningEffort::Minimal)));
        assert!(!model.supports_reasoning(ReasoningSetting::Kimi(KimiThinkingMode::On)));
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
            context_length: 272_000,
            context_length_display: None,
            compaction_trigger_percent_used: 95,
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
        let kimi = find_model("kimi-k2.5").expect("registry model");
        let glm = find_model("zai-org/GLM-5-TEE").expect("registry model");

        assert_eq!(gpt_54.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_54_mini.recommended_prompt_token_budget(), 104_000);
        assert_eq!(gpt_54_nano.recommended_prompt_token_budget(), 104_000);
        assert_eq!(kimi.recommended_prompt_token_budget(), 99_072);
        assert_eq!(glm.recommended_prompt_token_budget(), 68_000);
    }

    #[test]
    fn compaction_trigger_never_compacts_later_than_ninety_percent() {
        let model = ModelInfo {
            name: "synthetic-late-model",
            provider: ModelProvider::AzureOpenAi,
            context_length: 100,
            context_length_display: None,
            compaction_trigger_percent_used: 99,
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
