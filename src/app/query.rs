use ratatui_textarea::TextArea;
use std::collections::VecDeque;

use crate::{
    config::ReasoningSetting,
    features::planning::{PlanReviewState, PlanningAgentConfig, PlanningStage},
    stats::StatsTotals,
    todo::TodoSnapshot,
};

use super::{
    AccessMode, AppState, ApprovalMode, PendingAskUser, PendingReplyKind, PendingReplyReplaySeed,
    PendingShellApproval, PendingWriteApproval, SelectionPicker, SessionHistoryMessage,
    SessionState, SlashCommand, TranscriptEntry, UiState,
    session::{
        history_pending_status_label, next_request_context_percent,
        should_show_history_busy_indicator, shows_startup_banner, supported_reasoning_settings,
    },
    ui::{
        AskUserUiState, HistoryRenderCache, ShellApprovalUiState, picker_height,
        split_command_query,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputContext {
    WriteApproval,
    ShellApproval {
        editing: bool,
        can_move_up: bool,
        can_move_down: bool,
    },
    AskUser {
        editing: bool,
    },
    PlanReview,
    Stats,
    Picker,
    CommandPalette,
    Composer,
}

pub fn mode(state: &AppState) -> AccessMode {
    state.session.mode
}

pub fn approval_mode(state: &AppState) -> ApprovalMode {
    state.session.approval_mode
}

pub fn safety_model_name(state: &AppState) -> &str {
    &state.session.safety_model_name
}

pub fn safety_reasoning(state: &AppState) -> ReasoningSetting {
    state.session.safety_reasoning
}

pub fn memory_model_name(state: &AppState) -> &str {
    &state.session.memory_model_name
}

pub fn memory_reasoning(state: &AppState) -> ReasoningSetting {
    state.session.memory_reasoning
}

pub fn pending_write_approval(state: &AppState) -> Option<&PendingWriteApproval> {
    state.session.pending_write_approvals.front()
}

pub fn main_pending_write_approval_request_id(state: &AppState) -> Option<&str> {
    state
        .session
        .pending_write_approvals
        .iter()
        .find(|pending| pending.source_label.is_none())
        .map(|pending| pending.request_id.as_str())
}

pub fn main_pending_shell_approval_request_id(state: &AppState) -> Option<&str> {
    state
        .session
        .pending_shell_approvals
        .iter()
        .find(|pending| pending.source_label.is_none())
        .map(|pending| pending.request_id.as_str())
}

pub fn has_pending_shell_approval(state: &AppState) -> bool {
    !state.session.pending_shell_approvals.is_empty()
}

pub fn shell_approval_session(state: &AppState) -> Option<&PendingShellApproval> {
    state.session.pending_shell_approvals.front()
}

pub fn shell_approval_ui(state: &AppState) -> Option<&ShellApprovalUiState> {
    state.ui.pending_shell_approval.as_ref()
}

pub fn pending_ask_user(state: &AppState) -> Option<&PendingAskUser> {
    state.session.pending_ask_user.as_ref()
}

pub fn ask_user_session(state: &AppState) -> Option<&PendingAskUser> {
    pending_ask_user(state)
}

pub fn ask_user_ui(state: &AppState) -> Option<&AskUserUiState> {
    state.ui.pending_ask_user.as_ref()
}

pub fn has_pending_ask_user(state: &AppState) -> bool {
    pending_ask_user(state).is_some()
}

pub fn plan_review_selection_active(state: &AppState) -> bool {
    state.session.planning.stage == PlanningStage::Review
        && state.session.planning.review == Some(PlanReviewState::Selection)
}

pub fn selected_plan_review_index(state: &AppState) -> Option<usize> {
    plan_review_selection_active(state).then_some(state.ui.plan_review_selected_index)
}

pub fn planning_session_stage(state: &AppState) -> Option<PlanningStage> {
    (state.session.planning.stage != PlanningStage::Idle).then_some(state.session.planning.stage)
}

pub fn planning_draft_mode(state: &AppState) -> bool {
    state.session.planning.stage == PlanningStage::Drafting
}

pub fn plan_active(state: &AppState) -> bool {
    state.session.planning.stage != PlanningStage::Idle
        || state
            .session
            .pending_reply
            .as_ref()
            .is_some_and(|pending| pending.kind == PendingReplyKind::Planning)
}

pub fn should_quit(state: &AppState) -> bool {
    state.session.should_quit
}

pub fn composer(state: &AppState) -> &TextArea<'static> {
    &state.ui.composer.composer
}

pub fn entries(state: &AppState) -> &[TranscriptEntry] {
    &state.session.entries
}

pub fn session_history(state: &AppState) -> &[SessionHistoryMessage] {
    &state.session.session_history
}

pub fn session_title(state: &AppState) -> Option<&str> {
    state.session.session_title.as_deref()
}

pub fn session_title_pending(state: &AppState) -> bool {
    state.session.pending_session_title_reply_id.is_some()
}

pub fn shows_startup_banner_state(state: &AppState) -> bool {
    shows_startup_banner(&state.session)
}

pub fn has_pending_reply(state: &AppState) -> bool {
    state.session.pending_reply.is_some()
}

pub fn active_main_request_seed(state: &AppState) -> Option<&crate::app::MainRequestSeed> {
    state.session.active_main_request_seed.as_ref()
}

pub fn queued_messages(state: &AppState) -> &VecDeque<String> {
    &state.session.queued_messages
}

pub fn has_queued_messages(state: &AppState) -> bool {
    !state.session.queued_messages.is_empty()
}

pub fn should_show_history_busy_indicator_state(state: &AppState) -> bool {
    should_show_history_busy_indicator(&state.session)
}

pub fn history_pending_status_label_state(state: &AppState) -> &'static str {
    history_pending_status_label(&state.session)
}

