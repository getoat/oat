use super::*;

impl AppShell {
    pub(crate) fn set_composer_text_internal(&mut self, text: &str, reset_command_history: bool) {
        let mut composer = crate::app::ui::new_composer_with_text(text);
        composer.move_cursor(CursorMove::End);
        self.ui.composer.composer = composer;
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        if reset_command_history {
            self.session.command_history.reset_navigation();
        }
        self.sync_command_selection();
    }

    pub(crate) fn set_composer_wrap_width(&mut self, width: usize) {
        let width = width.max(1);
        if self.ui.composer.wrap_width != width {
            self.ui.composer.wrap_width = width;
            self.ui.invalidate_composer_layout();
            self.ui.composer.visual_column = None;
        }
    }

    pub(crate) fn composer_layout(&mut self) -> &ComposerLayout {
        self.ui.composer_layout()
    }

    pub(crate) fn composer_wrap_width(&self) -> usize {
        self.ui.composer.wrap_width
    }

    #[cfg(test)]
    pub(crate) fn set_composer_cursor(&mut self, row: u16, col: u16) {
        self.ui
            .composer
            .composer
            .move_cursor(CursorMove::Jump(row, col));
    }

    pub(crate) fn move_composer_cursor_up(&mut self) {
        let current_cursor = self.ui.composer.composer.cursor();
        let target = {
            let Some(cursor) = self.ui.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index == 0 {
                if cursor.visual_col > 0 {
                    Some((cursor.row.line_index, cursor.row.start_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.ui.composer.visual_column.unwrap_or(cursor.visual_col);
                self.ui
                    .composer_layout()
                    .target_cursor_for_row(cursor.row_index - 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.ui
                    .composer
                    .composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.ui.composer.visual_column = desired_col;
            }
            None => {
                self.ui.composer.visual_column = None;
            }
        }
    }

    pub(crate) fn move_composer_cursor_down(&mut self) {
        let current_cursor = self.ui.composer.composer.cursor();
        let target = {
            let Some(cursor) = self.ui.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index + 1 >= cursor.total_rows {
                if current_cursor.1 < cursor.row.end_col {
                    Some((cursor.row.line_index, cursor.row.end_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.ui.composer.visual_column.unwrap_or(cursor.visual_col);
                self.ui
                    .composer_layout()
                    .target_cursor_for_row(cursor.row_index + 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.ui
                    .composer
                    .composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.ui.composer.visual_column = desired_col;
            }
            None => {
                self.ui.composer.visual_column = None;
            }
        }
    }

    pub(crate) fn clear_composer(&mut self) {
        self.set_composer_text_internal("", true);
    }

    pub(crate) fn insert_composer_newline(&mut self) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui.composer.composer.insert_newline();
        self.sync_command_selection();
    }

    pub(crate) fn apply_composer_input(&mut self, input: EditorInput) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui
            .composer
            .composer
            .input(crate::app::ui::textarea_input(&input));
        self.sync_command_selection();
    }

    pub(crate) fn paste_into_composer(&mut self, text: &str) {
        self.session.command_history.reset_navigation();
        self.ui.invalidate_composer_layout();
        self.ui.composer.visual_column = None;
        self.ui
            .composer
            .composer
            .insert_str(crate::app::ui::normalize_pasted_line_endings(text));
        self.sync_command_selection();
    }

    pub(crate) fn record_submitted_input(&mut self, text: &str) {
        self.session.command_history.record(text);
    }

    pub(crate) fn should_recall_previous_input(&mut self) -> bool {
        let current_cursor = self.ui.composer.composer.cursor();
        self.ui
            .composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| cursor.row_index == 0 && cursor.visual_col == 0)
    }

    pub(crate) fn should_recall_next_input(&mut self) -> bool {
        let current_cursor = self.ui.composer.composer.cursor();
        self.ui
            .composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| {
                cursor.row_index + 1 >= cursor.total_rows && current_cursor.1 == cursor.row.end_col
            })
    }

    pub(crate) fn recall_previous_input(&mut self) -> bool {
        let current = self.ui.composer.composer.lines().join("\n");
        let Some(previous) = self.session.command_history.previous(&current) else {
            return false;
        };
        self.set_composer_text_internal(&previous, false);
        true
    }

    pub(crate) fn recall_next_input(&mut self) -> bool {
        let Some(next) = self.session.command_history.next() else {
            return false;
        };
        self.set_composer_text_internal(&next, false);
        true
    }
}
