use super::super::{Effect, WriteApprovalDecision};
use crate::app::ReducerContext;

pub(in crate::app::session) fn apply_write_approval(
    ctx: &mut ReducerContext<'_>,
    decision: WriteApprovalDecision,
) -> Option<String> {
    ctx.resolve_write_approval(decision)
        .map(|pending| pending.request_id)
}

pub(in crate::app::session) fn resolve_write_approval(
    request_id: Option<String>,
    decision: WriteApprovalDecision,
) -> Option<Effect> {
    request_id.map(|request_id| Effect::ResolveWriteApproval {
        request_id,
        decision,
    })
}
