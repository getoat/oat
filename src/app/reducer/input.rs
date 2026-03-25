use crate::app::session::submit::submit_message;
use crate::app::{Action, AppState, Effect, InputContext, ops, query};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::SelectPreviousCommand => {
            match query::input_context(state) {
                InputContext::WriteApproval => {}
                InputContext::ShellApproval { .. } => {
                    ops::approvals::move_shell_approval_selection(state, -1);
                }
                InputContext::AskUser { .. } => {
                    ops::ask_user::move_ask_user_answer_up(state);
                }
                InputContext::PlanReview => ops::planning::move_plan_review_selection(state, -1),
                InputContext::Picker => ops::picker::move_picker_selection_up(state),
                InputContext::CommandPalette => ops::composer::move_command_selection_up(state),
                InputContext::Composer => {
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
            match query::input_context(state) {
                InputContext::WriteApproval => {}
                InputContext::ShellApproval { .. } => {
                    ops::approvals::move_shell_approval_selection(state, 1);
                }
                InputContext::AskUser { .. } => {
                    ops::ask_user::move_ask_user_answer_down(state);
                }
                InputContext::PlanReview => ops::planning::move_plan_review_selection(state, 1),
                InputContext::Picker => ops::picker::move_picker_selection_down(state),
                InputContext::CommandPalette => ops::composer::move_command_selection_down(state),
                InputContext::Composer => {
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
            if matches!(
                query::input_context(state),
                InputContext::WriteApproval
                    | InputContext::ShellApproval { .. }
                    | InputContext::PlanReview
            ) {
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
            match query::input_context(state) {
                InputContext::WriteApproval => return None,
                InputContext::ShellApproval { editing: true, .. } => {
                    ops::approvals::apply_shell_approval_input(state, input)
                }
                InputContext::ShellApproval { .. } | InputContext::PlanReview => {}
                InputContext::AskUser { editing: true } => {
                    ops::ask_user::apply_ask_user_input(state, input)
                }
                InputContext::AskUser { editing: false } => {}
                InputContext::Composer | InputContext::CommandPalette | InputContext::Picker => {
                    ops::composer::apply_composer_input(state, input);
                }
            }
            None
        }
        Action::Paste(text) => {
            match query::input_context(state) {
                InputContext::WriteApproval => return None,
                InputContext::ShellApproval { editing: true, .. } => {
                    ops::approvals::paste_into_shell_approval_detail(state, &text)
                }
                InputContext::ShellApproval { .. } | InputContext::PlanReview => {}
                InputContext::AskUser { editing: true } => {
                    ops::ask_user::paste_into_ask_user_detail(state, &text)
                }
                InputContext::AskUser { editing: false } => {}
                InputContext::Composer | InputContext::CommandPalette | InputContext::Picker => {
                    ops::composer::paste_into_composer(state, &text);
                }
            }
            None
        }
        _ => None,
    }
}
