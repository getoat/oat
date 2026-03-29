use regex::Regex;
use std::sync::LazyLock;

use super::{
    BackgroundTerminalInspectResult, BackgroundTerminalSnapshot, BackgroundTerminalStatus,
};

static ANSI_ESCAPE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").expect("valid ANSI escape regex"));

pub(crate) fn normalize_terminal_output(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    ANSI_ESCAPE_PATTERN.replace_all(&text, "").into_owned()
}

pub(crate) fn format_terminal_list_message(terminals: &[BackgroundTerminalSnapshot]) -> String {
    if terminals.is_empty() {
        return "No background terminals.".into();
    }

    let lines = terminals
        .iter()
        .map(|terminal| {
            format!(
                "- `{}` {} [{}] cwd=`{}`",
                terminal.id,
                terminal.label,
                status_label(terminal),
                terminal.cwd
            )
        })
        .collect::<Vec<_>>();
    format!("Background terminals:\n{}", lines.join("\n"))
}

pub(crate) fn format_terminal_inspect_message(result: &BackgroundTerminalInspectResult) -> String {
    let terminal = &result.snapshot;
    let mut lines = vec![
        format!("Terminal `{}`: {}", terminal.id, terminal.label),
        format!("Status: {}", status_label(terminal)),
        format!("Working directory: `{}`", terminal.cwd),
        format!("Sequence: {}", result.output.sequence),
    ];

    if let Some(pid) = terminal.pid {
        lines.push(format!("PID: {pid}"));
    }
    if result.output.output_truncated {
        lines.push("Output: truncated to retained tail".into());
    }
    if result.output.text.is_empty() {
        lines.push("Output:\n(empty)".into());
    } else {
        lines.push(format!("Output:\n{}", result.output.text));
    }

    lines.join("\n")
}

fn status_label(terminal: &BackgroundTerminalSnapshot) -> String {
    match terminal.status {
        BackgroundTerminalStatus::Running => "running".into(),
        BackgroundTerminalStatus::Cancelled => "cancelled".into(),
        BackgroundTerminalStatus::SpawnFailed => terminal
            .error
            .as_ref()
            .map(|error| format!("spawn failed: {error}"))
            .unwrap_or_else(|| "spawn failed".into()),
        BackgroundTerminalStatus::Exited => terminal
            .exit_info
            .as_ref()
            .map(|exit| {
                if exit.success {
                    format!("exited ({})", exit.code.unwrap_or(0))
                } else {
                    format!(
                        "exited non-zero ({})",
                        exit.code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "signal".into())
                    )
                }
            })
            .unwrap_or_else(|| "exited".into()),
    }
}
