use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{Action, App},
    config::ReasoningEffort,
    ui::render::{
        render,
        test_support::{ask_user_request, buffer_string},
    },
};

#[test]
fn render_replaces_input_with_ask_user_panel() {
    let backend = TestBackend::new(120, 14);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.begin_ask_user("call-1".into(), ask_user_request());

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Clarify implementation"));
    assert!(rendered.contains("Recommended"));
    assert!(rendered.contains("Something else"));
    assert!(rendered.contains("Review"));
    assert!(rendered.contains("Tab to add optional details"));
    assert!(rendered.contains("Which scope should this change cover?"));
}

#[test]
fn render_shows_typed_ask_user_detail_text() {
    let backend = TestBackend::new(120, 16);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.begin_ask_user("call-1".into(), ask_user_request());
    app.apply(Action::AskUserToggleDetailEditor);
    app.apply(Action::Paste("typed details".into()));

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("typed details"));
    assert!(rendered.contains("Details (editing)"));
}
