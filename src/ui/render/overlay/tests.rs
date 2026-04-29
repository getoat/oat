use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{
        Action, App, ModelPickerTab, SelectionPicker, SessionPickerEntry, selectable_models_for_tab,
    },
    config::{ReasoningEffort, ReasoningSetting},
    ui::render::{render, test_support::buffer_string},
};

fn draw_app(app: &mut App, width: u16, height: u16) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| render(frame, app))
        .expect("render succeeds");
    buffer_string(terminal.backend())
}

#[test]
fn render_shows_model_picker_details() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();

    let rendered = draw_app(&mut app, 160, 12);
    assert!(rendered.contains("Models"));
    assert!(rendered.contains("gpt-5.4"));
    assert!(rendered.contains("provider"));
    assert!(rendered.contains("$in"));
    assert!(rendered.contains("$cache"));
    assert!(rendered.contains("$out"));
    assert!(rendered.contains("Azure OpenAI"));
    assert!(rendered.contains("272K"));
    assert!(!rendered.contains(">272K"));
}

#[test]
fn render_shows_reasoning_picker() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_reasoning_picker();

    let rendered = draw_app(&mut app, 120, 12);
    assert!(rendered.contains("Reasoning"));
    assert!(rendered.contains("low"));
    assert!(rendered.contains("medium"));
    assert!(rendered.contains("high"));
}

#[test]
fn render_model_picker_scrolls_to_lower_provider_rows() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();

    let chutes_index = selectable_models_for_tab(ModelPickerTab::NormalAgent, "gpt-5.4")
        .iter()
        .position(|model| model.name == "zai-org/GLM-5-TEE")
        .expect("chutes model in picker");

    for _ in 0..chutes_index {
        app.apply(Action::SelectNextCommand);
    }

    let rendered = draw_app(&mut app, 160, 12);
    assert!(rendered.contains("Chutes AI"));
    assert!(rendered.contains("zai-org/GLM-5-TEE"));
}

#[test]
fn render_reasoning_picker_scrolls_to_selected_option_on_small_screens() {
    let mut app = App::new(
        true,
        false,
        "xiaomi/mimo-v2-pro",
        ReasoningSetting::Gpt(ReasoningEffort::XHigh),
    );
    app.open_reasoning_picker();

    let rendered = draw_app(&mut app, 120, 10);
    assert!(rendered.contains("xhigh"));
    assert!(!rendered.contains("> medium"));
}

#[test]
fn render_command_palette_scrolls_to_selected_command_on_small_screens() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.composer_mut().insert_str("/");
    app.sync_command_selection();

    for _ in 0..14 {
        app.apply(Action::SelectNextCommand);
    }

    let rendered = draw_app(&mut app, 120, 10);
    assert!(rendered.contains("/quit"));
    assert!(!rendered.contains("/new"));
}

#[test]
fn render_model_picker_normal_tab_keeps_selected_item_visible_on_small_screens() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();

    let target_index = selectable_models_for_tab(ModelPickerTab::NormalAgent, "gpt-5.4")
        .iter()
        .position(|model| model.name == "xiaomi/mimo-v2-flash")
        .expect("target model in picker");
    let current_index = selectable_models_for_tab(ModelPickerTab::NormalAgent, "gpt-5.4")
        .iter()
        .position(|model| model.name == "gpt-5.4")
        .expect("current model in picker");

    for _ in current_index..target_index {
        app.apply(Action::SelectNextCommand);
    }

    let rendered = draw_app(&mut app, 160, 10);
    assert!(rendered.contains("xiaomi/mimo-v2-flash"));
}

#[test]
fn render_model_picker_planning_tab_keeps_selected_item_visible_on_small_screens() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();
    app.apply(Action::PickerTabRight);

    let target_index = selectable_models_for_tab(ModelPickerTab::PlanningAgents, "gpt-5.4")
        .iter()
        .position(|model| model.name == "xiaomi/mimo-v2-flash")
        .expect("target model in picker");

    for _ in 0..target_index {
        app.apply(Action::SelectNextCommand);
    }

    let rendered = draw_app(&mut app, 160, 10);
    assert!(rendered.contains("xiaomi/mimo-v2-flash"));
    assert!(rendered.contains("OpenRouter"));
    assert!(rendered.contains("Enter sets reasoning"));
}

#[test]
fn render_model_picker_safety_tab_keeps_selected_item_visible_on_small_screens() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.open_model_picker();
    app.apply(Action::PickerTabRight);
    app.apply(Action::PickerTabRight);

    let target_index = selectable_models_for_tab(ModelPickerTab::SafetyModel, "gpt-5.4")
        .iter()
        .position(|model| model.name == "xiaomi/mimo-v2-flash")
        .expect("target model in picker");
    let current_index = selectable_models_for_tab(ModelPickerTab::SafetyModel, "gpt-5.4")
        .iter()
        .position(|model| model.name == "gpt-5.4")
        .expect("current model in picker");

    for _ in current_index..target_index {
        app.apply(Action::SelectNextCommand);
    }

    let rendered = draw_app(&mut app, 160, 10);
    assert!(rendered.contains("xiaomi/mimo-v2-flash"));
    assert!(rendered.contains("OpenRouter"));
    assert!(rendered.contains("Enter sets reasoning"));
}

#[test]
fn render_session_picker_marks_non_resumable_entries() {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    app.state_mut().ui.picker = Some(SelectionPicker::Session {
        entries: vec![
            SessionPickerEntry {
                session_id: "session-1".into(),
                title: "Resumable".into(),
                detail: "Last active Mar 29, 2026 12:00 UTC | gpt-5.4".into(),
                resumable: true,
            },
            SessionPickerEntry {
                session_id: "session-2".into(),
                title: "Unavailable".into(),
                detail:
                    "Last active Mar 29, 2026 11:00 UTC | missing-model | saved model unavailable"
                        .into(),
                resumable: false,
            },
        ],
        selected_index: 1,
    });

    let rendered = draw_app(&mut app, 160, 12);
    assert!(rendered.contains("Sessions"));
    assert!(rendered.contains("Unavailable"));
    assert!(rendered.contains("not resumable"));
}
