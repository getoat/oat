use std::{io::Read, sync::Arc, time::Duration};

use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use tokio::sync::{mpsc, watch};

use crate::app::ActivityDisplayState;
use crate::debug_log::log_debug;

use super::{
    BackgroundTerminalInspectRequest, BackgroundTerminalInspectResult, BackgroundTerminalManager,
    BackgroundTerminalSnapshot, BackgroundTerminalSpawnRequest, BackgroundTerminalStatus,
    BackgroundTerminalUiEvent, TerminalExitInfo, TerminalOutputSlice,
    format::normalize_terminal_output,
    pty::spawn_pty,
    store::{Inner, State, TerminalRecord, snapshot_from_record},
};
use crate::background_terminals::buffer::TokenTailBuffer;

const DEFAULT_MAX_RUNNING_TERMINALS: usize = 8;
const DEFAULT_MAX_FINISHED_TERMINALS: usize = 20;
const DEFAULT_RETAINED_OUTPUT_TOKENS: usize = 10_000;

impl BackgroundTerminalManager {
    pub(crate) fn new(ui_tx: mpsc::UnboundedSender<BackgroundTerminalUiEvent>) -> Self {
        let (notify_tx, _) = watch::channel(0);
        Self {
            inner: Arc::new(Inner {
                state: std::sync::Mutex::new(State {
                    next_id: 1,
                    max_running: DEFAULT_MAX_RUNNING_TERMINALS,
                    max_finished: DEFAULT_MAX_FINISHED_TERMINALS,
                    generation: 0,
                    records: std::collections::HashMap::new(),
                    killers: std::collections::HashMap::new(),
                    finished_order: std::collections::VecDeque::new(),
                }),
                notify_tx,
                ui_tx,
            }),
        }
    }

    pub(crate) async fn start(
        &self,
        request: BackgroundTerminalSpawnRequest,
    ) -> Result<BackgroundTerminalSnapshot> {
        let id = {
            let mut state = self.inner.state.lock().expect("terminal state lock");
            if state.running_count() >= state.max_running {
                bail!(
                    "Background terminal limit reached: {} running, max {}.",
                    state.running_count(),
                    state.max_running
                );
            }
            let id = format!("terminal-{}", state.next_id);
            state.next_id += 1;
            id
        };

        match spawn_pty(&request.script, std::path::Path::new(&request.cwd)) {
            Ok(spawned) => {
                let now = Utc::now();
                let snapshot = {
                    let mut state = self.inner.state.lock().expect("terminal state lock");
                    state.records.insert(
                        id.clone(),
                        TerminalRecord {
                            id: id.clone(),
                            label: request.label.clone(),
                            status: BackgroundTerminalStatus::Running,
                            cwd: request.cwd.clone(),
                            pid: spawned.pid,
                            started_at: now,
                            ended_at: None,
                            last_activity_at: now,
                            exit_info: None,
                            error: None,
                            output: TokenTailBuffer::new(DEFAULT_RETAINED_OUTPUT_TOKENS),
                        },
                    );
                    state.killers.insert(id.clone(), spawned.killer);
                    self.bump_generation(&mut state);
                    snapshot_from_record(state.records.get(&id).expect("record exists"))
                };

                let _ = self.inner.ui_tx.send(BackgroundTerminalUiEvent::Spawned {
                    id: id.clone(),
                    label: request.label,
                    cwd: request.cwd,
                    pid: spawned.pid,
                });

                self.spawn_reader(id.clone(), spawned.reader);
                self.spawn_waiter(id, spawned.child);
                Ok(snapshot)
            }
            Err(error) => {
                let message = error.to_string();
                let now = Utc::now();
                let snapshot = {
                    let mut state = self.inner.state.lock().expect("terminal state lock");
                    state.records.insert(
                        id.clone(),
                        TerminalRecord {
                            id: id.clone(),
                            label: request.label.clone(),
                            status: BackgroundTerminalStatus::SpawnFailed,
                            cwd: request.cwd.clone(),
                            pid: None,
                            started_at: now,
                            ended_at: Some(now),
                            last_activity_at: now,
                            exit_info: None,
                            error: Some(message.clone()),
                            output: TokenTailBuffer::new(DEFAULT_RETAINED_OUTPUT_TOKENS),
                        },
                    );
                    state.finished_order.push_back(id.clone());
                    self.prune_finished_locked(&mut state);
                    self.bump_generation(&mut state);
                    snapshot_from_record(state.records.get(&id).expect("record exists"))
                };
                let _ = self
                    .inner
                    .ui_tx
                    .send(BackgroundTerminalUiEvent::StateChanged {
                        id,
                        label: request.label,
                        state: ActivityDisplayState::Failed,
                        status_text: format!("spawn failed: {message}"),
                        detail_text: Some(format!("cwd: {}", request.cwd)),
                    });
                Ok(snapshot)
            }
        }
    }

