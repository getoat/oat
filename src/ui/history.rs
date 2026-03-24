use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{
    App, ChatMessage, MessageStyle, SubagentStatusEntry, ToolCall, ToolResultEntry, TranscriptEntry,
};

use super::{
    markdown::{push_message_lines, push_pending_lines, rendered_line_text},
    subagent_activity::push_subagent_status_lines,
    tool_activity::{push_tool_call_lines, push_tool_result_lines},
};

const MAX_VISIBLE_TOOL_ACTIVITY: usize = 5;
const STARTUP_VERSION: &str = env!("CARGO_PKG_VERSION");
const STARTUP_SPARKLE_INTERVAL_TICKS: usize = 3;
const STARTUP_BANNER_LINES: [&str; 7] = [
    "                         ░██    ",
    "                         ░██    ",
    " ░███████   ░██████   ░████████ ",
    "░██    ░██       ░██     ░██    ",
    "░██    ░██  ░███████     ░██    ",
    "░██    ░██ ░██   ░██     ░██    ",
    " ░███████   ░█████░██     ░████ ",
];

#[derive(Clone, Copy)]
enum VisibleEntry<'a> {
    Message(&'a ChatMessage),
    ToolCall(&'a ToolCall),
    ToolResult(&'a ToolResultEntry),
    SubagentStatus(&'a SubagentStatusEntry),
}

impl VisibleEntry<'_> {
    fn is_tool_activity(self) -> bool {
        matches!(
            self,
            Self::ToolCall(_) | Self::ToolResult(_) | Self::SubagentStatus(_)
        )
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
    let content_width = content_area.width as usize;
    let use_cache = app.history_cache_allowed();
    let mut owned_lines = None;
    let total_lines = if use_cache {
        if app.cached_history_lines(content_width, accent).is_none() {
            let lines = build_history_lines(app, content_width, accent, loading_frame);
            app.store_history_render_cache(content_width, accent, lines);
        }
        app.cached_history_lines(content_width, accent)
            .expect("history cache should be populated")
            .len()
    } else {
        app.clear_history_render_cache();
        owned_lines = Some(build_history_lines(
            app,
            content_width,
            accent,
            loading_frame,
        ));
        owned_lines.as_ref().expect("owned history lines").len()
    };
    let visible_count = content_area.height as usize;
    let start = app.sync_history_viewport(total_lines, visible_count);
    let mut visible_lines = if use_cache {
        let lines = app
            .cached_history_lines(content_width, accent)
            .expect("history cache should be populated");
        history_viewport_lines(lines, start, visible_count)
    } else {
        history_viewport_lines(
            owned_lines
                .as_ref()
                .expect("owned history lines")
                .as_slice(),
            start,
            visible_count,
        )
    };
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

fn build_history_lines(
    app: &App,
    width: usize,
    accent: Color,
    loading_frame: &str,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.shows_startup_banner() {
        push_startup_banner_lines(&mut lines, accent, app.tick_count());
        lines.push(Line::default());
    }
    let visible_entries = visible_entries(app);
    let mut index = 0;

    while index < visible_entries.len() {
        if visible_entries[index].is_tool_activity() {
            let run_end = tool_activity_run_end(&visible_entries, index);
            push_tool_activity_run_lines(&mut lines, &visible_entries[index..run_end], width);
            index = run_end;
            continue;
        }

        push_visible_entry_lines(&mut lines, visible_entries[index], width, accent);
        lines.push(Line::default());
        index += 1;
    }

    if app.should_show_history_busy_indicator() {
        push_pending_lines(
            &mut lines,
            width,
            accent,
            loading_frame,
            app.history_pending_status_label(),
        );
    }

    lines
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
    lines: &[Line<'static>],
    start: usize,
    visible_count: usize,
) -> Vec<Line<'static>> {
    if visible_count == 0 {
        return Vec::new();
    }

    lines
        .iter()
        .skip(start)
        .take(visible_count)
        .cloned()
        .collect()
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
            TranscriptEntry::SubagentStatus(status) => Some(VisibleEntry::SubagentStatus(status)),
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
        VisibleEntry::Message(message) => {
            if message.style == MessageStyle::Commentary {
                lines.push(commentary_separator_line(width));
            }
            push_message_lines(lines, message, width, accent);
        }
        VisibleEntry::ToolCall(tool_call) => push_tool_call_lines(lines, tool_call, width),
        VisibleEntry::ToolResult(tool_result) => push_tool_result_lines(lines, tool_result, width),
        VisibleEntry::SubagentStatus(status) => push_subagent_status_lines(lines, status, width),
    }
}

fn commentary_separator_line(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(1)),
        Style::default().fg(Color::DarkGray),
    ))
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
            VisibleEntry::SubagentStatus(status) => {
                push_subagent_status_lines(lines, status, width)
            }
            VisibleEntry::Message(_) => {}
        }
        lines.push(Line::default());
    }
}

fn push_startup_banner_lines(lines: &mut Vec<Line<'static>>, accent: Color, tick_count: usize) {
    lines.extend(
        STARTUP_BANNER_LINES
            .iter()
            .enumerate()
            .map(|(line_index, line)| sparkling_startup_line(line, accent, line_index, tick_count)),
    );
    lines.push(Line::from(Span::styled(
        centered_startup_version(),
        Style::default().fg(accent),
    )));
}

