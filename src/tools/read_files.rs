use std::path::PathBuf;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    common::ToolExecError,
    read_file::{MAX_READFILE_LIMIT, ReadFileArgs},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadFilesTool {
    root: PathBuf,
    allow_full_system_access: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReadFilesArgs {
    pub files: Vec<ReadFileArgs>,
}

impl ReadFilesTool {
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
        read_files_lines_with_access(&self.root, &args.files, self.allow_full_system_access)
    }
}

pub(crate) fn read_files_lines(
    root: &std::path::Path,
    files: &[ReadFileArgs],
) -> Result<String, ToolExecError> {
    read_files_lines_with_access(root, files, false)
}

pub(crate) fn read_files_lines_with_access(
    root: &std::path::Path,
    files: &[ReadFileArgs],
    allow_full_system_access: bool,
) -> Result<String, ToolExecError> {
    if files.is_empty() || files.len() > 5 {
        return Err(ToolExecError::new(
            "files must contain between 1 and 5 entries",
        ));
    }

    let mut sections = Vec::with_capacity(files.len());
    for file in files {
        let content = super::read_file::read_file_lines_with_access(
            root,
            &file.filename,
            file.offset,
            file.limit,
            allow_full_system_access,
        )?;
        sections.push(format!("==> {} <==\n{content}", file.filename));
    }

    Ok(sections.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::common::test_support::sample_tree;

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
}
