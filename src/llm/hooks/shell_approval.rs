use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rig::{
    agent::{PromptHook, ToolCallHookAction},
    completion::CompletionModel,
};
use tokio::sync::oneshot;

use crate::{
    app::{AccessMode, ApprovalMode, CommandRisk, ShellApprovalDecision},
    completion_request::CompletionRequestSnapshot,
    tools::{
        RUN_SHELL_SCRIPT_TOOL_NAME, START_BACKGROUND_TERMINAL_TOOL_NAME, ShellCommandRequest,
        StartBackgroundTerminalArgs, display_requested_shell_cwd, display_shell_command,
    },
};

use super::super::{
    CompletionCapture, EventCallback, InteractionResolveResult, ResumeOverride, ResumeRequest,
    StreamEvent,
    resume::ResumeOverrideController,
    safety::{SafetyClassifier, shell_pattern_matches},
};
use super::write_approval::scoped_request_id;

#[derive(Clone)]
pub struct ShellApprovalController {
    inner: Arc<Mutex<ShellApprovalState>>,
}

struct ShellApprovalState {
    default_mode: ApprovalMode,
    low: ShellRiskApprovalBucket,
    medium: ShellRiskApprovalBucket,
    high: ShellRiskApprovalBucket,
    pending: HashMap<String, PendingShellApprovalEntry>,
}

struct PendingShellApprovalEntry {
    risk: CommandRisk,
    sender: oneshot::Sender<ShellApprovalDecision>,
    snapshot: Option<CompletionRequestSnapshot>,
    command: String,
    working_directory: String,
}

#[derive(Clone)]
struct ShellRiskApprovalBucket {
    mode: ApprovalMode,
    patterns: Vec<String>,
}

