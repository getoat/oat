use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    ApplyPatchesArgs, DeletePathArgs, TextPatch, WriteFileArgs,
    apply_patches::APPLY_PATCH_TOOL_NAME, delete_path::DELETE_PATH_TOOL_NAME,
    write_file::WRITE_FILE_TOOL_NAME,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffPreviewLine {
    pub old_line_number: Option<usize>,
    pub new_line_number: Option<usize>,
    pub prefix: char,
    pub text: String,
    pub kind: DiffKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationPreview {
    pub target: String,
    pub summary: Option<String>,
    pub lines: Vec<DiffPreviewLine>,
}

pub fn mutation_preview(
    tool_name: &str,
    raw_args: &str,
    workspace_root: &Path,
) -> Option<MutationPreview> {
    match tool_name {
        APPLY_PATCH_TOOL_NAME => {
            let args: ApplyPatchesArgs = serde_json::from_str(raw_args).ok()?;
            Some(apply_patches_preview(args, workspace_root))
        }
        WRITE_FILE_TOOL_NAME => {
            let args: WriteFileArgs = serde_json::from_str(raw_args).ok()?;
            Some(MutationPreview {
                target: args.filename,
                summary: normalize_intent(args.intent.as_deref()),
                lines: numbered_diff_lines('+', &args.content, DiffKind::Added, None, Some(1)),
            })
        }
        DELETE_PATH_TOOL_NAME => {
            let args: DeletePathArgs = serde_json::from_str(raw_args).ok()?;
            Some(MutationPreview {
                target: args.path.clone(),
                summary: normalize_intent(args.intent.as_deref()),
                lines: delete_path_preview_lines(workspace_root, &args.path),
            })
        }
        _ => None,
    }
}

pub fn write_approval_summary(tool_name: &str, raw_args: &str, workspace_root: &Path) -> String {
    if let Some(preview) = mutation_preview(tool_name, raw_args, workspace_root) {
        if let Some(summary) = preview.summary {
            return summary;
        }

        return match tool_name {
            APPLY_PATCH_TOOL_NAME => format!("No reason provided for changing {}", preview.target),
            WRITE_FILE_TOOL_NAME => format!("No reason provided for creating {}", preview.target),
            DELETE_PATH_TOOL_NAME => format!("No reason provided for deleting {}", preview.target),
            _ => "No reason provided for this write request".to_string(),
        };
    }

    "No reason provided for this write request".to_string()
}

fn normalize_intent(intent: Option<&str>) -> Option<String> {
    let intent = intent?;
    let normalized = intent.split_whitespace().collect::<Vec<_>>().join(" ");
    (!normalized.is_empty()).then_some(normalized)
}

fn apply_patches_preview(args: ApplyPatchesArgs, workspace_root: &Path) -> MutationPreview {
    let mut lines = Vec::new();
    let mut content = read_preview_file(workspace_root, &args.filename);

    for patch in args.patches {
        let preview_lines = content
            .as_mut()
            .and_then(|updated| apply_numbered_patch_preview(updated, &patch))
            .unwrap_or_else(|| {
                let mut fallback =
                    numbered_diff_lines('-', &patch.old_text, DiffKind::Removed, None, None);
                fallback.extend(numbered_diff_lines(
                    '+',
                    &patch.new_text,
                    DiffKind::Added,
                    None,
                    None,
                ));
                fallback
            });

        if preview_lines
            .iter()
            .all(|line| line.old_line_number.is_none() && line.new_line_number.is_none())
        {
            content = None;
        }

        lines.extend(preview_lines);
    }

    MutationPreview {
        target: args.filename,
        summary: normalize_intent(args.intent.as_deref()),
        lines,
    }
}

fn apply_numbered_patch_preview(
    updated: &mut String,
    patch: &TextPatch,
) -> Option<Vec<DiffPreviewLine>> {
    if patch.old_text.is_empty() {
        return None;
    }

    let start = unique_match_index(updated, &patch.old_text)?;
    let start_line = line_number_for_offset(updated, start);
    let end = start + patch.old_text.len();
    updated.replace_range(start..end, &patch.new_text);

    let mut lines = numbered_diff_lines(
        '-',
        &patch.old_text,
        DiffKind::Removed,
        Some(start_line),
        None,
    );
    lines.extend(numbered_diff_lines(
        '+',
        &patch.new_text,
        DiffKind::Added,
        None,
        Some(start_line),
    ));
    Some(lines)
}

fn delete_path_preview_lines(workspace_root: &Path, raw_path: &str) -> Vec<DiffPreviewLine> {
    if let Some(content) = read_preview_file(workspace_root, raw_path) {
        return numbered_diff_lines('-', &content, DiffKind::Removed, Some(1), None);
    }

    vec![DiffPreviewLine {
        old_line_number: Some(1),
        new_line_number: None,
        prefix: '-',
        text: raw_path.to_string(),
        kind: DiffKind::Removed,
    }]
}

fn read_preview_file(workspace_root: &Path, raw_path: &str) -> Option<String> {
    let path = preview_path(workspace_root, raw_path)?;
    let metadata = fs::metadata(&path).ok()?;
    metadata
        .is_file()
        .then(|| fs::read_to_string(path).ok())
        .flatten()
}

fn preview_path(workspace_root: &Path, raw_path: &str) -> Option<PathBuf> {
    let canonical_root = workspace_root.canonicalize().ok()?;
    let candidate = canonical_root.join(raw_path);
    let canonical_path = candidate.canonicalize().ok()?;
    (canonical_path != canonical_root && canonical_path.starts_with(&canonical_root))
        .then_some(canonical_path)
}

fn unique_match_index(text: &str, pattern: &str) -> Option<usize> {
    let mut matches = text.match_indices(pattern);
    let (index, _) = matches.next()?;
    matches.next().is_none().then_some(index)
}

fn line_number_for_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn numbered_diff_lines(
    prefix: char,
    text: &str,
    kind: DiffKind,
    old_start: Option<usize>,
    new_start: Option<usize>,
) -> Vec<DiffPreviewLine> {
    let has_numbered_content = !text.is_empty();
    preview_text_lines(text)
        .into_iter()
        .enumerate()
        .map(|(index, text)| DiffPreviewLine {
            old_line_number: has_numbered_content
                .then(|| old_start.map(|start| start + index))
                .flatten(),
            new_line_number: has_numbered_content
                .then(|| new_start.map(|start| start + index))
                .flatten(),
            prefix,
            text,
            kind,
        })
        .collect()
}

fn preview_text_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        vec!["(empty)".to_string()]
    } else {
        text.lines().map(ToOwned::to_owned).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::{TempTree, tree_with_mutation_targets};

    #[test]
    fn apply_patches_preview_numbers_multiline_changes() {
        let tree = tree_with_mutation_targets();
        let preview = mutation_preview(
            APPLY_PATCH_TOOL_NAME,
            r#"{"filename":"src/lib.rs","patches":[{"old_text":"fn alpha() {}\nfn beta() {}","new_text":"fn gamma() {}\nfn delta() {}"}],"intent":"replace both functions"}"#,
            &tree.root,
        )
        .expect("preview");

        assert_eq!(preview.target, "src/lib.rs");
        assert_eq!(preview.summary.as_deref(), Some("replace both functions"));
        assert_eq!(preview.lines[0].old_line_number, Some(1));
        assert_eq!(preview.lines[1].old_line_number, Some(2));
        assert_eq!(preview.lines[2].new_line_number, Some(1));
        assert_eq!(preview.lines[3].new_line_number, Some(2));
    }

    #[test]
    fn apply_patches_preview_falls_back_when_match_is_ambiguous() {
        let tree = TempTree::new();
        fs::write(tree.root.join("repeat.txt"), "same\nsame\n").expect("repeat file");

        let preview = mutation_preview(
            APPLY_PATCH_TOOL_NAME,
            r#"{"filename":"repeat.txt","patches":[{"old_text":"same","new_text":"new"}],"intent":"dedupe"}"#,
            &tree.root,
        )
        .expect("preview");

        assert_eq!(preview.lines[0].old_line_number, None);
        assert_eq!(preview.lines[1].new_line_number, None);
    }

    #[test]
    fn delete_path_preview_uses_file_contents_when_available() {
        let tree = TempTree::new();
        fs::write(tree.root.join("notes.txt"), "alpha\nbeta\n").expect("notes");

        let preview = mutation_preview(
            DELETE_PATH_TOOL_NAME,
            r#"{"path":"notes.txt","intent":"remove notes"}"#,
            &tree.root,
        )
        .expect("preview");

        assert_eq!(preview.lines.len(), 2);
        assert_eq!(preview.lines[0].old_line_number, Some(1));
        assert_eq!(preview.lines[1].old_line_number, Some(2));
    }

    #[test]
    fn write_approval_summary_falls_back_to_missing_reason_message() {
        let tree = TempTree::new();

        let summary = write_approval_summary(
            WRITE_FILE_TOOL_NAME,
            r#"{"filename":"notes.txt","content":"hello"}"#,
            &tree.root,
        );

        assert_eq!(summary, "No reason provided for creating notes.txt");
    }
}
