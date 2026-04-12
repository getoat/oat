use super::*;
use crate::{
    features::planning::{PlanningAgentConfig, PlanningConfig},
    tool_policy,
};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

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

fn sample_config() -> AppConfig {
    AppConfig {
        azure: Some(AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            api_version: default_api_version(),
        }),
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning: ReasoningEffort::Medium.into(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning: ReasoningEffort::Medium.into(),
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        memory: MemoryConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    }
}

#[test]
fn parses_expected_config_shape() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [ui]
            show_thinking = false
            show_tool_output = true
            command_history_limit = 42

            [subagents]
            max_concurrent = 6

            [memory]
            enabled = true
            auto_inject = false
            auto_inject_token_budget = 1234
            max_auto_results = 9
            max_candidate_search_results = 77

            [memory.retrieval.search]
            min_total_score = 2.4
            min_semantic_score = 0.61
            min_lexical_score = 1.7

            [memory.retrieval.auto_inject]
            min_total_score = 3.9
            min_semantic_score = 0.74
            min_lexical_score = 2.3

            [memory.retrieval.candidate_linking]
            min_total_score = 2.7
            min_semantic_score = 0.63
            min_lexical_score = 1.8

            [memory.extraction]
            enabled = true
            model_name = "gpt-5.4"
            reasoning = "high"
            max_evidence_tokens = 9000
            max_related_memories = 33
            max_candidates_per_turn = 7
            min_candidate_confidence = 40
            min_active_confidence = 82
            run_in_background = false

            [tools]
            search_include_patterns = [".research/**"]
            max_output_tokens = 2048

            [tools.web_search]
            mode = "cached"
            "#,
    )
    .expect("config parses");

    let azure = config.azure.as_ref().expect("azure config");
    assert_eq!(azure.resource_name, "demo-resource");
    assert_eq!(azure.api_key, "secret");
    assert_eq!(azure.api_version, DEFAULT_API_VERSION);
    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(config.safety.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert!(!config.ui.show_thinking);
    assert!(config.ui.show_tool_output);
    assert_eq!(config.ui.command_history_limit, 42);
    assert_eq!(config.subagents.max_concurrent, 6);
    assert!(config.memory.enabled);
    assert!(!config.memory.auto_inject);
    assert_eq!(config.memory.auto_inject_token_budget, 1234);
    assert_eq!(config.memory.max_auto_results, 9);
    assert_eq!(config.memory.max_candidate_search_results, 77);
    assert_eq!(config.memory.retrieval.search.min_total_score, 2.4);
    assert_eq!(config.memory.retrieval.search.min_semantic_score, 0.61);
    assert_eq!(config.memory.retrieval.search.min_lexical_score, 1.7);
    assert_eq!(config.memory.retrieval.auto_inject.min_total_score, 3.9);
    assert_eq!(config.memory.retrieval.auto_inject.min_semantic_score, 0.74);
    assert_eq!(config.memory.retrieval.auto_inject.min_lexical_score, 2.3);
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_total_score,
        2.7
    );
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_semantic_score,
        0.63
    );
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_lexical_score,
        1.8
    );
    assert!(config.memory.extraction.enabled);
    assert_eq!(config.memory.extraction.model_name, "gpt-5.4");
    assert_eq!(
        config.memory.extraction.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    assert_eq!(config.memory.extraction.max_evidence_tokens, 9000);
    assert_eq!(config.memory.extraction.max_related_memories, 33);
    assert_eq!(config.memory.extraction.max_candidates_per_turn, 7);
    assert_eq!(config.memory.extraction.min_candidate_confidence, 40);
    assert_eq!(config.memory.extraction.min_active_confidence, 82);
    assert!(!config.memory.extraction.run_in_background);
    assert_eq!(config.tools.search_include_patterns, vec![".research/**"]);
    assert_eq!(config.tools.max_output_tokens, 2048);
    assert_eq!(config.tools.web_search.mode, WebSearchMode::Cached);
}

#[test]
fn defaults_history_to_step_summary_with_one_retained_step() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
        "#,
    )
    .expect("config parses");

    assert_eq!(config.history.mode, HistoryMode::StepSummary);
    assert_eq!(config.history.retained_steps, 1);
}

