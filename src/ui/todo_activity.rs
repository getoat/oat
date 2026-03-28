use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::todo::{TodoSnapshot, TodoStatus};

use super::wrap::wrap_text;

pub(super) fn push_todo_snapshot_lines(
    lines: &mut Vec<Line<'static>>,
    snapshot: &TodoSnapshot,
    width: usize,
    accent: Color,
) {
    if !snapshot.has_list {
        lines.push(Line::from(Span::styled(
            "No active todo items.",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        return;
    }

    let content_width = width.max(1);
    for task in &snapshot.tasks {
        let checkbox = match task.status {
            TodoStatus::Todo | TodoStatus::InProgress => "[ ]",
            TodoStatus::Done => "[x]",
        };
        let wrapped = wrap_text(&format!("{checkbox} {}", task.description), content_width);
        let style = task_style(task.status, accent);

        for (index, chunk) in wrapped.into_iter().enumerate() {
            if index == 0 {
                lines.push(Line::from(Span::styled(chunk, style)));
            } else {
                lines.push(Line::from(Span::styled(format!("    {chunk}"), style)));
            }
        }
    }
}

fn task_style(status: TodoStatus, accent: Color) -> Style {
    match status {
        TodoStatus::Todo => Style::default(),
        TodoStatus::InProgress => Style::default().fg(accent).add_modifier(Modifier::BOLD),
        TodoStatus::Done => Style::default().fg(Color::Green),
    }
}
