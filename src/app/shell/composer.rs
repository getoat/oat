#[cfg(test)]
use ratatui_textarea::CursorMove;

use super::App;
use crate::composer::ComposerLayout;

impl App {
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
}