#[test]
fn parses_explicit_history_settings() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [history]
            mode = "turn_summary"
            retained_steps = 3
        "#,
    )
    .expect("config parses");

    assert_eq!(config.history.mode, HistoryMode::TurnSummary);
    assert_eq!(config.history.retained_steps, 3);
}

#[test]
fn ui_config_defaults_tool_output_to_hidden() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
            "#,
    )
    .expect("config parses");

    assert!(config.ui.show_thinking);
    assert!(!config.ui.show_tool_output);
    assert_eq!(config.ui.command_history_limit, 20);
    assert_eq!(config.subagents.max_concurrent, 4);
    assert!(config.memory.enabled);
    assert!(config.memory.auto_inject);
    assert_eq!(config.memory.auto_inject_token_budget, 3000);
    assert_eq!(config.memory.max_auto_results, 12);
    assert_eq!(config.memory.max_candidate_search_results, 50);
    assert_eq!(config.memory.retrieval.search.min_total_score, 2.2);
    assert_eq!(config.memory.retrieval.search.min_semantic_score, 0.58);
    assert_eq!(config.memory.retrieval.search.min_lexical_score, 1.6);
    assert_eq!(config.memory.retrieval.auto_inject.min_total_score, 3.6);
    assert_eq!(config.memory.retrieval.auto_inject.min_semantic_score, 0.72);
    assert_eq!(config.memory.retrieval.auto_inject.min_lexical_score, 2.2);
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_total_score,
        2.4
    );
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_semantic_score,
        0.6
    );
    assert_eq!(
        config.memory.retrieval.candidate_linking.min_lexical_score,
        1.6
    );
    assert!(config.memory.extraction.enabled);
    assert_eq!(config.memory.extraction.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.memory.extraction.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(config.memory.extraction.max_evidence_tokens, 12000);
    assert_eq!(config.memory.extraction.max_related_memories, 24);
    assert_eq!(config.memory.extraction.max_candidates_per_turn, 10);
    assert_eq!(config.memory.extraction.min_candidate_confidence, 55);
    assert_eq!(config.memory.extraction.min_active_confidence, 85);
    assert!(config.memory.extraction.run_in_background);
    assert!(config.tools.search_include_patterns.is_empty());
    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(config.safety.model_name, "gpt-5.4-mini");
    assert_eq!(config.tools.web_search.mode, WebSearchMode::Live);
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(
        config.tools.max_output_tokens,
        tool_policy::default_tool_output_max_tokens()
    );
}

#[test]
fn legacy_web_search_enabled_false_maps_to_disabled() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [tools.web_search]
            enabled = false
            "#,
    )
    .expect("config parses");

    assert_eq!(config.tools.web_search.mode, WebSearchMode::Disabled);
}

#[test]
fn legacy_web_search_enabled_true_maps_to_live() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [tools.web_search]
            enabled = true
            "#,
    )
    .expect("config parses");

    assert_eq!(config.tools.web_search.mode, WebSearchMode::Live);
}

#[test]
fn explicit_web_search_mode_takes_precedence_over_legacy_enabled() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [tools.web_search]
            mode = "cached"
            enabled = false
            "#,
    )
    .expect("config parses");

    assert_eq!(config.tools.web_search.mode, WebSearchMode::Cached);
}

#[test]
fn endpoint_is_derived_from_resource_name() {
    let azure = AzureConfig {
        resource_name: "demo-resource".into(),
        api_key: "secret".into(),
        api_version: default_api_version(),
    };

    assert_eq!(azure.endpoint(), "https://demo-resource.openai.azure.com");
}

#[test]
fn validation_rejects_blank_required_provider_fields_for_selected_model() {
    let mut config = sample_config();
    config.azure.as_mut().expect("azure").resource_name.clear();
    assert!(config.validate().is_err());
}

#[test]
fn reasoning_setting_parser_supports_xhigh_and_default_literals() {
    assert_eq!(
        ReasoningSetting::parse_unscoped("xhigh"),
        Some(ReasoningSetting::Gpt(ReasoningEffort::XHigh))
    );
    assert_eq!(
        ReasoningSetting::parse_unscoped("none"),
        Some(ReasoningSetting::Gpt(ReasoningEffort::None))
    );
    assert_eq!(
        ReasoningSetting::parse_unscoped("default"),
        Some(ReasoningSetting::Default)
    );
}

