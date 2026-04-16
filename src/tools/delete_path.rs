use std::path::{Path, PathBuf};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::{ToolExecError, display_path, resolve_workspace_path_with_access};

pub const DELETE_PATH_TOOL_NAME: &str = "DeletePath";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletePathTool {
    root: PathBuf,
    allow_full_system_access: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DeletePathArgs {
    pub path: String,
    #[serde(default)]
    pub intent: Option<String>,
}

impl DeletePathTool {
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

impl Tool for DeletePathTool {
    const NAME: &'static str = DELETE_PATH_TOOL_NAME;
    type Error = ToolExecError;
    type Args = DeletePathArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Delete a file or directory in the current workspace. Directory deletion is recursive. Always include intent as a short plain-language reason that explains why the removal is needed.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File or directory path relative to the current workspace root."
                    },
                    "intent": {
                        "type": "string",
                        "description": "Short sentence explaining why this path should be removed for the user. Focus on purpose or outcome, not the mechanical deletion."
                    }
                },
                "required": ["path", "intent"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        delete_path_with_access(&self.root, &args.path, self.allow_full_system_access)
    }
}

pub(crate) fn delete_path(root: &Path, raw_path: &str) -> Result<String, ToolExecError> {
    delete_path_with_access(root, raw_path, false)
}

pub(crate) fn delete_path_with_access(
    root: &Path,
    raw_path: &str,
    allow_full_system_access: bool,
) -> Result<String, ToolExecError> {
    let path = resolve_workspace_path_with_access(root, raw_path, allow_full_system_access)?;
    let canonical_root = root.canonicalize()?;
    if path == canonical_root {
        return Err(ToolExecError::new("refusing to delete the workspace root"));
    }
    if path == Path::new("/") {
        return Err(ToolExecError::new("refusing to delete the filesystem root"));
    }

    let metadata = std::fs::metadata(&path)?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(&path)?;
        Ok(format!("Deleted directory {}.", display_path(root, &path)))
    } else {
        std::fs::remove_file(&path)?;
        Ok(format!("Deleted file {}.", display_path(root, &path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::TempTree;

    #[test]
    fn delete_path_removes_files_and_directories() {
        let tree = TempTree::new();
        std::fs::create_dir_all(tree.root.join("dir/sub")).expect("dir");
        std::fs::write(tree.root.join("dir/sub/file.txt"), "hello").expect("file");

        let dir_output = delete_path(&tree.root, "dir").expect("delete dir");
        assert_eq!(dir_output, "Deleted directory dir.");
        assert!(!tree.root.join("dir").exists());

        std::fs::write(tree.root.join("file.txt"), "hello").expect("file");
        let file_output = delete_path(&tree.root, "file.txt").expect("delete file");
        assert_eq!(file_output, "Deleted file file.txt.");
        assert!(!tree.root.join("file.txt").exists());
    }

    #[test]
    fn delete_path_rejects_workspace_root() {
        let tree = TempTree::new();

        let error = delete_path(&tree.root, ".").expect_err("root delete must fail");

        assert!(error.to_string().contains("workspace root"));
    }

    #[tokio::test]
    async fn mutation_tool_definition_requires_intent() {
        let definition = DeletePathTool::new(PathBuf::from("."))
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
