use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::{
    app::{App, query},
    ui::wrap::wrap_text,
};

use super::helpers::composer_content_width;

const MAX_QUEUE_ROWS: usize = 8;
const QUEUED_MARKER: &str = "[queued]";

pub(super) fn queued_message_strip_height(app: &App, panel_width: u16, screen_height: u16) -> u16 {
    if !query::has_queued_messages(app.state()) {
        return 0;
    }

    let content_rows = build_queue_strip_lines(
        app,
        composer_content_width(panel_width),
        accentless_user_color(),
        queue_strip_max_content_rows(screen_height),
    )
    .len() as u16;

    content_rows.saturating_add(2)
}

pub(super) fn render_queued_message_strip(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Line::from(Span::styled(
            " Queued ",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        )));
    let lines = build_queue_strip_lines(
        app,
        composer_content_width(area.width),
        accent,
        area.height.saturating_sub(2) as usize,
    );
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn build_queue_strip_lines(
    app: &App,
    content_width: usize,
    accent: Color,
    max_content_rows: usize,
) -> Vec<Line<'static>> {
    if max_content_rows == 0 {
        return Vec::new();
    }

    let blocks = query::queued_messages(app.state())
        .iter()
        .map(|message| render_queued_message_block(message, content_width, accent))
        .collect::<Vec<_>>();

    let mut lines = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        let needs_separator = !lines.is_empty();
        let remaining_messages = blocks.len().saturating_sub(index + 1);
        let reserved_summary = usize::from(remaining_messages > 0);
        let next_len = lines.len() + usize::from(needs_separator) + block.len() + reserved_summary;

        if next_len <= max_content_rows {
            if needs_separator {
                lines.push(Line::default());
            }
            lines.extend(block.iter().cloned());
            continue;
        }

        if lines.is_empty() {
            let available = max_content_rows.saturating_sub(reserved_summary).max(1);
            lines.extend(block.iter().take(available).cloned());
            if reserved_summary == 1 {
                lines.truncate(max_content_rows.saturating_sub(1));
                lines.push(queue_summary_line(remaining_messages));
            }
            return lines;
        }

        lines.push(queue_summary_line(blocks.len() - index));
        return lines;
    }

    lines
}

fn render_queued_message_block(
    text: &str,
    content_width: usize,
    accent: Color,
) -> Vec<Line<'static>> {
    let prefix_width = queued_prefix_width();
    let wrapped = wrap_text(text, content_width.saturating_sub(prefix_width).max(1));
    let marker_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let queued_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);
    let mut lines = Vec::new();

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled("●", marker_style),
                Span::raw(" "),
                Span::styled("User", marker_style),
                Span::raw("  "),
                Span::styled(QUEUED_MARKER, queued_style),
                Span::raw("  "),
                Span::raw(chunk),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(prefix_width)),
                Span::raw(chunk),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("●", marker_style),
            Span::raw(" "),
            Span::styled("User", marker_style),
            Span::raw("  "),
            Span::styled(QUEUED_MARKER, queued_style),
        ]));
    }

    lines
}

fn queue_summary_line(hidden_count: usize) -> Line<'static> {
    Line::from(Span::styled(
        format!("… +{hidden_count} more queued"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    ))
}

fn queue_strip_max_content_rows(screen_height: u16) -> usize {
    usize::from((screen_height / 3).max(1)).min(MAX_QUEUE_ROWS)
}

fn queued_prefix_width() -> usize {
    1 + 1 + "User".chars().count() + 2 + QUEUED_MARKER.chars().count() + 2
}

fn accentless_user_color() -> Color {
    Color::Gray
}