#[test]
fn parses_legacy_reasoning_effort_key() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4-mini"
            reasoning_effort = "medium"
            "#,
    )
    .expect("config parses");

    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
}

#[test]
fn model_table_overrides_legacy_azure_selection() {
    let config: AppConfig = toml::from_str(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4"
            reasoning = "low"

            [model]
            model_name = "kimi-k2.5"
            reasoning = "off"
            "#,
    )
    .expect("config parses");

    assert_eq!(config.model.model_name, "kimi-k2.5");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::Off)
    );
}

#[test]
fn parses_chutes_model_and_defaults_safety_from_model() {
    let config: AppConfig = toml::from_str(
        r#"
            [chutes]
            api_key = "secret"

            [model]
            model_name = "zai-org/GLM-5-TEE"
            reasoning = "default"
            "#,
    )
    .expect("config parses");

    assert!(config.azure.is_none());
    assert_eq!(config.chutes.as_ref().expect("chutes").api_key, "secret");
    assert_eq!(config.model.model_name, "zai-org/GLM-5-TEE");
    assert_eq!(config.model.reasoning, ReasoningSetting::Default);
    assert_eq!(config.safety.model_name, "zai-org/GLM-5-TEE");
    assert_eq!(config.safety.reasoning, ReasoningSetting::Default);
}

#[test]
fn parses_ollama_model_and_defaults_safety_from_model() {
    let config: AppConfig = toml::from_str(
        r#"
            [ollama]
            api_key = "ollama-secret"

            [model]
            model_name = "glm-5.1:cloud"
            reasoning = "default"
            "#,
    )
    .expect("config parses");

    assert!(config.azure.is_none());
    assert_eq!(
        config.ollama.as_ref().expect("ollama").api_key,
        "ollama-secret"
    );
    assert_eq!(config.model.model_name, "glm-5.1:cloud");
    assert_eq!(config.model.reasoning, ReasoningSetting::Default);
    assert_eq!(config.safety.model_name, "glm-5.1:cloud");
    assert_eq!(config.safety.reasoning, ReasoningSetting::Default);
}

#[test]
fn known_model_parse_rejects_cross_family_reasoning_value() {
    let error = toml::from_str::<AppConfig>(
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "kimi-k2.5"
            reasoning = "medium"
            "#,
    )
    .expect_err("config should fail to parse");

    assert!(
        error
            .to_string()
            .contains("azure.reasoning `medium` is not supported by model `kimi-k2.5`")
    );
}

#[test]
fn unknown_model_parse_rejects_config_immediately() {
    let error = toml::from_str::<AppConfig>(
        r#"
            [model]
            model_name = "mystery-model"
            reasoning = "medium"
            "#,
    )
    .expect_err("config should fail to parse");

    assert!(
        error
            .to_string()
            .contains("Warning: unknown model.model_name `mystery-model`")
    );
}

