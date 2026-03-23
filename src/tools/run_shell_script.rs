use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{process::Command, time::timeout};

use super::common::{ToolExecError, resolve_workspace_path};

pub const RUN_SHELL_SCRIPT_TOOL_NAME: &str = "RunShellScript";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunShellScriptTool {
    root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RunShellScriptArgs {
    pub script: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    pub intent: String,
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
    let cwd = resolve_shell_cwd(root, args.cwd.as_deref())?;
    let cwd_label = display_shell_cwd(root, &cwd);
    let display_command = display_shell_command(args.script.as_str());
    let timeout_ms = args.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1);

    let mut child = Command::new("bash");
    child.arg("-lc");
    child.arg(args.script.as_str());
    child.current_dir(&cwd);
    child.stdout(Stdio::piped());
    child.stderr(Stdio::piped());

    let output = timeout(Duration::from_millis(timeout_ms), child.output())
        .await
        .map_err(|_| {
            ToolExecError::new(format!(
                "Shell command timed out after {timeout_ms}ms.\nCommand: {display_command}\nWorking directory: {cwd_label}"
            ))
        })?
        .map_err(|error| ToolExecError::new(format!("failed to launch shell command: {error}")))?;

    Ok(format_shell_result(
        &display_command,
        &cwd_label,
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).as_ref(),
        String::from_utf8_lossy(&output.stderr).as_ref(),
    ))
}

pub(crate) fn resolve_shell_cwd(
    root: &Path,
    raw_cwd: Option<&str>,
) -> Result<PathBuf, ToolExecError> {
    raw_cwd
        .map(|cwd| resolve_workspace_path(root, cwd))
        .transpose()
        .map(|cwd| cwd.unwrap_or_else(|| root.to_path_buf()))
}

pub(crate) fn display_shell_cwd(root: &Path, cwd: &Path) -> String {
    match cwd.strip_prefix(root) {
        Ok(path) if path.as_os_str().is_empty() => ".".into(),
        Ok(path) => path.display().to_string(),
        Err(_) => cwd.display().to_string(),
    }
}

pub fn display_shell_command(script: &str) -> String {
    script.to_string()
}

pub fn display_requested_shell_cwd(raw_cwd: Option<&str>) -> String {
    raw_cwd
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .unwrap_or(".")
        .to_string()
}

fn format_shell_result(
    command: &str,
    cwd: &str,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
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

    if !stdout.is_empty() {
        sections.push(format!("STDOUT:\n{stdout}"));
    }
    if !stderr.is_empty() {
        sections.push(format!("STDERR:\n{stderr}"));
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

    #[tokio::test]
    async fn run_shell_script_executes_in_workspace_root() {
        let tree = TempTree::new();
        std::fs::write(tree.root.join("note.txt"), "hello\n").expect("fixture");
        let args = RunShellScriptArgs {
            script: "pwd && cat note.txt".into(),
            cwd: None,
            timeout_ms: Some(5_000),
            intent: "inspect workspace".into(),
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
            script: "pwd && cat file.txt".into(),
            cwd: Some("nested".into()),
            timeout_ms: Some(5_000),
            intent: "inspect nested dir".into(),
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
            script: "sleep 1".into(),
            cwd: None,
            timeout_ms: Some(10),
            intent: "timeout".into(),
        };

        let error = run_shell_script(&tree.root, &args)
            .await
            .expect_err("must time out");
        assert!(error.to_string().contains("timed out"));
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
