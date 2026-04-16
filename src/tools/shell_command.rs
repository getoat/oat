use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::common::{ToolExecError, resolve_workspace_path_with_access};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ShellCommandRequest {
    pub script: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub intent: String,
}

impl ShellCommandRequest {
    pub fn resolve_cwd_with_access(
        &self,
        root: &Path,
        allow_full_system_access: bool,
    ) -> Result<PathBuf, ToolExecError> {
        resolve_shell_cwd_with_access(root, self.cwd.as_deref(), allow_full_system_access)
    }

    pub fn cwd_label_with_access(
        &self,
        root: &Path,
        allow_full_system_access: bool,
    ) -> Result<String, ToolExecError> {
        let cwd = self.resolve_cwd_with_access(root, allow_full_system_access)?;
        Ok(display_shell_cwd(root, &cwd))
    }

    pub fn display_command(&self) -> String {
        display_shell_command(&self.script)
    }
}

#[cfg(test)]
pub(crate) fn resolve_shell_cwd(
    root: &Path,
    raw_cwd: Option<&str>,
) -> Result<PathBuf, ToolExecError> {
    resolve_shell_cwd_with_access(root, raw_cwd, false)
}

pub(crate) fn resolve_shell_cwd_with_access(
    root: &Path,
    raw_cwd: Option<&str>,
    allow_full_system_access: bool,
) -> Result<PathBuf, ToolExecError> {
    raw_cwd
        .map(|cwd| resolve_workspace_path_with_access(root, cwd, allow_full_system_access))
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
