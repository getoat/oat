use crate::{
    config::{ReasoningEffort, ReasoningSetting},
    model_registry,
};

use super::{
    MessageStyle, PendingReplyKind, SessionState, TranscriptEntry, startup_banner_message,
};

pub fn shows_startup_banner(session: &SessionState) -> bool {
    session.session_history.is_empty()
        && session.entries.len() == 1
        && matches!(
            session.entries.first(),
            Some(TranscriptEntry::Message(crate::app::session::ChatMessage {
                speaker: crate::app::session::Speaker::Agent,
                style: MessageStyle::Plain,
                text,
            })) if text == &startup_banner_message(&session.model_name, session.initial_mode)
        )
}

#[cfg(test)]
pub fn has_visible_pending_content(session: &SessionState) -> bool {
    session
        .pending_reply
        .as_ref()
        .is_some_and(|pending| pending.has_visible_content)
}

pub fn should_show_history_busy_indicator(session: &SessionState) -> bool {
    session.pending_reply.as_ref().is_some_and(|pending| {
        pending.text_entry_index.is_none() && pending.reasoning_entry_index.is_none()
    })
}

pub fn history_pending_status_label(session: &SessionState) -> &'static str {
    if !session.pending_write_approvals.is_empty() || !session.pending_shell_approvals.is_empty() {
        "Waiting"
    } else if session
        .pending_reply
        .as_ref()
        .is_some_and(|pending| pending.kind == PendingReplyKind::Compacting)
    {
        "Compacting context..."
    } else {
        "thinking"
    }
}

pub fn current_model_info(session: &SessionState) -> Option<&'static model_registry::ModelInfo> {
    model_registry::find_model(&session.model_name)
}

pub fn supported_reasoning_settings(session: &SessionState) -> Vec<ReasoningSetting> {
    model_registry::reasoning_settings_for_model(&session.model_name)
        .map(|settings| settings.to_vec())
        .unwrap_or_else(|| {
            vec![
                ReasoningSetting::Gpt(ReasoningEffort::Minimal),
                ReasoningSetting::Gpt(ReasoningEffort::Low),
                ReasoningSetting::Gpt(ReasoningEffort::Medium),
                ReasoningSetting::Gpt(ReasoningEffort::High),
                ReasoningSetting::Gpt(ReasoningEffort::XHigh),
            ]
        })
}

pub fn next_request_context_percent(session: &SessionState) -> u64 {
    let Some(model) = current_model_info(session) else {
        return 0;
    };
    if model.context_length == 0 {
        return 0;
    }

    session.estimated_session_history_tokens * 100 / model.context_length as u64
}
