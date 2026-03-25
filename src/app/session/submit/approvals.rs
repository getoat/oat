use super::super::Effect;
use crate::app::{AppState, ops};

pub(super) fn submit_ask_user(state: &mut AppState) -> Option<Effect> {
    let (request_id, response, _summary) = ops::ask_user::advance_ask_user(state)?;
    Some(Effect::ResolveAskUser {
        request_id,
        response,
    })
}

pub(super) fn submit_shell_approval(state: &mut AppState) -> Option<Effect> {
    let (request_id, decision, _risk) = ops::approvals::submit_shell_approval(state)?;
    Some(Effect::ResolveShellApproval {
        request_id,
        decision,
    })
}
