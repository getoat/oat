use super::*;

impl AppShell {
    pub(crate) fn move_ask_user_tab_left(&mut self) {
        self.move_ask_user_tab(-1);
    }

    pub(crate) fn move_ask_user_tab_right(&mut self) {
        self.move_ask_user_tab(1);
    }

    pub(crate) fn move_ask_user_answer_up(&mut self) {
        self.move_ask_user_answer(-1);
    }

    pub(crate) fn move_ask_user_answer_down(&mut self) {
        self.move_ask_user_answer(1);
    }

    pub(crate) fn toggle_ask_user_detail_editing(&mut self) {
        let Some(pending) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(session_pending) = self.session.pending_ask_user.as_ref() else {
            return;
        };
        if pending.active_tab >= session_pending.questions.len() {
            return;
        }

        pending.detail_editing = !pending.detail_editing;
    }

    pub(crate) fn apply_ask_user_input(&mut self, input: EditorInput) {
        let Some(question) = self.active_ask_user_detail_input_mut() else {
            return;
        };
        question.input(crate::app::ui::textarea_input(&input));
    }

    pub(crate) fn paste_into_ask_user_detail(&mut self, text: &str) {
        let Some(question) = self.active_ask_user_detail_input_mut() else {
            return;
        };
        question.insert_str(crate::app::ui::normalize_pasted_line_endings(text));
    }

    pub(crate) fn submit_ask_user_response(&mut self) -> Option<(String, AskUserResponse, String)> {
        let pending = self.session.pending_ask_user.as_ref()?;
        let ui = self.ui.pending_ask_user.as_ref()?;
        if ui.active_tab != pending.questions.len() {
            return None;
        }
        if !pending.is_complete(|index| ui.detail_text(index)) {
            self.push_error_message("Complete all AskUser questions before submitting.");
            return None;
        }

        let response = pending.response(|index| ui.detail_text(index));
        let request_id = pending.request_id.clone();
        let summary = response.transcript_summary();
        self.session.pending_ask_user = None;
        self.ui.pending_ask_user = None;
        self.push_user_message(summary.clone());
        Some((request_id, response, summary))
    }

    pub(crate) fn advance_ask_user(&mut self) -> Option<(String, AskUserResponse, String)> {
        let Some(pending) = self.session.pending_ask_user.as_ref() else {
            return None;
        };
        let Some(ui) = self.ui.pending_ask_user.as_ref() else {
            return None;
        };
        if ui.active_tab == pending.questions.len() {
            return self.submit_ask_user_response();
        }

        let question = &pending.questions[ui.active_tab];
        if !question.is_complete(ui.detail_text(ui.active_tab)) {
            self.push_error_message("`Something else` requires details before continuing.");
            if let Some(ui) = self.ui.pending_ask_user.as_mut() {
                ui.detail_editing = true;
            }
            return None;
        }

        if let Some(ui) = self.ui.pending_ask_user.as_mut() {
            ui.detail_editing = false;
            ui.active_tab += 1;
        }
        None
    }

    fn active_ask_user_detail_input_mut(&mut self) -> Option<&mut TextArea<'static>> {
        let ui = self.ui.pending_ask_user.as_mut()?;
        let session = self.session.pending_ask_user.as_ref()?;
        if !ui.detail_editing || ui.active_tab >= session.questions.len() {
            return None;
        }
        ui.detail_inputs.get_mut(ui.active_tab)
    }

    fn move_ask_user_tab(&mut self, direction: isize) {
        let Some(ui) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(session) = self.session.pending_ask_user.as_ref() else {
            return;
        };

        let tab_count = session.questions.len() + 1;
        ui.active_tab =
            (ui.active_tab as isize + direction).rem_euclid(tab_count as isize) as usize;
        if ui.active_tab >= session.questions.len() {
            ui.detail_editing = false;
        }
    }

    fn move_ask_user_answer(&mut self, direction: isize) {
        let Some(ui) = self.ui.pending_ask_user.as_mut() else {
            return;
        };
        let Some(pending) = self.session.pending_ask_user.as_mut() else {
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

    pub(crate) fn ask_user_session(&self) -> Option<&PendingAskUser> {
        self.session.pending_ask_user.as_ref()
    }

    pub(crate) fn pending_ask_user(&self) -> Option<&PendingAskUser> {
        self.session.pending_ask_user.as_ref()
    }

    pub(crate) fn ask_user_ui(&self) -> Option<&AskUserUiState> {
        self.ui.pending_ask_user.as_ref()
    }
}
