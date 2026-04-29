use std::collections::HashMap;

use rig::{
    OneOrMany,
    completion::{
        Message as RigMessage,
        message::{AssistantContent, Text, ToolResult, ToolResultContent, UserContent},
    },
};
use serde_json::json;
use tokio::sync::oneshot;

use super::{
    AskUserController, InteractionResolveResult, LlmService, ResumeOverride, ResumeRequest,
    WriteApprovalController,
    agent_builder::{
        RequestFeatures, http_headers_for_model, mode_preamble, openai_base_url_for_model,
        reasoning_params, request_params,
    },
    compaction::{
        COMPACTION_SUMMARY_PREFIX, message_contains_tool_state, rebuild_compacted_history,
    },
    hooks::{ask_user::PendingAskUserEntry, write_approval::PendingWriteApprovalEntry},
    resume::{ReplayProbe, reconcile_stream_text},
    safety::{SafetyClassifierRiskOutput, minimum_shell_risk, safety_classifier_preamble},
    streaming::{
        PartialToolCall, format_tool_arguments, parse_commentary_message,
        reconcile_completed_reasoning_text, resolve_commentary_message,
    },
};
use crate::{
    agent::AgentContext,
    app::{AccessMode, ApprovalMode, CommandRisk, WriteApprovalDecision},
    ask_user::{
        AskUserAnswer, AskUserAnsweredQuestion, AskUserQuestion, AskUserRequest, AskUserResponse,
        AskUserSelectedAnswer,
    },
    completion_request::CompletionRequestSnapshot,
    config::{
        AppConfig, AzureConfig, CodexAuthMode, CodexConfig, HistoryConfig, KimiThinkingMode,
        MemoryConfig, ModelSelectionConfig, OllamaConfig, OpenRouterConfig, OpencodeConfig,
        ReasoningEffort, ReasoningSetting, SafetyConfig, SubagentConfig, ToolConfig, UiConfig,
        WebSearchMode,
    },
    features::planning::PlanningConfig,
    web::WebService,
};

fn sample_config() -> AppConfig {
    AppConfig {
        azure: Some(AzureConfig {
            resource_name: "demo-resource".into(),
            api_key: "secret".into(),
            api_version: "2025-01-01-preview".into(),
        }),
        chutes: None,
        codex: None,
        ollama: None,
        opencode: None,
        openrouter: None,
        model: ModelSelectionConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning: ReasoningEffort::Minimal.into(),
        },
        safety: SafetyConfig {
            model_name: "gpt-5.4-mini".into(),
            reasoning: ReasoningEffort::Low.into(),
        },
        memory: MemoryConfig::default(),
        ui: UiConfig {
            show_thinking: true,
            show_tool_output: false,
            command_history_limit: 20,
        },
        subagents: SubagentConfig { max_concurrent: 4 },
        planning: PlanningConfig::default(),
        history: HistoryConfig::default(),
        tools: ToolConfig::default(),
    }
}

fn test_web_service() -> WebService {
    WebService::new(sample_config().tools.max_output_tokens).expect("web service")
}

#[test]
fn reasoning_params_match_requested_effort() {
    let params = reasoning_params(
        &sample_config().model.model_name,
        sample_config().model.reasoning,
    );
    assert_eq!(params, json!({ "reasoning_effort": "minimal" }));
}

#[test]
fn kimi_reasoning_params_match_requested_mode() {
    let on = reasoning_params("kimi-k2.5", ReasoningSetting::Kimi(KimiThinkingMode::On));
    let off = reasoning_params("kimi-k2.5", ReasoningSetting::Kimi(KimiThinkingMode::Off));

    assert_eq!(on, json!({}));
    assert_eq!(off, json!({ "thinking": { "type": "disabled" } }));
}

#[test]
fn default_reasoning_params_emit_no_extra_fields() {
    assert_eq!(
        reasoning_params("zai-org/GLM-5-TEE", ReasoningSetting::Default),
        json!({})
    );
}

