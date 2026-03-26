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

    assert!(detail.contains("ctx 256K"));
}
