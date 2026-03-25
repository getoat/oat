use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{Action, App},
    config::ReasoningEffort,
    ui::render::{
        render,
        test_support::{buffer_lines, buffer_string},
    },
};

use super::{pending_shell_approval_height, pending_write_approval_height};

#[test]
fn pending_write_approval_height_matches_wrapped_summary_lines() {
    let short = crate::app::PendingWriteApproval {
        request_id: "call-1".into(),
        tool_name: "ApplyPatches".into(),
        arguments: "{\"filename\":\"src/lib.rs\",\"patches\":[{\"old_text\":\"a\",\"new_text\":\"b\"}],\"intent\":\"Fix startup\"}".into(),
        summary: "Fix startup".into(),
        target: Some("src/lib.rs".into()),
        source_label: None,
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
        source_label: None,
    };
    assert!(pending_write_approval_height(&wrapped, 36) > 6);
}

#[test]
fn pending_shell_approval_height_grows_for_multiline_commands() {
    let mut short = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    short.state_mut().session.pending_shell_approvals.push_back(
        crate::app::PendingShellApproval::new(
            "call-1".into(),
            crate::app::CommandRisk::Low,
            "read-only inspection command with no obvious mutation".into(),
            "pwd".into(),
            ".".into(),
            "inspect workspace".into(),
            None,
        ),
    );
    short.state_mut().ui.pending_shell_approval = short
        .state_mut()
        .session
        .pending_shell_approvals
        .front()
        .map(crate::app::ui::ShellApprovalUiState::new);

    let mut multiline = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    multiline
        .state_mut()
        .session
        .pending_shell_approvals
        .push_back(crate::app::PendingShellApproval::new(
            "call-2".into(),
            crate::app::CommandRisk::Low,
            "read-only inspection command with no obvious mutation".into(),
            "printf one\nprintf two".into(),
            ".".into(),
            "inspect workspace".into(),
            None,
        ));
    multiline.state_mut().ui.pending_shell_approval = multiline
        .state_mut()
        .session
        .pending_shell_approvals
        .front()
        .map(crate::app::ui::ShellApprovalUiState::new);

    assert!(
        pending_shell_approval_height(&multiline, 120) > pending_shell_approval_height(&short, 120)
    );
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
        event: crate::app::StreamEvent::WriteApprovalRequested {
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
fn render_shows_multiline_shell_command_as_multiple_rows() {
    let backend = TestBackend::new(120, 18);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("run shell");
    app.apply(Action::SubmitMessage);
    app.apply(Action::StreamEvent {
        reply_id: 1,
        event: crate::app::StreamEvent::ShellApprovalRequested {
            request_id: "call-1".into(),
            risk: crate::app::CommandRisk::Low,
            risk_explanation: "read-only inspection command with no obvious mutation".into(),
            command: "printf 'one\\n'\nprintf 'two\\n'".into(),
            working_directory: ".".into(),
            reason: "inspect output".into(),
        },
    });

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    let lines = buffer_lines(terminal.backend());
    assert!(rendered.contains("Shell Approval Required"));
    assert!(rendered.contains("Command:"));
    assert!(lines.iter().any(|line| line.contains("printf 'one\\n'")));
    assert!(lines.iter().any(|line| line.contains("printf 'two\\n'")));
}
