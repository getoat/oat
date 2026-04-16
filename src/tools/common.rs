use std::{
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use ignore::WalkBuilder;

use crate::tool_policy::SearchPathPolicy;

#[derive(Debug)]
pub(crate) struct VisibleEntry {
    pub(crate) path: PathBuf,
    pub(crate) depth: usize,
    pub(crate) is_dir: bool,
}

#[derive(Debug)]
pub(crate) struct CollectedVisibleEntries {
    pub(crate) entries: Vec<VisibleEntry>,
    pub(crate) truncated: bool,
}

#[derive(Debug)]
pub struct ToolExecError(String);

impl ToolExecError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
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

pub(crate) fn collect_visible_entries(
    target: &Path,
    recursive: bool,
    policy: &SearchPathPolicy,
) -> Result<Vec<VisibleEntry>, ToolExecError> {
    Ok(collect_visible_entries_limited(target, recursive, policy, None)?.entries)
}

pub(crate) fn collect_visible_entries_limited(
    target: &Path,
    recursive: bool,
    policy: &SearchPathPolicy,
    limit: Option<usize>,
) -> Result<CollectedVisibleEntries, ToolExecError> {
    let mut builder = walk_builder(target, policy);
    if !recursive {
        builder.max_depth(Some(1));
    }

    let mut entries = Vec::new();
    let mut truncated = false;
    let limit_with_probe = limit.and_then(|value| value.checked_add(1));
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
        let relative_path = path
            .strip_prefix(target)
            .map_err(|error| ToolExecError::new(error.to_string()))?;
        if !policy.should_include(relative_path, is_dir) {
            continue;
        }

        entries.push(VisibleEntry {
            path: path.to_path_buf(),
            depth,
            is_dir,
        });

        if limit_with_probe.is_some_and(|value| entries.len() >= value) {
            truncated = true;
            break;
        }
    }

    entries.sort_by(|left, right| {
        left.path
            .parent()
            .cmp(&right.path.parent())
            .then_with(|| right.is_dir.cmp(&left.is_dir))
            .then_with(|| left.path.file_name().cmp(&right.path.file_name()))
    });

    if let Some(limit) = limit {
        entries.truncate(limit);
    }

    Ok(CollectedVisibleEntries { entries, truncated })
}

