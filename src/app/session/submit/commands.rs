use super::super::{
    Effect, PendingReplyKind, SessionHistoryMessage, SideChannelKind, SlashCommand,
};
use crate::app::{AppState, ops, query};
use crate::model_registry::{self, ParseReasoningSettingError};

pub(super) fn submit_command(
    state: &mut AppState,
    command_name: &str,
    arguments: &str,
) -> Option<Effect> {
    let Some(command) = ops::composer::selected_command(state) else {
        ops::transcript::push_error_message(
            state,
            format!(
                "Unknown command `{command_name}`. Try /new, /resume, /btw, /compact, /memory, /stats, /model, /effort, /login, /logout, /terminals, /terminal, /kill-terminal, /plan, or /quit."
            ),
        );
        return None;
    };

    if !command.matches_exact(command_name) {
        ops::composer::set_composer_text(state, command.canonical_name());
        return None;
    }

    match command {
        SlashCommand::NewSession => {
            ops::session::reset_session(state);
            Some(Effect::RotateSession)
        }
        SlashCommand::Resume => submit_resume_command(state, arguments),
        SlashCommand::Btw => submit_btw_command(state, arguments),
        SlashCommand::Compact => submit_compact_command(state, arguments),
        SlashCommand::Memory => submit_memory_command(state, arguments),
        SlashCommand::Stats => submit_stats_command(state, arguments),
        SlashCommand::Model => submit_model_command(state, arguments),
        SlashCommand::Plan => submit_plan_command(state, arguments),
        SlashCommand::Login => submit_login_command(state, arguments),
        SlashCommand::Logout => submit_logout_command(state, arguments),
        SlashCommand::Terminals => submit_terminals_command(state, arguments),
        SlashCommand::Terminal => submit_terminal_command(state, arguments),
        SlashCommand::KillTerminal => submit_kill_terminal_command(state, arguments),
        SlashCommand::Quit => {
            ops::session::set_should_quit(state);
            None
        }
        SlashCommand::Effort => submit_effort_command(state, arguments),
    }
}

fn submit_btw_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    let prompt = arguments.trim();
    if prompt.is_empty() {
        ops::transcript::push_error_message(state, "Usage: /btw <question>");
        return None;
    }

    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    let pending = ops::session::begin_side_reply(state, reply_id, SideChannelKind::Btw);
    let (history, history_model_name) = build_btw_request_context(state);
    ops::transcript::push_tagged_user_message(state, pending.label.clone(), prompt.to_string());
    ops::history::resume_history_follow(state);

    Some(Effect::PromptSideChannel {
        reply_id,
        prompt: prompt.to_string(),
        history,
        history_model_name,
    })
}

fn submit_resume_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /resume");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::OpenSessionPicker)
}

fn build_btw_request_context(state: &AppState) -> (Vec<SessionHistoryMessage>, Option<String>) {
    if let Some(seed) = query::active_main_request_seed(state) {
        let mut history = seed.history.clone();
        history.push(SessionHistoryMessage::user(seed.model_prompt.clone()));
        return (history, seed.history_model_name.clone());
    }

    (
        state.session.session_history.to_vec(),
        state.session.last_history_model_name.clone(),
    )
}

fn submit_stats_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /stats");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::ShowStats)
}

fn submit_memory_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        ops::transcript::push_error_message(
            state,
            "Usage: /memory <search <query> | show <id> | candidates | promote <id> | archive <id> | replace <id> <text> | clear | reindex>",
        );
        return None;
    }

    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let subcommand = parts.next().unwrap_or_default();
    let first_arg = parts.next().unwrap_or_default().trim();
    let second_arg = parts.next().unwrap_or_default().trim();
    ops::composer::clear_composer(state);

    match subcommand {
        "search" => {
            if first_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory search <query>");
                None
            } else {
                Some(Effect::SearchMemories {
                    query: trimmed["search".len()..].trim().to_string(),
                    include_candidates: false,
                })
            }
        }
        "show" => {
            if first_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory show <id>");
                None
            } else {
                Some(Effect::ShowMemory {
                    id: first_arg.to_string(),
                })
            }
        }
        "candidates" => Some(Effect::ListMemoryCandidates),
        "promote" => {
            if first_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory promote <id>");
                None
            } else {
                Some(Effect::PromoteMemory {
                    id: first_arg.to_string(),
                })
            }
        }
        "archive" => {
            if first_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory archive <id>");
                None
            } else {
                Some(Effect::ArchiveMemory {
                    id: first_arg.to_string(),
                })
            }
        }
        "replace" => {
            if first_arg.is_empty() || second_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory replace <id> <text>");
                None
            } else {
                Some(Effect::ReplaceMemory {
                    id: first_arg.to_string(),
                    text: second_arg.to_string(),
                })
            }
        }
        "clear" => {
            if !first_arg.is_empty() {
                ops::transcript::push_error_message(state, "Usage: /memory clear");
                None
            } else {
                Some(Effect::ClearMemories)
            }
        }
        "reindex" => Some(Effect::RebuildMemoryIndexes),
        _ => {
            ops::transcript::push_error_message(
                state,
                "Usage: /memory <search <query> | show <id> | candidates | promote <id> | archive <id> | replace <id> <text> | clear | reindex>",
            );
            None
        }
    }
}

