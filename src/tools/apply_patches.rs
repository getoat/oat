use std::path::{Path, PathBuf};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::{ToolExecError, display_path, resolve_workspace_path_with_access};

pub const APPLY_PATCH_TOOL_NAME: &str = "ApplyPatches";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApplyPatchesTool {
    root: PathBuf,
    allow_full_system_access: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ApplyPatchesArgs {
    pub filename: String,
    pub patches: Vec<TextPatch>,
    #[serde(default)]
    pub intent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TextPatch {
    pub old_text: String,
    pub new_text: String,
}

impl ApplyPatchesTool {
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

impl Tool for ApplyPatchesTool {
    const NAME: &'static str = APPLY_PATCH_TOOL_NAME;
    type Error = ToolExecError;
    type Args = ApplyPatchesArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Apply between 1 and 5 exact text replacements to a single existing workspace file. Each old_text must match exactly once at the moment that patch is applied. The full call fails if any patch is invalid. Always include intent as a short plain-language reason that explains why the change is needed, not just what text is changing.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "filename": {
                        "type": "string",
                        "description": "File path relative to the current workspace root."
                    },
                    "patches": {
                        "type": "array",
                        "description": "Between 1 and 5 exact replacements to apply in order to the same file.",
                        "minItems": 1,
                        "maxItems": 5,
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_text": {
                                    "type": "string",
                                    "description": "Exact existing text to replace. Must appear exactly once when this patch is applied."
                                },
                                "new_text": {
                                    "type": "string",
                                    "description": "Replacement text."
                                }
                            },
                            "required": ["old_text", "new_text"]
                        }
                    },
                    "intent": {
                        "type": "string",
                        "description": "Short sentence explaining why this change is needed for the user. Focus on purpose or outcome, not the mechanical text replacement."
                    }
                },
                "required": ["filename", "patches", "intent"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        apply_patches_with_access(
            &self.root,
            &args.filename,
            &args.patches,
            self.allow_full_system_access,
        )
    }
}

pub(crate) fn apply_patches(
    root: &Path,
    filename: &str,
    patches: &[TextPatch],
) -> Result<String, ToolExecError> {
    apply_patches_with_access(root, filename, patches, false)
}

pub(crate) fn apply_patches_with_access(
    root: &Path,
    filename: &str,
    patches: &[TextPatch],
    allow_full_system_access: bool,
) -> Result<String, ToolExecError> {
    if patches.is_empty() || patches.len() > 5 {
        return Err(ToolExecError::new(
            "patches must contain between 1 and 5 entries",
        ));
    }

    let path = resolve_workspace_path_with_access(root, filename, allow_full_system_access)?;
    let metadata = std::fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(ToolExecError::new(format!("{filename} is not a file")));
    }

    let mut updated = std::fs::read_to_string(&path)?;
    for (index, patch) in patches.iter().enumerate() {
        if patch.old_text.is_empty() {
            return Err(ToolExecError::new(format!(
                "patch {} old_text must not be empty",
                index + 1
            )));
        }

        let match_count = updated.matches(&patch.old_text).count();
        if match_count == 0 {
            return Err(ToolExecError::new(format!(
                "patch {} old_text was not found in {}",
                index + 1,
                display_path(root, &path)
            )));
        }
        if match_count > 1 {
            return Err(ToolExecError::new(format!(
                "patch {} old_text matched {match_count} times in {}; it must match exactly once",
                index + 1,
                display_path(root, &path)
            )));
        }

        updated = updated.replacen(&patch.old_text, &patch.new_text, 1);
    }
    std::fs::write(&path, updated)?;

    Ok(format!("Updated {}.", display_path(root, &path)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::{TempTree, tree_with_mutation_targets};

    #[test]
    fn apply_patches_replaces_exactly_one_match() {
        let tree = tree_with_mutation_targets();

        let output = apply_patches(
            &tree.root,
            "src/lib.rs",
            &[TextPatch {
                old_text: "fn beta() {}".into(),
                new_text: "fn gamma() {}".into(),
            }],
        )
        .expect("edit succeeds");

        assert_eq!(output, "Updated src/lib.rs.");
        let updated = std::fs::read_to_string(tree.root.join("src/lib.rs")).expect("file exists");
        assert_eq!(updated, "fn alpha() {}\nfn gamma() {}\n");
    }

    #[test]
    fn apply_patches_rejects_missing_match() {
        let tree = tree_with_mutation_targets();

        let error = apply_patches(
            &tree.root,
            "src/lib.rs",
            &[TextPatch {
                old_text: "fn missing() {}".into(),
                new_text: "fn gamma() {}".into(),
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains("patch 1 old_text was not found"));
    }

    #[test]
    fn apply_patches_rejects_multiple_matches() {
        let tree = TempTree::new();
        std::fs::write(tree.root.join("repeat.txt"), "same\nsame\n").expect("repeat file");

        let error = apply_patches(
            &tree.root,
            "repeat.txt",
            &[TextPatch {
                old_text: "same".into(),
                new_text: "new".into(),
            }],
        )
        .unwrap_err();

        assert!(error.to_string().contains("must match exactly once"));
    }

    #[test]
    fn apply_patches_applies_multiple_replacements_to_one_file() {
        let tree = tree_with_mutation_targets();

        let output = apply_patches(
            &tree.root,
            "src/lib.rs",
            &[
                TextPatch {
                    old_text: "fn alpha() {}".into(),
                    new_text: "fn alpha_renamed() {}".into(),
                },
                TextPatch {
                    old_text: "fn beta() {}".into(),
                    new_text: "fn beta_renamed() {}".into(),
                },
            ],
        )
        .expect("patches succeed");

        assert_eq!(output, "Updated src/lib.rs.");
        let updated = std::fs::read_to_string(tree.root.join("src/lib.rs")).expect("file exists");
        assert_eq!(updated, "fn alpha_renamed() {}\nfn beta_renamed() {}\n");
    }

    #[test]
    fn apply_patches_rejects_more_than_five_entries() {
        let tree = tree_with_mutation_targets();
        let patches = (0..6)
            .map(|index| TextPatch {
                old_text: format!("missing-{index}"),
                new_text: format!("new-{index}"),
            })
            .collect::<Vec<_>>();

        let error = apply_patches(&tree.root, "src/lib.rs", &patches).expect_err("must fail");

        assert!(
            error
                .to_string()
                .contains("patches must contain between 1 and 5 entries")
        );
    }

    #[tokio::test]
    async fn mutation_tool_definition_requires_intent() {
        let definition = ApplyPatchesTool::new(PathBuf::from("."))
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
