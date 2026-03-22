use std::path::{Path, PathBuf};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use super::common::{
    ToolExecError, collect_visible_entries, display_path, is_path_visible, resolve_path,
};
use crate::tool_policy::SearchPathPolicy;

const MAX_LIST_ENTRIES: usize = 400;

#[derive(Clone)]
pub struct ListTool {
    root: PathBuf,
    policy: SearchPathPolicy,
}

#[derive(Debug, Deserialize)]
pub struct ListArgs {
    pub dir: String,
    pub recursive: Option<bool>,
}

impl ListTool {
    pub fn new(root: PathBuf, policy: SearchPathPolicy) -> Self {
        Self { root, policy }
    }
}

impl Tool for ListTool {
    const NAME: &'static str = "List";
    type Error = ToolExecError;
    type Args = ListArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List files and directories under a directory in the current workspace while respecting .gitignore plus the default search filters for hidden files/directories and common generated/package artifacts. Set recursive=true for a full tree.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "dir": {
                        "type": "string",
                        "description": "Directory path relative to the current workspace root."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Whether to recurse and return a tree of all nested files and directories."
                    }
                },
                "required": ["dir"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        list_directory(
            &self.root,
            &self.policy,
            &args.dir,
            args.recursive.unwrap_or(false),
        )
    }
}

pub(crate) fn list_directory(
    root: &Path,
    policy: &SearchPathPolicy,
    dir: &str,
    recursive: bool,
) -> Result<String, ToolExecError> {
    let target = resolve_path(root, dir)?;
    let metadata = std::fs::metadata(&target)?;
    if !metadata.is_dir() {
        return Err(ToolExecError::new(format!("{dir} is not a directory")));
    }
    if target != root && !is_path_visible(root, &target, policy)? {
        return Err(ToolExecError::new(SearchPathPolicy::excluded_message(dir)));
    }

    let mut lines = Vec::new();
    let display_root = display_path(root, &target);
    lines.push(format!("{display_root}/"));
    for entry in collect_visible_entries(&target, recursive, policy)? {
        if lines.len() >= MAX_LIST_ENTRIES {
            lines.push(format!(
                "{}... truncated after {MAX_LIST_ENTRIES} entries",
                "  ".repeat(entry.depth.max(1))
            ));
            break;
        }

        let mut label = display_path(root, &entry.path);
        if entry.is_dir {
            label.push('/');
        }
        lines.push(format!("{}{}", "  ".repeat(entry.depth), label));
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::{gitignored_tree, large_tree, sample_tree};

    #[test]
    fn list_directory_supports_recursive_tree() {
        let tree = sample_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let output = list_directory(&tree.root, &policy, ".", true).expect("list succeeds");

        assert!(output.contains("src/"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("src/nested/lib.rs"));
    }

    #[test]
    fn list_directory_respects_gitignore_patterns() {
        let tree = gitignored_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let output = list_directory(&tree.root, &policy, ".", true).expect("list succeeds");

        assert!(output.contains("visible.txt"));
        assert!(output.contains("src/lib.rs"));
        assert!(!output.contains("hidden.log"));
        assert!(!output.contains("ignored/secret.txt"));
        assert!(!output.contains("src/generated/skip.rs"));
    }

    #[test]
    fn list_directory_rejects_explicit_ignored_directory() {
        let tree = gitignored_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let error = list_directory(&tree.root, &policy, "ignored", true)
            .expect_err("ignored dir must fail");

        assert!(
            error
                .to_string()
                .contains("excluded by the default search filters")
        );
    }

    #[test]
    fn list_directory_truncates_large_trees() {
        let tree = large_tree(MAX_LIST_ENTRIES + 50);
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let output = list_directory(&tree.root, &policy, "files", true).expect("list succeeds");

        assert!(output.contains(&format!("... truncated after {MAX_LIST_ENTRIES} entries")));
    }

    #[test]
    fn list_directory_excludes_node_modules_by_default() {
        let tree = large_tree(1);
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        std::fs::create_dir_all(tree.root.join("node_modules/react")).expect("node_modules");
        std::fs::write(
            tree.root.join("node_modules/react/index.js"),
            "export {};\n",
        )
        .expect("generated file");

        let output = list_directory(&tree.root, &policy, ".", true).expect("list succeeds");

        assert!(!output.contains("node_modules/"));
        assert!(!output.contains("node_modules/react/index.js"));
    }

    #[test]
    fn list_directory_can_include_hidden_directory_when_configured() {
        let tree = sample_tree();
        let policy = SearchPathPolicy::new(&[".research/**".into()]).expect("policy builds");
        std::fs::create_dir_all(tree.root.join(".research")).expect("hidden dir");
        std::fs::write(tree.root.join(".research/findings.md"), "hello\n").expect("hidden file");

        let output = list_directory(&tree.root, &policy, ".", true).expect("list succeeds");

        assert!(output.contains(".research/"));
        assert!(output.contains(".research/findings.md"));
    }
}
