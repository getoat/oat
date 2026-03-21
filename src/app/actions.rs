use ratatui_textarea::Input;
use rig::completion::Message as RigMessage;

use super::state::{
    App, ChatMessage, MessageStyle, PendingReply, SlashCommand, Speaker, ToolCall, ToolResultEntry,
    TranscriptEntry,
};
use crate::llm::StreamEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ClearComposerOrQuit,
    ToggleMode,
    SelectPreviousCommand,
    SelectNextCommand,
    InsertComposerNewline,
    SubmitMessage,
    Editor(Input),
    Paste(String),
    StreamEvent { reply_id: u64, event: StreamEvent },
    Tick,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    PromptModel {
        reply_id: u64,
        prompt: String,
        history: Vec<RigMessage>,
    },
}

impl App {
    pub fn apply(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::ClearComposerOrQuit => {
                if self.composer_has_content() {
                    self.clear_composer();
                } else {
                    self.should_quit = true;
                }
                None
            }
            Action::ToggleMode => {
                self.mode.toggle();
                None
            }
            Action::SelectPreviousCommand => {
                if self.command_palette_visible() {
                    self.move_command_selection_up();
                } else {
                    self.move_composer_cursor_up();
                }
                None
            }
            Action::SelectNextCommand => {
                if self.command_palette_visible() {
                    self.move_command_selection_down();
                } else {
                    self.move_composer_cursor_down();
                }
                None
            }
            Action::InsertComposerNewline => {
                self.composer.insert_newline();
                None
            }
            Action::SubmitMessage => submit_message(self),
            Action::Editor(input) => {
                self.composer.input(input);
                self.sync_command_selection();
                None
            }
            Action::Paste(text) => {
                self.composer.insert_str(text);
                self.sync_command_selection();
                None
            }
            Action::StreamEvent { reply_id, event } => {
                on_stream_event(self, reply_id, event);
                None
            }
            Action::Tick => {
                self.tick_count = self.tick_count.wrapping_add(1);
                None
            }
        }
    }
}

fn submit_message(app: &mut App) -> Option<Effect> {
    if let Some(query) = app.command_query().map(str::to_owned) {
        return submit_command(app, &query);
    }

    if app.pending_reply.is_some() {
        return None;
    }

    let prompt = app.composer.lines().join("\n");
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return None;
    }

    app.entries.push(TranscriptEntry::Message(ChatMessage {
        speaker: Speaker::User,
        text: prompt.clone(),
        style: MessageStyle::Plain,
    }));
    app.clear_composer();
    let reply_id = app.next_reply_id();
    app.pending_reply = Some(PendingReply {
        id: reply_id,
        reasoning_entry_index: None,
        text_entry_index: None,
    });

    Some(Effect::PromptModel {
        reply_id,
        prompt,
        history: app.session_history().to_vec(),
    })
}

fn submit_command(app: &mut App, query: &str) -> Option<Effect> {
    let Some(command) = app.selected_command() else {
        app.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::Agent,
            text: format!("Unknown command `{query}`. Try /new or /quit."),
            style: MessageStyle::Error,
        }));
        return None;
    };

    if !command.matches_exact(query) {
        app.set_composer_text(command.canonical_name());
        return None;
    }

    match command {
        SlashCommand::NewSession => {
            app.reset_session();
            None
        }
        SlashCommand::Quit => {
            app.should_quit = true;
            None
        }
    }
}

fn on_stream_event(app: &mut App, reply_id: u64, event: StreamEvent) {
    if app.active_reply_id() != Some(reply_id) {
        return;
    }

    match event {
        StreamEvent::TextDelta(delta) => append_stream_text(app, &delta),
        StreamEvent::ReasoningDelta(delta) => append_stream_reasoning(app, &delta),
        StreamEvent::ToolCall { name, arguments } => {
            app.entries.push(TranscriptEntry::ToolCall(ToolCall {
                name,
                parameter: arguments,
            }));
        }
        StreamEvent::ToolResult { name, output } => {
            app.entries
                .push(TranscriptEntry::ToolResult(ToolResultEntry {
                    name,
                    output,
                }));
        }
        StreamEvent::Finished { history } => {
            if let Some(history) = history {
                app.replace_session_history(history);
            }
            app.pending_reply = None;
        }
        StreamEvent::Failed(error) => {
            app.pending_reply = None;
            app.entries.push(TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text: format!("Request failed: {error}"),
                style: MessageStyle::Error,
            }));
        }
    }
}

