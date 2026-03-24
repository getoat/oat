use ratatui::{layout::Rect, style::Color, text::Line};
use ratatui_textarea::{CursorMove, Input, TextArea};
use rig::completion::Message as RigMessage;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use crate::{
    ask_user::{
        AskUserAnswer, AskUserAnsweredQuestion, AskUserQuestion, AskUserRequest, AskUserResponse,
        AskUserSelectedAnswer, SOMETHING_ELSE_ID, SOMETHING_ELSE_LABEL,
    },
    completion_request::estimated_history_context_tokens,
    composer::{ComposerLayout, slice_line},
    config::ReasoningEffort,
    model_registry,
    planning::{
        PlanningAgentConfig, contains_proposed_plan, default_planning_reasoning,
        strip_planning_ready_tags, strip_proposed_plan_tags,
    },
    stats::StatsTotals,
    tools::{MutationPreview, mutation_preview, write_approval_summary},
};

const COMMANDS: [SlashCommand; 7] = [
    SlashCommand::NewSession,
    SlashCommand::Compact,
    SlashCommand::Stats,
    SlashCommand::Model,
    SlashCommand::Effort,
    SlashCommand::Plan,
    SlashCommand::Quit,
];
const DEFAULT_COMMAND_HISTORY_LIMIT: usize = 20;
const DEFAULT_COMPOSER_WRAP_WIDTH: usize = 80;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Compact,
    Stats,
    Model,
    Effort,
    Plan,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Compact => "/compact",
            Self::Stats => "/stats",
            Self::Model => "/model",
            Self::Effort => "/effort",
            Self::Plan => "/plan",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Compact => &[],
            Self::Stats => &["/status"],
            Self::Model => &["/models"],
            Self::Effort => &["/reasoning", "/thinking"],
            Self::Plan => &[],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Compact => "Compact the internal model history",
            Self::Stats => "Show session and historical usage stats",
            Self::Model => "Select the model and reasoning effort",
            Self::Effort => "Set reasoning effort for the current model",
            Self::Plan => "Start an interactive planning session",
            Self::Quit => "Exit the app",
        }
    }

    pub fn usage(self) -> Option<&'static str> {
        match self {
            Self::Model => Some("/model"),
            Self::Compact => Some("/compact"),
            Self::Effort => Some("/effort <minimal|low|medium|high|xhigh>"),
            Self::Plan => Some("/plan"),
            Self::NewSession | Self::Stats | Self::Quit => None,
        }
    }

    pub fn all_names(self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.canonical_name()).chain(self.aliases().iter().copied())
    }

    pub fn display_name(self) -> String {
        let aliases = self.aliases();
        if aliases.is_empty() {
            self.canonical_name().to_string()
        } else {
            format!("{} ({})", self.canonical_name(), aliases.join(", "))
        }
    }

    fn matches_prefix(self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.all_names()
            .any(|name| name.to_ascii_lowercase().starts_with(&query))
    }

    pub fn matches_exact(self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.all_names()
            .any(|name| name.eq_ignore_ascii_case(&query))
    }

    pub fn filtered(query: &str) -> Vec<Self> {
        COMMANDS
            .into_iter()
            .filter(|command| command.matches_prefix(query))
            .collect()
    }

    pub fn parse(query: &str) -> Option<Self> {
        COMMANDS
            .into_iter()
            .find(|command| command.matches_exact(query))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelPickerTab {
    NormalAgent,
    PlanningAgents,
    SafetyModel,
}

impl ModelPickerTab {
    pub fn title(self) -> &'static str {
        match self {
            Self::NormalAgent => "Normal agent",
            Self::PlanningAgents => "Planning agents",
            Self::SafetyModel => "Safety model",
        }
    }

    fn toggle(&mut self, direction: isize) {
        let tabs = [Self::NormalAgent, Self::PlanningAgents, Self::SafetyModel];
        let current = tabs.iter().position(|tab| *tab == *self).unwrap_or(0);
        let next = (current as isize + direction).rem_euclid(tabs.len() as isize) as usize;
        *self = tabs[next];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasoningPickerTarget {
    NormalAgent,
    PlanningAgent,
    SafetyModel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionPicker {
    Model {
        active_tab: ModelPickerTab,
        normal_selected_index: usize,
        planning_selected_index: usize,
        safety_selected_index: usize,
    },
    Reasoning {
        target: ReasoningPickerTarget,
        model_name: String,
        options: Vec<ReasoningEffort>,
        selected_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PickerSelection {
    Model(String),
    Reasoning(ReasoningEffort),
    PlanningAgent(PlanningAgentConfig),
    SafetySelection {
        model_name: String,
        reasoning_effort: ReasoningEffort,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubagentStatusKind {
    Subagent,
    Planning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

impl AccessMode {
    pub fn toggle(&mut self) {
        *self = match self {
            Self::ReadOnly => Self::ReadWrite,
            Self::ReadWrite => Self::ReadOnly,
        };
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::ReadWrite => "Write",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteApprovalDecision {
    AllowOnce,
    AllowAllSession,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalMode {
    Manual,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWriteApproval {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub summary: String,
    pub target: Option<String>,
    pub source_label: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CommandRisk {
    Low,
    Medium,
    High,
}

impl CommandRisk {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellApprovalDecision {
    AllowOnce,
    AllowPattern(String),
    AllowAllRisk,
    Deny(Option<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellApprovalEditMode {
    Pattern,
    Deny,
}

#[derive(Debug)]
pub struct PendingShellApproval {
    pub request_id: String,
    pub risk: CommandRisk,
    pub risk_explanation: String,
    pub command: String,
    pub working_directory: String,
    pub reason: String,
    pub source_label: Option<String>,
    pub selected_index: usize,
    pub edit_mode: Option<ShellApprovalEditMode>,
    pub pattern_input: TextArea<'static>,
    pub deny_input: TextArea<'static>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Speaker {
    User,
    Agent,
}

impl Speaker {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "you",
            Self::Agent => "oat",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub speaker: Speaker,
    pub text: String,
    pub style: MessageStyle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageStyle {
    Plain,
    Commentary,
    Thinking,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub parameter: String,
    pub preview: Option<MutationPreview>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResultEntry {
    pub name: String,
    pub output: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubagentDisplayState {
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentStatusEntry {
    pub id: String,
    pub kind: SubagentStatusKind,
    pub display_label: String,
    pub state: SubagentDisplayState,
    pub status_text: String,
    pub latest_tool_name: Option<String>,
}

#[derive(Clone, Debug)]
pub enum TranscriptEntry {
    Message(ChatMessage),
    ToolCall(ToolCall),
    ToolResult(ToolResultEntry),
    SubagentStatus(SubagentStatusEntry),
}

#[derive(Debug)]
pub(super) struct PendingReply {
    pub(super) id: u64,
    pub(super) kind: PendingReplyKind,
    pub(super) reasoning_entry_index: Option<usize>,
    pub(super) text_entry_index: Option<usize>,
    pub(super) staged_reasoning_text: String,
    pub(super) staged_plain_text: String,
    pub(super) plain_text: String,
    pub(super) reasoning_text: String,
    pub(super) commentary_messages: Vec<String>,
    pub(super) has_visible_content: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PendingReplyReplaySeed {
    pub plain_text: String,
    pub reasoning_text: String,
    pub commentary_messages: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingReplyKind {
    Normal,
    Planning,
    Compacting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanningSessionStage {
    Drafting,
    Conversation,
    RunningFanout,
    Finalizing,
}

impl PendingReply {
    pub(super) fn new(id: u64, kind: PendingReplyKind) -> Self {
        Self {
            id,
            kind,
            reasoning_entry_index: None,
            text_entry_index: None,
            staged_reasoning_text: String::new(),
            staged_plain_text: String::new(),
            plain_text: String::new(),
            reasoning_text: String::new(),
            commentary_messages: Vec::new(),
            has_visible_content: false,
        }
    }

    fn reset_active_stream_segment(&mut self) {
        self.reasoning_entry_index = None;
        self.text_entry_index = None;
        self.staged_reasoning_text.clear();
        self.staged_plain_text.clear();
    }
}

fn pending_stream_text_is_visible(style: MessageStyle, text: &str) -> bool {
    match style {
        MessageStyle::Plain => {
            let visible_text = strip_planning_ready_tags(&strip_proposed_plan_tags(text));
            !visible_text.trim().is_empty()
        }
        MessageStyle::Commentary | MessageStyle::Thinking => !text.trim().is_empty(),
        MessageStyle::Error => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingPlanReviewMode {
    Selection,
    Feedback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingPlanReview {
    pub(super) mode: PendingPlanReviewMode,
    pub(super) selected_index: usize,
}

#[derive(Debug)]
pub struct PendingAskUser {
    pub(crate) request_id: String,
    pub(crate) title: String,
    pub(crate) active_tab: usize,
    pub(crate) detail_editing: bool,
    pub(crate) questions: Vec<PendingAskUserQuestion>,
}

#[derive(Debug)]
pub(crate) struct PendingAskUserQuestion {
    pub(crate) id: String,
    pub(crate) prompt: String,
    pub(crate) answers: Vec<PendingAskUserAnswer>,
    pub(crate) selected_index: usize,
    pub(crate) detail: TextArea<'static>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingAskUserAnswer {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) is_recommended: bool,
    pub(crate) is_something_else: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HistorySelectionPoint {
    row: usize,
    column: usize,
}

#[derive(Debug, Default)]
pub(super) struct HistoryViewState {
    pub(super) scroll_top: Option<usize>,
    viewport_rows: usize,
    total_lines: usize,
    snapshot_area: Rect,
    snapshot_lines: Vec<String>,
    selection_anchor: Option<HistorySelectionPoint>,
    selection_focus: Option<HistorySelectionPoint>,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryRenderCache {
    pub(crate) width: usize,
    pub(crate) accent: Color,
    pub(crate) transcript_revision: u64,
    pub(crate) lines: Vec<Line<'static>>,
}

#[derive(Debug)]
struct CommandRecallState {
    entries: Vec<String>,
    browsing_index: Option<usize>,
    draft: Option<String>,
    limit: usize,
    dirty: bool,
}

#[derive(Debug)]
pub struct App {
    pub(super) workspace_root: PathBuf,
    pub(super) initial_mode: AccessMode,
    pub(super) initial_approval_mode: ApprovalMode,
    pub(super) mode: AccessMode,
    pub(super) approval_mode: ApprovalMode,
    pub(super) pending_write_approvals: VecDeque<PendingWriteApproval>,
    pub(super) pending_shell_approvals: VecDeque<PendingShellApproval>,
    pub(super) should_quit: bool,
    pub(super) composer: TextArea<'static>,
    pub(super) entries: Vec<TranscriptEntry>,
    pub(super) transcript_revision: u64,
    pub(super) session_history: Vec<RigMessage>,
    pub(super) estimated_session_history_tokens: u64,
    pub(super) pending_reply: Option<PendingReply>,
    pub(super) next_reply_id: u64,
    pub(super) tick_count: usize,
    pub(super) show_thinking: bool,
    pub(super) show_tool_output: bool,
    pub(super) model_name: String,
    pub(super) last_history_model_name: Option<String>,
    pub(super) reasoning_effort: ReasoningEffort,
    pub(super) safety_model_name: String,
    pub(super) safety_reasoning_effort: ReasoningEffort,
    pub(super) planning_agents: Vec<PlanningAgentConfig>,
    pub(super) session_stats: StatsTotals,
    pub(super) selected_command: SlashCommand,
    pub(super) picker: Option<SelectionPicker>,
    pub(super) planning_session: Option<PlanningSessionStage>,
    pub(super) pending_plan_review: Option<PendingPlanReview>,
    pub(super) pending_ask_user: Option<PendingAskUser>,
    pub(super) history_render_cache: Option<HistoryRenderCache>,
    pub(super) history: HistoryViewState,
    command_history: CommandRecallState,
    composer_wrap_width: usize,
    composer_visual_column: Option<usize>,
    composer_layout_cache: Option<ComposerLayout>,
}

impl App {
    pub fn new(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning_effort: ReasoningEffort,
    ) -> Self {
        Self::with_startup(
            show_thinking,
            show_tool_output,
            model_name,
            reasoning_effort,
            Vec::new(),
            AccessMode::ReadOnly,
            ApprovalMode::Manual,
        )
    }

    pub fn with_startup(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning_effort: ReasoningEffort,
        planning_agents: Vec<PlanningAgentConfig>,
        initial_mode: AccessMode,
        initial_approval_mode: ApprovalMode,
    ) -> Self {
        let model_name = model_name.into();
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            initial_mode,
            initial_approval_mode,
            mode: initial_mode,
            approval_mode: initial_approval_mode,
            pending_write_approvals: VecDeque::new(),
            pending_shell_approvals: VecDeque::new(),
            should_quit: false,
            composer: new_composer(),
            entries: vec![TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text: welcome_message(&model_name, initial_mode),
                style: MessageStyle::Plain,
            })],
            transcript_revision: 0,
            session_history: Vec::new(),
            estimated_session_history_tokens: 0,
            pending_reply: None,
            next_reply_id: 1,
            tick_count: 0,
            show_thinking,
            show_tool_output,
            safety_model_name: model_name.clone(),
            model_name,
            last_history_model_name: None,
            reasoning_effort,
            safety_reasoning_effort: reasoning_effort,
            planning_agents,
            session_stats: StatsTotals::default(),
            selected_command: SlashCommand::NewSession,
            picker: None,
            planning_session: None,
            pending_plan_review: None,
            pending_ask_user: None,
            history_render_cache: None,
            history: HistoryViewState::default(),
            command_history: CommandRecallState::default(),
            composer_wrap_width: DEFAULT_COMPOSER_WRAP_WIDTH,
            composer_visual_column: None,
            composer_layout_cache: None,
        }
    }

    pub fn mode(&self) -> AccessMode {
        self.mode
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn approval_mode(&self) -> ApprovalMode {
        self.approval_mode
    }

    pub fn safety_model_name(&self) -> &str {
        &self.safety_model_name
    }

    pub fn safety_reasoning_effort(&self) -> ReasoningEffort {
        self.safety_reasoning_effort
    }

    pub fn pending_write_approval(&self) -> Option<&PendingWriteApproval> {
        self.pending_write_approvals.front()
    }

    pub fn main_pending_write_approval_request_id(&self) -> Option<&str> {
        self.pending_write_approvals
            .iter()
            .find(|pending| pending.source_label.is_none())
            .map(|pending| pending.request_id.as_str())
    }

    pub fn has_pending_write_approval(&self) -> bool {
        !self.pending_write_approvals.is_empty()
    }

    pub fn pending_shell_approval(&self) -> Option<&PendingShellApproval> {
        self.pending_shell_approvals.front()
    }

    pub fn main_pending_shell_approval_request_id(&self) -> Option<&str> {
        self.pending_shell_approvals
            .iter()
            .find(|pending| pending.source_label.is_none())
            .map(|pending| pending.request_id.as_str())
    }

    pub fn has_pending_shell_approval(&self) -> bool {
        !self.pending_shell_approvals.is_empty()
    }

    pub fn shell_approval_editing(&self) -> bool {
        self.pending_shell_approval()
            .is_some_and(PendingShellApproval::is_editing)
    }

    pub fn shell_approval_editor_can_move_up(&self) -> bool {
        self.pending_shell_approval()
            .is_some_and(PendingShellApproval::editor_can_move_up)
    }

    pub fn shell_approval_editor_can_move_down(&self) -> bool {
        self.pending_shell_approval()
            .is_some_and(PendingShellApproval::editor_can_move_down)
    }

    pub fn pending_ask_user(&self) -> Option<&PendingAskUser> {
        self.pending_ask_user.as_ref()
    }

    pub fn has_pending_ask_user(&self) -> bool {
        self.pending_ask_user.is_some()
    }

    pub fn ask_user_review_active(&self) -> bool {
        self.pending_ask_user
            .as_ref()
            .is_some_and(|pending| pending.active_tab == pending.questions.len())
    }

    pub fn ask_user_detail_editing(&self) -> bool {
        self.pending_ask_user
            .as_ref()
            .is_some_and(|pending| pending.detail_editing)
    }

    pub fn plan_review_selection_active(&self) -> bool {
        self.pending_plan_review
            .as_ref()
            .is_some_and(|review| review.mode == PendingPlanReviewMode::Selection)
    }

    pub fn plan_review_feedback_active(&self) -> bool {
        self.pending_plan_review
            .as_ref()
            .is_some_and(|review| review.mode == PendingPlanReviewMode::Feedback)
    }

    pub fn planning_session_stage(&self) -> Option<PlanningSessionStage> {
        self.planning_session
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn composer(&self) -> &TextArea<'static> {
        &self.composer
    }

    pub fn composer_mut(&mut self) -> &mut TextArea<'static> {
        self.invalidate_composer_layout();
        self.composer_visual_column = None;
        &mut self.composer
    }

    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    pub fn latest_proposed_plan_message(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|entry| match entry {
            TranscriptEntry::Message(message) if contains_proposed_plan(&message.text) => {
                Some(message.text.as_str())
            }
            _ => None,
        })
    }

    pub fn session_history(&self) -> &[RigMessage] {
        &self.session_history
    }

    pub(crate) fn shows_startup_banner(&self) -> bool {
        self.session_history.is_empty()
            && self.entries.len() == 1
            && matches!(
                self.entries.first(),
                Some(TranscriptEntry::Message(ChatMessage {
                    speaker: Speaker::Agent,
                    style: MessageStyle::Plain,
                    text,
                })) if text == &welcome_message(&self.model_name, self.initial_mode)
            )
    }

    pub fn has_pending_reply(&self) -> bool {
        self.pending_reply.is_some()
    }

    pub fn has_visible_pending_content(&self) -> bool {
        self.pending_reply
            .as_ref()
            .is_some_and(|pending| pending.has_visible_content)
    }

    pub fn should_show_history_busy_indicator(&self) -> bool {
        self.pending_reply.as_ref().is_some_and(|pending| {
            pending.text_entry_index.is_none() && pending.reasoning_entry_index.is_none()
        })
    }

    pub fn history_pending_status_label(&self) -> &'static str {
        if self.has_pending_write_approval() || self.has_pending_shell_approval() {
            "Waiting"
        } else if self
            .pending_reply
            .as_ref()
            .is_some_and(|pending| pending.kind == PendingReplyKind::Compacting)
        {
            "Compacting context..."
        } else {
            "thinking"
        }
    }

    pub fn composer_height(&mut self) -> u16 {
        (self.composer_layout().height() as u16).saturating_add(2)
    }

    pub fn overlay_height(&self) -> u16 {
        if let Some(picker) = self.selection_picker() {
            return picker_height(picker);
        }

        self.command_palette_height()
    }

    pub fn command_palette_height(&self) -> u16 {
        if !self.command_palette_visible() {
            return 0;
        }

        let line_count = self.filtered_commands().len().clamp(1, 4) as u16;
        line_count + 2
    }

    pub fn composer_has_content(&self) -> bool {
        self.composer.lines().iter().any(|line| !line.is_empty())
    }

    pub fn show_thinking(&self) -> bool {
        self.show_thinking
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    pub fn reasoning_effort(&self) -> ReasoningEffort {
        self.reasoning_effort
    }

    pub fn last_history_model_name(&self) -> Option<&str> {
        self.last_history_model_name.as_deref()
    }

    pub fn planning_agents(&self) -> &[PlanningAgentConfig] {
        &self.planning_agents
    }

    pub fn planning_draft_mode(&self) -> bool {
        self.planning_session == Some(PlanningSessionStage::Drafting)
    }

    pub fn plan_active(&self) -> bool {
        self.planning_session.is_some()
            || self.pending_plan_review.is_some()
            || self
                .pending_reply
                .as_ref()
                .is_some_and(|pending| pending.kind == PendingReplyKind::Planning)
    }

    pub fn current_model_info(&self) -> Option<&'static model_registry::ModelInfo> {
        model_registry::find_model(&self.model_name)
    }

    pub fn show_tool_output(&self) -> bool {
        self.show_tool_output
    }

    pub fn session_stats(&self) -> StatsTotals {
        self.session_stats
    }

    pub fn estimated_next_request_context_tokens(&self) -> u64 {
        self.estimated_session_history_tokens
    }

    pub fn next_request_context_percent(&self) -> u64 {
        let Some(model) = self.current_model_info() else {
            return 0;
        };
        if model.context_length == 0 {
            return 0;
        }

        self.estimated_next_request_context_tokens() * 100 / model.context_length as u64
    }

    pub fn tick_count(&self) -> usize {
        self.tick_count
    }

    pub fn command_palette_visible(&self) -> bool {
        self.selection_picker().is_none() && self.command_query().is_some()
    }

    pub fn selection_picker(&self) -> Option<&SelectionPicker> {
        self.picker.as_ref()
    }

    pub fn selection_picker_visible(&self) -> bool {
        self.selection_picker().is_some()
    }

    pub fn history_is_pinned(&self) -> bool {
        self.history.is_pinned()
    }

    pub fn history_status_label(&self) -> &'static str {
        if self.history_is_pinned() {
            "History pinned  End latest"
        } else {
            "History live  PgUp/PgDn scroll"
        }
    }

    pub fn command_query(&self) -> Option<&str> {
        let [line] = self.composer.lines() else {
            return None;
        };

        line.starts_with('/').then_some(line.as_str())
    }

    pub fn command_name(&self) -> Option<&str> {
        self.command_query()
            .map(|query| split_command_query(query).0)
    }

    pub fn command_arguments(&self) -> Option<&str> {
        self.command_query()
            .map(|query| split_command_query(query).1)
    }

    pub fn filtered_commands(&self) -> Vec<SlashCommand> {
        self.command_name()
            .map(SlashCommand::filtered)
            .unwrap_or_default()
    }

    pub fn selected_command(&self) -> Option<SlashCommand> {
        let commands = self.filtered_commands();
        commands
            .contains(&self.selected_command)
            .then_some(self.selected_command)
            .or_else(|| commands.first().copied())
    }

    pub fn supported_reasoning_levels(&self) -> Vec<ReasoningEffort> {
        model_registry::reasoning_levels_for_model(&self.model_name)
            .map(|levels| levels.to_vec())
            .unwrap_or_else(|| {
                vec![
                    ReasoningEffort::Minimal,
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                    ReasoningEffort::XHigh,
                ]
            })
    }

    pub(crate) fn active_reply_id(&self) -> Option<u64> {
        self.pending_reply.as_ref().map(|pending| pending.id)
    }

    pub(crate) fn active_reply_kind(&self) -> Option<PendingReplyKind> {
        self.pending_reply.as_ref().map(|pending| pending.kind)
    }

    pub(crate) fn pending_reply_replay_seed(&self) -> Option<PendingReplyReplaySeed> {
        self.pending_reply
            .as_ref()
            .map(|pending| PendingReplyReplaySeed {
                plain_text: pending.plain_text.clone(),
                reasoning_text: pending.reasoning_text.clone(),
                commentary_messages: pending.commentary_messages.clone(),
            })
    }

    pub(crate) fn ensure_pending_reply(&mut self, kind: PendingReplyKind) -> u64 {
        if let Some(pending) = self.pending_reply.as_ref() {
            return pending.id;
        }

        let reply_id = self.next_reply_id();
        self.pending_reply = Some(PendingReply::new(reply_id, kind));
        reply_id
    }

    pub(super) fn next_reply_id(&mut self) -> u64 {
        let id = self.next_reply_id;
        self.next_reply_id = self.next_reply_id.wrapping_add(1);
        id
    }

    pub(super) fn set_composer_text(&mut self, text: &str) {
        self.set_composer_text_internal(text, true);
    }

    fn set_composer_text_internal(&mut self, text: &str, reset_command_history: bool) {
        let mut composer = new_composer_with_text(text);
        composer.move_cursor(CursorMove::End);
        self.composer = composer;
        self.invalidate_composer_layout();
        self.composer_visual_column = None;
        if reset_command_history {
            self.command_history.reset_navigation();
        }
        self.sync_command_selection();
    }

    pub(crate) fn set_composer_wrap_width(&mut self, width: usize) {
        let width = width.max(1);
        if self.composer_wrap_width != width {
            self.composer_wrap_width = width;
            self.invalidate_composer_layout();
            self.composer_visual_column = None;
        }
    }

    pub(crate) fn composer_layout(&mut self) -> &ComposerLayout {
        if self.composer_layout_cache.is_none() {
            self.composer_layout_cache = Some(ComposerLayout::new(
                self.composer.lines(),
                self.composer_wrap_width,
            ));
        }

        self.composer_layout_cache
            .as_ref()
            .expect("composer layout cache should be populated")
    }

    pub(crate) fn composer_wrap_width(&self) -> usize {
        self.composer_wrap_width
    }

    fn invalidate_composer_layout(&mut self) {
        self.composer_layout_cache = None;
    }

    pub(crate) fn restore_command_history(&mut self, entries: Vec<String>, limit: usize) {
        self.command_history.restore(entries, limit);
    }

    pub(crate) fn take_command_history_to_persist(&mut self) -> Option<Vec<String>> {
        self.command_history.take_dirty_entries()
    }

    pub(super) fn reset_session(&mut self) {
        self.entries = vec![TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::Agent,
            text: welcome_message(&self.model_name, self.initial_mode),
            style: MessageStyle::Plain,
        })];
        self.bump_transcript_revision();
        self.tick_count = 0;
        self.mode = self.initial_mode;
        self.session_history.clear();
        self.estimated_session_history_tokens = 0;
        self.last_history_model_name = None;
        self.pending_reply = None;
        self.pending_write_approvals.clear();
        self.pending_shell_approvals.clear();
        self.pending_plan_review = None;
        self.pending_ask_user = None;
        self.approval_mode = self.initial_approval_mode;
        self.resume_history_follow();
        self.history.reset();
        self.picker = None;
        self.planning_session = None;
        self.command_history.reset_navigation();
        self.clear_composer();
    }

    pub(crate) fn replace_session_history(&mut self, history: Vec<RigMessage>) {
        self.estimated_session_history_tokens = estimated_history_context_tokens(&history);
        self.session_history = history;
    }

    pub(crate) fn set_last_history_model_name(&mut self, model_name: Option<impl Into<String>>) {
        self.last_history_model_name = model_name.map(Into::into);
    }

    pub(crate) fn set_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.reasoning_effort = reasoning_effort;
    }

    pub(crate) fn set_safety_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.safety_reasoning_effort = reasoning_effort;
    }

    pub(crate) fn set_session_stats(&mut self, session_stats: StatsTotals) {
        self.session_stats = session_stats;
    }

    pub(crate) fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.model_name = model_name.into();
    }

    pub(crate) fn set_safety_model_name(&mut self, model_name: impl Into<String>) {
        self.safety_model_name = model_name.into();
    }

    pub(crate) fn set_planning_agents(&mut self, planning_agents: Vec<PlanningAgentConfig>) {
        self.planning_agents = planning_agents;
    }

    #[cfg(test)]
    pub(crate) fn set_workspace_root(&mut self, workspace_root: PathBuf) {
        self.workspace_root = workspace_root;
    }

    pub(crate) fn cancel_pending_reply(&mut self) {
        self.pending_reply = None;
        self.pending_write_approvals.clear();
        self.pending_shell_approvals.clear();
        self.pending_ask_user = None;
        if self.planning_session == Some(PlanningSessionStage::RunningFanout) {
            self.planning_session = Some(PlanningSessionStage::Conversation);
        }
        self.push_error_message("Request cancelled.");
    }

    pub(crate) fn begin_ask_user(&mut self, request_id: String, request: AskUserRequest) {
        self.pending_ask_user = Some(PendingAskUser::new(request_id, request));
    }

    pub(crate) fn clear_pending_ask_user(&mut self) {
        self.pending_ask_user = None;
    }

    pub(crate) fn begin_plan_review(&mut self) {
        self.clear_planning_session();
        self.pending_plan_review = Some(PendingPlanReview {
            mode: PendingPlanReviewMode::Selection,
            selected_index: 0,
        });
        self.clear_composer();
    }

    pub(crate) fn begin_plan_review_feedback(&mut self) {
        self.pending_plan_review = Some(PendingPlanReview {
            mode: PendingPlanReviewMode::Feedback,
            selected_index: 0,
        });
        self.clear_composer();
    }

    pub(crate) fn clear_plan_review(&mut self) {
        self.pending_plan_review = None;
    }

    pub(crate) fn selected_plan_review_index(&self) -> Option<usize> {
        self.pending_plan_review
            .as_ref()
            .filter(|review| review.mode == PendingPlanReviewMode::Selection)
            .map(|review| review.selected_index)
    }

    pub(crate) fn move_plan_review_selection(&mut self, direction: isize) {
        let Some(review) = self.pending_plan_review.as_mut() else {
            return;
        };
        if review.mode != PendingPlanReviewMode::Selection {
            return;
        }

        review.selected_index = (review.selected_index as isize + direction).rem_euclid(2) as usize;
    }

    pub(super) fn begin_write_approval(
        &mut self,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        self.enqueue_write_approval(None, request_id, tool_name, arguments);
    }

    pub(super) fn begin_subagent_write_approval(
        &mut self,
        subagent_id: String,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        self.enqueue_write_approval(Some(subagent_id), request_id, tool_name, arguments);
    }

    pub(super) fn begin_shell_approval(
        &mut self,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        self.enqueue_shell_approval(
            None,
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        );
    }

    pub(super) fn begin_subagent_shell_approval(
        &mut self,
        subagent_id: String,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        self.enqueue_shell_approval(
            Some(subagent_id),
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        );
    }

    fn enqueue_write_approval(
        &mut self,
        source_label: Option<String>,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        let preview = mutation_preview(&tool_name, &arguments, &self.workspace_root);
        let source_context = source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        let approval = PendingWriteApproval {
            request_id,
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            summary: write_approval_summary(&tool_name, &arguments, &self.workspace_root),
            target: preview.as_ref().map(|preview| preview.target.clone()),
            source_label,
        };
        self.push_agent_message(format!(
            "Write approval required for `{}`{}.",
            approval.tool_name, source_context
        ));
        self.pending_write_approvals.push_back(approval);
    }

    fn enqueue_shell_approval(
        &mut self,
        source_label: Option<String>,
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
    ) {
        let source_context = source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        let approval = PendingShellApproval::new(
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
            source_label,
        );
        self.push_agent_message(format!(
            "{} risk shell approval required{}.",
            approval.risk.label(),
            source_context
        ));
        self.pending_shell_approvals.push_back(approval);
    }

    pub(super) fn resolve_write_approval(
        &mut self,
        decision: WriteApprovalDecision,
    ) -> Option<PendingWriteApproval> {
        let pending = self.pending_write_approvals.pop_front()?;
        let source_context = pending
            .source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        match decision {
            WriteApprovalDecision::AllowOnce => {
                self.push_agent_message(format!(
                    "Approved `{}` once{}.",
                    pending.tool_name, source_context
                ));
            }
            WriteApprovalDecision::AllowAllSession => {
                self.approval_mode = ApprovalMode::Disabled;
                self.push_agent_message(format!(
                    "Approved `{}` and all future writes for this session{}.",
                    pending.tool_name, source_context
                ));
            }
            WriteApprovalDecision::Deny => {
                self.push_error_message(format!(
                    "Denied `{}`{}.",
                    pending.tool_name, source_context
                ));
            }
        }
        Some(pending)
    }

    pub(super) fn move_shell_approval_selection(&mut self, direction: isize) {
        let Some(pending) = self.pending_shell_approvals.front_mut() else {
            return;
        };
        pending.move_selection(direction);
    }

    pub(super) fn cancel_shell_approval_editing(&mut self) -> bool {
        let Some(pending) = self.pending_shell_approvals.front_mut() else {
            return false;
        };
        if pending.edit_mode != Some(ShellApprovalEditMode::Deny) {
            return false;
        }
        pending.cancel_editing();
        true
    }

    pub(super) fn toggle_shell_approval_detail_editing(&mut self) {
        let Some(pending) = self.pending_shell_approvals.front_mut() else {
            return;
        };
        if pending.selected_index != 3 {
            return;
        }
        pending.edit_mode = match pending.edit_mode {
            Some(ShellApprovalEditMode::Deny) => None,
            _ => Some(ShellApprovalEditMode::Deny),
        };
    }

    pub(super) fn apply_shell_approval_input(&mut self, input: Input) {
        let Some(pending) = self.pending_shell_approvals.front_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.input(input);
    }

    pub(super) fn paste_into_shell_approval_detail(&mut self, text: &str) {
        let Some(pending) = self.pending_shell_approvals.front_mut() else {
            return;
        };
        let Some(editor) = pending.active_editor_mut() else {
            return;
        };
        editor.insert_str(normalize_pasted_line_endings(text));
    }

    pub(super) fn submit_shell_approval(
        &mut self,
    ) -> Option<(String, ShellApprovalDecision, CommandRisk)> {
        let pending = self.pending_shell_approvals.front_mut()?;
        if pending.is_editing() {
            if pending.edit_mode == Some(ShellApprovalEditMode::Pattern)
                && pending.selected_decision().is_none()
            {
                self.push_error_message("Provide a non-empty shell approval pattern.");
                return None;
            }
            if pending.edit_mode == Some(ShellApprovalEditMode::Deny) {
                pending.cancel_editing();
            }
        } else if pending.selected_index == 1 {
            pending.begin_editing();
            return None;
        }

        let pending = self.pending_shell_approvals.pop_front()?;
        let source_context = pending
            .source_label
            .as_ref()
            .map(|source| format!(" from `{source}`"))
            .unwrap_or_default();
        let decision = pending
            .selected_decision()
            .unwrap_or(ShellApprovalDecision::Deny(None));
        match &decision {
            ShellApprovalDecision::AllowOnce => self.push_agent_message(format!(
                "Approved {} risk shell command once{}.",
                pending.risk.as_str(),
                source_context
            )),
            ShellApprovalDecision::AllowPattern(pattern) => self.push_agent_message(format!(
                "Approved {} risk shell commands matching `{}`{}.",
                pending.risk.as_str(),
                pattern,
                source_context
            )),
            ShellApprovalDecision::AllowAllRisk => self.push_agent_message(format!(
                "Approved all future {} risk shell commands this session{}.",
                pending.risk.as_str(),
                source_context
            )),
            ShellApprovalDecision::Deny(note) => {
                let suffix = note
                    .as_deref()
                    .filter(|note| !note.is_empty())
                    .map(|note| format!(" ({note})"))
                    .unwrap_or_default();
                self.push_error_message(format!(
                    "Denied {} risk shell command{}{}.",
                    pending.risk.as_str(),
                    source_context,
                    suffix
                ));
            }
        }

        Some((pending.request_id, decision, pending.risk))
    }

    pub fn push_agent_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Plain);
    }

    pub(crate) fn push_user_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::User, text, MessageStyle::Plain);
    }

    pub fn push_error_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Error);
    }

    pub(super) fn push_agent_error(&mut self, text: impl Into<String>) {
        self.push_error_message(text);
    }

    pub(super) fn push_agent_commentary(&mut self, text: impl Into<String>) {
        let text = text.into();
        if let Some(pending) = self.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.commentary_messages.push(text.clone());
            pending.has_visible_content = true;
        }
        self.push_message(Speaker::Agent, text, MessageStyle::Commentary);
    }

    pub(super) fn push_tool_call(&mut self, name: String, parameter: String) {
        if let Some(pending) = self.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            pending.has_visible_content = true;
        }
        self.entries.push(TranscriptEntry::ToolCall(ToolCall {
            preview: mutation_preview(&name, &parameter, &self.workspace_root),
            name,
            parameter,
        }));
        self.bump_transcript_revision();
    }

    pub(super) fn push_tool_result(&mut self, name: String, output: String) {
        if let Some(pending) = self.pending_reply.as_mut() {
            pending.reset_active_stream_segment();
            if self.show_tool_output {
                pending.has_visible_content = true;
            }
        }
        self.entries
            .push(TranscriptEntry::ToolResult(ToolResultEntry {
                name,
                output,
            }));
        self.bump_transcript_revision();
    }

    pub(super) fn upsert_subagent_status(
        &mut self,
        id: String,
        kind: SubagentStatusKind,
        display_label: String,
        state: SubagentDisplayState,
        status_text: String,
    ) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.kind = kind;
            entry.display_label = display_label;
            entry.state = state;
            entry.status_text = status_text;
            self.bump_transcript_revision();
            return;
        }

        self.entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                id,
                kind,
                display_label,
                state,
                status_text,
                latest_tool_name: None,
            }));
        self.bump_transcript_revision();
    }

    pub(super) fn set_subagent_latest_tool(&mut self, id: String, latest_tool_name: String) {
        if let Some(TranscriptEntry::SubagentStatus(entry)) = self.entries.iter_mut().find(
            |entry| matches!(entry, TranscriptEntry::SubagentStatus(status) if status.id == id),
        ) {
            entry.latest_tool_name = Some(latest_tool_name);
            self.bump_transcript_revision();
            return;
        }

        self.entries
            .push(TranscriptEntry::SubagentStatus(SubagentStatusEntry {
                display_label: id.clone(),
                id,
                kind: SubagentStatusKind::Subagent,
                state: SubagentDisplayState::Running,
                status_text: "running".into(),
                latest_tool_name: Some(latest_tool_name),
            }));
        self.bump_transcript_revision();
    }

    pub(super) fn append_pending_stream_message(&mut self, delta: &str, style: MessageStyle) {
        if delta.is_empty() {
            return;
        }

        if self.pending_reply.is_none() || style == MessageStyle::Error {
            return;
        }

        let existing_index = {
            let pending = self
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            let crossed_style_boundary = match style {
                MessageStyle::Plain => pending.reasoning_entry_index.is_some(),
                MessageStyle::Commentary => true,
                MessageStyle::Thinking => pending.text_entry_index.is_some(),
                MessageStyle::Error => false,
            };
            if crossed_style_boundary {
                pending.reset_active_stream_segment();
            }
            match style {
                MessageStyle::Plain => pending.text_entry_index,
                MessageStyle::Commentary => None,
                MessageStyle::Thinking => pending.reasoning_entry_index,
                MessageStyle::Error => None,
            }
        };

        let Some(existing_index) = existing_index else {
            let mut pending_text = delta.to_string();
            {
                let pending = self
                    .pending_reply
                    .as_mut()
                    .expect("pending reply checked above");
                match style {
                    MessageStyle::Plain => {
                        pending.plain_text.push_str(delta);
                        pending.staged_plain_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_plain_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_plain_text);
                    }
                    MessageStyle::Thinking => {
                        pending.reasoning_text.push_str(delta);
                        pending.staged_reasoning_text.push_str(delta);
                        if !pending_stream_text_is_visible(style, &pending.staged_reasoning_text) {
                            return;
                        }
                        pending_text = std::mem::take(&mut pending.staged_reasoning_text);
                    }
                    MessageStyle::Commentary => {
                        if !pending_stream_text_is_visible(style, delta) {
                            return;
                        }
                    }
                    MessageStyle::Error => return,
                }
            }

            self.push_message(Speaker::Agent, pending_text, style);
            let index = self.entries.len() - 1;
            let pending = self
                .pending_reply
                .as_mut()
                .expect("pending reply checked above");
            pending.has_visible_content = true;
            match style {
                MessageStyle::Plain => pending.text_entry_index = Some(index),
                MessageStyle::Commentary => {}
                MessageStyle::Thinking => pending.reasoning_entry_index = Some(index),
                MessageStyle::Error => {}
            }
            return;
        };

        if let Some(TranscriptEntry::Message(message)) = self.entries.get_mut(existing_index) {
            message.text.push_str(delta);
            if style == MessageStyle::Plain
                && let Some(pending) = self.pending_reply.as_mut()
            {
                pending.plain_text.push_str(delta);
            }
            self.bump_transcript_revision();
        }
    }

    fn push_message(&mut self, speaker: Speaker, text: impl Into<String>, style: MessageStyle) {
        self.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker,
            text: text.into(),
            style,
        }));
        self.bump_transcript_revision();
    }

    pub(super) fn move_command_selection_up(&mut self) {
        self.move_command_selection(-1);
    }

    pub(super) fn move_command_selection_down(&mut self) {
        self.move_command_selection(1);
    }

    pub(crate) fn open_model_picker(&mut self) {
        let normal_selected_index = model_registry::models()
            .iter()
            .position(|model| model.name == self.model_name)
            .unwrap_or(0);
        let safety_selected_index = model_registry::models()
            .iter()
            .position(|model| model.name == self.safety_model_name)
            .unwrap_or(0);
        self.picker = Some(SelectionPicker::Model {
            active_tab: ModelPickerTab::NormalAgent,
            normal_selected_index,
            planning_selected_index: 0,
            safety_selected_index,
        });
    }

    pub(crate) fn open_reasoning_picker(&mut self) {
        self.open_reasoning_picker_for(ReasoningPickerTarget::NormalAgent, self.model_name.clone());
    }

    pub(crate) fn open_reasoning_picker_for(
        &mut self,
        target: ReasoningPickerTarget,
        model_name: String,
    ) {
        let Some(options) = model_registry::reasoning_levels_for_model(&model_name) else {
            self.picker = None;
            return;
        };

        let selected_index = match target {
            ReasoningPickerTarget::NormalAgent => options
                .iter()
                .position(|level| *level == self.reasoning_effort)
                .unwrap_or(0),
            ReasoningPickerTarget::PlanningAgent => options
                .iter()
                .position(|level| {
                    self.planning_agents
                        .iter()
                        .find(|agent| agent.model_name == model_name)
                        .map(|agent| *level == agent.reasoning_effort)
                        .unwrap_or(false)
                })
                .unwrap_or_else(|| {
                    options
                        .iter()
                        .position(|level| *level == default_planning_reasoning(&model_name))
                        .unwrap_or(0)
                }),
            ReasoningPickerTarget::SafetyModel => options
                .iter()
                .position(|level| {
                    model_name == self.safety_model_name && *level == self.safety_reasoning_effort
                })
                .unwrap_or_else(|| {
                    options
                        .iter()
                        .position(|level| *level == default_planning_reasoning(&model_name))
                        .unwrap_or(0)
                }),
        };
        self.picker = Some(SelectionPicker::Reasoning {
            target,
            model_name,
            options: options.to_vec(),
            selected_index,
        });
    }

    pub(super) fn cancel_picker(&mut self) -> bool {
        self.picker.take().is_some()
    }

    pub(super) fn move_picker_selection_up(&mut self) {
        self.move_picker_selection(-1);
    }

    pub(super) fn move_picker_selection_down(&mut self) {
        self.move_picker_selection(1);
    }

    pub(super) fn move_picker_tab_left(&mut self) {
        self.move_picker_tab(-1);
    }

    pub(super) fn move_picker_tab_right(&mut self) {
        self.move_picker_tab(1);
    }

    pub(super) fn toggle_picker_selection(&mut self) -> Option<Vec<PlanningAgentConfig>> {
        let planning_selected_index = match self.picker.as_ref()? {
            SelectionPicker::Model {
                active_tab: ModelPickerTab::PlanningAgents,
                planning_selected_index,
                ..
            } => *planning_selected_index,
            _ => return None,
        };
        let model_name = match self.planning_models().get(planning_selected_index) {
            Some(model) => model.name.to_string(),
            None => return None,
        };

        if let Some(existing_index) = self
            .planning_agents
            .iter()
            .position(|agent| agent.model_name == model_name)
        {
            self.planning_agents.remove(existing_index);
        } else {
            self.planning_agents.push(PlanningAgentConfig {
                model_name,
                reasoning_effort: default_planning_reasoning(
                    self.planning_models()
                        .get(planning_selected_index)
                        .map(|model| model.name)
                        .unwrap_or_default(),
                ),
            });
        }

        Some(self.planning_agents.clone())
    }

    pub(super) fn apply_picker_selection(&mut self) -> Option<PickerSelection> {
        let picker = self.picker.take()?;
        match picker {
            SelectionPicker::Model {
                active_tab,
                normal_selected_index,
                planning_selected_index,
                safety_selected_index,
            } => match active_tab {
                ModelPickerTab::NormalAgent => model_registry::models()
                    .get(normal_selected_index)
                    .map(|model| PickerSelection::Model(model.name.to_string())),
                ModelPickerTab::PlanningAgents => {
                    let model_name = self
                        .planning_models()
                        .get(planning_selected_index)
                        .map(|model| model.name.to_string())?;
                    self.open_reasoning_picker_for(
                        ReasoningPickerTarget::PlanningAgent,
                        model_name,
                    );
                    None
                }
                ModelPickerTab::SafetyModel => {
                    let model_name = model_registry::models()
                        .get(safety_selected_index)
                        .map(|model| model.name.to_string())?;
                    self.open_reasoning_picker_for(ReasoningPickerTarget::SafetyModel, model_name);
                    None
                }
            },
            SelectionPicker::Reasoning {
                target,
                model_name,
                options,
                selected_index,
            } => options
                .get(selected_index)
                .copied()
                .map(|reasoning_effort| match target {
                    ReasoningPickerTarget::NormalAgent => {
                        PickerSelection::Reasoning(reasoning_effort)
                    }
                    ReasoningPickerTarget::PlanningAgent => {
                        let planning_agent = PlanningAgentConfig {
                            model_name,
                            reasoning_effort,
                        };
                        if let Some(existing) = self
                            .planning_agents
                            .iter_mut()
                            .find(|agent| agent.model_name == planning_agent.model_name)
                        {
                            *existing = planning_agent.clone();
                        } else {
                            self.planning_agents.push(planning_agent.clone());
                        }
                        PickerSelection::PlanningAgent(planning_agent)
                    }
                    ReasoningPickerTarget::SafetyModel => {
                        self.safety_model_name = model_name.clone();
                        self.safety_reasoning_effort = reasoning_effort;
                        PickerSelection::SafetySelection {
                            model_name,
                            reasoning_effort,
                        }
                    }
                }),
        }
    }

    pub(super) fn move_ask_user_tab_left(&mut self) {
        self.move_ask_user_tab(-1);
    }

    pub(super) fn move_ask_user_tab_right(&mut self) {
        self.move_ask_user_tab(1);
    }

    pub(super) fn move_ask_user_answer_up(&mut self) {
        self.move_ask_user_answer(-1);
    }

    pub(super) fn move_ask_user_answer_down(&mut self) {
        self.move_ask_user_answer(1);
    }

    pub(super) fn toggle_ask_user_detail_editing(&mut self) {
        let Some(pending) = self.pending_ask_user.as_mut() else {
            return;
        };
        if pending.active_tab >= pending.questions.len() {
            return;
        }

        pending.detail_editing = !pending.detail_editing;
    }

    pub(super) fn apply_ask_user_input(&mut self, input: Input) {
        let Some(question) = self.active_ask_user_question_mut() else {
            return;
        };
        question.detail.input(input);
    }

    pub(super) fn paste_into_ask_user_detail(&mut self, text: &str) {
        let Some(question) = self.active_ask_user_question_mut() else {
            return;
        };
        question
            .detail
            .insert_str(normalize_pasted_line_endings(text));
    }

    pub(super) fn submit_ask_user_response(&mut self) -> Option<(String, AskUserResponse, String)> {
        let pending = self.pending_ask_user.as_ref()?;
        if pending.active_tab != pending.questions.len() {
            return None;
        }
        if !pending.is_complete() {
            self.push_error_message("Complete all AskUser questions before submitting.");
            return None;
        }

        let response = pending.response();
        let request_id = pending.request_id.clone();
        let summary = response.transcript_summary();
        self.pending_ask_user = None;
        self.push_user_message(summary.clone());
        Some((request_id, response, summary))
    }

    pub(super) fn advance_ask_user(&mut self) -> Option<(String, AskUserResponse, String)> {
        let Some(pending) = self.pending_ask_user.as_ref() else {
            return None;
        };
        if pending.active_tab == pending.questions.len() {
            return self.submit_ask_user_response();
        }

        let question = &pending.questions[pending.active_tab];
        if !question.is_complete() {
            self.push_error_message("`Something else` requires details before continuing.");
            if let Some(pending) = self.pending_ask_user.as_mut() {
                pending.detail_editing = true;
            }
            return None;
        }

        if let Some(pending) = self.pending_ask_user.as_mut() {
            pending.detail_editing = false;
            pending.active_tab += 1;
        }
        None
    }

    pub(super) fn move_composer_cursor_up(&mut self) {
        let current_cursor = self.composer.cursor();
        let target = {
            let Some(cursor) = self.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index == 0 {
                if cursor.visual_col > 0 {
                    Some((cursor.row.line_index, cursor.row.start_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.composer_visual_column.unwrap_or(cursor.visual_col);
                self.composer_layout()
                    .target_cursor_for_row(cursor.row_index - 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.composer_visual_column = desired_col;
            }
            None => {
                self.composer_visual_column = None;
            }
        }
    }

    pub(super) fn move_composer_cursor_down(&mut self) {
        let current_cursor = self.composer.cursor();
        let target = {
            let Some(cursor) = self.composer_layout().cursor_state(current_cursor) else {
                return;
            };

            if cursor.row_index + 1 >= cursor.total_rows {
                if current_cursor.1 < cursor.row.end_col {
                    Some((cursor.row.line_index, cursor.row.end_col, None))
                } else {
                    None
                }
            } else {
                let desired_col = self.composer_visual_column.unwrap_or(cursor.visual_col);
                self.composer_layout()
                    .target_cursor_for_row(cursor.row_index + 1, desired_col)
                    .map(|(row, col)| (row, col, Some(desired_col)))
            }
        };

        match target {
            Some((row, col, desired_col)) => {
                self.composer
                    .move_cursor(CursorMove::Jump(row as u16, col as u16));
                self.composer_visual_column = desired_col;
            }
            None => {
                self.composer_visual_column = None;
            }
        }
    }

    pub(super) fn clear_composer(&mut self) {
        self.set_composer_text_internal("", true);
    }

    pub(crate) fn enter_planning_draft_mode(&mut self) {
        self.planning_session = Some(PlanningSessionStage::Drafting);
        self.clear_composer();
    }

    pub(crate) fn cancel_planning_draft_mode(&mut self) -> bool {
        if self.planning_session != Some(PlanningSessionStage::Drafting) {
            return false;
        }

        self.planning_session = None;
        self.clear_composer();
        true
    }

    pub(crate) fn consume_planning_draft_mode(&mut self) -> bool {
        let was_active = self.planning_draft_mode();
        if was_active {
            self.planning_session = Some(PlanningSessionStage::Conversation);
        }
        was_active
    }

    pub(crate) fn begin_planning_conversation(&mut self) {
        self.planning_session = Some(PlanningSessionStage::Conversation);
    }

    pub(crate) fn begin_planning_fanout(&mut self) {
        self.planning_session = Some(PlanningSessionStage::RunningFanout);
    }

    pub(crate) fn begin_planning_finalization(&mut self) {
        self.planning_session = Some(PlanningSessionStage::Finalizing);
    }

    pub(crate) fn clear_planning_session(&mut self) {
        self.planning_session = None;
    }

    pub(super) fn insert_composer_newline(&mut self) {
        self.command_history.reset_navigation();
        self.invalidate_composer_layout();
        self.composer_visual_column = None;
        self.composer.insert_newline();
        self.sync_command_selection();
    }

    pub(super) fn apply_composer_input(&mut self, input: Input) {
        self.command_history.reset_navigation();
        self.invalidate_composer_layout();
        self.composer_visual_column = None;
        self.composer.input(input);
        self.sync_command_selection();
    }

    pub(super) fn paste_into_composer(&mut self, text: &str) {
        self.command_history.reset_navigation();
        self.invalidate_composer_layout();
        self.composer_visual_column = None;
        self.composer
            .insert_str(normalize_pasted_line_endings(text));
        self.sync_command_selection();
    }

    pub(super) fn record_submitted_input(&mut self, text: &str) {
        self.command_history.record(text);
    }

    pub(super) fn should_recall_previous_input(&mut self) -> bool {
        let current_cursor = self.composer.cursor();
        self.composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| cursor.row_index == 0 && cursor.visual_col == 0)
    }

    pub(super) fn should_recall_next_input(&mut self) -> bool {
        let current_cursor = self.composer.cursor();
        self.composer_layout()
            .cursor_state(current_cursor)
            .is_some_and(|cursor| {
                cursor.row_index + 1 >= cursor.total_rows && current_cursor.1 == cursor.row.end_col
            })
    }

    pub(super) fn recall_previous_input(&mut self) -> bool {
        let current = self.composer.lines().join("\n");
        let Some(previous) = self.command_history.previous(&current) else {
            return false;
        };
        self.set_composer_text_internal(&previous, false);
        true
    }

    pub(super) fn recall_next_input(&mut self) -> bool {
        let Some(next) = self.command_history.next() else {
            return false;
        };
        self.set_composer_text_internal(&next, false);
        true
    }

    pub(crate) fn sync_history_viewport(
        &mut self,
        total_lines: usize,
        viewport_rows: usize,
    ) -> usize {
        self.history.sync_viewport(total_lines, viewport_rows)
    }

    pub(crate) fn history_total_lines(&self) -> usize {
        self.history.total_lines()
    }

    pub(crate) fn history_viewport_rows(&self) -> usize {
        self.history.viewport_rows()
    }

    pub(crate) fn history_scroll_position(&self) -> usize {
        self.history.scroll_position()
    }

    pub(crate) fn update_history_snapshot(&mut self, area: Rect, lines: Vec<String>) {
        self.history.update_snapshot(area, lines);
    }

    pub(crate) fn history_cache_allowed(&self) -> bool {
        !self.shows_startup_banner() && !self.should_show_history_busy_indicator()
    }

    pub(crate) fn cached_history_lines(
        &self,
        width: usize,
        accent: Color,
    ) -> Option<&[Line<'static>]> {
        let cache = self.history_render_cache.as_ref()?;
        (cache.width == width
            && cache.accent == accent
            && cache.transcript_revision == self.transcript_revision)
            .then_some(cache.lines.as_slice())
    }

    pub(crate) fn store_history_render_cache(
        &mut self,
        width: usize,
        accent: Color,
        lines: Vec<Line<'static>>,
    ) {
        self.history_render_cache = Some(HistoryRenderCache {
            width,
            accent,
            transcript_revision: self.transcript_revision,
            lines,
        });
    }

    pub(crate) fn clear_history_render_cache(&mut self) {
        self.history_render_cache = None;
    }

    fn bump_transcript_revision(&mut self) {
        self.transcript_revision = self.transcript_revision.wrapping_add(1);
        self.history_render_cache = None;
    }

    pub(super) fn scroll_history_page_up(&mut self) {
        self.scroll_history_up(self.history.page_rows());
    }

    pub(super) fn scroll_history_page_down(&mut self) {
        self.scroll_history_down(self.history.page_rows());
    }

    pub(super) fn scroll_history_up(&mut self, lines: usize) {
        self.history.scroll_up(lines);
    }

    pub(super) fn scroll_history_down(&mut self, lines: usize) {
        self.history.scroll_down(lines);
    }

    pub(super) fn scroll_history_to_top(&mut self) {
        self.history.scroll_to_top();
    }

    pub(super) fn resume_history_follow(&mut self) {
        self.history.resume_follow();
    }

    pub(super) fn start_history_selection(&mut self, column: u16, row: u16) {
        self.history.start_selection(column, row);
    }

    pub(super) fn update_history_selection(&mut self, column: u16, row: u16) {
        self.history.update_selection(column, row);
    }

    pub(super) fn finish_history_selection(&mut self, column: u16, row: u16) -> Option<String> {
        self.history.finish_selection(column, row)
    }

    pub(crate) fn history_selection_span_for_row(&self, row: usize) -> Option<(usize, usize)> {
        self.history.selection_span_for_row(row)
    }

    fn move_command_selection(&mut self, direction: isize) {
        let commands = self.filtered_commands();
        if commands.is_empty() {
            return;
        }

        let current_index = commands
            .iter()
            .position(|command| *command == self.selected_command)
            .unwrap_or(0);
        let next_index = (current_index as isize + direction).rem_euclid(commands.len() as isize);
        self.selected_command = commands[next_index as usize];
    }

    pub(super) fn sync_command_selection(&mut self) {
        let commands = self.filtered_commands();
        if let Some(command) = commands.first().copied()
            && !commands.contains(&self.selected_command)
        {
            self.selected_command = command;
        }
    }

    fn planning_models(&self) -> Vec<&'static model_registry::ModelInfo> {
        model_registry::models()
            .iter()
            .filter(|model| model.name != self.model_name)
            .collect()
    }

    fn active_ask_user_question_mut(&mut self) -> Option<&mut PendingAskUserQuestion> {
        let pending = self.pending_ask_user.as_mut()?;
        if !pending.detail_editing || pending.active_tab >= pending.questions.len() {
            return None;
        }
        pending.questions.get_mut(pending.active_tab)
    }

    fn move_ask_user_tab(&mut self, direction: isize) {
        let Some(pending) = self.pending_ask_user.as_mut() else {
            return;
        };

        let tab_count = pending.questions.len() + 1;
        pending.active_tab =
            (pending.active_tab as isize + direction).rem_euclid(tab_count as isize) as usize;
        if pending.active_tab >= pending.questions.len() {
            pending.detail_editing = false;
        }
    }

    fn move_ask_user_answer(&mut self, direction: isize) {
        let Some(pending) = self.pending_ask_user.as_mut() else {
            return;
        };
        if pending.active_tab >= pending.questions.len() {
            return;
        }

        let question = &mut pending.questions[pending.active_tab];
        let len = question.answers.len();
        if len == 0 {
            return;
        }
        question.selected_index =
            (question.selected_index as isize + direction).rem_euclid(len as isize) as usize;
        if question.selected_answer().is_something_else {
            pending.detail_editing = true;
        }
    }

    fn move_picker_tab(&mut self, direction: isize) {
        let Some(SelectionPicker::Model { active_tab, .. }) = self.picker.as_mut() else {
            return;
        };

        active_tab.toggle(direction);
    }

    fn move_picker_selection(&mut self, direction: isize) {
        let planning_len = self.planning_models().len();
        let Some(picker) = self.picker.as_mut() else {
            return;
        };

        match picker {
            SelectionPicker::Model {
                active_tab,
                normal_selected_index,
                planning_selected_index,
                safety_selected_index,
            } => match active_tab {
                ModelPickerTab::NormalAgent => {
                    let len = model_registry::models().len();
                    if len == 0 {
                        return;
                    }
                    *normal_selected_index = (*normal_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
                ModelPickerTab::PlanningAgents => {
                    if planning_len == 0 {
                        return;
                    }
                    *planning_selected_index = (*planning_selected_index as isize + direction)
                        .rem_euclid(planning_len as isize)
                        as usize;
                }
                ModelPickerTab::SafetyModel => {
                    let len = model_registry::models().len();
                    if len == 0 {
                        return;
                    }
                    *safety_selected_index = (*safety_selected_index as isize + direction)
                        .rem_euclid(len as isize)
                        as usize;
                }
            },
            SelectionPicker::Reasoning {
                options,
                selected_index,
                ..
            } => {
                let len = options.len();
                if len == 0 {
                    return;
                }
                *selected_index =
                    (*selected_index as isize + direction).rem_euclid(len as isize) as usize;
            }
        }
    }
}

impl HistoryViewState {
    fn is_pinned(&self) -> bool {
        self.scroll_top.is_some()
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn sync_viewport(&mut self, total_lines: usize, viewport_rows: usize) -> usize {
        self.total_lines = total_lines;
        self.viewport_rows = viewport_rows.max(1);
        let max_start = self.max_start();
        if let Some(top) = self.scroll_top.as_mut() {
            *top = (*top).min(max_start);
            *top
        } else {
            max_start
        }
    }

    fn total_lines(&self) -> usize {
        self.total_lines
    }

    fn viewport_rows(&self) -> usize {
        self.viewport_rows.max(1)
    }

    fn scroll_position(&self) -> usize {
        self.current_start()
    }

    fn update_snapshot(&mut self, area: Rect, lines: Vec<String>) {
        self.snapshot_area = area;
        self.snapshot_lines = lines;
    }

    fn page_rows(&self) -> usize {
        self.viewport_rows.max(1)
    }

    fn scroll_up(&mut self, lines: usize) {
        let current = self.current_start();
        self.scroll_top = Some(current.saturating_sub(lines));
    }

    fn scroll_down(&mut self, lines: usize) {
        let current = self.current_start();
        self.scroll_top = Some(current.saturating_add(lines).min(self.max_start()));
    }

    fn scroll_to_top(&mut self) {
        self.scroll_top = Some(0);
    }

    fn resume_follow(&mut self) {
        self.scroll_top = None;
    }

    fn start_selection(&mut self, column: u16, row: u16) {
        let point = self.selection_point(column, row, false);
        self.selection_anchor = point;
        self.selection_focus = point;
    }

    fn update_selection(&mut self, column: u16, row: u16) {
        if self.selection_anchor.is_none() {
            return;
        }
        self.selection_focus = self.selection_point(column, row, true);
    }

    fn finish_selection(&mut self, column: u16, row: u16) -> Option<String> {
        let anchor = self.selection_anchor?;
        let focus = self
            .selection_point(column, row, true)
            .or(self.selection_focus)?;
        self.selection_anchor = None;
        self.selection_focus = None;
        (anchor != focus).then(|| self.selected_text(anchor, focus))
    }

    fn selection_span_for_row(&self, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.ordered_selection_points()?;
        if row < start.row || row > end.row {
            return None;
        }

        let line_width = self.snapshot_lines.get(row)?.chars().count().max(1);
        let span = if start.row == end.row {
            (start.column, end.column + 1)
        } else if row == start.row {
            (start.column, line_width)
        } else if row == end.row {
            (0, end.column + 1)
        } else {
            (0, line_width)
        };

        Some((span.0.min(line_width), span.1.min(line_width)))
    }

    fn current_start(&self) -> usize {
        self.scroll_top.unwrap_or(self.max_start())
    }

    fn max_start(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_rows.max(1))
    }

    fn selection_point(&self, column: u16, row: u16, clamp: bool) -> Option<HistorySelectionPoint> {
        if self.snapshot_lines.is_empty() || self.snapshot_area.width == 0 {
            return None;
        }

        let area = self.snapshot_area;
        let min_row = area.y;
        let max_row = area
            .y
            .saturating_add(self.snapshot_lines.len().saturating_sub(1) as u16);
        let row = if clamp {
            row.clamp(min_row, max_row)
        } else if row < min_row || row > max_row {
            return None;
        } else {
            row
        };

        let min_column = area.x;
        let max_column = area.x.saturating_add(area.width.saturating_sub(1));
        let column = if clamp {
            column.clamp(min_column, max_column)
        } else if column < min_column || column > max_column {
            return None;
        } else {
            column
        };

        let row_index = row.saturating_sub(area.y) as usize;
        let line_width = self.snapshot_lines[row_index].chars().count();
        let column_index = column.saturating_sub(area.x) as usize;

        Some(HistorySelectionPoint {
            row: row_index,
            column: column_index.min(line_width.saturating_sub(1)),
        })
    }

    fn ordered_selection_points(&self) -> Option<(HistorySelectionPoint, HistorySelectionPoint)> {
        let anchor = self.selection_anchor?;
        let focus = self.selection_focus?;
        if anchor == focus {
            return None;
        }

        Some(
            if (anchor.row, anchor.column) <= (focus.row, focus.column) {
                (anchor, focus)
            } else {
                (focus, anchor)
            },
        )
    }

    fn selected_text(&self, anchor: HistorySelectionPoint, focus: HistorySelectionPoint) -> String {
        let (start, end) = if (anchor.row, anchor.column) <= (focus.row, focus.column) {
            (anchor, focus)
        } else {
            (focus, anchor)
        };

        let mut lines = Vec::new();
        for row in start.row..=end.row {
            let line = &self.snapshot_lines[row];
            let segment = if start.row == end.row {
                slice_line(line, start.column, end.column + 1)
            } else if row == start.row {
                slice_line(line, start.column, line.chars().count())
            } else if row == end.row {
                slice_line(line, 0, end.column + 1)
            } else {
                line.clone()
            };
            lines.push(segment);
        }

        lines.join("\n")
    }
}

impl PendingAskUser {
    fn new(request_id: String, request: AskUserRequest) -> Self {
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or("Ask User")
            .to_string();
        let questions = request
            .questions
            .into_iter()
            .map(PendingAskUserQuestion::from_request)
            .collect();
        Self {
            request_id,
            title,
            active_tab: 0,
            detail_editing: false,
            questions,
        }
    }

    fn is_complete(&self) -> bool {
        self.questions
            .iter()
            .all(PendingAskUserQuestion::is_complete)
    }

    fn response(&self) -> AskUserResponse {
        AskUserResponse {
            questions: self
                .questions
                .iter()
                .map(PendingAskUserQuestion::response)
                .collect(),
        }
    }
}

impl PendingAskUserQuestion {
    fn from_request(question: AskUserQuestion) -> Self {
        let mut answers = question
            .answers
            .into_iter()
            .enumerate()
            .map(|(index, answer)| PendingAskUserAnswer::from_request(answer, index == 0))
            .collect::<Vec<_>>();
        answers.push(PendingAskUserAnswer {
            id: SOMETHING_ELSE_ID.into(),
            label: SOMETHING_ELSE_LABEL.into(),
            is_recommended: false,
            is_something_else: true,
        });

        Self {
            id: question.id,
            prompt: question.prompt,
            answers,
            selected_index: 0,
            detail: new_text_area_with_text("", ""),
        }
    }

    fn selected_answer(&self) -> &PendingAskUserAnswer {
        &self.answers[self.selected_index]
    }

    fn detail_text(&self) -> String {
        self.detail.lines().join("\n").trim().to_string()
    }

    fn is_complete(&self) -> bool {
        !self.selected_answer().is_something_else || !self.detail_text().is_empty()
    }

    fn response(&self) -> AskUserAnsweredQuestion {
        let selected = self.selected_answer();
        AskUserAnsweredQuestion {
            id: self.id.clone(),
            prompt: self.prompt.clone(),
            selected_answer: AskUserSelectedAnswer {
                id: selected.id.clone(),
                label: selected.label.clone(),
                is_recommended: selected.is_recommended,
                is_something_else: selected.is_something_else,
            },
            details: self.detail_text(),
        }
    }
}

impl PendingAskUserAnswer {
    fn from_request(answer: AskUserAnswer, is_recommended: bool) -> Self {
        Self {
            id: answer.id,
            label: answer.label,
            is_recommended,
            is_something_else: false,
        }
    }
}

impl PendingShellApproval {
    fn new(
        request_id: String,
        risk: CommandRisk,
        risk_explanation: String,
        command: String,
        working_directory: String,
        reason: String,
        source_label: Option<String>,
    ) -> Self {
        let mut pattern_input =
            new_text_area_with_text(&default_shell_approval_pattern(&command), "");
        pattern_input.move_cursor(CursorMove::End);
        Self {
            request_id,
            risk,
            risk_explanation,
            command: command.clone(),
            working_directory,
            reason,
            source_label,
            selected_index: 0,
            edit_mode: None,
            pattern_input,
            deny_input: new_text_area_with_text("", ""),
        }
    }

    fn option_count(&self) -> usize {
        4
    }

    fn move_selection(&mut self, direction: isize) {
        self.selected_index = (self.selected_index as isize + direction)
            .rem_euclid(self.option_count() as isize) as usize;
        match self.selected_index {
            1 => {
                self.edit_mode = Some(ShellApprovalEditMode::Pattern);
                self.pattern_input.move_cursor(CursorMove::End);
            }
            3 => {
                if self.edit_mode == Some(ShellApprovalEditMode::Pattern) {
                    self.edit_mode = None;
                }
            }
            _ => self.edit_mode = None,
        }
    }

    fn selected_edit_mode(&self) -> Option<ShellApprovalEditMode> {
        match self.selected_index {
            1 => Some(ShellApprovalEditMode::Pattern),
            3 => Some(ShellApprovalEditMode::Deny),
            _ => None,
        }
    }

    fn begin_editing(&mut self) {
        self.edit_mode = self.selected_edit_mode();
        if self.edit_mode == Some(ShellApprovalEditMode::Pattern) {
            self.pattern_input.move_cursor(CursorMove::End);
        }
    }

    fn cancel_editing(&mut self) {
        self.edit_mode = None;
    }

    fn is_editing(&self) -> bool {
        self.edit_mode.is_some()
    }

    fn active_editor_mut(&mut self) -> Option<&mut TextArea<'static>> {
        match self.edit_mode {
            Some(ShellApprovalEditMode::Pattern) => Some(&mut self.pattern_input),
            Some(ShellApprovalEditMode::Deny) => Some(&mut self.deny_input),
            None => None,
        }
    }

    fn active_editor(&self) -> Option<&TextArea<'static>> {
        match self.edit_mode {
            Some(ShellApprovalEditMode::Pattern) => Some(&self.pattern_input),
            Some(ShellApprovalEditMode::Deny) => Some(&self.deny_input),
            None => None,
        }
    }

    fn editor_can_move_up(&self) -> bool {
        self.active_editor()
            .is_some_and(|editor| editor.cursor().0 > 0)
    }

    fn editor_can_move_down(&self) -> bool {
        self.active_editor().is_some_and(|editor| {
            let current_row = editor.cursor().0;
            current_row + 1 < editor.lines().len()
        })
    }

    fn selected_decision(&self) -> Option<ShellApprovalDecision> {
        match self.selected_index {
            0 => Some(ShellApprovalDecision::AllowOnce),
            1 => {
                let pattern = self.pattern_input.lines().join("\n").trim().to_string();
                (!pattern.is_empty()).then_some(ShellApprovalDecision::AllowPattern(pattern))
            }
            2 => Some(ShellApprovalDecision::AllowAllRisk),
            3 => {
                let note = self.deny_input.lines().join("\n").trim().to_string();
                Some(ShellApprovalDecision::Deny(
                    (!note.is_empty()).then_some(note),
                ))
            }
            _ => None,
        }
    }
}

fn default_shell_approval_pattern(command: &str) -> String {
    let first_line = command.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return command.trim().to_string();
    }

    let tokens = shell_command_prefix_tokens(first_line);
    let word_tokens = tokens
        .iter()
        .copied()
        .filter(|token| !is_shell_redirection_token(token))
        .collect::<Vec<_>>();
    let has_extra_shell_syntax = command.lines().nth(1).is_some()
        || tokens.iter().any(|token| is_shell_redirection_token(token));

    match word_tokens.as_slice() {
        [] => first_line.to_string(),
        [single] if has_extra_shell_syntax => format!("{single} *"),
        [single] => (*single).to_string(),
        many => format!("{} *", many[..many.len() - 1].join(" ")),
    }
}

fn shell_command_prefix_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < line.len() {
        let ch = line[index..]
            .chars()
            .next()
            .expect("valid char boundary while tokenizing shell command");
        if ch.is_whitespace() {
            index += ch.len_utf8();
            continue;
        }
        if starts_with_shell_control_operator(&line[index..]) {
            break;
        }

        let start = index;
        let mut in_single_quotes = false;
        let mut in_double_quotes = false;
        let mut escaped = false;

        while index < line.len() {
            let ch = line[index..]
                .chars()
                .next()
                .expect("valid char boundary while scanning shell token");

            if escaped {
                escaped = false;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && ch == '\\' {
                escaped = true;
                index += ch.len_utf8();
                continue;
            }

            if !in_double_quotes && ch == '\'' {
                in_single_quotes = !in_single_quotes;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && ch == '"' {
                in_double_quotes = !in_double_quotes;
                index += ch.len_utf8();
                continue;
            }

            if !in_single_quotes && !in_double_quotes {
                if ch.is_whitespace() || starts_with_shell_control_operator(&line[index..]) {
                    break;
                }
            }

            index += ch.len_utf8();
        }

        tokens.push(&line[start..index]);

        if starts_with_shell_control_operator(&line[index..]) {
            break;
        }
    }

    tokens
}

fn starts_with_shell_control_operator(input: &str) -> bool {
    input.starts_with("&&")
        || input.starts_with("||")
        || input.starts_with('|')
        || input.starts_with(';')
        || input.starts_with('&')
}

fn is_shell_redirection_token(token: &str) -> bool {
    let trimmed = token.trim_start_matches(|ch: char| ch.is_ascii_digit());
    trimmed.starts_with('<') || trimmed.starts_with('>')
}

impl Default for CommandRecallState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            browsing_index: None,
            draft: None,
            limit: DEFAULT_COMMAND_HISTORY_LIMIT,
            dirty: false,
        }
    }
}