#[test]
fn load_from_path_replaces_unknown_main_model_with_default_selection() {
    let path = unique_temp_path("stale-main-model");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(config.safety.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_path_replaces_unknown_openrouter_model_with_openrouter_default() {
    let path = unique_temp_path("stale-openrouter-main-model");

    fs::write(
        &path,
        r#"
            [openrouter]
            api_key = "or-secret"

            [model]
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.model.model_name, "openai/gpt-5.4-mini");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(config.safety.model_name, "openai/gpt-5.4-mini");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_path_replaces_unknown_ollama_model_with_ollama_default() {
    let path = unique_temp_path("stale-ollama-main-model");

    fs::write(
        &path,
        r#"
            [ollama]
            api_key = "ollama-secret"

            [model]
            model_name = "glm-4.9:cloud"
            reasoning = "default"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.model.model_name, "glm-5.1:cloud");
    assert_eq!(config.model.reasoning, ReasoningSetting::Default);
    assert_eq!(config.safety.model_name, "glm-5.1:cloud");
    assert_eq!(config.safety.reasoning, ReasoningSetting::Default);

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_path_normalizes_invalid_main_model_reasoning() {
    let path = unique_temp_path("invalid-main-reasoning");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "xhigh"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_path_rebases_unknown_safety_and_memory_models_to_current_main_selection() {
    let path = unique_temp_path("stale-secondary-models");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "kimi-k2.5"
            reasoning = "on"

            [safety]
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"

            [memory.extraction]
            enabled = true
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.safety.model_name, "kimi-k2.5");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::On)
    );
    assert_eq!(config.memory.extraction.model_name, "kimi-k2.5");
    assert_eq!(
        config.memory.extraction.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::On)
    );
    assert!(config.memory.extraction.enabled);

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_path_sanitizes_planning_agents() {
    let path = unique_temp_path("stale-planning-models");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"

            [[planning.agents]]
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"

            [[planning.agents]]
            model_name = "gpt-5.4-mini"
            reasoning = "high"

            [[planning.agents]]
            model_name = "gpt-5.4"
            reasoning = "high"

            [[planning.agents]]
            model_name = "gpt-5.4"
            reasoning = "low"

            [[planning.agents]]
            model_name = "kimi-k2.5"
            reasoning = "medium"

            [[planning.agents]]
            model_name = "gpt-5.2"
            reasoning = "on"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(
        config.planning.agents,
        vec![
            PlanningAgentConfig {
                model_name: "gpt-5.4".into(),
                reasoning: ReasoningEffort::High.into(),
            },
            PlanningAgentConfig {
                model_name: "kimi-k2.5".into(),
                reasoning: KimiThinkingMode::On.into(),
            },
            PlanningAgentConfig {
                model_name: "gpt-5.2".into(),
                reasoning: ReasoningEffort::Medium.into(),
            },
        ]
    );

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn load_from_paths_uses_merged_main_selection_for_stale_override_sections() {
    let home_path = unique_temp_path("home-stale-main-fallback");
    let cwd_path = unique_temp_path("cwd-stale-main-fallback");

    fs::write(
        &home_path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "kimi-k2.5"
            reasoning = "on"
            "#,
    )
    .expect("write home config");

    fs::write(
        &cwd_path,
        r#"
            [safety]
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"

            [memory.extraction]
            enabled = true
            model_name = "qwen/qwen3.6-plus:free"
            reasoning = "medium"
            "#,
    )
    .expect("write cwd config");

    let config =
        AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path)).expect("load merged config");

    assert_eq!(config.model.model_name, "kimi-k2.5");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::On)
    );
    assert_eq!(config.safety.model_name, "kimi-k2.5");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::On)
    );
    assert_eq!(config.memory.extraction.model_name, "kimi-k2.5");
    assert_eq!(
        config.memory.extraction.reasoning,
        ReasoningSetting::Kimi(KimiThinkingMode::On)
    );

    fs::remove_file(home_path).expect("remove home config");
    fs::remove_file(cwd_path).expect("remove cwd config");
}

#[test]
fn validation_requires_chutes_credentials_for_selected_chutes_model() {
    let config = AppConfig {
        azure: None,
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "zai-org/GLM-5-TEE".into(),
            reasoning: ReasoningSetting::Default,
        },
        safety: SafetyConfig {
            model_name: "zai-org/GLM-5-TEE".into(),
            reasoning: ReasoningSetting::Default,
        },
        memory: MemoryConfig::default(),
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    };

    let error = config.validate().expect_err("validation should fail");
    assert!(error.to_string().contains("missing the [chutes] table"));
}

#[test]
fn validation_requires_ollama_credentials_for_selected_ollama_model() {
    let config = AppConfig {
        azure: None,
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "glm-5.1:cloud".into(),
            reasoning: ReasoningSetting::Default,
        },
        safety: SafetyConfig {
            model_name: "glm-5.1:cloud".into(),
            reasoning: ReasoningSetting::Default,
        },
        memory: MemoryConfig::default(),
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    };

    let error = config.validate().expect_err("validation should fail");
    assert!(error.to_string().contains("missing the [ollama] table"));
}

#[test]
fn validation_rejects_blank_ollama_api_key() {
    let config = toml::from_str::<AppConfig>(
        r#"
            [ollama]
            api_key = "   "

            [model]
            model_name = "glm-5.1:cloud"
            reasoning = "default"
            "#,
    )
    .expect("config parses");

    let error = config
        .validate()
        .expect_err("config should fail validation");

    assert!(
        error
            .to_string()
            .contains("ollama.api_key must not be empty")
    );
}

