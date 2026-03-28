use crate::{
    app::{
        AppState, PendingReply, PendingReplyKind, PendingReplyReplaySeed, SessionHistoryMessage,
        SessionState, UiState,
    },
    features::planning::PlanningStage,
};

pub(crate) fn active_reply_id(state: &AppState) -> Option<u64> {
    state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.id)
}

pub(crate) fn active_reply_kind(state: &AppState) -> Option<PendingReplyKind> {
    state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.kind)
}

pub(crate) fn pending_reply_replay_seed(state: &AppState) -> Option<PendingReplyReplaySeed> {
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

pub(crate) fn next_reply_id(state: &mut AppState) -> u64 {
    state.session.next_reply_id()
}

pub(crate) fn ensure_pending_reply(state: &mut AppState, kind: PendingReplyKind) -> u64 {
    state.session.ensure_pending_reply(kind)
}

pub(crate) fn set_pending_reply(state: &mut AppState, reply_id: u64, kind: PendingReplyKind) {
    state.session.pending_reply = Some(PendingReply::new(reply_id, kind));
}

pub(crate) fn begin_session_title_request(state: &mut AppState, reply_id: u64) {
    state.session.pending_session_title_reply_id = Some(reply_id);
}

pub(crate) fn store_session_title(state: &mut AppState, reply_id: u64, title: String) -> bool {
    if state.session.pending_session_title_reply_id != Some(reply_id) {
        return false;
    }

    state.session.pending_session_title_reply_id = None;
    let title = title.trim();
    if !title.is_empty() {
        state.session.session_title = Some(title.to_string());
    }
    true
}

pub(crate) fn clear_pending_reply_only(state: &mut AppState) {
    state.session.pending_reply = None;
}

pub(crate) fn replace_session_history(state: &mut AppState, history: Vec<SessionHistoryMessage>) {
    state.session.replace_session_history(history);
}

pub(crate) fn set_last_history_model_name(
    state: &mut AppState,
    model_name: Option<impl Into<String>>,
) {
    state.session.last_history_model_name = model_name.map(Into::into);
}

pub(crate) fn reset_session(state: &mut AppState) {
    let model_name = state.session.model_name.clone();
    let reasoning = state.session.reasoning;
    let planning_agents = state.session.planning_agents.clone();
    let workspace_root = state.session.workspace_root.clone();
    let safety_model_name = state.session.safety_model_name.clone();
    let safety_reasoning = state.session.safety_reasoning;
    let session_stats = state.session.session_stats;
    let next_reply_id = state.session.next_reply_id;
    let mut command_history = std::mem::take(&mut state.ui.command_history);
    command_history.reset_navigation();

    state.session = SessionState::with_startup(
        state.session.show_thinking,
        state.session.show_tool_output,
        model_name,
        reasoning,
        planning_agents,
        state.session.initial_mode,
        state.session.initial_approval_mode,
    );
    state.session.workspace_root = workspace_root;
    state.session.safety_model_name = safety_model_name;
    state.session.safety_reasoning = safety_reasoning;
    state.session.session_stats = session_stats;
    state.session.next_reply_id = next_reply_id;
    state.ui = UiState::default();
    state.ui.command_history = command_history;
}

pub(crate) fn set_should_quit(state: &mut AppState) {
    state.session.should_quit = true;
}

pub(crate) fn cancel_pending_reply(state: &mut AppState) {
    state.session.pending_reply = None;
    state.session.pending_write_approvals.clear();
    state.session.pending_shell_approvals.clear();
    state.session.pending_ask_user = None;
    state.ui.pending_shell_approval = None;
    state.ui.pending_ask_user = None;
    if state.session.planning.stage == PlanningStage::RunningFanout {
        crate::features::planning::start_conversation(&mut state.session.planning);
    }
}

pub(crate) fn restore_command_history(state: &mut AppState, entries: Vec<String>, limit: usize) {
    state.ui.command_history.restore(entries, limit);
}

pub(crate) fn take_command_history_to_persist(state: &mut AppState) -> Option<Vec<String>> {
    state.ui.command_history.take_dirty_entries()
}
