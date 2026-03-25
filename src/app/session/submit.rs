use super::{Effect, PendingReply, PendingReplyKind, SlashCommand};
use crate::app::{AppShell as App, PickerSelection};
use crate::config::ReasoningEffort;
use crate::features::planning::{PlanningStage, planning_conversation_prompt};

pub(super) fn submit_message(app: &mut App) -> Option<Effect> {
    if app.has_pending_write_approval() {
        return None;
    }

    if app.has_pending_shell_approval() {
        return submit_shell_approval(app);
    }

    if app.plan_review_selection_active() {
        return submit_plan_review_selection(app);
    }

    if app.has_pending_ask_user() {
        return submit_ask_user(app);
    }

    if app.selection_picker_visible() {
        return submit_picker_selection(app);
    }

    let submitted = app.composer().lines().join("\n");
    let submitted = submitted.trim().to_owned();

    if app.plan_review_feedback_active() {
        return submit_plan_revision_feedback(app, &submitted);
    }

    if app.command_query().is_some() {
        let command_name = app.command_name().unwrap_or_default().to_owned();
        let arguments = app.command_arguments().unwrap_or_default().to_owned();
        return submit_command(app, &command_name, &arguments);
    }

    if app.planning_draft_mode() {
        return submit_planning_draft(app, &submitted);
    }

    if matches!(
        app.planning_session_stage(),
        Some(PlanningStage::Conversation | PlanningStage::Finalizing)
    ) {
        return submit_planning_turn(app, &submitted);
    }

    if app.session.pending_reply.is_some() {
        return None;
    }

    if submitted.is_empty() {
        return None;
    }

    app.record_submitted_input(&submitted);
    app.clear_plan_review();
    app.push_user_message(submitted.clone());
    app.resume_history_follow();
    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Normal));

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted,
        history: app.session_history().to_vec(),
        history_model_name: app.last_history_model_name().map(str::to_string),
    })
}

fn submit_ask_user(app: &mut App) -> Option<Effect> {
    let (request_id, response, _summary) = app.advance_ask_user()?;
    Some(Effect::ResolveAskUser {
        request_id,
        response,
    })
}

fn submit_shell_approval(app: &mut App) -> Option<Effect> {
    let (request_id, decision, _risk) = app.submit_shell_approval()?;
    Some(Effect::ResolveShellApproval {
        request_id,
        decision,
    })
}

pub(super) fn submit_plan_acceptance(app: &mut App) -> Option<Effect> {
    if app.session.pending_reply.is_some() || !app.plan_review_selection_active() {
        return None;
    }

    let visible_prompt = accepted_plan_prompt().to_string();
    let prompt = accepted_plan_implementation_prompt(app);
    app.record_submitted_input(&visible_prompt);
    app.accept_plan_review_for_implementation();
    app.push_user_message(visible_prompt);
    app.resume_history_follow();
    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Normal));

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: Vec::new(),
        history_model_name: None,
    })
}

fn submit_plan_review_selection(app: &mut App) -> Option<Effect> {
    match app.selected_plan_review_index().unwrap_or(0) {
        0 => submit_plan_acceptance(app),
        1 => {
            app.begin_plan_review_feedback();
            None
        }
        _ => None,
    }
}

fn submit_plan_revision_feedback(app: &mut App, submitted: &str) -> Option<Effect> {
    if app.session.pending_reply.is_some()
        || submitted.is_empty()
        || !app.plan_review_feedback_active()
    {
        return None;
    }

    let prompt = format!(
        "Revise the proposed plan based on these comments. Respond with an updated <proposed_plan> block and do not begin implementation yet. Do not use subagents for this revision.\n\n{}",
        submitted
    );
    app.record_submitted_input(submitted);
    app.clear_plan_review();
    app.push_user_message(prompt.clone());
    app.resume_history_follow();
    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Planning));

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: app.session_history().to_vec(),
        history_model_name: app.last_history_model_name().map(str::to_string),
    })
}

fn submit_planning_draft(app: &mut App, submitted: &str) -> Option<Effect> {
    if app.session.pending_reply.is_some() || submitted.is_empty() {
        return None;
    }

    app.consume_planning_draft_mode();
    app.record_submitted_input(submitted);
    app.push_user_message(submitted.to_string());
    app.resume_history_follow();
    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Planning));

    Some(Effect::PromptModel {
        reply_id,
        prompt: planning_conversation_prompt(submitted),
        history: app.session_history().to_vec(),
        history_model_name: app.last_history_model_name().map(str::to_string),
    })
}

fn submit_planning_turn(app: &mut App, submitted: &str) -> Option<Effect> {
    if app.session.pending_reply.is_some() || submitted.is_empty() {
        return None;
    }

    app.record_submitted_input(submitted);
    app.push_user_message(submitted.to_string());
    app.resume_history_follow();
    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Planning));

    Some(Effect::PromptModel {
        reply_id,
        prompt: submitted.to_string(),
        history: app.session_history().to_vec(),
        history_model_name: app.last_history_model_name().map(str::to_string),
    })
}

