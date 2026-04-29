//! Agent-managed "current task" plus mutable acceptance criteria.
//!
//! Populated and mutated by the task tools in `crate::tools::task`, read by
//! the end-of-turn critic in `crate::llm::critic` to decide whether the agent
//! actually finished the work.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const MAX_CRITERIA_PER_TASK: usize = 32;
pub const MAX_CAPTURED_COMMAND_BYTES: usize = 4_096;

pub type CriterionId = u32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AcceptanceCriterion {
    pub id: CriterionId,
    pub text: String,
    pub verification_hint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActiveTask {
    pub description: String,
    pub criteria: Vec<AcceptanceCriterion>,
    pub source_messages: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub next_criterion_id: CriterionId,
}

impl ActiveTask {
    pub fn new(description: String, source_messages: Vec<String>) -> Self {
        Self {
            description,
            criteria: Vec::new(),
            source_messages,
            created_at: Utc::now(),
            next_criterion_id: 1,
        }
    }

    pub fn allocate_criterion_id(&mut self) -> CriterionId {
        let id = self.next_criterion_id;
        self.next_criterion_id = self.next_criterion_id.wrapping_add(1).max(1);
        id
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn find(&self, id: CriterionId) -> Option<&AcceptanceCriterion> {
        self.criteria.iter().find(|c| c.id == id)
    }

    pub fn find_mut(&mut self, id: CriterionId) -> Option<&mut AcceptanceCriterion> {
        self.criteria.iter_mut().find(|c| c.id == id)
    }

    pub fn remove(&mut self, id: CriterionId) -> bool {
        if let Some(pos) = self.criteria.iter().position(|c| c.id == id) {
            self.criteria.remove(pos);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileTouchKind {
    Written,
    Edited,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FileTouch {
    pub path: String,
    pub kind: FileTouchKind,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandRun {
    pub command: String,
    pub working_dir: Option<String>,
    pub exit_code: Option<i32>,
    pub stdout_head: String,
    pub stdout_tail: String,
    pub stderr_head: String,
    pub stderr_tail: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct TurnEvidence {
    pub files_touched: Vec<FileTouch>,
    pub commands_run: Vec<CommandRun>,
}

impl TurnEvidence {
    pub fn reset(&mut self) {
        self.files_touched.clear();
        self.commands_run.clear();
    }

    pub fn record_file_touch(&mut self, touch: FileTouch) {
        self.files_touched.push(touch);
    }

    pub fn record_command(&mut self, run: CommandRun) {
        self.commands_run.push(run);
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.files_touched.is_empty() && self.commands_run.is_empty()
    }
}

/// Parse the plain-text output produced by `RunShellScriptTool` into
/// `(exit_code, stdout, stderr)`. The tool formats output as:
///
/// ```text
/// Command: ...
///
/// Working directory: ...
///
/// Exit code: 0
///
/// STDOUT: 12 bytes across 1 line
/// hello world
///
/// STDERR: 5 bytes across 1 line
/// oops
/// ```
///
/// (with either STDOUT/STDERR possibly missing, or `Output: (empty)` if both
/// are empty). We scan for the headers and extract the bodies. If the format
/// doesn't match (e.g. a different tool or a raw string), returns the whole
/// output as stdout with `exit_code = None`.
pub fn parse_run_shell_script_output(output: &str) -> (Option<i32>, String, String) {
    let mut exit_code: Option<i32> = None;
    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("Exit code: ") {
            exit_code = rest.trim().parse::<i32>().ok();
            break;
        }
    }

    let stdout = extract_section(output, "STDOUT:");
    let stderr = extract_section(output, "STDERR:");

    if stdout.is_none() && stderr.is_none() && exit_code.is_none() {
        return (None, output.to_string(), String::new());
    }

    (
        exit_code,
        stdout.unwrap_or_default(),
        stderr.unwrap_or_default(),
    )
}

fn extract_section(output: &str, header: &str) -> Option<String> {
    let header_pos = output.find(header)?;
    let after_header = &output[header_pos + header.len()..];
    let first_newline = after_header.find('\n')?;
    let body_start = &after_header[first_newline + 1..];

    // Section ends at the next top-level header line or end of string.
    let end = next_section_boundary(body_start);
    let body = &body_start[..end];
    Some(body.trim_end_matches('\n').to_string())
}

fn next_section_boundary(text: &str) -> usize {
    const BOUNDARIES: [&str; 4] = [
        "\nSTDOUT:",
        "\nSTDERR:",
        "\nOutput: (empty)",
        "\nExit code:",
    ];
    let mut earliest = text.len();
    for boundary in BOUNDARIES {
        if let Some(pos) = text.find(boundary) {
            earliest = earliest.min(pos);
        }
    }
    earliest
}

pub fn truncate_output_head_tail(text: &str, max_bytes: usize) -> (String, String) {
    let bytes = text.as_bytes();
    if bytes.len() <= max_bytes {
        return (text.to_string(), String::new());
    }
    let half = max_bytes / 2;
    let head = safe_byte_slice(text, 0, half);
    let tail_start = bytes.len().saturating_sub(half);
    let tail = safe_byte_slice(text, tail_start, bytes.len());
    (head, tail)
}

fn safe_byte_slice(text: &str, start: usize, end: usize) -> String {
    let mut s = start.min(text.len());
    let mut e = end.min(text.len());
    while s < text.len() && !text.is_char_boundary(s) {
        s += 1;
    }
    while e > s && !text.is_char_boundary(e) {
        e -= 1;
    }
    text[s..e].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_stable_ids() {
        let mut task = ActiveTask::new("demo".into(), vec!["user said foo".into()]);
        let a = task.allocate_criterion_id();
        let b = task.allocate_criterion_id();
        assert_ne!(a, b);
    }

    #[test]
    fn truncates_large_output_into_head_and_tail() {
        let text: String = (0..1000).map(|i| (b'a' + (i as u8 % 26)) as char).collect();
        let (head, tail) = truncate_output_head_tail(&text, 100);
        assert!(head.len() <= 100);
        assert!(tail.len() <= 100);
        assert!(head.len() + tail.len() <= 100);
    }

    #[test]
    fn short_outputs_are_preserved_in_head() {
        let (head, tail) = truncate_output_head_tail("hello", 100);
        assert_eq!(head, "hello");
        assert!(tail.is_empty());
    }

    #[test]
    fn parses_run_shell_script_output_with_both_streams() {
        let raw = "Command: echo hi; echo oops 1>&2\n\nWorking directory: .\n\nExit code: 0\n\nSTDOUT: 3 bytes across 1 line\nhi\n\nSTDERR: 5 bytes across 1 line\noops\n";
        let (exit, stdout, stderr) = parse_run_shell_script_output(raw);
        assert_eq!(exit, Some(0));
        assert_eq!(stdout, "hi");
        assert_eq!(stderr, "oops");
    }

    #[test]
    fn parses_run_shell_script_output_stdout_only() {
        let raw = "Command: ls\n\nWorking directory: .\n\nExit code: 0\n\nSTDOUT: 12 bytes across 1 line\nhello world\n";
        let (exit, stdout, stderr) = parse_run_shell_script_output(raw);
        assert_eq!(exit, Some(0));
        assert_eq!(stdout, "hello world");
        assert_eq!(stderr, "");
    }

    #[test]
    fn parses_run_shell_script_output_nonzero_exit() {
        let raw = "Command: false\n\nWorking directory: .\n\nExit code: 1\n\nOutput: (empty)";
        let (exit, stdout, stderr) = parse_run_shell_script_output(raw);
        assert_eq!(exit, Some(1));
        assert_eq!(stdout, "");
        assert_eq!(stderr, "");
    }

    #[test]
    fn falls_back_when_format_does_not_match() {
        let raw = "just a raw string with no headers";
        let (exit, stdout, stderr) = parse_run_shell_script_output(raw);
        assert_eq!(exit, None);
        assert_eq!(stdout, "just a raw string with no headers");
        assert_eq!(stderr, "");
    }
}
