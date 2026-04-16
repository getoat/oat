use crate::app::session::PendingReplyActivity;
use crate::{
    app::{
        AppState, MainRequestSeed, PendingReply, PendingReplyKind, PendingReplyReplaySeed,
        PendingSideReply, SessionHistoryMessage, SessionState, SideChannelKind, UiState,
    },
    debug_log::log_debug,
    features::planning::PlanningStage,
    history_reduction::{compact_tool_traces, reduce_history},
    llm,
    todo::{TodoSnapshot, TodoStatus},
};

const TODO_HISTORY_PREFIX: &str = "[oat-todo] ";

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
    let history = sanitize_session_history_messages(history);
    let history = apply_current_todo_to_history(history, state.session.current_todo.as_ref());
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
    initialize_pending_reply_history(state);
}

pub(crate) fn initialize_pending_reply_history(state: &mut AppState) {
    let Some(visible_prompt) = state
        .session
        .active_main_request_seed
        .as_ref()
        .map(|seed| seed.visible_prompt.clone())
    else {
        return;
    };
    let Some(pending) = state.session.pending_reply.as_mut() else {
        return;
    };
    pending.initialize_canonical_turn(&visible_prompt);
    sync_pending_reply_history(state);
}

pub(crate) fn append_pending_reply_history_text(state: &mut AppState, delta: &str) {
    let Some(pending) = state.session.pending_reply.as_mut() else {
        return;
    };
    pending.append_canonical_assistant_text(delta);
    sync_pending_reply_history(state);
}

pub(crate) fn push_pending_reply_history_tool_call(
    state: &mut AppState,
    name: &str,
    arguments: &str,
) {
    let Some(pending) = state.session.pending_reply.as_mut() else {
        return;
    };
    pending.push_canonical_tool_call(name, arguments);
    sync_pending_reply_history(state);
}

pub(crate) fn push_pending_reply_history_tool_result(
    state: &mut AppState,
    name: &str,
    output: &str,
) {
    let Some(pending) = state.session.pending_reply.as_mut() else {
        return;
    };
    if pending.push_canonical_tool_result(name, output) {
        sync_pending_reply_history(state);
    }
}

pub(crate) fn sync_pending_reply_history(state: &mut AppState) {
    let Some(seed) = state.session.active_main_request_seed.as_ref() else {
        return;
    };
    let Some(pending) = state.session.pending_reply.as_ref() else {
        return;
    };

    let reduced_turn = reduce_history(
        pending.canonical_turn_messages(),
        state.session.history_mode,
        state.session.history_retained_steps,
        false,
    );

    match llm::history_from_rig(reduced_turn) {
        Ok(mut turn_history) => {
            let mut history = seed.history.clone();
            history.append(&mut turn_history);
            replace_session_history(state, history);
            state.session.last_history_model_name = Some(state.session.model_name.clone());
        }
        Err(error) => {
            log_debug(
                "session",
                format!("sync_pending_reply_history_failed error={error}"),
            );
        }
    }
}

pub(crate) fn persist_safe_pending_reply_history(state: &mut AppState) {
    let Some(seed) = state.session.active_main_request_seed.as_ref() else {
        return;
    };
    let Some(pending) = state.session.pending_reply.as_ref() else {
        return;
    };

    let safe_turn = pending.safe_canonical_turn_messages();
    let reduced_turn = reduce_history(
        &safe_turn,
        state.session.history_mode,
        state.session.history_retained_steps,
        false,
    );

    match llm::history_from_rig(reduced_turn) {
        Ok(mut turn_history) => {
            let mut history = seed.history.clone();
            history.append(&mut turn_history);
            replace_session_history(state, history);
            state.session.last_history_model_name = Some(state.session.model_name.clone());
        }
        Err(error) => {
            log_debug(
                "session",
                format!("persist_safe_pending_reply_history_failed error={error}"),
            );
        }
    }
}