#[test]
fn openai_base_url_targets_provider_endpoint() {
    assert_eq!(
        openai_base_url_for_model(&sample_config(), "gpt-5.4-mini").expect("base url"),
        "https://demo-resource.openai.azure.com/openai/v1"
    );
}

#[test]
fn openrouter_reasoning_params_use_reasoning_object() {
    assert_eq!(
        reasoning_params(
            "minimax/minimax-m2.7",
            ReasoningSetting::Gpt(ReasoningEffort::XHigh)
        ),
        json!({ "reasoning": { "effort": "xhigh" } })
    );
    assert_eq!(
        reasoning_params(
            "xiaomi/mimo-v2-pro",
            ReasoningSetting::Gpt(ReasoningEffort::None)
        ),
        json!({ "reasoning": { "effort": "none" } })
    );
}

#[test]
fn codex_reasoning_params_use_responses_shape() {
    assert_eq!(
        reasoning_params(
            "codex/gpt-5.3-codex",
            ReasoningSetting::Gpt(ReasoningEffort::High)
        ),
        json!({
            "reasoning": {
                "effort": "high",
                "summary": "auto"
            },
            "store": false
        })
    );
}

#[test]
fn responses_models_add_live_hosted_web_search_when_enabled() {
    let params = request_params(
        "gpt-5.4-mini",
        ReasoningSetting::Gpt(ReasoningEffort::Minimal),
        RequestFeatures {
            web_search: Some(WebSearchMode::Live),
        },
    );

    assert_eq!(params["reasoning_effort"], "minimal");
    assert_eq!(params["tools"][0]["type"], "web_search");
    assert_eq!(params["tools"][0]["external_web_access"], true);
}

#[test]
fn responses_models_add_cached_hosted_web_search_when_enabled() {
    let params = request_params(
        "gpt-5.4-mini",
        ReasoningSetting::Gpt(ReasoningEffort::Minimal),
        RequestFeatures {
            web_search: Some(WebSearchMode::Cached),
        },
    );

    assert_eq!(params["reasoning_effort"], "minimal");
    assert_eq!(params["tools"][0]["type"], "web_search");
    assert_eq!(params["tools"][0]["external_web_access"], false);
}

#[test]
fn request_params_skip_hosted_search_when_feature_disabled() {
    let params = request_params(
        "gpt-5.4-mini",
        ReasoningSetting::Gpt(ReasoningEffort::Minimal),
        RequestFeatures::default(),
    );

    assert!(params.get("tools").is_none());
}

#[test]
fn request_params_skip_hosted_search_for_non_responses_models() {
    let params = request_params(
        "gpt-5-mini",
        ReasoningSetting::Default,
        RequestFeatures {
            web_search: Some(WebSearchMode::Live),
        },
    );

    assert!(params.get("tools").is_none());
}

#[test]
fn ollama_base_url_targets_ollama_cloud() {
    let mut config = sample_config();
    config.ollama = Some(OllamaConfig {
        api_key: "ollama-secret".into(),
    });

    assert_eq!(
        openai_base_url_for_model(&config, "glm-5.1:cloud").expect("base url"),
        "https://ollama.com/v1"
    );

    let headers = http_headers_for_model(&config, "glm-5.1:cloud").expect("headers");
    assert!(headers.get("HTTP-Referer").is_none());
    assert!(headers.get("X-OpenRouter-Title").is_none());
}

#[test]
fn opencode_openai_compatible_models_use_go_v1_base_url() {
    let mut config = sample_config();
    config.opencode = Some(OpencodeConfig {
        api_key: "opencode-secret".into(),
    });

    assert_eq!(
        openai_base_url_for_model(&config, "opencode-go/glm-5.1").expect("base url"),
        "https://opencode.ai/zen/go/v1"
    );

    let headers = http_headers_for_model(&config, "opencode-go/glm-5.1").expect("headers");
    assert!(headers.get("HTTP-Referer").is_none());
    assert!(headers.get("X-OpenRouter-Title").is_none());
}