fn append_stream_text(app: &mut App, delta: &str) {
    if delta.is_empty() {
        return;
    }

    let Some(pending) = app.pending_reply.as_mut() else {
        return;
    };

    if let Some(index) = pending.text_entry_index
        && let Some(TranscriptEntry::Message(message)) = app.entries.get_mut(index)
    {
        message.text.push_str(delta);
        return;
    }

    app.entries.push(TranscriptEntry::Message(ChatMessage {
        speaker: Speaker::Agent,
        text: delta.to_string(),
        style: MessageStyle::Plain,
    }));
    pending.text_entry_index = Some(app.entries.len() - 1);
}

fn append_stream_reasoning(app: &mut App, delta: &str) {
    if delta.is_empty() || !app.show_thinking {
        return;
    }

    let Some(pending) = app.pending_reply.as_mut() else {
        return;
    };

    if let Some(index) = pending.reasoning_entry_index
        && let Some(TranscriptEntry::Message(message)) = app.entries.get_mut(index)
    {
        message.text.push_str(delta);
        return;
    }

    app.entries.push(TranscriptEntry::Message(ChatMessage {
        speaker: Speaker::Agent,
        text: delta.to_string(),
        style: MessageStyle::Thinking,
    }));
    pending.reasoning_entry_index = Some(app.entries.len() - 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_app(show_thinking: bool) -> App {
        App::new(show_thinking, false, "gpt-5-mini")
    }

    #[test]
    fn clear_composer_or_quit_quits_when_composer_is_empty() {
        let mut app = new_app(true);

        app.apply(Action::ClearComposerOrQuit);

        assert!(app.should_quit);
    }

    #[test]
    fn clear_composer_or_quit_clears_composer_before_quitting() {
        let mut app = new_app(true);
        app.composer.insert_str("draft");

        app.apply(Action::ClearComposerOrQuit);

        assert!(!app.should_quit);
        assert!(!app.composer_has_content());
    }

    #[test]
    fn submit_message_ignores_empty_input() {
        let mut app = new_app(true);

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(app.entries.len(), 1);
        assert!(app.pending_reply.is_none());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_records_prompt_and_returns_effect() {
        let mut app = new_app(true);
        app.composer.insert_str("hello\nworld");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "hello\nworld".into(),
                history: Vec::new(),
            })
        );
        assert_eq!(app.entries.len(), 2);
        match &app.entries[1] {
            TranscriptEntry::Message(message) => {
                assert_eq!(message.speaker, Speaker::User);
                assert_eq!(message.text, "hello\nworld");
            }
            entry => panic!("expected user message, got {entry:?}"),
        }
        assert!(app.pending_reply.is_some());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn submit_message_does_nothing_while_reply_is_pending() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 5,
            reasoning_entry_index: None,
            text_entry_index: None,
        });
        app.composer.insert_str("new prompt");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(app.entries.len(), 1);
        assert!(app.composer_has_content());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_clones_canonical_history_into_effect() {
        let mut app = new_app(true);
        app.replace_session_history(vec![RigMessage::assistant("previous")]);
        app.composer.insert_str("next");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "next".into(),
                history: vec![RigMessage::assistant("previous")],
            })
        );
    }

    #[test]
    fn stream_text_creates_and_updates_agent_message() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Hello".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta(", world".into()),
        });

        match &app.entries[1] {
            TranscriptEntry::Message(message) => {
                assert_eq!(message.style, MessageStyle::Plain);
                assert_eq!(message.text, "Hello, world");
            }
            entry => panic!("expected agent message, got {entry:?}"),
        }
    }

    #[test]
    fn stream_reasoning_is_hidden_when_config_disables_it() {
        let mut app = new_app(false);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ReasoningDelta("thinking".into()),
        });

        assert_eq!(app.entries.len(), 1);
    }

    #[test]
    fn stream_tool_call_adds_transcript_entry() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src","recursive":true}"#.into(),
            },
        });

        match &app.entries[1] {
            TranscriptEntry::ToolCall(tool_call) => {
                assert_eq!(tool_call.name, "List");
                assert_eq!(tool_call.parameter, r#"{"dir":"src","recursive":true}"#);
            }
            entry => panic!("expected tool call, got {entry:?}"),
        }
    }

    #[test]
    fn stream_tool_result_adds_transcript_entry() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolResult {
                name: "ReadFile".into(),
                output: "1 | hello".into(),
            },
        });

        match &app.entries[1] {
            TranscriptEntry::ToolResult(tool_result) => {
                assert_eq!(tool_result.name, "ReadFile");
                assert_eq!(tool_result.output, "1 | hello");
            }
            entry => panic!("expected tool result, got {entry:?}"),
        }
    }

    #[test]
    fn stream_failure_appends_error_message() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Failed("boom".into()),
        });

        assert!(app.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("boom"));
    }

    #[test]
    fn partial_command_completes_before_execution() {
        let mut app = new_app(true);
        app.composer.insert_str("/n");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert_eq!(app.composer.lines(), ["/new"]);
        assert_eq!(app.entries.len(), 1);
    }

    #[test]
    fn clear_alias_starts_new_session() {
        let mut app = new_app(true);
        app.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::User,
            text: "old".into(),
            style: MessageStyle::Plain,
        }));
        app.pending_reply = Some(PendingReply {
            id: 8,
            reasoning_entry_index: None,
            text_entry_index: None,
        });
        app.composer.insert_str("/clear");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert_eq!(app.entries.len(), 1);
        assert!(app.pending_reply.is_none());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn command_selection_wraps() {
        let mut app = new_app(true);
        app.composer.insert_str("/");
        app.sync_command_selection();

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.selected_command(), Some(SlashCommand::Quit));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.selected_command(), Some(SlashCommand::NewSession));
    }

    #[test]
    fn quit_command_sets_should_quit() {
        let mut app = new_app(true);
        app.composer.insert_str("/quit");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.should_quit());
    }

    #[test]
    fn stale_stream_events_are_ignored_after_new_session() {
        let mut app = new_app(true);
        app.replace_session_history(vec![RigMessage::assistant("previous")]);
        app.pending_reply = Some(PendingReply {
            id: 11,
            reasoning_entry_index: None,
            text_entry_index: None,
        });
        app.entries.push(TranscriptEntry::Message(ChatMessage {
            speaker: Speaker::User,
            text: "hello".into(),
            style: MessageStyle::Plain,
        }));
        app.composer.insert_str("/new");
        app.sync_command_selection();

        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 11,
            event: StreamEvent::TextDelta("stale".into()),
        });

        assert_eq!(app.entries.len(), 1);
        assert!(app.session_history().is_empty());
    }

    #[test]
    fn finished_stream_replaces_canonical_history() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 2,
            reasoning_entry_index: None,
            text_entry_index: None,
        });
        app.replace_session_history(vec![RigMessage::assistant("old")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Finished {
                history: Some(vec![
                    RigMessage::user("hello"),
                    RigMessage::assistant("world"),
                ]),
            },
        });

        assert!(app.pending_reply.is_none());
        assert_eq!(
            app.session_history(),
            &[RigMessage::user("hello"), RigMessage::assistant("world")]
        );
    }

    #[test]
    fn failed_stream_keeps_previous_canonical_history() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 2,
            reasoning_entry_index: None,
            text_entry_index: None,
        });
        app.replace_session_history(vec![RigMessage::assistant("stable")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Failed("boom".into()),
        });

        assert_eq!(app.session_history(), &[RigMessage::assistant("stable")]);
    }
}
