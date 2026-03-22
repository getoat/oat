use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};
use serde::Deserialize;
use tui_markdown::from_str as markdown_from_str;

use crate::app::{
    App, ChatMessage, MessageStyle, SlashCommand, Speaker, ToolCall, ToolResultEntry,
    TranscriptEntry,
};

use super::theme::accent_color;

const LOADING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MAX_VISIBLE_TOOL_ACTIVITY: usize = 5;
const CODE_BLOCK_HORIZONTAL_PADDING: usize = 1;
pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let accent = accent_color(app.mode());
    let input_height = if let Some(pending) = app.pending_write_approval() {
        pending_write_approval_height(pending, screen.width)
    } else {
        app.composer_height().max(3)
    };
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
    let mut visible_lines = history_viewport_lines(lines, start, visible_count);
    let history_snapshot = visible_lines
        .iter()
        .map(rendered_line_text)
        .collect::<Vec<_>>();
    app.update_history_snapshot(content_area, history_snapshot);
    apply_history_selection_highlight(&mut visible_lines, app, accent);
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

#[derive(Debug, PartialEq, Eq)]
enum MarkdownSegment {
    Markdown(String),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
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
    if let Some(pending) = app.pending_write_approval() {
        render_write_approval_prompt(frame, pending, area, accent);
        return;
    }

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

fn render_write_approval_prompt(
    frame: &mut Frame,
    pending: &crate::app::PendingWriteApproval,
    area: Rect,
    accent: Color,
) {
    let mut lines = vec![Line::from(Span::styled(
        approval_intent_summary(pending),
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    ))];

    lines.push(Line::from(vec![
        Span::styled(
            "[a]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" allow once"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[s]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" allow all this session"),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[d]",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" deny"),
    ]));

    let prompt = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .title(" Write Approval Required ")
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .border_style(Style::default().fg(Color::Yellow)),
    );
    frame.render_widget(prompt, area);
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
    let mode_label = mode_status_label(app.mode(), app.write_approval_policy());

    let mut spans = vec![
        Span::styled(
            mode_label,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {} • {}",
            app.model_name(),
            app.reasoning_effort().as_str(),
        )),
    ];

    if let Some(pending) = app.pending_write_approval() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("Approval pending: {}", pending.tool_name),
            Style::default().fg(Color::Yellow),
        ));
    } else {
        let hint = match app.mode() {
            crate::app::AccessMode::ReadOnly => "  Tab switches to write mode for edits",
            crate::app::AccessMode::ReadWrite => "",
        };
        if !hint.is_empty() {
            spans.push(Span::styled(hint, Style::default().fg(Color::Gray)));
        }
    }

    let mode = Paragraph::new(Line::from(spans));
    frame.render_widget(mode, area);
}

fn mode_status_label(
    mode: crate::app::AccessMode,
    policy: crate::app::WriteApprovalPolicy,
) -> &'static str {
    match (mode, policy) {
        (crate::app::AccessMode::ReadWrite, crate::app::WriteApprovalPolicy::AllowAllSession) => {
            "Write (!)"
        }
        _ => mode.label(),
    }
}

fn pending_write_approval_height(
    pending: &crate::app::PendingWriteApproval,
    panel_width: u16,
) -> u16 {
    let content_width = panel_width.saturating_sub(4) as usize;
    let summary_lines = wrap_text(&approval_intent_summary(pending), content_width.max(1)).len();
    (summary_lines + 3 + 2) as u16
}

fn approval_intent_summary(pending: &crate::app::PendingWriteApproval) -> String {
    if let Some(preview) = mutation_preview(&pending.tool_name, &pending.arguments) {
        if let Some(summary) = preview.summary.as_ref() {
            return summary.clone();
        }

        return missing_intent_summary(&pending.tool_name, &preview.target);
    }

    "No reason provided for this write request".to_string()
}