#[test]
fn opencode_anthropic_models_use_go_base_url() {
    let mut config = sample_config();
    config.opencode = Some(OpencodeConfig {
        api_key: "opencode-secret".into(),
    });

    assert_eq!(
        openai_base_url_for_model(&config, "opencode-go/minimax-m2.7").expect("base url"),
        "https://opencode.ai/zen/go"
    );
}

#[test]
fn openrouter_base_url_and_headers_target_openrouter() {
    let mut config = sample_config();
    config.openrouter = Some(OpenRouterConfig {
        api_key: "or-secret".into(),
    });

    assert_eq!(
        openai_base_url_for_model(&config, "minimax/minimax-m2.7").expect("base url"),
        "https://openrouter.ai/api/v1"
    );

    let headers = http_headers_for_model(&config, "minimax/minimax-m2.7").expect("headers");
    assert_eq!(
        headers.get("HTTP-Referer").expect("referer"),
        "https://getoat.app"
    );
    assert_eq!(
        headers.get("X-OpenRouter-Title").expect("openrouter title"),
        "oat"
    );
}

#[test]
fn codex_headers_include_required_chatgpt_headers() {
    let mut config = sample_config();
    config.codex = Some(CodexConfig {
        auth_mode: Some(CodexAuthMode::Chatgpt),
        access_token: Some("token".into()),
        account_id: Some("acct-123".into()),
        ..CodexConfig::default()
    });

    let headers = http_headers_for_model(&config, "codex/gpt-5.3-codex").expect("headers");
    assert_eq!(
        headers.get("OpenAI-Beta").expect("beta header"),
        "responses=experimental"
    );
    assert_eq!(
        headers.get("originator").expect("originator header"),
        "codex_cli_rs"
    );
    assert_eq!(
        headers
            .get("chatgpt-account-id")
            .expect("account id header"),
        "acct-123"
    );
}

#[test]
fn non_openrouter_models_do_not_add_attribution_headers() {
    let headers = http_headers_for_model(&sample_config(), "gpt-5.4-mini").expect("headers");
    assert!(headers.get("HTTP-Referer").is_none());
    assert!(headers.get("X-OpenRouter-Title").is_none());
}

#[test]
fn format_tool_result_joins_text_parts() {
    let tool_result = ToolResult {
        id: "call_1".into(),
        call_id: None,
        content: OneOrMany::many(vec![
            ToolResultContent::Text(Text {
                text: "first".into(),
            }),
            ToolResultContent::Text(Text {
                text: "second".into(),
            }),
        ])
        .expect("non-empty"),
    };

    assert_eq!(
        super::compaction::format_tool_result(&tool_result),
        "first\nsecond"
    );
}

#[test]
fn format_tool_arguments_serializes_json_compactly() {
    assert_eq!(
        format_tool_arguments(&json!({ "dir": "src", "recursive": true })),
        r#"{"dir":"src","recursive":true}"#
    );
}

#[test]
fn reconcile_stream_text_passes_through_normal_deltas() {
    let mut replay_probe = None;
    assert_eq!(
        reconcile_stream_text(" and more", &mut replay_probe),
        " and more"
    );
}

#[test]
fn reconcile_stream_text_strips_replayed_prefix_at_new_segment_start() {
    let mut replay_probe = Some(ReplayProbe::new("message 1"));
    assert_eq!(
        reconcile_stream_text("message 1 and more", &mut replay_probe),
        " and more"
    );
    assert_eq!(replay_probe, None);
}

#[test]
fn reconcile_stream_text_suppresses_fully_replayed_chunk() {
    let mut replay_probe = Some(ReplayProbe::new("message 1"));
    assert_eq!(reconcile_stream_text("message 1", &mut replay_probe), "");
    assert_eq!(replay_probe, None);
}