impl CommandRecallState {
    fn restore(&mut self, mut entries: Vec<String>, limit: usize) {
        self.limit = limit;
        self.browsing_index = None;
        self.draft = None;
        self.dirty = false;
        self.entries.clear();
        self.entries.append(&mut entries);
        self.trim_to_limit();
    }

    fn record(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }

        if self.entries.last().is_some_and(|entry| entry == text) {
            self.browsing_index = None;
            self.draft = None;
            return;
        }

        self.entries.push(text.to_string());
        self.trim_to_limit();
        self.browsing_index = None;
        self.draft = None;
        self.dirty = true;
    }

    fn previous(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }

        match self.browsing_index {
            Some(index) if index > 0 => self.browsing_index = Some(index - 1),
            Some(_) => {}
            None => {
                self.draft = Some(current.to_string());
                self.browsing_index = Some(self.entries.len() - 1);
            }
        }

        self.browsing_index.map(|index| self.entries[index].clone())
    }

    fn next(&mut self) -> Option<String> {
        match self.browsing_index {
            None => None,
            Some(index) if index + 1 < self.entries.len() => {
                self.browsing_index = Some(index + 1);
                self.browsing_index.map(|index| self.entries[index].clone())
            }
            Some(_) => {
                self.browsing_index = None;
                Some(self.draft.take().unwrap_or_default())
            }
        }
    }

    fn reset_navigation(&mut self) {
        self.browsing_index = None;
        self.draft = None;
    }

    fn take_dirty_entries(&mut self) -> Option<Vec<String>> {
        if !self.dirty {
            return None;
        }

        self.dirty = false;
        Some(self.entries.clone())
    }

    fn trim_to_limit(&mut self) {
        self.entries.retain(|entry| !entry.trim().is_empty());
        self.entries.dedup();
        if self.entries.len() > self.limit {
            self.entries.drain(..self.entries.len() - self.limit);
        }
    }
}

