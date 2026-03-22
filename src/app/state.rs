use ratatui::layout::Rect;
use ratatui_textarea::{CursorMove, TextArea};
use rig::completion::Message as RigMessage;

use crate::config::ReasoningEffort;

const COMMANDS: [SlashCommand; 4] = [
    SlashCommand::NewSession,
    SlashCommand::Stats,
    SlashCommand::Effort,
    SlashCommand::Quit,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Stats,
    Effort,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Stats => "/stats",
            Self::Effort => "/effort",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Stats => &["/status"],
            Self::Effort => &["/reasoning", "/thinking"],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Stats => "Show session and historical usage stats",
            Self::Effort => "Set reasoning effort for the current model",
            Self::Quit => "Exit the app",
        }
    }

    pub fn usage(self) -> Option<&'static str> {
        match self {
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
            Self::ReadWrite => "Read-Write",
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

#[derive(Debug)]
pub struct App {
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
    pub(super) selected_command: SlashCommand,
    pub(super) history_scroll_top: Option<usize>,
    pub(super) history_viewport_rows: usize,
    pub(super) history_total_lines: usize,
    pub(super) history_snapshot_area: Rect,
    pub(super) history_snapshot_lines: Vec<String>,
    history_selection_anchor: Option<HistorySelectionPoint>,
    history_selection_focus: Option<HistorySelectionPoint>,
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
            selected_command: SlashCommand::NewSession,
            history_scroll_top: None,
            history_viewport_rows: 1,
            history_total_lines: 0,
            history_snapshot_area: Rect::default(),
            history_snapshot_lines: Vec::new(),
            history_selection_anchor: None,
            history_selection_focus: None,
        }
    }

    pub fn mode(&self) -> AccessMode {
        self.mode
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

    pub fn show_tool_output(&self) -> bool {
        self.show_tool_output
    }

    pub fn tick_count(&self) -> usize {
        self.tick_count
    }

    pub fn command_palette_visible(&self) -> bool {
        self.command_query().is_some()
    }

    pub fn history_is_pinned(&self) -> bool {
        self.history_scroll_top.is_some()
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

    pub(super) fn active_reply_id(&self) -> Option<u64> {
        self.pending_reply.as_ref().map(|pending| pending.id)
    }

    pub(super) fn next_reply_id(&mut self) -> u64 {
        let id = self.next_reply_id;
        self.next_reply_id = self.next_reply_id.wrapping_add(1);
        id
    }

    pub(super) fn set_composer_text(&mut self, text: &str) {
        let mut composer = new_composer_with_text(text);
        composer.move_cursor(CursorMove::End);
        self.composer = composer;
        self.sync_command_selection();
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
        self.history_total_lines = 0;
        self.clear_composer();
    }

    pub(super) fn replace_session_history(&mut self, history: Vec<RigMessage>) {
        self.session_history = history;
    }

    pub(crate) fn set_reasoning_effort(&mut self, reasoning_effort: ReasoningEffort) {
        self.reasoning_effort = reasoning_effort;
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
        let approval = PendingWriteApproval {
            request_id,
            tool_name,
            arguments,
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
        self.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::Agent,
            text: text.into(),
            style: MessageStyle::Plain,
        }));
    }

    pub fn push_error_message(&mut self, text: impl Into<String>) {
        self.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::Agent,
            text: text.into(),
            style: MessageStyle::Error,
        }));
    }

    pub(super) fn move_command_selection_up(&mut self) {
        self.move_command_selection(-1);
    }

    pub(super) fn move_command_selection_down(&mut self) {
        self.move_command_selection(1);
    }

    pub(super) fn move_composer_cursor_up(&mut self) {
        self.composer.move_cursor(CursorMove::Up);
    }

    pub(super) fn move_composer_cursor_down(&mut self) {
        self.composer.move_cursor(CursorMove::Down);
    }

    pub(super) fn clear_composer(&mut self) {
        self.composer = new_composer();
        self.sync_command_selection();
    }

    pub(crate) fn sync_history_viewport(
        &mut self,
        total_lines: usize,
        viewport_rows: usize,
    ) -> usize {
        self.history_total_lines = total_lines;
        self.history_viewport_rows = viewport_rows.max(1);
        let max_start = self.history_max_start();
        if let Some(top) = self.history_scroll_top.as_mut() {
            *top = (*top).min(max_start);
            *top
        } else {
            max_start
        }
    }

    pub(crate) fn history_total_lines(&self) -> usize {
        self.history_total_lines
    }

    pub(crate) fn history_viewport_rows(&self) -> usize {
        self.history_viewport_rows
    }

    pub(crate) fn history_scroll_position(&self) -> usize {
        self.history_current_start()
    }

    pub(crate) fn update_history_snapshot(&mut self, area: Rect, lines: Vec<String>) {
        self.history_snapshot_area = area;
        self.history_snapshot_lines = lines;
    }

    pub(super) fn scroll_history_page_up(&mut self) {
        self.scroll_history_up(self.history_page_rows());
    }

    pub(super) fn scroll_history_page_down(&mut self) {
        self.scroll_history_down(self.history_page_rows());
    }

    pub(super) fn scroll_history_up(&mut self, lines: usize) {
        let current = self.history_current_start();
        self.history_scroll_top = Some(current.saturating_sub(lines));
    }

    pub(super) fn scroll_history_down(&mut self, lines: usize) {
        let current = self.history_current_start();
        self.history_scroll_top = Some(current.saturating_add(lines).min(self.history_max_start()));
    }

    pub(super) fn scroll_history_to_top(&mut self) {
        self.history_scroll_top = Some(0);
    }

    pub(super) fn resume_history_follow(&mut self) {
        self.history_scroll_top = None;
    }

    pub(super) fn start_history_selection(&mut self, column: u16, row: u16) {
        let point = self.history_selection_point(column, row, false);
        self.history_selection_anchor = point;
        self.history_selection_focus = point;
    }

    pub(super) fn update_history_selection(&mut self, column: u16, row: u16) {
        if self.history_selection_anchor.is_none() {
            return;
        }
        self.history_selection_focus = self.history_selection_point(column, row, true);
    }

    pub(super) fn finish_history_selection(&mut self, column: u16, row: u16) -> Option<String> {
        let anchor = self.history_selection_anchor?;
        let focus = self
            .history_selection_point(column, row, true)
            .or(self.history_selection_focus)?;
        self.history_selection_anchor = None;
        self.history_selection_focus = None;
        (anchor != focus).then(|| self.selected_history_text(anchor, focus))
    }

    pub(crate) fn history_selection_span_for_row(&self, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.ordered_history_selection_points()?;
        if row < start.row || row > end.row {
            return None;
        }

        let line_width = self.history_snapshot_lines.get(row)?.chars().count().max(1);
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
        if let Some(command) = commands.first().copied() {
            if !commands.contains(&self.selected_command) {
                self.selected_command = command;
            }
        }
    }

    fn history_current_start(&self) -> usize {
        self.history_scroll_top.unwrap_or(self.history_max_start())
    }

    fn history_max_start(&self) -> usize {
        self.history_total_lines
            .saturating_sub(self.history_viewport_rows.max(1))
    }

    fn history_page_rows(&self) -> usize {
        self.history_viewport_rows.max(1)
    }

    fn history_selection_point(
        &self,
        column: u16,
        row: u16,
        clamp: bool,
    ) -> Option<HistorySelectionPoint> {
        if self.history_snapshot_lines.is_empty() || self.history_snapshot_area.width == 0 {
            return None;
        }

        let area = self.history_snapshot_area;
        let min_row = area.y;
        let max_row = area
            .y
            .saturating_add(self.history_snapshot_lines.len().saturating_sub(1) as u16);
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
        let line_width = self.history_snapshot_lines[row_index].chars().count();
        let column_index = column.saturating_sub(area.x) as usize;

        Some(HistorySelectionPoint {
            row: row_index,
            column: column_index.min(line_width.saturating_sub(1)),
        })
    }

    fn ordered_history_selection_points(
        &self,
    ) -> Option<(HistorySelectionPoint, HistorySelectionPoint)> {
        let anchor = self.history_selection_anchor?;
        let focus = self.history_selection_focus?;
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

    fn selected_history_text(
        &self,
        anchor: HistorySelectionPoint,
        focus: HistorySelectionPoint,
    ) -> String {
        let (start, end) = if (anchor.row, anchor.column) <= (focus.row, focus.column) {
            (anchor, focus)
        } else {
            (focus, anchor)
        };

        let mut lines = Vec::new();
        for row in start.row..=end.row {
            let line = &self.history_snapshot_lines[row];
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

fn new_composer() -> TextArea<'static> {
    new_composer_with_text("")
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
        assert_eq!(mode.label(), "Read-Write");

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
        app.history_scroll_top = Some(50);

        let start = app.sync_history_viewport(20, 6);

        assert_eq!(start, 14);
        assert_eq!(app.history_scroll_top, Some(14));
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
    fn beginning_write_approval_tracks_pending_request() {
        let mut app = App::new(true, false, "gpt-5-mini", ReasoningEffort::Medium);

        app.begin_write_approval(
            "call-1".into(),
            "ApplyPatch".into(),
            "{\"filename\":\"src/lib.rs\"}".into(),
        );

        let pending = app.pending_write_approval().expect("pending approval");
        assert_eq!(pending.request_id, "call-1");
        assert_eq!(pending.tool_name, "ApplyPatch");
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
