use ratatui_textarea::{CursorMove, TextArea};
use rig::completion::Message as RigMessage;

const COMMANDS: [SlashCommand; 2] = [SlashCommand::NewSession, SlashCommand::Quit];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SlashCommand {
    NewSession,
    Quit,
}

impl SlashCommand {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::NewSession => "/new",
            Self::Quit => "/quit",
        }
    }

    pub fn aliases(self) -> &'static [&'static str] {
        match self {
            Self::NewSession => &["/clear"],
            Self::Quit => &["/exit"],
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::NewSession => "Start a new session",
            Self::Quit => "Exit the app",
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

#[derive(Debug)]
pub struct App {
    pub(super) mode: AccessMode,
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
    pub(super) selected_command: SlashCommand,
    pub(super) history_scroll_top: Option<usize>,
    pub(super) history_viewport_rows: usize,
    pub(super) history_total_lines: usize,
}

impl App {
    pub fn new(show_thinking: bool, show_tool_output: bool, model_name: impl Into<String>) -> Self {
        let model_name = model_name.into();
        Self {
            mode: AccessMode::ReadOnly,
            should_quit: false,
            composer: new_composer(),
            entries: vec![TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text: welcome_message(&model_name),
                style: MessageStyle::Plain,
            })],
            session_history: Vec::new(),
            pending_reply: None,
            next_reply_id: 1,
            tick_count: 0,
            show_thinking,
            show_tool_output,
            model_name,
            selected_command: SlashCommand::NewSession,
            history_scroll_top: None,
            history_viewport_rows: 1,
            history_total_lines: 0,
        }
    }

    pub fn mode(&self) -> AccessMode {
        self.mode
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

    pub fn filtered_commands(&self) -> Vec<SlashCommand> {
        self.command_query()
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
            text: welcome_message(&self.model_name),
            style: MessageStyle::Plain,
        })];
        self.session_history.clear();
        self.pending_reply = None;
        self.resume_history_follow();
        self.history_total_lines = 0;
        self.clear_composer();
    }

    pub(super) fn replace_session_history(&mut self, history: Vec<RigMessage>) {
        self.session_history = history;
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
}

fn new_composer() -> TextArea<'static> {
    new_composer_with_text("")
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

fn welcome_message(model_name: &str) -> String {
    format!(
        "Loaded Azure model `{model_name}` from config.toml. Send a message to start a one-shot response, or type / for commands."
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
        let app = App::new(true, false, "gpt-5-mini");

        assert_eq!(app.mode(), AccessMode::ReadOnly);
        assert!(!app.should_quit());
        assert!(!app.has_pending_reply());
        assert_eq!(app.entries().len(), 1);
        assert_eq!(app.model_name(), "gpt-5-mini");
        assert!(!app.show_tool_output());
    }

    #[test]
    fn composer_height_grows_with_multiple_lines() {
        let mut app = App::new(true, false, "gpt-5-mini");
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
        assert_eq!(SlashCommand::filtered("/ex"), vec![SlashCommand::Quit]);
    }

    #[test]
    fn command_palette_only_shows_for_single_line_slash_input() {
        let mut app = App::new(true, false, "gpt-5-mini");
        app.composer.insert_str("/");
        assert!(app.command_palette_visible());

        app.composer.insert_newline();
        assert!(!app.command_palette_visible());
    }

    #[test]
    fn history_status_defaults_to_live_follow() {
        let app = App::new(true, false, "gpt-5-mini");

        assert!(!app.history_is_pinned());
        assert_eq!(app.history_status_label(), "History live  PgUp/PgDn scroll");
    }

    #[test]
    fn sync_history_viewport_clamps_pinned_position() {
        let mut app = App::new(true, false, "gpt-5-mini");
        app.history_scroll_top = Some(50);

        let start = app.sync_history_viewport(20, 6);

        assert_eq!(start, 14);
        assert_eq!(app.history_scroll_top, Some(14));
    }
}
