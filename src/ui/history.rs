use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, ChatMessage, ToolCall, ToolResultEntry, TranscriptEntry};

use super::{
    markdown::{push_message_lines, push_pending_lines, rendered_line_text},
    tool_activity::{push_tool_call_lines, push_tool_result_lines},
};

const MAX_VISIBLE_TOOL_ACTIVITY: usize = 5;

#[derive(Clone, Copy)]
enum VisibleEntry<'a> {
    Message(&'a ChatMessage),
    ToolCall(&'a ToolCall),
    ToolResult(&'a ToolResultEntry),
}

impl VisibleEntry<'_> {
    fn is_tool_activity(self) -> bool {
        matches!(self, Self::ToolCall(_) | Self::ToolResult(_))
    }
}

pub(super) fn render_history(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    accent: Color,
    loading_frame: &str,
) {
    let show_scrollbar = area.width > 1;
    let history_layout = if show_scrollbar {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(area)
    };
    let content_area = history_layout[0];
    let mut lines = Vec::new();
    let visible_entries = visible_entries(app);
    let mut index = 0;

    while index < visible_entries.len() {
        if visible_entries[index].is_tool_activity() {
            let run_end = tool_activity_run_end(&visible_entries, index);
            push_tool_activity_run_lines(
                &mut lines,
                &visible_entries[index..run_end],
                content_area.width as usize,
            );
            index = run_end;
            continue;
        }

        push_visible_entry_lines(
            &mut lines,
            visible_entries[index],
            content_area.width as usize,
            accent,
        );
        lines.push(Line::default());
        index += 1;
    }

    if app.has_pending_reply() && !app.has_visible_pending_content() {
        push_pending_lines(
            &mut lines,
            content_area.width as usize,
            accent,
            loading_frame,
        );
    }

    let visible_count = content_area.height as usize;
    let start = app.sync_history_viewport(lines.len(), visible_count);
    let mut visible_lines = history_viewport_lines(lines, start, visible_count);
    let history_snapshot = visible_lines
        .iter()
        .map(rendered_line_text)
        .collect::<Vec<_>>();
    app.update_history_snapshot(content_area, history_snapshot);
    apply_history_selection_highlight(&mut visible_lines, app, accent);
    frame.render_widget(Paragraph::new(visible_lines), content_area);

    if show_scrollbar && app.history_total_lines() > app.history_viewport_rows() {
        render_history_scrollbar(frame, history_layout[1], app, accent);
    }
}

pub(super) fn scrollbar_thumb_bounds(
    track_height: usize,
    total_lines: usize,
    viewport_rows: usize,
    scroll_position: usize,
) -> (usize, usize) {
    if track_height == 0 {
        return (0, 0);
    }

    let total_lines = total_lines.max(1);
    let viewport_rows = viewport_rows.max(1).min(total_lines);
    let thumb_len =
        ((viewport_rows as f64 / total_lines as f64) * track_height as f64).round() as usize;
    let thumb_len = thumb_len.clamp(1, track_height);

    let max_scroll = total_lines.saturating_sub(viewport_rows);
    let max_thumb_start = track_height.saturating_sub(thumb_len);
    let thumb_start = if max_scroll == 0 || max_thumb_start == 0 {
        0
    } else {
        ((scroll_position.min(max_scroll) as f64 / max_scroll as f64) * max_thumb_start as f64)
            .round() as usize
    };

    (thumb_start, thumb_len)
}

fn history_viewport_lines(
    lines: Vec<Line<'static>>,
    start: usize,
    visible_count: usize,
) -> Vec<Line<'static>> {
    if visible_count == 0 {
        return Vec::new();
    }

    lines.into_iter().skip(start).take(visible_count).collect()
}

