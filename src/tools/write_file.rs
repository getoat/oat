use std::path::{Path, PathBuf};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::{ToolExecError, display_path, resolve_workspace_path_with_access};

pub const WRITE_FILE_TOOL_NAME: &str = "WriteFile";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteFileTool {
    root: PathBuf,
    allow_full_system_access: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WriteFileArgs {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub intent: Option<String>,
}

impl WriteFileTool {
    #[cfg(test)]
    pub fn new(root: PathBuf) -> Self {
        Self::new_with_access(root, false)
    }

    pub fn new_with_access(root: PathBuf, allow_full_system_access: bool) -> Self {
        Self {
            root,
            allow_full_system_access,
        }
    }
}

impl Tool for WriteFileTool {
    const NAME: &'static str = WRITE_FILE_TOOL_NAME;
    type Error = ToolExecError;
    type Args = WriteFileArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Create a new workspace file with the provided content. Fails if the file already exists. Missing parent directories will be created automatically. Always include intent as a short plain-language reason that explains why the new file is needed.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filename": {
                        "type": "string",
                        "description": "File path relative to the current workspace root."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file contents to write."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Short sentence explaining why this new file is needed for the user. Focus on purpose or outcome, not the mechanical file creation."
                    }
                },
                "required": ["filename", "content", "intent"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        write_file_with_access(
            &self.root,
            &args.filename,
            &args.content,
            self.allow_full_system_access,
        )
    }
}

#[cfg(test)]
pub(crate) fn write_file(
    root: &Path,
    filename: &str,
    content: &str,
) -> Result<String, ToolExecError> {
    write_file_with_access(root, filename, content, false)
}

pub(crate) fn write_file_with_access(
    root: &Path,
    filename: &str,
    content: &str,
    allow_full_system_access: bool,
) -> Result<String, ToolExecError> {
    let path = resolve_workspace_path_with_access(root, filename, allow_full_system_access)?;
    if path == root.canonicalize()? {
        return Err(ToolExecError::new(
            "refusing to write to the workspace root",
        ));
    }
    if path.exists() {
        return Err(ToolExecError::new(format!(
            "{} already exists; use ApplyPatches to modify existing files",
            display_path(root, &path)
        )));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;

    Ok(format!("Wrote {}.", display_path(root, &path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::{TempTree, tree_with_mutation_targets};

    #[test]
    fn write_file_creates_missing_parent_directories() {
        let tree = TempTree::new();

        let output = write_file(&tree.root, "nested/deep/file.txt", "hello").expect("write");

        assert_eq!(output, "Wrote nested/deep/file.txt.");
        let written =
            std::fs::read_to_string(tree.root.join("nested/deep/file.txt")).expect("file exists");
        assert_eq!(written, "hello");
    }

    #[test]
    fn write_file_rejects_existing_file() {
        let tree = tree_with_mutation_targets();

        let error = write_file(&tree.root, "src/lib.rs", "replacement\n").expect_err("write");

        assert!(
            error
                .to_string()
                .contains("already exists; use ApplyPatches to modify existing files")
        );
    }

    #[test]
    fn mutation_path_resolution_rejects_workspace_escape() {
        let tree = TempTree::new();

        let error = write_file(&tree.root, "../escape.txt", "bad").expect_err("escape must fail");

        assert!(
            error
                .to_string()
                .contains("escapes the current workspace root")
        );
    }

    #[test]
    fn mutation_path_resolution_allows_workspace_escape_with_full_access() {
        let tree = TempTree::new();
        let outside = tree.root.with_extension("outside.txt");

        let output = write_file_with_access(
            &tree.root,
            outside.to_str().expect("utf-8 path"),
            "hello",
            true,
        )
        .expect("write succeeds");

        assert_eq!(output, format!("Wrote {}.", outside.display()));
        assert_eq!(
            std::fs::read_to_string(&outside).expect("file exists"),
            "hello"
        );
        let _ = std::fs::remove_file(outside);
    }

    #[tokio::test]
    async fn mutation_tool_definition_requires_intent() {
        let definition = WriteFileTool::new(PathBuf::from("."))
            .definition(String::new())
            .await;

        assert_eq!(
            definition.parameters["properties"]["intent"]["type"],
            "string"
        );
        assert!(
            definition.parameters["required"]
                .as_array()
                .expect("required array")
                .iter()
                .any(|value| value == "intent")
        );
    }
}