#[test]
fn validation_requires_opencode_credentials_for_selected_opencode_model() {
    let config = AppConfig {
        azure: None,
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "opencode-go/glm-5.1".into(),
            reasoning: ReasoningSetting::Default,
        },
        safety: SafetyConfig {
            model_name: "opencode-go/glm-5.1".into(),
            reasoning: ReasoningSetting::Default,
        },
        memory: MemoryConfig::default(),
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    };

    let error = config.validate().expect_err("validation should fail");
    assert!(error.to_string().contains("missing the [opencode] table"));
}

#[test]
fn validation_rejects_blank_opencode_api_key() {
    let config = toml::from_str::<AppConfig>(
        r#"
            [opencode]
            api_key = "   "

            [model]
            model_name = "opencode-go/glm-5.1"
            reasoning = "default"
            "#,
    )
    .expect("config parses");

    let error = config
        .validate()
        .expect_err("config should fail validation");

    assert!(
        error
            .to_string()
            .contains("opencode.api_key must not be empty")
    );
}

#[test]
fn parses_opencode_model_and_validates_api_key() {
    let config: AppConfig = toml::from_str(
        r#"
            [opencode]
            api_key = "opencode-secret"

            [model]
            model_name = "opencode-go/glm-5.1"
            reasoning = "default"
            "#,
    )
    .expect("config parses");

    let opencode = config.opencode.as_ref().expect("opencode config");
    assert_eq!(opencode.api_key, "opencode-secret");
    assert_eq!(config.model.model_name, "opencode-go/glm-5.1");
    assert_eq!(config.model.reasoning, ReasoningSetting::Default);
    assert_eq!(config.safety.model_name, "opencode-go/glm-5.1");
    assert_eq!(config.safety.reasoning, ReasoningSetting::Default);
}

#[test]
fn load_from_path_replaces_unknown_opencode_model_with_opencode_default() {
    let path = unique_temp_path("stale-opencode-main-model");

    fs::write(
        &path,
        r#"
            [opencode]
            api_key = "opencode-secret"

            [model]
            model_name = "opencode-go/minimax-m2.8"
            reasoning = "default"
            "#,
    )
    .expect("write temp config");

    let config = AppConfig::load_from_path(&path).expect("load sanitized config");

    assert_eq!(config.model.model_name, DEFAULT_OPENCODE_MODEL_NAME);
    assert_eq!(config.model.reasoning, ReasoningSetting::Default);
    assert_eq!(config.safety.model_name, DEFAULT_OPENCODE_MODEL_NAME);
    assert_eq!(config.safety.reasoning, ReasoningSetting::Default);

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn validation_requires_openrouter_credentials_for_selected_openrouter_model() {
    let config = AppConfig {
        azure: None,
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "minimax/minimax-m2.7".into(),
            reasoning: ReasoningEffort::Medium.into(),
        },
        safety: SafetyConfig {
            model_name: "minimax/minimax-m2.7".into(),
            reasoning: ReasoningEffort::Medium.into(),
        },
        memory: MemoryConfig::default(),
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    };

    let error = config.validate().expect_err("validation should fail");
    assert!(error.to_string().contains("missing the [openrouter] table"));
}

#[test]
fn parses_openrouter_model_and_validates_api_key() {
    let config: AppConfig = toml::from_str(
        r#"
            [openrouter]
            api_key = "or-secret"

            [model]
            model_name = "xiaomi/mimo-v2-pro"
            reasoning = "xhigh"
            "#,
    )
    .expect("config parses");

    let openrouter = config.openrouter.as_ref().expect("openrouter config");
    assert_eq!(openrouter.api_key, "or-secret");
    assert_eq!(config.model.model_name, "xiaomi/mimo-v2-pro");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::XHigh)
    );
    assert_eq!(config.safety.model_name, "xiaomi/mimo-v2-pro");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::XHigh)
    );
}

