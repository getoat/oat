use ratatui_textarea::{Input, Key, TextArea};

use crate::{
    app::session::{AccessMode, EditorInput, EditorKey},
    composer::ComposerLayout,
    model_registry,
};

use super::picker::SelectionPicker;

pub const DEFAULT_COMPOSER_WRAP_WIDTH: usize = 80;

pub fn new_composer() -> TextArea<'static> {
    new_composer_with_text("")
}

pub fn split_command_query(query: &str) -> (&str, &str) {
    let mut parts = query.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arguments = parts.next().unwrap_or("").trim();
    (name, arguments)
}

pub fn new_composer_with_text(text: &str) -> TextArea<'static> {
    new_text_area_with_text(text, "Send a message...")
}

pub fn new_text_area_with_text(text: &str, placeholder: &str) -> TextArea<'static> {
    let mut composer = if text.is_empty() {
        TextArea::default()
    } else {
        TextArea::from(text.lines())
    };
    composer.set_placeholder_text(placeholder);
    composer
}

pub fn textarea_input(input: &EditorInput) -> Input {
    Input {
        key: match input.key {
            EditorKey::Char(value) => Key::Char(value),
            EditorKey::F(value) => Key::F(value),
            EditorKey::Backspace => Key::Backspace,
            EditorKey::Enter => Key::Enter,
            EditorKey::Left => Key::Left,
            EditorKey::Right => Key::Right,
            EditorKey::Up => Key::Up,
            EditorKey::Down => Key::Down,
            EditorKey::Tab => Key::Tab,
            EditorKey::Delete => Key::Delete,
            EditorKey::Home => Key::Home,
            EditorKey::End => Key::End,
            EditorKey::PageUp => Key::PageUp,
            EditorKey::PageDown => Key::PageDown,
            EditorKey::Esc => Key::Esc,
            EditorKey::Null => Key::Null,
        },
        ctrl: input.ctrl,
        alt: input.alt,
        shift: input.shift,
    }
}

pub fn normalize_pasted_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn welcome_message(model_name: &str, mode: AccessMode) -> String {
    crate::app::session::startup_banner_message(model_name, mode)
}

pub fn picker_height(picker: &SelectionPicker) -> u16 {
    let line_count = match picker {
        SelectionPicker::Model { .. } => model_registry::models().len().max(1) + 1,
        SelectionPicker::Reasoning { options, .. } => options.len(),
    }
    .clamp(1, 4) as u16;

    line_count + 2
}

#[derive(Debug)]
pub struct ComposerUiState {
    pub composer: TextArea<'static>,
    pub wrap_width: usize,
    pub visual_column: Option<usize>,
    pub layout_cache: Option<ComposerLayout>,
}

impl Default for ComposerUiState {
    fn default() -> Self {
        Self {
            composer: new_composer(),
            wrap_width: DEFAULT_COMPOSER_WRAP_WIDTH,
            visual_column: None,
            layout_cache: None,
        }
    }
}
