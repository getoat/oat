use super::App;
use crate::app::{PendingAskUser, ui::AskUserUiState};

impl App {
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
