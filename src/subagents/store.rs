use std::{collections::HashMap, sync::Mutex};

use anyhow::{Result, anyhow, bail};
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
    time::Instant,
};

use crate::{app::AccessMode, stats::StatsStore};

use super::{
    SubagentManager, SubagentSnapshot, SubagentStatus, SubagentUiEvent,
    waiting::snapshot_from_record,
};

pub(super) struct Inner {
    pub(super) state: Mutex<State>,
    pub(super) notify_tx: watch::Sender<u64>,
    pub(super) ui_tx: mpsc::UnboundedSender<SubagentUiEvent>,
    pub(super) stats: StatsStore,
}

pub(super) struct State {
    pub(super) next_id: u64,
    pub(super) max_concurrent: usize,
    pub(super) generation: u64,
    pub(super) records: HashMap<String, SubagentRecord>,
    pub(super) tasks: HashMap<String, JoinHandle<()>>,
}

#[derive(Clone, Debug)]
pub(super) struct SubagentRecord {
    pub(super) id: String,
    pub(super) status: SubagentStatus,
    pub(super) access_mode: AccessMode,
    #[allow(dead_code)]
    pub(super) model_name: String,
    pub(super) latest_tool_name: Option<String>,
    pub(super) output: Option<String>,
    pub(super) error: Option<String>,
    pub(super) failure_log_path: Option<String>,
    pub(super) last_activity_at: Instant,
    pub(super) waiting_for_approval: bool,
    pub(super) pending_approval_request_id: Option<String>,
}

impl SubagentManager {
    pub fn new(
        max_concurrent: usize,
        ui_tx: mpsc::UnboundedSender<SubagentUiEvent>,
        stats: StatsStore,
    ) -> Self {
        let (notify_tx, _) = watch::channel(0);
        Self {
            inner: std::sync::Arc::new(Inner {
                state: Mutex::new(State {
                    next_id: 1,
                    max_concurrent,
                    generation: 0,
                    records: HashMap::new(),
                    tasks: HashMap::new(),
                }),
                notify_tx,
                ui_tx,
                stats,
            }),
        }
    }

    pub async fn cancel_all_running(&self, emit_ui_events: bool) -> Vec<String> {
        let (cancelled_ids, tasks) = {
            let mut state = self.inner.state.lock().expect("subagent state lock");
            let cancelled_ids = state
                .records
                .values()
                .filter(|record| record.status == SubagentStatus::Running)
                .map(|record| record.id.clone())
                .collect::<Vec<_>>();
            let mut cancelled_ids = cancelled_ids;
            cancelled_ids.sort();

            if cancelled_ids.is_empty() {
                return Vec::new();
            }

            for id in &cancelled_ids {
                if let Some(record) = state.records.get_mut(id) {
                    record.status = SubagentStatus::Cancelled;
                    record.output = None;
                    record.error = None;
                    record.failure_log_path = None;
                    record.last_activity_at = Instant::now();
                    record.waiting_for_approval = false;
                    record.pending_approval_request_id = None;
                }
            }

            let tasks = cancelled_ids
                .iter()
                .filter_map(|id| state.tasks.remove(id).map(|handle| (id.clone(), handle)))
                .collect::<Vec<_>>();

            self.bump_generation(&mut state);
            (cancelled_ids, tasks)
        };

        if emit_ui_events {
            for id in &cancelled_ids {
                let _ = self
                    .inner
                    .ui_tx
                    .send(SubagentUiEvent::Cancelled { id: id.clone() });
            }
        }

        for (_, handle) in &tasks {
            handle.abort();
        }

        for (_, handle) in tasks {
            let _ = handle.await;
        }

        cancelled_ids
    }

