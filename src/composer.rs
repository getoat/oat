#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerVisualRow {
    pub line_index: usize,
    pub start_col: usize,
    pub end_col: usize,
    pub ends_line: bool,
}

impl ComposerVisualRow {
    pub fn len(&self) -> usize {
        self.end_col.saturating_sub(self.start_col)
    }

    pub fn max_cursor_col(&self) -> usize {
        if self.ends_line {
            self.len()
        } else {
            self.len().saturating_sub(1)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerCursorState {
    pub row_index: usize,
    pub total_rows: usize,
    pub visual_col: usize,
    pub row: ComposerVisualRow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposerLayout {
    rows: Vec<ComposerVisualRow>,
}

impl ComposerLayout {
    pub fn new(lines: &[String], width: usize) -> Self {
        let width = width.max(1);
        let mut rows = Vec::new();

        for (line_index, line) in lines.iter().enumerate() {
            rows.extend(wrap_line(line_index, line, width));
        }

        if rows.is_empty() {
            rows.push(ComposerVisualRow {
                line_index: 0,
                start_col: 0,
                end_col: 0,
                ends_line: true,
            });
        }

        Self { rows }
    }

    pub fn rows(&self) -> &[ComposerVisualRow] {
        &self.rows
    }

    pub fn height(&self) -> usize {
        self.rows.len().max(1)
    }

    pub fn cursor_state(&self, cursor: (usize, usize)) -> Option<ComposerCursorState> {
        let (line_index, col) = cursor;
        let row_index = self
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| row_contains_cursor(row, line_index, col).then_some(index))
            .or_else(|| {
                self.rows
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(index, row)| (row.line_index == line_index).then_some(index))
            })?;
        let row = self.rows[row_index].clone();
        Some(ComposerCursorState {
            row_index,
            total_rows: self.rows.len(),
            visual_col: col.saturating_sub(row.start_col),
            row,
        })
    }

    pub fn target_cursor_for_row(
        &self,
        row_index: usize,
        desired_visual_col: usize,
    ) -> Option<(usize, usize)> {
        let row = self.rows.get(row_index)?;
        let visual_col = desired_visual_col.min(row.max_cursor_col());
        Some((row.line_index, row.start_col + visual_col))
    }
}

fn row_contains_cursor(row: &ComposerVisualRow, line_index: usize, col: usize) -> bool {
    if row.line_index != line_index {
        return false;
    }

    if row.start_col == row.end_col {
        return col == row.start_col;
    }

    if row.ends_line {
        col >= row.start_col && col <= row.end_col
    } else {
        col >= row.start_col && col < row.end_col
    }
}

fn wrap_line(line_index: usize, line: &str, width: usize) -> Vec<ComposerVisualRow> {
    let char_count = line.chars().count();
    if char_count == 0 {
        return vec![ComposerVisualRow {
            line_index,
            start_col: 0,
            end_col: 0,
            ends_line: true,
        }];
    }

    let chars = line.chars().collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= width {
            rows.push(ComposerVisualRow {
                line_index,
                start_col: start,
                end_col: chars.len(),
                ends_line: true,
            });
            break;
        }

        let end = last_wrappable_boundary(&chars[start..start + width])
            .map(|boundary| start + boundary)
            .filter(|boundary| *boundary > start)
            .unwrap_or(start + width);
        rows.push(ComposerVisualRow {
            line_index,
            start_col: start,
            end_col: end,
            ends_line: false,
        });
        start = end;
    }

    rows
}

fn last_wrappable_boundary(chars: &[char]) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, ch)| (ch.is_whitespace() && index > 0).then_some(index + 1))
}

pub fn slice_line(line: &str, start: usize, end: usize) -> String {
    line.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_long_line_at_word_boundaries_when_possible() {
        let layout = ComposerLayout::new(&["alpha beta gamma".into()], 6);

        assert_eq!(
            layout.rows(),
            &[
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 0,
                    end_col: 6,
                    ends_line: false,
                },
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 6,
                    end_col: 11,
                    ends_line: false,
                },
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 11,
                    end_col: 16,
                    ends_line: true,
                },
            ]
        );
    }

    #[test]
    fn wraps_long_tokens_when_no_boundary_exists() {
        let layout = ComposerLayout::new(&["abcdefgh".into()], 3);

        assert_eq!(
            layout.rows(),
            &[
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 0,
                    end_col: 3,
                    ends_line: false,
                },
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 3,
                    end_col: 6,
                    ends_line: false,
                },
                ComposerVisualRow {
                    line_index: 0,
                    start_col: 6,
                    end_col: 8,
                    ends_line: true,
                },
            ]
        );
    }

    #[test]
    fn cursor_at_wrap_boundary_belongs_to_next_visual_row() {
        let layout = ComposerLayout::new(&["alpha beta".into()], 6);
        let state = layout.cursor_state((0, 6)).expect("cursor state");

        assert_eq!(state.row_index, 1);
        assert_eq!(state.visual_col, 0);
    }

    #[test]
    fn target_cursor_for_non_terminal_row_clamps_inside_the_row() {
        let layout = ComposerLayout::new(&["alpha beta".into()], 6);

        assert_eq!(layout.target_cursor_for_row(0, 10), Some((0, 5)));
        assert_eq!(layout.target_cursor_for_row(1, 10), Some((0, 10)));
    }
}