fn missing_intent_summary(tool_name: &str, target: &str) -> String {
    match tool_name {
        "ApplyPatches" => format!("No reason provided for changing {target}"),
        "WriteFile" => format!("No reason provided for creating {target}"),
        "DeletePath" => format!("No reason provided for deleting {target}"),
        _ => "No reason provided for this write request".to_string(),
    }
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
    let rendered = render_markdown_message_lines(&message.text, content_width);
    push_prefixed_styled_lines(lines, rendered, message.speaker, accent);
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

fn render_markdown_message_lines(text: &str, content_width: usize) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();

    for segment in markdown_segments(text) {
        match segment {
            MarkdownSegment::Markdown(markdown) => {
                rendered.extend(wrap_styled_lines(markdown_lines(&markdown), content_width));
            }
            MarkdownSegment::CodeBlock { language, code } => {
                rendered.extend(render_code_block_lines(
                    language.as_deref(),
                    &code,
                    content_width,
                ));
            }
        }
    }

    if rendered.is_empty() {
        rendered.push(Line::default());
    }

    rendered
}

fn markdown_segments(text: &str) -> Vec<MarkdownSegment> {
    let mut segments = Vec::new();
    let mut markdown = String::new();
    let mut code = String::new();
    let mut language = None;
    let mut in_code_block = false;

    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);

        if in_code_block {
            if is_closing_code_fence(line) {
                segments.push(MarkdownSegment::CodeBlock {
                    language: language.take(),
                    code: std::mem::take(&mut code),
                });
                in_code_block = false;
            } else {
                code.push_str(raw_line);
            }
            continue;
        }

        if let Some(next_language) = opening_code_fence_language(line) {
            if !markdown.is_empty() {
                segments.push(MarkdownSegment::Markdown(std::mem::take(&mut markdown)));
            }
            language = next_language;
            in_code_block = true;
        } else {
            markdown.push_str(raw_line);
        }
    }

    if in_code_block {
        return vec![MarkdownSegment::Markdown(text.to_string())];
    }

    if !markdown.is_empty() {
        segments.push(MarkdownSegment::Markdown(markdown));
    }

    if segments.is_empty() {
        segments.push(MarkdownSegment::Markdown(String::new()));
    }

    segments
}

fn opening_code_fence_language(line: &str) -> Option<Option<String>> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }

    let rest = &trimmed[3..];
    if rest.starts_with('`') {
        return None;
    }

    let language = rest.trim();
    Some((!language.is_empty()).then(|| language.to_string()))
}

fn is_closing_code_fence(line: &str) -> bool {
    line.trim() == "```"
}

fn render_code_block_lines(
    language: Option<&str>,
    code: &str,
    content_width: usize,
) -> Vec<Line<'static>> {
    let inner_width = content_width
        .saturating_sub(CODE_BLOCK_HORIZONTAL_PADDING * 2)
        .max(1);
    let mut block_lines = Vec::new();

    if let Some(language) = language.filter(|language| !language.is_empty()) {
        let header = Line::from(Span::styled(
            language.to_string(),
            code_block_header_style(),
        ));
        block_lines.extend(wrap_styled_lines(vec![header], inner_width));
    }

    let body = wrap_styled_lines(code_block_body_lines(code, language), inner_width);
    block_lines.extend(body);

    if block_lines.is_empty() {
        block_lines.push(Line::default());
    }

    let target_width = block_lines
        .iter()
        .map(rendered_line_width)
        .max()
        .unwrap_or(0);

    block_lines
        .into_iter()
        .map(|line| decorate_code_block_line(line, target_width))
        .collect()
}

fn code_block_body_lines(code: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let mut lines = markdown_lines(&fenced_code_block_markdown(
        code,
        normalized_highlight_language(language),
    ));
    strip_outer_code_fences(&mut lines);

    if lines.is_empty() {
        lines.push(Line::default());
    }

    lines
}

fn fenced_code_block_markdown(code: &str, language: Option<&str>) -> String {
    let mut markdown = String::from("```");
    if let Some(language) = language.filter(|language| !language.is_empty()) {
        markdown.push_str(language);
    }
    markdown.push('\n');
    markdown.push_str(code);
    if !code.ends_with('\n') {
        markdown.push('\n');
    }
    markdown.push_str("```");
    markdown
}

fn normalized_highlight_language(language: Option<&str>) -> Option<&str> {
    let language = language?.trim();
    if language.is_empty() {
        return None;
    }

    match language.to_ascii_lowercase().as_str() {
        "c#" | "csharp" | "c-sharp" | "c_sharp" | "c sharp" => Some("C#"),
        _ => Some(language),
    }
}

