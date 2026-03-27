use crate::{
    ask_user::AskUserResponse, config::ReasoningSetting, features::planning::PlanningAgentConfig,
};

use super::{AccessMode, SessionHistoryMessage, ShellApprovalDecision, WriteApprovalDecision};

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    PromptModel {
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
    },
    CompactHistory,
    ShowStats,
    OpenModelPicker,
    LoginCodex,
    LogoutCodex,
    RotateSession,
    SetModelSelection {
        model_name: String,
    },
    SetReasoning {
        reasoning: ReasoningSetting,
    },
    SetPlanningAgents {
        planning_agents: Vec<PlanningAgentConfig>,
    },
    SetSafetySelection {
        model_name: String,
        reasoning: ReasoningSetting,
    },
    RunPlanningWorkflow {
        reply_id: u64,
        description: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
    },
    RebuildLlm {
        access_mode: AccessMode,
    },
    ResolveWriteApproval {
        request_id: String,
        decision: WriteApprovalDecision,
    },
    ResolveShellApproval {
        request_id: String,
        decision: ShellApprovalDecision,
    },
    ResolveAskUser {
        request_id: String,
        response: AskUserResponse,
    },
    CopyToClipboard {
        text: String,
    },
    CancelPendingReply,
}
