use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    app::{Action, Effect},
    config::ReasoningEffort,
    stats::StatsTotals,
    tools::{DiffKind, DiffPreviewLine, MutationPreview, mutation_preview},
};

use super::*;
use crate::ui::render::test_support::{
    buffer_lines, buffer_string, word_has_foreground, word_has_modifier,
};
use crate::ui::{markdown::rendered_line_text, tool_activity::push_mutation_tool_call_lines};

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
    assert!(rendered.contains("Loaded Azure OpenAI model"));
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
        banner_foregrounds(terminal.backend().buffer())
            .iter()
            .any(|color| *color == accent_color(app.mode(), app.plan_active())),
        "expected startup banner to retain the base accent color"
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
fn render_keeps_startup_banner_sparkling() {
    let backend = TestBackend::new(140, 16);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("initial render succeeds");
    let before = banner_foregrounds(terminal.backend().buffer());
    assert!(
        has_multiple_unique_colors(&before),
        "expected startup banner to use more than one shade while sparkling"
    );

    for _ in 0..4 {
        app.apply(Action::Tick);
    }

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("sparkle render succeeds");
    let after = banner_foregrounds(terminal.backend().buffer());
    assert!(
        before != after,
        "expected startup banner sparkle colors to change over time"
    );
}

#[test]
fn render_restores_composer_in_plan_feedback_mode() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.begin_plan_review_feedback();
    app.composer_mut().insert_str("revise this");

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("revise this"));
    assert!(!rendered.contains("Plan Ready"));
    assert!(rendered.contains("Plan"));
    assert!(app.plan_active());
    assert!(word_has_foreground(
        terminal.backend().buffer(),
        "Plan",
        accent_color(app.mode(), true),
    ));
}

#[test]
fn render_keeps_thinking_visible_for_whitespace_only_pending_text() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("hello");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::TextDelta("\n ".into()),
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("thinking"));
}

#[test]
fn render_keeps_thinking_visible_for_plan_wrapper_prefix() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.enter_planning_draft_mode();
    app.composer_mut().insert_str("Make a roadmap");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::TextDelta("<proposed_plan>\n".into()),
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("thinking"));
}

#[test]
fn render_pinned_history_shows_pinned_state_without_footer_busy_indicator() {
    let backend = TestBackend::new(120, 10);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    for index in 0..8 {
        app.push_agent_message(format!("history line {index}"));
    }
    app.composer_mut().insert_str("hello");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));
    app.apply(Action::ScrollHistoryToTop);

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Pinned"));
    assert!(!rendered.contains(" Busy"));
}

#[test]
fn render_shows_chat_busy_indicator_after_tool_call_starts() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("git status");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::ToolCall {
            name: "RunShellScript".into(),
            arguments: "{\"command\":\"git status\"}".into(),
        },
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("RunShellScript"));
    assert!(rendered.contains("thinking"));
}

#[test]
fn render_shows_waiting_in_chat_when_write_approval_is_pending() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("edit this");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::WriteApprovalRequested {
            request_id: "call-1".into(),
            tool_name: "WriteFile".into(),
            arguments: "{\"filename\":\"src/new.rs\",\"content\":\"hi\",\"intent\":\"Add helper\"}"
                .into(),
        },
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Waiting"));
    assert!(!rendered.contains("thinking"));
}

#[test]
fn render_approval_pending_takes_precedence_over_pinned_history_busy_indicator() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    for index in 0..8 {
        app.push_agent_message(format!("history line {index}"));
    }
    app.apply(Action::SubagentEvent(
        crate::subagents::SubagentUiEvent::WriteApprovalRequested {
            id: "subagent-2".into(),
            request_id: "call-2".into(),
            tool_name: "WriteFile".into(),
            arguments: "{\"filename\":\"src/new.rs\",\"content\":\"hi\",\"intent\":\"Add helper\"}"
                .into(),
        },
    ));
    app.apply(Action::ScrollHistoryToTop);

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Approval pending: WriteFile from subagent-2"));
}

#[test]
fn render_write_approval_panel_identifies_subagent_source() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
    app.apply(Action::SubagentEvent(
        crate::subagents::SubagentUiEvent::WriteApprovalRequested {
            id: "subagent-2".into(),
            request_id: "call-2".into(),
            tool_name: "WriteFile".into(),
            arguments: "{\"filename\":\"src/new.rs\",\"content\":\"hi\",\"intent\":\"Add helper\"}"
                .into(),
        },
    ));

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Source: subagent-2"));
    assert!(rendered.contains("Approval pending: WriteFile from subagent-2"));
}

