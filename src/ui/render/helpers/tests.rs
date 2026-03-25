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
