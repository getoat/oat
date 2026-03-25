use ratatui_textarea::TextArea;

use crate::{
    config::ReasoningEffort,
    features::planning::{PlanReviewState, PlanningAgentConfig, PlanningStage},
    model_registry,
    stats::StatsTotals,
};

use super::{
    AccessMode, AppState, ApprovalMode, PendingAskUser, PendingReplyKind, PendingReplyReplaySeed,
    PendingShellApproval, PendingWriteApproval, SelectionPicker, SessionHistoryMessage,
    SessionState, SlashCommand, TranscriptEntry, UiState,
    session::{
        current_model_info, history_pending_status_label, next_request_context_percent,
        should_show_history_busy_indicator, shows_startup_banner, supported_reasoning_levels,
    },
    ui::{AskUserUiState, ShellApprovalUiState, picker_height, split_command_query},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputTarget {
    Composer,
    CommandPalette,
    Picker,
    ShellApprovalSelection,
    ShellApprovalEditor,
    AskUserSelection,
    AskUserEditor,
    PlanReviewSelection,
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

pub fn safety_reasoning_effort(state: &AppState) -> ReasoningEffort {
    state.session.safety_reasoning_effort
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

pub fn has_pending_write_approval(state: &AppState) -> bool {
    !state.session.pending_write_approvals.is_empty()
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

pub fn shell_approval_editing(state: &AppState) -> bool {
    shell_approval_ui(state).is_some_and(ShellApprovalUiState::is_editing)
}

pub fn shell_approval_editor_can_move_up(state: &AppState) -> bool {
    shell_approval_ui(state).is_some_and(ShellApprovalUiState::editor_can_move_up)
}

pub fn shell_approval_editor_can_move_down(state: &AppState) -> bool {
    shell_approval_ui(state).is_some_and(ShellApprovalUiState::editor_can_move_down)
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

pub fn plan_review_feedback_active(state: &AppState) -> bool {
    state.session.planning.stage == PlanningStage::Review
        && state.session.planning.review == Some(PlanReviewState::Feedback)
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

pub fn shows_startup_banner_state(state: &AppState) -> bool {
    shows_startup_banner(&state.session)
}

pub fn has_pending_reply(state: &AppState) -> bool {
    state.session.pending_reply.is_some()
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

pub fn reasoning_effort(state: &AppState) -> ReasoningEffort {
    state.session.reasoning_effort
}

pub fn last_history_model_name(state: &AppState) -> Option<&str> {
    state.session.last_history_model_name.as_deref()
}

pub fn planning_agents(state: &AppState) -> &[PlanningAgentConfig] {
    &state.session.planning_agents
}

pub fn current_model_info_state(state: &AppState) -> Option<&'static model_registry::ModelInfo> {
    current_model_info(&state.session)
}

pub fn session_stats(state: &AppState) -> StatsTotals {
    state.session.session_stats
}

pub fn next_request_context_percent_state(state: &AppState) -> u64 {
    next_request_context_percent(&state.session)
}

pub fn tick_count(state: &AppState) -> usize {
    state.session.tick_count
}

pub fn selection_picker(state: &AppState) -> Option<&SelectionPicker> {
    state.ui.picker.as_ref()
}

pub fn selection_picker_visible(state: &AppState) -> bool {
    state.ui.picker.is_some()
}

pub fn history_is_pinned(state: &AppState) -> bool {
    state.ui.history.is_pinned()
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

pub fn supported_reasoning_levels_state(state: &AppState) -> Vec<ReasoningEffort> {
    supported_reasoning_levels(&state.session)
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

pub fn command_palette_height(state: &AppState) -> u16 {
    if !command_palette_visible(state) {
        return 0;
    }

    let line_count = filtered_commands(state).len().clamp(1, 4) as u16;
    line_count + 2
}

pub fn overlay_height(state: &AppState) -> u16 {
    if let Some(picker) = selection_picker(state) {
        return picker_height(picker);
    }

    command_palette_height(state)
}

pub fn active_input_target(state: &AppState) -> InputTarget {
    active_input_target_parts(&state.session, &state.ui)
}

pub(crate) fn active_input_target_parts(session: &SessionState, ui: &UiState) -> InputTarget {
    if !session.pending_shell_approvals.is_empty() {
        if ui
            .pending_shell_approval
            .as_ref()
            .is_some_and(ShellApprovalUiState::is_editing)
        {
            InputTarget::ShellApprovalEditor
        } else {
            InputTarget::ShellApprovalSelection
        }
    } else if session.pending_ask_user.is_some() {
        if ui
            .pending_ask_user
            .as_ref()
            .is_some_and(|pending| pending.detail_editing)
        {
            InputTarget::AskUserEditor
        } else {
            InputTarget::AskUserSelection
        }
    } else if session.planning.stage == PlanningStage::Review
        && session.planning.review == Some(PlanReviewState::Selection)
    {
        InputTarget::PlanReviewSelection
    } else if ui.picker.is_some() {
        InputTarget::Picker
    } else {
        let [line] = ui.composer.composer.lines() else {
            return InputTarget::Composer;
        };
        if ui.picker.is_none() && line.starts_with('/') {
            InputTarget::CommandPalette
        } else {
            InputTarget::Composer
        }
    }
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
    fn active_input_target_prefers_overlays_before_picker_and_composer() {
        let mut state = state();
        state.ui.composer.composer.insert_str("/model");

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
            active_input_target(&state),
            InputTarget::ShellApprovalSelection
        );

        state
            .ui
            .pending_shell_approval
            .as_mut()
            .expect("shell ui")
            .edit_mode = Some(ShellApprovalEditMode::Deny);
        assert_eq!(
            active_input_target(&state),
            InputTarget::ShellApprovalEditor
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
        assert_eq!(active_input_target(&state), InputTarget::AskUserSelection);

        state
            .ui
            .pending_ask_user
            .as_mut()
            .expect("ask user ui")
            .detail_editing = true;
        assert_eq!(active_input_target(&state), InputTarget::AskUserEditor);
    }

    #[test]
    fn active_input_target_prefers_review_then_picker_then_command_palette() {
        let mut state = state();
        state.ui.composer.composer.insert_str("/plan");

        state.session.planning.stage = PlanningStage::Review;
        state.session.planning.review = Some(PlanReviewState::Selection);
        assert_eq!(
            active_input_target(&state),
            InputTarget::PlanReviewSelection
        );

        state.session.planning.stage = PlanningStage::Idle;
        state.session.planning.review = None;
        state.ui.picker = Some(SelectionPicker::Model {
            active_tab: ModelPickerTab::NormalAgent,
            normal_selected_index: 0,
            planning_selected_index: 0,
            safety_selected_index: 0,
        });
        assert_eq!(active_input_target(&state), InputTarget::Picker);

        state.ui.picker = None;
        assert_eq!(active_input_target(&state), InputTarget::CommandPalette);

        state.ui.composer.composer = crate::app::ui::new_composer_with_text("plain text");
        assert_eq!(active_input_target(&state), InputTarget::Composer);
    }
}
