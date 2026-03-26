use super::super::{Effect, PendingReplyKind, SlashCommand};
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
                "Unknown command `{command_name}`. Try /new, /stats, /model, /plan, /quit, or /effort."
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
        SlashCommand::Compact => submit_compact_command(state, arguments),
        SlashCommand::Stats => submit_stats_command(state, arguments),
        SlashCommand::Model => submit_model_command(state, arguments),
        SlashCommand::Plan => submit_plan_command(state, arguments),
        SlashCommand::Quit => {
            ops::session::set_should_quit(state);
            None
        }
        SlashCommand::Effort => submit_effort_command(state, arguments),
    }
}

fn submit_stats_command(state: &mut AppState, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ops::transcript::push_error_message(state, "Usage: /stats");
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::ShowStats)
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
    ops::picker::open_model_picker(state);
    None
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
        ChatMessage, MessageStyle, PendingReply, PendingReplyKind, SessionHistoryMessage, Speaker,
        TranscriptEntry,
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

        assert!(effect.is_none());
        assert!(app.selection_picker_visible());
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
