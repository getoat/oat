mod apply_patches;
mod common;
mod delete_path;
mod grep;
mod list;
mod preview;
mod read_file;
mod read_files;
mod write_file;

use std::path::{Path, PathBuf};

use rig::tool::{Tool, ToolDyn};

use crate::app::AccessMode;

pub use apply_patches::{ApplyPatchesArgs, ApplyPatchesTool, TextPatch};
pub use delete_path::{DeletePathArgs, DeletePathTool};
pub use grep::{GrepArgs, GrepTool};
pub use list::{ListArgs, ListTool};
pub use preview::{
    DiffKind, DiffPreviewLine, MutationPreview, mutation_preview, write_approval_summary,
};
pub use read_file::{ReadFileArgs, ReadFileTool};
pub use read_files::{ReadFilesArgs, ReadFilesTool};
pub use write_file::{WriteFileArgs, WriteFileTool};

#[derive(Clone, Copy)]
struct ToolDescriptor {
    name: &'static str,
    access_mode: ToolAccess,
    constructor: fn(PathBuf) -> Box<dyn ToolDyn>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ToolAccess {
    ReadOnly,
    Mutation,
}

const TOOL_DESCRIPTORS: [ToolDescriptor; 7] = [
    ToolDescriptor::read_only(ListTool::NAME, |root| Box::new(ListTool::new(root))),
    ToolDescriptor::read_only(ReadFileTool::NAME, |root| Box::new(ReadFileTool::new(root))),
    ToolDescriptor::read_only(ReadFilesTool::NAME, |root| {
        Box::new(ReadFilesTool::new(root))
    }),
    ToolDescriptor::read_only(GrepTool::NAME, |root| Box::new(GrepTool::new(root))),
    ToolDescriptor::mutation(ApplyPatchesTool::NAME, |root| {
        Box::new(ApplyPatchesTool::new(root))
    }),
    ToolDescriptor::mutation(WriteFileTool::NAME, |root| {
        Box::new(WriteFileTool::new(root))
    }),
    ToolDescriptor::mutation(DeletePathTool::NAME, |root| {
        Box::new(DeletePathTool::new(root))
    }),
];

impl ToolDescriptor {
    const fn read_only(name: &'static str, constructor: fn(PathBuf) -> Box<dyn ToolDyn>) -> Self {
        Self {
            name,
            access_mode: ToolAccess::ReadOnly,
            constructor,
        }
    }

    const fn mutation(name: &'static str, constructor: fn(PathBuf) -> Box<dyn ToolDyn>) -> Self {
        Self {
            name,
            access_mode: ToolAccess::Mutation,
            constructor,
        }
    }

    fn is_enabled(self, access_mode: AccessMode) -> bool {
        self.access_mode == ToolAccess::ReadOnly || access_mode == AccessMode::ReadWrite
    }
}

pub fn tool_names_for_mode(access_mode: AccessMode) -> Vec<String> {
    TOOL_DESCRIPTORS
        .into_iter()
        .filter(|tool| tool.is_enabled(access_mode))
        .map(|tool| tool.name.to_string())
        .collect()
}

pub fn tools_for_mode(root: &Path, access_mode: AccessMode) -> Vec<Box<dyn ToolDyn>> {
    TOOL_DESCRIPTORS
        .into_iter()
        .filter(|tool| tool.is_enabled(access_mode))
        .map(|tool| (tool.constructor)(root.to_path_buf()))
        .collect()
}

pub fn is_mutation_tool(tool_name: &str) -> bool {
    TOOL_DESCRIPTORS.iter().any(|tool| {
        tool.access_mode == ToolAccess::Mutation && tool.name.eq_ignore_ascii_case(tool_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_mode_exposes_only_read_tools() {
        let tool_names = tool_names_for_mode(AccessMode::ReadOnly);

        assert_eq!(tool_names, vec!["List", "ReadFile", "ReadFiles", "Grep"]);
    }

    #[test]
    fn read_write_mode_exposes_all_tools() {
        let tool_names = tool_names_for_mode(AccessMode::ReadWrite);

        assert!(tool_names.contains(&"ApplyPatches".to_string()));
        assert!(tool_names.contains(&"WriteFile".to_string()));
        assert!(tool_names.contains(&"DeletePath".to_string()));
    }

    #[test]
    fn mutation_classification_matches_write_tools() {
        for tool_name in tool_names_for_mode(AccessMode::ReadOnly) {
            assert!(
                !is_mutation_tool(&tool_name),
                "{tool_name} should be read-only"
            );
        }

        for tool_name in ["ApplyPatches", "WriteFile", "DeletePath"] {
            assert!(is_mutation_tool(tool_name), "{tool_name} should be mutable");
        }
    }
}