fn new_composer() -> TextArea<'static> {
    new_composer_with_text("")
}

fn picker_height(picker: &SelectionPicker) -> u16 {
    let line_count = match picker {
        SelectionPicker::Model { .. } => model_registry::models().len().max(1) + 1,
        SelectionPicker::Reasoning { options, .. } => options.len(),
    }
    .clamp(1, 4) as u16;

    line_count + 2
}

fn split_command_query(query: &str) -> (&str, &str) {
    let mut parts = query.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("");
    let arguments = parts.next().unwrap_or("").trim();
    (name, arguments)
}

fn new_composer_with_text(text: &str) -> TextArea<'static> {
    new_text_area_with_text(text, "Send a message...")
}

fn new_text_area_with_text(text: &str, placeholder: &str) -> TextArea<'static> {
    let mut composer = if text.is_empty() {
        TextArea::default()
    } else {
        TextArea::from(text.lines())
    };
    composer.set_placeholder_text(placeholder);
    composer
}

fn normalize_pasted_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn welcome_message(model_name: &str, mode: AccessMode) -> String {
    let _ = mode;
    format!(
        "Loaded Azure model `{model_name}` from config. Send a message to start a one-shot response, or type / for commands."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest};
    use ratatui::{style::Color, text::Line};
    use ratatui_textarea::CursorMove;

    fn sample_ask_user_request() -> AskUserRequest {
        AskUserRequest {
            title: Some("Clarify implementation".into()),
            questions: vec![
                AskUserQuestion {
                    id: "scope".into(),
                    prompt: "Which scope?".into(),
                    answers: vec![
                        AskUserAnswer {
                            id: "narrow".into(),
                            label: "Narrow".into(),
                        },
                        AskUserAnswer {
                            id: "broad".into(),
                            label: "Broad".into(),
                        },
                    ],
                },
                AskUserQuestion {
                    id: "rollout".into(),
                    prompt: "Which rollout?".into(),
                    answers: vec![AskUserAnswer {
                        id: "single".into(),
                        label: "Single step".into(),
                    }],
                },
            ],
        }
    }

    #[test]
    fn access_mode_toggle_updates_mode_and_label() {
        let mut mode = AccessMode::ReadOnly;
        assert_eq!(mode.label(), "Read-only");

        mode.toggle();
        assert_eq!(mode, AccessMode::ReadWrite);
        assert_eq!(mode.label(), "Write");

        mode.toggle();
        assert_eq!(mode, AccessMode::ReadOnly);
    }

    #[test]
    fn app_starts_in_read_only_mode_with_greeting() {
        let app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        assert_eq!(app.mode(), AccessMode::ReadOnly);
        assert_eq!(app.approval_mode(), ApprovalMode::Manual);
        assert!(!app.has_pending_write_approval());
        assert!(!app.should_quit());
        assert!(!app.has_pending_reply());
        assert_eq!(app.entries().len(), 1);
        assert!(app.shows_startup_banner());
        assert_eq!(app.model_name(), "gpt-5-mini");
        assert_eq!(app.reasoning_effort(), ReasoningEffort::Medium);
        assert!(!app.show_tool_output());
    }

    #[test]
    fn composer_height_grows_with_multiple_lines() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer_mut().insert_str("one");
        assert_eq!(app.composer_height(), 3);

        app.composer_mut().insert_newline();
        app.composer_mut().insert_str("two");
        assert_eq!(app.composer_height(), 4);
    }

    #[test]
    fn composer_height_counts_wrapped_visual_rows() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.set_composer_wrap_width(6);
        app.composer.insert_str("alpha beta gamma");

        assert_eq!(app.composer_height(), 5);
    }

    #[test]
    fn slash_command_filter_matches_alias_prefixes() {
        assert_eq!(
            SlashCommand::filtered("/cl"),
            vec![SlashCommand::NewSession]
        );
        assert_eq!(SlashCommand::filtered("/st"), vec![SlashCommand::Stats]);
        assert_eq!(SlashCommand::filtered("/mo"), vec![SlashCommand::Model]);
        assert_eq!(SlashCommand::filtered("/ex"), vec![SlashCommand::Quit]);
        assert_eq!(SlashCommand::filtered("/th"), vec![SlashCommand::Effort]);
    }

    #[test]
    fn command_palette_only_shows_for_single_line_slash_input() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer.insert_str("/");
        assert!(app.command_palette_visible());

        app.composer.insert_newline();
        assert!(!app.command_palette_visible());
    }

    #[test]
    fn paste_into_composer_preserves_newlines_and_whitespace() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        app.paste_into_composer("  alpha\n\tbeta\r\ngamma  \n");

        assert_eq!(
            app.composer.lines(),
            &[
                "  alpha".to_string(),
                "\tbeta".to_string(),
                "gamma  ".to_string(),
                String::new(),
            ]
        );
    }

    #[test]
    fn history_status_defaults_to_live_follow() {
        let app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        assert!(!app.history_is_pinned());
        assert_eq!(app.history_status_label(), "History live  PgUp/PgDn scroll");
    }

    #[test]
    fn sync_history_viewport_clamps_pinned_position() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.history.scroll_top = Some(50);

        let start = app.sync_history_viewport(20, 6);

        assert_eq!(start, 14);
        assert_eq!(app.history.scroll_top, Some(14));
    }

    #[test]
    fn command_name_and_arguments_split_on_first_space() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer.insert_str("/effort xhigh");

        assert_eq!(app.command_name(), Some("/effort"));
        assert_eq!(app.command_arguments(), Some("xhigh"));
        assert_eq!(app.filtered_commands(), vec![SlashCommand::Effort]);
    }

    #[test]
    fn history_selection_extracts_visible_text_across_rows() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.update_history_snapshot(
            Rect {
                x: 4,
                y: 2,
                width: 20,
                height: 2,
            },
            vec!["alpha".into(), "beta".into()],
        );

        app.start_history_selection(5, 2);
        let selected = app.finish_history_selection(6, 3);

        assert_eq!(selected.as_deref(), Some("lpha\nbet"));
    }

    #[test]
    fn model_picker_uses_current_registry_model_as_selection() {
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);

        app.open_model_picker();

        assert_eq!(
            app.selection_picker(),
            Some(&SelectionPicker::Model {
                active_tab: ModelPickerTab::NormalAgent,
                normal_selected_index: 1,
                planning_selected_index: 0,
                safety_selected_index: 1,
            })
        );
        assert_eq!(app.overlay_height(), 6);
    }

    #[test]
    fn safety_reasoning_picker_uses_current_safety_selection() {
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
        app.set_safety_model_name("gpt-5.4");
        app.set_safety_reasoning_effort(ReasoningEffort::High);

        app.open_reasoning_picker_for(ReasoningPickerTarget::SafetyModel, "gpt-5.4".into());

        assert_eq!(
            app.selection_picker(),
            Some(&SelectionPicker::Reasoning {
                target: ReasoningPickerTarget::SafetyModel,
                model_name: "gpt-5.4".into(),
                options: vec![
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                ],
                selected_index: 2,
            })
        );
    }

    #[test]
    fn reasoning_picker_uses_current_reasoning_effort_as_selection() {
        let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::High);

        app.open_reasoning_picker();

        assert_eq!(
            app.selection_picker(),
            Some(&SelectionPicker::Reasoning {
                target: ReasoningPickerTarget::NormalAgent,
                model_name: "gpt-5.4".into(),
                options: vec![
                    ReasoningEffort::Low,
                    ReasoningEffort::Medium,
                    ReasoningEffort::High,
                ],
                selected_index: 2,
            })
        );
    }

    #[test]
    fn next_request_context_percent_uses_session_history_and_model_window() {
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
        app.replace_session_history(vec![RigMessage::assistant("token ".repeat(8_000))]);

        let estimated = app.estimated_next_request_context_tokens();

        assert!(estimated >= 4_000);
        assert_eq!(
            app.next_request_context_percent(),
            estimated * 100 / 272_000
        );
    }

    #[test]
    fn new_session_clears_cached_session_history_estimate() {
        let mut app = App::new(true, false, "gpt-5.4-mini", ReasoningEffort::Medium);
        app.replace_session_history(vec![RigMessage::assistant("token ".repeat(8_000))]);

        assert!(app.estimated_next_request_context_tokens() > 0);

        app.reset_session();

        assert_eq!(app.estimated_next_request_context_tokens(), 0);
    }

    #[test]
    fn supported_reasoning_levels_fall_back_for_unknown_model() {
        let app = App::new(true, false, "custom-deployment", ReasoningEffort::Medium);

        assert_eq!(
            app.supported_reasoning_levels(),
            vec![
                ReasoningEffort::Minimal,
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::XHigh,
            ]
        );
    }

    #[test]
    fn transcript_mutation_invalidates_history_render_cache() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.store_history_render_cache(80, Color::Cyan, vec![Line::from("cached transcript line")]);

        assert!(app.cached_history_lines(80, Color::Cyan).is_some());

        app.push_agent_message("new transcript line");

        assert!(app.cached_history_lines(80, Color::Cyan).is_none());
    }

    #[test]
    fn beginning_write_approval_tracks_pending_request() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        app.begin_write_approval(
            "call-1".into(),
            "ApplyPatches".into(),
            "{\"filename\":\"src/lib.rs\"}".into(),
        );

        let pending = app.pending_write_approval().expect("pending approval");
        assert_eq!(pending.request_id, "call-1");
        assert_eq!(pending.tool_name, "ApplyPatches");
    }

    #[test]
    fn main_pending_approval_request_ids_skip_subagent_entries() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        app.begin_subagent_write_approval(
            "subagent-1".into(),
            "sub-write".into(),
            "WriteFile".into(),
            "{}".into(),
        );
        app.begin_write_approval("main-write".into(), "WriteFile".into(), "{}".into());
        app.begin_subagent_shell_approval(
            "subagent-2".into(),
            "sub-shell".into(),
            CommandRisk::Medium,
            "explanation".into(),
            "git status".into(),
            "workspace root".into(),
            "reason".into(),
        );
        app.begin_shell_approval(
            "main-shell".into(),
            CommandRisk::Low,
            "explanation".into(),
            "pwd".into(),
            "workspace root".into(),
            "reason".into(),
        );

        assert_eq!(
            app.main_pending_write_approval_request_id(),
            Some("main-write")
        );
        assert_eq!(
            app.main_pending_shell_approval_request_id(),
            Some("main-shell")
        );
    }

    #[test]
    fn allow_all_session_updates_policy_and_clears_pending_request() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_write_approval("call-1".into(), "WriteFile".into(), "{}".into());

        let pending = app
            .resolve_write_approval(WriteApprovalDecision::AllowAllSession)
            .expect("pending approval");

        assert_eq!(pending.request_id, "call-1");
        assert_eq!(app.approval_mode(), ApprovalMode::Disabled);
        assert!(!app.has_pending_write_approval());
    }

    #[test]
    fn custom_startup_defaults_restore_on_new_session() {
        let mut app = App::with_startup(
            true,
            false,
            "gpt-5-mini",
            ReasoningEffort::Medium,
            Vec::new(),
            AccessMode::ReadWrite,
            ApprovalMode::Disabled,
        );

        app.mode = AccessMode::ReadOnly;
        app.approval_mode = ApprovalMode::Manual;
        app.tick_count = 12;
        app.reset_session();

        assert_eq!(app.mode(), AccessMode::ReadWrite);
        assert_eq!(app.approval_mode(), ApprovalMode::Disabled);
        assert_eq!(app.tick_count(), 0);
        assert_eq!(app.entries().len(), 1);
        assert!(app.shows_startup_banner());
    }

    #[test]
    fn begin_ask_user_defaults_to_first_answer_and_adds_something_else() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        app.begin_ask_user("call-1".into(), sample_ask_user_request());

        let pending = app.pending_ask_user().expect("pending ask user");
        assert_eq!(pending.request_id, "call-1");
        assert_eq!(pending.title, "Clarify implementation");
        assert_eq!(pending.active_tab, 0);
        assert_eq!(pending.questions[0].selected_index, 0);
        assert_eq!(pending.questions[0].answers.len(), 3);
        assert_eq!(
            pending.questions[0]
                .answers
                .last()
                .map(|answer| answer.label.as_str()),
            Some("Something else")
        );
        assert!(pending.questions[0].answers[0].is_recommended);
    }

    #[test]
    fn ask_user_something_else_requires_details() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_ask_user("call-1".into(), sample_ask_user_request());

        app.move_ask_user_answer_down();
        app.move_ask_user_answer_down();

        let pending = app.pending_ask_user.as_ref().expect("pending ask user");
        let question = &pending.questions[0];
        assert!(question.selected_answer().is_something_else);
        assert!(pending.detail_editing);
        assert!(!question.is_complete());
    }

    #[test]
    fn ask_user_submit_returns_structured_response_and_summary_message() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_ask_user("call-1".into(), sample_ask_user_request());

        app.move_ask_user_answer_down();
        app.move_ask_user_answer_down();
        app.paste_into_ask_user_detail("parser only");
        app.move_ask_user_tab_right();
        app.move_ask_user_tab_right();

        let (request_id, response, summary) = app
            .submit_ask_user_response()
            .expect("ask user response should submit");

        assert_eq!(request_id, "call-1");
        assert_eq!(response.questions.len(), 2);
        assert_eq!(
            response.questions[0].selected_answer.label,
            "Something else"
        );
        assert_eq!(response.questions[0].details, "parser only");
        assert!(summary.contains("Questions answered"));
        let TranscriptEntry::Message(message) = app.entries.last().expect("summary entry") else {
            panic!("expected summary message");
        };
        assert_eq!(message.speaker, Speaker::User);
        assert!(message.text.contains("Which scope?: Something else"));
        assert!(!app.has_pending_ask_user());
    }

    #[test]
    fn pending_reply_replay_seed_tracks_plain_reasoning_and_commentary() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.append_pending_stream_message("plain", MessageStyle::Plain);
        app.append_pending_stream_message("thought", MessageStyle::Thinking);
        app.push_agent_commentary("note");

        let seed = app
            .pending_reply_replay_seed()
            .expect("pending reply replay seed");
        assert_eq!(seed.plain_text, "plain");
        assert_eq!(seed.reasoning_text, "thought");
        assert_eq!(seed.commentary_messages, vec!["note"]);
    }

    #[test]
    fn advance_ask_user_moves_through_questions_before_review() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_ask_user("call-1".into(), sample_ask_user_request());

        assert!(app.advance_ask_user().is_none());
        assert_eq!(
            app.pending_ask_user().map(|pending| pending.active_tab),
            Some(1)
        );
        assert!(app.advance_ask_user().is_none());
        assert_eq!(
            app.pending_ask_user().map(|pending| pending.active_tab),
            Some(2)
        );
    }

    #[test]
    fn restoring_command_history_collapses_consecutive_duplicates() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.restore_command_history(
            vec!["alpha".into(), "alpha".into(), "beta".into(), "beta".into()],
            20,
        );

        app.apply(crate::app::actions::Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["beta"]);
        app.apply(crate::app::actions::Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["beta"]);
        assert_eq!(app.composer.cursor(), (0, 0));
        app.apply(crate::app::actions::Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["alpha"]);
    }

    #[test]
    fn shell_approval_editor_vertical_movement_tracks_multiline_pattern_bounds() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_shell_approval(
            "call-1".into(),
            CommandRisk::Low,
            "read-only inspection command with no obvious mutation".into(),
            "first\nsecond\nthird".into(),
            ".".into(),
            "review shell command".into(),
        );
        app.pending_shell_approvals
            .front_mut()
            .expect("shell approval")
            .pattern_input = TextArea::from(["first", "second", "third"]);

        app.move_shell_approval_selection(1);
        assert!(app.shell_approval_editing());
        assert!(!app.shell_approval_editor_can_move_up());
        assert!(app.shell_approval_editor_can_move_down());

        let pending = app
            .pending_shell_approvals
            .front_mut()
            .expect("shell approval");
        pending.pattern_input.move_cursor(CursorMove::Down);

        assert!(app.shell_approval_editor_can_move_up());
        assert!(app.shell_approval_editor_can_move_down());

        let pending = app
            .pending_shell_approvals
            .front_mut()
            .expect("shell approval");
        pending.pattern_input.move_cursor(CursorMove::Down);

        assert!(app.shell_approval_editor_can_move_up());
        assert!(!app.shell_approval_editor_can_move_down());
    }

    #[test]
    fn shell_approval_pattern_defaults_to_program_prefix_for_heredoc_commands() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_shell_approval(
            "call-1".into(),
            CommandRisk::Low,
            "read-only inspection command with no obvious mutation".into(),
            "python3 - <<'PY'\nprint('hello world')\nPY".into(),
            ".".into(),
            "inspect output".into(),
        );

        let pending = app.pending_shell_approvals.front().expect("shell approval");
        assert_eq!(pending.pattern_input.lines(), ["python3 *"]);
    }

    #[test]
    fn shell_approval_pattern_preserves_quoted_prefix_before_wildcard() {
        assert_eq!(
            default_shell_approval_pattern(r#"rg "foo bar" src"#),
            r#"rg "foo bar" *"#
        );
    }

    #[test]
    fn shell_approval_pattern_keeps_single_word_commands_exact() {
        assert_eq!(default_shell_approval_pattern("pwd"), "pwd");
    }
}
