use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::{ToolExecError, display_path, is_unsafe_system_path, resolve_path_with_access};

pub(crate) const MAX_READFILE_LIMIT: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFileTool {
    root: PathBuf,
    allow_full_system_access: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReadFileArgs {
    pub filename: String,
    pub offset: usize,
    pub limit: usize,
}

impl ReadFileTool {
    pub fn new_with_access(root: PathBuf, allow_full_system_access: bool) -> Self {
        Self {
            root,
            allow_full_system_access,
        }
    }
}

impl Tool for ReadFileTool {
    const NAME: &'static str = "ReadFile";
    type Error = ToolExecError;
    type Args = ReadFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Read lines from a file in the current workspace. offset is zero-based, limit is in lines, and limit must be <= {MAX_READFILE_LIMIT}."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filename": {
                        "type": "string",
                        "description": "File path relative to the current workspace root."
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Zero-based starting line offset."
                    },
                    "limit": {
                        "type": "integer",
                        "description": format!("Number of lines to read. Must be between 1 and {MAX_READFILE_LIMIT}.")
                    }
                },
                "required": ["filename", "offset", "limit"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        read_file_lines_with_access(
            &self.root,
            &args.filename,
            args.offset,
            args.limit,
            self.allow_full_system_access,
        )
    }
}

#[cfg(test)]
pub(crate) fn read_file_lines(
    root: &Path,
    filename: &str,
    offset: usize,
    limit: usize,
) -> Result<String, ToolExecError> {
    read_file_lines_with_access(root, filename, offset, limit, false)
}

pub(crate) fn read_file_lines_with_access(
    root: &Path,
    filename: &str,
    offset: usize,
    limit: usize,
    allow_full_system_access: bool,
) -> Result<String, ToolExecError> {
    if limit == 0 || limit > MAX_READFILE_LIMIT {
        return Err(ToolExecError::new(format!(
            "limit must be between 1 and {MAX_READFILE_LIMIT}"
        )));
    }

    let path = resolve_path_with_access(root, filename, allow_full_system_access)?;
    if is_unsafe_system_path(&path) {
        return Err(ToolExecError::new(format!(
            "{filename} points to an unsupported system path"
        )));
    }
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(ToolExecError::new(format!("{filename} is not a file")));
    }

    let file = std::fs::File::open(&path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();

    for (index, line) in reader.lines().enumerate().skip(offset).take(limit) {
        let line = line?;
        lines.push(format!("{:>6} | {}", index + 1, line));
    }

    if lines.is_empty() {
        return Ok(format!(
            "No lines returned from {} at offset {} with limit {}.",
            display_path(root, &path),
            offset,
            limit
        ));
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::sample_tree;

    #[test]
    fn read_file_formats_line_numbers_and_honors_limit() {
        let tree = sample_tree();
        let output = read_file_lines(&tree.root, "README.md", 0, 2).expect("read succeeds");

        assert!(output.contains("1 | hello"));
        assert!(output.contains("2 | world"));
    }

    #[test]
    fn read_file_rejects_large_limits() {
        let tree = sample_tree();
        let error = read_file_lines(&tree.root, "README.md", 0, 301).expect_err("must fail");
        assert!(
            error
                .to_string()
                .contains("limit must be between 1 and 300")
        );
    }

    #[test]
    fn read_file_rejects_proc_like_system_paths() {
        let tree = sample_tree();
        let error = read_file_lines_with_access(&tree.root, "/proc/version", 0, 1, true)
            .expect_err("proc-like path must fail");
        assert!(error.to_string().contains("unsupported system path"));
    }
}
