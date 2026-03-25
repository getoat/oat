use ratatui::{layout::Rect, style::Color, text::Line};

use crate::composer::slice_line;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistorySelectionPoint {
    row: usize,
    column: usize,
}

#[derive(Debug, Default)]
pub struct HistoryViewState {
    pub scroll_top: Option<usize>,
    viewport_rows: usize,
    total_lines: usize,
    snapshot_area: Rect,
    snapshot_lines: Vec<String>,
    selection_anchor: Option<HistorySelectionPoint>,
    selection_focus: Option<HistorySelectionPoint>,
}

#[derive(Clone, Debug)]
pub struct HistoryRenderCache {
    pub width: usize,
    pub accent: Color,
    pub transcript_revision: u64,
    pub lines: Vec<Line<'static>>,
}

impl HistoryViewState {
    pub fn is_pinned(&self) -> bool {
        self.scroll_top.is_some()
    }

    pub fn sync_viewport(&mut self, total_lines: usize, viewport_rows: usize) -> usize {
        self.total_lines = total_lines;
        self.viewport_rows = viewport_rows.max(1);
        let max_start = self.max_start();
        if let Some(top) = self.scroll_top.as_mut() {
            *top = (*top).min(max_start);
            *top
        } else {
            max_start
        }
    }

    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    pub fn viewport_rows(&self) -> usize {
        self.viewport_rows.max(1)
    }

    pub fn scroll_position(&self) -> usize {
        self.current_start()
    }

    pub fn update_snapshot(&mut self, area: Rect, lines: Vec<String>) {
        self.snapshot_area = area;
        self.snapshot_lines = lines;
    }

    pub fn page_rows(&self) -> usize {
        self.viewport_rows.max(1)
    }

    pub fn scroll_up(&mut self, lines: usize) {
        let current = self.current_start();
        self.scroll_top = Some(current.saturating_sub(lines));
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let current = self.current_start();
        self.scroll_top = Some(current.saturating_add(lines).min(self.max_start()));
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_top = Some(0);
    }

    pub fn resume_follow(&mut self) {
        self.scroll_top = None;
    }

    pub fn start_selection(&mut self, column: u16, row: u16) {
        let point = self.selection_point(column, row, false);
        self.selection_anchor = point;
        self.selection_focus = point;
    }

    pub fn update_selection(&mut self, column: u16, row: u16) {
        if self.selection_anchor.is_none() {
            return;
        }
        self.selection_focus = self.selection_point(column, row, true);
    }

    pub fn finish_selection(&mut self, column: u16, row: u16) -> Option<String> {
        let anchor = self.selection_anchor?;
        let focus = self
            .selection_point(column, row, true)
            .or(self.selection_focus)?;
        self.selection_anchor = None;
        self.selection_focus = None;
        (anchor != focus).then(|| self.selected_text(anchor, focus))
    }

    pub fn selection_span_for_row(&self, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.ordered_selection_points()?;
        if row < start.row || row > end.row {
            return None;
        }

        let line_width = self.snapshot_lines.get(row)?.chars().count().max(1);
        let span = if start.row == end.row {
            (start.column, end.column + 1)
        } else if row == start.row {
            (start.column, line_width)
        } else if row == end.row {
            (0, end.column + 1)
        } else {
            (0, line_width)
        };

        Some((span.0.min(line_width), span.1.min(line_width)))
    }

    fn current_start(&self) -> usize {
        self.scroll_top.unwrap_or(self.max_start())
    }

    fn max_start(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_rows.max(1))
    }

    fn selection_point(&self, column: u16, row: u16, clamp: bool) -> Option<HistorySelectionPoint> {
        if self.snapshot_lines.is_empty() || self.snapshot_area.width == 0 {
            return None;
        }

        let area = self.snapshot_area;
        let min_row = area.y;
        let max_row = area
            .y
            .saturating_add(self.snapshot_lines.len().saturating_sub(1) as u16);
        let row = if clamp {
            row.clamp(min_row, max_row)
        } else if row < min_row || row > max_row {
            return None;
        } else {
            row
        };

        let min_column = area.x;
        let max_column = area.x.saturating_add(area.width.saturating_sub(1));
        let column = if clamp {
            column.clamp(min_column, max_column)
        } else if column < min_column || column > max_column {
            return None;
        } else {
            column
        };

        let row_index = row.saturating_sub(area.y) as usize;
        let line_width = self.snapshot_lines[row_index].chars().count();
        let column_index = column.saturating_sub(area.x) as usize;

        Some(HistorySelectionPoint {
            row: row_index,
            column: column_index.min(line_width.saturating_sub(1)),
        })
    }

    fn ordered_selection_points(&self) -> Option<(HistorySelectionPoint, HistorySelectionPoint)> {
        let anchor = self.selection_anchor?;
        let focus = self.selection_focus?;
        if anchor == focus {
            return None;
        }

        Some(
            if (anchor.row, anchor.column) <= (focus.row, focus.column) {
                (anchor, focus)
            } else {
                (focus, anchor)
            },
        )
    }

    fn selected_text(&self, anchor: HistorySelectionPoint, focus: HistorySelectionPoint) -> String {
        let (start, end) = if (anchor.row, anchor.column) <= (focus.row, focus.column) {
            (anchor, focus)
        } else {
            (focus, anchor)
        };

        let mut lines = Vec::new();
        for row in start.row..=end.row {
            let line = &self.snapshot_lines[row];
            let segment = if start.row == end.row {
                slice_line(line, start.column, end.column + 1)
            } else if row == start.row {
                slice_line(line, start.column, line.chars().count())
            } else if row == end.row {
                slice_line(line, 0, end.column + 1)
            } else {
                line.clone()
            };
            lines.push(segment);
        }

        lines.join("\n")
    }
}