#[test]
fn parses_codex_model_and_auth_from_config_table() {
    let config: AppConfig = toml::from_str(
        r#"
            [codex]
            auth_mode = "api_key"
            OPENAI_API_KEY = "codex-secret"

            [model]
            model_name = "codex/gpt-5.3-codex"
            reasoning = "medium"
            "#,
    )
    .expect("config parses");

    let codex = config.codex.as_ref().expect("codex config");
    assert_eq!(codex.auth_mode, Some(CodexAuthMode::ApiKey));
    assert_eq!(codex.openai_api_key.as_deref(), Some("codex-secret"));
    assert_eq!(config.model.model_name, "codex/gpt-5.3-codex");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    assert_eq!(config.safety.model_name, "codex/gpt-5.3-codex");
}

#[test]
fn codex_auth_mode_requires_matching_credentials() {
    let config = toml::from_str::<AppConfig>(
        r#"
            [codex]
            auth_mode = "api_key"

            [model]
            model_name = "codex/gpt-5.3-codex"
            reasoning = "medium"
            "#,
    )
    .expect("config parses");

    let error = config
        .validate()
        .expect_err("config should fail validation");

    assert!(
        error
            .to_string()
            .contains("codex.OPENAI_API_KEY must not be empty")
    );
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
            model_name = "gpt-5.4"
            reasoning = "low"

            [ui]
            show_thinking = true
            show_tool_output = false
            command_history_limit = 50

            [subagents]
            max_concurrent = 8

            [tools]
            max_output_tokens = 4000
            "#,
    )
    .expect("write home config");

    fs::write(
        &cwd_path,
        r#"
            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "high"

            [ui]
            show_tool_output = true
            command_history_limit = 7

            [subagents]
            max_concurrent = 2

            [tools]
            search_include_patterns = [".scratch/**"]
            "#,
    )
    .expect("write cwd config");

    let config =
        AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path)).expect("merged config loads");

    let azure = config.azure.as_ref().expect("azure");
    assert_eq!(azure.resource_name, "home-resource");
    assert_eq!(azure.api_key, "home-secret");
    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    assert_eq!(config.safety.model_name, "gpt-5.4-mini");
    assert_eq!(
        config.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    assert!(config.ui.show_thinking);
    assert!(config.ui.show_tool_output);
    assert_eq!(config.ui.command_history_limit, 7);
    assert_eq!(config.subagents.max_concurrent, 2);
    assert_eq!(config.tools.max_output_tokens, 4000);
    assert_eq!(config.tools.search_include_patterns, vec![".scratch/**"]);

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
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
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

    let config =
        AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path)).expect("merged config loads");

    assert_eq!(config.model.model_name, "gpt-5.4-mini");
    assert_eq!(config.safety.model_name, "gpt-5.4-mini");
    assert!(!config.ui.show_thinking);
    assert!(!config.ui.show_tool_output);
    assert_eq!(config.ui.command_history_limit, 9);
    assert_eq!(config.subagents.max_concurrent, 4);
    assert_eq!(
        config.tools.max_output_tokens,
        tool_policy::default_tool_output_max_tokens()
    );

    fs::remove_file(home_path).expect("remove home config");
    fs::remove_file(cwd_path).expect("remove cwd config");
}

