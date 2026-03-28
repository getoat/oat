use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use crate::todo::{TodoSnapshot, TodoTask, validate_and_normalize_tasks};

use super::common::ToolExecError;

#[derive(Clone, Default)]
pub struct TodoTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TodoOperation {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Deserialize)]
pub struct TodoArgs {
    operation: TodoOperation,
    #[serde(default)]
    tasks: Vec<TodoTask>,
}

impl Tool for TodoTool {
    const NAME: &'static str = "Todo";
    type Error = ToolExecError;
    type Args = TodoArgs;
    type Output = TodoSnapshot;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Create, replace, or delete a short todo list for your next few tasks. Keep task descriptions concise and single-line. Use `todo`, `in progress`, or `done` for statuses.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "delete"],
                        "description": "Whether to create a new todo list, replace the current list, or delete it."
                    },
                    "tasks": {
                        "type": "array",
                        "maxItems": 10,
                        "description": "Ordered todo items for create or update. Omit this field for delete.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "description": {
                                    "type": "string",
                                    "description": "A single concise sentence describing the task."
                                },
                                "status": {
                                    "type": "string",
                                    "enum": ["todo", "in progress", "done"],
                                    "description": "Current task status."
                                }
                            },
                            "required": ["description", "status"]
                        }
                    }
                },
                "required": ["operation"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let snapshot = match args.operation {
            TodoOperation::Create | TodoOperation::Update => TodoSnapshot::new(
                validate_and_normalize_tasks(args.tasks).map_err(ToolExecError::new)?,
            ),
            TodoOperation::Delete => {
                if !args.tasks.is_empty() {
                    return Err(ToolExecError::new("Todo delete does not accept any tasks."));
                }
                TodoSnapshot::cleared()
            }
        };

        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo::{TodoSnapshot, TodoStatus};

    #[tokio::test]
    async fn create_normalizes_tasks() {
        let tool = TodoTool;
        let output = tool
            .call(TodoArgs {
                operation: TodoOperation::Create,
                tasks: vec![TodoTask {
                    description: "  Inspect render path. ".into(),
                    status: TodoStatus::InProgress,
                }],
            })
            .await
            .expect("todo create succeeds");

        assert_eq!(
            output,
            TodoSnapshot::new(vec![TodoTask {
                description: "Inspect render path.".into(),
                status: TodoStatus::InProgress,
            }])
        );
    }

    #[tokio::test]
    async fn delete_requires_no_tasks() {
        let tool = TodoTool;
        let error = tool
            .call(TodoArgs {
                operation: TodoOperation::Delete,
                tasks: vec![TodoTask {
                    description: "Should fail.".into(),
                    status: TodoStatus::Todo,
                }],
            })
            .await
            .expect_err("delete with tasks must fail");

        assert!(error.to_string().contains("does not accept any tasks"));
    }

    #[tokio::test]
    async fn update_rejects_empty_tasks() {
        let tool = TodoTool;
        let error = tool
            .call(TodoArgs {
                operation: TodoOperation::Update,
                tasks: Vec::new(),
            })
            .await
            .expect_err("empty update must fail");

        assert!(error.to_string().contains("at least one task"));
    }
}
