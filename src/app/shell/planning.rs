use super::App;
#[cfg(test)]
use crate::ask_user::AskUserRequest;

impl App {
    #[cfg(test)]
    pub(crate) fn begin_ask_user(&mut self, request_id: String, request: AskUserRequest) {
        self.reducer_context().begin_ask_user(request_id, request);
    }

    #[cfg(test)]
    pub(crate) fn begin_plan_review(&mut self) {
        self.reducer_context().begin_plan_review();
    }

    #[cfg(test)]
    pub(crate) fn begin_plan_review_feedback(&mut self) {
        self.reducer_context().begin_plan_review_feedback();
    }

    pub(crate) fn selected_plan_review_index(&self) -> Option<usize> {
        self.plan_review_selection_active()
            .then_some(self.ui.plan_review_selected_index)
    }

    #[cfg(test)]
    pub(crate) fn enter_planning_draft_mode(&mut self) {
        self.reducer_context().enter_planning_draft_mode();
    }

    #[cfg(test)]
    pub(crate) fn begin_planning_conversation(&mut self) {
        self.reducer_context().begin_planning_conversation();
    }
}
