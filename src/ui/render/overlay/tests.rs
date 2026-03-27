use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{Action, App},
    config::ReasoningEffort,
    ui::render::{render, test_support::buffer_string},
};

#[test]
fn render_shows_model_picker_details() {
    let backend = TestBackend::new(160, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Models"));
    assert!(rendered.contains("gpt-5.4"));
    assert!(rendered.contains("Azure OpenAI"));
    assert!(rendered.contains("ctx 272K"));
    assert!(!rendered.contains(">272K"));
}

#[test]
fn render_shows_reasoning_picker() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_reasoning_picker();

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Reasoning"));
    assert!(rendered.contains("low"));
    assert!(rendered.contains("medium"));
    assert!(rendered.contains("high"));
}

#[test]
fn render_model_picker_scrolls_to_lower_provider_rows() {
    let backend = TestBackend::new(160, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();

    for _ in 0..5 {
        app.apply(Action::SelectNextCommand);
    }

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Chutes AI"));
    assert!(rendered.contains("MiniMaxAI/MiniMax-M2.5-TEE"));
}
