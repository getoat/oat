use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::session::ProposedPlanEntry;
use crate::app::{
    App, BackgroundTerminalStatusEntry, ChatMessage, HostedToolStatusEntry, MessageStyle,
    SubagentStatusEntry, ToolCall, ToolResultEntry, TranscriptEntry, query,
};
use crate::todo::TodoSnapshot;

use super::{
    background_terminal_activity::push_background_terminal_status_lines,
    hosted_tool_activity::push_hosted_tool_status_lines,
    markdown::{push_message_lines, push_pending_lines, rendered_line_text},
    scrollbar::render_vertical_scrollbar,
    subagent_activity::push_subagent_status_lines,
    todo_activity::push_todo_snapshot_lines,
    tool_activity::{push_tool_call_lines, push_tool_result_lines},
};

#[cfg(test)]
use super::scrollbar::scrollbar_thumb_bounds;

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
    ProposedPlan(&'a ProposedPlanEntry),
    ToolCall(&'a ToolCall),
    ToolResult(&'a ToolResultEntry),
    HostedToolStatus(&'a HostedToolStatusEntry),
    TodoSnapshot(&'a TodoSnapshot),
    SubagentStatus(&'a SubagentStatusEntry),
    BackgroundTerminalStatus(&'a BackgroundTerminalStatusEntry),
}

impl VisibleEntry<'_> {
    fn is_tool_activity(self) -> bool {
        matches!(
            self,
            Self::ToolCall(_)
                | Self::ToolResult(_)
                | Self::HostedToolStatus(_)
                | Self::SubagentStatus(_)
                | Self::BackgroundTerminalStatus(_)
        )
    }
}

#[derive(Clone, Copy)]
enum CacheAction {
    Rebuild,
    RefreshTail,
    Reuse,
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
    let use_cache = !query::shows_startup_banner_state(app.state())
        && !query::should_show_history_busy_indicator_state(app.state());
    let mut owned_lines = None;
    let total_lines = if use_cache {
        let structure_revision = query::transcript_structure_revision(app.state());
        let tail_revision = query::transcript_tail_revision(app.state());
        let cache_action = query::history_render_cache(app.state())
            .map(|cache| {
                if cache.width != content_width
                    || cache.accent != accent
                    || cache.structure_revision != structure_revision
                {
                    CacheAction::Rebuild
                } else if cache.tail_revision != tail_revision {
                    CacheAction::RefreshTail
                } else {
                    CacheAction::Reuse
                }
            })
            .unwrap_or(CacheAction::Rebuild);

        let cache = match cache_action {
            CacheAction::Rebuild => build_history_render_cache(app, content_width, accent),
            CacheAction::RefreshTail => {
                let mut cache = app
                    .state_mut()
                    .ui
                    .history_render_cache
                    .take()
                    .expect("history cache should be populated");
                if !refresh_history_tail_cache(app, &mut cache, content_width, accent) {
                    cache = build_history_render_cache(app, content_width, accent);
                }
                cache
            }
            CacheAction::Reuse => app
                .state_mut()
                .ui
                .history_render_cache
                .take()
                .expect("history cache should be populated"),
        };
        let total_lines = cache.lines.len();
        crate::app::ops::history::set_history_render_cache(app.state_mut(), cache);
        total_lines
    } else {
        crate::app::ops::history::clear_history_render_cache(app.state_mut());
        owned_lines = Some(build_history_lines(
            app,
            content_width,
            accent,
            loading_frame,
        ));
        owned_lines.as_ref().expect("owned history lines").len()
    };
    let visible_count = content_area.height as usize;
    let start = crate::app::ops::history::sync_history_viewport(
        app.state_mut(),
        total_lines,
        visible_count,
    );
    let mut visible_lines = if use_cache {
        let lines = query::history_render_cache(app.state())
            .as_ref()
            .expect("history cache should be populated")
            .lines
            .as_slice();
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
    crate::app::ops::history::update_history_snapshot(
        app.state_mut(),
        content_area,
        history_snapshot,
    );
    apply_history_selection_highlight(&mut visible_lines, app, accent);
    frame.render_widget(Paragraph::new(visible_lines), content_area);

    if show_scrollbar
        && query::history_total_lines(app.state()) > query::history_viewport_rows(app.state())
    {
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
    if query::shows_startup_banner_state(app.state()) {
        push_startup_banner_lines(&mut lines, accent, query::tick_count(app.state()));
        lines.push(Line::default());
    }
    let visible_entries = visible_entries(app);
    let mut index = 0;

    while index < visible_entries.len() {
        if visible_entries[index].1.is_tool_activity() {
            let run_end = tool_activity_run_end(&visible_entries, index);
            push_tool_activity_run_lines(&mut lines, &visible_entries[index..run_end], width);
            index = run_end;
            continue;
        }

        push_visible_entry_lines(&mut lines, visible_entries[index].1, width, accent);
        lines.push(Line::default());
        index += 1;
    }

    if query::should_show_history_busy_indicator_state(app.state()) {
        push_pending_lines(
            &mut lines,
            width,
            accent,
            loading_frame,
            query::history_pending_status_label_state(app.state()),
        );
    }

    lines
}

fn build_history_render_cache(
    app: &App,
    width: usize,
    accent: Color,
) -> crate::app::ui::HistoryRenderCache {
    let visible_entries = visible_entries(app);
    let mut lines = Vec::new();
    let mut tail = None;
    let mut index = 0;

    while index < visible_entries.len() {
        if visible_entries[index].1.is_tool_activity() {
            let run_end = tool_activity_run_end(&visible_entries, index);
            push_tool_activity_run_lines(&mut lines, &visible_entries[index..run_end], width);
            index = run_end;
            continue;
        }

        let start = lines.len();
        push_visible_entry_lines(&mut lines, visible_entries[index].1, width, accent);
        lines.push(Line::default());
        if index + 1 == visible_entries.len() {
            tail = Some(crate::app::ui::HistoryTailRenderCache {
                transcript_index: visible_entries[index].0,
                start,
                end: lines.len(),
            });
        }
        index += 1;
    }

    crate::app::ui::HistoryRenderCache {
        width,
        accent,
        structure_revision: query::transcript_structure_revision(app.state()),
        tail_revision: query::transcript_tail_revision(app.state()),
        lines,
        tail,
    }
}

fn refresh_history_tail_cache(
    app: &App,
    cache: &mut crate::app::ui::HistoryRenderCache,
    width: usize,
    accent: Color,
) -> bool {
    let Some(tail) = cache.tail.clone() else {
        return false;
    };
    let Some((transcript_index, entry)) = visible_entries(app).last().copied() else {
        return false;
    };
    if transcript_index != tail.transcript_index || entry.is_tool_activity() {
        return false;
    }

    let mut replacement = Vec::new();
    push_visible_entry_lines(&mut replacement, entry, width, accent);
    replacement.push(Line::default());
    let replacement_len = replacement.len();
    cache.lines.splice(tail.start..tail.end, replacement);
    cache.tail = Some(crate::app::ui::HistoryTailRenderCache {
        transcript_index,
        start: tail.start,
        end: tail.start + replacement_len,
    });
    cache.tail_revision = query::transcript_tail_revision(app.state());
    true
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
        if let Some((start, end)) = query::history_selection_span(app.state(), row) {
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
    render_vertical_scrollbar(
        frame,
        area,
        query::history_total_lines(app.state()),
        query::history_viewport_rows(app.state()),
        query::history_scroll_position(app.state()),
        accent,
    );
}

fn visible_entries(app: &App) -> Vec<(usize, VisibleEntry<'_>)> {
    query::entries(app.state())
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match entry {
            TranscriptEntry::Message(message) => Some((index, VisibleEntry::Message(message))),
            TranscriptEntry::ProposedPlan(plan) => Some((index, VisibleEntry::ProposedPlan(plan))),
            TranscriptEntry::ToolCall(tool_call) => {
                Some((index, VisibleEntry::ToolCall(tool_call)))
            }
            TranscriptEntry::ToolResult(tool_result) => query::show_tool_output(app.state())
                .then_some((index, VisibleEntry::ToolResult(tool_result))),
            TranscriptEntry::HostedToolStatus(status) => {
                Some((index, VisibleEntry::HostedToolStatus(status)))
            }
            TranscriptEntry::TodoSnapshot(snapshot) => {
                Some((index, VisibleEntry::TodoSnapshot(snapshot)))
            }
            TranscriptEntry::SubagentStatus(status) => {
                Some((index, VisibleEntry::SubagentStatus(status)))
            }
            TranscriptEntry::BackgroundTerminalStatus(status) => {
                Some((index, VisibleEntry::BackgroundTerminalStatus(status)))
            }
        })
        .collect()
}

fn tool_activity_run_end(entries: &[(usize, VisibleEntry<'_>)], start: usize) -> usize {
    let mut end = start;
    while end < entries.len() && entries[end].1.is_tool_activity() {
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
        VisibleEntry::ProposedPlan(plan) => {
            let message = ChatMessage {
                speaker: crate::app::Speaker::Agent,
                text: plan.markdown.clone(),
                style: MessageStyle::Plain,
                tag: None,
            };
            push_message_lines(lines, &message, width, accent);
        }
        VisibleEntry::ToolCall(tool_call) => push_tool_call_lines(lines, tool_call, width),
        VisibleEntry::ToolResult(tool_result) => push_tool_result_lines(lines, tool_result, width),
        VisibleEntry::HostedToolStatus(status) => {
            push_hosted_tool_status_lines(lines, status, width)
        }
        VisibleEntry::TodoSnapshot(snapshot) => {
            push_todo_snapshot_lines(lines, snapshot, width, accent)
        }
        VisibleEntry::SubagentStatus(status) => push_subagent_status_lines(lines, status, width),
        VisibleEntry::BackgroundTerminalStatus(status) => {
            push_background_terminal_status_lines(lines, status, width)
        }
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
    entries: &[(usize, VisibleEntry<'_>)],
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

    for (_, entry) in entries.iter().skip(hidden_count).copied() {
        match entry {
            VisibleEntry::ToolCall(tool_call) => push_tool_call_lines(lines, tool_call, width),
            VisibleEntry::ToolResult(tool_result) => {
                push_tool_result_lines(lines, tool_result, width)
            }
            VisibleEntry::HostedToolStatus(status) => {
                push_hosted_tool_status_lines(lines, status, width)
            }
            VisibleEntry::TodoSnapshot(_) => {
                unreachable!("todo snapshots are rendered outside tool activity runs")
            }
            VisibleEntry::SubagentStatus(status) => {
                push_subagent_status_lines(lines, status, width)
            }
            VisibleEntry::BackgroundTerminalStatus(status) => {
                push_background_terminal_status_lines(lines, status, width)
            }
            VisibleEntry::ProposedPlan(_) => {}
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
        app::{Action, Effect, StreamEvent},
        config::ReasoningEffort,
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

    #[test]
    fn scrollbar_thumb_reaches_bottom_at_max_scroll() {
        let (start, len) = scrollbar_thumb_bounds(10, 30, 6, 24);
        assert_eq!(start + len, 10);
    }

    #[test]
    fn scrollbar_thumb_size_stays_constant_while_scrolling() {
        assert_eq!(scrollbar_thumb_bounds(10, 30, 10, 0).1, 3);
        assert_eq!(scrollbar_thumb_bounds(10, 30, 10, 10).1, 3);
        assert_eq!(scrollbar_thumb_bounds(10, 30, 10, 20).1, 3);
    }
}
