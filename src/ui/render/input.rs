use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::{
    app::{App, ops, query},
    ui::wrap::wrap_text,
};

use super::{
    approvals::{render_shell_approval_prompt, render_write_approval_prompt},
    ask_user::render_ask_user_prompt,
    helpers::{collect_line_range, composer_content_width},
    planning::render_plan_review_prompt,
};

pub(super) fn render_input(frame: &mut Frame, app: &mut App, area: Rect, accent: Color) {
    if let Some(pending) = query::pending_write_approval(app.state()) {
        render_write_approval_prompt(frame, pending, area, accent);
        return;
    }

    if query::has_pending_shell_approval(app.state()) {
        render_shell_approval_prompt(frame, app, area, accent);
        return;
    }

    if query::has_pending_ask_user(app.state()) {
        render_ask_user_prompt(frame, app, area, accent);
        return;
    }

    if query::plan_review_selection_active(app.state()) {
        render_plan_review_prompt(frame, app, area, accent);
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(Style::default().fg(accent));
    ops::composer::set_composer_wrap_width(app.state_mut(), composer_content_width(area.width));
    let paragraph = Paragraph::new(render_composer_lines(app, accent)).block(block);
    frame.render_widget(paragraph, area);
}

fn render_composer_lines(app: &mut App, accent: Color) -> Vec<Line<'static>> {
    let show_placeholder = {
        let composer = query::composer(app.state());
        composer.lines() == [String::new()] && !composer.placeholder_text().is_empty()
    };
    if show_placeholder {
        let placeholder = query::composer(app.state()).placeholder_text().to_owned();
        let placeholder_style = Style::default().fg(Color::DarkGray);
        let cursor_style = Style::default().bg(accent).fg(Color::Black);
        let content_width = query::composer_wrap_width(app.state());
        let placeholder_rows = if content_width <= 1 {
            vec![String::new()]
        } else {
            wrap_text(&placeholder, content_width.saturating_sub(1))
        };
        let mut lines = Vec::new();
        for (index, row) in placeholder_rows.into_iter().enumerate() {
            if index == 0 {
                let mut spans = vec![Span::styled(" ", cursor_style)];
                if !row.is_empty() {
                    spans.push(Span::styled(row, placeholder_style));
                }
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(row, placeholder_style)));
            }
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(" ", cursor_style)));
        }
        return lines;
    }

    let cursor_position = query::composer(app.state()).cursor();
    let base_style = query::composer(app.state()).style();
    let (rows, cursor) = {
        let layout = ops::composer::composer_layout(app.state_mut());
        (layout.rows().to_vec(), layout.cursor_state(cursor_position))
    };
    let cursor_style = Style::default().bg(accent).fg(Color::Black);
    let mut lines = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let cursor_col = cursor
            .as_ref()
            .filter(|state| state.row_index == index)
            .map(|state| state.visual_col);
        lines.push(render_composer_row(
            &query::composer(app.state()).lines()[row.line_index],
            row.start_col,
            row.end_col,
            cursor_col,
            base_style,
            cursor_style,
        ));
    }

    if lines.is_empty() {
        vec![Line::default()]
    } else {
        lines
    }
}

pub(super) fn render_composer_row(
    line: &str,
    start_col: usize,
    end_col: usize,
    cursor_col: Option<usize>,
    base_style: Style,
    cursor_style: Style,
) -> Line<'static> {
    let row_len = end_col.saturating_sub(start_col);
    if row_len == 0 {
        return match cursor_col {
            Some(_) => Line::from(Span::styled(" ", cursor_style)),
            None => Line::default(),
        };
    }

    let Some(cursor_col) = cursor_col else {
        return Line::from(Span::styled(
            collect_line_range(line, start_col, end_col),
            base_style,
        ));
    };

    let mut before = String::new();
    let mut after = String::new();
    let mut current = None;

    for (index, ch) in line.chars().enumerate() {
        if index < start_col {
            continue;
        }
        if index >= end_col {
            break;
        }

        let visual_index = index - start_col;
        if visual_index == cursor_col && current.is_none() {
            current = Some(ch);
        } else if current.is_none() {
            before.push(ch);
        } else {
            after.push(ch);
        }
    }

    if current.is_none() && cursor_col >= row_len {
        let text = collect_line_range(line, start_col, end_col);
        let mut spans = Vec::new();
        if !text.is_empty() {
            spans.push(Span::styled(text, base_style));
        }
        spans.push(Span::styled(" ", cursor_style));
        return Line::from(spans);
    }

    let mut spans = Vec::new();
    if !before.is_empty() {
        spans.push(Span::styled(before, base_style));
    }
    if let Some(current) = current {
        spans.push(Span::styled(current.to_string(), cursor_style));
    } else {
        spans.push(Span::styled(" ", cursor_style));
    }
    if !after.is_empty() {
        spans.push(Span::styled(after, base_style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests;