fn strip_outer_code_fences(lines: &mut Vec<Line<'static>>) {
    let should_strip_first = lines.first().is_some_and(is_opening_code_fence_line);
    let should_strip_last = lines
        .last()
        .is_some_and(is_closing_code_fence_line_rendered);

    if should_strip_first {
        lines.remove(0);
    }
    if should_strip_last && !lines.is_empty() {
        lines.pop();
    }
}

fn is_opening_code_fence_line(line: &Line<'_>) -> bool {
    rendered_line_text(line).starts_with("```")
}

fn is_closing_code_fence_line_rendered(line: &Line<'_>) -> bool {
    rendered_line_text(line).trim() == "```"
}

fn rendered_line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn rendered_line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum()
}

fn decorate_code_block_line(line: Line<'static>, target_width: usize) -> Line<'static> {
    let base_style = code_block_style();
    let mut spans = Vec::with_capacity(line.spans.len() + 2);
    let padding = " ".repeat(CODE_BLOCK_HORIZONTAL_PADDING);
    let line_width = rendered_line_width(&line);
    let trailing_padding_width =
        target_width.saturating_sub(line_width) + CODE_BLOCK_HORIZONTAL_PADDING;
    spans.push(Span::styled(padding.clone(), base_style));
    spans.extend(
        line.spans
            .into_iter()
            .map(|span| Span::styled(span.content.into_owned(), base_style.patch(span.style))),
    );
    spans.push(Span::styled(" ".repeat(trailing_padding_width), base_style));

    Line {
        style: base_style.patch(line.style),
        alignment: line.alignment,
        spans,
    }
}

fn code_block_style() -> Style {
    Style::default().fg(Color::White).bg(Color::Black)
}

fn code_block_header_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
        .add_modifier(Modifier::DIM)
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
    if let Some(preview) = mutation_preview(&tool_call.name, &tool_call.parameter) {
        push_mutation_tool_call_lines(lines, prefix, &tool_call.name, &preview, width);
        return;
    }

    let body = format!("{}  {}", tool_call.name, tool_call.parameter);
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);
    let wrapped = wrap_text(&body, content_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    prefix.to_string(),
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

fn push_mutation_tool_call_lines(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    tool_name: &str,
    preview: &MutationPreview,
    width: usize,
) {
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);
    let header = format!("{tool_name}  {}", preview.target);
    let wrapped = wrap_text(&header, content_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    prefix.to_string(),
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

    let indent = " ".repeat(prefix.chars().count() + 2);
    if let Some(summary) = &preview.summary {
        let wrapped = wrap_text(&format!("why: {summary}"), content_width);
        for chunk in wrapped {
            lines.push(Line::from(Span::styled(
                format!("{indent}{chunk}"),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }

    for diff in &preview.lines {
        let wrapped = wrap_text(&format!("{} {}", diff.prefix, diff.text), content_width);
        for (index, chunk) in wrapped.into_iter().enumerate() {
            let text = if index == 0 {
                format!("{indent}{chunk}")
            } else {
                format!("{indent}{chunk}")
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(diff.color),
            )));
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApplyPatchesPreviewArgs {
    filename: String,
    patches: Vec<TextPatchPreview>,
    #[serde(default)]
    intent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TextPatchPreview {
    old_text: String,
    new_text: String,
}

#[derive(Debug, Deserialize)]
struct WriteFilePreviewArgs {
    filename: String,
    content: String,
    #[serde(default)]
    intent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeletePathPreviewArgs {
    path: String,
    #[serde(default)]
    intent: Option<String>,
}

#[derive(Debug)]
struct MutationPreview {
    target: String,
    summary: Option<String>,
    lines: Vec<DiffPreviewLine>,
}

#[derive(Debug)]
struct DiffPreviewLine {
    prefix: char,
    text: String,
    color: Color,
}

fn mutation_preview(tool_name: &str, raw_args: &str) -> Option<MutationPreview> {
    match tool_name {
        "ApplyPatches" => {
            let args: ApplyPatchesPreviewArgs = serde_json::from_str(raw_args).ok()?;
            let mut lines = Vec::new();
            for patch in args.patches {
                lines.extend(diff_lines('-', &patch.old_text, Color::Red));
                lines.extend(diff_lines('+', &patch.new_text, Color::Green));
            }
            Some(MutationPreview {
                target: args.filename,
                summary: normalize_intent(args.intent.as_deref()),
                lines,
            })
        }
        "WriteFile" => {
            let args: WriteFilePreviewArgs = serde_json::from_str(raw_args).ok()?;
            Some(MutationPreview {
                target: args.filename,
                summary: normalize_intent(args.intent.as_deref()),
                lines: diff_lines('+', &args.content, Color::Green),
            })
        }
        "DeletePath" => {
            let args: DeletePathPreviewArgs = serde_json::from_str(raw_args).ok()?;
            Some(MutationPreview {
                target: args.path.clone(),
                summary: normalize_intent(args.intent.as_deref()),
                lines: vec![DiffPreviewLine {
                    prefix: '-',
                    text: args.path,
                    color: Color::Red,
                }],
            })
        }
        _ => None,
    }
}

fn normalize_intent(intent: Option<&str>) -> Option<String> {
    let intent = intent?;
    let normalized = intent.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn diff_lines(prefix: char, text: &str, color: Color) -> Vec<DiffPreviewLine> {
    let lines = if text.is_empty() {
        vec!["(empty)".to_string()]
    } else {
        text.lines().map(ToOwned::to_owned).collect::<Vec<_>>()
    };

    lines
        .into_iter()
        .map(|text| DiffPreviewLine {
            prefix,
            text,
            color,
        })
        .collect()
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
    fn markdown_segments_leave_plain_text_unchanged() {
        assert_eq!(
            markdown_segments("plain text"),
            vec![MarkdownSegment::Markdown("plain text".into())]
        );
    }

    #[test]
    fn markdown_segments_extract_fenced_code_blocks_with_language() {
        assert_eq!(
            markdown_segments("Before\n```rust\nlet value = 1;\n```\nAfter"),
            vec![
                MarkdownSegment::Markdown("Before\n".into()),
                MarkdownSegment::CodeBlock {
                    language: Some("rust".into()),
                    code: "let value = 1;\n".into(),
                },
                MarkdownSegment::Markdown("After".into()),
            ]
        );
    }

    #[test]
    fn markdown_segments_extract_fenced_code_blocks_without_language() {
        assert_eq!(
            markdown_segments("```\nplain text\n```"),
            vec![MarkdownSegment::CodeBlock {
                language: None,
                code: "plain text\n".into(),
            }]
        );
    }

    #[test]
    fn markdown_segments_fall_back_to_plain_markdown_for_unclosed_fences() {
        assert_eq!(
            markdown_segments("Before\n```rust\nlet value = 1;\n"),
            vec![MarkdownSegment::Markdown(
                "Before\n```rust\nlet value = 1;\n".into()
            )]
        );
    }

    #[test]
    fn normalized_highlight_language_maps_csharp_aliases() {
        assert_eq!(normalized_highlight_language(Some("csharp")), Some("C#"));
        assert_eq!(normalized_highlight_language(Some("c#")), Some("C#"));
        assert_eq!(normalized_highlight_language(Some("c sharp")), Some("C#"));
        assert_eq!(normalized_highlight_language(Some("rust")), Some("rust"));
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
        assert!(rendered.contains("Read-only"));
        assert!(rendered.contains("gpt-5-mini • medium"));

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
        assert!(rendered.contains("Write"));
        assert!(!rendered.contains("approvals required"));
    }

    #[test]
    fn mode_status_label_marks_session_preapproved_write_mode() {
        assert_eq!(
            mode_status_label(
                crate::app::AccessMode::ReadWrite,
                crate::app::WriteApprovalPolicy::AskEveryTime,
            ),
            "Write"
        );
        assert_eq!(
            mode_status_label(
                crate::app::AccessMode::ReadWrite,
                crate::app::WriteApprovalPolicy::AllowAllSession,
            ),
            "Write (!)"
        );
    }

    #[test]
    fn pending_write_approval_height_matches_wrapped_summary_lines() {
        let short = crate::app::PendingWriteApproval {
            request_id: "call-1".into(),
            tool_name: "ApplyPatches".into(),
            arguments: "{\"filename\":\"src/lib.rs\",\"patches\":[{\"old_text\":\"a\",\"new_text\":\"b\"}],\"intent\":\"Fix startup\"}".into(),
        };
        assert_eq!(pending_write_approval_height(&short, 120), 6);

        let wrapped = crate::app::PendingWriteApproval {
            request_id: "call-2".into(),
            tool_name: "ApplyPatches".into(),
            arguments: "{\"filename\":\"src/lib.rs\",\"patches\":[{\"old_text\":\"a\",\"new_text\":\"b\"}],\"intent\":\"Fix the broken startup path so the app launches again after config bootstrap changes\"}".into(),
        };
        assert!(pending_write_approval_height(&wrapped, 36) > 6);
    }

    #[test]
    fn render_replaces_input_with_three_line_write_approval_panel() {
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("edit this file");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::WriteApprovalRequested {
                request_id: "call-1".into(),
                tool_name: "ApplyPatches".into(),
                arguments: "{\"filename\":\"src/lib.rs\",\"patches\":[{\"old_text\":\"a\",\"new_text\":\"b\"}],\"intent\":\"Fix the broken startup path so the app launches again\"}".into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        let lines = buffer_lines(terminal.backend());

        assert!(rendered.contains("Fix the broken startup path so the app launches again"));
        assert!(lines.iter().any(|line| line.contains("[a] allow once")));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("[s] allow all this session"))
        );
        assert!(lines.iter().any(|line| line.contains("[d] deny")));
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
    fn render_shows_apply_patches_tool_call_as_diff() {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show patch tool");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolCall {
                name: "ApplyPatches".into(),
                arguments: r#"{"filename":"src/lib.rs","patches":[{"old_text":"old line","new_text":"new line"}],"intent":"Fix the broken startup path so the app launches again"}"#.into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("ApplyPatches"));
        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains("why: Fix the broken startup path so the app launches again"));
        assert!(rendered.contains("- old line"));
        assert!(rendered.contains("+ new line"));
        assert!(word_has_foreground(
            terminal.backend().buffer(),
            "old",
            Color::Red
        ));
        assert!(word_has_foreground(
            terminal.backend().buffer(),
            "new",
            Color::Green
        ));
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
    fn render_hides_fenced_code_markers_for_agent_messages() {
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render code");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("```rust\nlet value = 1;\n```".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(!rendered.contains("```"));
        assert!(rendered.contains("rust"));
        assert!(rendered.contains("let value = 1;"));
    }

    #[test]
    fn render_highlights_active_history_selection() {
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.push_agent_message("alpha beta gamma");

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("initial render succeeds");

        let buffer = terminal.backend().buffer();
        let (row, column) = find_word_position(buffer, "alpha").expect("alpha position");
        app.apply(Action::StartHistorySelection { column, row });
        app.apply(Action::UpdateHistorySelection {
            column: column + 4,
            row,
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("selection render succeeds");

        let selected_row = terminal
            .backend()
            .buffer()
            .content
            .chunks(terminal.backend().buffer().area.width as usize)
            .nth(row as usize)
            .expect("selected row");
        for index in column as usize..=column as usize + 4 {
            assert_eq!(selected_row[index].bg, accent_color(app.mode()));
        }
    }

    #[test]
    fn render_keeps_markdown_formatting_around_code_blocks() {
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render mixed markdown");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta(
                "- first item\n\n```rust\nlet value = 1;\n```\n\n**after**".into(),
            ),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let lines = buffer_lines(terminal.backend());
        let first_row = lines
            .iter()
            .position(|line| line.contains("first item"))
            .expect("first list item row");
        let code_row = lines
            .iter()
            .position(|line| line.contains("let value = 1;"))
            .expect("code row");

        assert!(code_row > first_row);
        assert!(
            word_has_modifier(terminal.backend().buffer(), "after", Modifier::BOLD),
            "expected bold markdown after the code block"
        );
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
    fn render_applies_syntax_highlighting_to_known_code_block_languages() {
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render highlighted code");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("```rust\nlet value = \"hi\";\n```".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let buffer = terminal.backend().buffer();
        assert!(
            word_has_background(buffer, "let", Color::Black),
            "expected code block background for highlighted Rust code"
        );
        assert!(
            word_has_foreground_not(buffer, "let", Color::White),
            "expected syntax-highlighted Rust keyword color"
        );
    }

    #[test]
    fn render_applies_syntax_highlighting_to_csharp_aliases() {
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render csharp code");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta(
                "```csharp\npublic class Demo { }\n```".into(),
            ),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let buffer = terminal.backend().buffer();
        assert!(
            word_has_background(buffer, "public", Color::Black),
            "expected code block background for C# alias"
        );
        assert!(
            word_has_foreground_not(buffer, "public", Color::White),
            "expected syntax-highlighted C# keyword color"
        );
    }

    #[test]
    fn render_styles_unknown_language_code_blocks_without_showing_fences() {
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render plain code");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("```unknownlang\nplain text\n```".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(!rendered.contains("```"));
        assert!(
            word_has_background(terminal.backend().buffer(), "plain", Color::Black),
            "expected fallback code block background"
        );
    }

    #[test]
    fn render_pads_shorter_code_block_lines_to_the_block_width() {
        let backend = TestBackend::new(100, 18);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("render multiline code");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::TextDelta("```text\nalpha\nbetagamma\n```".into()),
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let lines = buffer_lines(terminal.backend());
        let alpha_row = lines
            .iter()
            .position(|line| line.contains("alpha"))
            .expect("alpha row");
        let betagamma_row = lines
            .iter()
            .position(|line| line.contains("betagamma"))
            .expect("betagamma row");

        let buffer = terminal.backend().buffer();
        let alpha_cells = buffer
            .content
            .chunks(buffer.area.width as usize)
            .nth(alpha_row)
            .expect("alpha row cells");
        let betagamma_cells = buffer
            .content
            .chunks(buffer.area.width as usize)
            .nth(betagamma_row)
            .expect("betagamma row cells");
        assert!(
            longest_background_run(alpha_cells, Color::Black)
                >= longest_background_run(betagamma_cells, Color::Black),
            "expected shorter code row background to match the widest line"
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
        assert!(rendered.contains("gpt-5-mini • medium"));
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
                    if row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.modifier.contains(modifier))
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn word_has_background(
        buffer: &ratatui::buffer::Buffer,
        word: &str,
        background: Color,
    ) -> bool {
        let width = buffer.area.width as usize;
        let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

        for row in buffer.content.chunks(width) {
            for start in 0..=row.len().saturating_sub(symbols.len()) {
                if row[start..start + symbols.len()]
                    .iter()
                    .map(|cell| cell.symbol())
                    .eq(symbols.iter().map(String::as_str))
                {
                    if row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.bg == background)
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn word_has_foreground(
        buffer: &ratatui::buffer::Buffer,
        word: &str,
        foreground: Color,
    ) -> bool {
        let width = buffer.area.width as usize;
        let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

        for row in buffer.content.chunks(width) {
            for start in 0..=row.len().saturating_sub(symbols.len()) {
                if row[start..start + symbols.len()]
                    .iter()
                    .map(|cell| cell.symbol())
                    .eq(symbols.iter().map(String::as_str))
                    && row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.fg == foreground)
                {
                    return true;
                }
            }
        }

        false
    }

    fn word_has_foreground_not(
        buffer: &ratatui::buffer::Buffer,
        word: &str,
        foreground: Color,
    ) -> bool {
        let width = buffer.area.width as usize;
        let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

        for row in buffer.content.chunks(width) {
            for start in 0..=row.len().saturating_sub(symbols.len()) {
                if row[start..start + symbols.len()]
                    .iter()
                    .map(|cell| cell.symbol())
                    .eq(symbols.iter().map(String::as_str))
                {
                    if row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.fg != foreground)
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn longest_background_run(row: &[ratatui::buffer::Cell], background: Color) -> usize {
        let mut longest = 0;
        let mut current = 0;

        for cell in row {
            if cell.bg == background {
                current += 1;
                longest = longest.max(current);
            } else {
                current = 0;
            }
        }

        longest
    }

    fn find_word_position(buffer: &ratatui::buffer::Buffer, word: &str) -> Option<(u16, u16)> {
        let width = buffer.area.width as usize;
        let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

        for (row_index, row) in buffer.content.chunks(width).enumerate() {
            for start in 0..=row.len().saturating_sub(symbols.len()) {
                if row[start..start + symbols.len()]
                    .iter()
                    .map(|cell| cell.symbol())
                    .eq(symbols.iter().map(String::as_str))
                {
                    return Some((row_index as u16, start as u16));
                }
            }
        }

        None
    }
}
