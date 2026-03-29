use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use chrono::{DateTime, Utc};
use portable_pty::ChildKiller;
use tokio::sync::{mpsc, watch};

use crate::background_terminals::buffer::TokenTailBuffer;

use super::{
    BackgroundTerminalSnapshot, BackgroundTerminalStatus, BackgroundTerminalUiEvent,
    TerminalExitInfo,
};

pub(super) struct Inner {
    pub(super) state: Mutex<State>,
    pub(super) notify_tx: watch::Sender<u64>,
    pub(super) ui_tx: mpsc::UnboundedSender<BackgroundTerminalUiEvent>,
}

pub(super) struct State {
    pub(super) next_id: u64,
    pub(super) max_running: usize,
    pub(super) max_finished: usize,
    pub(super) generation: u64,
    pub(super) records: HashMap<String, TerminalRecord>,
    pub(super) killers: HashMap<String, Box<dyn ChildKiller + Send + Sync>>,
    pub(super) finished_order: VecDeque<String>,
}

pub(super) struct TerminalRecord {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) status: BackgroundTerminalStatus,
    pub(super) cwd: String,
    pub(super) pid: Option<u32>,
    pub(super) started_at: DateTime<Utc>,
    pub(super) ended_at: Option<DateTime<Utc>>,
    pub(super) last_activity_at: DateTime<Utc>,
    pub(super) exit_info: Option<TerminalExitInfo>,
    pub(super) error: Option<String>,
    pub(super) output: TokenTailBuffer,
}

impl State {
    pub(super) fn running_count(&self) -> usize {
        self.records
            .values()
            .filter(|record| record.status == BackgroundTerminalStatus::Running)
            .count()
    }
}

pub(super) fn snapshot_from_record(record: &TerminalRecord) -> BackgroundTerminalSnapshot {
    BackgroundTerminalSnapshot {
        id: record.id.clone(),
        label: record.label.clone(),
        status: record.status,
        cwd: record.cwd.clone(),
        pid: record.pid,
        started_at: record.started_at,
        ended_at: record.ended_at,
        last_activity_at: record.last_activity_at,
        retained_output_tokens: record.output.retained_tokens(),
        output_sequence: record.output.sequence(),
        output_truncated: record.output.output_truncated(),
        exit_info: record.exit_info.clone(),
        error: record.error.clone(),
    }
}
