use crate::ask_user::{
    AskUserAnswer, AskUserAnsweredQuestion, AskUserQuestion, AskUserRequest, AskUserResponse,
    AskUserSelectedAnswer, SOMETHING_ELSE_ID, SOMETHING_ELSE_LABEL,
};

#[derive(Debug)]
pub struct PendingAskUser {
    pub request_id: String,
    pub title: String,
    pub questions: Vec<PendingAskUserQuestion>,
}

impl PendingAskUser {
    pub fn new(request_id: String, request: AskUserRequest) -> Self {
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Ask User")
            .to_string();
        let questions = request
            .questions
            .into_iter()
            .map(PendingAskUserQuestion::from_request)
            .collect();
        Self {
            request_id,
            title,
            questions,
        }
    }

    pub fn is_complete<F>(&self, mut detail_for_question: F) -> bool
    where
        F: FnMut(usize) -> String,
    {
        self.questions
            .iter()
            .enumerate()
            .all(|(index, question)| question.is_complete(detail_for_question(index)))
    }

    pub fn response<F>(&self, mut detail_for_question: F) -> AskUserResponse
    where
        F: FnMut(usize) -> String,
    {
        AskUserResponse {
            questions: self
                .questions
                .iter()
                .enumerate()
                .map(|(index, question)| question.response(detail_for_question(index)))
                .collect(),
        }
    }
}

#[derive(Debug)]
pub struct PendingAskUserQuestion {
    pub id: String,
    pub prompt: String,
    pub answers: Vec<PendingAskUserAnswer>,
    pub selected_index: usize,
}

impl PendingAskUserQuestion {
    fn from_request(question: AskUserQuestion) -> Self {
        let mut answers = question
            .answers
            .into_iter()
            .enumerate()
            .map(|(index, answer)| PendingAskUserAnswer::from_request(answer, index == 0))
            .collect::<Vec<_>>();
        answers.push(PendingAskUserAnswer {
            id: SOMETHING_ELSE_ID.into(),
            label: SOMETHING_ELSE_LABEL.into(),
            is_recommended: false,
            is_something_else: true,
        });

        Self {
            id: question.id,
            prompt: question.prompt,
            answers,
            selected_index: 0,
        }
    }

    pub fn selected_answer(&self) -> &PendingAskUserAnswer {
        &self.answers[self.selected_index]
    }

    pub fn is_complete(&self, detail_text: String) -> bool {
        !self.selected_answer().is_something_else || !detail_text.trim().is_empty()
    }

    fn response(&self, detail_text: String) -> AskUserAnsweredQuestion {
        let selected = self.selected_answer();
        AskUserAnsweredQuestion {
            id: self.id.clone(),
            prompt: self.prompt.clone(),
            selected_answer: AskUserSelectedAnswer {
                id: selected.id.clone(),
                label: selected.label.clone(),
                is_recommended: selected.is_recommended,
                is_something_else: selected.is_something_else,
            },
            details: detail_text.trim().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAskUserAnswer {
    pub id: String,
    pub label: String,
    pub is_recommended: bool,
    pub is_something_else: bool,
}

impl PendingAskUserAnswer {
    fn from_request(answer: AskUserAnswer, is_recommended: bool) -> Self {
        Self {
            id: answer.id,
            label: answer.label,
            is_recommended,
            is_something_else: false,
        }
    }
}
