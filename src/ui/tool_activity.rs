use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::{
    app::{ToolCall, ToolResultEntry},
    tools::{DiffKind, MutationPreview},
};

use super::wrap::wrap_text;

pub(super) fn push_tool_call_lines(
    lines: &mut Vec<Line<'static>>,
    tool_call: &ToolCall,
    width: usize,
) {
    let prefix = "◇ tool";
    if let Some(preview) = tool_call.preview.as_ref() {
        push_mutation_tool_call_lines(lines, prefix, &tool_call.name, preview, width);
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

pub(super) fn push_mutation_tool_call_lines(
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
    let gutter_width = diff_gutter_digit_width(preview);
    let blank_gutter = diff_gutter(None, None, gutter_width);
    let diff_content_width = content_width
        .saturating_sub(blank_gutter.chars().count())
        .max(1);
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
        let wrapped = wrap_text(
            &format!("{} {}", diff.prefix, diff.text),
            diff_content_width,
        );
        for (index, chunk) in wrapped.into_iter().enumerate() {
            let gutter = if index == 0 {
                diff_gutter(diff.old_line_number, diff.new_line_number, gutter_width)
            } else {
                blank_gutter.clone()
            };
            lines.push(Line::from(vec![
                Span::raw(indent.clone()),
                Span::styled(gutter, Style::default().fg(Color::DarkGray)),
                Span::styled(chunk, Style::default().fg(diff_kind_color(diff.kind))),
            ]));
        }
    }
}

pub(super) fn push_tool_result_lines(
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

fn diff_kind_color(kind: DiffKind) -> Color {
    match kind {
        DiffKind::Added => Color::Green,
        DiffKind::Removed => Color::Red,
    }
}

fn diff_gutter_digit_width(preview: &MutationPreview) -> usize {
    preview
        .lines
        .iter()
        .flat_map(|line| [line.old_line_number, line.new_line_number])
        .flatten()
        .max()
        .unwrap_or(1)
        .to_string()
        .len()
}

fn diff_gutter(
    old_line_number: Option<usize>,
    new_line_number: Option<usize>,
    width: usize,
) -> String {
    let old = old_line_number
        .map(|line| line.to_string())
        .unwrap_or_default();
    let new = new_line_number
        .map(|line| line.to_string())
        .unwrap_or_default();
    format!("{old:>width$} {new:>width$} | ")
}
