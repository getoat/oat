use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use serde_json::Value;

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
    if tool_call.name == "WebRun" {
        push_web_run_call_lines(lines, tool_call, width);
        return;
    }

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
    if tool_result.name == "WebRun" {
        push_web_run_result_lines(lines, tool_result, width);
        return;
    }

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

fn push_web_run_call_lines(lines: &mut Vec<Line<'static>>, tool_call: &ToolCall, width: usize) {
    let mut call_lines = parse_web_run_call_lines(&tool_call.parameter);
    if call_lines.is_empty() {
        call_lines.push(format!("WebRun  {}", tool_call.parameter));
    }
    push_activity_lines(lines, "◇ web", &call_lines, Color::Yellow, width);
}

fn push_web_run_result_lines(
    lines: &mut Vec<Line<'static>>,
    tool_result: &ToolResultEntry,
    width: usize,
) {
    let mut result_lines = parse_web_run_result_lines(&tool_result.output);
    if result_lines.is_empty() {
        result_lines.push(format!("WebRun  {}", tool_result.output));
    }
    push_activity_lines(lines, "↳ web", &result_lines, Color::DarkGray, width);
}

fn push_activity_lines(
    lines: &mut Vec<Line<'static>>,
    prefix: &str,
    body_lines: &[String],
    prefix_color: Color,
    width: usize,
) {
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);

    for (line_index, body_line) in body_lines.iter().enumerate() {
        let wrapped = wrap_text(body_line, content_width);
        let wrapped = if wrapped.is_empty() {
            vec![String::new()]
        } else {
            wrapped
        };

        for (chunk_index, chunk) in wrapped.into_iter().enumerate() {
            if line_index == 0 && chunk_index == 0 {
                lines.push(Line::from(vec![
                    Span::styled(
                        prefix.to_string(),
                        Style::default()
                            .fg(prefix_color)
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
}

fn parse_web_run_call_lines(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut lines = Vec::new();

    if let Some(searches) = object.get("search_query").and_then(Value::as_array) {
        for search in searches {
            let query = search
                .get("q")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if !query.is_empty() {
                lines.push(format!("search  {query}"));
            }
        }
    }

    if let Some(opens) = object.get("open").and_then(Value::as_array) {
        for open in opens {
            let target = open
                .get("ref_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if target.is_empty() {
                continue;
            }
            let line = open.get("lineno").and_then(Value::as_u64);
            match line {
                Some(line) => lines.push(format!("open  {target} @ L{line}")),
                None => lines.push(format!("open  {target}")),
            }
        }
    }

    if let Some(finds) = object.get("find").and_then(Value::as_array) {
        for find in finds {
            let target = find
                .get("ref_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let pattern = find
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            match (pattern.is_empty(), target.is_empty()) {
                (false, false) => lines.push(format!("find  '{pattern}' in {target}")),
                (false, true) => lines.push(format!("find  '{pattern}'")),
                (true, false) => lines.push(format!("find  {target}")),
                (true, true) => {}
            }
        }
    }

    lines
}

fn parse_web_run_result_lines(raw: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut lines = Vec::new();

    if let Some(searches) = object.get("search_query").and_then(Value::as_array) {
        for search in searches {
            let query = search
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let results = search
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let summary = if query.is_empty() {
                format!("search results  {}", results.len())
            } else {
                format!("search results  {} for {}", results.len(), query)
            };
            lines.push(summary);
            for result in results.iter().take(3) {
                let title = result
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let ref_id = result
                    .get("ref_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let url = result
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let snippet = result
                    .get("snippet")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let mut line = String::new();
                if !ref_id.is_empty() {
                    line.push_str(ref_id);
                    line.push_str("  ");
                }
                if !title.is_empty() {
                    line.push_str(title);
                } else if !url.is_empty() {
                    line.push_str(url);
                }
                if !url.is_empty() && title != url {
                    line.push_str("  ");
                    line.push_str(url);
                }
                if !snippet.is_empty() {
                    line.push_str("  ");
                    line.push_str(snippet);
                }
                if !line.is_empty() {
                    lines.push(line);
                }
            }
        }
    }

    if let Some(opens) = object.get("open").and_then(Value::as_array) {
        for open in opens {
            let ref_id = open
                .get("ref_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let title = open
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let final_url = open
                .get("final_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let start = open
                .get("start_lineno")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            let end = open.get("end_lineno").and_then(Value::as_u64).unwrap_or(0);
            let total = open.get("total_lines").and_then(Value::as_u64).unwrap_or(0);
            let next = open.get("next_lineno").and_then(Value::as_u64);
            let mut summary = String::from("opened page");
            if !ref_id.is_empty() {
                summary.push_str("  ");
                summary.push_str(ref_id);
            }
            if !title.is_empty() {
                summary.push_str("  ");
                summary.push_str(title);
            } else if !final_url.is_empty() {
                summary.push_str("  ");
                summary.push_str(final_url);
            }
            if total > 0 {
                summary.push_str(&format!("  L{start}-{end} of {total}"));
            } else {
                summary.push_str("  empty page");
            }
            if let Some(next) = next {
                summary.push_str(&format!("  next L{next}"));
            }
            lines.push(summary);

            if !final_url.is_empty() && title != final_url {
                lines.push(final_url.to_string());
            }

            if let Some(content) = open.get("content").and_then(Value::as_str) {
                for line in content.lines().take(4) {
                    let trimmed = line.trim_end();
                    if !trimmed.is_empty() {
                        lines.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    if let Some(finds) = object.get("find").and_then(Value::as_array) {
        for find in finds {
            let pattern = find
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let total = find
                .get("total_matches")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let returned = find
                .get("returned_matches")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let url = find
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let mut summary = if pattern.is_empty() {
                format!("find results  {returned}/{total}")
            } else {
                format!("find results  '{pattern}'  {returned}/{total}")
            };
            if !url.is_empty() {
                summary.push_str("  ");
                summary.push_str(url);
            }
            lines.push(summary);

            if let Some(matches) = find.get("matches").and_then(Value::as_array) {
                for matched in matches.iter().take(4) {
                    let line_no = matched.get("line").and_then(Value::as_u64).unwrap_or(0);
                    let text = matched
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    if line_no > 0 && !text.is_empty() {
                        lines.push(format!("L{line_no}: {text}"));
                    }
                }
            }
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn renders_web_run_call_as_browse_actions() {
        let mut lines = Vec::new();
        push_tool_call_lines(
            &mut lines,
            &ToolCall {
                name: "WebRun".into(),
                parameter: r#"{"open":[{"ref_id":"https://example.com","lineno":120}],"find":[{"ref_id":"web_1","pattern":"rust"}]}"#.into(),
                preview: None,
            },
            200,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("◇ web"));
        assert!(rendered.contains("open"));
        assert!(rendered.contains("https://example.com"));
        assert!(rendered.contains("L120"));
        assert!(rendered.contains("find"));
        assert!(rendered.contains("'rust' in web_1"));
    }

    #[test]
    fn renders_web_run_open_result_as_summary_and_excerpt() {
        let mut lines = Vec::new();
        push_tool_result_lines(
            &mut lines,
            &ToolResultEntry {
                name: "WebRun".into(),
                output: r#"{"open":[{"ref_id":"web_1","final_url":"https://example.com/docs","title":"Example Docs","start_lineno":1,"end_lineno":3,"total_lines":10,"next_lineno":4,"content":"L1: Title\nL2: Intro\nL3: Details"}]}"#.into(),
            },
            200,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("↳ web"));
        assert!(rendered.contains("opened page"));
        assert!(rendered.contains("web_1"));
        assert!(rendered.contains("Example Docs"));
        assert!(rendered.contains("L1-3 of 10"));
        assert!(rendered.contains("next L4"));
        assert!(rendered.contains("https://example.com/docs"));
        assert!(rendered.contains("L1: Title"));
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