pub(crate) fn is_path_visible(
    root: &Path,
    target: &Path,
    policy: &SearchPathPolicy,
) -> Result<bool, ToolExecError> {
    let max_depth = target
        .strip_prefix(root)
        .map_err(|error| ToolExecError::new(error.to_string()))?
        .components()
        .count();
    let mut builder = walk_builder(root, policy);
    builder.max_depth(Some(max_depth));

    for result in builder.build() {
        let entry = result.map_err(|error| ToolExecError::new(error.to_string()))?;
        if entry.path() == target {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(crate) fn resolve_path(root: &Path, raw_path: &str) -> Result<PathBuf, ToolExecError> {
    resolve_path_with_access(root, raw_path, false)
}

pub(crate) fn resolve_path_with_access(
    root: &Path,
    raw_path: &str,
    allow_full_system_access: bool,
) -> Result<PathBuf, ToolExecError> {
    let canonical_root = root.canonicalize()?;
    let joined = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        canonical_root.join(raw_path)
    };
    let canonical_path = joined.canonicalize()?;
    if !allow_full_system_access && !canonical_path.starts_with(&canonical_root) {
        return Err(ToolExecError::new(format!(
            "path {raw_path} escapes the current workspace root"
        )));
    }

    Ok(canonical_path)
}

pub(crate) fn resolve_workspace_path(
    root: &Path,
    raw_path: &str,
) -> Result<PathBuf, ToolExecError> {
    resolve_workspace_path_with_access(root, raw_path, false)
}

pub(crate) fn resolve_workspace_path_with_access(
    root: &Path,
    raw_path: &str,
    allow_full_system_access: bool,
) -> Result<PathBuf, ToolExecError> {
    let canonical_root = root.canonicalize()?;
    let joined = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        canonical_root.join(raw_path)
    };

    let mut existing_ancestor = joined.clone();
    while !existing_ancestor.exists() {
        if !existing_ancestor.pop() {
            return Err(ToolExecError::new(format!(
                "path {raw_path} escapes the current workspace root"
            )));
        }
    }

    let canonical_ancestor = existing_ancestor.canonicalize()?;
    if !allow_full_system_access && !canonical_ancestor.starts_with(&canonical_root) {
        return Err(ToolExecError::new(format!(
            "path {raw_path} escapes the current workspace root"
        )));
    }

    if joined == existing_ancestor {
        return Ok(canonical_ancestor);
    }

    let suffix = joined
        .strip_prefix(&existing_ancestor)
        .map_err(|error| ToolExecError::new(error.to_string()))?;
    Ok(canonical_ancestor.join(suffix))
}

pub(crate) fn path_is_within_root(root: &Path, path: &Path) -> bool {
    let Ok(canonical_root) = root.canonicalize() else {
        return false;
    };
    let Ok(canonical_path) = path.canonicalize() else {
        return false;
    };
    canonical_path.starts_with(canonical_root)
}

pub(crate) fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(crate) fn is_unsafe_system_path(path: &Path) -> bool {
    path.starts_with("/proc") || path.starts_with("/sys") || path.starts_with("/dev")
}

fn walk_builder(target: &Path, policy: &SearchPathPolicy) -> WalkBuilder {
    let mut builder = WalkBuilder::new(target);
    builder.hidden(false);
    builder.ignore(false);
    builder.git_ignore(true);
    builder.git_global(false);
    builder.git_exclude(false);
    builder.parents(true);
    builder.require_git(false);
    let root = target.to_path_buf();
    let policy = policy.clone();
    builder.filter_entry(move |entry| {
        if entry.path() != root && is_unsafe_system_path(entry.path()) {
            return false;
        }

        entry.path() == root
            || entry
                .path()
                .strip_prefix(&root)
                .ok()
                .is_some_and(|relative_path| {
                    let is_dir = entry
                        .file_type()
                        .is_some_and(|file_type| file_type.is_dir());
                    policy.should_include(relative_path, is_dir)
                })
    });
    builder
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    pub(crate) struct TempTree {
        pub(crate) root: PathBuf,
    }

    impl TempTree {
        pub(crate) fn new() -> Self {
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

    pub(crate) fn sample_tree() -> TempTree {
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

    pub(crate) fn tree_with_mutation_targets() -> TempTree {
        let tree = TempTree::new();
        fs::create_dir_all(tree.root.join("src")).expect("dirs created");
        fs::write(
            tree.root.join("src/lib.rs"),
            "fn alpha() {}\nfn beta() {}\n",
        )
        .expect("lib file");
        tree
    }

    pub(crate) fn gitignored_tree() -> TempTree {
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

    pub(crate) fn large_tree(file_count: usize) -> TempTree {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::large_tree;

    #[test]
    fn limited_visible_entry_collection_reports_truncation() {
        let tree = large_tree(8);
        let policy = SearchPathPolicy::new(&[]).expect("policy builds");

        let collected =
            collect_visible_entries_limited(&tree.root.join("files"), true, &policy, Some(3))
                .expect("collection succeeds");

        assert_eq!(collected.entries.len(), 3);
        assert!(collected.truncated);
    }

    #[test]
    fn unsafe_system_path_detection_matches_proc_like_roots() {
        assert!(is_unsafe_system_path(Path::new("/proc")));
        assert!(is_unsafe_system_path(Path::new("/sys/kernel")));
        assert!(is_unsafe_system_path(Path::new("/dev/null")));
        assert!(!is_unsafe_system_path(Path::new("/tmp/work")));
    }
}
