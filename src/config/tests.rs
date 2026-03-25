use super::*;
use crate::{features::planning::PlanningConfig, tool_policy};
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

            [subagents]
            max_concurrent = 6

            [tools]
            search_include_patterns = [".research/**"]
            max_output_tokens = 2048
            "#,
    )
    .expect("config parses");

    assert_eq!(config.azure.resource_name, "demo-resource");
    assert_eq!(config.azure.reasoning_effort, ReasoningEffort::Medium);
    assert_eq!(config.safety.model_name, "gpt-5-mini");
    assert_eq!(config.safety.reasoning_effort, ReasoningEffort::Medium);
    assert!(!config.ui.show_thinking);
    assert!(config.ui.show_tool_output);
    assert_eq!(config.ui.command_history_limit, 42);
    assert_eq!(config.subagents.max_concurrent, 6);
    assert_eq!(config.tools.search_include_patterns, vec![".research/**"]);
    assert_eq!(config.tools.max_output_tokens, 2048);
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
    assert_eq!(config.subagents.max_concurrent, 4);
    assert!(config.tools.search_include_patterns.is_empty());
    assert_eq!(config.safety.model_name, "gpt-5-mini");
    assert_eq!(config.safety.reasoning_effort, ReasoningEffort::Medium);
    assert_eq!(
        config.tools.max_output_tokens,
        tool_policy::default_tool_output_max_tokens()
    );
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
        safety: SafetyConfig {
            model_name: "gpt-5-mini".into(),
            reasoning_effort: ReasoningEffort::Low,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        tools: ToolConfig::default(),
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
fn known_registry_models_reject_unsupported_reasoning_effort() {
    let config = AppConfig {
        azure: AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Minimal,
            api_version: default_api_version(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        tools: ToolConfig::default(),
    };

    assert!(config.validate().is_err());
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
            [azure]
            model_name = "cwd-model"
            reasoning_effort = "high"

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

    assert_eq!(config.azure.resource_name, "home-resource");
    assert_eq!(config.azure.api_key, "home-secret");
    assert_eq!(config.azure.model_name, "cwd-model");
    assert_eq!(config.azure.reasoning_effort, ReasoningEffort::High);
    assert_eq!(config.safety.model_name, "cwd-model");
    assert_eq!(config.safety.reasoning_effort, ReasoningEffort::High);
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

    let config =
        AppConfig::load_from_paths(Some(&home_path), Some(&cwd_path)).expect("merged config loads");

    assert_eq!(config.azure.model_name, "gpt-5-mini");
    assert_eq!(config.safety.model_name, "gpt-5-mini");
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
fn set_planning_agents_updates_config_file() {
    let path = std::env::temp_dir().join(format!(
        "oat-planning-config-{}-{}.toml",
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
            model_name = "gpt-5.4-mini"
            reasoning_effort = "medium"
            "#,
    )
    .expect("write temp config");

    let updated = AppConfig::set_planning_agents_at_path(
        &path,
        &[PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning_effort: ReasoningEffort::High,
        }],
    )
    .expect("update planning config");

    assert_eq!(
        updated.planning.agents,
        vec![PlanningAgentConfig {
            model_name: "gpt-5.4".into(),
            reasoning_effort: ReasoningEffort::High,
        }]
    );

    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("model_name = \"gpt-5.4\""));
    assert!(raw.contains("reasoning_effort = \"high\""));

    fs::remove_file(path).expect("remove temp config");
}

#[test]
fn set_model_selection_updates_config_file() {
    let path = unique_temp_path("model-selection");

    fs::write(
        &path,
        r#"
            [azure]
            resource_name = "demo-resource"
            api_key = "secret"
            model_name = "gpt-5-mini"
            reasoning_effort = "minimal"
            "#,
    )
    .expect("write temp config");

    let updated =
        AppConfig::set_model_selection_at_path(&path, "gpt-5.4-mini", ReasoningEffort::Medium)
            .expect("update config");

    assert_eq!(updated.azure.model_name, "gpt-5.4-mini");
    assert_eq!(updated.azure.reasoning_effort, ReasoningEffort::Medium);
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("model_name = \"gpt-5.4-mini\""));
    assert!(raw.contains("reasoning_effort = \"medium\""));

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
            model_name = "gpt-5.4-mini"
            reasoning_effort = "medium"
            "#,
    )
    .expect("write temp config");

    let updated = AppConfig::set_safety_selection_at_path(&path, "gpt-5.4", ReasoningEffort::High)
        .expect("update config");

    assert_eq!(updated.safety.model_name, "gpt-5.4");
    assert_eq!(updated.safety.reasoning_effort, ReasoningEffort::High);
    let raw = fs::read_to_string(&path).expect("read updated config");
    assert!(raw.contains("[safety]"));
    assert!(raw.contains("model_name = \"gpt-5.4\""));
    assert!(raw.contains("reasoning_effort = \"high\""));

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
    let config = AppConfig {
        azure: AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
            api_version: default_api_version(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig { max_concurrent: 0 },
        planning: PlanningConfig::default(),
        tools: ToolConfig::default(),
    };

    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_zero_tool_output_token_limit() {
    let config = AppConfig {
        azure: AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
            api_version: default_api_version(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        tools: ToolConfig {
            search_include_patterns: Vec::new(),
            max_output_tokens: 0,
        },
    };

    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_invalid_search_include_patterns() {
    let config = AppConfig {
        azure: AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
            api_version: default_api_version(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        tools: ToolConfig {
            search_include_patterns: vec!["[".into()],
            max_output_tokens: tool_policy::default_tool_output_max_tokens(),
        },
    };

    assert!(config.validate().is_err());
}

#[test]
fn validation_rejects_unsupported_safety_reasoning_effort() {
    let config = AppConfig {
        azure: AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Medium,
            api_version: default_api_version(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning_effort: ReasoningEffort::Minimal,
        },
        ui: UiConfig::default(),
        subagents: SubagentConfig::default(),
        planning: PlanningConfig::default(),
        tools: ToolConfig::default(),
    };

    assert!(config.validate().is_err());
}
