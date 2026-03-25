use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::app::{Action, EditorInput, EditorKey, InputContext};

const MOUSE_SCROLL_LINES: usize = 3;

#[cfg(test)]
pub fn map_event(event: Event) -> Option<Action> {
    map_event_with_context(event, InputContext::Composer)
}

pub(crate) fn map_event_with_context(event: Event, context: InputContext) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(map_key_event(key, context)),
        Event::Mouse(mouse) => map_mouse_event(mouse),
        Event::Paste(text) => (!matches!(
            context,
            InputContext::WriteApproval | InputContext::PlanReview
        ))
        .then_some(Action::Paste(text)),
        _ => None,
    }
}

fn map_key_event(key: KeyEvent, context: InputContext) -> Action {
    match context {
        InputContext::WriteApproval => match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::CancelPendingReply,
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                Action::ClearComposerOrQuit
            }
            (KeyCode::Char('a'), KeyModifiers::NONE) => Action::ApproveWriteOnce,
            (KeyCode::Char('s'), KeyModifiers::NONE) => Action::ApproveWriteAllSession,
            (KeyCode::Char('d'), KeyModifiers::NONE) => Action::DenyWrite,
            (KeyCode::PageUp, _) => Action::ScrollHistoryPageUp,
            (KeyCode::PageDown, _) => Action::ScrollHistoryPageDown,
            (KeyCode::Home, _) => Action::ScrollHistoryToTop,
            (KeyCode::End, _) => Action::ScrollHistoryToBottom,
            _ => Action::Tick,
        },
        InputContext::ShellApproval {
            editing,
            can_move_up,
            can_move_down,
        } => match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::CancelPendingReply,
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                Action::ClearComposerOrQuit
            }
            (KeyCode::Tab, _) => Action::ShellApprovalToggleDetailEditor,
            (KeyCode::Up, KeyModifiers::NONE) if editing && can_move_up => {
                Action::Editor(editor_input_from_key(key))
            }
            (KeyCode::Down, KeyModifiers::NONE) if editing && can_move_down => {
                Action::Editor(editor_input_from_key(key))
            }
            (KeyCode::Up, KeyModifiers::NONE) => Action::SelectPreviousCommand,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SelectNextCommand,
            (KeyCode::Enter, _) => Action::SubmitMessage,
            (KeyCode::PageUp, _) => Action::ScrollHistoryPageUp,
            (KeyCode::PageDown, _) => Action::ScrollHistoryPageDown,
            (KeyCode::Home, _) => Action::ScrollHistoryToTop,
            (KeyCode::End, _) => Action::ScrollHistoryToBottom,
            _ if editing => Action::Editor(editor_input_from_key(key)),
            _ => Action::Tick,
        },
        InputContext::AskUser { .. } => match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::CancelPendingReply,
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                Action::ClearComposerOrQuit
            }
            (KeyCode::Left, KeyModifiers::NONE) => Action::AskUserTabLeft,
            (KeyCode::Right, KeyModifiers::NONE) => Action::AskUserTabRight,
            (KeyCode::Tab, _) => Action::AskUserToggleDetailEditor,
            (KeyCode::Up, KeyModifiers::NONE) => Action::SelectPreviousCommand,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SelectNextCommand,
            (KeyCode::PageUp, _) => Action::ScrollHistoryPageUp,
            (KeyCode::PageDown, _) => Action::ScrollHistoryPageDown,
            (KeyCode::Home, _) => Action::ScrollHistoryToTop,
            (KeyCode::End, _) => Action::ScrollHistoryToBottom,
            (KeyCode::Enter, _) => Action::SubmitMessage,
            _ => Action::Editor(editor_input_from_key(key)),
        },
        InputContext::PlanReview => match (key.code, key.modifiers) {
            (KeyCode::Char('1'), KeyModifiers::NONE) => Action::AcceptPlanAndImplement,
            (KeyCode::Char('2'), KeyModifiers::NONE) => Action::SuggestPlanChanges,
            (KeyCode::Up, KeyModifiers::NONE) => Action::SelectPreviousCommand,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SelectNextCommand,
            (KeyCode::Enter, _) => Action::SubmitMessage,
            (KeyCode::PageUp, _) => Action::ScrollHistoryPageUp,
            (KeyCode::PageDown, _) => Action::ScrollHistoryPageDown,
            (KeyCode::Home, _) => Action::ScrollHistoryToTop,
            (KeyCode::End, _) => Action::ScrollHistoryToBottom,
            _ => Action::Tick,
        },
        context => match (key.code, key.modifiers) {
            (KeyCode::Esc, _) => Action::CancelPendingReply,
            (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                Action::ClearComposerOrQuit
            }
            (KeyCode::Tab, _) => Action::ToggleMode,
            (KeyCode::Left, KeyModifiers::NONE) if context == InputContext::Picker => {
                Action::PickerTabLeft
            }
            (KeyCode::Right, KeyModifiers::NONE) if context == InputContext::Picker => {
                Action::PickerTabRight
            }
            (KeyCode::Char(' '), KeyModifiers::NONE) if context == InputContext::Picker => {
                Action::TogglePickerSelection
            }
            (KeyCode::Up, KeyModifiers::NONE) => Action::SelectPreviousCommand,
            (KeyCode::Down, KeyModifiers::NONE) => Action::SelectNextCommand,
            (KeyCode::PageUp, _) => Action::ScrollHistoryPageUp,
            (KeyCode::PageDown, _) => Action::ScrollHistoryPageDown,
            (KeyCode::Home, _) => Action::ScrollHistoryToTop,
            (KeyCode::End, _) => Action::ScrollHistoryToBottom,
            (KeyCode::Enter, modifiers) if modifiers.contains(KeyModifiers::ALT) => {
                Action::InsertComposerNewline
            }
            (KeyCode::Char('j'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                Action::InsertComposerNewline
            }
            (KeyCode::Enter, _) => Action::SubmitMessage,
            _ => Action::Editor(editor_input_from_key(key)),
        },
    }
}