pub fn show_tool_output(state: &AppState) -> bool {
    state.session.show_tool_output
}

pub fn model_name(state: &AppState) -> &str {
    &state.session.model_name
}

pub fn reasoning(state: &AppState) -> ReasoningSetting {
    state.session.reasoning
}

pub fn last_history_model_name(state: &AppState) -> Option<&str> {
    state.session.last_history_model_name.as_deref()
}

pub fn planning_agents(state: &AppState) -> &[PlanningAgentConfig] {
    &state.session.planning_agents
}

#[cfg_attr(not(test), allow(dead_code))]
pub fn current_todo(state: &AppState) -> Option<&TodoSnapshot> {
    state.session.current_todo.as_ref()
}

pub fn session_stats(state: &AppState) -> StatsTotals {
    state.session.session_stats
}

pub fn active_background_terminal_count(state: &AppState) -> usize {
    state.session.active_background_terminal_count
}

pub fn transcript_revision(state: &AppState) -> u64 {
    state.session.transcript_revision
}

pub fn next_request_context_percent_state(state: &AppState) -> u64 {
    next_request_context_percent(&state.session)
}

pub fn tick_count(state: &AppState) -> usize {
    state.session.tick_count
}

pub fn history_render_cache(state: &AppState) -> Option<&HistoryRenderCache> {
    state.ui.history_render_cache.as_ref()
}

pub fn selection_picker(state: &AppState) -> Option<&SelectionPicker> {
    state.ui.picker.as_ref()
}

#[cfg(test)]
pub fn stats_screen_visible(state: &AppState) -> bool {
    state.ui.stats_screen.is_some()
}

#[cfg(test)]
pub fn selection_picker_visible(state: &AppState) -> bool {
    state.ui.picker.is_some()
}

pub fn history_is_pinned(state: &AppState) -> bool {
    state.ui.history.is_pinned()
}

pub fn history_total_lines(state: &AppState) -> usize {
    state.ui.history.total_lines()
}

pub fn history_viewport_rows(state: &AppState) -> usize {
    state.ui.history.viewport_rows()
}

pub fn history_scroll_position(state: &AppState) -> usize {
    state.ui.history.scroll_position()
}

pub fn history_selection_span(state: &AppState, row: usize) -> Option<(usize, usize)> {
    state.ui.history.selection_span_for_row(row)
}

pub fn composer_wrap_width(state: &AppState) -> usize {
    state.ui.composer.wrap_width
}