    pub(crate) fn list(&self) -> Vec<BackgroundTerminalSnapshot> {
        let state = self.inner.state.lock().expect("terminal state lock");
        let mut snapshots = state
            .records
            .values()
            .map(snapshot_from_record)
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            sort_rank(left.status)
                .cmp(&sort_rank(right.status))
                .then_with(|| right.started_at.cmp(&left.started_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        snapshots
    }

    pub(crate) async fn inspect(
        &self,
        id: &str,
        request: BackgroundTerminalInspectRequest,
    ) -> Result<BackgroundTerminalInspectResult> {
        let mut rx = self.inner.notify_tx.subscribe();
        let wait = request
            .wait_for_change_ms
            .map(Duration::from_millis)
            .filter(|duration| !duration.is_zero());
        let deadline = wait.map(|duration| tokio::time::Instant::now() + duration);

        loop {
            let result = self.inspect_now(id, request.after_sequence)?;
            let should_wait = request.after_sequence.is_some_and(|after| {
                result.snapshot.status == BackgroundTerminalStatus::Running
                    && result.output.sequence <= after
            });
            if !should_wait {
                return Ok(result);
            }

            let Some(deadline) = deadline else {
                return Ok(result);
            };
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Ok(result);
            }
            if tokio::time::timeout(remaining, rx.changed()).await.is_err() {
                return Ok(result);
            }
        }
    }

    pub(crate) fn kill(&self, id: &str) -> Result<BackgroundTerminalSnapshot> {
        log_debug("background_terminal_manager", format!("kill_start id={id}"));
        let killer = {
            let mut state = self.inner.state.lock().expect("terminal state lock");
            let record = state
                .records
                .get_mut(id)
                .ok_or_else(|| anyhow!("Unknown background terminal `{id}`."))?;
            if record.status != BackgroundTerminalStatus::Running {
                log_debug(
                    "background_terminal_manager",
                    format!("kill_skip_non_running id={id} status={:?}", record.status),
                );
                return Ok(snapshot_from_record(record));
            }
            let now = Utc::now();
            record.status = BackgroundTerminalStatus::Cancelled;
            record.ended_at = Some(now);
            record.last_activity_at = now;
            record.exit_info = None;
            state.finished_order.push_back(id.to_string());
            let killer = state.killers.remove(id);
            self.prune_finished_locked(&mut state);
            self.bump_generation(&mut state);
            killer
        };

        if let Some(mut killer) = killer {
            let _ = killer.kill();
            log_debug(
                "background_terminal_manager",
                format!("kill_signal_sent id={id}"),
            );
        }

        if let Some(snapshot) = self.try_snapshot(id) {
            let _ = self
                .inner
                .ui_tx
                .send(BackgroundTerminalUiEvent::StateChanged {
                    id: snapshot.id.clone(),
                    label: snapshot.label.clone(),
                    state: ActivityDisplayState::Cancelled,
                    status_text: "cancelled".into(),
                    detail_text: Some(format!("cwd: {}", snapshot.cwd)),
                });
            Ok(snapshot)
        } else {
            Err(anyhow!("Unknown background terminal `{id}`."))
        }
    }

    pub(crate) fn cancel_all_running(&self) -> Vec<String> {
        let (ids, killers) = {
            let mut state = self.inner.state.lock().expect("terminal state lock");
            let ids = state
                .records
                .values()
                .filter(|record| record.status == BackgroundTerminalStatus::Running)
                .map(|record| record.id.clone())
                .collect::<Vec<_>>();
            let mut killers = Vec::new();
            for id in &ids {
                if let Some(record) = state.records.get_mut(id) {
                    let now = Utc::now();
                    record.status = BackgroundTerminalStatus::Cancelled;
                    record.ended_at = Some(now);
                    record.last_activity_at = now;
                }
                state.finished_order.push_back(id.clone());
                if let Some(killer) = state.killers.remove(id) {
                    killers.push((id.clone(), killer));
                }
            }
            self.prune_finished_locked(&mut state);
            self.bump_generation(&mut state);
            (ids, killers)
        };

        for (id, mut killer) in killers {
            let _ = killer.kill();
            log_debug(
                "background_terminal_manager",
                format!("cancel_all_signal_sent id={id}"),
            );
            if let Some(snapshot) = self.try_snapshot(&id) {
                let _ = self
                    .inner
                    .ui_tx
                    .send(BackgroundTerminalUiEvent::StateChanged {
                        id: snapshot.id.clone(),
                        label: snapshot.label.clone(),
                        state: ActivityDisplayState::Cancelled,
                        status_text: "cancelled".into(),
                        detail_text: Some(format!("cwd: {}", snapshot.cwd)),
                    });
            }
        }

        ids
    }

    pub(crate) fn inspect_now(
        &self,
        id: &str,
        after_sequence: Option<u64>,
    ) -> Result<BackgroundTerminalInspectResult> {
        let state = self.inner.state.lock().expect("terminal state lock");
        let record = state
            .records
            .get(id)
            .ok_or_else(|| anyhow!("Unknown background terminal `{id}`."))?;
        let read = record.output.read_after(after_sequence);
        Ok(BackgroundTerminalInspectResult {
            snapshot: snapshot_from_record(record),
            output: TerminalOutputSlice {
                sequence: read.sequence,
                text: read.text,
                output_truncated: read.output_truncated,
                cursor_expired: read.cursor_expired,
            },
        })
    }

    fn try_snapshot(&self, id: &str) -> Option<BackgroundTerminalSnapshot> {
        let state = self.inner.state.lock().expect("terminal state lock");
        state.records.get(id).map(snapshot_from_record)
    }

    fn append_output(&self, id: &str, text: String) {
        let mut state = self.inner.state.lock().expect("terminal state lock");
        if let Some(record) = state.records.get_mut(id) {
            record.last_activity_at = Utc::now();
            record.output.append(text);
            self.bump_generation(&mut state);
        }
    }

    fn mark_exited(&self, id: &str, exit_info: TerminalExitInfo) {
        let snapshot = {
            let mut state = self.inner.state.lock().expect("terminal state lock");
            if !state.records.contains_key(id) {
                return;
            }
            state.killers.remove(id);
            if state
                .records
                .get(id)
                .is_some_and(|record| record.status != BackgroundTerminalStatus::Running)
            {
                self.bump_generation(&mut state);
                return;
            }
            let now = Utc::now();
            {
                let record = state
                    .records
                    .get_mut(id)
                    .expect("record exists after contains check");
                record.status = BackgroundTerminalStatus::Exited;
                record.ended_at = Some(now);
                record.last_activity_at = now;
                record.exit_info = Some(exit_info.clone());
            }
            state.finished_order.push_back(id.to_string());
            self.prune_finished_locked(&mut state);
            self.bump_generation(&mut state);
            snapshot_from_record(state.records.get(id).expect("record exists"))
        };

        let status_text = if exit_info.success {
            format!("exited {}", exit_info.code.unwrap_or(0))
        } else {
            format!(
                "exited non-zero {}",
                exit_info
                    .code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "signal".into())
            )
        };
        let _ = self
            .inner
            .ui_tx
            .send(BackgroundTerminalUiEvent::StateChanged {
                id: snapshot.id,
                label: snapshot.label,
                state: ActivityDisplayState::Completed,
                status_text,
                detail_text: Some(format!("cwd: {}", snapshot.cwd)),
            });
    }

