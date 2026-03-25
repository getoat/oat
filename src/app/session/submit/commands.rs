use super::super::{Effect, PendingReplyKind, SlashCommand};
use crate::app::{AppState, ops, query};
use crate::config::ReasoningEffort;

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
    let supported_levels = query::supported_reasoning_levels_state(state);
    let supported = supported_levels
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        ops::transcript::push_error_message(
            state,
            format!(
                "Usage: /effort <{supported}>. Current effort is `{}`.",
                state.session.reasoning_effort.as_str()
            ),
        );
        return None;
    }

    let Some(reasoning_effort) = ReasoningEffort::parse(value) else {
        ops::transcript::push_error_message(
            state,
            format!("Unknown reasoning effort `{value}`. Choose one of: {supported}."),
        );
        return None;
    };

    if !supported_levels.contains(&reasoning_effort) {
        if let Some(model) = query::current_model_info_state(state) {
            ops::transcript::push_error_message(
                state,
                format!(
                    "Model `{}` supports reasoning efforts: {supported}.",
                    model.name
                ),
            );
        } else {
            ops::transcript::push_error_message(
                state,
                format!(
                    "Reasoning effort `{}` is not supported. Choose one of: {supported}.",
                    reasoning_effort.as_str()
                ),
            );
        }
        return None;
    }

    if reasoning_effort == state.session.reasoning_effort {
        ops::composer::clear_composer(state);
        ops::transcript::push_agent_message(
            state,
            format!(
                "Reasoning effort is already set to `{}`.",
                reasoning_effort.as_str()
            ),
        );
        return None;
    }

    ops::composer::clear_composer(state);
    Some(Effect::SetReasoningEffort { reasoning_effort })
}