#[test]
fn render_shows_latest_subagent_tool_name() {
    let backend = TestBackend::new(80, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
    app.apply(Action::SubagentEvent(
        crate::subagents::SubagentUiEvent::Spawned {
            id: "subagent-2".into(),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: crate::subagents::SubagentActivityKind::General,
        },
    ));
    app.apply(Action::SubagentEvent(
        crate::subagents::SubagentUiEvent::Updated {
            id: "subagent-2".into(),
            latest_tool_name: Some("Grep".into()),
        },
    ));

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let lines = buffer_lines(terminal.backend());
    assert!(lines.iter().any(|line| line.contains("subagent-2")));
    assert!(lines.iter().any(|line| line.contains("tool: Grep")));
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
        event: crate::app::StreamEvent::ToolCall {
            name: "List".into(),
            arguments: r#"{"dir":"src","recursive":true}"#.into(),
        },
    });
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::ToolResult {
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
            event: crate::app::StreamEvent::ToolCall {
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
            event: crate::app::StreamEvent::ToolCall {
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
        event: crate::app::StreamEvent::ToolCall {
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
        event: crate::app::StreamEvent::ToolCall {
            name: "List".into(),
            arguments: r#"{"dir":"src","recursive":true}"#.into(),
        },
    });
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::ToolResult {
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
            event: crate::app::StreamEvent::ToolCall {
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
            event: crate::app::StreamEvent::ToolCall {
                name: format!("List{index}"),
                arguments: format!(r#"{{"dir":"src/{index}"}}"#),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::app::StreamEvent::ToolResult {
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
            event: crate::app::StreamEvent::ToolCall {
                name: format!("First{index}"),
                arguments: format!(r#"{{"dir":"first/{index}"}}"#),
            },
        });
    }

    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::TextDelta("separator".into()),
    });

    for index in 1..=6 {
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: crate::app::StreamEvent::ToolCall {
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
        event: crate::app::StreamEvent::TextDelta("- first item\n- second item".into()),
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
        event: crate::app::StreamEvent::TextDelta("```rust\nlet value = 1;\n```".into()),
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
fn render_hides_proposed_plan_wrapper_tags() {
    let backend = TestBackend::new(100, 16);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("render plan");
    app.apply(Action::SubmitMessage);
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::TextDelta(
            "<proposed_plan>\n# Plan\n\n- step one\n</proposed_plan>".into(),
        ),
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(!rendered.contains("<proposed_plan>"));
    assert!(!rendered.contains("</proposed_plan>"));
    assert!(rendered.contains("Plan"));
    assert!(rendered.contains("step one"));
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
        assert_eq!(cell.bg, accent_color(app.mode(), app.plan_active()));
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
        event: crate::app::StreamEvent::TextDelta(
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
        event: crate::app::StreamEvent::TextDelta("**bold** and *italic*".into()),
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
        event: crate::app::StreamEvent::TextDelta("```rust\nlet value = \"hi\";\n```".into()),
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
        event: crate::app::StreamEvent::TextDelta("```csharp\npublic class Demo { }\n```".into()),
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
        event: crate::app::StreamEvent::TextDelta("```unknownlang\nplain text\n```".into()),
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
        event: crate::app::StreamEvent::TextDelta("```text\nalpha\nbetagamma\n```".into()),
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
fn render_scrollback_reveals_older_messages() {
    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

    for index in 1..=8 {
        app.composer_mut().insert_str(format!("message {index}"));
        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: index as u64,
            event: crate::app::StreamEvent::Finished { history: None },
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
            event: crate::app::StreamEvent::Finished { history: None },
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
        event: crate::app::StreamEvent::TextDelta(initial_items),
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
        event: crate::app::StreamEvent::TextDelta("\n- item 13\n- item 14".into()),
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
            event: crate::app::StreamEvent::Finished { history: None },
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

fn word_has_background(buffer: &ratatui::buffer::Buffer, word: &str, background: Color) -> bool {
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

fn banner_foregrounds(buffer: &ratatui::buffer::Buffer) -> Vec<Color> {
    buffer
        .content
        .iter()
        .filter(|cell| matches!(cell.symbol(), "█" | "░"))
        .map(|cell| cell.fg)
        .collect()
}

fn has_multiple_unique_colors(colors: &[Color]) -> bool {
    colors
        .iter()
        .enumerate()
        .any(|(index, color)| colors.iter().skip(index + 1).any(|other| other != color))
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
