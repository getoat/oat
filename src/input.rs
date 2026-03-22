use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_textarea::Input;

use crate::app::Action;

const MOUSE_SCROLL_LINES: usize = 3;

pub fn map_event(event: Event) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(map_key_event(key)),
        Event::Mouse(mouse) => map_mouse_event(mouse),
        Event::Paste(text) => Some(Action::Paste(text)),
        _ => None,
    }
}

fn map_key_event(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => Action::CancelPendingReply,
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Action::ClearComposerOrQuit
        }
        (KeyCode::Tab, _) => Action::ToggleMode,
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
