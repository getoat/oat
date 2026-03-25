use super::super::{Effect, PendingReplyKind, SlashCommand};
use crate::app::ReducerContext;
use crate::config::ReasoningEffort;

pub(super) fn submit_command(
    ctx: &mut ReducerContext<'_>,
    command_name: &str,
    arguments: &str,
) -> Option<Effect> {
    let Some(command) = ctx.selected_command() else {
        ctx.push_error_message(format!(
            "Unknown command `{command_name}`. Try /new, /stats, /model, /plan, /quit, or /effort."
        ));
        return None;
    };

    if !command.matches_exact(command_name) {
        ctx.set_composer_text(command.canonical_name());
        return None;
    }

    match command {
        SlashCommand::NewSession => {
            ctx.reset_session();
            Some(Effect::RotateSession)
        }
        SlashCommand::Compact => submit_compact_command(ctx, arguments),
        SlashCommand::Stats => submit_stats_command(ctx, arguments),
        SlashCommand::Model => submit_model_command(ctx, arguments),
        SlashCommand::Plan => submit_plan_command(ctx, arguments),
        SlashCommand::Quit => {
            ctx.set_should_quit();
            None
        }
        SlashCommand::Effort => submit_effort_command(ctx, arguments),
    }
}

fn submit_stats_command(ctx: &mut ReducerContext<'_>, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ctx.push_error_message("Usage: /stats");
        return None;
    }

    ctx.clear_composer();
    Some(Effect::ShowStats)
}

fn submit_compact_command(ctx: &mut ReducerContext<'_>, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ctx.push_error_message("Usage: /compact");
        return None;
    }

    if ctx.has_pending_reply() {
        return None;
    }

    if ctx.session_history().is_empty() {
        ctx.clear_composer();
        ctx.push_agent_message("Nothing to compact.");
        return None;
    }

    ctx.clear_composer();
    let reply_id = ctx.next_reply_id();
    ctx.set_pending_reply(reply_id, PendingReplyKind::Compacting);
    Some(Effect::CompactHistory)
}

fn submit_model_command(ctx: &mut ReducerContext<'_>, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ctx.push_error_message("Usage: /model");
        return None;
    }

    ctx.clear_composer();
    ctx.open_model_picker();
    None
}

fn submit_plan_command(ctx: &mut ReducerContext<'_>, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        ctx.push_error_message("Usage: /plan");
        return None;
    }

    ctx.enter_planning_draft_mode();
    ctx.push_agent_message(
        "Describe what you want planned, then press Enter to start an interactive planning session.",
    );
    None
}

fn submit_effort_command(ctx: &mut ReducerContext<'_>, arguments: &str) -> Option<Effect> {
    let value = arguments.trim();
    let supported_levels = ctx.supported_reasoning_levels();
    let supported = supported_levels
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        ctx.push_error_message(format!(
            "Usage: /effort <{supported}>. Current effort is `{}`.",
            ctx.reasoning_effort().as_str()
        ));
        return None;
    }

    let Some(reasoning_effort) = ReasoningEffort::parse(value) else {
        ctx.push_error_message(format!(
            "Unknown reasoning effort `{value}`. Choose one of: {supported}."
        ));
        return None;
    };

    if !supported_levels.contains(&reasoning_effort) {
        if let Some(model) = ctx.current_model_info() {
            ctx.push_error_message(format!(
                "Model `{}` supports reasoning efforts: {supported}.",
                model.name
            ));
        } else {
            ctx.push_error_message(format!(
                "Reasoning effort `{}` is not supported. Choose one of: {supported}.",
                reasoning_effort.as_str()
            ));
        }
        return None;
    }

    if reasoning_effort == ctx.reasoning_effort() {
        ctx.clear_composer();
        ctx.push_agent_message(format!(
            "Reasoning effort is already set to `{}`.",
            reasoning_effort.as_str()
        ));
        return None;
    }

    ctx.clear_composer();
    Some(Effect::SetReasoningEffort { reasoning_effort })
}