    pub fn inspect(&self, id: &str) -> Result<SubagentSnapshot> {
        let state = self.inner.state.lock().expect("subagent state lock");
        let record = state
            .records
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("Unknown subagent `{id}`."))?;
        Ok(snapshot_from_record(record))
    }

    pub(super) fn register_running(
        &self,
        access_mode: AccessMode,
        model_name: String,
    ) -> Result<String> {
        let now = Instant::now();
        let mut state = self.inner.state.lock().expect("subagent state lock");
        let active_count = state
            .records
            .values()
            .filter(|record| record.status == SubagentStatus::Running)
            .count();
        if active_count >= state.max_concurrent {
            bail!(
                "Subagent limit reached: {} running, max {}.",
                active_count,
                state.max_concurrent
            );
        }

        let id = format!("subagent-{}", state.next_id);
        state.next_id += 1;
        state.records.insert(
            id.clone(),
            SubagentRecord {
                id: id.clone(),
                status: SubagentStatus::Running,
                access_mode,
                model_name,
                latest_tool_name: None,
                output: None,
                error: None,
                failure_log_path: None,
                last_activity_at: now,
                waiting_for_approval: false,
                pending_approval_request_id: None,
            },
        );
        self.bump_generation(&mut state);
        Ok(id)
    }

    pub(super) fn insert_task_handle(&self, id: &str, handle: JoinHandle<()>) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if state
            .records
            .get(id)
            .is_some_and(|record| record.status == SubagentStatus::Running)
        {
            state.tasks.insert(id.to_string(), handle);
        }
    }

    pub(super) fn mark_activity(&self, id: &str) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id)
            && record.status == SubagentStatus::Running
        {
            record.last_activity_at = Instant::now();
            record.waiting_for_approval = false;
            record.pending_approval_request_id = None;
            self.bump_generation(&mut state);
            return true;
        }

        false
    }

    pub(super) fn record_tool_activity(&self, id: &str, tool_name: String) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id)
            && record.status == SubagentStatus::Running
        {
            record.last_activity_at = Instant::now();
            record.latest_tool_name = Some(tool_name.clone());
            record.waiting_for_approval = false;
            record.pending_approval_request_id = None;
            self.bump_generation(&mut state);
            drop(state);
            let _ = self.inner.ui_tx.send(SubagentUiEvent::Updated {
                id: id.to_string(),
                latest_tool_name: Some(tool_name),
            });
            return true;
        }

        false
    }

    pub(super) fn record_approval_wait(
        &self,
        id: &str,
        request_id: String,
        tool_name: String,
    ) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id)
            && record.status == SubagentStatus::Running
        {
            record.last_activity_at = Instant::now();
            record.latest_tool_name = Some(tool_name.clone());
            record.waiting_for_approval = true;
            record.pending_approval_request_id = Some(request_id);
            self.bump_generation(&mut state);
            drop(state);
            let _ = self.inner.ui_tx.send(SubagentUiEvent::Updated {
                id: id.to_string(),
                latest_tool_name: Some(tool_name),
            });
            return true;
        }

        false
    }

    pub(crate) fn clear_waiting_for_approval(&self, request_id: &str) -> bool {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        let mut cleared = false;
        for record in state.records.values_mut() {
            if record.status == SubagentStatus::Running
                && record.pending_approval_request_id.as_deref() == Some(request_id)
            {
                record.last_activity_at = Instant::now();
                record.waiting_for_approval = false;
                record.pending_approval_request_id = None;
                cleared = true;
            }
        }
        if cleared {
            self.bump_generation(&mut state);
        }
        cleared
    }

    pub(super) fn mark_completed(&self, id: &str, output: String) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id) {
            if record.status != SubagentStatus::Running {
                return;
            }
            record.status = SubagentStatus::Completed;
            record.output = Some(output);
            record.error = None;
            record.failure_log_path = None;
            record.last_activity_at = Instant::now();
            record.waiting_for_approval = false;
            record.pending_approval_request_id = None;
            state.tasks.remove(id);
            self.bump_generation(&mut state);
            let _ = self
                .inner
                .ui_tx
                .send(SubagentUiEvent::Completed { id: id.to_string() });
        }
    }

    #[cfg(test)]
    fn mark_failed(&self, id: &str, error: String, failure_log_path: Option<String>) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        if let Some(record) = state.records.get_mut(id) {
            if record.status != SubagentStatus::Running {
                return;
            }
            record.status = SubagentStatus::Failed;
            record.output = None;
            record.error = Some(error.clone());
            record.failure_log_path = failure_log_path.clone();
            record.last_activity_at = Instant::now();
            record.waiting_for_approval = false;
            record.pending_approval_request_id = None;
            state.tasks.remove(id);
            self.bump_generation(&mut state);
            let _ = self.inner.ui_tx.send(SubagentUiEvent::Failed {
                id: id.to_string(),
                error,
                log_path: failure_log_path,
            });
        }
    }

    pub(super) fn bump_generation(&self, state: &mut State) {
        state.generation = state.generation.wrapping_add(1);
        let _ = self.inner.notify_tx.send(state.generation);
    }

    #[cfg(test)]
    pub(crate) fn register_running_for_test(
        &self,
        id: &str,
        last_activity_ago: std::time::Duration,
    ) {
        let mut state = self.inner.state.lock().expect("subagent state lock");
        state.records.insert(
            id.to_string(),
            SubagentRecord {
                id: id.to_string(),
                status: SubagentStatus::Running,
                access_mode: AccessMode::ReadOnly,
                model_name: "gpt-5.4-mini".into(),
                latest_tool_name: None,
                output: None,
                error: None,
                failure_log_path: None,
                last_activity_at: Instant::now() - last_activity_ago,
                waiting_for_approval: false,
                pending_approval_request_id: None,
            },
        );
        self.bump_generation(&mut state);
    }

    #[cfg(test)]
    pub(crate) fn complete_for_test(&self, id: &str, output: &str) {
        self.mark_completed(id, output.to_string());
    }

    #[cfg(test)]
    pub(crate) fn fail_for_test(&self, id: &str, error: &str) {
        self.mark_failed(id, error.to_string(), None);
    }

    #[cfg(test)]
    pub(crate) async fn cancel_all_running_for_test(&self) -> Vec<String> {
        self.cancel_all_running(true).await
    }

    #[cfg(test)]
    pub(crate) fn mark_activity_for_test(&self, id: &str) {
        self.mark_activity(id);
    }

    #[cfg(test)]
    pub(crate) fn record_approval_wait_for_test(
        &self,
        id: &str,
        request_id: &str,
        tool_name: &str,
    ) {
        self.record_approval_wait(id, request_id.to_string(), tool_name.to_string());
    }

    #[cfg(test)]
    pub(crate) fn clear_waiting_for_approval_for_test(&self, request_id: &str) {
        self.clear_waiting_for_approval(request_id);
    }
}
