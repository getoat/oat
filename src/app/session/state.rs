use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
};

use crate::{
    config::{HistoryMode, ReasoningSetting},
    features::planning::{PlanningAgentConfig, PlanningFeatureState},
    stats::StatsTotals,
    todo::TodoSnapshot,
};

use super::{
    AccessMode, ApprovalMode, ChatMessage, MainRequestSeed, MessageStyle, PendingAskUser,
    PendingReply, PendingReplyKind, PendingShellApproval, PendingSideReply, PendingWriteApproval,
    SessionHistoryMessage, Speaker, TranscriptEntry, startup_banner_message,
};

#[derive(Debug)]
pub struct SessionState {
    pub workspace_root: PathBuf,
    pub full_system_access: bool,
    pub initial_mode: AccessMode,
    pub initial_approval_mode: ApprovalMode,
    pub mode: AccessMode,
    pub approval_mode: ApprovalMode,
    pub pending_write_approvals: VecDeque<PendingWriteApproval>,
    pub pending_shell_approvals: VecDeque<PendingShellApproval>,
    pub should_quit: bool,
    pub entries: Vec<TranscriptEntry>,
    pub transcript_revision: u64,
    pub transcript_structure_revision: u64,
    pub transcript_tail_revision: u64,
    pub session_history: Vec<SessionHistoryMessage>,
    pub estimated_session_history_tokens: u64,
    pub pending_reply: Option<PendingReply>,
    pub next_reply_id: u64,
    pub session_title: Option<String>,
    pub pending_codex_login: bool,
    pub pending_session_title_reply_id: Option<u64>,
    pub tick_count: usize,
    pub show_thinking: bool,
    pub show_tool_output: bool,
    pub model_name: String,
    pub last_history_model_name: Option<String>,
    pub history_mode: HistoryMode,
    pub history_retained_steps: usize,
    pub reasoning: ReasoningSetting,
    pub safety_model_name: String,
    pub safety_reasoning: ReasoningSetting,
    pub memory_model_name: String,
    pub memory_reasoning: ReasoningSetting,
    pub planning_agents: Vec<PlanningAgentConfig>,
    pub session_stats: StatsTotals,
    pub active_background_terminal_count: usize,
    pub planning: PlanningFeatureState,
    pub pending_ask_user: Option<PendingAskUser>,
    pub current_todo: Option<TodoSnapshot>,
    pub queued_messages: VecDeque<String>,
    pub active_main_request_seed: Option<MainRequestSeed>,
    pub pending_side_replies: HashMap<u64, PendingSideReply>,
    pub next_side_channel_label_id: u64,
}

impl SessionState {
    #[cfg(test)]
    pub fn new(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning: impl Into<ReasoningSetting>,
    ) -> Self {
        Self::with_startup(
            show_thinking,
            show_tool_output,
            model_name,
            reasoning.into(),
            Vec::new(),
            false,
            AccessMode::ReadOnly,
            ApprovalMode::Manual,
        )
    }

    pub fn with_startup(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning: impl Into<ReasoningSetting>,
        planning_agents: Vec<PlanningAgentConfig>,
        full_system_access: bool,
        initial_mode: AccessMode,
        initial_approval_mode: ApprovalMode,
    ) -> Self {
        let model_name = model_name.into();
        let reasoning = reasoning.into();
        let safety_model_name = model_name.clone();
        let memory_model_name = model_name.clone();
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            full_system_access,
            initial_mode,
            initial_approval_mode,
            mode: initial_mode,
            approval_mode: initial_approval_mode,
            pending_write_approvals: VecDeque::new(),
            pending_shell_approvals: VecDeque::new(),
            should_quit: false,
            entries: vec![TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text: startup_banner_message(&model_name, initial_mode),
                style: MessageStyle::Plain,
                tag: None,
            })],
            transcript_revision: 0,
            transcript_structure_revision: 0,
            transcript_tail_revision: 0,
            session_history: Vec::new(),
            estimated_session_history_tokens: 0,
            pending_reply: None,
            next_reply_id: 1,
            session_title: None,
            pending_codex_login: false,
            pending_session_title_reply_id: None,
            tick_count: 0,
            show_thinking,
            show_tool_output,
            safety_model_name,
            model_name,
            last_history_model_name: None,
            history_mode: HistoryMode::Full,
            history_retained_steps: 1,
            reasoning,
            safety_reasoning: reasoning,
            memory_model_name,
            memory_reasoning: reasoning,
            planning_agents,
            session_stats: StatsTotals::default(),
            active_background_terminal_count: 0,
            planning: PlanningFeatureState::default(),
            pending_ask_user: None,
            current_todo: None,
            queued_messages: VecDeque::new(),
            active_main_request_seed: None,
            pending_side_replies: HashMap::new(),
            next_side_channel_label_id: 1,
        }
    }

    pub fn replace_session_history(&mut self, history: Vec<SessionHistoryMessage>) {
        self.estimated_session_history_tokens =
            history.iter().map(|message| message.estimated_tokens).sum();
        self.session_history = history;
    }

    pub fn enqueue_shell_approval(
        &mut self,
        source_label: Option<String>,
        request_id: String,
        risk: crate::app::session::CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        self.pending_shell_approvals
            .push_back(PendingShellApproval::new(
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
                source_label,
            ));
    }

    pub fn next_reply_id(&mut self) -> u64 {
        let id = self.next_reply_id;
        self.next_reply_id = self.next_reply_id.wrapping_add(1);
        id
    }

    pub fn ensure_pending_reply(&mut self, kind: PendingReplyKind) -> u64 {
        if let Some(pending) = self.pending_reply.as_ref() {
            return pending.id;
        }

        let reply_id = self.next_reply_id();
        self.pending_reply = Some(PendingReply::new(reply_id, kind));
        reply_id
    }
}
