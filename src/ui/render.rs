use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use tui_markdown::from_str as markdown_from_str;

use crate::app::{
    App, ChatMessage, MessageStyle, SlashCommand, Speaker, ToolCall, ToolResultEntry,
    TranscriptEntry,
};

use super::theme::accent_color;

const LOADING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_VISIBLE_TOOL_ACTIVITY: usize = 5;

pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let accent = accent_color(app.mode());
    let input_height = app.composer_height().max(3);
    let command_height = app.command_palette_height();
    let mut constraints = vec![Constraint::Min(1)];
    if command_height > 0 {
        constraints.push(Constraint::Length(command_height));
    }
    constraints.push(Constraint::Length(input_height));
    constraints.push(Constraint::Length(1));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(screen);

    let mut section = 0;
    render_history(frame, app, layout[section], accent);
    section += 1;
    if command_height > 0 {
        render_command_palette(frame, app, layout[section], accent);
        section += 1;
    }
    render_input(frame, app, layout[section], accent);
    render_mode(frame, app, layout[section + 1], accent);
}

fn render_history(frame: &mut Frame, app: &mut App, area: Rect, accent: Color) {
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
            loading_frame(app),
        );
    }

    let visible_count = content_area.height as usize;
    let start = app.sync_history_viewport(lines.len(), visible_count);
    let visible_lines = history_viewport_lines(lines, start, visible_count);
    let history = Paragraph::new(visible_lines);
    frame.render_widget(history, content_area);

    if show_scrollbar && app.history_total_lines() > app.history_viewport_rows() {
        render_history_scrollbar(frame, history_layout[1], app, accent);
    }
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

fn scrollbar_thumb_bounds(
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

fn render_input(frame: &mut Frame, app: &mut App, area: Rect, accent: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(Style::default().fg(accent));
    app.composer_mut().set_block(block);
    app.composer_mut().set_cursor_line_style(Style::default());
    app.composer_mut()
        .set_cursor_style(Style::default().bg(accent).fg(Color::Black));
    app.composer_mut()
        .set_placeholder_style(Style::default().fg(Color::DarkGray));
    frame.render_widget(app.composer(), area);
}

fn render_command_palette(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let commands = app.filtered_commands();
    let selected = app.selected_command();
    let lines = if commands.is_empty() {
        vec![Line::from(Span::styled(
            "No matching commands",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        commands
            .into_iter()
            .take(visible_rows)
            .map(|command| command_palette_line(command, selected, accent))
            .collect()
    };

    let palette = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Commands ")
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(accent)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(palette, area);
}

fn render_mode(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let mode = Paragraph::new(Line::from(vec![
        Span::styled(
            app.mode().label(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {}  Effort {}  Thinking {}  Tool output {}  {}  / commands  Tab toggle  Ctrl+C clear/quit",
            app.model_name(),
            app.reasoning_effort().as_str(),
            if app.show_thinking() {
                "visible"
            } else {
                "hidden"
            },
            if app.show_tool_output() {
                "visible"
            } else {
                "hidden"
            },
            app.history_status_label(),
        )),
    ]));
    frame.render_widget(mode, area);
}

fn command_palette_line(
    command: SlashCommand,
    selected: Option<SlashCommand>,
    accent: Color,
) -> Line<'static> {
    let is_selected = Some(command) == selected;
    let marker = if is_selected { "›" } else { " " };
    let name_style = if is_selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    let mut spans = vec![
        Span::styled(marker, name_style),
        Span::raw(" "),
        Span::styled(command.canonical_name(), name_style),
        Span::raw("  "),
        Span::styled(command.description(), Style::default().fg(Color::Gray)),
    ];
    if !command.aliases().is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("aliases: {}", command.aliases().join(", ")),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}

fn push_message_lines(
    lines: &mut Vec<Line<'static>>,
    message: &ChatMessage,
    width: usize,
    accent: Color,
) {
    if should_render_markdown(message) {
        push_markdown_message_lines(lines, message, width, accent);
        return;
    }

    push_plain_message_lines(lines, message, width, accent);
}

fn push_plain_message_lines(
    lines: &mut Vec<Line<'static>>,
    message: &ChatMessage,
    width: usize,
    accent: Color,
) {
    let prefix_text = prefix_text(message.speaker);
    let content_width = width.saturating_sub(prefix_width(message.speaker)).max(1);
    let wrapped = wrap_text(&message.text, content_width);
    let body_style = message_style(message.style);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            let (marker, label_style) = prefix_marker(message.speaker, accent);
            lines.push(Line::from(vec![
                Span::styled(marker, label_style),
                Span::raw(" "),
                Span::styled(prefix_text.clone(), label_style),
                Span::raw("  "),
                Span::styled(chunk, body_style),
            ]));
        } else {
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(prefix_width(message.speaker))),
                Span::styled(chunk, body_style),
            ]));
        }
    }
}

