use ratatui_textarea::TextArea;

use crate::{
    app::{AppState, EditorInput, PendingAskUser},
    ask_user::{AskUserRequest, AskUserResponse},
};

use super::transcript::{push_error_message, push_user_message};

pub(crate) fn begin_ask_user(state: &mut AppState, request_id: String, request: AskUserRequest) {
    state.session.pending_ask_user = Some(PendingAskUser::new(request_id, request));
    state.ui.pending_ask_user = state
        .session
        .pending_ask_user
        .as_ref()
        .map(crate::app::ui::AskUserUiState::new);
}

pub(crate) fn clear_pending_ask_user(state: &mut AppState) {
    state.session.pending_ask_user = None;
    state.ui.pending_ask_user = None;
}

pub(crate) fn move_ask_user_tab_left(state: &mut AppState) {
    move_ask_user_tab(state, -1);
}

pub(crate) fn move_ask_user_tab_right(state: &mut AppState) {
    move_ask_user_tab(state, 1);
}

pub(crate) fn move_ask_user_answer_up(state: &mut AppState) {
    move_ask_user_answer(state, -1);
}

pub(crate) fn move_ask_user_answer_down(state: &mut AppState) {
    move_ask_user_answer(state, 1);
}

pub(crate) fn toggle_ask_user_detail_editing(state: &mut AppState) {
    let Some(pending) = state.ui.pending_ask_user.as_mut() else {
        return;
    };
    let Some(session_pending) = state.session.pending_ask_user.as_ref() else {
        return;
    };
    if pending.active_tab >= session_pending.questions.len() {
        return;
    }

    pending.detail_editing = !pending.detail_editing;
}

pub(crate) fn apply_ask_user_input(state: &mut AppState, input: EditorInput) {
    let Some(question) = active_ask_user_detail_input_mut(state) else {
        return;
    };
    question.input(crate::app::ui::textarea_input(&input));
}

pub(crate) fn paste_into_ask_user_detail(state: &mut AppState, text: &str) {
    let Some(question) = active_ask_user_detail_input_mut(state) else {
        return;
    };
    question.insert_str(crate::app::ui::normalize_pasted_line_endings(text));
}

pub(crate) fn advance_ask_user(state: &mut AppState) -> Option<(String, AskUserResponse, String)> {
    let Some(pending) = state.session.pending_ask_user.as_ref() else {
        return None;
    };
    let Some(ui) = state.ui.pending_ask_user.as_ref() else {
        return None;
    };
    if ui.active_tab == pending.questions.len() {
        return submit_ask_user_response(state);
    }

    let question = &pending.questions[ui.active_tab];
    if !question.is_complete(ui.detail_text(ui.active_tab)) {
        push_error_message(
            state,
            "`Something else` requires details before continuing.",
        );
        if let Some(ui) = state.ui.pending_ask_user.as_mut() {
            ui.detail_editing = true;
        }
        return None;
    }

    if let Some(ui) = state.ui.pending_ask_user.as_mut() {
        ui.detail_editing = false;
        ui.active_tab += 1;
    }
    None
}

fn submit_ask_user_response(state: &mut AppState) -> Option<(String, AskUserResponse, String)> {
    let pending = state.session.pending_ask_user.as_ref()?;
    let ui = state.ui.pending_ask_user.as_ref()?;
    if ui.active_tab != pending.questions.len() {
        return None;
    }
    if !pending.is_complete(|index| ui.detail_text(index)) {
        push_error_message(state, "Complete all AskUser questions before submitting.");
        return None;
    }

    let response = pending.response(|index| ui.detail_text(index));
    let request_id = pending.request_id.clone();
    let summary = response.transcript_summary();
    state.session.pending_ask_user = None;
    state.ui.pending_ask_user = None;
    push_user_message(state, summary.clone());
    Some((request_id, response, summary))
}

fn active_ask_user_detail_input_mut(state: &mut AppState) -> Option<&mut TextArea<'static>> {
    let ui = state.ui.pending_ask_user.as_mut()?;
    let session = state.session.pending_ask_user.as_ref()?;
    if !ui.detail_editing || ui.active_tab >= session.questions.len() {
        return None;
    }
    ui.detail_inputs.get_mut(ui.active_tab)
}

fn move_ask_user_tab(state: &mut AppState, direction: isize) {
    let Some(ui) = state.ui.pending_ask_user.as_mut() else {
        return;
    };
    let Some(session) = state.session.pending_ask_user.as_ref() else {
        return;
    };

    let tab_count = session.questions.len() + 1;
    ui.active_tab = (ui.active_tab as isize + direction).rem_euclid(tab_count as isize) as usize;
    if ui.active_tab >= session.questions.len() {
        ui.detail_editing = false;
    }
}

fn move_ask_user_answer(state: &mut AppState, direction: isize) {
    let Some(ui) = state.ui.pending_ask_user.as_mut() else {
        return;
    };
    let Some(pending) = state.session.pending_ask_user.as_mut() else {
        return;
    };
    if ui.active_tab >= pending.questions.len() {
        return;
    }

    let question = &mut pending.questions[ui.active_tab];
    let len = question.answers.len();
    if len == 0 {
        return;
    }
    question.selected_index =
        (question.selected_index as isize + direction).rem_euclid(len as isize) as usize;
    if question.selected_answer().is_something_else {
        ui.detail_editing = true;
    }
}
