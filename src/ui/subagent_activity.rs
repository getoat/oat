use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::{SubagentDisplayState, SubagentStatusEntry, SubagentStatusKind};

use super::wrap::wrap_text;

pub(super) fn push_subagent_status_lines(
    lines: &mut Vec<Line<'static>>,
    entry: &SubagentStatusEntry,
    width: usize,
) {
    let prefix = match entry.kind {
        SubagentStatusKind::Subagent => "● subagent",
        SubagentStatusKind::Planning => "● planning",
    };
    let body = format!("{}  {}", entry.display_label, entry.status_text);
    let content_width = width.saturating_sub(prefix.chars().count() + 2).max(1);
    let wrapped = wrap_text(&body, content_width);

    for (index, chunk) in wrapped.into_iter().enumerate() {
        if index == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(status_color(entry.state))
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

    if let Some(latest_tool_name) = &entry.latest_tool_name {
        let detail_prefix_width = prefix.chars().count() + 2;
        let detail_width = width.saturating_sub(detail_prefix_width).max(1);
        let detail = truncate_single_line(&format!("tool: {latest_tool_name}"), detail_width);
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(detail_prefix_width)),
            Span::styled(detail, Style::default().fg(Color::DarkGray)),
        ]));
    }
}

fn status_color(state: SubagentDisplayState) -> Color {
    match state {
        SubagentDisplayState::Running => Color::Cyan,
        SubagentDisplayState::Completed => Color::Green,
        SubagentDisplayState::Failed => Color::Red,
    }
}

fn truncate_single_line(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= width {
        return text.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }

    chars[..width - 3].iter().collect::<String>() + "..."
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
    fn latest_tool_is_rendered_on_single_follow_on_line() {
        let mut lines = Vec::new();
        push_subagent_status_lines(
            &mut lines,
            &SubagentStatusEntry {
                id: "subagent-1".into(),
                kind: SubagentStatusKind::Subagent,
                display_label: "subagent-1".into(),
                state: SubagentDisplayState::Running,
                status_text: "running in read-only mode".into(),
                latest_tool_name: Some("VeryLongToolNameThatShouldBeTruncated".into()),
            },
            44,
        );

        let tool_lines = lines
            .iter()
            .map(line_text)
            .filter(|line| line.contains("tool:"))
            .collect::<Vec<_>>();

        assert_eq!(tool_lines.len(), 1);
        assert!(tool_lines[0].chars().count() <= 44);
        assert!(tool_lines[0].ends_with("..."));
    }

    #[test]
    fn planning_entries_use_planning_label() {
        let mut lines = Vec::new();
        push_subagent_status_lines(
            &mut lines,
            &SubagentStatusEntry {
                id: "subagent-2".into(),
                kind: SubagentStatusKind::Planning,
                display_label: "Planning with gpt-5.4".into(),
                state: SubagentDisplayState::Running,
                status_text: "running in read-only mode".into(),
                latest_tool_name: None,
            },
            60,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(rendered.contains("planning"));
        assert!(rendered.contains("Planning with gpt-5.4"));
    }
}
