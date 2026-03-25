use ratatui_textarea::{CursorMove, TextArea};

use crate::app::session::{
    PendingShellApproval, ShellApprovalDecision, default_shell_approval_pattern,
};

use super::composer::new_text_area_with_text;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellApprovalEditMode {
    Pattern,
    Deny,
}

#[derive(Debug)]
pub struct ShellApprovalUiState {
    pub request_id: String,
    pub selected_index: usize,
    pub edit_mode: Option<ShellApprovalEditMode>,
    pub pattern_input: TextArea<'static>,
    pub deny_input: TextArea<'static>,
}

impl ShellApprovalUiState {
    pub fn new(pending: &PendingShellApproval) -> Self {
        let mut pattern_input =
            new_text_area_with_text(&default_shell_approval_pattern(&pending.command), "");
        pattern_input.move_cursor(CursorMove::End);
        Self {
            request_id: pending.request_id.clone(),
            selected_index: 0,
            edit_mode: None,
            pattern_input,
            deny_input: new_text_area_with_text("", ""),
        }
    }

    fn option_count(&self) -> usize {
        4
    }

    pub fn move_selection(&mut self, direction: isize) {
        self.selected_index = (self.selected_index as isize + direction)
            .rem_euclid(self.option_count() as isize) as usize;
        match self.selected_index {
            1 => {
                self.edit_mode = Some(ShellApprovalEditMode::Pattern);
                self.pattern_input.move_cursor(CursorMove::End);
            }
            3 => {
                if self.edit_mode == Some(ShellApprovalEditMode::Pattern) {
                    self.edit_mode = None;
                }
            }
            _ => self.edit_mode = None,
        }
    }

    fn selected_edit_mode(&self) -> Option<ShellApprovalEditMode> {
        match self.selected_index {
            1 => Some(ShellApprovalEditMode::Pattern),
            3 => Some(ShellApprovalEditMode::Deny),
            _ => None,
        }
    }

    pub fn begin_editing(&mut self) {
        self.edit_mode = self.selected_edit_mode();
        if self.edit_mode == Some(ShellApprovalEditMode::Pattern) {
            self.pattern_input.move_cursor(CursorMove::End);
        }
    }

    pub fn cancel_editing(&mut self) {
        self.edit_mode = None;
    }

    pub fn is_editing(&self) -> bool {
        self.edit_mode.is_some()
    }

    pub fn active_editor_mut(&mut self) -> Option<&mut TextArea<'static>> {
        match self.edit_mode {
            Some(ShellApprovalEditMode::Pattern) => Some(&mut self.pattern_input),
            Some(ShellApprovalEditMode::Deny) => Some(&mut self.deny_input),
            None => None,
        }
    }

    fn active_editor(&self) -> Option<&TextArea<'static>> {
        match self.edit_mode {
            Some(ShellApprovalEditMode::Pattern) => Some(&self.pattern_input),
            Some(ShellApprovalEditMode::Deny) => Some(&self.deny_input),
            None => None,
        }
    }

    pub fn editor_can_move_up(&self) -> bool {
        self.active_editor()
            .is_some_and(|editor| editor.cursor().0 > 0)
    }

    pub fn editor_can_move_down(&self) -> bool {
        self.active_editor().is_some_and(|editor| {
            let current_row = editor.cursor().0;
            current_row + 1 < editor.lines().len()
        })
    }

    pub fn selected_decision(&self) -> Option<ShellApprovalDecision> {
        match self.selected_index {
            0 => Some(ShellApprovalDecision::AllowOnce),
            1 => {
                let pattern = self.pattern_input.lines().join("\n").trim().to_string();
                (!pattern.is_empty()).then_some(ShellApprovalDecision::AllowPattern(pattern))
            }
            2 => Some(ShellApprovalDecision::AllowAllRisk),
            3 => {
                let note = self.deny_input.lines().join("\n").trim().to_string();
                Some(ShellApprovalDecision::Deny(
                    (!note.is_empty()).then_some(note),
                ))
            }
            _ => None,
        }
    }
}
