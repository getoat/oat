use super::super::{Effect, PendingReplyKind, StreamEvent, TurnEndReason};
use crate::app::{AppState, MessageStyle, ops, query};
use crate::features::planning::{PlanningReply, PlanningStage, parse_planning_reply};

pub(crate) fn on_stream_event(
    state: &mut AppState,
    reply_id: u64,
    event: StreamEvent,
) -> Option<Effect> {
    if let StreamEvent::SessionTitleGenerated(title) = event {
        let _ = ops::session::store_session_title(state, reply_id, title);
        return None;
    }

    if ops::session::active_reply_id(state) != Some(reply_id) {
        return None;
    }

    match event {
        StreamEvent::TextDelta(delta) => {
            ops::transcript::append_pending_stream_message(state, &delta, MessageStyle::Plain);
            None
        }
        StreamEvent::Commentary(message) => {
            ops::transcript::push_agent_commentary(state, message);
            None
        }
        StreamEvent::ReasoningDelta(delta) => {
            if state.session.show_thinking {
                ops::transcript::append_pending_stream_message(
                    state,
                    &delta,
                    MessageStyle::Thinking,
                );
            }
            None
        }
        StreamEvent::ToolCall { name, arguments } => {
            ops::transcript::push_tool_call(state, name, arguments);
            None
        }
        StreamEvent::ToolResult { name, output } => {
            ops::transcript::push_tool_result(state, name, output);
            None
        }
        StreamEvent::AskUserRequested {
            request_id,
            request,
        } => {
            ops::ask_user::begin_ask_user(state, request_id, request);
            None
        }
        StreamEvent::WriteApprovalRequested {
            request_id,
            tool_name,
            arguments,
        } => {
            ops::approvals::begin_write_approval(state, request_id, tool_name, arguments);
            None
        }
        StreamEvent::ShellApprovalRequested {
            request_id,
            risk,
            risk_explanation,
            command,
            working_directory,
            reason,
        } => {
            ops::approvals::begin_shell_approval(
                state,
                request_id,
                risk,
                risk_explanation,
                command,
                working_directory,
                reason,
            );
            None
        }
        StreamEvent::PlanningFinalizationStarted => {
            ops::planning::begin_planning_finalization(state);
            None
        }
        StreamEvent::CompactionFinished {
            history,
            model_name,
        } => {
            ops::session::replace_session_history(state, history);
            ops::session::set_last_history_model_name(state, Some(model_name));
            ops::ask_user::clear_pending_ask_user(state);
            ops::session::clear_pending_reply_only(state);
            ops::transcript::push_agent_message(state, "Context compacted.");
            None
        }
        StreamEvent::TurnEnded { reason, history } => {
            let pending_kind = ops::session::active_reply_kind(state);
            let planning_stage = query::planning_session_stage(state);
            let final_text = ops::session::pending_reply_replay_seed(state)
                .map(|pending| pending.plain_text)
                .unwrap_or_default();
            if let Some(history) = history {
                ops::session::replace_session_history(state, history);
                let model_name = state.session.model_name.clone();
                ops::session::set_last_history_model_name(state, Some(model_name));
            }
            ops::ask_user::clear_pending_ask_user(state);
            if reason == TurnEndReason::InterruptedAtStepBoundary {
                ops::session::clear_pending_reply_only(state);
                return None;
            }
            let planning_reply = matches!(pending_kind, Some(PendingReplyKind::Planning))
                .then(|| parse_planning_reply(&final_text));
            if planning_stage == Some(PlanningStage::Conversation) {
                if let Some(PlanningReply::ReadyBrief(brief)) = planning_reply.clone() {
                    ops::planning::set_planning_brief(state, brief.markdown.clone());
                    ops::transcript::discard_pending_text_entry(state);
                    ops::session::clear_pending_reply_only(state);
                    ops::planning::begin_planning_fanout(state);
                    let reply_id = ops::session::next_reply_id(state);
                    ops::session::set_pending_reply(state, reply_id, PendingReplyKind::Planning);
                    return Some(Effect::RunPlanningWorkflow {
                        reply_id,
                        description: brief.markdown,
                        history: state.session.session_history.to_vec(),
                        history_model_name: state.session.last_history_model_name.clone(),
                    });
                }
            }
            if let Some(PlanningReply::ProposedPlan(plan)) = planning_reply {
                ops::planning::store_proposed_plan(state, plan);
            }
            ops::session::clear_pending_reply_only(state);
            if pending_kind == Some(PendingReplyKind::Planning)
                && matches!(
                    parse_planning_reply(&final_text),
                    PlanningReply::ProposedPlan(_)
                )
            {
                ops::planning::begin_plan_review(state);
            }
            None
        }
        StreamEvent::Failed(error) => {
            if query::planning_session_stage(state) == Some(PlanningStage::RunningFanout) {
                ops::planning::begin_planning_conversation(state);
            }
            ops::ask_user::clear_pending_ask_user(state);
            ops::session::clear_pending_reply_only(state);
            ops::transcript::push_error_message(state, format!("Request failed: {error}"));
            None
        }
        StreamEvent::SessionTitleGenerated(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{
            Action, ChatMessage, MessageStyle, PendingReply, PendingReplyKind,
            SessionHistoryMessage, Speaker, TranscriptEntry,
            session::test_support::{new_app, registry_app},
        },
        ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest},
        features::planning::PlanningStage,
    };

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
    fn stream_text_creates_and_updates_agent_message() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
    fn interrupted_step_boundary_replaces_history_and_clears_pending_reply() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        let updated_history = vec![SessionHistoryMessage::assistant("tool-informed context")];
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TurnEnded {
                reason: TurnEndReason::InterruptedAtStepBoundary,
                history: Some(updated_history.clone()),
            },
        });

        assert_eq!(app.session_history(), updated_history.as_slice());
        assert!(app.state_mut().session.pending_reply.is_none());
    }

    #[test]
    fn whitespace_only_text_delta_stays_pending_without_visible_content() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta("\n  ".into()),
        });

        assert_eq!(app.entries().len(), 1);
        assert!(!app.has_visible_pending_content());
        assert_eq!(
            app.state_mut()
                .session
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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Planning));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Planning));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::ReasoningDelta("thinking".into()),
        });

        assert_eq!(app.entries().len(), 1);
    }

    #[test]
    fn stream_commentary_adds_agent_commentary_entry() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
            app.state_mut()
                .session
                .pending_reply
                .as_ref()
                .expect("pending reply")
                .plain_text,
            "BeforeAfter"
        );
    }

    #[test]
    fn stream_text_after_tool_result_starts_new_message_entry() {
        let mut app = crate::app::App::new(
            true,
            true,
            "gpt-5-mini",
            crate::config::ReasoningEffort::Medium,
        );
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

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
    fn stream_failure_appends_error_message() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Failed("boom".into()),
        });

        assert!(app.state_mut().session.pending_reply.is_none());
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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Normal));
        app.begin_ask_user("call-1".into(), ask_user_request());

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::Failed("boom".into()),
        });

        assert!(app.state_mut().session.pending_reply.is_none());
        assert!(!app.has_pending_ask_user());
    }

    #[test]
    fn stale_stream_events_are_ignored_after_new_session() {
        let mut app = new_app(true);
        app.replace_session_history(vec![SessionHistoryMessage::assistant("previous")]);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(11, PendingReplyKind::Normal));
        app.state_mut()
            .session
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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(2, PendingReplyKind::Normal));
        app.replace_session_history(vec![SessionHistoryMessage::assistant("old")]);

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::TurnEnded {
                reason: TurnEndReason::Completed,
                history: Some(vec![
                    SessionHistoryMessage::user("hello"),
                    SessionHistoryMessage::assistant("world"),
                ]),
            },
        });

        assert!(app.state_mut().session.pending_reply.is_none());
        assert_eq!(
            app.session_history(),
            &[
                SessionHistoryMessage::user("hello"),
                SessionHistoryMessage::assistant("world")
            ]
        );
    }

    #[test]
    fn finished_plan_response_enters_plan_review_selection_mode() {
        let mut app = registry_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(1, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TextDelta(
                "<proposed_plan>\n# Test Plan\n\nSummary\n</proposed_plan>".into(),
            ),
        });
        app.apply(Action::StreamEvent {
            reply_id: 1,
            event: StreamEvent::TurnEnded {
                reason: TurnEndReason::Completed,
                history: None,
            },
        });

        assert!(app.plan_review_selection_active());
    }

    #[test]
    fn finished_planning_stream_clears_plan_active_state() {
        let mut app = registry_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(2, PendingReplyKind::Planning));

        app.apply(Action::StreamEvent {
            reply_id: 2,
            event: StreamEvent::TurnEnded {
                reason: TurnEndReason::Completed,
                history: None,
            },
        });

        assert!(app.state_mut().session.pending_reply.is_none());
        assert!(!app.plan_active());
    }

    #[test]
    fn planning_ready_response_starts_planner_fanout() {
        let mut app = registry_app(true);
        app.begin_planning_conversation();
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(2, PendingReplyKind::Planning));
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
            event: StreamEvent::TurnEnded {
                reason: TurnEndReason::Completed,
                history: None,
            },
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
            app.state_mut()
                .session
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
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(2, PendingReplyKind::Normal));
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
    fn session_title_event_updates_title_without_active_reply() {
        let mut app = new_app(true);
        app.state_mut().session.pending_session_title_reply_id = Some(7);

        app.apply(Action::StreamEvent {
            reply_id: 7,
            event: StreamEvent::SessionTitleGenerated("Fix planning rejection flow".into()),
        });

        assert_eq!(
            app.state().session.session_title.as_deref(),
            Some("Fix planning rejection flow")
        );
        assert_eq!(app.state().session.pending_session_title_reply_id, None);
    }

    #[test]
    fn stale_session_title_event_is_ignored() {
        let mut app = new_app(true);
        app.state_mut().session.pending_session_title_reply_id = Some(8);

        app.apply(Action::StreamEvent {
            reply_id: 9,
            event: StreamEvent::SessionTitleGenerated("Stale title".into()),
        });

        assert_eq!(app.state().session.session_title, None);
        assert_eq!(app.state().session.pending_session_title_reply_id, Some(8));
    }
}