fn submit_compact_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /compact");
        return None;
    }

    if query::has_pending_reply(state) {
        return None;
    }

    if state.session.session_history.is_empty() {
        ops::composer::clear_composer(state);
        ops::transcript::push_agent_message(state, "Nothing to compact.");
        return None;
    }

    ops::composer::clear_composer(state);
    let reply_id = ops::session::next_reply_id(state);
    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Compacting);
    Some(Effect::CompactHistory)
}

fn submit_model_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /model");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::OpenModelPicker)
}

fn submit_plan_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /plan");
        return None;
    }

    ops::planning::enter_planning_draft_mode(state);
    ops::transcript::push_agent_message(
        state,
        "Describe what you want planned, then press Enter to start an interactive planning session.",
    );
    None
}

fn submit_terminals_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /terminals");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::ListBackgroundTerminals)
}

fn submit_terminal_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    let id = arguments.trim();
    if id.is_empty() {
        ops::transcript::push_error_message(state, "Usage: /terminal <id>");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::InspectBackgroundTerminal { id: id.into() })
}

fn submit_kill_terminal_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    let id = arguments.trim();
    if id.is_empty() {
        ops::transcript::push_error_message(state, "Usage: /kill-terminal <id>");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::KillBackgroundTerminal { id: id.into() })
}

fn submit_login_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /login");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::LoginCodex)
}

fn submit_logout_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /logout");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::LogoutCodex)
}

