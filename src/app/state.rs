use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, Input, TextArea};
use rig::completion::{
    Message as RigMessage,
    message::{
        AssistantContent, DocumentSourceKind, ReasoningContent, ToolResultContent, UserContent,
    },
};
use std::path::{Path, PathBuf};

use crate::{
    config::ReasoningEffort,
    model_registry,
    stats::StatsTotals,
    tools::{MutationPreview, mutation_preview, write_approval_summary},
};

const COMMANDS: [SlashCommand; 5] = [
    SlashCommand::NewSession,
    SlashCommand::Stats,
    SlashCommand::Model,
    SlashCommand::Effort,
    SlashCommand::Quit,
];
const DEFAULT_COMMAND_HISTORY_LIMIT: usize = 20;
const ESTIMATED_MESSAGE_OVERHEAD_TOKENS: u64 = 4;
const ESTIMATED_CONTENT_OVERHEAD_TOKENS: u64 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Stats,
    Model,
    Effort,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Stats => "/stats",
            Self::Model => "/model",
            Self::Effort => "/effort",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Stats => &["/status"],
            Self::Model => &["/models"],
            Self::Effort => &["/reasoning", "/thinking"],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Stats => "Show session and historical usage stats",
            Self::Model => "Select the model and reasoning effort",
            Self::Effort => "Set reasoning effort for the current model",
            Self::Quit => "Exit the app",
        }
    }

    pub fn usage(self) -> Option<&'static str> {
        match self {
            Self::Model => Some("/model"),
            Self::Effort => Some("/effort <minimal|low|medium|high|xhigh>"),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionPicker {
    Model {
        selected_index: usize,
    },
    Reasoning {
        model_name: String,
        options: Vec<ReasoningEffort>,
        selected_index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PickerSelection {
    Model(String),
    Reasoning(ReasoningEffort),
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
pub enum WriteApprovalPolicy {
    AskEveryTime,
    AllowAllSession,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingWriteApproval {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: String,
    pub summary: String,
    pub target: Option<String>,
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

#[derive(Clone, Debug)]
pub enum TranscriptEntry {
    Message(ChatMessage),
    ToolCall(ToolCall),
    ToolResult(ToolResultEntry),
}

#[derive(Debug)]
pub(super) struct PendingReply {
    pub(super) id: u64,
    pub(super) reasoning_entry_index: Option<usize>,
    pub(super) text_entry_index: Option<usize>,
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
    pub(super) mode: AccessMode,
    pub(super) write_approval_policy: WriteApprovalPolicy,
    pub(super) pending_write_approval: Option<PendingWriteApproval>,
    pub(super) should_quit: bool,
    pub(super) composer: TextArea<'static>,
    pub(super) entries: Vec<TranscriptEntry>,
    pub(super) session_history: Vec<RigMessage>,
    pub(super) pending_reply: Option<PendingReply>,
    pub(super) next_reply_id: u64,
    pub(super) tick_count: usize,
    pub(super) show_thinking: bool,
    pub(super) show_tool_output: bool,
    pub(super) model_name: String,
    pub(super) reasoning_effort: ReasoningEffort,
    pub(super) session_stats: StatsTotals,
    pub(super) selected_command: SlashCommand,
    pub(super) picker: Option<SelectionPicker>,
    pub(super) history: HistoryViewState,
    command_history: CommandRecallState,
}

impl App {
    pub fn new(
        show_thinking: bool,
        show_tool_output: bool,
        model_name: impl Into<String>,
        reasoning_effort: ReasoningEffort,
    ) -> Self {
        let model_name = model_name.into();
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            mode: AccessMode::ReadOnly,
            write_approval_policy: WriteApprovalPolicy::AskEveryTime,
            pending_write_approval: None,
            should_quit: false,
            composer: new_composer(),
            entries: vec![TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text: welcome_message(&model_name, AccessMode::ReadOnly),
                style: MessageStyle::Plain,
            })],
            session_history: Vec::new(),
            pending_reply: None,
            next_reply_id: 1,
            tick_count: 0,
            show_thinking,
            show_tool_output,
            model_name,
            reasoning_effort,
            session_stats: StatsTotals::default(),
            selected_command: SlashCommand::NewSession,
            picker: None,
            history: HistoryViewState::default(),
            command_history: CommandRecallState::default(),
        }
    }

    pub fn mode(&self) -> AccessMode {
        self.mode
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn write_approval_policy(&self) -> WriteApprovalPolicy {
        self.write_approval_policy
    }

    pub fn pending_write_approval(&self) -> Option<&PendingWriteApproval> {
        self.pending_write_approval.as_ref()
    }

    pub fn has_pending_write_approval(&self) -> bool {
        self.pending_write_approval.is_some()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn composer(&self) -> &TextArea<'static> {
        &self.composer
    }

    pub fn composer_mut(&mut self) -> &mut TextArea<'static> {
        &mut self.composer
    }

    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    pub fn session_history(&self) -> &[RigMessage] {
        &self.session_history
    }

    pub fn has_pending_reply(&self) -> bool {
        self.pending_reply.is_some()
    }

    pub fn has_visible_pending_content(&self) -> bool {
        self.pending_reply.as_ref().is_some_and(|pending| {
            pending.text_entry_index.is_some()
                || (self.show_thinking && pending.reasoning_entry_index.is_some())
        })
    }

    pub fn composer_height(&self) -> u16 {
        self.composer.lines().len().max(1) as u16 + 2
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
        estimated_history_context_tokens(&self.session_history)
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

    pub(super) fn active_reply_id(&self) -> Option<u64> {
        self.pending_reply.as_ref().map(|pending| pending.id)
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
        if reset_command_history {
            self.command_history.reset_navigation();
        }
        self.sync_command_selection();
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
            text: welcome_message(&self.model_name, self.mode),
            style: MessageStyle::Plain,
        })];
        self.session_history.clear();
        self.pending_reply = None;
        self.pending_write_approval = None;
        self.write_approval_policy = WriteApprovalPolicy::AskEveryTime;
        self.resume_history_follow();
        self.history.reset();
        self.picker = None;
        self.command_history.reset_navigation();
        self.clear_composer();
    }

    pub(super) fn replace_session_history(&mut self, history: Vec<RigMessage>) {
        self.session_history = history;
    }

    pub(crate) fn set_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.reasoning_effort = reasoning_effort;
    }

    pub(crate) fn set_session_stats(&mut self, session_stats: StatsTotals) {
        self.session_stats = session_stats;
    }

    pub(crate) fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.model_name = model_name.into();
    }

    #[cfg(test)]
    pub(crate) fn set_workspace_root(&mut self, workspace_root: PathBuf) {
        self.workspace_root = workspace_root;
    }

    pub(crate) fn cancel_pending_reply(&mut self) {
        self.pending_reply = None;
        self.pending_write_approval = None;
        self.push_error_message("Request cancelled.");
    }

    pub(super) fn begin_write_approval(
        &mut self,
        request_id: String,
        tool_name: String,
        arguments: String,
    ) {
        let preview = mutation_preview(&tool_name, &arguments, &self.workspace_root);
        let approval = PendingWriteApproval {
            request_id,
            tool_name: tool_name.clone(),
            arguments: arguments.clone(),
            summary: write_approval_summary(&tool_name, &arguments, &self.workspace_root),
            target: preview.as_ref().map(|preview| preview.target.clone()),
        };
        self.push_agent_message(format!(
            "Write approval required for `{}`.",
            approval.tool_name
        ));
        self.pending_write_approval = Some(approval);
    }

    pub(super) fn resolve_write_approval(
        &mut self,
        decision: WriteApprovalDecision,
    ) -> Option<PendingWriteApproval> {
        let pending = self.pending_write_approval.take()?;
        match decision {
            WriteApprovalDecision::AllowOnce => {
                self.push_agent_message(format!("Approved `{}` once.", pending.tool_name));
            }
            WriteApprovalDecision::AllowAllSession => {
                self.write_approval_policy = WriteApprovalPolicy::AllowAllSession;
                self.push_agent_message(format!(
                    "Approved `{}` and all future writes for this session.",
                    pending.tool_name
                ));
            }
            WriteApprovalDecision::Deny => {
                self.push_error_message(format!("Denied `{}`.", pending.tool_name));
            }
        }
        Some(pending)
    }

    pub fn push_agent_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Plain);
    }

    pub fn push_error_message(&mut self, text: impl Into<String>) {
        self.push_message(Speaker::Agent, text, MessageStyle::Error);
    }

    pub(super) fn push_agent_error(&mut self, text: impl Into<String>) {
        self.push_error_message(text);
    }

    pub(super) fn push_tool_call(&mut self, name: String, parameter: String) {
        self.entries.push(TranscriptEntry::ToolCall(ToolCall {
            preview: mutation_preview(&name, &parameter, &self.workspace_root),
            name,
            parameter,
        }));
    }

    pub(super) fn push_tool_result(&mut self, name: String, output: String) {
        self.entries
            .push(TranscriptEntry::ToolResult(ToolResultEntry {
                name,
                output,
            }));
    }

    pub(super) fn append_pending_stream_message(&mut self, delta: &str, style: MessageStyle) {
        if delta.is_empty() {
            return;
        }

        let Some(existing_index) = self.pending_reply.as_ref().and_then(|pending| match style {
            MessageStyle::Plain => pending.text_entry_index,
            MessageStyle::Thinking => pending.reasoning_entry_index,
            MessageStyle::Error => None,
        }) else {
            if self.pending_reply.is_none() || style == MessageStyle::Error {
                return;
            }
            self.push_message(Speaker::Agent, delta.to_string(), style);
            let index = self.entries.len() - 1;
            if let Some(pending) = self.pending_reply.as_mut() {
                match style {
                    MessageStyle::Plain => pending.text_entry_index = Some(index),
                    MessageStyle::Thinking => pending.reasoning_entry_index = Some(index),
                    MessageStyle::Error => {}
                }
            }
            return;
        };

        if let Some(TranscriptEntry::Message(message)) = self.entries.get_mut(existing_index) {
            message.text.push_str(delta);
        }
    }

    fn push_message(&mut self, speaker: Speaker, text: impl Into<String>, style: MessageStyle) {
        self.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker,
            text: text.into(),
            style,
        }));
    }

    pub(super) fn move_command_selection_up(&mut self) {
        self.move_command_selection(-1);
    }

    pub(super) fn move_command_selection_down(&mut self) {
        self.move_command_selection(1);
    }

    pub(crate) fn open_model_picker(&mut self) {
        let selected_index = model_registry::models()
            .iter()
            .position(|model| model.name == self.model_name)
            .unwrap_or(0);
        self.picker = Some(SelectionPicker::Model { selected_index });
    }

    pub(crate) fn open_reasoning_picker(&mut self) {
        let Some(options) = model_registry::reasoning_levels_for_model(&self.model_name) else {
            self.picker = None;
            return;
        };

        let selected_index = options
            .iter()
            .position(|level| *level == self.reasoning_effort)
            .unwrap_or(0);
        self.picker = Some(SelectionPicker::Reasoning {
            model_name: self.model_name.clone(),
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

    pub(super) fn apply_picker_selection(&mut self) -> Option<PickerSelection> {
        let picker = self.picker.take()?;
        match picker {
            SelectionPicker::Model { selected_index } => model_registry::models()
                .get(selected_index)
                .map(|model| PickerSelection::Model(model.name.to_string())),
            SelectionPicker::Reasoning {
                options,
                selected_index,
                ..
            } => options
                .get(selected_index)
                .copied()
                .map(PickerSelection::Reasoning),
        }
    }

    pub(super) fn move_composer_cursor_up(&mut self) {
        self.composer.move_cursor(CursorMove::Up);
    }

    pub(super) fn move_composer_cursor_down(&mut self) {
        self.composer.move_cursor(CursorMove::Down);
    }

    pub(super) fn clear_composer(&mut self) {
        self.set_composer_text_internal("", true);
    }

    pub(super) fn insert_composer_newline(&mut self) {
        self.command_history.reset_navigation();
        self.composer.insert_newline();
        self.sync_command_selection();
    }

    pub(super) fn apply_composer_input(&mut self, input: Input) {
        self.command_history.reset_navigation();
        self.composer.input(input);
        self.sync_command_selection();
    }

    pub(super) fn paste_into_composer(&mut self, text: &str) {
        self.command_history.reset_navigation();
        self.composer.insert_str(text);
        self.sync_command_selection();
    }

    pub(super) fn record_submitted_input(&mut self, text: &str) {
        self.command_history.record(text);
    }

    pub(super) fn should_recall_previous_input(&self) -> bool {
        self.composer.cursor().0 == 0
    }

    pub(super) fn should_recall_next_input(&self) -> bool {
        let current_row = self.composer.cursor().0;
        current_row + 1 >= self.composer.lines().len()
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

    fn move_picker_selection(&mut self, direction: isize) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };

        match picker {
            SelectionPicker::Model { selected_index } => {
                let len = model_registry::models().len();
                if len == 0 {
                    return;
                }
                *selected_index =
                    (*selected_index as isize + direction).rem_euclid(len as isize) as usize;
            }
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
        SelectionPicker::Model { .. } => model_registry::models().len(),
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
    let mut composer = if text.is_empty() {
        TextArea::default()
    } else {
        TextArea::from(text.lines())
    };
    composer.set_placeholder_text("Send a message...");
    composer
}

fn estimated_history_context_tokens(history: &[RigMessage]) -> u64 {
    history.iter().map(estimated_message_tokens).sum()
}

fn estimated_message_tokens(message: &RigMessage) -> u64 {
    match message {
        RigMessage::System { content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS + estimated_text_tokens(content)
        }
        RigMessage::User { content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS
                + content
                    .iter()
                    .map(estimated_user_content_tokens)
                    .sum::<u64>()
        }
        RigMessage::Assistant { id, content } => {
            ESTIMATED_MESSAGE_OVERHEAD_TOKENS
                + id.as_deref().map(estimated_text_tokens).unwrap_or(0)
                + content
                    .iter()
                    .map(estimated_assistant_content_tokens)
                    .sum::<u64>()
        }
    }
}

fn estimated_user_content_tokens(content: &UserContent) -> u64 {
    ESTIMATED_CONTENT_OVERHEAD_TOKENS
        + match content {
            UserContent::Text(text) => estimated_text_tokens(text.text()),
            UserContent::ToolResult(tool_result) => {
                estimated_text_tokens(&tool_result.id)
                    + tool_result
                        .call_id
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + tool_result
                        .content
                        .iter()
                        .map(estimated_tool_result_content_tokens)
                        .sum::<u64>()
            }
            UserContent::Image(image) => estimated_document_source_tokens(&image.data),
            UserContent::Audio(audio) => estimated_document_source_tokens(&audio.data),
            UserContent::Video(video) => estimated_document_source_tokens(&video.data),
            UserContent::Document(document) => estimated_document_source_tokens(&document.data),
        }
}

fn estimated_assistant_content_tokens(content: &AssistantContent) -> u64 {
    ESTIMATED_CONTENT_OVERHEAD_TOKENS
        + match content {
            AssistantContent::Text(text) => estimated_text_tokens(text.text()),
            AssistantContent::ToolCall(tool_call) => {
                estimated_text_tokens(&tool_call.id)
                    + tool_call
                        .call_id
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + estimated_text_tokens(&tool_call.function.name)
                    + estimated_json_tokens(&tool_call.function.arguments)
                    + tool_call
                        .signature
                        .as_deref()
                        .map(estimated_text_tokens)
                        .unwrap_or(0)
                    + tool_call
                        .additional_params
                        .as_ref()
                        .map(estimated_json_tokens)
                        .unwrap_or(0)
            }
            AssistantContent::Reasoning(reasoning) => {
                reasoning
                    .id
                    .as_deref()
                    .map(estimated_text_tokens)
                    .unwrap_or(0)
                    + reasoning
                        .content
                        .iter()
                        .map(estimated_reasoning_content_tokens)
                        .sum::<u64>()
            }
            AssistantContent::Image(image) => estimated_document_source_tokens(&image.data),
        }
}

fn estimated_tool_result_content_tokens(content: &ToolResultContent) -> u64 {
    match content {
        ToolResultContent::Text(text) => estimated_text_tokens(text.text()),
        ToolResultContent::Image(image) => estimated_document_source_tokens(&image.data),
    }
}

fn estimated_reasoning_content_tokens(content: &ReasoningContent) -> u64 {
    match content {
        ReasoningContent::Text { text, signature } => {
            estimated_text_tokens(text)
                + signature.as_deref().map(estimated_text_tokens).unwrap_or(0)
        }
        ReasoningContent::Encrypted(_) => 0,
        ReasoningContent::Redacted { data } => estimated_text_tokens(data),
        ReasoningContent::Summary(summary) => estimated_text_tokens(summary),
        _ => {
            debug_assert!(
                false,
                "unhandled reasoning content variant in context estimation: {content:?}"
            );
            0
        }
    }
}

fn estimated_document_source_tokens(source: &DocumentSourceKind) -> u64 {
    match source {
        DocumentSourceKind::Url(value)
        | DocumentSourceKind::Base64(value)
        | DocumentSourceKind::String(value) => estimated_text_tokens(value),
        DocumentSourceKind::Raw(value) => value.len().div_ceil(4) as u64,
        DocumentSourceKind::Unknown => 0,
        _ => {
            debug_assert!(
                false,
                "unhandled document source variant in context estimation: {source:?}"
            );
            0
        }
    }
}

fn estimated_json_tokens(value: &serde_json::Value) -> u64 {
    serde_json::to_string(value)
        .map(|json| estimated_text_tokens(&json))
        .unwrap_or(0)
}

fn estimated_text_tokens(text: &str) -> u64 {
    if text.is_empty() {
        0
    } else {
        text.chars().count().div_ceil(4) as u64
    }
}

fn slice_line(line: &str, start: usize, end: usize) -> String {
    line.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
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
        assert_eq!(
            app.write_approval_policy(),
            WriteApprovalPolicy::AskEveryTime
        );
        assert!(!app.has_pending_write_approval());
        assert!(!app.should_quit());
        assert!(!app.has_pending_reply());
        assert_eq!(app.entries().len(), 1);
        assert_eq!(app.model_name(), "gpt-5-mini");
        assert_eq!(app.reasoning_effort(), ReasoningEffort::Medium);
        assert!(!app.show_tool_output());
    }

    #[test]
    fn composer_height_grows_with_multiple_lines() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.composer.insert_str("one");
        assert_eq!(app.composer_height(), 3);

        app.composer.insert_newline();
        app.composer.insert_str("two");
        assert_eq!(app.composer_height(), 4);
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
            Some(&SelectionPicker::Model { selected_index: 1 })
        );
        assert_eq!(app.overlay_height(), 5);
    }

    #[test]
    fn reasoning_picker_uses_current_reasoning_effort_as_selection() {
        let mut app = App::new(true, false, "gpt-5.4", ReasoningEffort::High);

        app.open_reasoning_picker();

        assert_eq!(
            app.selection_picker(),
            Some(&SelectionPicker::Reasoning {
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
        app.replace_session_history(vec![RigMessage::assistant("a".repeat(16_000))]);

        assert_eq!(app.next_request_context_percent(), 1);
        assert!(app.estimated_next_request_context_tokens() >= 4_000);
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
    fn allow_all_session_updates_policy_and_clears_pending_request() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);
        app.begin_write_approval("call-1".into(), "WriteFile".into(), "{}".into());

        let pending = app
            .resolve_write_approval(WriteApprovalDecision::AllowAllSession)
            .expect("pending approval");

        assert_eq!(pending.request_id, "call-1");
        assert_eq!(
            app.write_approval_policy(),
            WriteApprovalPolicy::AllowAllSession
        );
        assert!(!app.has_pending_write_approval());
    }
}