fn push_markdown_message_lines(
    lines: &mut Vec<Line<'static>>,
    message: &ChatMessage,
    width: usize,
    accent: Color,
) {
    let content_width = width.saturating_sub(prefix_width(message.speaker)).max(1);
    let wrapped = wrap_styled_lines(markdown_lines(&message.text), content_width);
    push_prefixed_styled_lines(lines, wrapped, message.speaker, accent);
}

fn should_render_markdown(message: &ChatMessage) -> bool {
    message.speaker == Speaker::Agent && message.style == MessageStyle::Plain
}

fn markdown_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines = markdown_from_str(text)
        .lines
        .into_iter()
        .map(into_owned_line)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        lines.push(Line::default());
    }

    lines
}

fn into_owned_line(line: Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line.spans.into_iter().map(into_owned_span).collect(),
    }
}

fn into_owned_span(span: Span<'_>) -> Span<'static> {
    Span::styled(span.content.into_owned(), span.style)
}

fn push_prefixed_styled_lines(
    lines: &mut Vec<Line<'static>>,
    body_lines: Vec<Line<'static>>,
    speaker: Speaker,
    accent: Color,
) {
    let prefix_text = prefix_text(speaker);
    let prefix_padding = " ".repeat(prefix_width(speaker));

    for (index, body_line) in body_lines.into_iter().enumerate() {
        let mut spans = Vec::new();
        if index == 0 {
            let (marker, label_style) = prefix_marker(speaker, accent);
            spans.push(Span::styled(marker, label_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(prefix_text.clone(), label_style));
            spans.push(Span::raw("  "));
        } else {
            spans.push(Span::raw(prefix_padding.clone()));
        }
        spans.extend(body_line.spans);

        lines.push(Line {
            style: body_line.style,
            alignment: body_line.alignment,
            spans,
        });
    }
}

fn push_pending_lines(
    lines: &mut Vec<Line<'static>>,
    width: usize,
    accent: Color,
    frame_text: &str,
) {
    let pending = format!("{frame_text} thinking");
    let message = ChatMessage {
        speaker: Speaker::Agent,
        text: pending,
        style: MessageStyle::Thinking,
    };
    push_message_lines(lines, &message, width, accent);
}

fn push_tool_call_lines(lines: &mut Vec<Line<'static>>, tool_call: &ToolCall, width: usize) {
    let prefix = "◇ tool";
    let body = format!("{}  {}", tool_call.name, tool_call.parameter);
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);
    let wrapped = wrap_text(&body, content_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(chunk, Style::default().fg(Color::Gray)),
            ]));
        } else {
            lines.push(Line::from(format!(
                "{}{}",
                " ".repeat(prefix.chars().count() + 2),
                chunk
            )));
        }
    }
}

fn push_tool_result_lines(
    lines: &mut Vec<Line<'static>>,
    tool_result: &ToolResultEntry,
    width: usize,
) {
    let prefix = "↳ result";
    let body = format!("{}  {}", tool_result.name, tool_result.output);
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);
    let wrapped = wrap_text(&body, content_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(chunk, Style::default().fg(Color::Gray)),
            ]));
        } else {
            lines.push(Line::from(format!(
                "{}{}",
                " ".repeat(prefix.chars().count() + 2),
                chunk
            )));
        }
    }
}

fn prefix_text(speaker: Speaker) -> String {
    speaker.label().to_string()
}

fn prefix_width(speaker: Speaker) -> usize {
    1 + 1 + speaker.label().chars().count() + 2
}

fn prefix_marker(speaker: Speaker, accent: Color) -> (&'static str, Style) {
    match speaker {
        Speaker::User => (
            "●",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Speaker::Agent => ("◦", Style::default().fg(Color::Gray)),
    }
}

fn message_style(style: MessageStyle) -> Style {
    match style {
        MessageStyle::Plain => Style::default(),
        MessageStyle::Thinking => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
        MessageStyle::Error => Style::default().fg(Color::Red),
    }
}

fn loading_frame(app: &App) -> &'static str {
    LOADING_FRAMES[app.tick_count() % LOADING_FRAMES.len()]
}

fn wrap_styled_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut wrapped = Vec::new();

    for line in lines {
        wrap_styled_line(line, width, &mut wrapped);
    }

    if wrapped.is_empty() {
        wrapped.push(Line::default());
    }

    wrapped
}

