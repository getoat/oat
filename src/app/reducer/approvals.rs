use crate::app::session::events::{apply_write_approval, resolve_write_approval};
use crate::app::{Action, AppState, Effect, WriteApprovalDecision};

pub(super) fn handle(state: &mut AppState, action: Action) -> Option<Effect> {
    match action {
        Action::ApproveWriteOnce => resolve_write_approval(
            apply_write_approval(state, WriteApprovalDecision::AllowOnce),
            WriteApprovalDecision::AllowOnce,
        ),
        Action::ApproveWriteAllSession => resolve_write_approval(
            apply_write_approval(state, WriteApprovalDecision::AllowAllSession),
            WriteApprovalDecision::AllowAllSession,
        ),
        Action::DenyWrite => resolve_write_approval(
            apply_write_approval(state, WriteApprovalDecision::Deny),
            WriteApprovalDecision::Deny,
        ),
        _ => None,
    }
}
