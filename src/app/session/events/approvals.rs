use super::super::{Effect, WriteApprovalDecision};
use crate::app::{AppState, ops};

pub(crate) fn apply_write_approval(
    state: &mut AppState,
    decision: WriteApprovalDecision,
) -> Option<String> {
    ops::approvals::resolve_write_approval(state, decision).map(|pending| pending.request_id)
}

pub(crate) fn resolve_write_approval(
    request_id: Option<String>,
    decision: WriteApprovalDecision,
) -> Option<Effect> {
    request_id.map(|request_id| Effect::ResolveWriteApproval {
        request_id,
        decision,
    })
}
