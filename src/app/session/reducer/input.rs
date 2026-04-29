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
                InputContext::Stats => ops::stats::move_stats_selection(state, -1),
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
                InputContext::Stats => ops::stats::move_stats_selection(state, 1),
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
                    | InputContext::Stats
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
                InputContext::ShellApproval { .. }
                | InputContext::PlanReview
                | InputContext::Stats => {}
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
                InputContext::ShellApproval { .. }
                | InputContext::PlanReview
                | InputContext::Stats => {}
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

#[cfg(test)]
mod tests {
    use crate::{
        app::{
            Action, ModelPickerTab, SlashCommand, selectable_models_for_tab,
            session::test_support::{new_app, registry_app},
        },
        config::ReasoningEffort,
        features::planning::PlanningAgentConfig,
    };

    #[test]
    fn up_arrow_recalls_previous_submitted_input() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);
        assert_eq!(app.composer().cursor(), (0, 0));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["first"]);
    }

    #[test]
    fn down_arrow_restores_newer_history_and_original_draft() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);
        app.composer_mut().insert_str("draft");

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["draft"]);
        assert_eq!(app.composer().cursor(), (0, 0));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.composer().lines(), ["draft"]);
    }

    #[test]
    fn up_arrow_keeps_multiline_cursor_navigation_when_not_at_top() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("line one");
        app.composer_mut().insert_newline();
        app.composer_mut().insert_str("line two");

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer().lines(), ["line one", "line two"]);
        assert_eq!(app.composer().cursor().0, 0);
    }

    #[test]
    fn up_arrow_on_first_visual_row_moves_to_visual_start_before_history() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("alpha beta");
        app.set_composer_cursor(0, 3);

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer().lines(), ["alpha beta"]);
        assert_eq!(app.composer().cursor(), (0, 0));
    }

    #[test]
    fn down_arrow_on_last_visual_row_moves_to_visual_end_before_history() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("alpha beta");
        app.set_composer_cursor(0, 7);

        app.apply(Action::SelectNextCommand);

        assert_eq!(app.composer().lines(), ["alpha beta"]);
        assert_eq!(app.composer().cursor(), (0, 10));
    }

    #[test]
    fn up_and_down_navigate_wrapped_visual_rows() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.composer_mut().insert_str("alpha beta gamma");

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().cursor(), (0, 10));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().cursor(), (0, 5));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.composer().cursor(), (0, 10));
    }

    #[test]
    fn command_selection_wraps() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/");
        app.sync_command_selection();

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.selected_command(), Some(SlashCommand::Quit));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.selected_command(), Some(SlashCommand::NewSession));
    }

    #[test]
    fn plan_review_selection_wraps_with_arrow_keys() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.selected_plan_review_index(), Some(1));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.selected_plan_review_index(), Some(0));
    }

    #[test]
    fn toggling_planning_picker_selection_persists_default_effort() {
        let mut app = registry_app(true);
        app.open_model_picker();
        app.apply(Action::PickerTabRight);

        let target_index =
            selectable_models_for_tab(ModelPickerTab::PlanningAgents, "gpt-5.4-mini")
                .iter()
                .position(|model| model.name == "gpt-5.4")
                .expect("target model in picker");
        for _ in 0..target_index {
            app.apply(Action::SelectNextCommand);
        }

        let effect = app.apply(Action::TogglePickerSelection);

        assert_eq!(
            effect,
            Some(crate::app::Effect::SetPlanningAgents {
                planning_agents: vec![PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning: ReasoningEffort::Low.into(),
                }],
            })
        );
    }
}
