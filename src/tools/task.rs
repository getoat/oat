//! Tools the agent uses to register an active task plus acceptance criteria.
//!
//! The tools are thin wrappers that return a [`TaskUpdate`] payload. The
//! streaming layer (see `src/llm/streaming.rs`) parses that payload from the
//! tool result JSON and emits a `TaskUpdate` stream event, which the reducer
//! uses to mutate `SessionState::current_task`.

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::task::{AcceptanceCriterion, CriterionId, MAX_CRITERIA_PER_TASK};

use super::common::ToolExecError;

pub fn parse_task_update(output: &str) -> Result<TaskUpdate, serde_json::Error> {
    serde_json::from_str(output).or_else(|_| {
        let nested = serde_json::from_str::<String>(output)?;
        serde_json::from_str(&nested)
    })
}

pub fn is_task_tool_name(name: &str) -> bool {
    matches!(
        name,
        "SetCurrentTask"
            | "ClearCurrentTask"
            | "AddCriterion"
            | "UpdateCriterion"
            | "RemoveCriterion"
    )
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum TaskUpdate {
    Set {
        description: String,
        #[serde(default)]
        criteria: Vec<CriterionDraft>,
    },
    Clear,
    AddCriterion {
        text: String,
        verification_hint: String,
    },
    UpdateCriterion {
        id: CriterionId,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        verification_hint: Option<String>,
    },
    RemoveCriterion {
        id: CriterionId,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CriterionDraft {
    pub text: String,
    pub verification_hint: String,
}

impl CriterionDraft {
    pub fn into_criterion(self, id: CriterionId) -> AcceptanceCriterion {
        AcceptanceCriterion {
            id,
            text: self.text,
            verification_hint: self.verification_hint,
        }
    }
}

fn validate_text(field: &str, value: &str) -> Result<String, ToolExecError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolExecError::new(format!("{field} must not be empty.")));
    }
    Ok(trimmed.to_string())
}

fn validate_hint(value: &str) -> Result<String, ToolExecError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ToolExecError::new(
            "verification_hint must not be empty. Describe a concrete, observable check (e.g., \"run X and confirm exit code 0\", \"read Y and confirm it names a real dependency\").",
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_criteria(criteria: Vec<CriterionDraft>) -> Result<Vec<CriterionDraft>, ToolExecError> {
    if criteria.len() > MAX_CRITERIA_PER_TASK {
        return Err(ToolExecError::new(format!(
            "A task cannot have more than {MAX_CRITERIA_PER_TASK} criteria."
        )));
    }
    criteria
        .into_iter()
        .map(|draft| {
            Ok(CriterionDraft {
                text: validate_text("criterion text", &draft.text)?,
                verification_hint: validate_hint(&draft.verification_hint)?,
            })
        })
        .collect()
}

#[derive(Clone, Default)]
pub struct SetCurrentTaskTool;

#[derive(Debug, Deserialize)]
pub struct SetCurrentTaskArgs {
    pub description: String,
    #[serde(default)]
    pub criteria: Vec<CriterionDraft>,
}

impl Tool for SetCurrentTaskTool {
    const NAME: &'static str = "SetCurrentTask";
    type Error = ToolExecError;
    type Args = SetCurrentTaskArgs;
    type Output = TaskUpdate;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Register the active task you are working on and, optionally, its acceptance criteria. Replaces any existing task. Use this as soon as the user's goal is clear so the end-of-turn critic can check your work. Each criterion must include a concrete, observable verification hint (e.g., 'read /app/solution.txt and confirm it contains a non-empty password').".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "A short, specific description of the current task."
                    },
                    "criteria": {
                        "type": "array",
                        "maxItems": MAX_CRITERIA_PER_TASK,
                        "description": "Initial acceptance criteria. Can be empty and added later with AddCriterion.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": {
                                    "type": "string",
                                    "description": "What must be true for the task to be considered complete."
                                },
                                "verification_hint": {
                                    "type": "string",
                                    "description": "A concrete, observable check that verifies this criterion (e.g. 'run `pytest tests/foo.py` and confirm exit 0', 'read /app/out.json and confirm it is valid JSON with key X')."
                                }
                            },
                            "required": ["text", "verification_hint"]
                        }
                    }
                },
                "required": ["description"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let description = validate_text("description", &args.description)?;
        let criteria = validate_criteria(args.criteria)?;
        Ok(TaskUpdate::Set {
            description,
            criteria,
        })
    }
}

#[derive(Clone, Default)]
pub struct ClearCurrentTaskTool;

#[derive(Debug, Default, Deserialize)]
pub struct ClearCurrentTaskArgs {}

