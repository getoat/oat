use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{Action, App, Effect},
    config::ReasoningEffort,
    ui::{
        render::{render, test_support::buffer_string, test_support::word_has_foreground},
        theme::accent_color,
    },
};

#[test]
fn render_shows_plan_footer_and_accent_during_planning_draft() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.enter_planning_draft_mode();

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Plan"));
    assert!(word_has_foreground(
        terminal.backend().buffer(),
        "Plan",
        accent_color(app.mode(), true),
    ));
}

#[test]
fn render_replaces_input_with_plan_review_prompt() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.begin_plan_review();

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Plan Ready"));
    assert!(rendered.contains("Accept this plan and begin implementation"));
    assert!(rendered.contains("Discuss this plan"));
    assert!(rendered.contains("› [1]"));
}

#[test]
fn render_highlights_selected_plan_review_option() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.begin_plan_review();
    app.apply(Action::SelectNextCommand);

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("› [2]"));
}

#[test]
fn render_shows_plan_footer_while_planning_run_is_pending() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.enter_planning_draft_mode();
    app.composer_mut().insert_str("Make a roadmap");
    let effect = app.apply(Action::SubmitMessage);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Plan"));
    assert!(!app.planning_draft_mode());
    assert!(app.plan_active());
}

#[test]
fn render_shows_write_footer_after_accepting_plan() {
    let backend = TestBackend::new(120, 12);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
    app.state_mut().session.planning.proposed_plan =
        Some(crate::features::planning::ProposedPlan {
            markdown: "# Test Plan\n\n- step one".into(),
            raw_block: "<proposed_plan>\n# Test Plan\n\n- step one\n</proposed_plan>".into(),
        });
    app.begin_plan_review();

    let effect = app.apply(Action::AcceptPlanAndImplement);
    assert!(matches!(effect, Some(Effect::PromptModel { .. })));

    terminal
        .draw(|frame| render(frame, &mut app))
        .expect("render succeeds");

    let rendered = buffer_string(terminal.backend());
    assert!(rendered.contains("Write"));
    assert!(!rendered.contains("Plan Ready"));
    assert!(word_has_foreground(
        terminal.backend().buffer(),
        "Write",
        accent_color(app.mode(), false),
    ));
}
