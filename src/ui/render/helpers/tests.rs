use ratatui::text::Line;

fn line_text(line: Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn mode_status_label_marks_session_preapproved_write_mode() {
    assert_eq!(
        super::mode_status_label(
            crate::app::AccessMode::ReadWrite,
            crate::app::ApprovalMode::Manual,
            false,
        ),
        "Write"
    );
    assert_eq!(
        super::mode_status_label(
            crate::app::AccessMode::ReadWrite,
            crate::app::ApprovalMode::Disabled,
            false,
        ),
        "Write (!)"
    );
}

#[test]
fn mode_status_label_prefers_plan_state() {
    assert_eq!(
        super::mode_status_label(
            crate::app::AccessMode::ReadWrite,
            crate::app::ApprovalMode::Disabled,
            true,
        ),
        "Plan"
    );
}

#[test]
fn model_picker_detail_uses_context_display_override_when_present() {
    let model = crate::model_registry::find_model("kimi-k2.5").expect("registry model");
    let detail = super::model_picker_detail(model);

    assert_eq!(detail, "Azure OpenAI      256K    0.60     0.10    3.00");
}

#[test]
fn model_picker_header_uses_dollar_cost_columns() {
    let header = line_text(super::model_picker_header_line(12));

    assert!(header.contains("provider"));
    assert!(header.contains("ctx"));
    assert!(header.contains("$in"));
    assert!(header.contains("$cache"));
    assert!(header.contains("$out"));
}

#[test]
fn display_model_name_hides_codex_namespace() {
    assert_eq!(
        super::display_model_name("codex/gpt-5.3-codex"),
        "gpt-5.3-codex"
    );
    assert_eq!(super::display_model_name("gpt-5.4"), "gpt-5.4");
}
