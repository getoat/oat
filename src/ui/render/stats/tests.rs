use std::collections::BTreeMap;

use ratatui::{Terminal, backend::TestBackend};

use crate::{
    app::{Action, App},
    config::ReasoningEffort,
    stats::{StatsReport, StatsTotals, ThinkingTokenTotals},
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

fn sample_report() -> StatsReport {
    let current = StatsTotals {
        request_count: 3,
        tool_call_count: 2,
        input_tokens: 1_200,
        cached_input_tokens: 200,
        output_tokens: 600,
        throughput_output_tokens: 600,
        thinking_tokens: ThinkingTokenTotals {
            tokens: 120,
            available_request_count: 3,
            unavailable_request_count: 0,
            estimated_request_count: 0,
        },
        estimated_cost_nanos_usd: 12_300_000,
        ttfb_total_millis: 150,
        ttfb_recorded_request_count: 3,
        total_request_millis: 3_000,
        timed_request_count: 3,
        completed_request_count: 3,
        usage_recorded_request_count: 3,
        usage_recorded_request_millis: 3_000,
        ..StatsTotals::default()
    };
    let historical = StatsTotals {
        request_count: 4,
        tool_call_count: 1,
        input_tokens: 4_000,
        cached_input_tokens: 500,
        output_tokens: 2_000,
        throughput_output_tokens: 2_000,
        thinking_tokens: ThinkingTokenTotals {
            tokens: 0,
            available_request_count: 0,
            unavailable_request_count: 4,
            estimated_request_count: 0,
        },
        estimated_cost_nanos_usd: 45_600_000,
        ttfb_total_millis: 800,
        ttfb_recorded_request_count: 4,
        total_request_millis: 8_000,
        timed_request_count: 4,
        completed_request_count: 4,
        usage_recorded_request_count: 4,
        usage_recorded_request_millis: 8_000,
        ..StatsTotals::default()
    };

    let mut current_models = BTreeMap::new();
    current_models.insert("gpt-5.4".into(), current);
    current_models.insert(
        "gpt-5.4-mini".into(),
        StatsTotals {
            request_count: 1,
            output_tokens: 50,
            throughput_output_tokens: 50,
            completed_request_count: 1,
            timed_request_count: 1,
            usage_recorded_request_count: 1,
            usage_recorded_request_millis: 500,
            total_request_millis: 500,
            thinking_tokens: ThinkingTokenTotals {
                tokens: 10,
                available_request_count: 1,
                unavailable_request_count: 0,
                estimated_request_count: 0,
            },
            ..StatsTotals::default()
        },
    );
    let mut historical_models = BTreeMap::new();
    historical_models.insert("gpt-5.4".into(), historical);
    historical_models.insert(
        "gpt-5.4-mini".into(),
        StatsTotals {
            request_count: 2,
            output_tokens: 200,
            throughput_output_tokens: 200,
            completed_request_count: 2,
            timed_request_count: 2,
            usage_recorded_request_count: 2,
            usage_recorded_request_millis: 2_000,
            total_request_millis: 2_000,
            thinking_tokens: ThinkingTokenTotals {
                tokens: 0,
                available_request_count: 0,
                unavailable_request_count: 2,
                estimated_request_count: 0,
            },
            ..StatsTotals::default()
        },
    );

    StatsReport {
        current,
        historical,
        current_models,
        historical_models,
        historical_session_count: 2,
    }
}

fn stats_app() -> App {
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    crate::app::ops::stats::open_stats_screen(app.state_mut(), sample_report());
    app
}

#[test]
fn render_overview_stats_screen() {
    let mut app = stats_app();

    let rendered = draw_app(&mut app, 120, 20);

    assert!(rendered.contains("Stats"));
    assert!(rendered.contains("Overview"));
    assert!(rendered.contains("Current Session"));
    assert!(rendered.contains("Historical (2)"));
    assert!(rendered.contains("Thinking tokens"));
    assert!(rendered.contains("Avg AI wait"));
    assert!(rendered.contains("In flight"));
    assert!(rendered.contains("n/a"));
}

#[test]
fn render_session_models_table() {
    let mut app = stats_app();
    app.apply(Action::StatsTabRight);

    let rendered = draw_app(&mut app, 120, 20);

    assert!(rendered.contains("Session Models"));
    assert!(rendered.contains("gpt-5.4"));
    assert!(rendered.contains("gpt-5.4-mini"));
    assert!(rendered.contains("Think"));
}

#[test]
fn render_historical_models_table() {
    let mut app = stats_app();
    app.apply(Action::StatsTabRight);
    app.apply(Action::StatsTabRight);

    let rendered = draw_app(&mut app, 120, 20);

    assert!(rendered.contains("Historical Models"));
    assert!(rendered.contains("gpt-5.4"));
    assert!(rendered.contains("n/a"));
}

#[test]
fn partial_thinking_coverage_is_marked_without_hiding_value() {
    let mut report = sample_report();
    report.current.thinking_tokens.unavailable_request_count = 1;
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    crate::app::ops::stats::open_stats_screen(app.state_mut(), report);

    let rendered = draw_app(&mut app, 120, 20);

    assert!(rendered.contains("120*"));
}

#[test]
fn estimated_thinking_coverage_is_marked() {
    let mut report = sample_report();
    report.current.thinking_tokens.estimated_request_count = 1;
    let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::Medium);
    crate::app::ops::stats::open_stats_screen(app.state_mut(), report);

    let rendered = draw_app(&mut app, 120, 20);

    assert!(rendered.contains("~120"));
}

#[test]
fn narrow_model_table_drops_optional_columns() {
    let mut app = stats_app();
    app.apply(Action::StatsTabRight);

    let rendered = draw_app(&mut app, 60, 18);

    assert!(rendered.contains("Think"));
    assert!(!rendered.contains("Cost"));
    assert!(!rendered.contains("Tok/s"));
}
