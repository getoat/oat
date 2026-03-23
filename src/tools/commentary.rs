use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use serde_json::json;

use super::common::ToolExecError;

#[derive(Clone, Default)]
pub struct CommentaryTool;

#[derive(Debug, Deserialize)]
pub struct CommentaryArgs {
    pub message: String,
}

impl CommentaryArgs {
    pub fn validated_message(&self) -> Result<String, ToolExecError> {
        let message = self.message.trim();
        if message.is_empty() {
            return Err(ToolExecError::new("Commentary message must not be empty."));
        }

        Ok(message.to_string())
    }
}

impl Tool for CommentaryTool {
    const NAME: &'static str = "Commentary";
    type Error = ToolExecError;
    type Args = CommentaryArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Share a short progress update with the user during a longer task. This renders as assistant commentary instead of normal tool activity.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "A concise progress update for the user."
                    }
                },
                "required": ["message"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let _ = args.validated_message()?;
        Ok("Commentary delivered.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commentary_args_trim_message() {
        let args = CommentaryArgs {
            message: "  checking the failing test now  ".into(),
        };

        assert_eq!(
            args.validated_message().expect("valid commentary"),
            "checking the failing test now"
        );
    }

    #[test]
    fn commentary_args_reject_empty_message() {
        let args = CommentaryArgs {
            message: "   ".into(),
        };

        let error = args
            .validated_message()
            .expect_err("empty commentary must fail");
        assert!(error.to_string().contains("must not be empty"));
    }
}
