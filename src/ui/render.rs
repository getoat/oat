use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::app::{
    App, ChatMessage, MessageStyle, SlashCommand, Speaker, ToolCall, ToolResultEntry,
    TranscriptEntry,
};

use super::theme::accent_color;

const LOADING_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

fn render_history(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let mut lines = Vec::new();

    for entry in app.entries() {
        match entry {
            TranscriptEntry::Message(message) => {
                push_message_lines(&mut lines, message, area.width as usize, accent);
            }
            TranscriptEntry::ToolCall(tool_call) => {
                push_tool_call_lines(&mut lines, tool_call, area.width as usize);
            }
            TranscriptEntry::ToolResult(tool_result) => {
                if app.show_tool_output() {
                    push_tool_result_lines(&mut lines, tool_result, area.width as usize);
                } else {
                    continue;
                }
            }
        }
        lines.push(Line::default());
    }

    if app.has_pending_reply() && !app.has_visible_pending_content() {
        push_pending_lines(&mut lines, area.width as usize, accent, loading_frame(app));
    }

    let visible_count = area.height as usize;
    let start = lines.len().saturating_sub(visible_count);
    let history = Paragraph::new(lines.into_iter().skip(start).collect::<Vec<_>>());
    frame.render_widget(history, area);
}

fn render_input(frame: &mut Frame, app: &mut App, area: Rect, accent: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(Style::default().fg(accent));
    app.composer_mut().set_block(block);
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
            "  {}  Thinking {}  Tool output {}  / commands  Tab toggle  Ctrl+C clear/quit",
            app.model_name(),
            if app.show_thinking() {
                "visible"
            } else {
                "hidden"
            },
            if app.show_tool_output() {
                "visible"
            } else {
                "hidden"
            }
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

    use crate::app::Action;

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
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5-mini");

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
        assert!(rendered.contains("Thinking visible"));
        assert!(rendered.contains("Tool output hidden"));

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
        let mut app = App::new(true, true, "gpt-5-mini");
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
        let mut app = App::new(true, false, "gpt-5-mini");
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
}
