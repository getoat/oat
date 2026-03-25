use ratatui::{layout::Rect, style::Color, text::Line};

use super::App;
use crate::app::ui::HistoryRenderCache;

impl App {
    pub(crate) fn sync_history_viewport(
        &mut self,
        total_lines: usize,
        viewport_rows: usize,
    ) -> usize {
        self.ui.history.sync_viewport(total_lines, viewport_rows)
    }

    pub(crate) fn history_total_lines(&self) -> usize {
        self.ui.history.total_lines()
    }

    pub(crate) fn history_viewport_rows(&self) -> usize {
        self.ui.history.viewport_rows()
    }

    pub(crate) fn history_scroll_position(&self) -> usize {
        self.ui.history.scroll_position()
    }

    pub(crate) fn update_history_snapshot(&mut self, area: Rect, lines: Vec<String>) {
        self.ui.history.update_snapshot(area, lines);
    }

    #[cfg(test)]
    pub(crate) fn update_history_snapshot_for_test(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        lines: Vec<String>,
    ) {
        self.update_history_snapshot(
            Rect {
                x,
                y,
                width,
                height,
            },
            lines,
        );
    }

    pub(crate) fn history_cache_allowed(&self) -> bool {
        !self.shows_startup_banner() && !self.should_show_history_busy_indicator()
    }

    pub(crate) fn cached_history_lines(
        &self,
        width: usize,
        accent: Color,
    ) -> Option<&[Line<'static>]> {
        let cache = self.ui.history_render_cache.as_ref()?;
        (cache.width == width
            && cache.accent == accent
            && cache.transcript_revision == self.session.transcript_revision)
            .then_some(cache.lines.as_slice())
    }

    pub(crate) fn store_history_render_cache(
        &mut self,
        width: usize,
        accent: Color,
        lines: Vec<Line<'static>>,
    ) {
        self.ui.history_render_cache = Some(HistoryRenderCache {
            width,
            accent,
            transcript_revision: self.session.transcript_revision,
            lines,
        });
    }

    pub(crate) fn clear_history_render_cache(&mut self) {
        self.ui.history_render_cache = None;
    }

    pub(crate) fn history_selection_span_for_row(&self, row: usize) -> Option<(usize, usize)> {
        self.ui.history.selection_span_for_row(row)
    }
}
