use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use tui_markdown::from_str as markdown_from_str;

use crate::app::{App, ChatMessage, MessageStyle, Speaker};

use super::wrap::{wrap_styled_lines, wrap_text};

const CODE_BLOCK_HORIZONTAL_PADDING: usize = 1;
const LOADING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const PROPOSED_PLAN_START_TAG: &str = "<proposed_plan>";
const PROPOSED_PLAN_END_TAG: &str = "</proposed_plan>";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MarkdownSegment {
    Markdown(String),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
}

pub(super) fn push_message_lines(
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

pub(super) fn push_pending_lines(
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

pub(super) fn message_style(style: MessageStyle) -> Style {
    match style {
        MessageStyle::Plain => Style::default(),
        MessageStyle::Thinking => Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::ITALIC),
        MessageStyle::Error => Style::default().fg(Color::Red),
    }
}

pub(super) fn markdown_segments(text: &str) -> Vec<MarkdownSegment> {
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

pub(super) fn normalized_highlight_language(language: Option<&str>) -> Option<&str> {
    let language = language?.trim();
    if language.is_empty() {
        return None;
    }

    match language.to_ascii_lowercase().as_str() {
        "c#" | "csharp" | "c-sharp" | "c_sharp" | "c sharp" => Some("C#"),
        _ => Some(language),
    }
}

pub(super) fn rendered_line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

pub(super) fn loading_frame(app: &App) -> &'static str {
    LOADING_FRAMES[app.tick_count() % LOADING_FRAMES.len()]
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
    let display_text = strip_proposed_plan_tags(&message.text);
    let rendered = render_markdown_message_lines(&display_text, content_width);
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

fn strip_proposed_plan_tags(text: &str) -> String {
    if let Some(inner) = text
        .trim()
        .strip_prefix(PROPOSED_PLAN_START_TAG)
        .and_then(|rest| rest.strip_suffix(PROPOSED_PLAN_END_TAG))
    {
        return inner.trim_matches('\n').to_string();
    }

    let mut stripped = String::new();
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.trim();
        if line == PROPOSED_PLAN_START_TAG || line == PROPOSED_PLAN_END_TAG {
            continue;
        }
        stripped.push_str(raw_line);
    }

    if stripped.is_empty() && !text.is_empty() {
        text.to_string()
    } else {
        stripped
    }
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