fn editor_input_from_key(key: KeyEvent) -> EditorInput {
    EditorInput {
        key: match key.code {
            KeyCode::Backspace => EditorKey::Backspace,
            KeyCode::Enter => EditorKey::Enter,
            KeyCode::Left => EditorKey::Left,
            KeyCode::Right => EditorKey::Right,
            KeyCode::Up => EditorKey::Up,
            KeyCode::Down => EditorKey::Down,
            KeyCode::Home => EditorKey::Home,
            KeyCode::End => EditorKey::End,
            KeyCode::PageUp => EditorKey::PageUp,
            KeyCode::PageDown => EditorKey::PageDown,
            KeyCode::Tab | KeyCode::BackTab => EditorKey::Tab,
            KeyCode::Delete => EditorKey::Delete,
            KeyCode::Esc => EditorKey::Esc,
            KeyCode::Char(value) => EditorKey::Char(value),
            KeyCode::F(value) => EditorKey::F(value),
            _ => EditorKey::Null,
        },
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
}

fn map_mouse_event(mouse: MouseEvent) -> Option<Action> {
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => Some(Action::StartHistorySelection {
            column: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::Drag(MouseButton::Left) => Some(Action::UpdateHistorySelection {
            column: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::Up(MouseButton::Left) => Some(Action::FinishHistorySelection {
            column: mouse.column,
            row: mouse.row,
        }),
        MouseEventKind::ScrollUp => Some(Action::ScrollHistoryUp {
            lines: MOUSE_SCROLL_LINES,
        }),
        MouseEventKind::ScrollDown => Some(Action::ScrollHistoryDown {
            lines: MOUSE_SCROLL_LINES,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_c_maps_to_clear_or_quit() {
        let action = map_event(Event::Key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(action, Some(Action::ClearComposerOrQuit));
    }

    #[test]
    fn escape_maps_to_cancel_pending_reply() {
        let action = map_event(Event::Key(key(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::CancelPendingReply));
    }

    #[test]
    fn enter_maps_to_submit() {
        let action = map_event(Event::Key(key(KeyCode::Enter, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::SubmitMessage));
    }

    #[test]
    fn alt_enter_maps_to_insert_newline() {
        let action = map_event(Event::Key(key(KeyCode::Enter, KeyModifiers::ALT)));
        assert_eq!(action, Some(Action::InsertComposerNewline));
    }

    #[test]
    fn up_arrow_maps_to_previous_command_selection() {
        let action = map_event(Event::Key(key(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::SelectPreviousCommand));
    }

    #[test]
    fn down_arrow_maps_to_next_command_selection() {
        let action = map_event(Event::Key(key(KeyCode::Down, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::SelectNextCommand));
    }

    #[test]
    fn picker_visible_remaps_left_right_and_space() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Left, KeyModifiers::NONE)),
                InputContext::Picker,
            ),
            Some(Action::PickerTabLeft)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Right, KeyModifiers::NONE)),
                InputContext::Picker,
            ),
            Some(Action::PickerTabRight)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char(' '), KeyModifiers::NONE)),
                InputContext::Picker,
            ),
            Some(Action::TogglePickerSelection)
        );
    }

    #[test]
    fn page_up_maps_to_history_scroll() {
        let action = map_event(Event::Key(key(KeyCode::PageUp, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::ScrollHistoryPageUp));
    }

    #[test]
    fn page_down_maps_to_history_scroll() {
        let action = map_event(Event::Key(key(KeyCode::PageDown, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::ScrollHistoryPageDown));
    }

    #[test]
    fn home_maps_to_history_top() {
        let action = map_event(Event::Key(key(KeyCode::Home, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::ScrollHistoryToTop));
    }

    #[test]
    fn end_maps_to_history_bottom() {
        let action = map_event(Event::Key(key(KeyCode::End, KeyModifiers::NONE)));
        assert_eq!(action, Some(Action::ScrollHistoryToBottom));
    }

    #[test]
    fn paste_maps_to_paste_action() {
        let action = map_event(Event::Paste("hello".into()));
        assert_eq!(action, Some(Action::Paste("hello".into())));
    }

    #[test]
    fn approval_prompt_maps_a_s_d_keys_to_write_decisions() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
                InputContext::WriteApproval,
            ),
            Some(Action::ApproveWriteOnce)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('s'), KeyModifiers::NONE)),
                InputContext::WriteApproval,
            ),
            Some(Action::ApproveWriteAllSession)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('d'), KeyModifiers::NONE)),
                InputContext::WriteApproval,
            ),
            Some(Action::DenyWrite)
        );
    }

    #[test]
    fn approval_prompt_ignores_regular_typing_and_paste() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
                InputContext::WriteApproval,
            ),
            Some(Action::Tick)
        );
        assert_eq!(
            map_event_with_context(Event::Paste("hello".into()), InputContext::WriteApproval),
            None
        );
    }

    #[test]
    fn shell_approval_prompt_maps_navigation_and_submit() {
        let context = InputContext::ShellApproval {
            editing: false,
            can_move_up: false,
            can_move_down: false,
        };
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Up, KeyModifiers::NONE)), context),
            Some(Action::SelectPreviousCommand)
        );
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Enter, KeyModifiers::NONE)), context),
            Some(Action::SubmitMessage)
        );
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Tab, KeyModifiers::NONE)), context),
            Some(Action::ShellApprovalToggleDetailEditor)
        );
    }

    #[test]
    fn shell_approval_editor_uses_up_down_for_multiline_navigation_when_possible() {
        let up = key(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(
            map_event_with_context(
                Event::Key(up),
                InputContext::ShellApproval {
                    editing: true,
                    can_move_up: true,
                    can_move_down: false,
                },
            ),
            Some(Action::Editor(editor_input_from_key(up)))
        );

        let down = key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(
            map_event_with_context(
                Event::Key(down),
                InputContext::ShellApproval {
                    editing: true,
                    can_move_up: false,
                    can_move_down: true,
                },
            ),
            Some(Action::Editor(editor_input_from_key(down)))
        );
    }

    #[test]
    fn shell_approval_editor_uses_option_navigation_at_text_bounds() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Up, KeyModifiers::NONE)),
                InputContext::ShellApproval {
                    editing: true,
                    can_move_up: false,
                    can_move_down: true,
                },
            ),
            Some(Action::SelectPreviousCommand)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Down, KeyModifiers::NONE)),
                InputContext::ShellApproval {
                    editing: true,
                    can_move_up: true,
                    can_move_down: false,
                },
            ),
            Some(Action::SelectNextCommand)
        );
    }

    #[test]
    fn plan_review_prompt_maps_numeric_choices() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('1'), KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::AcceptPlanAndImplement)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('2'), KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::SuggestPlanChanges)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Up, KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::SelectPreviousCommand)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Down, KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::SelectNextCommand)
        );
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::SubmitMessage)
        );
    }

    #[test]
    fn plan_review_prompt_ignores_regular_typing_and_paste() {
        assert_eq!(
            map_event_with_context(
                Event::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
                InputContext::PlanReview,
            ),
            Some(Action::Tick)
        );
        assert_eq!(
            map_event_with_context(Event::Paste("hello".into()), InputContext::PlanReview),
            None
        );
    }

    #[test]
    fn ask_user_mode_remaps_tab_and_horizontal_arrows() {
        let context = InputContext::AskUser { editing: false };
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Left, KeyModifiers::NONE)), context),
            Some(Action::AskUserTabLeft)
        );
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Right, KeyModifiers::NONE)), context),
            Some(Action::AskUserTabRight)
        );
        assert_eq!(
            map_event_with_context(Event::Key(key(KeyCode::Tab, KeyModifiers::NONE)), context),
            Some(Action::AskUserToggleDetailEditor)
        );
    }

    #[test]
    fn mouse_wheel_up_maps_to_history_scroll() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::ScrollUp)));
        assert_eq!(action, Some(Action::ScrollHistoryUp { lines: 3 }));
    }

    #[test]
    fn left_mouse_down_starts_history_selection() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::Down(MouseButton::Left))));
        assert_eq!(
            action,
            Some(Action::StartHistorySelection { column: 0, row: 0 })
        );
    }

    #[test]
    fn left_mouse_drag_updates_history_selection() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::Drag(MouseButton::Left))));
        assert_eq!(
            action,
            Some(Action::UpdateHistorySelection { column: 0, row: 0 })
        );
    }

    #[test]
    fn left_mouse_up_finishes_history_selection() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::Up(MouseButton::Left))));
        assert_eq!(
            action,
            Some(Action::FinishHistorySelection { column: 0, row: 0 })
        );
    }

    #[test]
    fn mouse_wheel_down_maps_to_history_scroll() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::ScrollDown)));
        assert_eq!(action, Some(Action::ScrollHistoryDown { lines: 3 }));
    }

    #[test]
    fn non_press_key_events_are_ignored() {
        let action = map_event(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )));
        assert_eq!(action, None);
    }

    #[test]
    fn ordinary_character_maps_to_editor_input() {
        let action = map_event(Event::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)));
        assert_eq!(
            action,
            Some(Action::Editor(editor_input_from_key(key(
                KeyCode::Char('x'),
                KeyModifiers::NONE
            ))))
        );
    }

    #[test]
    fn non_scroll_mouse_events_are_ignored() {
        let action = map_event(Event::Mouse(mouse(MouseEventKind::Moved)));
        assert_eq!(action, None);
    }
}
