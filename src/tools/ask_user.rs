use rig::{completion::ToolDefinition, tool::Tool};
use serde_json::json;

use crate::ask_user::AskUserRequest;

use super::common::ToolExecError;

#[derive(Clone, Default)]
pub struct AskUserTool;

impl Tool for AskUserTool {
    const NAME: &'static str = "AskUser";
    type Error = ToolExecError;
    type Args = AskUserRequest;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Ask the user 1 to 3 clarification questions through the interactive AskUser UI. Provide only meaningful multiple-choice options. For each question, include 1 to 3 answers and put the recommended answer first. Do not include `Something else`; the UI adds it automatically and requires user input when selected.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Optional overall title for the set of clarification questions."
                    },
                    "questions": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 3,
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Stable question identifier used in the tool result."
                                },
                                "prompt": {
                                    "type": "string",
                                    "description": "The question shown to the user."
                                },
                                "answers": {
                                    "type": "array",
                                    "minItems": 1,
                                    "maxItems": 3,
                                    "description": "Meaningful answer options. The first answer is treated as the recommended option.",
                                    "items": {
                                        "type": "object",
                                        "properties": {
                                            "id": {
                                                "type": "string",
                                                "description": "Stable answer identifier used in the tool result."
                                            },
                                            "label": {
                                                "type": "string",
                                                "description": "Answer label shown to the user."
                                            }
                                        },
                                        "required": ["id", "label"]
                                    }
                                }
                            },
                            "required": ["id", "prompt", "answers"]
                        }
                    }
                },
                "required": ["questions"]
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Err(ToolExecError::new(
            "AskUser requires the interactive runtime and should have been intercepted before execution.",
        ))
    }
}
