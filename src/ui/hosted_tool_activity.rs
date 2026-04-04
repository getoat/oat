use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::{ActivityDisplayState, HostedToolStatusEntry};

use super::wrap::wrap_text;

pub(super) fn push_hosted_tool_status_lines(
    lines: &mut Vec<Line<'static>>,
    entry: &HostedToolStatusEntry,
    width: usize,
) {
    let prefix = entry.kind.transcript_prefix();
    let body = hosted_tool_body(entry);
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
}

fn hosted_tool_body(entry: &HostedToolStatusEntry) -> String {
    let action = entry.kind.action_label(entry.state);

    if entry.detail.trim().is_empty() {
        action.to_string()
    } else {
        format!("{action}  {}", entry.detail)
    }
}

fn status_color(state: ActivityDisplayState) -> Color {
    match state {
        ActivityDisplayState::Running => Color::Cyan,
        ActivityDisplayState::Completed => Color::Green,
        ActivityDisplayState::Failed => Color::Red,
        ActivityDisplayState::Cancelled => Color::Yellow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::HostedToolKind;

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn renders_search_status_with_detail() {
        let mut lines = Vec::new();
        push_hosted_tool_status_lines(
            &mut lines,
            &HostedToolStatusEntry {
                id: "ws_1".into(),
                kind: HostedToolKind::Search,
                state: ActivityDisplayState::Running,
                detail: "latest rust news".into(),
            },
            60,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("search"));
        assert!(rendered.contains("Searching the web"));
        assert!(rendered.contains("latest rust news"));
    }

    #[test]
    fn renders_open_page_status_with_open_prefix() {
        let mut lines = Vec::new();
        push_hosted_tool_status_lines(
            &mut lines,
            &HostedToolStatusEntry {
                id: "ws_1".into(),
                kind: HostedToolKind::OpenPage,
                state: ActivityDisplayState::Completed,
                detail: "https://example.com".into(),
            },
            60,
        );

        let rendered = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("open"));
        assert!(rendered.contains("Opened page"));
        assert!(rendered.contains("https://example.com"));
    }
}
