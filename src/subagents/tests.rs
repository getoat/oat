use super::*;
use crate::completion_request::CompletionRequestSnapshot;
use crate::{app::StreamEvent, stats::StatsStore};
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::time::advance;

fn manager(max_concurrent: usize) -> (SubagentManager, mpsc::UnboundedReceiver<SubagentUiEvent>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (
        SubagentManager::new(max_concurrent, tx, StatsStore::new()),
        rx,
    )
}

#[test]
fn inspect_returns_running_snapshot() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));

    let snapshot = manager.inspect("subagent-1").expect("inspect succeeds");

    assert_eq!(snapshot.status, SubagentStatus::Running);
    assert_eq!(snapshot.id, "subagent-1");
}

#[tokio::test(start_paused = true)]
async fn wait_returns_completed_subagent_immediately() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.complete_for_test("subagent-1", "done");

    let result = manager
        .wait(&["subagent-1".into()], Some(Duration::from_secs(30)))
        .await
        .expect("wait succeeds");

    assert_eq!(result.completed_id.as_deref(), Some("subagent-1"));
    assert!(!result.timed_out_on_inactivity);
}

#[tokio::test(start_paused = true)]
async fn wait_times_out_after_inactivity() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait(&["subagent-1".into()], Some(Duration::from_millis(100)))
                .await
        }
    });

    advance(Duration::from_millis(101)).await;
    let result = wait.await.expect("join").expect("wait succeeds");

    assert_eq!(result.inactive_id.as_deref(), Some("subagent-1"));
    assert!(result.timed_out_on_inactivity);
}

#[tokio::test(start_paused = true)]
async fn wait_resets_timeout_on_activity() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait(&["subagent-1".into()], Some(Duration::from_millis(100)))
                .await
        }
    });

    advance(Duration::from_millis(80)).await;
    manager.mark_activity_for_test("subagent-1");
    advance(Duration::from_millis(80)).await;
    assert!(!wait.is_finished());

    manager.fail_for_test("subagent-1", "boom");
    let result = wait.await.expect("join").expect("wait succeeds");
    assert_eq!(result.failed_id.as_deref(), Some("subagent-1"));
}

#[tokio::test(start_paused = true)]
async fn wait_pauses_timeout_while_subagent_is_waiting_for_approval() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.record_approval_wait_for_test("subagent-1", "req-1", "RunShellScript");

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait(&["subagent-1".into()], Some(Duration::from_millis(100)))
                .await
        }
    });

    advance(Duration::from_secs(1)).await;
    assert!(!wait.is_finished());

    manager.clear_waiting_for_approval_for_test("req-1");
    advance(Duration::from_millis(101)).await;
    let result = wait.await.expect("join").expect("wait succeeds");

    assert_eq!(result.inactive_id.as_deref(), Some("subagent-1"));
    assert!(result.timed_out_on_inactivity);
}

#[tokio::test(start_paused = true)]
async fn wait_returns_cancelled_subagent_immediately() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.cancel_all_running_for_test().await;

    let result = manager
        .wait(&["subagent-1".into()], Some(Duration::from_secs(30)))
        .await
        .expect("wait succeeds");

    assert_eq!(result.cancelled_id.as_deref(), Some("subagent-1"));
    assert!(!result.timed_out_on_inactivity);
}

#[tokio::test(start_paused = true)]
async fn wait_all_returns_after_every_id_reaches_a_terminal_state() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.register_running_for_test("subagent-2", Duration::from_secs(0));

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait_all(
                    &["subagent-1".into(), "subagent-2".into()],
                    Some(Duration::from_secs(30)),
                )
                .await
        }
    });

    manager.complete_for_test("subagent-1", "done");
    tokio::task::yield_now().await;
    assert!(!wait.is_finished());

    manager.fail_for_test("subagent-2", "boom");
    let result = wait.await.expect("join").expect("wait succeeds");

    assert_eq!(result.completed_id.as_deref(), Some("subagent-1"));
    assert_eq!(result.failed_id.as_deref(), Some("subagent-2"));
    assert!(!result.timed_out_on_inactivity);
    assert!(
        result
            .subagents
            .iter()
            .all(|snapshot| snapshot.status != SubagentStatus::Running)
    );
}

#[tokio::test(start_paused = true)]
async fn wait_all_reports_inactivity_when_one_id_stalls() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.register_running_for_test("subagent-2", Duration::from_secs(0));
    manager.complete_for_test("subagent-1", "done");

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait_all(
                    &["subagent-1".into(), "subagent-2".into()],
                    Some(Duration::from_millis(100)),
                )
                .await
        }
    });

    advance(Duration::from_millis(101)).await;
    let result = wait.await.expect("join").expect("wait succeeds");

    assert_eq!(result.completed_id.as_deref(), Some("subagent-1"));
    assert_eq!(result.inactive_id.as_deref(), Some("subagent-2"));
    assert!(result.timed_out_on_inactivity);
}