#[test]
fn reconcile_stream_text_does_not_strip_unrelated_segment_start() {
    let mut replay_probe = Some(ReplayProbe::new("message 1"));
    assert_eq!(
        reconcile_stream_text("message 2", &mut replay_probe),
        "message 2"
    );
    assert_eq!(replay_probe, None);
}

#[test]
fn reconcile_stream_text_suppresses_chunked_replay_before_new_suffix() {
    let mut output = "Message 1".to_string();
    let mut replay_probe = Some(ReplayProbe::new(&output));

    let first = reconcile_stream_text("Mess", &mut replay_probe);
    assert_eq!(first, "");
    assert_eq!(
        replay_probe,
        Some(ReplayProbe {
            expected: "Message 1".into(),
            buffered: "Mess".into(),
        })
    );

    let second = reconcile_stream_text("age 1", &mut replay_probe);
    assert_eq!(second, "");
    assert_eq!(replay_probe, None);

    let third = reconcile_stream_text("\n\nMessage 2", &mut replay_probe);
    assert_eq!(third, "\n\nMessage 2");
    output.push_str(&third);
    assert_eq!(output, "Message 1\n\nMessage 2");
}

#[test]
fn reconcile_stream_text_emits_only_new_tail_when_chunk_finishes_replay() {
    let mut replay_probe = Some(ReplayProbe::new("Message 1"));
    assert_eq!(
        reconcile_stream_text("Message 1\n\nMessage 2", &mut replay_probe),
        "\n\nMessage 2"
    );
    assert_eq!(replay_probe, None);
}

#[test]
fn reconcile_stream_text_preserves_shared_prefix_until_divergence() {
    let mut replay_probe = Some(ReplayProbe::new("checking the model registry"));

    let first = reconcile_stream_text("checking the ", &mut replay_probe);
    assert_eq!(first, "");

    let second = reconcile_stream_text("plan in the registry", &mut replay_probe);
    assert_eq!(second, "checking the plan in the registry");
    assert_eq!(replay_probe, None);
}

#[test]
fn completed_reasoning_replay_is_suppressed_after_reasoning_deltas() {
    let mut replay_probe = None;
    assert_eq!(
        reconcile_completed_reasoning_text(
            "**Summarizing context**\n\nRepeated block",
            "**Summarizing context**\n\nRepeated block",
            &mut replay_probe,
        ),
        ""
    );
    assert_eq!(replay_probe, None);
}

#[test]
fn completed_reasoning_keeps_only_new_suffix_after_reasoning_deltas() {
    let mut replay_probe = None;
    assert_eq!(
        reconcile_completed_reasoning_text(
            "**Summarizing context**\n\nRepeated block\n\nExtra line",
            "**Summarizing context**\n\nRepeated block",
            &mut replay_probe,
        ),
        "\n\nExtra line"
    );
    assert_eq!(replay_probe, None);
}