fn wrap_styled_line(line: Line<'static>, width: usize, wrapped: &mut Vec<Line<'static>>) {
    if line.spans.is_empty() {
        wrapped.push(Line {
            style: line.style,
            alignment: line.alignment,
            spans: Vec::new(),
        });
        return;
    }

    let mut current = Vec::new();
    let mut current_width = 0;

    for span in line.spans {
        for segment in split_preserving_whitespace(span.content.as_ref()) {
            push_styled_segment(
                segment,
                span.style,
                width,
                line.style,
                line.alignment,
                &mut current,
                &mut current_width,
                wrapped,
            );
        }
    }

    if !current.is_empty() {
        wrapped.push(Line {
            style: line.style,
            alignment: line.alignment,
            spans: current,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn push_styled_segment(
    segment: String,
    style: Style,
    width: usize,
    line_style: Style,
    alignment: Option<ratatui::layout::Alignment>,
    current: &mut Vec<Span<'static>>,
    current_width: &mut usize,
    wrapped: &mut Vec<Line<'static>>,
) {
    if segment.is_empty() {
        return;
    }

    let segment_width = segment.chars().count();
    if *current_width + segment_width <= width {
        current.push(Span::styled(segment, style));
        *current_width += segment_width;
        return;
    }

    if !current.is_empty() {
        wrapped.push(Line {
            style: line_style,
            alignment,
            spans: std::mem::take(current),
        });
        *current_width = 0;
    }

    if segment_width <= width {
        current.push(Span::styled(segment, style));
        *current_width = segment_width;
        return;
    }

    let mut chunk = String::new();
    for ch in segment.chars() {
        chunk.push(ch);
        if chunk.chars().count() == width {
            wrapped.push(Line {
                style: line_style,
                alignment,
                spans: vec![Span::styled(std::mem::take(&mut chunk), style)],
            });
        }
    }

    if !chunk.is_empty() {
        *current_width = chunk.chars().count();
        current.push(Span::styled(chunk, style));
    }
}

fn split_preserving_whitespace(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut current_is_whitespace = None;

    for ch in text.chars() {
        let is_whitespace = ch.is_whitespace();
        match current_is_whitespace {
            Some(value) if value == is_whitespace => current.push(ch),
            Some(_) => {
                segments.push(std::mem::take(&mut current));
                current.push(ch);
                current_is_whitespace = Some(is_whitespace);
            }
            None => {
                current.push(ch);
                current_is_whitespace = Some(is_whitespace);
            }
        }
    }

    if !current.is_empty() {
        segments.push(current);
    }

    segments
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let paragraphs: Vec<&str> = text.split('\n').collect();

    for paragraph in paragraphs {
        if paragraph.is_empty() {
            lines.push(String::new());
        } else {
            wrap_paragraph(paragraph, width, &mut lines);
        }
    }

    if lines.is_empty() {
        lines.push(String::new());
    }

    lines
}

fn wrap_paragraph(paragraph: &str, width: usize, lines: &mut Vec<String>) {
    let mut current = String::new();

    for word in paragraph.split_whitespace() {
        let word_len = word.chars().count();

        if current.is_empty() {
            if word_len <= width {
                current.push_str(word);
            } else {
                push_split_word(word, width, lines, &mut current);
            }
            continue;
        }

        let candidate_len = current.chars().count() + 1 + word_len;
        if candidate_len <= width {
            current.push(' ');
            current.push_str(word);
            continue;
        }

        lines.push(std::mem::take(&mut current));
        if word_len <= width {
            current.push_str(word);
        } else {
            push_split_word(word, width, lines, &mut current);
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
}

fn push_split_word(word: &str, width: usize, lines: &mut Vec<String>, current: &mut String) {
    let mut chunk = String::new();

    for ch in word.chars() {
        chunk.push(ch);
        if chunk.chars().count() == width {
            lines.push(std::mem::take(&mut chunk));
        }
    }

    if !chunk.is_empty() {
        current.push_str(&chunk);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use crate::{app::Action, config::ReasoningEffort};

    use super::*;

    #[test]
    fn wrap_text_respects_width_and_newlines() {
        assert_eq!(wrap_text("", 4), vec![String::new()]);
        assert_eq!(wrap_text("abcde", 2), vec!["ab", "cd", "e"]);
        assert_eq!(wrap_text("ab\ncd", 2), vec!["ab", "cd"]);
    }

    #[test]
    fn wrap_text_keeps_punctuation_with_the_word_before_it() {
        assert_eq!(wrap_text("flight style .", 13), vec!["flight style", "."]);
        assert_eq!(wrap_text("flight style.", 13), vec!["flight style."]);
    }

    #[test]
    fn render_shows_mode_line_and_initial_prompt() {
        let backend = TestBackend::new(120, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Loaded Azure model"));
        assert!(rendered.contains("Effort medium"));
        assert!(rendered.contains("Read-only"));
        assert!(rendered.contains("Thinking visible"));
        assert!(rendered.contains("Tool output hidden"));
        assert!(rendered.contains("History live"));

        app.apply(Action::ToggleMode);
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Read-Write"));
    }

    #[test]
    fn message_style_marks_thinking_as_italic() {
        let style = message_style(MessageStyle::Thinking);
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn render_shows_tool_calls_and_results() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show tools");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src","recursive":true}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolResult {
                name: "List".into(),
                output: "src/\nsrc/main.rs".into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("◇ tool"));
        assert!(rendered.contains("↳ result"));
        assert!(rendered.contains("recursive"));
        assert!(rendered.contains("src/main.rs"));
    }

    #[test]
    fn render_hides_tool_results_when_config_disables_them() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show tools");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src","recursive":true}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolResult {
                name: "List".into(),
                output: "src/\nsrc/main.rs".into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("◇ tool"));
        assert!(!rendered.contains("↳ result"));
        assert!(!rendered.contains("src/main.rs"));
    }

    #[test]
    fn render_collapses_long_tool_runs_to_the_last_five_entries() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show tools");
        app.apply(Action::SubmitMessage);

        for index in 1..=6 {
            app.apply(Action::StreamEvent {
                reply_id: 1,
                event: crate::llm::StreamEvent::ToolCall {
                    name: format!("List{index}"),
                    arguments: format!(r#"{{"dir":"src/{index}"}}"#),
                },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("... 1 more tool calls"));
        assert!(!rendered.contains(r#"src/1"#));
        for index in 2..=6 {
            assert!(rendered.contains(&format!(r#"src/{index}"#)));
        }
    }

    #[test]
    fn render_ignores_hidden_tool_results_when_collapsing_runs() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show tools");
        app.apply(Action::SubmitMessage);

        for index in 1..=6 {
            app.apply(Action::StreamEvent {
                reply_id: 1,
                event: crate::llm::StreamEvent::ToolCall {
                    name: format!("List{index}"),
                    arguments: format!(r#"{{"dir":"src/{index}"}}"#),
                },
            });
            app.apply(Action::StreamEvent {
                reply_id: 1,
                event: crate::llm::StreamEvent::ToolResult {
                    name: format!("List{index}"),
                    output: format!("hidden result {index}"),
                },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("... 1 more tool calls"));
        assert!(!rendered.contains("hidden result"));
    }

    #[test]
    fn render_collapses_each_tool_run_independently() {
        let backend = TestBackend::new(100, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show tools");
        app.apply(Action::SubmitMessage);

        for index in 1..=6 {
            app.apply(Action::StreamEvent {
                reply_id: 1,
                event: crate::llm::StreamEvent::ToolCall {
                    name: format!("First{index}"),
                    arguments: format!(r#"{{"dir":"first/{index}"}}"#),
                },
            });
        }

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("separator".into()),
        });

        for index in 1..=6 {
            app.apply(Action::StreamEvent {
                reply_id: 1,
                event: crate::llm::StreamEvent::ToolCall {
                    name: format!("Second{index}"),
                    arguments: format!(r#"{{"dir":"second/{index}"}}"#),
                },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert_eq!(rendered.matches("... 1 more tool calls").count(), 2);
        assert!(rendered.contains("separator"));
    }

    #[test]
    fn render_formats_markdown_lists_for_agent_messages() {
        let backend = TestBackend::new(80, 14);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render list");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("- first item\n- second item".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let lines = buffer_lines(terminal.backend());
        let first_row = lines
            .iter()
            .position(|line| line.contains("first item"))
            .expect("first list item row");
        let second_row = lines
            .iter()
            .position(|line| line.contains("second item"))
            .expect("second list item row");

        assert!(second_row > first_row);
    }

    #[test]
    fn render_preserves_markdown_bold_and_italic_modifiers() {
        let backend = TestBackend::new(100, 14);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render emphasis");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("**bold** and *italic*".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let buffer = terminal.backend().buffer();
        assert!(
            word_has_modifier(buffer, "bold", Modifier::BOLD),
            "expected bold word to render with bold modifier"
        );
        assert!(
            word_has_modifier(buffer, "italic", Modifier::ITALIC),
            "expected italic word to render with italic modifier"
        );
    }

    #[test]
    fn render_input_does_not_underline_the_cursor_line() {
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("draft");

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let buffer = terminal.backend().buffer();
        assert!(
            !word_has_modifier(buffer, "draft", Modifier::UNDERLINED),
            "expected input text not to render with underline"
        );
    }

    #[test]
    fn render_scrollback_reveals_older_messages() {
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        for index in 1..=8 {
            app.composer_mut().insert_str(&format!("message {index}"));
            app.apply(Action::SubmitMessage);
            app.apply(Action::StreamEvent {
                reply_id: index as u64,
                event: crate::llm::StreamEvent::Finished { history: None },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("message 8"));
        assert!(!rendered.contains("message 4"));

        app.apply(Action::ScrollHistoryPageUp);
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("message 6"));
        assert!(!rendered.contains("message 8"));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn render_home_and_end_jump_history_viewport() {
        let backend = TestBackend::new(100, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        for index in 1..=8 {
            app.composer_mut().insert_str(&format!("entry {index}"));
            app.apply(Action::SubmitMessage);
            app.apply(Action::StreamEvent {
                reply_id: index as u64,
                event: crate::llm::StreamEvent::Finished { history: None },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        app.apply(Action::ScrollHistoryToTop);
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("entry 1"));
        assert!(!rendered.contains("entry 8"));

        app.apply(Action::ScrollHistoryToBottom);
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("entry 8"));
        assert!(!rendered.contains("entry 1"));
        assert!(rendered.contains("History live"));
    }

    #[test]
    fn render_keeps_pinned_history_stable_while_streaming() {
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("start");
        app.apply(Action::SubmitMessage);
        let initial_items = (1..=12)
            .map(|index| format!("- item {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta(initial_items),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        app.apply(Action::ScrollHistoryToTop);
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let before = buffer_string(terminal.backend());
        assert!(before.contains("item 1"));
        assert!(!before.contains("item 12"));
        assert!(app.history_is_pinned());

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("\n- item 13\n- item 14".into()),
        });
        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");
        let after = buffer_string(terminal.backend());
        assert!(after.contains("item 1"));
        assert!(!after.contains("item 14"));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn render_draws_accented_scrollbar_for_overflowing_history() {
        let backend = TestBackend::new(70, 10);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        for index in 1..=10 {
            app.composer_mut().insert_str(&format!("entry {index}"));
            app.apply(Action::SubmitMessage);
            app.apply(Action::StreamEvent {
                reply_id: index as u64,
                event: crate::llm::StreamEvent::Finished { history: None },
            });
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let buffer = terminal.backend().buffer();
        let width = buffer.area.width as usize;
        let right_column = buffer
            .content
            .chunks(width)
            .map(|row| &row[width - 1])
            .collect::<Vec<_>>();

        assert!(
            right_column.iter().any(|cell| cell.bg == Color::Magenta),
            "expected scrollbar thumb in the rightmost column"
        );
        assert!(
            right_column
                .iter()
                .filter(|cell| cell.bg == Color::Magenta)
                .all(|cell| cell.symbol() == " "),
            "expected scrollbar thumb to use the accent color"
        );
    }

    #[test]
    fn scrollbar_thumb_reaches_bottom_at_max_scroll() {
        let (start, len) = scrollbar_thumb_bounds(10, 30, 6, 24);

        assert_eq!(start + len, 10);
    }

    #[test]
    fn scrollbar_thumb_size_stays_constant_while_scrolling() {
        let positions = [0, 3, 6, 9, 12, 15, 18, 21, 24];
        let lengths = positions
            .into_iter()
            .map(|position| scrollbar_thumb_bounds(10, 30, 6, position).1)
            .collect::<Vec<_>>();

        assert!(lengths.iter().all(|length| *length == lengths[0]));
    }

    fn buffer_string(backend: &TestBackend) -> String {
        backend
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn buffer_lines(backend: &TestBackend) -> Vec<String> {
        let buffer = backend.buffer();
        let width = buffer.area.width as usize;
        buffer
            .content
            .chunks(width)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect()
    }

    fn word_has_modifier(buffer: &ratatui::buffer::Buffer, word: &str, modifier: Modifier) -> bool {
        let width = buffer.area.width as usize;
        let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

        for row in buffer.content.chunks(width) {
            for start in 0..=row.len().saturating_sub(symbols.len()) {
                if row[start..start + symbols.len()]
                    .iter()
                    .map(|cell| cell.symbol())
                    .eq(symbols.iter().map(String::as_str))
                {
                    return row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.modifier.contains(modifier));
                }
            }
        }

        false
    }
}
