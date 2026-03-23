use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_textarea::Input;

use crate::app::Action;

const MOUSE_SCROLL_LINES: usize = 3;

pub fn map_event(event: Event) -> Option<Action> {
    map_event_with_state(event, false, false, false, false)
}

pub fn map_event_with_state(
    event: Event,
    awaiting_write_approval: bool,
    awaiting_ask_user: bool,
    selection_picker_visible: bool,
    awaiting_plan_review: bool,
) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(map_key_event(
            key,
            awaiting_write_approval,
            awaiting_ask_user,
            selection_picker_visible,
            awaiting_plan_review,
        )),
        Event::Mouse(mouse) => map_mouse_event(mouse),
        Event::Paste(text) => {
            (!awaiting_write_approval && !awaiting_plan_review).then_some(Action::Paste(text))
        }
        _ => None,
    }
}

fn map_key_event(
    key: KeyEvent,
    awaiting_write_approval: bool,
    awaiting_ask_user: bool,
    selection_picker_visible: bool,
    awaiting_plan_review: bool,
) -> Action {
    if awaiting_write_approval {
        return match (key.code, key.modifiers) {
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
        };
    }

    if awaiting_ask_user {
        return match (key.code, key.modifiers) {
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
            _ => Action::Editor(Input::from(key)),
        };
    }

    if awaiting_plan_review {
        return match (key.code, key.modifiers) {
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
        };
    }

    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Action::CancelPendingReply,
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Action::ClearComposerOrQuit
        }
        (KeyCode::Tab, _) => Action::ToggleMode,
        (KeyCode::Left, KeyModifiers::NONE) if selection_picker_visible => Action::PickerTabLeft,
        (KeyCode::Right, KeyModifiers::NONE) if selection_picker_visible => Action::PickerTabRight,
        (KeyCode::Char(' '), KeyModifiers::NONE) if selection_picker_visible => {
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
        _ => Action::Editor(Input::from(key)),
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
            map_event_with_state(
                Event::Key(key(KeyCode::Left, KeyModifiers::NONE)),
                false,
                false,
                true,
                false
            ),
            Some(Action::PickerTabLeft)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Right, KeyModifiers::NONE)),
                false,
                false,
                true,
                false
            ),
            Some(Action::PickerTabRight)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char(' '), KeyModifiers::NONE)),
                false,
                false,
                true,
                false
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
            map_event_with_state(
                Event::Key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
                true,
                false,
                false,
                false,
            ),
            Some(Action::ApproveWriteOnce)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('s'), KeyModifiers::NONE)),
                true,
                false,
                false,
                false,
            ),
            Some(Action::ApproveWriteAllSession)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('d'), KeyModifiers::NONE)),
                true,
                false,
                false,
                false,
            ),
            Some(Action::DenyWrite)
        );
    }

    #[test]
    fn approval_prompt_ignores_regular_typing_and_paste() {
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
                true,
                false,
                false,
                false,
            ),
            Some(Action::Tick)
        );
        assert_eq!(
            map_event_with_state(Event::Paste("hello".into()), true, false, false, false),
            None
        );
    }

    #[test]
    fn plan_review_prompt_maps_numeric_choices() {
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('1'), KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::AcceptPlanAndImplement)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('2'), KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::SuggestPlanChanges)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Up, KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::SelectPreviousCommand)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Down, KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::SelectNextCommand)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Enter, KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::SubmitMessage)
        );
    }

    #[test]
    fn plan_review_prompt_ignores_regular_typing_and_paste() {
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Char('x'), KeyModifiers::NONE)),
                false,
                false,
                false,
                true,
            ),
            Some(Action::Tick)
        );
        assert_eq!(
            map_event_with_state(Event::Paste("hello".into()), false, false, false, true),
            None
        );
    }

    #[test]
    fn ask_user_mode_remaps_tab_and_horizontal_arrows() {
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Left, KeyModifiers::NONE)),
                false,
                true,
                false,
                false,
            ),
            Some(Action::AskUserTabLeft)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Right, KeyModifiers::NONE)),
                false,
                true,
                false,
                false,
            ),
            Some(Action::AskUserTabRight)
        );
        assert_eq!(
            map_event_with_state(
                Event::Key(key(KeyCode::Tab, KeyModifiers::NONE)),
                false,
                true,
                false,
                false,
            ),
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
            Some(Action::Editor(Input::from(key(
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
