use ratatui_textarea::Input;
use rig::completion::Message as RigMessage;

use super::state::{
    App, ChatMessage, MessageStyle, PendingReply, SlashCommand, Speaker, ToolCall, ToolResultEntry,
    TranscriptEntry,
};
use crate::config::ReasoningEffort;
use crate::llm::StreamEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    ClearComposerOrQuit,
    CancelPendingReply,
    ToggleMode,
    SelectPreviousCommand,
    SelectNextCommand,
    ScrollHistoryPageUp,
    ScrollHistoryPageDown,
    ScrollHistoryToTop,
    ScrollHistoryToBottom,
    ScrollHistoryUp { lines: usize },
    ScrollHistoryDown { lines: usize },
    InsertComposerNewline,
    SubmitMessage,
    Editor(Input),
    Paste(String),
    StartHistorySelection { column: u16, row: u16 },
    UpdateHistorySelection { column: u16, row: u16 },
    FinishHistorySelection { column: u16, row: u16 },
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
    ShowStats,
    RotateSession,
    SetReasoningEffort {
        reasoning_effort: ReasoningEffort,
    },
    CopyToClipboard {
        text: String,
    },
    CancelPendingReply,
}

impl App {
    pub fn apply(&mut self, action: Action) -> Option<Effect> {
        match action {
            Action::ClearComposerOrQuit => {
                if self.has_pending_reply() {
                    self.cancel_pending_reply();
                    Some(Effect::CancelPendingReply)
                } else if self.composer_has_content() {
                    self.clear_composer();
                    None
                } else {
                    self.should_quit = true;
                    None
                }
            }
            Action::CancelPendingReply => {
                if self.has_pending_reply() {
                    self.cancel_pending_reply();
                    Some(Effect::CancelPendingReply)
                } else {
                    None
                }
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
            Action::ScrollHistoryPageUp => {
                self.scroll_history_page_up();
                None
            }
            Action::ScrollHistoryPageDown => {
                self.scroll_history_page_down();
                None
            }
            Action::ScrollHistoryToTop => {
                self.scroll_history_to_top();
                None
            }
            Action::ScrollHistoryToBottom => {
                self.resume_history_follow();
                None
            }
            Action::ScrollHistoryUp { lines } => {
                self.scroll_history_up(lines);
                None
            }
            Action::ScrollHistoryDown { lines } => {
                self.scroll_history_down(lines);
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
            Action::StartHistorySelection { column, row } => {
                self.start_history_selection(column, row);
                None
            }
            Action::UpdateHistorySelection { column, row } => {
                self.update_history_selection(column, row);
                None
            }
            Action::FinishHistorySelection { column, row } => self
                .finish_history_selection(column, row)
                .map(|text| Effect::CopyToClipboard { text }),
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
    if app.command_query().is_some() {
        let command_name = app.command_name().unwrap_or_default().to_owned();
        let arguments = app.command_arguments().unwrap_or_default().to_owned();
        return submit_command(app, &command_name, &arguments);
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
    app.resume_history_follow();
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

fn submit_command(app: &mut App, command_name: &str, arguments: &str) -> Option<Effect> {
    let Some(command) = app.selected_command() else {
        app.push_error_message(format!(
            "Unknown command `{command_name}`. Try /new, /stats, /quit, or /effort."
        ));
        return None;
    };

    if !command.matches_exact(command_name) {
        app.set_composer_text(command.canonical_name());
        return None;
    }

    match command {
        SlashCommand::NewSession => {
            app.reset_session();
            Some(Effect::RotateSession)
        }
        SlashCommand::Stats => submit_stats_command(app, arguments),
        SlashCommand::Quit => {
            app.should_quit = true;
            None
        }
        SlashCommand::Effort => submit_effort_command(app, arguments),
    }
}

fn submit_stats_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /stats");
        return None;
    }

    app.clear_composer();
    Some(Effect::ShowStats)
}

fn submit_effort_command(app: &mut App, arguments: &str) -> Option<Effect> {
    let value = arguments.trim();
    if value.is_empty() {
        let supported = ReasoningEffort::supported_values().join(", ");
        app.push_error_message(format!(
            "Usage: /effort <{supported}>. Current effort is `{}`.",
            app.reasoning_effort().as_str()
        ));
        return None;
    }

    let Some(reasoning_effort) = ReasoningEffort::parse(value) else {
        let supported = ReasoningEffort::supported_values().join(", ");
        app.push_error_message(format!(
            "Unknown reasoning effort `{value}`. Choose one of: {supported}."
        ));
        return None;
    };

    if reasoning_effort == app.reasoning_effort() {
        app.clear_composer();
        app.push_agent_message(format!(
            "Reasoning effort is already set to `{}`.",
            reasoning_effort.as_str()
        ));
        return None;
    }

    app.clear_composer();
    Some(Effect::SetReasoningEffort { reasoning_effort })
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
    use ratatui::layout::Rect;

    use super::*;

    fn new_app(show_thinking: bool) -> App {
        App::new(show_thinking, false, "gpt-5-mini", ReasoningEffort::Medium)
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
    fn clear_composer_or_quit_cancels_pending_reply_instead_of_quitting() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        let effect = app.apply(Action::ClearComposerOrQuit);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(!app.should_quit);
        assert!(app.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("cancel message") else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
    }

    #[test]
    fn explicit_cancel_pending_reply_adds_cancellation_message() {
        let mut app = new_app(true);
        app.pending_reply = Some(PendingReply {
            id: 1,
            reasoning_entry_index: None,
            text_entry_index: None,
        });

        let effect = app.apply(Action::CancelPendingReply);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(app.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("cancel message") else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
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
    fn submit_message_resumes_live_history_follow() {
        let mut app = new_app(true);
        app.history_scroll_top = Some(3);
        app.composer.insert_str("hello");

        let _ = app.apply(Action::SubmitMessage);

        assert!(!app.history_is_pinned());
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
    fn effort_command_returns_effect_for_valid_value() {
        let mut app = new_app(true);
        app.composer.insert_str("/effort high");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetReasoningEffort {
                reasoning_effort: ReasoningEffort::High,
            })
        );
        assert!(!app.composer_has_content());
    }

    #[test]
    fn stats_command_returns_effect() {
        let mut app = new_app(true);
        app.composer.insert_str("/stats");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn status_alias_returns_stats_effect() {
        let mut app = new_app(true);
        app.composer.insert_str("/status");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
    }

    #[test]
    fn effort_alias_returns_effect_for_valid_value() {
        let mut app = new_app(true);
        app.composer.insert_str("/thinking xhigh");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetReasoningEffort {
                reasoning_effort: ReasoningEffort::XHigh,
            })
        );
    }

    #[test]
    fn effort_command_rejects_invalid_value() {
        let mut app = new_app(true);
        app.composer.insert_str("/effort turbo");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("Unknown reasoning effort"));
        assert!(app.composer_has_content());
    }

    #[test]
    fn effort_command_reports_noop_when_value_is_unchanged() {
        let mut app = new_app(true);
        app.composer.insert_str("/effort medium");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("message entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Plain);
        assert!(message.text.contains("already set"));
        assert!(!app.composer_has_content());
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

        assert_eq!(effect, Some(Effect::RotateSession));
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
    fn page_up_pins_history_above_live_tail() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);

        app.apply(Action::ScrollHistoryPageUp);

        assert_eq!(app.history_scroll_top, Some(20));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn page_down_clamps_at_bottom_without_resuming_follow() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);
        app.history_scroll_top = Some(24);

        app.apply(Action::ScrollHistoryPageDown);

        assert_eq!(app.history_scroll_top, Some(25));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn jump_to_bottom_resumes_live_follow() {
        let mut app = new_app(true);
        app.history_scroll_top = Some(7);

        app.apply(Action::ScrollHistoryToBottom);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn line_scroll_clamps_to_history_bounds() {
        let mut app = new_app(true);
        app.sync_history_viewport(18, 6);
        app.history_scroll_top = Some(2);

        app.apply(Action::ScrollHistoryUp { lines: 10 });
        assert_eq!(app.history_scroll_top, Some(0));

        app.apply(Action::ScrollHistoryDown { lines: 20 });
        assert_eq!(app.history_scroll_top, Some(12));
    }

    #[test]
    fn finishing_history_selection_returns_copy_effect() {
        let mut app = new_app(true);
        app.update_history_snapshot(
            Rect {
                x: 0,
                y: 0,
                width: 20,
                height: 2,
            },
            vec!["alpha".into(), "beta".into()],
        );

        assert!(
            app.apply(Action::StartHistorySelection { column: 1, row: 0 })
                .is_none()
        );
        let effect = app.apply(Action::FinishHistorySelection { column: 2, row: 1 });

        assert_eq!(
            effect,
            Some(Effect::CopyToClipboard {
                text: "lpha\nbet".into(),
            })
        );
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
