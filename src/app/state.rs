use super::{SessionState, UiState};

#[derive(Debug)]
pub struct AppState {
    pub session: SessionState,
    pub ui: UiState,
}

impl AppState {
    pub fn new(session: SessionState, ui: UiState) -> Self {
        Self { session, ui }
    }
}