impl ShellApprovalState {
    fn bucket_mut(&mut self, risk: CommandRisk) -> &mut ShellRiskApprovalBucket {
        match risk {
            CommandRisk::Low => &mut self.low,
            CommandRisk::Medium => &mut self.medium,
            CommandRisk::High => &mut self.high,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ShellApprovalHook {
    pub(crate) reply_id: u64,
    pub(crate) emit: EventCallback,
    pub(crate) access_mode: AccessMode,
    pub(crate) approvals: ShellApprovalController,
    pub(crate) request_id_prefix: String,
    pub(crate) safety: SafetyClassifier,
    pub(crate) capture: Option<CompletionCapture>,
    pub(crate) resume: Option<ResumeOverrideController>,
}

impl Default for ShellApprovalController {
    fn default() -> Self {
        Self::new(ApprovalMode::Manual)
    }
}

impl ShellApprovalController {
    pub fn new(mode: ApprovalMode) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ShellApprovalState {
                default_mode: mode,
                low: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                medium: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                high: ShellRiskApprovalBucket {
                    mode,
                    patterns: Vec::new(),
                },
                pending: HashMap::new(),
            })),
        }
    }

    pub(crate) fn can_resolve(&self, request_id: &str) -> bool {
        self.inner
            .lock()
            .expect("shell approval state lock")
            .pending
            .get(request_id)
            .is_some_and(|pending| !pending.sender.is_closed() || pending.snapshot.is_some())
    }

    async fn request_approval(
        &self,
        reply_id: u64,
        access_mode: AccessMode,
        internal_call_id: &str,
        command: &ShellCommandRequest,
        emit: &EventCallback,
        safety: &SafetyClassifier,
        snapshot: Option<CompletionRequestSnapshot>,
        resume: Option<&ResumeOverrideController>,
    ) -> ToolCallHookAction {
        let classification = safety.classify(access_mode, command).await;
        let display_command = display_shell_command(&command.script);
        let working_directory = display_requested_shell_cwd(command.cwd.as_deref());
        if access_mode == AccessMode::ReadOnly && classification.risk != CommandRisk::Low {
            return ToolCallHookAction::skip(format!(
                "{} risk shell commands require write mode. Switch to write mode before retrying.\nWorking directory: {}\nCommand: {}",
                classification.risk.label(),
                working_directory,
                display_command
            ));
        }

        if let Some(decision) = resume.and_then(|resume| {
            resume.consume_shell(classification.risk, &display_command, &working_directory)
        }) {
            {
                let mut state = self.inner.lock().expect("shell approval state lock");
                let bucket = state.bucket_mut(classification.risk);
                match &decision {
                    ShellApprovalDecision::AllowPattern(pattern) => {
                        if !bucket.patterns.iter().any(|existing| existing == pattern) {
                            bucket.patterns.push(pattern.clone());
                        }
                    }
                    ShellApprovalDecision::AllowAllRisk => {
                        bucket.mode = ApprovalMode::Disabled;
                    }
                    ShellApprovalDecision::AllowOnce | ShellApprovalDecision::Deny(_) => {}
                }
            }
            return match decision {
                ShellApprovalDecision::AllowOnce
                | ShellApprovalDecision::AllowPattern(_)
                | ShellApprovalDecision::AllowAllRisk => ToolCallHookAction::Continue,
                ShellApprovalDecision::Deny(note) => ToolCallHookAction::skip(
                    note.unwrap_or_else(|| "Shell command denied by user.".into()),
                ),
            };
        }

        let rx = {
            let mut state = self.inner.lock().expect("shell approval state lock");
            let bucket = state.bucket_mut(classification.risk);
            if matches!(bucket.mode, ApprovalMode::Disabled)
                || bucket
                    .patterns
                    .iter()
                    .any(|pattern| shell_pattern_matches(pattern, &display_command))
            {
                return ToolCallHookAction::Continue;
            }

            let (tx, rx) = oneshot::channel();
            state.pending.insert(
                internal_call_id.to_string(),
                PendingShellApprovalEntry {
                    risk: classification.risk,
                    sender: tx,
                    snapshot,
                    command: display_command.clone(),
                    working_directory: working_directory.clone(),
                },
            );
            rx
        };

        if !(emit)(
            reply_id,
            StreamEvent::ShellApprovalRequested {
                request_id: internal_call_id.to_string(),
                risk: classification.risk,
                risk_explanation: classification.risk_explanation.clone(),
                command: display_command.clone(),
                working_directory: working_directory.clone(),
                reason: classification.reason.clone(),
            },
        ) {
            let mut state = self.inner.lock().expect("shell approval state lock");
            state.pending.remove(internal_call_id);
            return ToolCallHookAction::skip(
                "Shell command cancelled because approval UI is unavailable.",
            );
        }

        match rx.await {
            Ok(ShellApprovalDecision::AllowOnce) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::AllowPattern(_)) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::AllowAllRisk) => ToolCallHookAction::Continue,
            Ok(ShellApprovalDecision::Deny(note)) => ToolCallHookAction::skip(
                note.unwrap_or_else(|| "Shell command denied by user.".into()),
            ),
            Err(_) => ToolCallHookAction::skip("Shell command cancelled before approval."),
        }
    }

    pub(crate) fn resolve(
        &self,
        request_id: &str,
        decision: ShellApprovalDecision,
    ) -> InteractionResolveResult {
        let pending = {
            let mut state = self.inner.lock().expect("shell approval state lock");
            let pending = state.pending.remove(request_id);
            if let Some(entry) = pending.as_ref() {
                let bucket = state.bucket_mut(entry.risk);
                match &decision {
                    ShellApprovalDecision::AllowPattern(pattern) => {
                        if !bucket.patterns.iter().any(|existing| existing == pattern) {
                            bucket.patterns.push(pattern.clone());
                        }
                    }
                    ShellApprovalDecision::AllowAllRisk => {
                        bucket.mode = ApprovalMode::Disabled;
                    }
                    ShellApprovalDecision::AllowOnce | ShellApprovalDecision::Deny(_) => {}
                }
            }
            pending
        };

        let Some(entry) = pending else {
            return InteractionResolveResult::Missing;
        };

        if entry.sender.send(decision.clone()).is_ok() {
            InteractionResolveResult::Resolved
        } else if let Some(snapshot) = entry.snapshot {
            InteractionResolveResult::Resume(ResumeRequest {
                snapshot,
                override_action: ResumeOverride::ShellApproval {
                    risk: entry.risk,
                    command: entry.command,
                    working_directory: entry.working_directory,
                    decision,
                },
            })
        } else {
            InteractionResolveResult::Missing
        }
    }

    pub(crate) fn reset_session(&self) {
        let mut state = self.inner.lock().expect("shell approval state lock");
        state.low = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        state.medium = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        state.high = ShellRiskApprovalBucket {
            mode: state.default_mode,
            patterns: Vec::new(),
        };
        for (_, entry) in state.pending.drain() {
            let _ = entry.sender.send(ShellApprovalDecision::Deny(None));
        }
    }

    pub(crate) fn cancel_pending(&self) {
        let mut state = self.inner.lock().expect("shell approval state lock");
        state.pending.clear();
    }
}

impl<M> PromptHook<M> for ShellApprovalHook
where
    M: CompletionModel,
{
    async fn on_tool_call(
        &self,
        tool_name: &str,
        _tool_call_id: Option<String>,
        internal_call_id: &str,
        args: &str,
    ) -> ToolCallHookAction {
        let command = match tool_name {
            RUN_SHELL_SCRIPT_TOOL_NAME => {
                match serde_json::from_str::<crate::tools::RunShellScriptArgs>(args) {
                    Ok(args) => args.command,
                    Err(error) => {
                        return ToolCallHookAction::skip(format!(
                            "RunShellScript request was invalid JSON: {error}"
                        ));
                    }
                }
            }
            START_BACKGROUND_TERMINAL_TOOL_NAME => {
                match serde_json::from_str::<StartBackgroundTerminalArgs>(args) {
                    Ok(args) => args.command,
                    Err(error) => {
                        return ToolCallHookAction::skip(format!(
                            "StartBackgroundTerminal request was invalid JSON: {error}"
                        ));
                    }
                }
            }
            _ => return ToolCallHookAction::Continue,
        };

        self.approvals
            .request_approval(
                self.reply_id,
                self.access_mode,
                &scoped_request_id(&self.request_id_prefix, internal_call_id),
                &command,
                &self.emit,
                &self.safety,
                self.capture.as_ref().and_then(CompletionCapture::snapshot),
                self.resume.as_ref(),
            )
            .await
    }
}
