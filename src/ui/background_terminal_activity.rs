use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::app::{ActivityDisplayState, BackgroundTerminalStatusEntry};

use super::wrap::wrap_text;

pub(super) fn push_background_terminal_status_lines(
    lines: &mut Vec<Line<'static>>,
    entry: &BackgroundTerminalStatusEntry,
    width: usize,
) {
    let prefix = "● terminal";
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

    if let Some(detail_text) = &entry.detail_text {
        let detail_prefix_width = prefix.chars().count() + 2;
        let detail_width = width.saturating_sub(detail_prefix_width).max(1);
        let detail = truncate_single_line(detail_text, detail_width);
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(detail_prefix_width)),
            Span::styled(detail, Style::default().fg(Color::DarkGray)),
        ]));
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
