use ratatui_textarea::Input;
use rig::completion::Message as RigMessage;

use super::state::{
    App, ChatMessage, MessageStyle, PendingReply, PickerSelection, SlashCommand, Speaker,
    SubagentDisplayState, TranscriptEntry, WriteApprovalDecision,
};
use crate::config::ReasoningEffort;
use crate::llm::StreamEvent;
use crate::model_registry;
use crate::subagents::SubagentUiEvent;

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
    ApproveWriteOnce,
    ApproveWriteAllSession,
    DenyWrite,
    Editor(Input),
    Paste(String),
    StartHistorySelection { column: u16, row: u16 },
    UpdateHistorySelection { column: u16, row: u16 },
    FinishHistorySelection { column: u16, row: u16 },
    StreamEvent { reply_id: u64, event: StreamEvent },
    SubagentEvent(SubagentUiEvent),
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
    SetModelSelection {
        model_name: String,
    },
    SetReasoningEffort {
        reasoning_effort: ReasoningEffort,
    },
    RebuildLlm {
        access_mode: super::state::AccessMode,
    },
    ResolveWriteApproval {
        request_id: String,
        decision: WriteApprovalDecision,
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
                } else if self.cancel_picker() {
                    None
                } else {
                    None
                }
            }
            Action::ToggleMode => {
                self.mode.toggle();
                Some(Effect::RebuildLlm {
                    access_mode: self.mode(),
                })
            }
            Action::SelectPreviousCommand => {
                if self.selection_picker_visible() {
                    self.move_picker_selection_up();
                } else if self.command_palette_visible() {
                    self.move_command_selection_up();
                } else if self.should_recall_previous_input() && self.recall_previous_input() {
                } else {
                    self.move_composer_cursor_up();
                }
                None
            }
            Action::SelectNextCommand => {
                if self.selection_picker_visible() {
                    self.move_picker_selection_down();
                } else if self.command_palette_visible() {
                    self.move_command_selection_down();
                } else if self.should_recall_next_input() && self.recall_next_input() {
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
                if self.has_pending_write_approval() {
                    return None;
                }
                self.insert_composer_newline();
                None
            }
            Action::SubmitMessage => submit_message(self),
            Action::ApproveWriteOnce => resolve_write_approval(
                apply_write_approval(self, WriteApprovalDecision::AllowOnce),
                WriteApprovalDecision::AllowOnce,
            ),
            Action::ApproveWriteAllSession => resolve_write_approval(
                apply_write_approval(self, WriteApprovalDecision::AllowAllSession),
                WriteApprovalDecision::AllowAllSession,
            ),
            Action::DenyWrite => resolve_write_approval(
                apply_write_approval(self, WriteApprovalDecision::Deny),
                WriteApprovalDecision::Deny,
            ),
            Action::Editor(input) => {
                if self.has_pending_write_approval() {
                    return None;
                }
                self.apply_composer_input(input);
                None
            }
            Action::Paste(text) => {
                if self.has_pending_write_approval() {
                    return None;
                }
                self.paste_into_composer(&text);
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
            Action::SubagentEvent(event) => {
                on_subagent_event(self, event);
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
    if app.has_pending_write_approval() {
        return None;
    }

    if app.selection_picker_visible() {
        return submit_picker_selection(app);
    }

    let submitted = app.composer.lines().join("\n");
    let submitted = submitted.trim().to_owned();

    if app.command_query().is_some() {
        let command_name = app.command_name().unwrap_or_default().to_owned();
        let arguments = app.command_arguments().unwrap_or_default().to_owned();
        return submit_command(app, &command_name, &arguments);
    }

    if app.pending_reply.is_some() {
        return None;
    }

    if submitted.is_empty() {
        return None;
    }

    app.record_submitted_input(&submitted);
    app.entries.push(TranscriptEntry::Message(ChatMessage {
        speaker: Speaker::User,
        text: submitted.clone(),
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
        prompt: submitted,
        history: app.session_history().to_vec(),
    })
}

fn submit_command(app: &mut App, command_name: &str, arguments: &str) -> Option<Effect> {
    let Some(command) = app.selected_command() else {
        app.push_error_message(format!(
            "Unknown command `{command_name}`. Try /new, /stats, /model, /quit, or /effort."
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
        SlashCommand::Model => submit_model_command(app, arguments),
        SlashCommand::Quit => {
            app.should_quit = true;
            None
        }
        SlashCommand::Effort => submit_effort_command(app, arguments),
    }
}

fn submit_picker_selection(app: &mut App) -> Option<Effect> {
    match app.apply_picker_selection()? {
        PickerSelection::Model(model_name) => Some(Effect::SetModelSelection { model_name }),
        PickerSelection::Reasoning(reasoning_effort) => {
            Some(Effect::SetReasoningEffort { reasoning_effort })
        }
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

fn submit_model_command(app: &mut App, arguments: &str) -> Option<Effect> {
    if !arguments.trim().is_empty() {
        app.push_error_message("Usage: /model");
        return None;
    }

    app.clear_composer();
    app.open_model_picker();
    None
}

fn submit_effort_command(app: &mut App, arguments: &str) -> Option<Effect> {
    let value = arguments.trim();
    let supported_levels = app.supported_reasoning_levels();
    let supported = supported_levels
        .iter()
        .map(|level| level.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if value.is_empty() {
        app.push_error_message(format!(
            "Usage: /effort <{supported}>. Current effort is `{}`.",
            app.reasoning_effort().as_str()
        ));
        return None;
    }

    let Some(reasoning_effort) = ReasoningEffort::parse(value) else {
        app.push_error_message(format!(
            "Unknown reasoning effort `{value}`. Choose one of: {supported}."
        ));
        return None;
    };

    if !supported_levels.contains(&reasoning_effort) {
        if let Some(model) = app.current_model_info() {
            app.push_error_message(format!(
                "Model `{}` supports reasoning efforts: {supported}.",
                model.name
            ));
        } else {
            app.push_error_message(format!(
                "Reasoning effort `{}` is not supported. Choose one of: {supported}.",
                reasoning_effort.as_str()
            ));
        }
        return None;
    }

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

pub(crate) fn compatible_reasoning_effort(
    model_name: &str,
    current: ReasoningEffort,
) -> ReasoningEffort {
    if let Some(model) = model_registry::find_model(model_name) {
        if model.supports_reasoning(current) {
            current
        } else {
            model
                .supported_reasoning_levels
                .iter()
                .find(|level| **level == ReasoningEffort::Medium)
                .copied()
                .or_else(|| model.supported_reasoning_levels.first().copied())
                .unwrap_or(current)
        }
    } else {
        current
    }
}

fn on_stream_event(app: &mut App, reply_id: u64, event: StreamEvent) {
    if app.active_reply_id() != Some(reply_id) {
        return;
    }

    match event {
        StreamEvent::TextDelta(delta) => {
            app.append_pending_stream_message(&delta, MessageStyle::Plain)
        }
        StreamEvent::ReasoningDelta(delta) => {
            if app.show_thinking() {
                app.append_pending_stream_message(&delta, MessageStyle::Thinking);
            }
        }
        StreamEvent::ToolCall { name, arguments } => app.push_tool_call(name, arguments),
        StreamEvent::ToolResult { name, output } => app.push_tool_result(name, output),
        StreamEvent::WriteApprovalRequested {
            request_id,
            tool_name,
            arguments,
        } => {
            app.begin_write_approval(request_id, tool_name, arguments);
        }
        StreamEvent::Finished { history } => {
            if let Some(history) = history {
                app.replace_session_history(history);
            }
            app.pending_reply = None;
        }
        StreamEvent::Failed(error) => {
            app.pending_reply = None;
            app.push_agent_error(format!("Request failed: {error}"));
        }
    }
}

fn on_subagent_event(app: &mut App, event: SubagentUiEvent) {
    match event {
        SubagentUiEvent::Spawned { id, access_mode } => {
            app.upsert_subagent_status(
                id,
                SubagentDisplayState::Running,
                format!(
                    "running in {} mode",
                    access_mode.label().to_ascii_lowercase()
                ),
            );
        }
        SubagentUiEvent::Updated {
            id,
            latest_tool_name,
        } => {
            if let Some(latest_tool_name) = latest_tool_name {
                app.set_subagent_latest_tool(id, latest_tool_name);
            }
        }
        SubagentUiEvent::Completed { id } => {
            app.upsert_subagent_status(id, SubagentDisplayState::Completed, "completed".into());
        }
        SubagentUiEvent::Failed {
            id,
            error,
            log_path,
        } => {
            app.upsert_subagent_status(
                id.clone(),
                SubagentDisplayState::Failed,
                format!("failed: {error}"),
            );
            let suffix = log_path
                .as_deref()
                .map(|path| format!(" Logged request to `{path}`."))
                .unwrap_or_default();
            app.push_error_message(format!("Subagent `{id}` failed: {error}{suffix}"));
        }
        SubagentUiEvent::WriteApprovalRequested {
            id,
            request_id,
            tool_name,
            arguments,
        } => {
            app.begin_subagent_write_approval(id, request_id, tool_name, arguments);
        }
    }
}

fn apply_write_approval(app: &mut App, decision: WriteApprovalDecision) -> Option<String> {
    app.resolve_write_approval(decision)
        .map(|pending| pending.request_id)
}

fn resolve_write_approval(
    request_id: Option<String>,
    decision: WriteApprovalDecision,
) -> Option<Effect> {
    request_id.map(|request_id| Effect::ResolveWriteApproval {
        request_id,
        decision,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    fn new_app(show_thinking: bool) -> App {
        App::new(show_thinking, false, "gpt-5-mini", ReasoningEffort::Medium)
    }

    fn registry_app(show_thinking: bool) -> App {
        App::new(
            show_thinking,
            false,
            "gpt-5.4-mini",
            ReasoningEffort::Medium,
        )
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
        app.history.scroll_top = Some(3);
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
    fn up_arrow_recalls_previous_submitted_input() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer.lines(), ["second"]);
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["first"]);
    }

    #[test]
    fn down_arrow_restores_newer_history_and_original_draft() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);
        app.composer.insert_str("draft");

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["second"]);

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.composer.lines(), ["draft"]);
    }

    #[test]
    fn up_arrow_keeps_multiline_cursor_navigation_when_not_at_top() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer.insert_str("line one");
        app.composer.insert_newline();
        app.composer.insert_str("line two");

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer.lines(), ["line one", "line two"]);
        assert_eq!(app.composer.cursor().0, 0);
    }

    #[test]
    fn slash_commands_are_not_added_to_recall_history() {
        let mut app = new_app(true);
        app.composer.insert_str("/new");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::RotateSession));
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), [""]);
    }

    #[test]
    fn consecutive_duplicate_messages_are_collapsed_in_recall_history() {
        let mut app = new_app(true);

        app.composer.insert_str("boo");
        let first = app.apply(Action::SubmitMessage);
        assert!(matches!(first, Some(Effect::PromptModel { .. })));
        app.pending_reply = None;

        app.composer.insert_str("boo");
        let second = app.apply(Action::SubmitMessage);
        assert!(matches!(second, Some(Effect::PromptModel { .. })));
        app.pending_reply = None;

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["boo"]);
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer.lines(), ["boo"]);
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
    fn subagent_failure_message_includes_log_path_when_available() {
        let mut app = new_app(true);

        app.apply(Action::SubagentEvent(SubagentUiEvent::Failed {
            id: "subagent-1".into(),
            error: "boom".into(),
            log_path: Some("/tmp/subagent-1.json".into()),
        }));

        let TranscriptEntry::Message(message) = app.entries.last().expect("message entry") else {
            panic!("expected message entry");
        };
        assert!(
            message
                .text
                .contains("Logged request to `/tmp/subagent-1.json`.")
        );
    }

    #[test]
    fn subagent_update_tracks_latest_tool_name() {
        let mut app = new_app(true);
        app.apply(Action::SubagentEvent(SubagentUiEvent::Spawned {
            id: "subagent-1".into(),
            access_mode: crate::app::AccessMode::ReadOnly,
        }));

        app.apply(Action::SubagentEvent(SubagentUiEvent::Updated {
            id: "subagent-1".into(),
            latest_tool_name: Some("Grep".into()),
        }));

        let TranscriptEntry::SubagentStatus(status) = app.entries.last().expect("status entry")
        else {
            panic!("expected subagent status entry");
        };
        assert_eq!(status.latest_tool_name.as_deref(), Some("Grep"));
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
    fn model_command_opens_model_picker() {
        let mut app = registry_app(true);
        app.composer.insert_str("/model");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.selection_picker_visible());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn submitting_model_picker_returns_model_selection_effect() {
        let mut app = registry_app(true);
        app.open_model_picker();
        app.apply(Action::SelectNextCommand);

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::SetModelSelection {
                model_name: "gpt-5.4-nano".into(),
            })
        );
        assert!(!app.selection_picker_visible());
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
    fn effort_command_rejects_unsupported_value_for_registry_model() {
        let mut app = registry_app(true);
        app.composer.insert_str("/effort xhigh");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries.last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("supports reasoning efforts"));
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
    fn escape_action_closes_active_picker() {
        let mut app = registry_app(true);
        app.open_model_picker();

        let effect = app.apply(Action::CancelPendingReply);

        assert!(effect.is_none());
        assert!(!app.selection_picker_visible());
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

        assert_eq!(app.history.scroll_top, Some(20));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn page_down_clamps_at_bottom_without_resuming_follow() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);
        app.history.scroll_top = Some(24);

        app.apply(Action::ScrollHistoryPageDown);

        assert_eq!(app.history.scroll_top, Some(25));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn jump_to_bottom_resumes_live_follow() {
        let mut app = new_app(true);
        app.history.scroll_top = Some(7);

        app.apply(Action::ScrollHistoryToBottom);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn line_scroll_clamps_to_history_bounds() {
        let mut app = new_app(true);
        app.sync_history_viewport(18, 6);
        app.history.scroll_top = Some(2);

        app.apply(Action::ScrollHistoryUp { lines: 10 });
        assert_eq!(app.history.scroll_top, Some(0));

        app.apply(Action::ScrollHistoryDown { lines: 20 });
        assert_eq!(app.history.scroll_top, Some(12));
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

    #[test]
    fn compatible_reasoning_effort_preserves_supported_level() {
        assert_eq!(
            compatible_reasoning_effort("gpt-5.4", ReasoningEffort::High),
            ReasoningEffort::High
        );
    }

    #[test]
    fn compatible_reasoning_effort_downgrades_to_medium_when_needed() {
        assert_eq!(
            compatible_reasoning_effort("gpt-5.4-mini", ReasoningEffort::Minimal),
            ReasoningEffort::Medium
        );
    }
}
