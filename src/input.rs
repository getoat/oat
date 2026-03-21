use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui_textarea::Input;

use crate::app::Action;

pub fn map_event(event: Event) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(map_key_event(key)),
        Event::Paste(text) => Some(Action::Paste(text)),
        _ => None,
    }
}

fn map_key_event(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
            Action::ClearComposerOrQuit
        }
        (KeyCode::Tab, _) => Action::ToggleMode,
        (KeyCode::Up, KeyModifiers::NONE) => Action::SelectPreviousCommand,
        (KeyCode::Down, KeyModifiers::NONE) => Action::SelectNextCommand,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn ctrl_c_maps_to_clear_or_quit() {
        let action = map_event(Event::Key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(action, Some(Action::ClearComposerOrQuit));
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
    fn paste_maps_to_paste_action() {
        let action = map_event(Event::Paste("hello".into()));
        assert_eq!(action, Some(Action::Paste("hello".into())));
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
}
