use ratatui::{Terminal, backend::TestBackend, style::Modifier};

use crate::{
    app::App,
    config::ReasoningEffort,
    ui::render::{
        render,
        test_support::{buffer_lines, word_has_modifier},
    },
};

#[test]
fn render_input_does_not_underline_the_cursor_line() {
    let backend = TestBackend::new(60, 8);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("draft");

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let buffer = terminal.backend().buffer();
    assert!(
        !word_has_modifier(buffer, "draft", Modifier::UNDERLINED),
        "expected input text not to render with underline"
    );
}

#[test]
fn render_wraps_composer_text_instead_of_horizontally_scrolling() {
    let backend = TestBackend::new(16, 10);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
    app.composer_mut().insert_str("alpha beta gamma");

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered_lines = buffer_lines(terminal.backend());
    assert!(
        rendered_lines
            .iter()
            .any(|line| line.contains("alpha beta")),
        "expected first wrapped row in composer: {rendered_lines:?}"
    );
    assert!(
        rendered_lines.iter().any(|line| line.contains("gamma")),
        "expected later wrapped row in composer: {rendered_lines:?}"
    );
}