#[tokio::test(start_paused = true)]
async fn wait_all_does_not_treat_approval_wait_as_terminal() {
    let (manager, _rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.record_approval_wait_for_test("subagent-1", "req-1", "WriteFile");

    let wait = tokio::spawn({
        let manager = manager.clone();
        async move {
            manager
                .wait_all(&["subagent-1".into()], Some(Duration::from_millis(100)))
                .await
        }
    });

    advance(Duration::from_secs(1)).await;
    assert!(!wait.is_finished());

    manager.clear_waiting_for_approval_for_test("req-1");
    manager.complete_for_test("subagent-1", "done");
    let result = wait.await.expect("join").expect("wait succeeds");

    assert_eq!(result.completed_id.as_deref(), Some("subagent-1"));
    assert!(!result.timed_out_on_inactivity);
}

#[test]
fn prompt_token_estimate_uses_tokenizer() {
    assert_eq!(
        estimate_prompt_tokens("Count tokens with the shared tokenizer."),
        count_text_tokens("Count tokens with the shared tokenizer.") as usize
    );
}

#[test]
fn tool_activity_updates_snapshot_and_emits_ui_event() {
    let (manager, mut rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));

    manager.handle_stream_event(
        "subagent-1",
        StreamEvent::ToolCall {
            name: "Grep".into(),
            arguments: "{}".into(),
        },
    );

    assert_eq!(
        rx.try_recv().expect("ui event"),
        SubagentUiEvent::Updated {
            id: "subagent-1".into(),
            latest_tool_name: Some("Grep".into()),
        }
    );
    assert_eq!(
        manager
            .inspect("subagent-1")
            .expect("inspect succeeds")
            .latest_tool_name
            .as_deref(),
        Some("Grep")
    );
}

#[tokio::test]
async fn cancelling_running_subagents_marks_them_cancelled_and_emits_ui_events() {
    let (manager, mut rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.register_running_for_test("subagent-2", Duration::from_secs(0));

    let cancelled = manager.cancel_all_running_for_test().await;

    assert_eq!(
        cancelled,
        vec!["subagent-1".to_string(), "subagent-2".to_string()]
    );
    assert_eq!(
        manager
            .inspect("subagent-1")
            .expect("inspect succeeds")
            .status,
        SubagentStatus::Cancelled
    );
    assert_eq!(
        rx.try_recv().expect("first event"),
        SubagentUiEvent::Cancelled {
            id: "subagent-1".into(),
        }
    );
    assert_eq!(
        rx.try_recv().expect("second event"),
        SubagentUiEvent::Cancelled {
            id: "subagent-2".into(),
        }
    );
}

#[tokio::test]
async fn cancelled_subagents_ignore_late_tool_activity() {
    let (manager, mut rx) = manager(4);
    manager.register_running_for_test("subagent-1", Duration::from_secs(0));
    manager.cancel_all_running_for_test().await;
    assert_eq!(
        rx.try_recv().expect("cancelled event"),
        SubagentUiEvent::Cancelled {
            id: "subagent-1".into(),
        }
    );

    manager.handle_stream_event(
        "subagent-1",
        StreamEvent::ToolCall {
            name: "Grep".into(),
            arguments: "{}".into(),
        },
    );

    assert!(rx.try_recv().is_err());
    assert!(
        manager
            .inspect("subagent-1")
            .expect("inspect succeeds")
            .latest_tool_name
            .is_none()
    );
}

#[test]
fn context_length_failures_are_normalized() {
    let message = normalize_subagent_failure(
        "CompletionError: ProviderError: Invalid status code 400 Bad Request with message: {\"error\":{\"message\":\"Input tokens exceed the configured limit of 272000 tokens. Your messages resulted in 801427 tokens. Please reduce the length of the messages.\",\"type\":\"invalid_request_error\",\"param\":\"messages\",\"code\":\"context_length_exceeded\"}}",
    );

    assert!(message.contains("801427 tokens > 272000"));
    assert!(message.contains("captured request"));
}

#[test]
fn failure_logs_are_persisted_as_json() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("oat-subagent-failures-{unique}"));
    let entry = SubagentFailureLog {
        schema_version: SUBAGENT_FAILURE_LOG_SCHEMA_VERSION,
        subagent_id: "subagent-7".into(),
        failed_at_unix_ms: 123,
        model_name: "gpt-5.4".into(),
        access_mode: "read-only".into(),
        prompt: "inspect src".into(),
        raw_error: "raw".into(),
        normalized_error: "normalized".into(),
        failing_request: Some(CompletionRequestSnapshot::capture(
            &rig::completion::Message::user("latest prompt"),
            &[rig::completion::Message::assistant("history item")],
        )),
    };

    let path = persist_subagent_failure_log(Some(&dir), &entry)
        .expect("persist succeeds")
        .expect("path returned");
    let payload = fs::read_to_string(&path).expect("payload readable");

    assert!(payload.contains("\"subagent_id\": \"subagent-7\""));
    assert!(payload.contains("\"normalized_error\": \"normalized\""));
    assert!(payload.contains("\"failing_request\""));
    assert!(payload.contains("\"latest prompt\""));

    let _ = fs::remove_dir_all(&dir);
}