pub(crate) fn reduce_session_history_messages(
    history: Vec<SessionHistoryMessage>,
    mode: crate::config::HistoryMode,
    retained_steps: usize,
    finalized: bool,
) -> Vec<SessionHistoryMessage> {
    let history = sanitize_session_history_messages(history);
    match llm::history_into_rig(history.clone()) {
        Ok(rig_history) => {
            let mut reduced = if matches!(mode, crate::config::HistoryMode::Full)
                || (matches!(mode, crate::config::HistoryMode::TurnSummary) && !finalized)
            {
                rig_history.clone()
            } else {
                reduce_history(&rig_history, mode, retained_steps, finalized)
            };
            if finalized {
                reduced = compact_tool_traces(&reduced);
            }
            if reduced == rig_history {
                history
            } else {
                llm::history_from_rig(reduced).unwrap_or(history)
            }
        }
        Err(_) => history,
    }
}

pub(crate) fn apply_current_todo_to_history(
    history: Vec<SessionHistoryMessage>,
    current_todo: Option<&TodoSnapshot>,
) -> Vec<SessionHistoryMessage> {
    let mut history = history
        .into_iter()
        .filter(|message| !is_synthetic_todo_summary(message))
        .collect::<Vec<_>>();

    if let Some(snapshot) = current_todo.filter(|snapshot| snapshot.has_list) {
        if let Ok(message) = SessionHistoryMessage::from_rig_message(
            rig::completion::Message::assistant(todo_summary_text(snapshot)),
        ) {
            history.push(message);
        }
    }

    history
}

pub(crate) fn sanitize_session_history_messages(
    history: Vec<SessionHistoryMessage>,
) -> Vec<SessionHistoryMessage> {
    match history
        .clone()
        .into_iter()
        .map(SessionHistoryMessage::into_rig_message)
        .collect::<anyhow::Result<Vec<_>>>()
    {
        Ok(rig_history) => {
            let sanitized_rig = llm::sanitize_rig_history(rig_history.clone());
            if sanitized_rig == rig_history {
                history
            } else {
                llm::history_from_rig(sanitized_rig).unwrap_or(history)
            }
        }
        Err(_) => history,
    }
}

fn is_synthetic_todo_summary(message: &SessionHistoryMessage) -> bool {
    message.payload.get("role").and_then(|value| value.as_str()) == Some("assistant")
        && message
            .payload
            .get("content")
            .and_then(|value| value.as_array())
            .is_some_and(|content| {
                content.len() == 1
                    && content[0].get("type").and_then(|value| value.as_str()) == Some("text")
                    && content[0]
                        .get("text")
                        .and_then(|value| value.as_str())
                        .is_some_and(|text| text.starts_with(TODO_HISTORY_PREFIX))
            })
}

fn todo_summary_text(snapshot: &TodoSnapshot) -> String {
    let tasks = if snapshot.tasks.is_empty() {
        "empty todo list".to_string()
    } else {
        snapshot
            .tasks
            .iter()
            .map(|task| format!("[{}] {}", todo_status_label(task.status), task.description))
            .collect::<Vec<_>>()
            .join("; ")
    };
    format!("{TODO_HISTORY_PREFIX}{tasks}")
}

fn todo_status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Todo => "todo",
        TodoStatus::InProgress => "in progress",
        TodoStatus::Done => "done",
    }
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
    let history_mode = state.session.history_mode;
    let history_retained_steps = state.session.history_retained_steps;
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
        state.session.full_system_access,
        state.session.initial_mode,
        state.session.initial_approval_mode,
    );
    state.session.workspace_root = workspace_root;
    state.session.safety_model_name = safety_model_name;
    state.session.safety_reasoning = safety_reasoning;
    state.session.memory_model_name = memory_model_name;
    state.session.memory_reasoning = memory_reasoning;
    state.session.history_mode = history_mode;
    state.session.history_retained_steps = history_retained_steps;
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
    persist_safe_pending_reply_history(state);
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
