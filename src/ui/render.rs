use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
};

use crate::app::{App, SelectionPicker, SlashCommand};

use super::{
    history::render_history, markdown::loading_frame, theme::accent_color, wrap::wrap_text,
};

pub fn render(frame: &mut Frame, app: &mut App) {
    let screen = frame.area();
    let accent = accent_color(app.mode());
    let input_height = if let Some(pending) = app.pending_write_approval() {
        pending_write_approval_height(pending, screen.width)
    } else {
        app.composer_height().max(3)
    };
    let overlay_height = app.overlay_height();
    let mut constraints = vec![Constraint::Min(1)];
    if overlay_height > 0 {
        constraints.push(Constraint::Length(overlay_height));
    }
    constraints.push(Constraint::Length(input_height));
    constraints.push(Constraint::Length(1));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(screen);

    let mut section = 0;
    render_history(frame, app, layout[section], accent, loading_frame(app));
    section += 1;
    if overlay_height > 0 {
        render_overlay(frame, app, layout[section], accent);
        section += 1;
    }
    render_input(frame, app, layout[section], accent);
    render_mode(frame, app, layout[section + 1], accent);
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
        pending.summary.clone(),
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

fn render_overlay(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    if let Some(picker) = app.selection_picker() {
        render_selection_picker(frame, picker, area, accent);
    } else {
        render_command_palette(frame, app, area, accent);
    }
}

fn render_selection_picker(frame: &mut Frame, picker: &SelectionPicker, area: Rect, accent: Color) {
    let visible_rows = area.height.saturating_sub(2) as usize;
    let (title, lines) = match picker {
        SelectionPicker::Model { selected_index } => {
            let lines: Vec<Line<'static>> = crate::model_registry::models()
                .iter()
                .take(visible_rows)
                .enumerate()
                .map(|(index, model)| {
                    selection_picker_line(
                        index == *selected_index,
                        model.name,
                        model_picker_detail(model),
                        accent,
                    )
                })
                .collect();
            (" Models ", lines)
        }
        SelectionPicker::Reasoning {
            model_name,
            options,
            selected_index,
        } => {
            let lines: Vec<Line<'static>> = options
                .iter()
                .take(visible_rows)
                .enumerate()
                .map(|(index, level)| {
                    selection_picker_line(
                        index == *selected_index,
                        level.as_str(),
                        format!("for {}", model_name),
                        accent,
                    )
                })
                .collect();
            (" Reasoning ", lines)
        }
    };

    let picker = Paragraph::new(lines)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .border_style(Style::default().fg(accent)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(picker, area);
}

fn render_mode(frame: &mut Frame, app: &App, area: Rect, accent: Color) {
    let mode_label = mode_status_label(app.mode(), app.approval_mode());
    let session_stats = app.session_stats();
    let context_percent = app.next_request_context_percent();

    let mut spans = vec![
        Span::styled(
            mode_label,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {} • {}  in {}  out {}  ctx {}  ${:.6}",
            app.model_name(),
            app.reasoning_effort().as_str(),
            format_compact_tokens(session_stats.input_tokens),
            format_compact_tokens(session_stats.output_tokens),
            format!("{context_percent}%"),
            session_stats.estimated_cost_usd(),
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
    approval_mode: crate::app::ApprovalMode,
) -> &'static str {
    match (mode, approval_mode) {
        (crate::app::AccessMode::ReadWrite, crate::app::ApprovalMode::Disabled) => "Write (!)",
        _ => mode.label(),
    }
}

fn pending_write_approval_height(
    pending: &crate::app::PendingWriteApproval,
    panel_width: u16,
) -> u16 {
    let content_width = panel_width.saturating_sub(4) as usize;
    let summary_lines = wrap_text(&pending.summary, content_width.max(1)).len();
    (summary_lines + 3 + 2) as u16
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

fn selection_picker_line(
    is_selected: bool,
    label: impl Into<String>,
    detail: impl Into<String>,
    accent: Color,
) -> Line<'static> {
    let marker = if is_selected { ">" } else { " " };
    let name_style = if is_selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    Line::from(vec![
        Span::styled(marker, name_style),
        Span::raw(" "),
        Span::styled(label.into(), name_style),
        Span::raw("  "),
        Span::styled(detail.into(), Style::default().fg(Color::Gray)),
    ])
}

fn model_picker_detail(model: &crate::model_registry::ModelInfo) -> String {
    let standard = format!(
        "{}  ctx {}  in {}  cache {}  out {}",
        model.provider.display_name(),
        format_context_length(model.context_length),
        format_price(model.pricing.input_per_million_tokens),
        format_price(model.pricing.cache_read_per_million_tokens),
        format_price(model.pricing.output_per_million_tokens),
    );

    if let Some(long_context) = model.long_context_pricing {
        format!(
            "{standard}  >{} in {}  cache {}  out {}",
            format_context_length(long_context.input_tokens_threshold),
            format_price(long_context.pricing.input_per_million_tokens),
            format_price(long_context.pricing.cache_read_per_million_tokens),
            format_price(long_context.pricing.output_per_million_tokens),
        )
    } else {
        standard
    }
}

fn format_context_length(context_length: usize) -> String {
    if context_length >= 1_000_000 {
        format!("{:.2}M", context_length as f64 / 1_000_000.0)
    } else if context_length >= 1_000 {
        format!("{}K", context_length / 1_000)
    } else {
        context_length.to_string()
    }
}

fn format_price(price: f64) -> String {
    if price == 0.0 {
        "0".to_string()
    } else if price < 0.1 {
        format!("{price:.3}")
    } else {
        format!("{price:.2}")
    }
}

fn format_compact_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.2}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::{
        app::{Action, MessageStyle},
        config::ReasoningEffort,
        stats::StatsTotals,
        tools::{DiffKind, DiffPreviewLine, MutationPreview, mutation_preview},
    };

    use super::*;
    use crate::ui::{
        history::scrollbar_thumb_bounds,
        markdown::{
            MarkdownSegment, markdown_segments, message_style, normalized_highlight_language,
            rendered_line_text,
        },
        tool_activity::push_mutation_tool_call_lines,
        wrap::wrap_text,
    };

    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time works")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("oat-render-{unique}"));
            fs::create_dir_all(&root).expect("temp root created");
            Self { root }
        }

        fn write(&self, relative_path: &str, content: &str) {
            let path = self.root.join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("parent directories created");
            }
            fs::write(path, content).expect("test file written");
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

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
        let backend = TestBackend::new(140, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
        app.set_session_stats(StatsTotals {
            input_tokens: 1_234,
            cached_input_tokens: 200,
            output_tokens: 345,
            estimated_cost_nanos_usd: 123_456_000,
            request_count: 2,
            tool_call_count: 0,
            tool_success_count: 0,
            tool_failure_count: 0,
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

        assert!(rendered.contains("░███████"));
        assert!(rendered.contains("v0.1.0"));
        assert!(rendered.contains("Loaded Azure model"));
        assert!(rendered.contains("Read-only"));
        assert!(rendered.contains("gpt-5.4-mini • medium"));
        assert!(rendered.contains("in 1.2K"));
        assert!(rendered.contains("out 345"));
        assert!(rendered.contains("ctx 0%"));
        assert!(rendered.contains("$0.123456"));
        assert!(
            buffer_lines(terminal.backend())
                .iter()
                .any(|line| line.contains("                         ░██")),
            "expected startup banner indentation to be preserved"
        );
        assert!(
            buffer_lines(terminal.backend())
                .iter()
                .any(|line| line.trim() == "v0.1.0"),
            "expected startup version to render underneath the banner"
        );
        assert!(
            word_has_foreground(
                terminal.backend().buffer(),
                "░███████",
                accent_color(app.mode())
            ),
            "expected startup banner to use the accent color"
        );

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
    fn render_animates_startup_version_into_accent_color() {
        let backend = TestBackend::new(140, 16);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("initial render succeeds");
        assert!(
            word_has_foreground(terminal.backend().buffer(), "v0.1.0", Color::DarkGray),
            "expected startup version to begin dim before the shimmer passes"
        );

        for _ in 0..8 {
            app.apply(Action::Tick);
        }

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("animated render succeeds");
        assert!(
            word_has_foreground(
                terminal.backend().buffer(),
                "v0.1.0",
                accent_color(app.mode())
            ),
            "expected startup version to settle into the accent color"
        );
    }

    #[test]
    fn render_shows_model_picker_details() {
        let backend = TestBackend::new(160, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
        app.open_model_picker();

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("Models"));
        assert!(rendered.contains("gpt-5.4"));
        assert!(rendered.contains("Azure OpenAI"));
        assert!(rendered.contains("ctx 1.05M"));
        assert!(rendered.contains(">272K"));
    }

    #[test]
    fn render_shows_reasoning_picker() {
        let backend = TestBackend::new(120, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
        app.open_reasoning_picker();

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("Reasoning"));
        assert!(rendered.contains("low"));
        assert!(rendered.contains("medium"));
        assert!(rendered.contains("high"));
    }

    #[test]
    fn mode_status_label_marks_session_preapproved_write_mode() {
        assert_eq!(
            mode_status_label(
                crate::app::AccessMode::ReadWrite,
                crate::app::ApprovalMode::Manual,
            ),
            "Write"
        );
        assert_eq!(
            mode_status_label(
                crate::app::AccessMode::ReadWrite,
                crate::app::ApprovalMode::Disabled,
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
            summary: "Fix startup".into(),
            target: Some("src/lib.rs".into()),
        };
        assert_eq!(pending_write_approval_height(&short, 120), 6);

        let wrapped = crate::app::PendingWriteApproval {
            request_id: "call-2".into(),
            tool_name: "ApplyPatches".into(),
            arguments: "{\"filename\":\"src/lib.rs\",\"patches\":[{\"old_text\":\"a\",\"new_text\":\"b\"}],\"intent\":\"Fix the broken startup path so the app launches again after config bootstrap changes\"}".into(),
            summary:
                "Fix the broken startup path so the app launches again after config bootstrap changes"
                    .into(),
            target: Some("src/lib.rs".into()),
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
        let tree = TempTree::new();
        tree.write("src/lib.rs", "old line\n");
        app.set_workspace_root(tree.root.clone());
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
        assert!(rendered.contains("1   | - old line"));
        assert!(rendered.contains("1 | + new line"));
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
    fn mutation_preview_numbers_multiline_apply_patch_lines() {
        let tree = TempTree::new();
        tree.write("src/lib.rs", "alpha\nold one\nold two\nomega\n");

        let preview = mutation_preview(
            "ApplyPatches",
            r#"{"filename":"src/lib.rs","patches":[{"old_text":"old one\nold two","new_text":"new one\nnew two"}]}"#,
            &tree.root,
        )
        .expect("preview");

        let lines = preview
            .lines
            .iter()
            .map(|line| {
                (
                    line.old_line_number,
                    line.new_line_number,
                    line.prefix,
                    line.text.as_str(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            lines,
            vec![
                (Some(2), None, '-', "old one"),
                (Some(3), None, '-', "old two"),
                (None, Some(2), '+', "new one"),
                (None, Some(3), '+', "new two"),
            ]
        );
    }

    #[test]
    fn mutation_preview_adjusts_line_numbers_after_line_count_change() {
        let tree = TempTree::new();
        tree.write(
            "src/lib.rs",
            "top\nold one\nold two\nstay\nnext old\nbottom\n",
        );

        let preview = mutation_preview(
            "ApplyPatches",
            r#"{"filename":"src/lib.rs","patches":[{"old_text":"old one\nold two","new_text":"new only"},{"old_text":"next old","new_text":"next new\nnext newer"}]}"#,
            &tree.root,
        )
        .expect("preview");

        let lines = preview
            .lines
            .iter()
            .map(|line| {
                (
                    line.old_line_number,
                    line.new_line_number,
                    line.prefix,
                    line.text.as_str(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            lines,
            vec![
                (Some(2), None, '-', "old one"),
                (Some(3), None, '-', "old two"),
                (None, Some(2), '+', "new only"),
                (Some(4), None, '-', "next old"),
                (None, Some(4), '+', "next new"),
                (None, Some(5), '+', "next newer"),
            ]
        );
    }

    #[test]
    fn render_shows_write_file_tool_call_with_line_numbers() {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("show write tool");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolCall {
                name: "WriteFile".into(),
                arguments:
                    r#"{"filename":"src/new.rs","content":"first line\nsecond line","intent":"Create a new file"}"#
                        .into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("1 | + first line"));
        assert!(rendered.contains("2 | + second line"));
    }

    #[test]
    fn render_shows_delete_path_tool_call_with_line_numbers() {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        let tree = TempTree::new();
        tree.write("notes.txt", "alpha\nbeta\n");
        app.set_workspace_root(tree.root.clone());
        app.composer_mut().insert_str("show delete tool");
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::llm::StreamEvent::ToolCall {
                name: "DeletePath".into(),
                arguments: r#"{"path":"notes.txt","intent":"Remove stale notes"}"#.into(),
            },
        });

        terminal
            .draw(|frame| render(frame, &mut app))
            .expect("render succeeds");

        let rendered = buffer_string(terminal.backend());
        assert!(rendered.contains("1   | - alpha"));
        assert!(rendered.contains("2   | - beta"));
    }

    #[test]
    fn wrapped_diff_continuations_do_not_repeat_line_numbers() {
        let preview = MutationPreview {
            target: "src/lib.rs".into(),
            summary: None,
            lines: vec![DiffPreviewLine {
                old_line_number: Some(12),
                new_line_number: None,
                prefix: '-',
                text: "this preview line should wrap cleanly".into(),
                kind: DiffKind::Removed,
            }],
        };
        let mut lines = Vec::new();

        push_mutation_tool_call_lines(&mut lines, "◇ tool", "ApplyPatches", &preview, 28);

        let rendered = lines.iter().map(rendered_line_text).collect::<Vec<_>>();
        let diff_rows = rendered
            .iter()
            .filter(|line| line.contains("|"))
            .cloned()
            .collect::<Vec<_>>();

        assert!(
            diff_rows.len() >= 2,
            "expected wrapped diff rows: {diff_rows:?}"
        );
        assert!(diff_rows[0].contains("12"));
        assert!(!diff_rows[1].contains("12"));
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
        for cell in selected_row.iter().skip(column as usize).take(5) {
            assert_eq!(cell.bg, accent_color(app.mode()));
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
            app.composer_mut().insert_str(format!("message {index}"));
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
            app.composer_mut().insert_str(format!("entry {index}"));
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
            app.composer_mut().insert_str(format!("entry {index}"));
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
                    && row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.modifier.contains(modifier))
                {
                    return true;
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
                    && row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.bg == background)
                {
                    return true;
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
                    && row[start..start + symbols.len()]
                        .iter()
                        .all(|cell| cell.fg != foreground)
                {
                    return true;
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
