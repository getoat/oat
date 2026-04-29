use crate::app::session::pending_stream_text_is_visible;
use crate::{
    app::{
        ActivityDisplayState, AppState, BackgroundTerminalStatusEntry, ChatMessage, HostedToolKind,
        HostedToolStatusEntry, MessageStyle, Speaker, SubagentStatusEntry, SubagentStatusKind,
        ToolCall, ToolResultEntry, TranscriptEntry,
    },
    todo::TodoSnapshot,
    tools::mutation_preview,
};

pub(crate) fn push_agent_message(state: &mut AppState, text: impl Into<String>) {
    push_message(state, Speaker::Agent, text, MessageStyle::Plain, None);
}

pub(crate) fn push_user_message(state: &mut AppState, text: impl Into<String>) {
    push_message(state, Speaker::User, text, MessageStyle::Plain, None);
}

pub(crate) fn push_tagged_agent_message(
    state: &mut AppState,
    tag: impl Into<String>,
    text: impl Into<String>,
) {
    push_message(
        state,
        Speaker::Agent,
        text,
        MessageStyle::Plain,
        Some(tag.into()),
    );
}

pub(crate) fn push_tagged_user_message(
    state: &mut AppState,
    tag: impl Into<String>,
    text: impl Into<String>,
) {
    push_message(
        state,
        Speaker::User,
        text,
        MessageStyle::Plain,
        Some(tag.into()),
    );
}

pub(crate) fn push_tagged_error_message(
    state: &mut AppState,
    tag: impl Into<String>,
    text: impl Into<String>,
) {
    push_message(
        state,
        Speaker::Agent,
        text,
        MessageStyle::Error,
        Some(tag.into()),
    );
}

pub(crate) fn push_error_message(state: &mut AppState, text: impl Into<String>) {
    push_message(state, Speaker::Agent, text, MessageStyle::Error, None);
}

pub(crate) fn push_agent_commentary(state: &mut AppState, text: impl Into<String>) {
    let text = text.into();
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.reset_active_stream_segment();
        pending.commentary_messages.push(text.clone());
        pending.has_visible_content = true;
    }
    push_message(state, Speaker::Agent, text, MessageStyle::Commentary, None);
}

pub(crate) fn push_tool_call(state: &mut AppState, name: String, parameter: String) {
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.reset_active_stream_segment();
        pending.has_visible_content = true;
    }
    state
        .session
        .entries
        .push(TranscriptEntry::ToolCall(ToolCall {
            preview: mutation_preview(&name, &parameter, &state.session.workspace_root),
            name,
            parameter,
        }));
    bump_transcript_revision(state);
}

pub(crate) fn push_tool_result(state: &mut AppState, name: String, output: String) {
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.reset_active_stream_segment();
        if state.session.show_tool_output {
            pending.has_visible_content = true;
        }
    }
    state
        .session
        .entries
        .push(TranscriptEntry::ToolResult(ToolResultEntry {
            name,
            output,
        }));
    bump_transcript_revision(state);
}

pub(crate) fn push_todo_snapshot(state: &mut AppState, snapshot: TodoSnapshot) {
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.reset_active_stream_segment();
        pending.has_visible_content = true;
    }
    state
        .session
        .entries
        .push(TranscriptEntry::TodoSnapshot(snapshot));
    bump_transcript_revision(state);
}

pub(crate) fn upsert_hosted_tool_status(
    state: &mut AppState,
    id: String,
    kind: HostedToolKind,
    display_state: ActivityDisplayState,
    detail: String,
) {
    if let Some(pending) = state.session.pending_reply.as_mut() {
        pending.reset_active_stream_segment();
        pending.has_visible_content = true;
    }

    if let Some(TranscriptEntry::HostedToolStatus(entry)) =
        state.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::HostedToolStatus(status) if status.id == id),
        )
    {
        entry.kind = kind;
        entry.state = display_state;
        entry.detail = detail;
        bump_transcript_revision(state);
        return;
    }

    state
        .session
        .entries
        .push(TranscriptEntry::HostedToolStatus(HostedToolStatusEntry {
            id,
            kind,
            state: display_state,
            detail,
        }));
    bump_transcript_revision(state);
}

pub(crate) fn upsert_subagent_status(
    state: &mut AppState,
    id: String,
    kind: SubagentStatusKind,
    display_label: String,
    display_state: ActivityDisplayState,
    status_text: String,
) {
    if let Some(TranscriptEntry::SubagentStatus(entry)) =
        state.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        )
    {
        entry.kind = kind;
        entry.display_label = display_label;
        entry.state = display_state;
        entry.status_text = status_text;
        bump_transcript_revision(state);
        return;
    }

    state
        .session
        .entries
        .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
            id,
            kind,
            display_label,
            state: display_state,
            status_text,
            latest_tool_name: None,
        }));
    bump_transcript_revision(state);
}

pub(crate) fn set_subagent_latest_tool(state: &mut AppState, id: String, latest_tool_name: String) {
    if let Some(TranscriptEntry::SubagentStatus(entry)) =
        state.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        )
    {
        entry.latest_tool_name = Some(latest_tool_name);
        bump_transcript_revision(state);
        return;
    }

    state
        .session
        .entries
        .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
            display_label: id.clone(),
            id,
            kind: SubagentStatusKind::Subagent,
            state: ActivityDisplayState::Running,
            status_text: "running".into(),
            latest_tool_name: Some(latest_tool_name),
        }));
    bump_transcript_revision(state);
}

