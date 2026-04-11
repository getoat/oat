use std::{
    collections::VecDeque,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    task::JoinHandle,
    time::timeout,
};

use super::{
    common::ToolExecError,
    shell_command::{ShellCommandRequest, display_shell_command, resolve_shell_cwd},
};

pub const RUN_SHELL_SCRIPT_TOOL_NAME: &str = "RunShellScript";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const OUTPUT_HEAD_BYTES: usize = 16 * 1024;
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;
const READ_CHUNK_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunShellScriptTool {
    root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RunShellScriptArgs {
    #[serde(flatten)]
    pub command: ShellCommandRequest,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct StreamCapture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total_bytes: usize,
    newline_count: usize,
    ends_with_newline: bool,
}

impl StreamCapture {
    fn push(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }

        self.total_bytes += chunk.len();
        self.newline_count += chunk.iter().filter(|&&byte| byte == b'\n').count();
        self.ends_with_newline = chunk.last().copied() == Some(b'\n');

        let head_remaining = OUTPUT_HEAD_BYTES.saturating_sub(self.head.len());
        if head_remaining > 0 {
            self.head
                .extend_from_slice(&chunk[..chunk.len().min(head_remaining)]);
        }

        for byte in chunk {
            if self.tail.len() == OUTPUT_TAIL_BYTES {
                self.tail.pop_front();
            }
            self.tail.push_back(*byte);
        }
    }

    fn is_empty(&self) -> bool {
        self.total_bytes == 0
    }

    fn line_count(&self) -> usize {
        if self.total_bytes == 0 {
            0
        } else {
            self.newline_count + usize::from(!self.ends_with_newline)
        }
    }

    fn excerpt(&self) -> String {
        let tail_bytes = self.tail.iter().copied().collect::<Vec<_>>();
        let overlap = self
            .head
            .len()
            .saturating_add(tail_bytes.len())
            .saturating_sub(self.total_bytes);
        let suffix = &tail_bytes[overlap.min(tail_bytes.len())..];
        let head = String::from_utf8_lossy(&self.head);
        let suffix = String::from_utf8_lossy(suffix);

        if self.total_bytes > OUTPUT_HEAD_BYTES + OUTPUT_TAIL_BYTES {
            format!("{head}\n\n[... omitted middle output ...]\n\n{suffix}",)
        } else {
            format!("{head}{suffix}")
        }
    }

    fn render_section(&self, label: &str) -> Option<String> {
        if self.is_empty() {
            return None;
        }

        let mut section = format!(
            "{label}: {} bytes across {} line{}",
            self.total_bytes,
            self.line_count(),
            if self.line_count() == 1 { "" } else { "s" }
        );
        if self.total_bytes > OUTPUT_HEAD_BYTES + OUTPUT_TAIL_BYTES {
            section.push_str(" (excerpt truncated)");
        }
        section.push_str("\n");
        section.push_str(&self.excerpt());
        Some(section)
    }
}

impl RunShellScriptTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl Tool for RunShellScriptTool {
    const NAME: &'static str = RUN_SHELL_SCRIPT_TOOL_NAME;
    type Error = ToolExecError;
    type Args = RunShellScriptArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Execute a focused bash script from the workspace. In read-only mode only low-risk commands can be approved. Always include intent as a short plain-language reason for running the command. Prefer concise, auditable scripts over long shell sessions.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "The bash script to execute with `bash -lc`."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the current workspace root."
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "description": "Optional timeout in milliseconds. Defaults to 30000."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Short sentence explaining why the command is needed."
                    }
                },
                "required": ["script", "intent"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        run_shell_script(&self.root, &args).await
    }
}

pub(crate) async fn run_shell_script(
    root: &Path,
    args: &RunShellScriptArgs,
) -> Result<String, ToolExecError> {
    let cwd = resolve_shell_cwd(root, args.command.cwd.as_deref())?;
    let cwd_label = args.command.cwd_label(root)?;
    let display_command = display_shell_command(args.command.script.as_str());
    let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1);

    let mut command = Command::new("bash");
    command.arg("-lc");
    command.arg(args.command.script.as_str());
    command.current_dir(&cwd);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| ToolExecError::new(format!("failed to launch shell command: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolExecError::new("failed to capture stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolExecError::new("failed to capture stderr"))?;

    let stdout_task = tokio::spawn(capture_stream(stdout));
    let stderr_task = tokio::spawn(capture_stream(stderr));

    let status = match timeout(Duration::from_millis(timeout_ms), child.wait()).await {
        Ok(status) => status.map_err(|error| {
            ToolExecError::new(format!("failed to wait on shell command: {error}"))
        })?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let stdout = join_capture(stdout_task).await?;
            let stderr = join_capture(stderr_task).await?;
            return Err(ToolExecError::new(format_timeout_result(
                &display_command,
                &cwd_label,
                timeout_ms,
                &stdout,
                &stderr,
            )));
        }
    };

    let stdout = join_capture(stdout_task).await?;
    let stderr = join_capture(stderr_task).await?;

    Ok(format_shell_result(
        &display_command,
        &cwd_label,
        status.code(),
        &stdout,
        &stderr,
    ))
}