#[test]
fn parse_commentary_message_extracts_trimmed_message() {
    assert_eq!(
        parse_commentary_message(r#"{"message":"  Checking the logs now.  "}"#)
            .expect("valid commentary"),
        "Checking the logs now."
    );
}

#[test]
fn parse_commentary_message_rejects_invalid_payload() {
    assert!(parse_commentary_message(r#"{"message":"   "}"#).is_err());
    assert!(parse_commentary_message(r#"{"text":"missing"}"#).is_err());
}

#[test]
fn resolve_commentary_message_prefers_longer_valid_payload() {
    let mut partials = HashMap::from([(
        "call-1".to_string(),
        PartialToolCall::new(
            Some("Commentary".into()),
            r#"{"message":"’ve mapped the registry."}"#.into(),
        ),
    )]);

    assert_eq!(
        resolve_commentary_message(
            &mut partials,
            "call-1",
            r#"{"message":"I’ve mapped the registry."}"#
        )
        .expect("commentary resolves"),
        "I’ve mapped the registry."
    );
    assert!(!partials.contains_key("call-1"));
}

#[test]
fn resolve_commentary_message_falls_back_when_no_delta_payload_exists() {
    let mut partials = HashMap::new();

    assert_eq!(
        resolve_commentary_message(&mut partials, "call-1", r#"{"message":"fallback"}"#)
            .expect("fallback commentary"),
        "fallback"
    );
}

#[test]
fn completion_capture_keeps_latest_request_snapshot() {
    let capture = super::CompletionCapture::new();
    let first_history = vec![RigMessage::system("be concise")];
    let first_prompt = RigMessage::user("inspect src");
    capture.record(&first_prompt, &first_history);

    let second_history = vec![RigMessage::assistant("Working on it.")];
    let second_prompt = RigMessage::user("continue");
    capture.record(&second_prompt, &second_history);

    let snapshot = capture.snapshot().expect("snapshot captured");
    assert_eq!(snapshot.history, second_history);
    assert_eq!(snapshot.prompt, second_prompt);
    assert_eq!(snapshot.message_count, 2);
}

#[test]
fn rebuild_compacted_history_keeps_recent_user_and_tool_state_plus_summary() {
    let history = vec![
        RigMessage::user("first user"),
        RigMessage::assistant("plain assistant"),
        RigMessage::Assistant {
            id: None,
            content: rig::OneOrMany::one(AssistantContent::tool_call(
                "tool-1",
                "List",
                json!({"path":"src"}),
            )),
        },
        RigMessage::User {
            content: rig::OneOrMany::one(UserContent::tool_result(
                "tool-1",
                rig::OneOrMany::one(ToolResultContent::text("tool output")),
            )),
        },
        RigMessage::user("latest user"),
    ];

    let rebuilt = rebuild_compacted_history(&history, "summary text");

    assert_eq!(rebuilt.len(), 5);
    assert!(
        matches!(&rebuilt[0], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text() == "first user")))
    );
    assert!(matches!(&rebuilt[1], RigMessage::Assistant { .. }));
    assert!(matches!(&rebuilt[2], RigMessage::User { .. }));
    assert!(
        matches!(&rebuilt[3], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text() == "latest user")))
    );
    assert!(
        matches!(&rebuilt[4], RigMessage::User { content } if content.iter().any(|item| matches!(item, UserContent::Text(text) if text.text().contains(COMPACTION_SUMMARY_PREFIX) && text.text().contains("summary text"))))
    );
}

#[test]
fn message_contains_tool_state_distinguishes_plain_and_tool_messages() {
    assert!(!message_contains_tool_state(&RigMessage::user(
        "plain user"
    )));
    assert!(!message_contains_tool_state(&RigMessage::assistant(
        "plain assistant"
    )));
    assert!(message_contains_tool_state(&RigMessage::Assistant {
        id: None,
        content: rig::OneOrMany::one(AssistantContent::tool_call(
            "tool-1",
            "List",
            json!({"path":"src"}),
        )),
    }));
    assert!(message_contains_tool_state(&RigMessage::User {
        content: rig::OneOrMany::one(UserContent::tool_result(
            "tool-1",
            rig::OneOrMany::one(ToolResultContent::text("tool output")),
        )),
    }));
}

#[test]
fn read_only_mode_preamble_uses_shared_prompt_and_read_only_suffix() {
    let preamble = mode_preamble(&AgentContext::main(AccessMode::ReadOnly));
    assert!(preamble.contains("You are oat: an opinionated agent thing."));
    assert!(preamble.contains("You are a provider-agnostic coding agent."));
    assert!(preamble.contains("You have three modes: read-only, write, and plan mode."));
    assert!(preamble.contains(
        "When you need to fetch or retrieve a web page, use the WebRun tool instead of hosted `web_search`."
    ));
    assert!(preamble.contains("Use `open` for a known URL"));
    assert!(
        preamble
            .contains("Intermediary updates are provided to the user via the `Commentary` tool.")
    );
    assert!(preamble.contains("You are currently in read-only mode."));
    assert!(!preamble.contains("{{EXECUTION_MODE}}"));
    assert!(preamble.contains("Do not print large amounts of code in read-only mode"));
    assert!(!preamble.contains("You are currently in write mode."));
}

#[tokio::test]
async fn read_write_mode_registers_mutation_tools() {
    let service = LlmService::from_config(
        &sample_config(),
        AgentContext::main(AccessMode::ReadWrite),
        WriteApprovalController::default(),
        Some(AskUserController::default()),
        true,
        None,
        None,
        None,
        test_web_service(),
    )
    .expect("service builds");

    assert!(service.tool_names.contains(&"AskUser".to_string()));
    assert!(service.tool_names.contains(&"Todo".to_string()));
    assert!(service.tool_names.contains(&"ApplyPatches".to_string()));
    assert!(service.tool_names.contains(&"WriteFile".to_string()));
    assert!(service.tool_names.contains(&"DeletePath".to_string()));
    assert!(
        service
            .preamble
            .contains("You are oat: an opinionated agent thing.")
    );
    assert!(
        service
            .preamble
            .contains("You are a provider-agnostic coding agent.")
    );
    assert!(
        service
            .preamble
            .contains("Persist until the task is fully handled end-to-end")
    );
    assert!(
        service
            .preamble
            .contains("You are currently in write mode.")
    );
    assert!(service.preamble.contains("they usually mean to file"));
    assert!(
        service
            .preamble
            .contains("While subagents are running, normally treat that as a handoff")
    );
    assert!(
        !service
            .preamble
            .contains("You are currently in read-only mode.")
    );
}

#[tokio::test]
async fn read_only_mode_omits_mutation_tools() {
    let service = LlmService::from_config(
        &sample_config(),
        AgentContext::main(AccessMode::ReadOnly),
        WriteApprovalController::default(),
        Some(AskUserController::default()),
        true,
        None,
        None,
        None,
        test_web_service(),
    )
    .expect("service builds");

    assert!(service.tool_names.contains(&"AskUser".to_string()));
    assert!(service.tool_names.contains(&"Todo".to_string()));
    assert!(!service.tool_names.contains(&"ApplyPatches".to_string()));
    assert!(!service.tool_names.contains(&"WriteFile".to_string()));
    assert!(!service.tool_names.contains(&"DeletePath".to_string()));
}

#[test]
fn write_approval_controller_returns_resume_request_when_waiter_is_gone() {
    let approvals = WriteApprovalController::default();
    let snapshot = CompletionRequestSnapshot::capture(&RigMessage::user("continue"), &[]);
    let (tx, rx) = oneshot::channel();
    drop(rx);
    approvals
        .inner
        .lock()
        .expect("approval state lock")
        .pending
        .insert(
            "call-1".into(),
            PendingWriteApprovalEntry {
                sender: tx,
                snapshot: Some(snapshot.clone()),
                tool_name: "WriteFile".into(),
                arguments: "{\"path\":\"src/main.rs\"}".into(),
            },
        );

    let result = approvals.resolve("call-1", WriteApprovalDecision::AllowOnce);

    assert_eq!(
        result,
        InteractionResolveResult::Resume(ResumeRequest {
            snapshot,
            override_action: ResumeOverride::WriteApproval {
                tool_name: "WriteFile".into(),
                arguments: "{\"path\":\"src/main.rs\"}".into(),
                decision: WriteApprovalDecision::AllowOnce,
            },
        })
    );
}

#[test]
fn write_approval_controller_can_resolve_when_waiter_is_live_or_snapshot_exists() {
    let approvals = WriteApprovalController::default();

    let (live_tx, _live_rx) = oneshot::channel();
    approvals
        .inner
        .lock()
        .expect("approval state lock")
        .pending
        .insert(
            "live".into(),
            PendingWriteApprovalEntry {
                sender: live_tx,
                snapshot: None,
                tool_name: "WriteFile".into(),
                arguments: "{}".into(),
            },
        );

    let (closed_tx, closed_rx) = oneshot::channel();
    drop(closed_rx);
    approvals
        .inner
        .lock()
        .expect("approval state lock")
        .pending
        .insert(
            "resume".into(),
            PendingWriteApprovalEntry {
                sender: closed_tx,
                snapshot: Some(CompletionRequestSnapshot::capture(
                    &RigMessage::user("continue"),
                    &[],
                )),
                tool_name: "WriteFile".into(),
                arguments: "{}".into(),
            },
        );

    let (dead_tx, dead_rx) = oneshot::channel();
    drop(dead_rx);
    approvals
        .inner
        .lock()
        .expect("approval state lock")
        .pending
        .insert(
            "dead".into(),
            PendingWriteApprovalEntry {
                sender: dead_tx,
                snapshot: None,
                tool_name: "WriteFile".into(),
                arguments: "{}".into(),
            },
        );

    assert!(approvals.can_resolve("live"));
    assert!(approvals.can_resolve("resume"));
    assert!(!approvals.can_resolve("dead"));
}

#[test]
fn ask_user_controller_returns_resume_request_when_waiter_is_gone() {
    let controller = AskUserController::default();
    let snapshot = CompletionRequestSnapshot::capture(&RigMessage::user("continue"), &[]);
    let request = AskUserRequest {
        title: Some("Clarify scope".into()),
        questions: vec![AskUserQuestion {
            id: "scope".into(),
            prompt: "Which scope?".into(),
            answers: vec![AskUserAnswer {
                id: "narrow".into(),
                label: "Narrow".into(),
            }],
        }],
    };
    let response = AskUserResponse {
        questions: vec![AskUserAnsweredQuestion {
            id: "scope".into(),
            prompt: "Which scope?".into(),
            selected_answer: AskUserSelectedAnswer {
                id: "narrow".into(),
                label: "Narrow".into(),
                is_recommended: true,
                is_something_else: false,
            },
            details: String::new(),
        }],
    };
    let (tx, rx) = oneshot::channel();
    drop(rx);
    controller
        .inner
        .lock()
        .expect("ask user state lock")
        .pending
        .insert(
            "call-2".into(),
            PendingAskUserEntry {
                sender: tx,
                snapshot: Some(snapshot.clone()),
                request: request.clone(),
            },
        );

    let result = controller.resolve("call-2", response.clone());

    assert_eq!(
        result,
        InteractionResolveResult::Resume(ResumeRequest {
            snapshot,
            override_action: ResumeOverride::AskUser { request, response },
        })
    );
}

#[test]
fn ask_user_controller_can_resolve_when_waiter_is_live_or_snapshot_exists() {
    let controller = AskUserController::default();
    let request = AskUserRequest {
        title: Some("Clarify scope".into()),
        questions: vec![AskUserQuestion {
            id: "scope".into(),
            prompt: "Which scope?".into(),
            answers: vec![AskUserAnswer {
                id: "narrow".into(),
                label: "Narrow".into(),
            }],
        }],
    };

    let (live_tx, _live_rx) = oneshot::channel();
    controller
        .inner
        .lock()
        .expect("ask user state lock")
        .pending
        .insert(
            "live".into(),
            PendingAskUserEntry {
                sender: live_tx,
                snapshot: None,
                request: request.clone(),
            },
        );

    let (closed_tx, closed_rx) = oneshot::channel();
    drop(closed_rx);
    controller
        .inner
        .lock()
        .expect("ask user state lock")
        .pending
        .insert(
            "resume".into(),
            PendingAskUserEntry {
                sender: closed_tx,
                snapshot: Some(CompletionRequestSnapshot::capture(
                    &RigMessage::user("continue"),
                    &[],
                )),
                request: request.clone(),
            },
        );

    let (dead_tx, dead_rx) = oneshot::channel();
    drop(dead_rx);
    controller
        .inner
        .lock()
        .expect("ask user state lock")
        .pending
        .insert(
            "dead".into(),
            PendingAskUserEntry {
                sender: dead_tx,
                snapshot: None,
                request,
            },
        );

    assert!(controller.can_resolve("live"));
    assert!(controller.can_resolve("resume"));
    assert!(!controller.can_resolve("dead"));
}

#[tokio::test]
async fn write_mode_preamble_is_the_same_for_both_approval_modes() {
    let manual = LlmService::from_config(
        &sample_config(),
        AgentContext::main(AccessMode::ReadWrite),
        WriteApprovalController::new(ApprovalMode::Manual),
        Some(AskUserController::default()),
        true,
        None,
        None,
        None,
        test_web_service(),
    )
    .expect("manual service builds")
    .preamble;
    let disabled = LlmService::from_config(
        &sample_config(),
        AgentContext::main(AccessMode::ReadWrite),
        WriteApprovalController::new(ApprovalMode::Disabled),
        Some(AskUserController::default()),
        true,
        None,
        None,
        None,
        test_web_service(),
    )
    .expect("disabled service builds")
    .preamble;

    assert!(manual.contains(
        "When you need to fetch or retrieve a web page, use the WebRun tool instead of hosted `web_search`."
    ));
    assert!(manual.contains("Use `open` for a known URL"));
    assert_eq!(manual, disabled);
}

#[tokio::test]
async fn todo_tool_can_be_disabled_independently_of_ask_user() {
    let service = LlmService::from_config(
        &sample_config(),
        AgentContext::main(AccessMode::ReadOnly),
        WriteApprovalController::default(),
        Some(AskUserController::default()),
        false,
        None,
        None,
        None,
        test_web_service(),
    )
    .expect("service builds");

    assert!(service.tool_names.contains(&"AskUser".to_string()));
    assert!(!service.tool_names.contains(&"Todo".to_string()));
}

#[test]
fn write_approval_controller_can_start_disabled() {
    let approvals = WriteApprovalController::new(ApprovalMode::Disabled);
    assert_eq!(approvals.mode(), ApprovalMode::Disabled);
}

#[test]
fn safety_preamble_allows_read_only_git_commands_to_be_low() {
    let preamble = safety_classifier_preamble();
    assert!(preamble.contains("structured output schema"));
    assert!(preamble.contains("Set `risk` to Low, Medium, or High."));
    assert!(preamble.contains("Set `explanation` to a concise justification."));
    assert!(preamble.contains("10 words or fewer when possible"));
    assert!(preamble.contains("side effects"));
    assert!(
        preamble
            .contains("Long-running, polling, watch-mode, or infinite commands can still be Low")
    );
    assert!(preamble.contains("Git commands are not automatically High."));
    assert!(preamble.contains("status, diff, log, show, and ls-remote can be Low"));
}

#[test]
fn minimum_shell_risk_does_not_force_git_status_high() {
    assert_eq!(minimum_shell_risk("git status", "git status"), None);
    assert_eq!(
        minimum_shell_risk("git diff --stat", "git diff --stat"),
        None
    );
    assert_eq!(
        minimum_shell_risk(
            "while true; do printf 'tick\\n'; sleep 1; done",
            "while true; do printf 'tick\\n'; sleep 1; done"
        ),
        None
    );
    assert_eq!(
        minimum_shell_risk("rm -rf target", "rm -rf target"),
        Some(CommandRisk::High)
    );
}

#[test]
fn safety_classifier_risk_output_converts_to_command_risk() {
    assert_eq!(
        CommandRisk::from(SafetyClassifierRiskOutput::Low),
        CommandRisk::Low
    );
    assert_eq!(
        CommandRisk::from(SafetyClassifierRiskOutput::Medium),
        CommandRisk::Medium
    );
    assert_eq!(
        CommandRisk::from(SafetyClassifierRiskOutput::High),
        CommandRisk::High
    );
}