pub(crate) fn upsert_background_terminal_status(
    state: &mut AppState,
    id: String,
    display_label: String,
    display_state: ActivityDisplayState,
    status_text: String,
    detail_text: Option<String>,
) {
    if let Some(TranscriptEntry::BackgroundTerminalStatus(entry)) =
        state.session.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::BackgroundTerminalStatus(status) if status.id == id),
        )
        {
        entry.display_label = display_label;
        entry.state = display_state;
        entry.status_text = status_text;
        entry.detail_text = detail_text;
        refresh_active_background_terminal_count(state);
        bump_transcript_revision(state);
        return;
    }

    state
        .session
        .entries
        .push(TranscriptEntry::BackgroundTerminalStatus(
            BackgroundTerminalStatusEntry {
                id,
                display_label,
                state: display_state,
                status_text,
                detail_text,
            },
        ));
    refresh_active_background_terminal_count(state);
    bump_transcript_revision(state);
}

fn refresh_active_background_terminal_count(state: &mut AppState) {
    state.session.active_background_terminal_count = state
        .session
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry,
                TranscriptEntry::BackgroundTerminalStatus(status)
                    if status.state == ActivityDisplayState::Running
            )
        })
        .count();
}

pub(crate) fn append_pending_stream_message(
    state: &mut AppState,
    delta: &str,
    style: MessageStyle,
) {
    if delta.is_empty() || state.session.pending_reply.is_none() || style == MessageStyle::Error {
        return;
    }

    let existing_index = {
        let pending = state
            .session
            .pending_reply
            .as_mut()
            .expect("pending reply checked above");
        let crossed_style_boundary = match style {
            MessageStyle::Plain => pending.reasoning_entry_index.is_some(),
            MessageStyle::Commentary => true,
            MessageStyle::Thinking => pending.text_entry_index.is_some(),
            MessageStyle::Error => false,
        };
        if crossed_style_boundary {
            pending.reset_active_stream_segment();
        }
        match style {
            MessageStyle::Plain => pending.text_entry_index,
            MessageStyle::Commentary => None,
            MessageStyle::Thinking => pending.reasoning_entry_index,
            MessageStyle::Error => None,
        }
    };

    let Some(existing_index) = existing_index else {
        let mut pending_text = delta.to_string();
        {
            let pending = state
                .session
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            match style {
                MessageStyle::Plain => {
                    pending.plain_text.push_str(delta);
                    pending.staged_plain_text.push_str(delta);
                    if !pending_stream_text_is_visible(style, &pending.staged_plain_text) {
                        return;
                    }
                    pending_text = std::mem::take(&mut pending.staged_plain_text);
                }
                MessageStyle::Thinking => {
                    pending.reasoning_text.push_str(delta);
                    pending.staged_reasoning_text.push_str(delta);
                    if !pending_stream_text_is_visible(style, &pending.staged_reasoning_text) {
                        return;
                    }
                    pending_text = std::mem::take(&mut pending.staged_reasoning_text);
                }
                MessageStyle::Commentary => {
                    if !pending_stream_text_is_visible(style, delta) {
                        return;
                    }
                }
                MessageStyle::Error => return,
            }
        }

        push_message(state, Speaker::Agent, pending_text, style, None);
        let index = state.session.entries.len() - 1;
        let pending = state
            .session
            .pending_reply
            .as_mut()
            .expect("pending reply checked above");
        pending.has_visible_content = true;
        match style {
            MessageStyle::Plain => pending.text_entry_index = Some(index),
            MessageStyle::Commentary => {}
            MessageStyle::Thinking => pending.reasoning_entry_index = Some(index),
            MessageStyle::Error => {}
        }
        return;
    };

    if let Some(TranscriptEntry::Message(message)) = state.session.entries.get_mut(existing_index) {
        message.text.push_str(delta);
        if style == MessageStyle::Plain
            && let Some(pending) = state.session.pending_reply.as_mut()
        {
            pending.plain_text.push_str(delta);
        }
        if existing_index + 1 == state.session.entries.len() {
            bump_transcript_tail_revision(state);
        } else {
            bump_transcript_revision(state);
        }
    }
}

pub(crate) fn discard_pending_text_entry(state: &mut AppState) {
    let Some(index) = state
        .session
        .pending_reply
        .as_ref()
        .and_then(|pending| pending.text_entry_index)
    else {
        return;
    };

    if index < state.session.entries.len() {
        state.session.entries.remove(index);
        bump_transcript_revision(state);
    }
}

fn push_message(
    state: &mut AppState,
    speaker: Speaker,
    text: impl Into<String>,
    style: MessageStyle,
    tag: Option<String>,
) {
    state
        .session
        .entries
        .push(TranscriptEntry::Message(ChatMessage {
            speaker,
            text: text.into(),
            style,
            tag,
        }));
    bump_transcript_revision(state);
}

pub(crate) fn bump_transcript_revision(state: &mut AppState) {
    state.session.transcript_revision = state.session.transcript_revision.wrapping_add(1);
    state.session.transcript_structure_revision =
        state.session.transcript_structure_revision.wrapping_add(1);
    state.ui.history_render_cache = None;
}

pub(crate) fn bump_transcript_tail_revision(state: &mut AppState) {
    state.session.transcript_revision = state.session.transcript_revision.wrapping_add(1);
    state.session.transcript_tail_revision = state.session.transcript_tail_revision.wrapping_add(1);
}
