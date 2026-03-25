use ratatui_textarea::TextArea;

use crate::app::session::PendingAskUser;

use super::composer::new_text_area_with_text;

#[derive(Debug)]
pub struct AskUserUiState {
    pub active_tab: usize,
    pub detail_editing: bool,
    pub detail_inputs: Vec<TextArea<'static>>,
}

impl AskUserUiState {
    pub fn new(pending: &PendingAskUser) -> Self {
        Self {
            active_tab: 0,
            detail_editing: false,
            detail_inputs: pending
                .questions
                .iter()
                .map(|_| new_text_area_with_text("", ""))
                .collect(),
        }
    }

    pub fn detail_text(&self, index: usize) -> String {
        self.detail_inputs
            .get(index)
            .map(|input| input.lines().join("\n").trim().to_string())
            .unwrap_or_default()
    }
}
