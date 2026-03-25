use std::sync::{Arc, Mutex};

use crate::{
    app::{CommandRisk, ShellApprovalDecision, WriteApprovalDecision},
    ask_user::{AskUserRequest, AskUserResponse},
    tools::{
        RUN_SHELL_SCRIPT_TOOL_NAME, RunShellScriptArgs, display_requested_shell_cwd,
        display_shell_command,
    },
};

use super::ResumeOverride;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayProbe {
    pub(crate) expected: String,
    pub(crate) buffered: String,
}

impl ReplayProbe {
    pub(crate) fn new(expected: &str) -> Self {
        Self {
            expected: expected.to_string(),
            buffered: String::new(),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ResumeOverrideController {
    inner: Arc<Mutex<Option<ResumeOverrideState>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResumeOverrideState {
    override_action: ResumeOverride,
    tool_call_suppressed: bool,
}

impl ResumeOverrideController {
    pub(crate) fn new(override_action: ResumeOverride) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Some(ResumeOverrideState {
                override_action,
                tool_call_suppressed: false,
            }))),
        }
    }

    pub(crate) fn consume_write(
        &self,
        tool_name: &str,
        arguments: &str,
    ) -> Option<WriteApprovalDecision> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::WriteApproval {
            tool_name: expected_tool_name,
            arguments: expected_arguments,
            decision: _,
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if expected_tool_name != tool_name || expected_arguments != arguments {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::WriteApproval { decision, .. } = state.override_action else {
            unreachable!("matched write override");
        };
        Some(decision)
    }

    pub(crate) fn consume_shell(
        &self,
        risk: CommandRisk,
        command: &str,
        working_directory: &str,
    ) -> Option<ShellApprovalDecision> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::ShellApproval {
            risk: expected_risk,
            command: expected_command,
            working_directory: expected_working_directory,
            ..
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if *expected_risk != risk
            || expected_command != command
            || expected_working_directory != working_directory
        {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::ShellApproval { decision, .. } = state.override_action else {
            unreachable!("matched shell override");
        };
        Some(decision)
    }

    pub(crate) fn consume_ask_user(&self, request: &AskUserRequest) -> Option<AskUserResponse> {
        let mut state = self.inner.lock().expect("resume override lock");
        let ResumeOverride::AskUser {
            request: expected_request,
            ..
        } = &state.as_ref()?.override_action
        else {
            return None;
        };
        if expected_request != request {
            return None;
        }
        let state = state.take()?;
        let ResumeOverride::AskUser { response, .. } = state.override_action else {
            unreachable!("matched ask user override");
        };
        Some(response)
    }

    pub(crate) fn suppress_matching_tool_call(&self, name: &str, arguments: &str) -> bool {
        let mut state = self.inner.lock().expect("resume override lock");
        let Some(state) = state.as_mut() else {
            return false;
        };
        if state.tool_call_suppressed {
            return false;
        }
        if !resume_override_matches_tool_call(&state.override_action, name, arguments) {
            return false;
        }
        state.tool_call_suppressed = true;
        true
    }
}

pub(crate) fn resume_override_matches_tool_call(
    override_action: &ResumeOverride,
    name: &str,
    arguments: &str,
) -> bool {
    match override_action {
        ResumeOverride::WriteApproval {
            tool_name,
            arguments: expected_arguments,
            ..
        } => tool_name == name && expected_arguments == arguments,
        ResumeOverride::ShellApproval {
            command,
            working_directory,
            ..
        } => {
            if name != RUN_SHELL_SCRIPT_TOOL_NAME {
                return false;
            }
            let Ok(args) = serde_json::from_str::<RunShellScriptArgs>(arguments) else {
                return false;
            };
            display_shell_command(&args.script) == *command
                && display_requested_shell_cwd(args.cwd.as_deref()) == *working_directory
        }
        ResumeOverride::AskUser { .. } => false,
    }
}

pub(crate) fn reconcile_stream_text(
    incoming: &str,
    replay_probe: &mut Option<ReplayProbe>,
) -> String {
    if incoming.is_empty() {
        return String::new();
    }

    let Some(probe) = replay_probe.as_mut() else {
        return incoming.to_string();
    };

    probe.buffered.push_str(incoming);

    if probe.expected.starts_with(&probe.buffered) {
        if probe.expected.len() == probe.buffered.len() {
            *replay_probe = None;
        }
        return String::new();
    }

    if probe.buffered.starts_with(&probe.expected) {
        let suffix = probe.buffered[probe.expected.len()..].to_string();
        *replay_probe = None;
        return suffix;
    }

    let buffered = probe.buffered.clone();
    *replay_probe = None;
    buffered
}
