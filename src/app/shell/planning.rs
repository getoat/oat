use super::*;
use crate::features::planning::accept_review_for_implementation;

impl AppShell {
    pub(crate) fn begin_ask_user(&mut self, request_id: String, request: AskUserRequest) {
        self.session.pending_ask_user = Some(PendingAskUser::new(request_id, request));
        self.ui.pending_ask_user = self
            .session
            .pending_ask_user
            .as_ref()
            .map(AskUserUiState::new);
    }

    pub(crate) fn clear_pending_ask_user(&mut self) {
        self.session.pending_ask_user = None;
        self.ui.pending_ask_user = None;
    }

    pub(crate) fn begin_plan_review(&mut self) {
        show_review(&mut self.session.planning, PlanReviewState::Selection);
        self.ui.plan_review_selected_index = 0;
        self.clear_composer();
    }

    pub(crate) fn begin_plan_review_feedback(&mut self) {
        request_review_changes(&mut self.session.planning);
        self.clear_composer();
    }

    pub(crate) fn clear_plan_review(&mut self) {
        clear_planning(&mut self.session.planning);
        self.ui.plan_review_selected_index = 0;
    }

    pub(crate) fn accept_plan_review_for_implementation(&mut self) {
        accept_review_for_implementation(&mut self.session.planning);
        self.ui.plan_review_selected_index = 0;
    }

    pub(crate) fn selected_plan_review_index(&self) -> Option<usize> {
        self.plan_review_selection_active()
            .then_some(self.ui.plan_review_selected_index)
    }

    pub(crate) fn move_plan_review_selection(&mut self, direction: isize) {
        if !self.plan_review_selection_active() {
            return;
        }

        self.ui.plan_review_selected_index =
            (self.ui.plan_review_selected_index as isize + direction).rem_euclid(2) as usize;
    }

    pub(crate) fn enter_planning_draft_mode(&mut self) {
        crate::features::planning::enter_draft(&mut self.session.planning);
        self.clear_composer();
    }

    pub(crate) fn cancel_planning_draft_mode(&mut self) -> bool {
        if self.session.planning.stage != PlanningStage::Drafting {
            return false;
        }

        cancel_draft(&mut self.session.planning);
        self.clear_composer();
        true
    }

    pub(crate) fn consume_planning_draft_mode(&mut self) -> bool {
        let was_active = self.planning_draft_mode();
        if was_active {
            start_conversation(&mut self.session.planning);
        }
        was_active
    }

    pub(crate) fn begin_planning_conversation(&mut self) {
        start_conversation(&mut self.session.planning);
    }

    pub(crate) fn begin_planning_fanout(&mut self) {
        accept_brief_and_start_fanout(&mut self.session.planning);
    }

    pub(crate) fn begin_planning_finalization(&mut self) {
        start_finalization(&mut self.session.planning);
    }
}