fn sparkling_startup_line(
    text: &str,
    accent: Color,
    row: usize,
    tick_count: usize,
) -> Line<'static> {
    let phase = tick_count / STARTUP_SPARKLE_INTERVAL_TICKS;
    let base_style = Style::default().fg(accent).add_modifier(Modifier::BOLD);
    let lighter_style = Style::default()
        .fg(startup_highlight_color(accent))
        .add_modifier(Modifier::BOLD);
    let darker_style = Style::default()
        .fg(startup_shadow_color(accent))
        .add_modifier(Modifier::BOLD);
    let spans: Vec<_> = text
        .chars()
        .enumerate()
        .map(|(column, ch)| {
            let style = if is_startup_banner_block(ch) {
                startup_sparkle_style(base_style, lighter_style, darker_style, row, column, phase)
            } else {
                base_style
            };
            Span::styled(ch.to_string(), style)
        })
        .collect();

    Line::from(spans)
}

fn startup_sparkle_style(
    base_style: Style,
    lighter_style: Style,
    darker_style: Style,
    row: usize,
    column: usize,
    phase: usize,
) -> Style {
    match sparkle_roll(row, column, phase) {
        0 | 1 => lighter_style,
        2 => darker_style,
        _ => base_style,
    }
}

fn sparkle_roll(row: usize, column: usize, phase: usize) -> usize {
    let seed = row as u64 * 37 + column as u64 * 17 + phase as u64 * 29 + 11;
    ((seed ^ (seed >> 3) ^ (seed >> 7)) % 23) as usize
}

fn is_startup_banner_block(ch: char) -> bool {
    matches!(ch, '█' | '░')
}

fn startup_shadow_color(accent: Color) -> Color {
    match accent {
        Color::Magenta => Color::Rgb(144, 72, 176),
        Color::Cyan => Color::Rgb(0, 146, 168),
        other => other,
    }
}

fn startup_highlight_color(accent: Color) -> Color {
    match accent {
        Color::Magenta => Color::LightMagenta,
        Color::Cyan => Color::LightCyan,
        other => other,
    }
}

fn centered_startup_version() -> String {
    let version = format!("v{STARTUP_VERSION}");
    let banner_width = STARTUP_BANNER_LINES
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(version.chars().count());
    let version_width = version.chars().count();
    let left_padding = banner_width.saturating_sub(version_width) / 2;

    format!("{}{}", " ".repeat(left_padding), version)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::{
        app::{Action, Effect},
        config::ReasoningEffort,
        llm::StreamEvent,
    };

    #[test]
    fn interleaved_stream_history_preserves_visible_order() {
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("Inspect the repo");
        let effect = app.apply(Action::SubmitMessage);
        assert!(matches!(
            effect,
            Some(Effect::PromptModel { reply_id: 1, .. })
        ));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Before tool".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src"}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("After tool".into()),
        });

        let lines = build_history_lines(&app, 80, Color::Cyan, "...");
        let rendered = lines.iter().map(rendered_line_text).collect::<Vec<_>>();

        let before_index = rendered
            .iter()
            .position(|line| line.contains("Before tool"))
            .expect("before-tool line");
        let tool_index = rendered
            .iter()
            .position(|line| line.contains("◇ tool") && line.contains("List"))
            .expect("tool line");
        let after_index = rendered
            .iter()
            .position(|line| line.contains("After tool"))
            .expect("after-tool line");

        assert!(before_index < tool_index);
        assert!(tool_index < after_index);
    }

    #[test]
    fn commentary_between_tool_runs_stays_in_visible_order() {
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("Inspect the repo");
        let effect = app.apply(Action::SubmitMessage);
        assert!(matches!(
            effect,
            Some(Effect::PromptModel { reply_id: 1, .. })
        ));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src"}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Commentary("Found the relevant module. Reading it now.".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "ReadFile".into(),
                arguments: r#"{"path":"src/main.rs"}"#.into(),
            },
        });

        let lines = build_history_lines(&app, 80, Color::Cyan, "...");
        let rendered = lines.iter().map(rendered_line_text).collect::<Vec<_>>();

        let first_tool_index = rendered
            .iter()
            .position(|line| line.contains("◇ tool") && line.contains("List"))
            .expect("list tool line");
        let commentary_index = rendered
            .iter()
            .position(|line| line.contains("Found the relevant module. Reading it now."))
            .expect("commentary line");
        let second_tool_index = rendered
            .iter()
            .position(|line| line.contains("◇ tool") && line.contains("ReadFile"))
            .expect("read file tool line");

        assert!(first_tool_index < commentary_index);
        assert!(commentary_index < second_tool_index);
    }

    #[test]
    fn commentary_renders_separator_before_message() {
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("Inspect the repo");
        let effect = app.apply(Action::SubmitMessage);
        assert!(matches!(
            effect,
            Some(Effect::PromptModel { reply_id: 1, .. })
        ));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Commentary("Checking the registry first.".into()),
        });

        let lines = build_history_lines(&app, 40, Color::Cyan, "...");
        let rendered = lines.iter().map(rendered_line_text).collect::<Vec<_>>();
        let separator = "─".repeat(40);
        let separator_index = rendered
            .iter()
            .position(|line| line == &separator)
            .expect("separator line");
        let commentary_index = rendered
            .iter()
            .position(|line| line.contains("Checking the registry first."))
            .expect("commentary line");

        assert_eq!(separator_index + 1, commentary_index);
    }
}