pub fn command_query(state: &AppState) -> Option<&str> {
    let [line] = state.ui.composer.composer.lines() else {
        return None;
    };

    line.starts_with('/').then_some(line.as_str())
}

pub fn command_name(state: &AppState) -> Option<&str> {
    command_query(state)
        .map(split_command_query)
        .map(|(name, _)| name)
}

pub fn filtered_commands(state: &AppState) -> Vec<SlashCommand> {
    command_name(state)
        .map(SlashCommand::filtered)
        .unwrap_or_default()
}

pub fn selected_command(state: &AppState) -> Option<SlashCommand> {
    let commands = filtered_commands(state);
    commands
        .contains(&state.ui.selected_command)
        .then_some(state.ui.selected_command)
        .or_else(|| commands.first().copied())
}

pub fn supported_reasoning_settings_state(state: &AppState) -> Vec<ReasoningSetting> {
    supported_reasoning_settings(&state.session)
}

pub fn active_reply_id(state: &AppState) -> Option<u64> {
    state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.id)
}

pub fn active_reply_kind(state: &AppState) -> Option<PendingReplyKind> {
    state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.kind)
}

pub fn pending_reply_replay_seed(state: &AppState) -> Option<PendingReplyReplaySeed> {
    state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| PendingReplyReplaySeed {
            plain_text: pending.plain_text.clone(),
            reasoning_text: pending.reasoning_text.clone(),
            commentary_messages: pending.commentary_messages.clone(),
        })
}

pub fn command_palette_visible(state: &AppState) -> bool {
    selection_picker(state).is_none() && command_query(state).is_some()
}

pub fn command_palette_height(state: &AppState, screen_height: u16) -> u16 {
    if !command_palette_visible(state) {
        return 0;
    }

    let line_count = filtered_commands(state).len().max(1) as u16;
    let max_height = (screen_height / 2).max(3);
    (line_count + 2).min(max_height)
}

pub fn overlay_height(state: &AppState, screen_height: u16) -> u16 {
    if let Some(picker) = selection_picker(state) {
        return picker_height(picker, screen_height);
    }

    command_palette_height(state, screen_height)
}

pub(crate) fn input_context(state: &AppState) -> InputContext {
    input_context_parts(&state.session, &state.ui)
}

pub(crate) fn input_context_parts(session: &SessionState, ui: &UiState) -> InputContext {
    if !session.pending_write_approvals.is_empty() {
        InputContext::WriteApproval
    } else if !session.pending_shell_approvals.is_empty() {
        let editing = ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::is_editing);
        InputContext::ShellApproval {
            editing,
            can_move_up: editing
                && ui
                    .pending_shell_approval
                    .as_ref()
                    .is_some_and(ShellApprovalUiState::editor_can_move_up),
            can_move_down: editing
                && ui
                    .pending_shell_approval
                    .as_ref()
                    .is_some_and(ShellApprovalUiState::editor_can_move_down),
        }
    } else if session.pending_ask_user.is_some() {
        InputContext::AskUser {
            editing: ui
                .pending_ask_user
                .as_ref()
                .is_some_and(|pending| pending.detail_editing),
        }
    } else if session.planning.stage == PlanningStage::Review
        && session.planning.review == Some(PlanReviewState::Selection)
    {
        InputContext::PlanReview
    } else if ui.stats_screen.is_some() {
        InputContext::Stats
    } else if ui.picker.is_some() {
        InputContext::Picker
    } else {
        let [line] = ui.composer.composer.lines() else {
            return InputContext::Composer;
        };
        if ui.picker.is_none() && line.starts_with('/') {
            InputContext::CommandPalette
        } else {
            InputContext::Composer
        }
    }
}