#[test]
fn set_reasoning_updates_model_table() {
    let path = unique_temp_path("reasoning-update");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let updated = AppConfig::set_reasoning_at_path(&path, ReasoningEffort::High.into())
        .expect("update config");

    assert_eq!(
        updated.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("[model]"));
    assert!(raw.contains("reasoning = \"high\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn set_planning_agents_updates_config_file() {
    let path = unique_temp_path("planning-update");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let updated = AppConfig::set_planning_agents_at_path(
        &path,
        &[PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning: ReasoningEffort::High.into(),
        }],
    )
    .expect("update planning config");

    assert_eq!(
        updated.planning.agents,
        vec![PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning: ReasoningEffort::High.into(),
        }]
    );

    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("model_name = \"gpt-5.4\""));
    assert!(raw.contains("reasoning = \"high\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn set_model_selection_updates_model_table() {
    let path = unique_temp_path("model-selection");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5.4"
            reasoning = "low"
            "#,
    )
    .expect("write temp config");

    let updated = AppConfig::set_model_selection_at_path(
        &path,
        "gpt-5.4-mini",
        ReasoningEffort::Medium.into(),
    )
    .expect("update config");

    assert_eq!(updated.model.model_name, "gpt-5.4-mini");
    assert_eq!(
        updated.model.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::Medium)
    );
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("[model]"));
    assert!(raw.contains("model_name = \"gpt-5.4-mini\""));
    assert!(raw.contains("reasoning = \"medium\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn set_safety_selection_updates_config_file() {
    let path = unique_temp_path("safety-selection");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let updated =
        AppConfig::set_safety_selection_at_path(&path, "gpt-5.4", ReasoningEffort::High.into())
            .expect("update config");

    assert_eq!(updated.safety.model_name, "gpt-5.4");
    assert_eq!(
        updated.safety.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("[safety]"));
    assert!(raw.contains("model_name = \"gpt-5.4\""));
    assert!(raw.contains("reasoning = \"high\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn set_memory_selection_updates_config_file() {
    let path = unique_temp_path("memory-selection");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"

            [model]
            model_name = "gpt-5.4-mini"
            reasoning = "medium"
            "#,
    )
    .expect("write temp config");

    let updated =
        AppConfig::set_memory_selection_at_path(&path, "gpt-5.4", ReasoningEffort::High.into())
            .expect("update config");

    assert_eq!(updated.memory.extraction.model_name, "gpt-5.4");
    assert_eq!(
        updated.memory.extraction.reasoning,
        ReasoningSetting::Gpt(ReasoningEffort::High)
    );
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("[memory.extraction]"));
    assert!(raw.contains("model_name = \"gpt-5.4\""));
    assert!(raw.contains("reasoning = \"high\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn default_config_update_path_prefers_existing_cwd_config() {
    let home_path = unique_temp_path("home-effort");
    let cwd_path = unique_temp_path("cwd-effort");
    fs::write(&cwd_path, "").expect("write cwd config");

    let selected =
        default_config_update_path(Some(&home_path), Some(&cwd_path)).expect("select path");

    assert_eq!(selected, cwd_path);

    fs::remove_file(selected).expect("remove cwd config");
}

#[test]
fn default_config_update_path_uses_home_when_cwd_config_is_missing() {
    let home_path = unique_temp_path("home-fallback");
    let cwd_path = unique_temp_path("cwd-missing");

    let selected =
        default_config_update_path(Some(&home_path), Some(&cwd_path)).expect("select path");

    assert_eq!(selected, home_path);
}

#[test]
fn validation_rejects_zero_subagent_concurrency() {
    let mut config = sample_config();
    config.subagents.max_concurrent = 0;
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_zero_tool_output_token_limit() {
    let mut config = sample_config();
    config.tools.max_output_tokens = 0;
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_invalid_memory_extraction_thresholds() {
    let mut config = sample_config();
    config.memory.extraction.min_candidate_confidence = 90;
    config.memory.extraction.min_active_confidence = 80;
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_invalid_memory_retrieval_thresholds() {
    let mut config = sample_config();
    config.memory.retrieval.search.min_total_score = 0.0;
    assert!(config.validate().is_err());

    let mut config = sample_config();
    config.memory.retrieval.auto_inject.min_semantic_score = 1.2;
    assert!(config.validate().is_err());

    let mut config = sample_config();
    config.memory.retrieval.candidate_linking.min_lexical_score = -1.0;
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_invalid_search_include_patterns() {
    let mut config = sample_config();
    config.tools.search_include_patterns = vec!["[".into()];
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_unsupported_safety_reasoning_setting() {
    let mut config = sample_config();
    config.safety.reasoning = ReasoningEffort::Minimal.into();
    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_unknown_safety_model() {
    let mut config = sample_config();
    config.safety.model_name = "mystery-model".into();

    let error = config.validate().expect_err("validation should fail");
    assert!(
        error
            .to_string()
            .contains("Warning: unknown safety.model_name `mystery-model`")
    );
}

#[test]
fn validation_rejects_duplicate_planning_models_with_specific_message() {
    let mut config = sample_config();
    config.planning.agents = vec![
        PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning: ReasoningEffort::High.into(),
        },
        PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning: ReasoningEffort::Low.into(),
        },
    ];

    let error = config.validate().expect_err("validation should fail");
    assert!(error.to_string().contains("duplicate model `gpt-5.4`"));
}