async fn capture_stream<R>(mut reader: R) -> io::Result<StreamCapture>
where
    R: AsyncRead + Unpin,
{
    let mut capture = StreamCapture::default();
    let mut buffer = vec![0_u8; READ_CHUNK_BYTES];

    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(capture);
        }
        capture.push(&buffer[..read]);
    }
}

async fn join_capture(
    handle: JoinHandle<io::Result<StreamCapture>>,
) -> Result<StreamCapture, ToolExecError> {
    handle
        .await
        .map_err(|error| ToolExecError::new(format!("failed to capture process output: {error}")))?
        .map_err(|error| ToolExecError::new(format!("failed to read process output: {error}")))
}

fn format_shell_result(
    command: &str,
    cwd: &str,
    exit_code: Option<i32>,
    stdout: &StreamCapture,
    stderr: &StreamCapture,
) -> String {
    let mut sections = vec![
        format!("Command: {command}"),
        format!("Working directory: {cwd}"),
        format!(
            "Exit code: {}",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated by signal".into())
        ),
    ];

    if let Some(section) = stdout.render_section("STDOUT") {
        sections.push(section);
    }
    if let Some(section) = stderr.render_section("STDERR") {
        sections.push(section);
    }
    if stdout.is_empty() && stderr.is_empty() {
        sections.push("Output: (empty)".into());
    }

    sections.join("\n\n")
}

fn format_timeout_result(
    command: &str,
    cwd: &str,
    timeout_ms: u64,
    stdout: &StreamCapture,
    stderr: &StreamCapture,
) -> String {
    let mut sections = vec![
        format!("Shell command timed out after {timeout_ms}ms."),
        format!("Command: {command}"),
        format!("Working directory: {cwd}"),
    ];

    if let Some(section) = stdout.render_section("Partial STDOUT") {
        sections.push(section);
    }
    if let Some(section) = stderr.render_section("Partial STDERR") {
        sections.push(section);
    }
    if stdout.is_empty() && stderr.is_empty() {
        sections.push("Output: (empty)".into());
    }

    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::TempTree;
    use crate::tools::display_requested_shell_cwd;

    #[tokio::test]
    async fn run_shell_script_executes_in_workspace_root() {
        let tree = TempTree::new();
        std::fs::write(tree.root.join("note.txt"), "hello\n").expect("fixture");
        let args = RunShellScriptArgs {
            command: ShellCommandRequest {
                script: "pwd && cat note.txt".into(),
                cwd: None,
                intent: "inspect workspace".into(),
            },
            timeout_ms: Some(5_000),
        };

        let output = run_shell_script(&tree.root, &args).await.expect("command");

        assert!(output.contains("Working directory: ."));
        assert!(output.contains("hello"));
    }

    #[tokio::test]
    async fn run_shell_script_honors_relative_cwd() {
        let tree = TempTree::new();
        std::fs::create_dir_all(tree.root.join("nested")).expect("dir");
        std::fs::write(tree.root.join("nested/file.txt"), "nested\n").expect("fixture");
        let args = RunShellScriptArgs {
            command: ShellCommandRequest {
                script: "pwd && cat file.txt".into(),
                cwd: Some("nested".into()),
                intent: "inspect nested dir".into(),
            },
            timeout_ms: Some(5_000),
        };

        let output = run_shell_script(&tree.root, &args).await.expect("command");

        assert!(output.contains("Working directory: nested"));
        assert!(output.contains("nested"));
    }

    #[test]
    fn resolve_shell_cwd_rejects_workspace_escape() {
        let tree = TempTree::new();
        let error = resolve_shell_cwd(&tree.root, Some("../outside")).expect_err("must fail");

        assert!(
            error
                .to_string()
                .contains("escapes the current workspace root")
        );
    }

    #[tokio::test]
    async fn run_shell_script_times_out() {
        let tree = TempTree::new();
        let args = RunShellScriptArgs {
            command: ShellCommandRequest {
                script: "printf start && sleep 1".into(),
                cwd: None,
                intent: "timeout".into(),
            },
            timeout_ms: Some(10),
        };

        let error = run_shell_script(&tree.root, &args)
            .await
            .expect_err("must time out");
        assert!(error.to_string().contains("timed out"));
        assert!(error.to_string().contains("Partial STDOUT"));
    }

    #[tokio::test]
    async fn run_shell_script_truncates_large_output() {
        let tree = TempTree::new();
        let args = RunShellScriptArgs {
            command: ShellCommandRequest {
                script: "for _ in $(seq 1 40000); do printf x; done".into(),
                cwd: None,
                intent: "large output".into(),
            },
            timeout_ms: Some(5_000),
        };

        let output = run_shell_script(&tree.root, &args).await.expect("command");

        assert!(output.contains("excerpt truncated"));
        assert!(output.contains("omitted middle output"));
    }

    #[test]
    fn display_shell_command_renders_cwd_and_script() {
        assert_eq!(display_shell_command("ls -la"), "ls -la");
    }

    #[test]
    fn display_requested_shell_cwd_defaults_to_workspace_root() {
        assert_eq!(display_requested_shell_cwd(None), ".");
        assert_eq!(display_requested_shell_cwd(Some(" src ")), "src");
    }
}