pub(crate) fn queue_dispatch_ready(state: &AppState) -> bool {
    has_queued_messages(state)
        && !has_pending_reply(state)
        && pending_write_approval(state).is_none()
        && !has_pending_shell_approval(state)
        && !has_pending_ask_user(state)
        && !plan_review_selection_active(state)
        && input_context(state) == InputContext::Composer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{
            ModelPickerTab, PendingShellApproval,
            session::PendingAskUser,
            ui::{AskUserUiState, ShellApprovalEditMode, ShellApprovalUiState},
        },
        ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest},
        config::ReasoningEffort,
        features::planning::{PlanReviewState, PlanningStage},
    };

    fn state() -> AppState {
        AppState::new(
            SessionState::new(true, false, "gpt-5-mini", ReasoningEffort::Medium),
            UiState::default(),
        )
    }

    fn ask_user_request() -> AskUserRequest {
        AskUserRequest {
            title: Some("Clarify".into()),
            questions: vec![AskUserQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                answers: vec![AskUserAnswer {
                    id: "narrow".into(),
                    label: "Narrow".into(),
                }],
            }],
        }
    }

    #[test]
    fn input_context_prefers_overlays_before_picker_and_composer() {
        let mut state = state();
        state.ui.composer.composer.insert_str("/model");

        state
            .session
            .pending_write_approvals
            .push_back(crate::app::PendingWriteApproval {
                request_id: "write-1".into(),
                tool_name: "WriteFile".into(),
                arguments: "{}".into(),
                summary: "write".into(),
                target: Some("src/lib.rs".into()),
                source_label: None,
            });
        assert_eq!(input_context(&state), InputContext::WriteApproval);
        state.session.pending_write_approvals.clear();

        state
            .session
            .pending_shell_approvals
            .push_back(PendingShellApproval::new(
                "req-1".into(),
                crate::app::CommandRisk::Low,
                "safe".into(),
                "git status".into(),
                ".".into(),
                "inspect".into(),
                None,
            ));
        state.ui.pending_shell_approval = state
            .session
            .pending_shell_approvals
            .front()
            .map(ShellApprovalUiState::new);

        assert_eq!(
            input_context(&state),
            InputContext::ShellApproval {
                editing: false,
                can_move_up: false,
                can_move_down: false,
            }
        );

        state
            .ui
            .pending_shell_approval
            .as_mut()
            .expect("shell ui")
            .edit_mode = Some(ShellApprovalEditMode::Deny);
        assert_eq!(
            input_context(&state),
            InputContext::ShellApproval {
                editing: true,
                can_move_up: false,
                can_move_down: false,
            }
        );

        state.session.pending_shell_approvals.clear();
        state.ui.pending_shell_approval = None;
        state.session.pending_ask_user =
            Some(PendingAskUser::new("ask-1".into(), ask_user_request()));
        state.ui.pending_ask_user = state
            .session
            .pending_ask_user
            .as_ref()
            .map(AskUserUiState::new);
        assert_eq!(
            input_context(&state),
            InputContext::AskUser { editing: false }
        );

        state
            .ui
            .pending_ask_user
            .as_mut()
            .expect("ask user ui")
            .detail_editing = true;
        assert_eq!(
            input_context(&state),
            InputContext::AskUser { editing: true }
        );
    }

    #[test]
    fn input_context_prefers_review_then_picker_then_command_palette() {
        let mut state = state();
        state.ui.composer.composer.insert_str("/plan");

        state.session.planning.stage = PlanningStage::Review;
        state.session.planning.review = Some(PlanReviewState::Selection);
        assert_eq!(input_context(&state), InputContext::PlanReview);

        state.session.planning.stage = PlanningStage::Idle;
        state.session.planning.review = None;
        state.ui.picker = Some(SelectionPicker::Model {
            active_tab: ModelPickerTab::NormalAgent,
            normal_selected_model: "gpt-5.4-mini".into(),
            planning_selected_model: "gpt-5.4".into(),
            safety_selected_model: "gpt-5.4-mini".into(),
            memory_selected_model: "gpt-5.4-mini".into(),
        });
        assert_eq!(input_context(&state), InputContext::Picker);

        state.ui.picker = None;
        assert_eq!(input_context(&state), InputContext::CommandPalette);

        state.ui.composer.composer = crate::app::ui::new_composer_with_text("plain text");
        assert_eq!(input_context(&state), InputContext::Composer);
    }

    #[test]
    fn active_background_terminal_count_reads_session_summary() {
        let mut state = state();
        state.session.active_background_terminal_count = 1;

        assert_eq!(active_background_terminal_count(&state), 1);
    }
}
