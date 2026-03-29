use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use chrono::Utc;
use portable_pty::ChildKiller;
use tokio::sync::{mpsc, watch};

use crate::{
    app::ActivityDisplayState,
    background_terminals::{
        BackgroundTerminalManager, BackgroundTerminalStatus, BackgroundTerminalUiEvent,
        store::{Inner, State, TerminalRecord},
    },
};

use super::buffer::TokenTailBuffer;

#[test]
fn token_tail_buffer_keeps_recent_output() {
    let mut buffer = TokenTailBuffer::new(8);
    buffer.append("hello ".into());
    let sequence = buffer.sequence();
    buffer.append("world".into());

    let read = buffer.read_after(Some(sequence));
    assert_eq!(read.text, "world");
}

#[derive(Debug)]
struct RecordingKiller {
    calls: Arc<AtomicUsize>,
}

impl ChildKiller for RecordingKiller {
    fn kill(&mut self) -> std::io::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        Box::new(Self {
            calls: Arc::clone(&self.calls),
        })
    }
}

#[test]
fn kill_marks_terminal_cancelled_and_uses_killer_handle() {
    let now = Utc::now();
    let calls = Arc::new(AtomicUsize::new(0));
    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel();
    let (notify_tx, _) = watch::channel(0);

    let manager = BackgroundTerminalManager {
        inner: Arc::new(Inner {
            state: std::sync::Mutex::new(State {
                next_id: 2,
                max_running: 8,
                max_finished: 20,
                generation: 0,
                records: std::collections::HashMap::from([(
                    "terminal-1".into(),
                    TerminalRecord {
                        id: "terminal-1".into(),
                        label: "loop".into(),
                        status: BackgroundTerminalStatus::Running,
                        cwd: ".".into(),
                        pid: Some(1234),
                        started_at: now,
                        ended_at: None,
                        last_activity_at: now,
                        exit_info: None,
                        error: None,
                        output: TokenTailBuffer::new(32),
                    },
                )]),
                killers: std::collections::HashMap::from([(
                    "terminal-1".into(),
                    Box::new(RecordingKiller {
                        calls: Arc::clone(&calls),
                    }) as Box<dyn ChildKiller + Send + Sync>,
                )]),
                finished_order: std::collections::VecDeque::new(),
            }),
            notify_tx,
            ui_tx,
        }),
    };

    let snapshot = manager.kill("terminal-1").expect("kill succeeds");
    assert_eq!(snapshot.status, BackgroundTerminalStatus::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let event = ui_rx.try_recv().expect("state change event");
    assert_eq!(
        event,
        BackgroundTerminalUiEvent::StateChanged {
            id: "terminal-1".into(),
            label: "loop".into(),
            state: ActivityDisplayState::Cancelled,
            status_text: "cancelled".into(),
            detail_text: Some("cwd: .".into()),
        }
    );

    let snapshot = manager.kill("terminal-1").expect("repeat kill succeeds");
    assert_eq!(snapshot.status, BackgroundTerminalStatus::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