impl Tool for ClearCurrentTaskTool {
    const NAME: &'static str = "ClearCurrentTask";
    type Error = ToolExecError;
    type Args = ClearCurrentTaskArgs;
    type Output = TaskUpdate;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Clear the active task and its acceptance criteria. Use this when the conversation has moved on and no specific task is currently being worked on."
                .to_string(),
            parameters: json!({"type": "object", "properties": {}}),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(TaskUpdate::Clear)
    }
}

#[derive(Clone, Default)]
pub struct AddCriterionTool;

#[derive(Debug, Deserialize)]
pub struct AddCriterionArgs {
    pub text: String,
    pub verification_hint: String,
}

impl Tool for AddCriterionTool {
    const NAME: &'static str = "AddCriterion";
    type Error = ToolExecError;
    type Args = AddCriterionArgs;
    type Output = TaskUpdate;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Append a new acceptance criterion to the active task. Fails if no task is active. Each criterion needs a concrete, observable verification hint."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "What must be true for the task to be considered complete."
                    },
                    "verification_hint": {
                        "type": "string",
                        "description": "A concrete, observable check that verifies this criterion."
                    }
                },
                "required": ["text", "verification_hint"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(TaskUpdate::AddCriterion {
            text: validate_text("text", &args.text)?,
            verification_hint: validate_hint(&args.verification_hint)?,
        })
    }
}

#[derive(Clone, Default)]
pub struct UpdateCriterionTool;

#[derive(Debug, Deserialize)]
pub struct UpdateCriterionArgs {
    pub id: CriterionId,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub verification_hint: Option<String>,
}

impl Tool for UpdateCriterionTool {
    const NAME: &'static str = "UpdateCriterion";
    type Error = ToolExecError;
    type Args = UpdateCriterionArgs;
    type Output = TaskUpdate;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Rewrite an existing criterion's text or verification hint. Provide only the fields you want to change."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "The id of the criterion to edit."
                    },
                    "text": {"type": "string"},
                    "verification_hint": {"type": "string"}
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.text.is_none() && args.verification_hint.is_none() {
            return Err(ToolExecError::new(
                "UpdateCriterion requires at least one of `text` or `verification_hint`.",
            ));
        }
        let text = args
            .text
            .as_deref()
            .map(|value| validate_text("text", value))
            .transpose()?;
        let verification_hint = args
            .verification_hint
            .as_deref()
            .map(validate_hint)
            .transpose()?;
        Ok(TaskUpdate::UpdateCriterion {
            id: args.id,
            text,
            verification_hint,
        })
    }
}

#[derive(Clone, Default)]
pub struct RemoveCriterionTool;

#[derive(Debug, Deserialize)]
pub struct RemoveCriterionArgs {
    pub id: CriterionId,
}

impl Tool for RemoveCriterionTool {
    const NAME: &'static str = "RemoveCriterion";
    type Error = ToolExecError;
    type Args = RemoveCriterionArgs;
    type Output = TaskUpdate;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Remove an acceptance criterion from the active task.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "The id of the criterion to remove."
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(TaskUpdate::RemoveCriterion { id: args.id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_task_requires_description() {
        let tool = SetCurrentTaskTool;
        let err = tool
            .call(SetCurrentTaskArgs {
                description: "  ".into(),
                criteria: Vec::new(),
            })
            .await
            .expect_err("empty description should fail");
        assert!(err.to_string().contains("description"));
    }

    #[tokio::test]
    async fn criterion_requires_verification_hint() {
        let tool = AddCriterionTool;
        let err = tool
            .call(AddCriterionArgs {
                text: "solution.txt is correct".into(),
                verification_hint: "".into(),
            })
            .await
            .expect_err("empty hint should fail");
        assert!(err.to_string().contains("verification_hint"));
    }

    #[tokio::test]
    async fn update_requires_at_least_one_field() {
        let tool = UpdateCriterionTool;
        let err = tool
            .call(UpdateCriterionArgs {
                id: 3,
                text: None,
                verification_hint: None,
            })
            .await
            .expect_err("empty update should fail");
        assert!(err.to_string().contains("at least one"));
    }

    #[tokio::test]
    async fn set_task_preserves_trimmed_criteria() {
        let tool = SetCurrentTaskTool;
        let update = tool
            .call(SetCurrentTaskArgs {
                description: "  Fix the bug  ".into(),
                criteria: vec![CriterionDraft {
                    text: "  output file exists  ".into(),
                    verification_hint: "  `test -s /app/out.txt` returns 0  ".into(),
                }],
            })
            .await
            .expect("should succeed");
        match update {
            TaskUpdate::Set {
                description,
                criteria,
            } => {
                assert_eq!(description, "Fix the bug");
                assert_eq!(criteria.len(), 1);
                assert_eq!(criteria[0].text, "output file exists");
                assert_eq!(
                    criteria[0].verification_hint,
                    "`test -s /app/out.txt` returns 0"
                );
            }
            _ => panic!("expected TaskUpdate::Set"),
        }
    }
}
