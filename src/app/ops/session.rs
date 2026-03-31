use crate::app::session::PendingReplyActivity;
use crate::{
    app::{
        AppState, MainRequestSeed, PendingReply, PendingReplyKind, PendingReplyReplaySeed,
        PendingSideReply, SessionHistoryMessage, SessionState, SideChannelKind, UiState,
    },
    debug_log::log_debug,
    features::planning::PlanningStage,
    todo::TodoSnapshot,
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
    log_debug(
        "session",
        format!("set_pending_reply id={reply_id} kind={kind:?}"),
    );
    state.session.pending_reply = Some(PendingReply::new(reply_id, kind));
}

pub(crate) fn set_pending_reply_activity(state: &mut AppState, activity: PendingReplyActivity) {
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.activity = activity;
    }
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
    let previous = state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.id);
    log_debug(
        "session",
        format!("clear_pending_reply_only previous={previous:?}"),
    );
    state.session.pending_reply = None;
}

pub(crate) fn enqueue_queued_message(state: &mut AppState, message: String) {
    state.session.queued_messages.push_back(message);
}

pub(crate) fn dequeue_queued_message(state: &mut AppState) -> Option<String> {
    state.session.queued_messages.pop_front()
}

pub(crate) fn replace_session_history(state: &mut AppState, history: Vec<SessionHistoryMessage>) {
    state.session.replace_session_history(history);
}

pub(crate) fn set_active_main_request_seed(
    state: &mut AppState,
    history: Vec<SessionHistoryMessage>,
    visible_prompt: String,
    model_prompt: String,
    history_model_name: Option<String>,
    transcript_len_before: usize,
) {
    state.session.active_main_request_seed = Some(MainRequestSeed {
        history,
        visible_prompt,
        model_prompt,
        history_model_name,
        transcript_len_before,
    });
}

pub(crate) fn clear_active_main_request_seed(state: &mut AppState) {
    state.session.active_main_request_seed = None;
}

pub(crate) fn canonicalize_main_turn_history(
    history: Vec<SessionHistoryMessage>,
    seed: Option<&MainRequestSeed>,
) -> Vec<SessionHistoryMessage> {
    let Some(seed) = seed else {
        return history;
    };
    if seed.visible_prompt == seed.model_prompt {
        return history;
    }
    if history.len() <= seed.history.len() || !history.starts_with(&seed.history) {
        return history;
    }

    let prompt_index = seed.history.len();
    if history
        .get(prompt_index)
        .is_some_and(|message| message == &SessionHistoryMessage::user(seed.model_prompt.clone()))
    {
        let mut canonical = history;
        canonical[prompt_index] = SessionHistoryMessage::user(seed.visible_prompt.clone());
        canonical
    } else {
        history
    }
}

pub(crate) fn begin_side_reply(
    state: &mut AppState,
    reply_id: u64,
    kind: SideChannelKind,
) -> PendingSideReply {
    let label_id = state.session.next_side_channel_label_id;
    state.session.next_side_channel_label_id =
        state.session.next_side_channel_label_id.wrapping_add(1);
    let reply = PendingSideReply {
        kind,
        label: format!("{} {label_id}", kind.label_prefix()),
    };
    state
        .session
        .pending_side_replies
        .insert(reply_id, reply.clone());
    reply
}

pub(crate) fn finish_side_reply(state: &mut AppState, reply_id: u64) -> Option<PendingSideReply> {
    state.session.pending_side_replies.remove(&reply_id)
}

pub(crate) fn set_last_history_model_name(
    state: &mut AppState,
    model_name: Option<impl Into<String>>,
) {
    state.session.last_history_model_name = model_name.map(Into::into);
}

pub(crate) fn set_current_todo(state: &mut AppState, todo: Option<TodoSnapshot>) {
    state.session.current_todo = todo;
}

pub(crate) fn reset_session(state: &mut AppState) {
    let model_name = state.session.model_name.clone();
    let reasoning = state.session.reasoning;
    let planning_agents = state.session.planning_agents.clone();
    let workspace_root = state.session.workspace_root.clone();
    let safety_model_name = state.session.safety_model_name.clone();
    let safety_reasoning = state.session.safety_reasoning;
    let memory_model_name = state.session.memory_model_name.clone();
    let memory_reasoning = state.session.memory_reasoning;
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
    state.session.memory_model_name = memory_model_name;
    state.session.memory_reasoning = memory_reasoning;
    state.session.session_stats = session_stats;
    state.session.next_reply_id = next_reply_id;
    state.ui = UiState::default();
    state.ui.command_history = command_history;
}

pub(crate) fn set_should_quit(state: &mut AppState) {
    state.session.should_quit = true;
}

pub(crate) fn cancel_pending_reply(state: &mut AppState) {
    let previous = state
        .session
        .pending_reply
        .as_ref()
        .map(|pending| pending.id);
    log_debug(
        "session",
        format!("cancel_pending_reply previous={previous:?}"),
    );
    state.session.pending_reply = None;
    state.session.pending_write_approvals.clear();
    state.session.pending_shell_approvals.clear();
    state.session.pending_ask_user = None;
    clear_active_main_request_seed(state);
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
