use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use regex::Regex;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use super::common::{
    ToolExecError, collect_visible_entries, display_path, is_path_visible, resolve_path,
};
use crate::tool_policy::SearchPathPolicy;

const MAX_GREP_MATCHES: usize = 100;

#[derive(Clone)]
pub struct GrepTool {
    root: PathBuf,
    policy: SearchPathPolicy,
}

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: String,
    pub recursive: Option<bool>,
}

impl GrepTool {
    pub fn new(root: PathBuf, policy: SearchPathPolicy) -> Self {
        Self { root, policy }
    }
}

impl Tool for GrepTool {
    const NAME: &'static str = "Grep";
    type Error = ToolExecError;
    type Args = GrepArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Search files in the current workspace using a regular expression pattern while respecting .gitignore plus the default search filters for hidden files/directories and common generated/package artifacts. Returns up to {MAX_GREP_MATCHES} matches as path:line:text."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Rust regex pattern to search for."
                    },
                    "path": {
                        "type": "string",
                        "description": "File or directory path relative to the current workspace root."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Whether to recurse into subdirectories when path is a directory. Defaults to true."
                    }
                },
                "required": ["pattern", "path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        grep_workspace(
            &self.root,
            &self.policy,
            &args.pattern,
            &args.path,
            args.recursive.unwrap_or(true),
        )
    }
}

pub(crate) fn grep_workspace(
    root: &Path,
    policy: &SearchPathPolicy,
    pattern: &str,
    path: &str,
    recursive: bool,
) -> Result<String, ToolExecError> {
    let regex = Regex::new(pattern)?;
    let target = resolve_path(root, path)?;
    let metadata = std::fs::metadata(&target)?;
    if target != root && !is_path_visible(root, &target, policy)? {
        return Err(ToolExecError::new(SearchPathPolicy::excluded_message(path)));
    }
    let files = if metadata.is_file() {
        vec![target]
    } else if metadata.is_dir() {
        collect_visible_entries(&target, recursive, policy)?
            .into_iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.path)
            .collect::<Vec<_>>()
    } else {
        return Err(ToolExecError::new(format!(
            "{path} is neither a file nor a directory"
        )));
    };

    let mut matches = Vec::new();
    for file in files {
        if matches.len() >= MAX_GREP_MATCHES {
            break;
        }

        let opened = match std::fs::File::open(&file) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let reader = BufReader::new(opened);

        for (line_number, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(line) => line,
                Err(_) => break,
            };

            if regex.is_match(&line) {
                matches.push(format!(
                    "{}:{}:{}",
                    display_path(root, &file),
                    line_number + 1,
                    line
                ));
            }

            if matches.len() >= MAX_GREP_MATCHES {
                break;
            }
        }
    }

    if matches.is_empty() {
        return Ok(format!("No matches for /{pattern}/ in {path}."));
    }

    if matches.len() == MAX_GREP_MATCHES {
        matches.push(format!("... truncated after {MAX_GREP_MATCHES} matches"));
    }

    Ok(matches.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::{TempTree, gitignored_tree, sample_tree};

    #[test]
    fn grep_returns_matching_lines() {
        let tree = sample_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let output = grep_workspace(&tree.root, &policy, "TODO", ".", true).expect("grep succeeds");

        assert!(output.contains("src/nested/lib.rs:2:// TODO: grep target"));
    }

    #[test]
    fn grep_respects_gitignore_patterns() {
        let tree = gitignored_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let output =
            grep_workspace(&tree.root, &policy, "needle", ".", true).expect("grep succeeds");

        assert!(output.contains("visible.txt:1:needle visible"));
        assert!(!output.contains("hidden.log"));
        assert!(!output.contains("ignored/secret.txt"));
    }

    #[test]
    fn grep_rejects_explicit_ignored_file() {
        let tree = gitignored_tree();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let error = grep_workspace(&tree.root, &policy, "needle", "hidden.log", true)
            .expect_err("ignored file must fail");

        assert!(
            error
                .to_string()
                .contains("excluded by the default search filters")
        );
    }

    #[test]
    fn grep_truncates_large_match_sets() {
        let tree = TempTree::new();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        let content = (0..(MAX_GREP_MATCHES + 25))
            .map(|index| format!("match line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tree.root.join("many.txt"), content).expect("matches file");

        let output = grep_workspace(&tree.root, &policy, "match line", "many.txt", true)
            .expect("grep succeeds");

        assert!(output.contains(&format!("... truncated after {MAX_GREP_MATCHES} matches")));
    }

    #[test]
    fn grep_excludes_node_modules_by_default() {
        let tree = TempTree::new();
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");
        std::fs::create_dir_all(tree.root.join("node_modules/react")).expect("node_modules");
        std::fs::write(
            tree.root.join("node_modules/react/index.js"),
            "dangerouslySetInnerHTML\n",
        )
        .expect("write generated file");
        std::fs::write(tree.root.join("src.txt"), "dangerouslySetInnerHTML\n")
            .expect("write source file");

        let output = grep_workspace(&tree.root, &policy, "dangerouslySetInnerHTML", ".", true)
            .expect("grep succeeds");

        assert!(output.contains("src.txt:1:dangerouslySetInnerHTML"));
        assert!(!output.contains("node_modules/react/index.js"));
    }

    #[test]
    fn grep_can_include_hidden_directory_when_configured() {
        let tree = TempTree::new();
        let policy = SearchPathPolicy::new(&[".research/**".into()]).expect("policy builds");
        std::fs::create_dir_all(tree.root.join(".research")).expect("hidden dir");
        std::fs::write(tree.root.join(".research/findings.md"), "needle hidden\n")
            .expect("hidden file");

        let output = grep_workspace(&tree.root, &policy, "needle", ".research", true)
            .expect("grep succeeds");

        assert!(output.contains(".research/findings.md:1:needle hidden"));
    }
}