fn apply_history_selection_highlight(lines: &mut [Line<'static>], app: &App, accent: Color) {
    let highlight = Style::default().bg(accent);
    for (row, line) in lines.iter_mut().enumerate() {
        if let Some((start, end)) = app.history_selection_span_for_row(row) {
            *line = highlight_line_range(line.clone(), start, end, highlight);
        }
    }
}

fn highlight_line_range(
    line: Line<'static>,
    start: usize,
    end: usize,
    highlight: Style,
) -> Line<'static> {
    if start >= end {
        return line;
    }

    let mut spans = Vec::new();
    let mut offset = 0;

    for span in line.spans {
        let content = span.content.into_owned();
        let width = content.chars().count();
        let span_start = offset;
        let span_end = offset + width;

        if width == 0 || end <= span_start || start >= span_end {
            spans.push(Span::styled(content, span.style));
            offset = span_end;
            continue;
        }

        let local_start = start.saturating_sub(span_start).min(width);
        let local_end = end.saturating_sub(span_start).min(width);

        let before = slice_chars(&content, 0, local_start);
        let selected = slice_chars(&content, local_start, local_end);
        let after = slice_chars(&content, local_end, width);

        if !before.is_empty() {
            spans.push(Span::styled(before, span.style));
        }
        if !selected.is_empty() {
            spans.push(Span::styled(selected, span.style.patch(highlight)));
        }
        if !after.is_empty() {
            spans.push(Span::styled(after, span.style));
        }

        offset = span_end;
    }

    Line {
        style: line.style,
        alignment: line.alignment,
        spans,
    }
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn render_history_scrollbar(frame: &mut Frame, area: Rect, app: &App, accent: Color) {
    let (thumb_start, thumb_len) = scrollbar_thumb_bounds(
        area.height as usize,
        app.history_total_lines(),
        app.history_viewport_rows(),
        app.history_scroll_position(),
    );

    let lines = (0..area.height as usize)
        .map(|index| {
            if index >= thumb_start && index < thumb_start + thumb_len {
                Line::from(Span::styled(" ", Style::default().bg(accent)))
            } else {
                Line::from(Span::styled("│", Style::default().fg(Color::DarkGray)))
            }
        })
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), area);
}

fn visible_entries(app: &App) -> Vec<VisibleEntry<'_>> {
    app.entries()
        .iter()
        .filter_map(|entry| match entry {
            TranscriptEntry::Message(message) => Some(VisibleEntry::Message(message)),
            TranscriptEntry::ToolCall(tool_call) => Some(VisibleEntry::ToolCall(tool_call)),
            TranscriptEntry::ToolResult(tool_result) => app
                .show_tool_output()
                .then_some(VisibleEntry::ToolResult(tool_result)),
        })
        .collect()
}

fn tool_activity_run_end(entries: &[VisibleEntry<'_>], start: usize) -> usize {
    let mut end = start;
    while end < entries.len() && entries[end].is_tool_activity() {
        end += 1;
    }
    end
}

fn push_visible_entry_lines(
    lines: &mut Vec<Line<'static>>,
    entry: VisibleEntry<'_>,
    width: usize,
    accent: Color,
) {
    match entry {
        VisibleEntry::Message(message) => push_message_lines(lines, message, width, accent),
        VisibleEntry::ToolCall(tool_call) => push_tool_call_lines(lines, tool_call, width),
        VisibleEntry::ToolResult(tool_result) => push_tool_result_lines(lines, tool_result, width),
    }
}

fn push_tool_activity_run_lines(
    lines: &mut Vec<Line<'static>>,
    entries: &[VisibleEntry<'_>],
    width: usize,
) {
    let hidden_count = entries.len().saturating_sub(MAX_VISIBLE_TOOL_ACTIVITY);
    if hidden_count > 0 {
        lines.push(Line::from(Span::styled(
            format!("... {hidden_count} more tool calls"),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    for entry in entries.iter().skip(hidden_count).copied() {
        match entry {
            VisibleEntry::ToolCall(tool_call) => push_tool_call_lines(lines, tool_call, width),
            VisibleEntry::ToolResult(tool_result) => {
                push_tool_result_lines(lines, tool_result, width)
            }
            VisibleEntry::Message(_) => {}
        }
        lines.push(Line::default());
    }
}
