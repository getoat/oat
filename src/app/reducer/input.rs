use crate::app::session::submit::submit_message;
use crate::app::{Action, AppState, Effect, InputTarget, ops, query};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::SelectPreviousCommand => {
            match query::active_input_target(state) {
                InputTarget::ShellApprovalSelection | InputTarget::ShellApprovalEditor => {
                    ops::approvals::move_shell_approval_selection(state, -1);
                }
                InputTarget::AskUserSelection | InputTarget::AskUserEditor => {
                    ops::ask_user::move_ask_user_answer_up(state);
                }
                InputTarget::PlanReviewSelection => {
                    ops::planning::move_plan_review_selection(state, -1)
                }
                InputTarget::Picker => ops::picker::move_picker_selection_up(state),
                InputTarget::CommandPalette => ops::composer::move_command_selection_up(state),
                InputTarget::Composer => {
                    if !(ops::composer::should_recall_previous_input(state)
                        && ops::composer::recall_previous_input(state))
                    {
                        ops::composer::move_composer_cursor_up(state);
                    }
                }
            }
            None
        }
        Action::SelectNextCommand => {
            match query::active_input_target(state) {
                InputTarget::ShellApprovalSelection | InputTarget::ShellApprovalEditor => {
                    ops::approvals::move_shell_approval_selection(state, 1);
                }
                InputTarget::AskUserSelection | InputTarget::AskUserEditor => {
                    ops::ask_user::move_ask_user_answer_down(state);
                }
                InputTarget::PlanReviewSelection => {
                    ops::planning::move_plan_review_selection(state, 1)
                }
                InputTarget::Picker => ops::picker::move_picker_selection_down(state),
                InputTarget::CommandPalette => ops::composer::move_command_selection_down(state),
                InputTarget::Composer => {
                    if !(ops::composer::should_recall_next_input(state)
                        && ops::composer::recall_next_input(state))
                    {
                        ops::composer::move_composer_cursor_down(state);
                    }
                }
            }
            None
        }
        Action::InsertComposerNewline => {
            if query::has_pending_write_approval(state)
                || query::has_pending_shell_approval(state)
                || query::plan_review_selection_active(state)
            {
                return None;
            }
            ops::composer::insert_composer_newline(state);
            None
        }
        Action::SubmitMessage => submit_message(state),
        Action::TogglePickerSelection => ops::picker::toggle_picker_selection(state)
            .map(|planning_agents| Effect::SetPlanningAgents { planning_agents }),
        Action::PickerTabLeft => {
            ops::picker::move_picker_tab_left(state);
            None
        }
        Action::PickerTabRight => {
            ops::picker::move_picker_tab_right(state);
            None
        }
        Action::AskUserTabLeft => {
            ops::ask_user::move_ask_user_tab_left(state);
            None
        }
        Action::AskUserTabRight => {
            ops::ask_user::move_ask_user_tab_right(state);
            None
        }
        Action::AskUserToggleDetailEditor => {
            ops::ask_user::toggle_ask_user_detail_editing(state);
            None
        }
        Action::ShellApprovalToggleDetailEditor => {
            ops::approvals::toggle_shell_approval_detail_editing(state);
            None
        }
        Action::Editor(input) => {
            if query::has_pending_write_approval(state) {
                return None;
            }
            match query::active_input_target(state) {
                InputTarget::ShellApprovalEditor => {
                    ops::approvals::apply_shell_approval_input(state, input)
                }
                InputTarget::ShellApprovalSelection | InputTarget::PlanReviewSelection => {}
                InputTarget::AskUserEditor => ops::ask_user::apply_ask_user_input(state, input),
                InputTarget::AskUserSelection => {}
                InputTarget::Composer | InputTarget::CommandPalette | InputTarget::Picker => {
                    ops::composer::apply_composer_input(state, input);
                }
            }
            None
        }
        Action::Paste(text) => {
            if query::has_pending_write_approval(state) {
                return None;
            }
            match query::active_input_target(state) {
                InputTarget::ShellApprovalEditor => {
                    ops::approvals::paste_into_shell_approval_detail(state, &text)
                }
                InputTarget::ShellApprovalSelection | InputTarget::PlanReviewSelection => {}
                InputTarget::AskUserEditor => {
                    ops::ask_user::paste_into_ask_user_detail(state, &text)
                }
                InputTarget::AskUserSelection => {}
                InputTarget::Composer | InputTarget::CommandPalette | InputTarget::Picker => {
                    ops::composer::paste_into_composer(state, &text);
                }
            }
            None
        }
        _ => None,
    }
}