    fn spawn_reader(&self, id: String, mut reader: Box<dyn Read + Send>) {
        let manager = self.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0_u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => {
                        let text = normalize_terminal_output(&buf[..len]);
                        manager.append_output(&id, text);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    fn spawn_waiter(&self, id: String, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        let manager = self.clone();
        std::thread::spawn(move || {
            log_debug("background_terminal_manager", format!("wait_start id={id}"));
            let status = child.wait();
            log_debug(
                "background_terminal_manager",
                format!("wait_done id={id} ok={}", status.is_ok()),
            );
            let Ok(status) = status else {
                manager.mark_exited(
                    &id,
                    TerminalExitInfo {
                        success: false,
                        code: None,
                    },
                );
                return;
            };
            manager.mark_exited(
                &id,
                TerminalExitInfo {
                    success: status.success(),
                    code: Some(i32::try_from(status.exit_code()).unwrap_or(i32::MAX)),
                },
            );
        });
    }

    fn prune_finished_locked(&self, state: &mut State) {
        while state.finished_order.len() > state.max_finished {
            let Some(id) = state.finished_order.pop_front() else {
                break;
            };
            if state
                .records
                .get(&id)
                .is_some_and(|record| record.status == BackgroundTerminalStatus::Running)
            {
                state.finished_order.push_back(id);
                break;
            }
            state.records.remove(&id);
            state.killers.remove(&id);
        }
    }

    fn bump_generation(&self, state: &mut State) {
        state.generation = state.generation.wrapping_add(1);
        let _ = self.inner.notify_tx.send(state.generation);
    }
}

fn sort_rank(status: BackgroundTerminalStatus) -> usize {
    match status {
        BackgroundTerminalStatus::Running => 0,
        BackgroundTerminalStatus::Exited => 1,
        BackgroundTerminalStatus::Cancelled => 2,
        BackgroundTerminalStatus::SpawnFailed => 3,
    }
}
