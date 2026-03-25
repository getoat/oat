use ratatui::{
    backend::TestBackend,
    style::{Color, Modifier},
};

use crate::ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest};

pub(crate) fn ask_user_request() -> AskUserRequest {
    AskUserRequest {
        title: Some("Clarify implementation".into()),
        questions: vec![AskUserQuestion {
            id: "scope".into(),
            prompt: "Which scope should this change cover?".into(),
            answers: vec![
                AskUserAnswer {
                    id: "narrow".into(),
                    label: "Only the parser".into(),
                },
                AskUserAnswer {
                    id: "broad".into(),
                    label: "The full pipeline".into(),
                },
            ],
        }],
    }
}

pub(crate) fn buffer_string(backend: &TestBackend) -> String {
    backend
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>()
}

pub(crate) fn buffer_lines(backend: &TestBackend) -> Vec<String> {
    let buffer = backend.buffer();
    let width = buffer.area.width as usize;
    buffer
        .content
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

pub(crate) fn word_has_modifier(
    buffer: &ratatui::buffer::Buffer,
    word: &str,
    modifier: Modifier,
) -> bool {
    let width = buffer.area.width as usize;
    let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

    for row in buffer.content.chunks(width) {
        for start in 0..=row.len().saturating_sub(symbols.len()) {
            if row[start..start + symbols.len()]
                .iter()
                .map(|cell| cell.symbol())
                .eq(symbols.iter().map(String::as_str))
                && row[start..start + symbols.len()]
                    .iter()
                    .all(|cell| cell.modifier.contains(modifier))
            {
                return true;
            }
        }
    }

    false
}

pub(crate) fn word_has_foreground(
    buffer: &ratatui::buffer::Buffer,
    word: &str,
    foreground: Color,
) -> bool {
    let width = buffer.area.width as usize;
    let symbols = word.chars().map(|ch| ch.to_string()).collect::<Vec<_>>();

    for row in buffer.content.chunks(width) {
        for start in 0..=row.len().saturating_sub(symbols.len()) {
            if row[start..start + symbols.len()]
                .iter()
                .map(|cell| cell.symbol())
                .eq(symbols.iter().map(String::as_str))
                && row[start..start + symbols.len()]
                    .iter()
                    .all(|cell| cell.fg == foreground)
            {
                return true;
            }
        }
    }

    false
}
