use std::{
    error::Error,
    fmt, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use ignore::WalkBuilder;
use regex::Regex;
use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

const MAX_READFILE_LIMIT: usize = 300;
const MAX_GREP_MATCHES: usize = 100;
const MAX_LIST_ENTRIES: usize = 400;

#[derive(Debug)]
struct VisibleEntry {
    path: PathBuf,
    depth: usize,
    is_dir: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListTool {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFileTool {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFilesTool {
    root: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrepTool {
    root: PathBuf,
}

impl ListTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl ReadFileTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl ReadFilesTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl GrepTool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListArgs {
    pub dir: String,
    pub recursive: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ReadFileArgs {
    pub filename: String,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Deserialize)]
pub struct ReadFilesArgs {
    pub files: Vec<ReadFileArgs>,
}

#[derive(Debug, Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    pub path: String,
    pub recursive: Option<bool>,
}

#[derive(Debug)]
pub struct ToolExecError(String);

impl ToolExecError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ToolExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for ToolExecError {}

impl From<std::io::Error> for ToolExecError {
    fn from(value: std::io::Error) -> Self {
        Self::new(value.to_string())
    }
}

impl From<regex::Error> for ToolExecError {
    fn from(value: regex::Error) -> Self {
        Self::new(value.to_string())
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
            description: "List files and directories under a directory in the current workspace while respecting .gitignore rules. Set recursive=true for a full tree.".to_string(),
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
        list_directory(&self.root, &args.dir, args.recursive.unwrap_or(false))
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
        read_file_lines(&self.root, &args.filename, args.offset, args.limit)
    }
}

impl Tool for ReadFilesTool {
    const NAME: &'static str = "ReadFiles";
    type Error = ToolExecError;
    type Args = ReadFilesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Read lines from up to 5 files in the current workspace in one call. Each file entry has its own zero-based offset and line limit, and each limit must be <= {MAX_READFILE_LIMIT}."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "description": "Files to read. Provide between 1 and 5 entries.",
                        "minItems": 1,
                        "maxItems": 5,
                        "items": {
                            "type": "object",
                            "properties": {
                                "filename": {
                                    "type": "string",
                                    "description": "File path relative to the current workspace root."
                                },
                                "offset": {
                                    "type": "integer",
                                    "description": "Zero-based starting line offset for this file."
                                },
                                "limit": {
                                    "type": "integer",
                                    "description": format!("Number of lines to read from this file. Must be between 1 and {MAX_READFILE_LIMIT}.")
                                }
                            },
                            "required": ["filename", "offset", "limit"]
                        }
                    }
                },
                "required": ["files"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        read_files_lines(&self.root, &args.files)
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
                "Search files in the current workspace using a regular expression pattern while respecting .gitignore rules. Returns up to {MAX_GREP_MATCHES} matches as path:line:text."
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
            &args.pattern,
            &args.path,
            args.recursive.unwrap_or(true),
        )
    }
}

fn list_directory(root: &Path, dir: &str, recursive: bool) -> Result<String, ToolExecError> {
    let target = resolve_path(root, dir)?;
    let metadata = fs::metadata(&target)?;
    if !metadata.is_dir() {
        return Err(ToolExecError::new(format!("{dir} is not a directory")));
    }
    if target != root && !is_path_visible(root, &target)? {
        return Err(ToolExecError::new(format!(
            "{dir} is ignored by .gitignore"
        )));
    }

    let mut lines = Vec::new();
    let display_root = display_path(root, &target);
    lines.push(format!("{display_root}/"));
    for entry in collect_visible_entries(&target, recursive)? {
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

fn read_file_lines(
    root: &Path,
    filename: &str,
    offset: usize,
    limit: usize,
) -> Result<String, ToolExecError> {
    if limit == 0 || limit > MAX_READFILE_LIMIT {
        return Err(ToolExecError::new(format!(
            "limit must be between 1 and {MAX_READFILE_LIMIT}"
        )));
    }

    let path = resolve_path(root, filename)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(ToolExecError::new(format!("{filename} is not a file")));
    }

    let file = fs::File::open(&path)?;
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

fn read_files_lines(root: &Path, files: &[ReadFileArgs]) -> Result<String, ToolExecError> {
    if files.is_empty() || files.len() > 5 {
        return Err(ToolExecError::new(
            "files must contain between 1 and 5 entries",
        ));
    }

    let mut sections = Vec::with_capacity(files.len());
    for file in files {
        let content = read_file_lines(root, &file.filename, file.offset, file.limit)?;
        sections.push(format!("==> {} <==\n{content}", file.filename));
    }

    Ok(sections.join("\n\n"))
}

fn grep_workspace(
    root: &Path,
    pattern: &str,
    path: &str,
    recursive: bool,
) -> Result<String, ToolExecError> {
    let regex = Regex::new(pattern)?;
    let target = resolve_path(root, path)?;
    let metadata = fs::metadata(&target)?;
    if target != root && !is_path_visible(root, &target)? {
        return Err(ToolExecError::new(format!(
            "{path} is ignored by .gitignore"
        )));
    }
    let files = if metadata.is_file() {
        vec![target]
    } else if metadata.is_dir() {
        collect_visible_entries(&target, recursive)?
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

        let opened = match fs::File::open(&file) {
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

fn collect_visible_entries(
    target: &Path,
    recursive: bool,
) -> Result<Vec<VisibleEntry>, ToolExecError> {
    let mut builder = WalkBuilder::new(target);
    builder.hidden(false);
    builder.ignore(false);
    builder.git_ignore(true);
    builder.git_global(false);
    builder.git_exclude(false);
    builder.parents(true);
    builder.require_git(false);
    if !recursive {
        builder.max_depth(Some(1));
    }

    let mut entries = Vec::new();
    for result in builder.build() {
        let entry = result.map_err(|error| ToolExecError::new(error.to_string()))?;
        let path = entry.path();
        if path == target {
            continue;
        }

        let depth = path
            .strip_prefix(target)
            .map_err(|error| ToolExecError::new(error.to_string()))?
            .components()
            .count();
        let is_dir = entry
            .file_type()
            .is_some_and(|file_type| file_type.is_dir());

        entries.push(VisibleEntry {
            path: path.to_path_buf(),
            depth,
            is_dir,
        });
    }

    entries.sort_by(|left, right| {
        left.path
            .parent()
            .cmp(&right.path.parent())
            .then_with(|| right.is_dir.cmp(&left.is_dir))
            .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
    });

    Ok(entries)
}

fn is_path_visible(root: &Path, target: &Path) -> Result<bool, ToolExecError> {
    let max_depth = target
        .strip_prefix(root)
        .map_err(|error| ToolExecError::new(error.to_string()))?
        .components()
        .count();
    let mut builder = WalkBuilder::new(root);
    builder.hidden(false);
    builder.ignore(false);
    builder.git_ignore(true);
    builder.git_global(false);
    builder.git_exclude(false);
    builder.parents(true);
    builder.require_git(false);
    builder.max_depth(Some(max_depth));

    for result in builder.build() {
        let entry = result.map_err(|error| ToolExecError::new(error.to_string()))?;
        if entry.path() == target {
            return Ok(true);
        }
    }

    Ok(false)
}

fn resolve_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolExecError> {
    let joined = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        root.join(raw_path)
    };

    let canonical_root = root.canonicalize()?;
    let canonical_path = joined.canonicalize()?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(ToolExecError::new(format!(
            "path {raw_path} escapes the current workspace root"
        )));
    }

    Ok(canonical_path)
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time works")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("oat-tools-{unique}"));
            fs::create_dir_all(&root).expect("temp root created");
            Self { root }
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn sample_tree() -> TempTree {
        let tree = TempTree::new();
        fs::create_dir_all(tree.root.join("src/nested")).expect("dirs created");
        fs::write(
            tree.root.join("src/main.rs"),
            "fn main() {}\nprintln!(\"hi\");\n",
        )
        .expect("main file");
        fs::write(
            tree.root.join("src/nested/lib.rs"),
            "pub fn helper() {}\n// TODO: grep target\n",
        )
        .expect("lib file");
        fs::write(tree.root.join("README.md"), "hello\nworld\n").expect("readme");
        tree
    }

    fn gitignored_tree() -> TempTree {
        let tree = TempTree::new();
        fs::create_dir_all(tree.root.join("src/generated")).expect("dirs created");
        fs::write(
            tree.root.join(".gitignore"),
            "*.log\nignored/\nsrc/generated/\n",
        )
        .expect("gitignore");
        fs::write(tree.root.join("visible.txt"), "needle visible\n").expect("visible");
        fs::write(tree.root.join("hidden.log"), "needle hidden\n").expect("hidden");
        fs::create_dir_all(tree.root.join("ignored")).expect("ignored dir");
        fs::write(tree.root.join("ignored/secret.txt"), "needle secret\n").expect("secret");
        fs::write(tree.root.join("src/lib.rs"), "pub fn keep() {}\n").expect("lib");
        fs::write(
            tree.root.join("src/generated/skip.rs"),
            "pub fn ignored() {}\n",
        )
        .expect("generated");
        tree
    }

    fn large_tree(file_count: usize) -> TempTree {
        let tree = TempTree::new();
        fs::create_dir_all(tree.root.join("files")).expect("dirs created");

        for index in 0..file_count {
            fs::write(
                tree.root.join("files").join(format!("file-{index:03}.txt")),
                format!("line {index}\n"),
            )
            .expect("file created");
        }

        tree
    }

    #[test]
    fn list_directory_supports_recursive_tree() {
        let tree = sample_tree();
        let output = list_directory(&tree.root, ".", true).expect("list succeeds");

        assert!(output.contains("src/"));
        assert!(output.contains("src/main.rs"));
        assert!(output.contains("src/nested/lib.rs"));
    }

    #[test]
    fn list_directory_respects_gitignore_patterns() {
        let tree = gitignored_tree();
        let output = list_directory(&tree.root, ".", true).expect("list succeeds");

        assert!(output.contains("visible.txt"));
        assert!(output.contains("src/lib.rs"));
        assert!(!output.contains("hidden.log"));
        assert!(!output.contains("ignored/secret.txt"));
        assert!(!output.contains("src/generated/skip.rs"));
    }

    #[test]
    fn list_directory_rejects_explicit_ignored_directory() {
        let tree = gitignored_tree();
        let error = list_directory(&tree.root, "ignored", true).expect_err("ignored dir must fail");

        assert!(error.to_string().contains("ignored by .gitignore"));
    }

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
    fn read_files_reads_multiple_files_with_independent_ranges() {
        let tree = sample_tree();
        let output = read_files_lines(
            &tree.root,
            &[
                ReadFileArgs {
                    filename: "README.md".into(),
                    offset: 1,
                    limit: 1,
                },
                ReadFileArgs {
                    filename: "src/main.rs".into(),
                    offset: 0,
                    limit: 1,
                },
            ],
        )
        .expect("read succeeds");

        assert!(output.contains("==> README.md <=="));
        assert!(output.contains("2 | world"));
        assert!(output.contains("==> src/main.rs <=="));
        assert!(output.contains("1 | fn main() {}"));
        assert!(!output.contains("1 | hello"));
        assert!(!output.contains("2 | println!(\"hi\");"));
    }

    #[test]
    fn read_files_rejects_more_than_five_entries() {
        let tree = sample_tree();
        let files = (0..6)
            .map(|_| ReadFileArgs {
                filename: "README.md".into(),
                offset: 0,
                limit: 1,
            })
            .collect::<Vec<_>>();

        let error = read_files_lines(&tree.root, &files).expect_err("must fail");

        assert!(
            error
                .to_string()
                .contains("files must contain between 1 and 5 entries")
        );
    }

    #[test]
    fn read_files_surfaces_per_file_no_lines_messages() {
        let tree = sample_tree();
        let output = read_files_lines(
            &tree.root,
            &[ReadFileArgs {
                filename: "README.md".into(),
                offset: 10,
                limit: 2,
            }],
        )
        .expect("read succeeds");

        assert!(output.contains("==> README.md <=="));
        assert!(output.contains("No lines returned from README.md at offset 10 with limit 2."));
    }

    #[test]
    fn grep_returns_matching_lines() {
        let tree = sample_tree();
        let output = grep_workspace(&tree.root, "TODO", ".", true).expect("grep succeeds");

        assert!(output.contains("src/nested/lib.rs:2:// TODO: grep target"));
    }

    #[test]
    fn grep_respects_gitignore_patterns() {
        let tree = gitignored_tree();
        let output = grep_workspace(&tree.root, "needle", ".", true).expect("grep succeeds");

        assert!(output.contains("visible.txt:1:needle visible"));
        assert!(!output.contains("hidden.log"));
        assert!(!output.contains("ignored/secret.txt"));
    }

    #[test]
    fn grep_rejects_explicit_ignored_file() {
        let tree = gitignored_tree();
        let error = grep_workspace(&tree.root, "needle", "hidden.log", true)
            .expect_err("ignored file must fail");

        assert!(error.to_string().contains("ignored by .gitignore"));
    }

    #[test]
    fn list_directory_truncates_large_trees() {
        let tree = large_tree(MAX_LIST_ENTRIES + 50);
        let output = list_directory(&tree.root, "files", true).expect("list succeeds");

        assert!(output.contains(&format!("... truncated after {MAX_LIST_ENTRIES} entries")));
    }

    #[test]
    fn grep_truncates_large_match_sets() {
        let tree = TempTree::new();
        let content = (0..(MAX_GREP_MATCHES + 25))
            .map(|index| format!("match line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(tree.root.join("many.txt"), content).expect("matches file");

        let output =
            grep_workspace(&tree.root, "match line", "many.txt", true).expect("grep succeeds");

        assert!(output.contains(&format!("... truncated after {MAX_GREP_MATCHES} matches")));
    }

    #[test]
    fn resolve_path_rejects_workspace_escape() {
        let tree = sample_tree();
        let error = resolve_path(&tree.root, "..").expect_err("escape must fail");
        assert!(
            error
                .to_string()
                .contains("escapes the current workspace root")
        );
    }
}
