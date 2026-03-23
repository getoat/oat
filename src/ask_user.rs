use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const SOMETHING_ELSE_ID: &str = "__something_else";
pub const SOMETHING_ELSE_LABEL: &str = "Something else";
pub const RECOMMENDED_LABEL: &str = "Recommended";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserRequest {
    pub title: Option<String>,
    pub questions: Vec<AskUserQuestion>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserQuestion {
    pub id: String,
    pub prompt: String,
    pub answers: Vec<AskUserAnswer>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserAnswer {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserResponse {
    pub questions: Vec<AskUserAnsweredQuestion>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserAnsweredQuestion {
    pub id: String,
    pub prompt: String,
    pub selected_answer: AskUserSelectedAnswer,
    pub details: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AskUserSelectedAnswer {
    pub id: String,
    pub label: String,
    pub is_recommended: bool,
    pub is_something_else: bool,
}

impl AskUserResponse {
    pub fn transcript_summary(&self) -> String {
        let mut lines = vec!["Questions answered:".to_string()];
        for question in &self.questions {
            let mut line = format!(
                "- {}: {}",
                question.prompt.trim(),
                question.selected_answer.label.trim()
            );
            if !question.details.trim().is_empty() {
                line.push_str(&format!(" ({})", question.details.trim()));
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

pub fn validate_request(request: &AskUserRequest) -> Result<(), String> {
    if let Some(title) = &request.title
        && title.trim().is_empty()
    {
        return Err("AskUser title must be non-empty when provided.".into());
    }
    if !(1..=3).contains(&request.questions.len()) {
        return Err("AskUser requires between 1 and 3 questions.".into());
    }

    let mut question_ids = HashSet::new();
    for question in &request.questions {
        if question.id.trim().is_empty() {
            return Err("AskUser question ids must be non-empty.".into());
        }
        if !question_ids.insert(question.id.as_str()) {
            return Err(format!("Duplicate AskUser question id `{}`.", question.id));
        }
        if question.prompt.trim().is_empty() {
            return Err(format!(
                "AskUser question `{}` must have a non-empty prompt.",
                question.id
            ));
        }
        if !(1..=3).contains(&question.answers.len()) {
            return Err(format!(
                "AskUser question `{}` must provide between 1 and 3 answers before the UI adds `Something else`.",
                question.id
            ));
        }

        let mut answer_ids = HashSet::new();
        for answer in &question.answers {
            if answer.id.trim().is_empty() {
                return Err(format!(
                    "AskUser question `{}` has an answer with an empty id.",
                    question.id
                ));
            }
            if !answer_ids.insert(answer.id.as_str()) {
                return Err(format!(
                    "AskUser question `{}` has duplicate answer id `{}`.",
                    question.id, answer.id
                ));
            }
            if answer.label.trim().is_empty() {
                return Err(format!(
                    "AskUser question `{}` has an answer with an empty label.",
                    question.id
                ));
            }
            if answer.label.eq_ignore_ascii_case(SOMETHING_ELSE_LABEL)
                || answer.id == SOMETHING_ELSE_ID
            {
                return Err(format!(
                    "AskUser question `{}` must not include `{}`; the UI adds it automatically.",
                    question.id, SOMETHING_ELSE_LABEL
                ));
            }
            if answer.label.to_ascii_lowercase().contains("recommended") {
                return Err(format!(
                    "AskUser question `{}` must not include `{}` in answer labels; the UI marks the first answer automatically.",
                    question.id, RECOMMENDED_LABEL
                ));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> AskUserRequest {
        AskUserRequest {
            title: Some("Clarify scope".into()),
            questions: vec![AskUserQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                answers: vec![
                    AskUserAnswer {
                        id: "narrow".into(),
                        label: "Narrow".into(),
                    },
                    AskUserAnswer {
                        id: "broad".into(),
                        label: "Broad".into(),
                    },
                ],
            }],
        }
    }

    #[test]
    fn validate_request_accepts_minimal_valid_payload() {
        assert!(validate_request(&sample_request()).is_ok());
    }

    #[test]
    fn validate_request_rejects_manual_something_else() {
        let mut request = sample_request();
        request.questions[0].answers.push(AskUserAnswer {
            id: SOMETHING_ELSE_ID.into(),
            label: SOMETHING_ELSE_LABEL.into(),
        });

        let error = validate_request(&request).expect_err("payload should fail");
        assert!(error.contains("UI adds it automatically"));
    }

    #[test]
    fn validate_request_rejects_manual_recommended_label() {
        let mut request = sample_request();
        request.questions[0].answers[1].label = "Broad (Recommended)".into();

        let error = validate_request(&request).expect_err("payload should fail");
        assert!(error.contains("first answer automatically"));
    }

    #[test]
    fn validate_request_rejects_blank_title() {
        let mut request = sample_request();
        request.title = Some("  ".into());

        let error = validate_request(&request).expect_err("payload should fail");
        assert!(error.contains("title"));
    }

    #[test]
    fn transcript_summary_includes_details_when_present() {
        let response = AskUserResponse {
            questions: vec![AskUserAnsweredQuestion {
                id: "scope".into(),
                prompt: "Which scope?".into(),
                selected_answer: AskUserSelectedAnswer {
                    id: "narrow".into(),
                    label: "Narrow".into(),
                    is_recommended: true,
                    is_something_else: false,
                },
                details: "just the parser".into(),
            }],
        };

        assert_eq!(
            response.transcript_summary(),
            "Questions answered:\n- Which scope?: Narrow (just the parser)"
        );
    }
}
