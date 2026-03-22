use crate::config::ReasoningEffort;

const GPT_5_4_REASONING_LEVELS: [ReasoningEffort; 3] = [
    ReasoningEffort::Low,
    ReasoningEffort::Medium,
    ReasoningEffort::High,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelProvider {
    AzureOpenAi,
}

impl ModelProvider {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::AzureOpenAi => "Azure OpenAI",
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
    pub pricing: ModelPricing,
    pub long_context_pricing: Option<LongContextPricing>,
    pub supported_reasoning_levels: &'static [ReasoningEffort],
}

impl ModelInfo {
    pub fn supports_reasoning(self, reasoning_effort: ReasoningEffort) -> bool {
        self.supported_reasoning_levels.contains(&reasoning_effort)
    }

    pub fn pricing_for_input_tokens(self, input_tokens: usize) -> ModelPricing {
        self.long_context_pricing
            .filter(|tier| input_tokens > tier.input_tokens_threshold)
            .map(|tier| tier.pricing)
            .unwrap_or(self.pricing)
    }

    pub fn recommended_prompt_token_budget(self) -> usize {
        let base_limit = self
            .long_context_pricing
            .map(|tier| tier.input_tokens_threshold)
            .unwrap_or(self.context_length / 2);
        base_limit.saturating_sub(32_000).max(8_000)
    }
}

const MODELS: [ModelInfo; 3] = [
    ModelInfo {
        name: "gpt-5.4",
        provider: ModelProvider::AzureOpenAi,
        context_length: 1_050_000,
        pricing: ModelPricing {
            input_per_million_tokens: 2.50,
            cache_read_per_million_tokens: 0.25,
            output_per_million_tokens: 15.00,
        },
        long_context_pricing: Some(LongContextPricing {
            input_tokens_threshold: 272_000,
            pricing: ModelPricing {
                input_per_million_tokens: 5.00,
                cache_read_per_million_tokens: 0.50,
                output_per_million_tokens: 22.50,
            },
        }),
        supported_reasoning_levels: &GPT_5_4_REASONING_LEVELS,
    },
    ModelInfo {
        name: "gpt-5.4-mini",
        provider: ModelProvider::AzureOpenAi,
        context_length: 400_000,
        pricing: ModelPricing {
            input_per_million_tokens: 0.75,
            cache_read_per_million_tokens: 0.075,
            output_per_million_tokens: 4.50,
        },
        long_context_pricing: None,
        supported_reasoning_levels: &GPT_5_4_REASONING_LEVELS,
    },
    ModelInfo {
        name: "gpt-5.4-nano",
        provider: ModelProvider::AzureOpenAi,
        context_length: 400_000,
        pricing: ModelPricing {
            input_per_million_tokens: 0.20,
            cache_read_per_million_tokens: 0.02,
            output_per_million_tokens: 1.25,
        },
        long_context_pricing: None,
        supported_reasoning_levels: &GPT_5_4_REASONING_LEVELS,
    },
];

pub fn models() -> &'static [ModelInfo] {
    &MODELS
}

pub fn find_model(name: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|model| model.name == name)
}

pub fn reasoning_levels_for_model(name: &str) -> Option<&'static [ReasoningEffort]> {
    find_model(name).map(|model| model.supported_reasoning_levels)
}

pub fn default_reasoning_for_model(name: &str) -> Option<ReasoningEffort> {
    reasoning_levels_for_model(name).and_then(|levels| levels.first().copied())
}

pub fn recommended_prompt_token_budget(name: &str) -> Option<usize> {
    find_model(name).map(|model| model.recommended_prompt_token_budget())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_models_are_available() {
        assert_eq!(models().len(), 3);
        assert!(find_model("gpt-5.4").is_some());
        assert!(find_model("gpt-5.4-mini").is_some());
        assert!(find_model("gpt-5.4-nano").is_some());
    }

    #[test]
    fn registry_exposes_reasoning_levels() {
        let model = find_model("gpt-5.4-mini").expect("registry model");
        assert!(model.supports_reasoning(ReasoningEffort::Medium));
        assert!(!model.supports_reasoning(ReasoningEffort::Minimal));
    }

    #[test]
    fn gpt_5_4_exposes_long_context_pricing_tier() {
        let model = find_model("gpt-5.4").expect("registry model");

        assert_eq!(
            model.long_context_pricing,
            Some(LongContextPricing {
                input_tokens_threshold: 272_000,
                pricing: ModelPricing {
                    input_per_million_tokens: 5.00,
                    cache_read_per_million_tokens: 0.50,
                    output_per_million_tokens: 22.50,
                },
            })
        );
        assert_eq!(model.pricing_for_input_tokens(272_000), model.pricing);
        assert_eq!(
            model.pricing_for_input_tokens(272_001),
            model.long_context_pricing.expect("long tier").pricing
        );
    }

    #[test]
    fn recommended_prompt_budget_uses_conservative_headroom() {
        let gpt_54 = find_model("gpt-5.4").expect("registry model");
        let gpt_54_mini = find_model("gpt-5.4-mini").expect("registry model");

        assert_eq!(gpt_54.recommended_prompt_token_budget(), 240_000);
        assert_eq!(gpt_54_mini.recommended_prompt_token_budget(), 168_000);
    }
}