fn submit_effort_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    let value = arguments.trim();
    let supported_settings = query::supported_reasoning_settings_state(state);
    let supported = supported_settings
        .iter()
        .map(|setting| setting.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        ops::transcript::push_error_message(
            state,
            format!(
                "Usage: /effort <{supported}>. Current reasoning is `{}`.",
                state.session.reasoning.as_str()
            ),
        );
        return None;
    }

    let reasoning =
        match model_registry::parse_reasoning_setting_for_model(&state.session.model_name, value) {
            Ok(reasoning) => reasoning,
            Err(ParseReasoningSettingError::Unknown) => {
                ops::transcript::push_error_message(
                    state,
                    format!("Unknown reasoning setting `{value}`. Choose one of: {supported}."),
                );
                return None;
            }
            Err(ParseReasoningSettingError::UnsupportedForModel { .. }) => {
                ops::transcript::push_error_message(
                    state,
                    format!(
                        "Model `{}` supports reasoning settings: {supported}.",
                        state.session.model_name
                    ),
                );
                return None;
            }
            Err(ParseReasoningSettingError::UnknownModel) => {
                ops::transcript::push_error_message(
                    state,
                    model_registry::unknown_model_message(
                        "session.model_name",
                        &state.session.model_name,
                    ),
                );
                return None;
            }
        };

    if reasoning == state.session.reasoning {
        ops::composer::clear_composer(state);
        ops::transcript::push_agent_message(
            state,
            format!("Reasoning is already set to `{}`.", reasoning.as_str()),
        );
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::SetReasoning { reasoning })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        ChatMessage, MainRequestSeed, MessageStyle, PendingReply, PendingReplyKind,
        SessionHistoryMessage, Speaker, TranscriptEntry,
        session::test_support::{new_app, registry_app},
    };
    use crate::config::{KimiThinkingMode, ReasoningEffort, ReasoningSetting};

    #[test]
    fn partial_command_completes_before_execution() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/n");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        assert_eq!(app.composer().lines(), ["/new"]);
        assert_eq!(app.entries().len(), 1);
    }

    #[test]
    fn slash_commands_are_not_added_to_recall_history() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/new");

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::RotateSession));
        app.apply(crate::app::Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), [""]);
    }

    #[test]
    fn model_command_opens_model_picker() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/model");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::OpenModelPicker));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn plan_command_enters_planning_draft_mode() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/plan");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.planning_draft_mode());
    }

    #[test]
    fn stats_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/stats");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn memory_clear_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/memory clear");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ClearMemories));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn memory_clear_command_rejects_extra_arguments() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/memory clear now");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected error entry");
        };
        assert_eq!(message.text, "Usage: /memory clear");
    }

    #[test]
    fn resume_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/resume");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::OpenSessionPicker));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn sessions_alias_returns_resume_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/sessions");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::OpenSessionPicker));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn terminals_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/terminals");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ListBackgroundTerminals));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn terminal_command_requires_an_id() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/terminal   ");
        app.sync_command_selection();
        app.apply(crate::app::Action::SelectNextCommand);

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected error entry");
        };
        assert_eq!(message.text, "Usage: /terminal <id>");
    }

    #[test]
    fn kill_terminal_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/kill-terminal terminal-3");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::KillBackgroundTerminal {
                id: "terminal-3".into()
            })
        );
        assert!(!app.composer_has_content());
    }

    #[test]
    fn login_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/login");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::LoginCodex));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn logout_command_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/logout");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::LogoutCodex));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn btw_command_requires_a_question() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/btw   ");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Usage: /btw <question>");
    }

    #[test]
    fn btw_command_uses_finalized_history_when_idle() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous answer")]);
        app.state_mut().session.last_history_model_name = Some("gpt-5-mini".into());
        app.composer_mut().insert_str("/btw follow up?");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptSideChannel {
                reply_id: 1,
                prompt: "follow up?".into(),
                history: vec![SessionHistoryMessage::assistant("previous answer")],
                history_model_name: Some("gpt-5-mini".into()),
            })
        );
        let TranscriptEntry::Message(message) = app.entries().last().expect("user entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.speaker, Speaker::User);
        assert_eq!(message.tag.as_deref(), Some("btw 1"));
        assert_eq!(message.text, "follow up?");
    }

    #[test]
    fn btw_command_clones_active_main_request_prompt_while_reply_is_running() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("main question");

        let initial = app.apply(crate::app::Action::SubmitMessage);
        assert!(matches!(initial, Some(Effect::PromptModel { .. })));

        app.composer_mut().insert_str("/btw side question");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptSideChannel {
                reply_id: 2,
                prompt: "side question".into(),
                history: vec![SessionHistoryMessage::user("main question")],
                history_model_name: None,
            })
        );
        assert!(app.has_pending_reply());
        assert_eq!(app.state().session.pending_side_replies.len(), 1);
    }

    #[test]
    fn btw_command_uses_model_prompt_when_visible_prompt_differs() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.state_mut().session.active_main_request_seed = Some(MainRequestSeed {
            history: vec![SessionHistoryMessage::assistant("prior")],
            visible_prompt: "I accept this plan. Begin implementation now.".into(),
            model_prompt: "You are no longer in Plan Mode. Begin implementation now.".into(),
            history_model_name: Some("gpt-5-mini".into()),
            transcript_len_before: 0,
        });
        app.composer_mut().insert_str("/btw side question");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptSideChannel {
                reply_id: 1,
                prompt: "side question".into(),
                history: vec![
                    SessionHistoryMessage::assistant("prior"),
                    SessionHistoryMessage::user(
                        "You are no longer in Plan Mode. Begin implementation now.",
                    ),
                ],
                history_model_name: Some("gpt-5-mini".into()),
            })
        );
    }

    #[test]
    fn compact_command_returns_effect() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.composer_mut().insert_str("/compact");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::CompactHistory));
        assert!(!app.composer_has_content());
        assert!(app.has_pending_reply());
        assert_eq!(app.history_pending_status_label(), "Compacting context...");
    }

    #[test]
    fn compact_command_without_history_reports_noop() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/compact");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(!app.has_pending_reply());
        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry") else {
            panic!("expected message entry");
        };
        assert_eq!(message.text, "Nothing to compact.");
    }

    #[test]
    fn status_alias_returns_stats_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/status");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
    }

    #[test]
    fn effort_command_returns_effect_for_valid_value() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/effort high");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetReasoning {
                reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
            })
        );
        assert!(!app.composer_has_content());
    }

    #[test]
    fn effort_alias_returns_effect_for_valid_value() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/thinking high");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetReasoning {
                reasoning: ReasoningSetting::Gpt(ReasoningEffort::High),
            })
        );
    }

    #[test]
    fn effort_command_returns_kimi_toggle_for_kimi_models() {
        let mut app = crate::app::App::new(true, false, "kimi-k2.5", KimiThinkingMode::On);
        app.composer_mut().insert_str("/effort off");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetReasoning {
                reasoning: ReasoningSetting::Kimi(KimiThinkingMode::Off),
            })
        );
    }

    #[test]
    fn effort_command_rejects_invalid_value() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/effort turbo");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("Unknown reasoning setting"));
        assert!(app.composer_has_content());
    }

    #[test]
    fn effort_command_rejects_gpt_value_for_kimi_model() {
        let mut app = crate::app::App::new(true, false, "kimi-k2.5", KimiThinkingMode::On);
        app.composer_mut().insert_str("/effort medium");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(
            message
                .text
                .contains("supports reasoning settings: on, off")
        );
    }

    #[test]
    fn effort_command_rejects_unsupported_value_for_registry_model() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/effort xhigh");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("supports reasoning settings"));
    }

    #[test]
    fn effort_command_reports_noop_when_value_is_unchanged() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/effort medium");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Plain);
        assert!(message.text.contains("already set"));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn clear_alias_starts_new_session() {
        let mut app = new_app(true);
        app.state_mut()
            .session
            .entries
            .push(TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::User,
                text: "old".into(),
                style: MessageStyle::Plain,
                tag: None,
            }));
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(8, PendingReplyKind::Normal));
        app.composer_mut().insert_str("/clear");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::RotateSession));
        assert_eq!(app.entries().len(), 1);
        assert!(app.state_mut().session.pending_reply.is_none());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn quit_command_sets_should_quit() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/quit");
        app.sync_command_selection();

        let effect = app.apply(crate::app::Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.should_quit());
    }
}