fn submit_command(app: &mut App, command_name: &str, arguments: &str) -> Option<Effect> {
    let Some(command) = app.selected_command() else {
        app.push_error_message(format!(
            "Unknown command `{command_name}`. Try /new, /stats, /model, /plan, /quit, or /effort."
        ));
        return None;
    };

    if !command.matches_exact(command_name) {
        app.set_composer_text(command.canonical_name());
        return None;
    }

    match command {
        SlashCommand::NewSession => {
            app.reset_session();
            Some(Effect::RotateSession)
        }
        SlashCommand::Compact => submit_compact_command(app, arguments),
        SlashCommand::Stats => submit_stats_command(app, arguments),
        SlashCommand::Model => submit_model_command(app, arguments),
        SlashCommand::Plan => submit_plan_command(app, arguments),
        SlashCommand::Quit => {
            app.session.should_quit = true;
            None
        }
        SlashCommand::Effort => submit_effort_command(app, arguments),
    }
}

fn submit_picker_selection(app: &mut App) -> Option<Effect> {
    match app.apply_picker_selection()? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Reasoning(reasoning_effort) => {
            Some(Effect::SetReasoningEffort { reasoning_effort })
        }
        PickerSelection::PlanningAgent(_) => Some(Effect::SetPlanningAgents {
            planning_agents: app.planning_agents().to_vec(),
        }),
        PickerSelection::SafetySelection {
            model_name,
            reasoning_effort,
        } => Some(Effect::SetSafetySelection {
            model_name,
            reasoning_effort,
        }),
    }
}

fn submit_stats_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /stats");
        return None;
    }

    app.clear_composer();
    Some(Effect::ShowStats)
}

fn submit_compact_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /compact");
        return None;
    }

    if app.has_pending_reply() {
        return None;
    }

    if app.session_history().is_empty() {
        app.clear_composer();
        app.push_agent_message("Nothing to compact.");
        return None;
    }

    app.clear_composer();
    let reply_id = app.session.next_reply_id();
    app.session.pending_reply = Some(PendingReply::new(reply_id, PendingReplyKind::Compacting));
    Some(Effect::CompactHistory)
}

fn submit_model_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /model");
        return None;
    }

    app.clear_composer();
    app.open_model_picker();
    None
}

fn submit_plan_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /plan");
        return None;
    }

    app.enter_planning_draft_mode();
    app.push_agent_message(
        "Describe what you want planned, then press Enter to start an interactive planning session.",
    );
    None
}

fn submit_effort_command(app: &mut App, arguments: &str) -> Option<Effect> {
    let value = arguments.trim();
    let supported_levels = app.supported_reasoning_levels();
    let supported = supported_levels
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        app.push_error_message(format!(
            "Usage: /effort <{supported}>. Current effort is `{}`.",
            app.reasoning_effort().as_str()
        ));
        return None;
    }

    let Some(reasoning_effort) = ReasoningEffort::parse(value) else {
        app.push_error_message(format!(
            "Unknown reasoning effort `{value}`. Choose one of: {supported}."
        ));
        return None;
    };

    if !supported_levels.contains(&reasoning_effort) {
        if let Some(model) = app.current_model_info() {
            app.push_error_message(format!(
                "Model `{}` supports reasoning efforts: {supported}.",
                model.name
            ));
        } else {
            app.push_error_message(format!(
                "Reasoning effort `{}` is not supported. Choose one of: {supported}.",
                reasoning_effort.as_str()
            ));
        }
        return None;
    }

    if reasoning_effort == app.reasoning_effort() {
        app.clear_composer();
        app.push_agent_message(format!(
            "Reasoning effort is already set to `{}`.",
            reasoning_effort.as_str()
        ));
        return None;
    }

    app.clear_composer();
    Some(Effect::SetReasoningEffort { reasoning_effort })
}

fn accepted_plan_prompt() -> &'static str {
    "I accept this plan. Begin implementation now."
}

fn accepted_plan_implementation_prompt(app: &App) -> String {
    let accepted_plan = app.latest_proposed_plan_message().unwrap_or(
        "<proposed_plan>\nAccepted plan content was not found in transcript.\n</proposed_plan>",
    );
    format!(
        concat!(
            "You are no longer in Plan Mode. The plan has been accepted for implementation.\n",
            "Do not say that you still need a developer or system transition out of plan mode.\n",
            "Use the accepted plan below as the implementation brief, explore the workspace as needed, and begin implementation now.\n\n",
            "Accepted plan:\n",
            "{accepted_plan}\n"
        ),
        accepted_plan = accepted_plan
    )
}
