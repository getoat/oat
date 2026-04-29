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
        session_title_prompt: Option<String>,
    },
    PromptSideChannel {
        reply_id: u64,
        prompt: String,
        history: Vec<SessionHistoryMessage>,
        history_model_name: Option<String>,
    },
    CompactHistory,
    SearchMemories {
        query: String,
        include_candidates: bool,
    },
    ShowMemory {
        id: String,
    },
    ListMemoryCandidates,
    ShowMemoryStats,
    PromoteMemory {
        id: String,
    },
    ArchiveMemory {
        id: String,
    },
    ReplaceMemory {
        id: String,
        text: String,
    },
    ClearMemories,
    RebuildMemoryIndexes,
    ShowStats,
    OpenSessionPicker,
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
    SetMemorySelection {
        model_name: String,
        reasoning: ReasoningSetting,
    },
    SetCriticSelection {
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
    ResumeSession {
        session_id: String,
    },
    ListBackgroundTerminals,
    InspectBackgroundTerminal {
        id: String,
    },
    KillBackgroundTerminal {
        id: String,
    },
    CancelPendingReply,
}
