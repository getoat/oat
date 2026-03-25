use super::events::{
    apply_write_approval, on_stream_event, on_subagent_event, resolve_write_approval,
};
use super::submit::{submit_message, submit_plan_acceptance};
use super::{Action, Effect, SessionState, WriteApprovalDecision};
use crate::app::{ReducerContext, UiState};

pub(crate) fn apply(
    session: &mut SessionState,
    ui: &mut UiState,
    action: Action,
) -> Option<Effect> {
    let mut ctx = ReducerContext::new(session, ui);
    match action {
        Action::ClearComposerOrQuit => {
            if ctx.has_pending_reply() {
                ctx.cancel_pending_reply();
                Some(Effect::CancelPendingReply)
            } else if ctx.composer_has_content() {
                ctx.clear_composer();
                None
            } else {
                ctx.set_should_quit();
                None
            }
        }
        Action::CancelPendingReply => {
            if ctx.cancel_shell_approval_editing() {
                None
            } else if ctx.has_pending_reply() {
                ctx.cancel_pending_reply();
                Some(Effect::CancelPendingReply)
            } else if ctx.cancel_picker() {
                None
            } else if ctx.cancel_planning_draft_mode() {
                ctx.push_agent_message("Planning draft cancelled.");
                None
            } else {
                None
            }
        }
        Action::ToggleMode => {
            ctx.session.mode.toggle();
            Some(Effect::RebuildLlm {
                access_mode: ctx.mode(),
            })
        }
        Action::SelectPreviousCommand => {
            if ctx.has_pending_shell_approval() {
                ctx.move_shell_approval_selection(-1);
            } else if ctx.has_pending_ask_user() {
                ctx.move_ask_user_answer_up();
            } else if ctx.plan_review_selection_active() {
                ctx.move_plan_review_selection(-1);
            } else if ctx.selection_picker_visible() {
                ctx.move_picker_selection_up();
            } else if ctx.command_palette_visible() {
                ctx.move_command_selection_up();
            } else if ctx.should_recall_previous_input() && ctx.recall_previous_input() {
            } else {
                ctx.move_composer_cursor_up();
            }
            None
        }
        Action::SelectNextCommand => {
            if ctx.has_pending_shell_approval() {
                ctx.move_shell_approval_selection(1);
            } else if ctx.has_pending_ask_user() {
                ctx.move_ask_user_answer_down();
            } else if ctx.plan_review_selection_active() {
                ctx.move_plan_review_selection(1);
            } else if ctx.selection_picker_visible() {
                ctx.move_picker_selection_down();
            } else if ctx.command_palette_visible() {
                ctx.move_command_selection_down();
            } else if ctx.should_recall_next_input() && ctx.recall_next_input() {
            } else {
                ctx.move_composer_cursor_down();
            }
            None
        }
        Action::ScrollHistoryPageUp => {
            ctx.scroll_history_page_up();
            None
        }
        Action::ScrollHistoryPageDown => {
            ctx.scroll_history_page_down();
            None
        }
        Action::ScrollHistoryToTop => {
            ctx.scroll_history_to_top();
            None
        }
        Action::ScrollHistoryToBottom => {
            ctx.resume_history_follow();
            None
        }
        Action::ScrollHistoryUp { lines } => {
            ctx.scroll_history_up(lines);
            None
        }
        Action::ScrollHistoryDown { lines } => {
            ctx.scroll_history_down(lines);
            None
        }
        Action::InsertComposerNewline => {
            if ctx.has_pending_write_approval()
                || ctx.has_pending_shell_approval()
                || ctx.plan_review_selection_active()
            {
                return None;
            }
            ctx.insert_composer_newline();
            None
        }
        Action::SubmitMessage => submit_message(&mut ctx),
        Action::TogglePickerSelection => ctx
            .toggle_picker_selection()
            .map(|planning_agents| Effect::SetPlanningAgents { planning_agents }),
        Action::PickerTabLeft => {
            ctx.move_picker_tab_left();
            None
        }
        Action::PickerTabRight => {
            ctx.move_picker_tab_right();
            None
        }
        Action::AskUserTabLeft => {
            ctx.move_ask_user_tab_left();
            None
        }
        Action::AskUserTabRight => {
            ctx.move_ask_user_tab_right();
            None
        }
        Action::AskUserToggleDetailEditor => {
            ctx.toggle_ask_user_detail_editing();
            None
        }
        Action::ShellApprovalToggleDetailEditor => {
            ctx.toggle_shell_approval_detail_editing();
            None
        }
        Action::ApproveWriteOnce => resolve_write_approval(
            apply_write_approval(&mut ctx, WriteApprovalDecision::AllowOnce),
            WriteApprovalDecision::AllowOnce,
        ),
        Action::ApproveWriteAllSession => resolve_write_approval(
            apply_write_approval(&mut ctx, WriteApprovalDecision::AllowAllSession),
            WriteApprovalDecision::AllowAllSession,
        ),
        Action::DenyWrite => resolve_write_approval(
            apply_write_approval(&mut ctx, WriteApprovalDecision::Deny),
            WriteApprovalDecision::Deny,
        ),
        Action::AcceptPlanAndImplement => submit_plan_acceptance(&mut ctx),
        Action::SuggestPlanChanges => {
            if ctx.plan_review_selection_active() {
                ctx.begin_plan_review_feedback();
            }
            None
        }
        Action::Editor(input) => {
            if ctx.has_pending_write_approval() || ctx.plan_review_selection_active() {
                return None;
            }
            if ctx.shell_approval_editing() {
                ctx.apply_shell_approval_input(input);
                return None;
            }
            if ctx.has_pending_shell_approval() {
                return None;
            }
            if ctx.ask_user_detail_editing() {
                ctx.apply_ask_user_input(input);
                return None;
            }
            if ctx.has_pending_ask_user() {
                return None;
            }
            ctx.apply_composer_input(input);
            None
        }
        Action::Paste(text) => {
            if ctx.has_pending_write_approval() || ctx.plan_review_selection_active() {
                return None;
            }
            if ctx.shell_approval_editing() {
                ctx.paste_into_shell_approval_detail(&text);
                return None;
            }
            if ctx.has_pending_shell_approval() {
                return None;
            }
            if ctx.ask_user_detail_editing() {
                ctx.paste_into_ask_user_detail(&text);
                return None;
            }
            if ctx.has_pending_ask_user() {
                return None;
            }
            ctx.paste_into_composer(&text);
            None
        }
        Action::StartHistorySelection { column, row } => {
            ctx.start_history_selection(column, row);
            None
        }
        Action::UpdateHistorySelection { column, row } => {
            ctx.update_history_selection(column, row);
            None
        }
        Action::FinishHistorySelection { column, row } => ctx
            .finish_history_selection(column, row)
            .map(|text| Effect::CopyToClipboard { text }),
        Action::StreamEvent { reply_id, event } => on_stream_event(&mut ctx, reply_id, event),
        Action::SubagentEvent(event) => {
            on_subagent_event(&mut ctx, event);
            None
        }
        Action::Tick => {
            ctx.session.tick_count = ctx.session.tick_count.wrapping_add(1);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{
            App, ChatMessage, MessageStyle, PendingReply, PendingReplyKind, SessionHistoryMessage,
            SlashCommand, Speaker, StreamEvent, SubagentDisplayState, TranscriptEntry,
            compatible_reasoning_effort,
        },
        ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest},
        config::ReasoningEffort,
        features::planning::{PlanningAgentConfig, PlanningStage, planning_conversation_prompt},
        subagents::{SubagentActivityKind, SubagentUiEvent},
    };

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

    fn ask_user_request() -> AskUserRequest {
        AskUserRequest {
            title: Some("Clarify implementation".into()),
            questions: vec![AskUserQuestion {
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
            }],
        }
    }

    #[test]
    fn clear_composer_or_quit_quits_when_composer_is_empty() {
        let mut app = new_app(true);

        app.apply(Action::ClearComposerOrQuit);

        assert!(app.session.should_quit);
    }

    #[test]
    fn clear_composer_or_quit_clears_composer_before_quitting() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("draft");

        app.apply(Action::ClearComposerOrQuit);

        assert!(!app.session.should_quit);
        assert!(!app.composer_has_content());
    }

    #[test]
    fn clear_composer_or_quit_cancels_pending_reply_instead_of_quitting() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        let effect = app.apply(Action::ClearComposerOrQuit);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(!app.session.should_quit);
        assert!(app.session.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("cancel message")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
    }

    #[test]
    fn explicit_cancel_pending_reply_adds_cancellation_message() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        let effect = app.apply(Action::CancelPendingReply);

        assert_eq!(effect, Some(Effect::CancelPendingReply));
        assert!(app.session.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("cancel message")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert_eq!(message.text, "Request cancelled.");
    }

    #[test]
    fn submit_message_ignores_empty_input() {
        let mut app = new_app(true);

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(app.entries().len(), 1);
        assert!(app.session.pending_reply.is_none());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_records_prompt_and_returns_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("hello\nworld");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "hello\nworld".into(),
                history: Vec::new(),
                history_model_name: None,
            })
        );
        assert_eq!(app.entries().len(), 2);
        match &app.entries()[1] {
            TranscriptEntry::Message(message) => {
                assert_eq!(message.speaker, Speaker::User);
                assert_eq!(message.text, "hello\nworld");
            }
            entry => panic!("expected user message, got {entry:?}"),
        }
        assert!(app.session.pending_reply.is_some());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn submit_message_resumes_live_history_follow() {
        let mut app = new_app(true);
        app.ui.history.scroll_top = Some(3);
        app.composer_mut().insert_str("hello");

        let _ = app.apply(Action::SubmitMessage);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn submit_message_does_nothing_while_reply_is_pending() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(5, PendingReplyKind::Normal));
        app.composer_mut().insert_str("new prompt");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(app.entries().len(), 1);
        assert!(app.composer_has_content());
        assert!(effect.is_none());
    }

    #[test]
    fn submit_message_clones_canonical_history_into_effect() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.composer_mut().insert_str("next");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "next".into(),
                history: vec![SessionHistoryMessage::assistant("previous")],
                history_model_name: None,
            })
        );
    }

    #[test]
    fn up_arrow_recalls_previous_submitted_input() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);
        assert_eq!(app.composer().cursor(), (0, 0));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["first"]);
    }

    #[test]
    fn down_arrow_restores_newer_history_and_original_draft() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["first".into(), "second".into()], 20);
        app.composer_mut().insert_str("draft");

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["draft"]);
        assert_eq!(app.composer().cursor(), (0, 0));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["second"]);

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.composer().lines(), ["draft"]);
    }

    #[test]
    fn up_arrow_keeps_multiline_cursor_navigation_when_not_at_top() {
        let mut app = new_app(true);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("line one");
        app.composer_mut().insert_newline();
        app.composer_mut().insert_str("line two");

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer().lines(), ["line one", "line two"]);
        assert_eq!(app.composer().cursor().0, 0);
    }

    #[test]
    fn up_arrow_on_first_visual_row_moves_to_visual_start_before_history() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("alpha beta");
        app.set_composer_cursor(0, 3);

        app.apply(Action::SelectPreviousCommand);

        assert_eq!(app.composer().lines(), ["alpha beta"]);
        assert_eq!(app.composer().cursor(), (0, 0));
    }

    #[test]
    fn down_arrow_on_last_visual_row_moves_to_visual_end_before_history() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.restore_command_history(vec!["previous".into()], 20);
        app.composer_mut().insert_str("alpha beta");
        app.set_composer_cursor(0, 7);

        app.apply(Action::SelectNextCommand);

        assert_eq!(app.composer().lines(), ["alpha beta"]);
        assert_eq!(app.composer().cursor(), (0, 10));
    }

    #[test]
    fn up_and_down_navigate_wrapped_visual_rows() {
        let mut app = new_app(true);
        app.set_composer_wrap_width(6);
        app.composer_mut().insert_str("alpha beta gamma");

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().cursor(), (0, 10));

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().cursor(), (0, 5));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.composer().cursor(), (0, 10));
    }

    #[test]
    fn slash_commands_are_not_added_to_recall_history() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/new");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::RotateSession));
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), [""]);
    }

    #[test]
    fn consecutive_duplicate_messages_are_collapsed_in_recall_history() {
        let mut app = new_app(true);

        app.composer_mut().insert_str("boo");
        let first = app.apply(Action::SubmitMessage);
        assert!(matches!(first, Some(Effect::PromptModel { .. })));
        app.session.pending_reply = None;

        app.composer_mut().insert_str("boo");
        let second = app.apply(Action::SubmitMessage);
        assert!(matches!(second, Some(Effect::PromptModel { .. })));
        app.session.pending_reply = None;

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["boo"]);
        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.composer().lines(), ["boo"]);
    }

    #[test]
    fn stream_text_creates_and_updates_agent_message() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Hello".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta(", world".into()),
        });

        match &app.entries()[1] {
            TranscriptEntry::Message(message) => {
                assert_eq!(message.style, MessageStyle::Plain);
                assert_eq!(message.text, "Hello, world");
            }
            entry => panic!("expected agent message, got {entry:?}"),
        }
    }

    #[test]
    fn whitespace_only_text_delta_stays_pending_without_visible_content() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("\n  ".into()),
        });

        assert_eq!(app.entries().len(), 1);
        assert!(!app.has_visible_pending_content());
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .expect("pending reply")
                .plain_text,
            "\n  "
        );
    }

    #[test]
    fn proposed_plan_wrapper_prefix_stays_pending_until_visible_text_arrives() {
        let mut app = registry_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("<proposed_plan>\n".into()),
        });

        assert_eq!(app.entries().len(), 1);
        assert!(!app.has_visible_pending_content());

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("# Test Plan\n".into()),
        });

        assert!(app.has_visible_pending_content());
        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain
                    && message.text == "<proposed_plan>\n# Test Plan\n"
        ));
    }

    #[test]
    fn planning_ready_wrapper_prefix_stays_pending_until_visible_text_arrives() {
        let mut app = registry_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("<planning_ready>\n".into()),
        });

        assert_eq!(app.entries().len(), 1);
        assert!(!app.has_visible_pending_content());

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("## Summary\n".into()),
        });

        assert!(app.has_visible_pending_content());
        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain
                    && message.text == "<planning_ready>\n## Summary\n"
        ));
    }

    #[test]
    fn stream_reasoning_is_hidden_when_config_disables_it() {
        let mut app = new_app(false);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ReasoningDelta("thinking".into()),
        });

        assert_eq!(app.entries().len(), 1);
    }

    #[test]
    fn stream_commentary_adds_agent_commentary_entry() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Commentary("Checking the current failure mode.".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Commentary
                    && message.text == "Checking the current failure mode."
        ));
    }

    #[test]
    fn stream_tool_call_adds_transcript_entry() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src","recursive":true}"#.into(),
            },
        });

        match &app.entries()[1] {
            TranscriptEntry::ToolCall(tool_call) => {
                assert_eq!(tool_call.name, "List");
                assert_eq!(tool_call.parameter, r#"{"dir":"src","recursive":true}"#);
            }
            entry => panic!("expected tool call, got {entry:?}"),
        }
    }

    #[test]
    fn stream_text_after_tool_call_starts_new_message_entry() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Before tool".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src"}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("After tool".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message) if message.text == "Before tool"
        ));
        assert!(matches!(
            &app.entries()[2],
            TranscriptEntry::ToolCall(tool_call)
                if tool_call.name == "List" && tool_call.parameter == r#"{"dir":"src"}"#
        ));
        assert!(matches!(
            &app.entries()[3],
            TranscriptEntry::Message(message) if message.text == "After tool"
        ));
    }

    #[test]
    fn stream_tool_result_adds_transcript_entry() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolResult {
                name: "ReadFile".into(),
                output: "1 | hello".into(),
            },
        });

        match &app.entries()[1] {
            TranscriptEntry::ToolResult(tool_result) => {
                assert_eq!(tool_result.name, "ReadFile");
                assert_eq!(tool_result.output, "1 | hello");
            }
            entry => panic!("expected tool result, got {entry:?}"),
        }
    }

    #[test]
    fn commentary_between_text_segments_stays_separate_from_final_reply_text() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Before".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Commentary("Inspecting the failing path.".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("After".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain && message.text == "Before"
        ));
        assert!(matches!(
            &app.entries()[2],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Commentary
                    && message.text == "Inspecting the failing path."
        ));
        assert!(matches!(
            &app.entries()[3],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain && message.text == "After"
        ));
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .expect("pending reply")
                .plain_text,
            "BeforeAfter"
        );
    }

    #[test]
    fn stream_text_after_tool_result_starts_new_message_entry() {
        let mut app = App::new(true, true, "gpt-5-mini", ReasoningEffort::Medium);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("Before result".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ToolResult {
                name: "ReadFile".into(),
                output: "1 | line".into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("After result".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message) if message.text == "Before result"
        ));
        assert!(matches!(
            &app.entries()[2],
            TranscriptEntry::ToolResult(tool_result)
                if tool_result.name == "ReadFile" && tool_result.output == "1 | line"
        ));
        assert!(matches!(
            &app.entries()[3],
            TranscriptEntry::Message(message) if message.text == "After result"
        ));
    }

    #[test]
    fn reasoning_followed_by_text_creates_distinct_entries_in_order() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ReasoningDelta("thinking".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("answer".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Thinking && message.text == "thinking"
        ));
        assert!(matches!(
            &app.entries()[2],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain && message.text == "answer"
        ));
    }

    #[test]
    fn text_reasoning_text_creates_three_ordered_segments() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("first".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ReasoningDelta("thought".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("second".into()),
        });

        assert!(matches!(
            &app.entries()[1],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain && message.text == "first"
        ));
        assert!(matches!(
            &app.entries()[2],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Thinking && message.text == "thought"
        ));
        assert!(matches!(
            &app.entries()[3],
            TranscriptEntry::Message(message)
                if message.style == MessageStyle::Plain && message.text == "second"
        ));
    }

    #[test]
    fn ask_user_stream_event_starts_pending_interaction() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::AskUserRequested {
                request_id: "call-1".into(),
                request: ask_user_request(),
            },
        });

        assert!(app.has_pending_ask_user());
        assert_eq!(
            app.pending_ask_user()
                .map(|pending| pending.request_id.as_str()),
            Some("call-1")
        );
    }

    #[test]
    fn ask_user_review_submission_returns_resolve_effect() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.begin_ask_user("call-1".into(), ask_user_request());

        let first = app.apply(Action::SubmitMessage);
        assert!(first.is_none());
        assert_eq!(app.ask_user_ui().map(|ui| ui.active_tab), Some(1));

        let effect = app.apply(Action::SubmitMessage);

        match effect {
            Some(Effect::ResolveAskUser {
                request_id,
                response,
            }) => {
                assert_eq!(request_id, "call-1");
                assert_eq!(response.questions.len(), 1);
                assert_eq!(response.questions[0].selected_answer.label, "Narrow");
            }
            other => panic!("expected ResolveAskUser effect, got {other:?}"),
        }
        assert!(!app.has_pending_ask_user());
    }

    #[test]
    fn stream_failure_appends_error_message() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Failed("boom".into()),
        });

        assert!(app.session.pending_reply.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("boom"));
    }

    #[test]
    fn stream_failure_while_waiting_for_interaction_clears_pending_reply_at_app_layer() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.begin_ask_user("call-1".into(), ask_user_request());

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Failed("boom".into()),
        });

        assert!(app.session.pending_reply.is_none());
        assert!(!app.has_pending_ask_user());
    }

    #[test]
    fn subagent_failure_message_includes_log_path_when_available() {
        let mut app = new_app(true);

        app.apply(Action::SubagentEvent(SubagentUiEvent::Failed {
            id: "subagent-1".into(),
            error: "boom".into(),
            log_path: Some("/tmp/subagent-1.json".into()),
        }));

        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry") else {
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
            activity_kind: SubagentActivityKind::General,
        }));

        app.apply(Action::SubagentEvent(SubagentUiEvent::Updated {
            id: "subagent-1".into(),
            latest_tool_name: Some("Grep".into()),
        }));

        let TranscriptEntry::SubagentStatus(status) = app.entries().last().expect("status entry")
        else {
            panic!("expected subagent status entry");
        };
        assert_eq!(status.latest_tool_name.as_deref(), Some("Grep"));
    }

    #[test]
    fn partial_command_completes_before_execution() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/n");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert_eq!(app.composer().lines(), ["/new"]);
        assert_eq!(app.entries().len(), 1);
    }

    #[test]
    fn model_command_opens_model_picker() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/model");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.selection_picker_visible());
        assert!(!app.composer_has_content());
    }

    #[test]
    fn plan_command_enters_planning_draft_mode() {
        let mut app = registry_app(true);
        app.composer_mut().insert_str("/plan");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.planning_draft_mode());
    }

    #[test]
    fn planning_draft_submission_starts_planning_workflow() {
        let mut app = registry_app(true);
        app.enter_planning_draft_mode();
        app.composer_mut().insert_str("Add a planning workflow");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: planning_conversation_prompt("Add a planning workflow"),
                history: Vec::new(),
                history_model_name: None,
            })
        );
        assert!(!app.planning_draft_mode());
        assert!(app.plan_active());
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Planning)
        );
    }

    #[test]
    fn finished_plan_response_enters_plan_review_selection_mode() {
        let mut app = registry_app(true);
        app.session.pending_reply = Some(PendingReply::new(1, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta(
                "<proposed_plan>\n# Test Plan\n\nSummary\n</proposed_plan>".into(),
            ),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Finished { history: None },
        });

        assert!(app.plan_review_selection_active());
        assert!(!app.plan_review_feedback_active());
    }

    #[test]
    fn accepting_plan_starts_normal_prompt_model_turn() {
        let mut app = registry_app(true);
        app.push_agent_message("<proposed_plan>\n# Test Plan\n\n- step one\n</proposed_plan>");
        app.begin_plan_review();

        let effect = app.apply(Action::AcceptPlanAndImplement);

        assert!(matches!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                history,
                prompt,
                ..
            }) if history.is_empty()
                && prompt.contains("You are no longer in Plan Mode")
                && prompt.contains("# Test Plan")
                && prompt.contains("step one")
        ));
        assert!(app.session.pending_reply.is_some());
        assert!(!app.plan_review_selection_active());
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Normal)
        );
    }

    #[test]
    fn suggesting_plan_changes_enters_feedback_mode() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        let effect = app.apply(Action::SuggestPlanChanges);

        assert!(effect.is_none());
        assert!(app.plan_review_feedback_active());
        assert!(!app.plan_review_selection_active());
    }

    #[test]
    fn plan_review_arrow_selection_and_enter_can_choose_feedback() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        let move_effect = app.apply(Action::SelectNextCommand);
        assert!(move_effect.is_none());
        assert_eq!(app.selected_plan_review_index(), Some(1));

        let submit_effect = app.apply(Action::SubmitMessage);
        assert!(submit_effect.is_none());
        assert!(app.plan_review_feedback_active());
    }

    #[test]
    fn plan_review_selection_wraps_with_arrow_keys() {
        let mut app = registry_app(true);
        app.begin_plan_review();

        app.apply(Action::SelectPreviousCommand);
        assert_eq!(app.selected_plan_review_index(), Some(1));

        app.apply(Action::SelectNextCommand);
        assert_eq!(app.selected_plan_review_index(), Some(0));
    }

    #[test]
    fn submitting_plan_feedback_regenerates_plan_with_main_agent_prompt() {
        let mut app = registry_app(true);
        app.begin_plan_review_feedback();
        app.composer_mut().insert_str("Cover rollback and tests.");

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(
            effect,
            Some(Effect::PromptModel {
                reply_id: 1,
                prompt: "Revise the proposed plan based on these comments. Respond with an updated <proposed_plan> block and do not begin implementation yet. Do not use subagents for this revision.\n\nCover rollback and tests.".into(),
                history: Vec::new(),
                history_model_name: None,
            })
        );
        assert!(app.session.pending_reply.is_some());
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Planning)
        );
        assert!(!app.plan_review_feedback_active());
    }

    #[test]
    fn cancelled_subagent_event_updates_status_without_error_message() {
        let mut app = new_app(true);
        app.apply(Action::SubagentEvent(SubagentUiEvent::Spawned {
            id: "subagent-1".into(),
            access_mode: crate::app::AccessMode::ReadOnly,
            activity_kind: SubagentActivityKind::General,
        }));

        let entry_count_before = app.entries().len();
        app.apply(Action::SubagentEvent(SubagentUiEvent::Cancelled {
            id: "subagent-1".into(),
        }));

        assert_eq!(app.entries().len(), entry_count_before);
        let TranscriptEntry::SubagentStatus(status) = app.entries().last().expect("status entry")
        else {
            panic!("expected subagent status entry");
        };
        assert_eq!(status.state, SubagentDisplayState::Cancelled);
        assert_eq!(status.status_text, "cancelled");
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
    fn toggling_planning_picker_selection_persists_default_effort() {
        let mut app = registry_app(true);
        app.open_model_picker();
        app.apply(Action::PickerTabRight);

        let effect = app.apply(Action::TogglePickerSelection);

        assert_eq!(
            effect,
            Some(Effect::SetPlanningAgents {
                planning_agents: vec![PlanningAgentConfig {
                    model_name: "gpt-5.4".into(),
                    reasoning_effort: ReasoningEffort::Low,
                }],
            })
        );
    }

    #[test]
    fn effort_command_returns_effect_for_valid_value() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/effort high");
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
        app.composer_mut().insert_str("/stats");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
        assert!(!app.composer_has_content());
    }

    #[test]
    fn compact_command_returns_effect() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.composer_mut().insert_str("/compact");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::CompactHistory));
        assert!(!app.composer_has_content());
        assert!(app.has_pending_reply());
        assert_eq!(app.history_pending_status_label(), "Compacting context...");
    }

    #[test]
    fn compact_command_without_history_reports_noop() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/compact");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(!app.has_pending_reply());
        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry") else {
            panic!("expected message entry");
        };
        assert_eq!(message.text, "Nothing to compact.");
    }

    #[test]
    fn status_alias_returns_stats_effect() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/status");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::ShowStats));
    }

    #[test]
    fn effort_alias_returns_effect_for_valid_value() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/thinking xhigh");
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
        app.composer_mut().insert_str("/effort turbo");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
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
        app.composer_mut().insert_str("/effort xhigh");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("error entry exists")
        else {
            panic!("expected message entry");
        };
        assert_eq!(message.style, MessageStyle::Error);
        assert!(message.text.contains("supports reasoning efforts"));
    }

    #[test]
    fn effort_command_reports_noop_when_value_is_unchanged() {
        let mut app = new_app(true);
        app.composer_mut().insert_str("/effort medium");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        let TranscriptEntry::Message(message) = app.entries().last().expect("message entry exists")
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
        app.session
            .entries
            .push(TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::User,
                text: "old".into(),
                style: MessageStyle::Plain,
            }));
        app.session.pending_reply = Some(PendingReply::new(8, PendingReplyKind::Normal));
        app.composer_mut().insert_str("/clear");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert_eq!(effect, Some(Effect::RotateSession));
        assert_eq!(app.entries().len(), 1);
        assert!(app.session.pending_reply.is_none());
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
        app.composer_mut().insert_str("/");
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

        assert_eq!(app.ui.history.scroll_top, Some(20));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn page_down_clamps_at_bottom_without_resuming_follow() {
        let mut app = new_app(true);
        app.sync_history_viewport(30, 5);
        app.ui.history.scroll_top = Some(24);

        app.apply(Action::ScrollHistoryPageDown);

        assert_eq!(app.ui.history.scroll_top, Some(25));
        assert!(app.history_is_pinned());
    }

    #[test]
    fn jump_to_bottom_resumes_live_follow() {
        let mut app = new_app(true);
        app.ui.history.scroll_top = Some(7);

        app.apply(Action::ScrollHistoryToBottom);

        assert!(!app.history_is_pinned());
    }

    #[test]
    fn line_scroll_clamps_to_history_bounds() {
        let mut app = new_app(true);
        app.sync_history_viewport(18, 6);
        app.ui.history.scroll_top = Some(2);

        app.apply(Action::ScrollHistoryUp { lines: 10 });
        assert_eq!(app.ui.history.scroll_top, Some(0));

        app.apply(Action::ScrollHistoryDown { lines: 20 });
        assert_eq!(app.ui.history.scroll_top, Some(12));
    }

    #[test]
    fn finishing_history_selection_returns_copy_effect() {
        let mut app = new_app(true);
        app.update_history_snapshot_for_test(0, 0, 20, 2, vec!["alpha".into(), "beta".into()]);

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
        app.composer_mut().insert_str("/quit");
        app.sync_command_selection();

        let effect = app.apply(Action::SubmitMessage);

        assert!(effect.is_none());
        assert!(app.should_quit());
    }

    #[test]
    fn stale_stream_events_are_ignored_after_new_session() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.session.pending_reply = Some(PendingReply::new(11, PendingReplyKind::Normal));
        app.session
            .entries
            .push(TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::User,
                text: "hello".into(),
                style: MessageStyle::Plain,
            }));
        app.composer_mut().insert_str("/new");
        app.sync_command_selection();

        app.apply(Action::SubmitMessage);
        app.apply(Action::StreamEvent {
            reply_id: 11,
            event: StreamEvent::TextDelta("stale".into()),
        });

        assert_eq!(app.entries().len(), 1);
        assert!(app.session_history().is_empty());
    }

    #[test]
    fn finished_stream_replaces_canonical_history() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(2, PendingReplyKind::Normal));
        app.replace_session_history(vec![SessionHistoryMessage::assistant("old")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Finished {
                history: Some(vec![
                    SessionHistoryMessage::user("hello"),
                    SessionHistoryMessage::assistant("world"),
                ]),
            },
        });

        assert!(app.session.pending_reply.is_none());
        assert_eq!(
            app.session_history(),
            &[
                SessionHistoryMessage::user("hello"),
                SessionHistoryMessage::assistant("world")
            ]
        );
    }

    #[test]
    fn finished_planning_stream_clears_plan_active_state() {
        let mut app = registry_app(true);
        app.session.pending_reply = Some(PendingReply::new(2, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Finished { history: None },
        });

        assert!(app.session.pending_reply.is_none());
        assert!(!app.plan_active());
    }

    #[test]
    fn planning_ready_response_starts_planner_fanout() {
        let mut app = registry_app(true);
        app.begin_planning_conversation();
        app.session.pending_reply = Some(PendingReply::new(2, PendingReplyKind::Planning));
        app.replace_session_history(vec![SessionHistoryMessage::user("plan this")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::TextDelta("<planning_ready>\n## Summary\n".into()),
        });
        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::ToolCall {
                name: "List".into(),
                arguments: r#"{"dir":"src"}"#.into(),
            },
        });
        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::TextDelta("Stable brief\n</planning_ready>".into()),
        });
        let effect = app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Finished { history: None },
        });

        assert!(matches!(
            effect,
            Some(Effect::RunPlanningWorkflow {
                description,
                history,
                ..
            }) if description == "## Summary\nStable brief" && history == vec![SessionHistoryMessage::user("plan this")]
        ));
        assert_eq!(
            app.session
                .pending_reply
                .as_ref()
                .map(|pending| pending.kind),
            Some(PendingReplyKind::Planning)
        );
        assert_eq!(
            app.planning_session_stage(),
            Some(PlanningStage::RunningFanout)
        );
    }

    #[test]
    fn failed_stream_keeps_previous_canonical_history() {
        let mut app = new_app(true);
        app.session.pending_reply = Some(PendingReply::new(2, PendingReplyKind::Normal));
        app.replace_session_history(vec![SessionHistoryMessage::assistant("stable")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::Failed("boom".into()),
        });

        assert_eq!(
            app.session_history(),
            &[SessionHistoryMessage::assistant("stable")]
        );
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
