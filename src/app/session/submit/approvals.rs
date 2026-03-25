use super::super::Effect;
use crate::app::ReducerContext;

pub(super) fn submit_ask_user(ctx: &mut ReducerContext<'_>) -> Option<Effect> {
    let (request_id, response, _summary) = ctx.advance_ask_user()?;
    Some(Effect::ResolveAskUser {
        request_id,
        response,
    })
}

pub(super) fn submit_shell_approval(ctx: &mut ReducerContext<'_>) -> Option<Effect> {
    let (request_id, decision, _risk) = ctx.submit_shell_approval()?;
    Some(Effect::ResolveShellApproval {
        request_id,
        decision,
    })
}
