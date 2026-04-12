use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::{
        AccessMode, AppState, ApprovalMode, SessionHistoryMessage, SessionState, TranscriptEntry,
        UiState,
    },
    config::{HistoryMode, ReasoningSetting},
    features::planning::{PlanningAgentConfig, PlanningFeatureState},
    model_registry,
    todo::TodoSnapshot,
};

const SESSIONS_DIR_RELATIVE_PATH: &str = ".config/oat/sessions";
const SNAPSHOT_FILE_NAME: &str = "snapshot.json";
const EVENTS_FILE_NAME: &str = "events.jsonl";
const SNAPSHOT_TMP_FILE_NAME: &str = "snapshot.json.tmp";
const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistedSessionInterface {
    Tui,
    Headless,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistedSessionStatus {
    Active,
    Finalized,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PersistedSessionRuntimeState {
    pub title: Option<String>,
    pub model_name: String,
    pub reasoning: ReasoningSetting,
    pub safety_model_name: String,
    pub safety_reasoning: ReasoningSetting,
    pub memory_model_name: String,
    pub memory_reasoning: ReasoningSetting,
    pub planning_agents: Vec<PlanningAgentConfig>,
    pub access_mode: AccessMode,
    pub approval_mode: ApprovalMode,
    pub show_thinking: bool,
    pub show_tool_output: bool,
    pub history_mode: HistoryMode,
    pub history_retained_steps: usize,
    pub preamble_text: String,
    pub session_history: Vec<SessionHistoryMessage>,
    pub last_history_model_name: Option<String>,
    pub planning: PlanningFeatureState,
    pub current_todo: Option<TodoSnapshot>,
    pub next_reply_id: u64,
    pub next_side_channel_label_id: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PersistedSessionSnapshot {
    pub schema_version: u32,
    pub session_id: String,
    pub interface: PersistedSessionInterface,
    pub workspace_root: PathBuf,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    pub finished_at_unix_ms: Option<u64>,
    pub status: PersistedSessionStatus,
    pub transcript: Vec<TranscriptEntry>,
    pub runtime: PersistedSessionRuntimeState,
    pub last_applied_event_seq: u64,
}

impl PersistedSessionSnapshot {
    fn new_from_app_state(
        state: &AppState,
        session_id: String,
        interface: PersistedSessionInterface,
        started_at_unix_ms: u64,
        preamble_text: String,
    ) -> Self {
        let session_history = crate::app::ops::session::sanitize_session_history_messages(
            state.session.session_history.clone(),
        );
        Self {
            schema_version: SCHEMA_VERSION,
            session_id,
            interface,
            workspace_root: state.session.workspace_root.clone(),
            started_at_unix_ms,
            updated_at_unix_ms: started_at_unix_ms,
            finished_at_unix_ms: None,
            status: PersistedSessionStatus::Active,
            transcript: state.session.entries.clone(),
            runtime: PersistedSessionRuntimeState {
                title: state.session.session_title.clone(),
                model_name: state.session.model_name.clone(),
                reasoning: state.session.reasoning,
                safety_model_name: state.session.safety_model_name.clone(),
                safety_reasoning: state.session.safety_reasoning,
                memory_model_name: state.session.memory_model_name.clone(),
                memory_reasoning: state.session.memory_reasoning,
                planning_agents: state.session.planning_agents.clone(),
                access_mode: state.session.mode,
                approval_mode: state.session.approval_mode,
                show_thinking: state.session.show_thinking,
                show_tool_output: state.session.show_tool_output,
                history_mode: state.session.history_mode,
                history_retained_steps: state.session.history_retained_steps,
                preamble_text,
                session_history,
                last_history_model_name: state.session.last_history_model_name.clone(),
                planning: state.session.planning.clone(),
                current_todo: state.session.current_todo.clone(),
                next_reply_id: state.session.next_reply_id,
                next_side_channel_label_id: state.session.next_side_channel_label_id,
            },
            last_applied_event_seq: 0,
        }
    }

    pub fn into_app_state(
        self,
        initial_mode: AccessMode,
        initial_approval_mode: ApprovalMode,
    ) -> AppState {
        let current_todo = self.runtime.current_todo.clone();
        let sanitized_history = crate::app::ops::session::apply_current_todo_to_history(
            crate::app::ops::session::sanitize_session_history_messages(
                self.runtime.session_history.clone(),
            ),
            current_todo.as_ref(),
        );
        let mut session = SessionState::with_startup(
            self.runtime.show_thinking,
            self.runtime.show_tool_output,
            self.runtime.model_name.clone(),
            self.runtime.reasoning,
            self.runtime.planning_agents.clone(),
            initial_mode,
            initial_approval_mode,
        );
        session.workspace_root = self.workspace_root;
        session.mode = self.runtime.access_mode;
        session.approval_mode = self.runtime.approval_mode;
        session.history_mode = self.runtime.history_mode;
        session.history_retained_steps = self.runtime.history_retained_steps;
        session.entries = self.transcript;
        session.transcript_revision = 0;
        session.current_todo = current_todo;
        session.replace_session_history(sanitized_history);
        session.last_history_model_name = self.runtime.last_history_model_name;
        session.session_title = self.runtime.title;
        session.reasoning = self.runtime.reasoning;
        session.model_name = self.runtime.model_name;
        session.safety_model_name = self.runtime.safety_model_name;
        session.safety_reasoning = self.runtime.safety_reasoning;
        session.memory_model_name = self.runtime.memory_model_name;
        session.memory_reasoning = self.runtime.memory_reasoning;
        session.planning_agents = self.runtime.planning_agents;
        session.planning = self.runtime.planning;
        session.next_reply_id = self.runtime.next_reply_id;
        session.next_side_channel_label_id = self.runtime.next_side_channel_label_id;
        session.pending_reply = None;
        session.pending_write_approvals.clear();
        session.pending_shell_approvals.clear();
        session.pending_ask_user = None;
        session.pending_side_replies.clear();
        session.active_main_request_seed = None;
        session.active_background_terminal_count = 0;
        session.queued_messages.clear();

        AppState::new(session, UiState::default())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum SessionJournalEvent {
    SessionStarted {
        snapshot: PersistedSessionSnapshot,
    },
    TranscriptAppended {
        entries: Vec<TranscriptEntry>,
    },
    TranscriptReplaced {
        index: usize,
        entry: TranscriptEntry,
    },
    TranscriptTruncated {
        len: usize,
    },
    TranscriptReset {
        entries: Vec<TranscriptEntry>,
    },
    RuntimeStateUpdated {
        runtime: PersistedSessionRuntimeState,
    },
    SessionFinalized {
        finished_at_unix_ms: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SessionJournalRecord {
    schema_version: u32,
    session_id: String,
    seq: u64,
    ts_unix_ms: u64,
    #[serde(flatten)]
    event: SessionJournalEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionListEntry {
    pub session_id: String,
    pub title: String,
    pub detail: String,
    pub resumable: bool,
}

#[derive(Debug)]
struct LiveSessionState {
    session_id: String,
    interface: PersistedSessionInterface,
    started_at_unix_ms: u64,
    activated: bool,
    next_seq: u64,
    baseline_snapshot: PersistedSessionSnapshot,
    current_snapshot: PersistedSessionSnapshot,
}

impl LiveSessionState {
    fn new_tui(state: &AppState, preamble_text: &str) -> Self {
        let session_id = Uuid::now_v7().to_string();
        let started_at_unix_ms = unix_timestamp_ms();
        let baseline_snapshot = PersistedSessionSnapshot::new_from_app_state(
            state,
            session_id.clone(),
            PersistedSessionInterface::Tui,
            started_at_unix_ms,
            preamble_text.to_string(),
        );
        Self {
            session_id,
            interface: PersistedSessionInterface::Tui,
            started_at_unix_ms,
            activated: false,
            next_seq: 1,
            current_snapshot: baseline_snapshot.clone(),
            baseline_snapshot,
        }
    }

    fn from_snapshot(snapshot: PersistedSessionSnapshot) -> Self {
        let mut current_snapshot = snapshot.clone();
        current_snapshot.finished_at_unix_ms = None;
        current_snapshot.status = PersistedSessionStatus::Active;
        Self {
            session_id: current_snapshot.session_id.clone(),
            interface: current_snapshot.interface,
            started_at_unix_ms: current_snapshot.started_at_unix_ms,
            activated: true,
            next_seq: current_snapshot.last_applied_event_seq + 1,
            baseline_snapshot: current_snapshot.clone(),
            current_snapshot,
        }
    }
}

pub struct SessionStore {
    root_dir: Option<PathBuf>,
    live: LiveSessionState,
    disabled: bool,
}

impl SessionStore {
    pub fn new_tui(state: &AppState, preamble_text: &str) -> Self {
        Self {
            root_dir: default_sessions_dir(),
            live: LiveSessionState::new_tui(state, preamble_text),
            disabled: false,
        }
    }

    pub fn sync_tui(&mut self, state: &AppState, preamble_text: &str) -> Result<()> {
        if self.disabled {
            return Ok(());
        }

        let result = (|| {
            let candidate = PersistedSessionSnapshot::new_from_app_state(
                state,
                self.live.session_id.clone(),
                self.live.interface,
                self.live.started_at_unix_ms,
                preamble_text.to_string(),
            );
            let events = diff_snapshot(&self.live.current_snapshot, &candidate);
            if !self.live.activated {
                if events.is_empty() {
                    return Ok(());
                }
                self.activate_live_session()?;
            } else if events.is_empty() {
                return Ok(());
            }

            for event in events {
                self.append_event(event)?;
            }
            self.write_snapshot()?;
            Ok(())
        })();
        if result.is_err() {
            self.disabled = true;
        }
        result
    }

    pub fn rotate_to_new_tui_session(
        &mut self,
        state: &AppState,
        preamble_text: &str,
    ) -> Result<()> {
        if !self.disabled {
            self.finalize_current_session()?;
        }
        self.disabled = false;
        self.live = LiveSessionState::new_tui(state, preamble_text);
        Ok(())
    }

    pub fn finalize_current_session(&mut self) -> Result<()> {
        if self.disabled || !self.live.activated {
            return Ok(());
        }
        if self.live.current_snapshot.finished_at_unix_ms.is_some() {
            return Ok(());
        }

        let result = (|| {
            let finished_at_unix_ms = unix_timestamp_ms();
            self.append_event(SessionJournalEvent::SessionFinalized {
                finished_at_unix_ms,
            })?;
            self.write_snapshot()?;
            Ok(())
        })();
        if result.is_err() {
            self.disabled = true;
        }
        result
    }

    pub fn list_sessions_for_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<Vec<SessionListEntry>> {
        let Some(root_dir) = self.root_dir.as_deref() else {
            return Ok(Vec::new());
        };
        if !root_dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(root_dir)
            .with_context(|| format!("failed to read {}", root_dir.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", root_dir.display()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let snapshot_path = path.join(SNAPSHOT_FILE_NAME);
            if !snapshot_path.exists() {
                continue;
            }
            let snapshot = match read_snapshot(&snapshot_path) {
                Ok(snapshot) => snapshot,
                Err(_) => continue,
            };
            if snapshot.workspace_root != workspace_root {
                continue;
            }
            let resumable = model_registry::find_model(&snapshot.runtime.model_name).is_some()
                && model_registry::find_model(&snapshot.runtime.safety_model_name).is_some()
                && model_registry::find_model(&snapshot.runtime.memory_model_name).is_some();
            let reason_suffix = if resumable {
                String::new()
            } else {
                " | saved model unavailable".to_string()
            };
            let updated_at = format_timestamp(snapshot.updated_at_unix_ms);
            let title = snapshot
                .runtime
                .title
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "Untitled session".to_string());
            entries.push((
                snapshot.updated_at_unix_ms,
                SessionListEntry {
                    session_id: snapshot.session_id.clone(),
                    title,
                    detail: format!(
                        "Last active {} | {}{}",
                        updated_at, snapshot.runtime.model_name, reason_suffix
                    ),
                    resumable,
                },
            ));
        }

        entries.sort_by(|left, right| right.0.cmp(&left.0));
        Ok(entries.into_iter().map(|(_, entry)| entry).collect())
    }

    pub fn load_session(&self, session_id: &str) -> Result<PersistedSessionSnapshot> {
        let Some(root_dir) = self.root_dir.as_deref() else {
            return Err(anyhow!("session storage is unavailable"));
        };
        let session_dir = root_dir.join(session_id);
        let snapshot_path = session_dir.join(SNAPSHOT_FILE_NAME);
        let events_path = session_dir.join(EVENTS_FILE_NAME);

        let mut snapshot = if snapshot_path.exists() {
            read_snapshot(&snapshot_path)?
        } else {
            rebuild_snapshot_from_journal(&events_path)?
        };
        if snapshot.schema_version != SCHEMA_VERSION {
            return Err(anyhow!(
                "session `{session_id}` uses unsupported schema version {}",
                snapshot.schema_version
            ));
        }
        replay_journal_tail(&events_path, &mut snapshot)?;
        sanitize_persisted_snapshot(&mut snapshot);
        Ok(snapshot)
    }

    pub fn attach_resumed_session(&mut self, mut snapshot: PersistedSessionSnapshot) {
        self.disabled = false;
        sanitize_persisted_snapshot(&mut snapshot);
        self.live = LiveSessionState::from_snapshot(snapshot);
    }

    pub fn current_session_id(&self) -> &str {
        &self.live.session_id
    }

    fn activate_live_session(&mut self) -> Result<()> {
        self.ensure_live_session_dir()?;
        self.append_event(SessionJournalEvent::SessionStarted {
            snapshot: self.live.baseline_snapshot.clone(),
        })?;
        self.write_snapshot()?;
        self.live.activated = true;
        Ok(())
    }

    fn append_event(&mut self, event: SessionJournalEvent) -> Result<()> {
        let Some(root_dir) = self.root_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(root_dir)
            .with_context(|| format!("failed to create {}", root_dir.display()))?;

        let session_dir = root_dir.join(&self.live.session_id);
        fs::create_dir_all(&session_dir)
            .with_context(|| format!("failed to create {}", session_dir.display()))?;

        let ts_unix_ms = unix_timestamp_ms();
        let seq = self.live.next_seq;
        let record = SessionJournalRecord {
            schema_version: SCHEMA_VERSION,
            session_id: self.live.session_id.clone(),
            seq,
            ts_unix_ms,
            event,
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(session_dir.join(EVENTS_FILE_NAME))
            .with_context(|| {
                format!(
                    "failed to open {}",
                    session_dir.join(EVENTS_FILE_NAME).display()
                )
            })?;
        let encoded = serde_json::to_string(&record).with_context(|| {
            format!(
                "failed to serialize event for session {}",
                self.live.session_id
            )
        })?;
        writeln!(file, "{encoded}").with_context(|| {
            format!(
                "failed to append {}",
                session_dir.join(EVENTS_FILE_NAME).display()
            )
        })?;
        apply_event_to_snapshot(&mut self.live.current_snapshot, &record)?;
        self.live.next_seq += 1;
        Ok(())
    }

    fn write_snapshot(&self) -> Result<()> {
        if !self.live.activated {
            return Ok(());
        }
        let Some(root_dir) = self.root_dir.as_deref() else {
            return Ok(());
        };
        let session_dir = root_dir.join(&self.live.session_id);
        fs::create_dir_all(&session_dir)
            .with_context(|| format!("failed to create {}", session_dir.display()))?;
        let snapshot_path = session_dir.join(SNAPSHOT_FILE_NAME);
        let tmp_path = session_dir.join(SNAPSHOT_TMP_FILE_NAME);
        let payload = serde_json::to_string_pretty(&self.live.current_snapshot)
            .with_context(|| format!("failed to serialize {}", snapshot_path.display()))?;
        fs::write(&tmp_path, payload)
            .with_context(|| format!("failed to write {}", tmp_path.display()))?;
        fs::rename(&tmp_path, &snapshot_path).with_context(|| {
            format!(
                "failed to move {} into place at {}",
                tmp_path.display(),
                snapshot_path.display()
            )
        })?;
        Ok(())
    }

    fn ensure_live_session_dir(&self) -> Result<()> {
        let Some(root_dir) = self.root_dir.as_deref() else {
            return Ok(());
        };
        fs::create_dir_all(root_dir)
            .with_context(|| format!("failed to create {}", root_dir.display()))?;
        let session_dir = root_dir.join(&self.live.session_id);
        fs::create_dir_all(&session_dir)
            .with_context(|| format!("failed to create {}", session_dir.display()))?;
        Ok(())
    }
}

fn diff_snapshot(
    previous: &PersistedSessionSnapshot,
    current: &PersistedSessionSnapshot,
) -> Vec<SessionJournalEvent> {
    let mut events = Vec::new();

    if let Some(event) = diff_transcript(&previous.transcript, &current.transcript) {
        events.push(event);
    }
    if previous.runtime != current.runtime {
        events.push(SessionJournalEvent::RuntimeStateUpdated {
            runtime: current.runtime.clone(),
        });
    }

    events
}

fn diff_transcript(
    previous: &[TranscriptEntry],
    current: &[TranscriptEntry],
) -> Option<SessionJournalEvent> {
    if previous == current {
        return None;
    }
    if current.starts_with(previous) {
        return Some(SessionJournalEvent::TranscriptAppended {
            entries: current[previous.len()..].to_vec(),
        });
    }
    if previous.starts_with(current) {
        return Some(SessionJournalEvent::TranscriptTruncated { len: current.len() });
    }
    if previous.len() == current.len() {
        let differing = previous
            .iter()
            .zip(current.iter())
            .enumerate()
            .filter(|(_, (left, right))| left != right)
            .collect::<Vec<_>>();
        if differing.len() == 1 {
            let (index, (_, entry)) = differing[0];
            return Some(SessionJournalEvent::TranscriptReplaced {
                index,
                entry: (*entry).clone(),
            });
        }
    }

    Some(SessionJournalEvent::TranscriptReset {
        entries: current.to_vec(),
    })
}

fn apply_event_to_snapshot(
    snapshot: &mut PersistedSessionSnapshot,
    record: &SessionJournalRecord,
) -> Result<()> {
    if record.schema_version != SCHEMA_VERSION {
        return Err(anyhow!(
            "unsupported journal schema version {}",
            record.schema_version
        ));
    }
    if record.session_id != snapshot.session_id {
        return Err(anyhow!(
            "session id mismatch: journal has `{}`, snapshot has `{}`",
            record.session_id,
            snapshot.session_id
        ));
    }

    match &record.event {
        SessionJournalEvent::SessionStarted { snapshot: seeded } => {
            *snapshot = seeded.clone();
        }
        SessionJournalEvent::TranscriptAppended { entries } => {
            snapshot.transcript.extend(entries.clone());
        }
        SessionJournalEvent::TranscriptReplaced { index, entry } => {
            if *index >= snapshot.transcript.len() {
                return Err(anyhow!(
                    "cannot replace transcript entry {} in len {}",
                    index,
                    snapshot.transcript.len()
                ));
            }
            snapshot.transcript[*index] = entry.clone();
        }
        SessionJournalEvent::TranscriptTruncated { len } => {
            snapshot.transcript.truncate(*len);
        }
        SessionJournalEvent::TranscriptReset { entries } => {
            snapshot.transcript = entries.clone();
        }
        SessionJournalEvent::RuntimeStateUpdated { runtime } => {
            snapshot.runtime = runtime.clone();
        }
        SessionJournalEvent::SessionFinalized {
            finished_at_unix_ms,
        } => {
            snapshot.finished_at_unix_ms = Some(*finished_at_unix_ms);
            snapshot.status = PersistedSessionStatus::Finalized;
        }
    }

    snapshot.updated_at_unix_ms = record.ts_unix_ms;
    snapshot.last_applied_event_seq = record.seq;
    Ok(())
}

fn replay_journal_tail(events_path: &Path, snapshot: &mut PersistedSessionSnapshot) -> Result<()> {
    if !events_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(events_path)
        .with_context(|| format!("failed to read {}", events_path.display()))?;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionJournalRecord = serde_json::from_str(line)
            .with_context(|| format!("failed to parse {}", events_path.display()))?;
        if record.seq <= snapshot.last_applied_event_seq {
            continue;
        }
        apply_event_to_snapshot(snapshot, &record)?;
    }
    Ok(())
}

fn rebuild_snapshot_from_journal(events_path: &Path) -> Result<PersistedSessionSnapshot> {
    let raw = fs::read_to_string(events_path)
        .with_context(|| format!("failed to read {}", events_path.display()))?;
    let mut snapshot = None;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let record: SessionJournalRecord = serde_json::from_str(line)
            .with_context(|| format!("failed to parse {}", events_path.display()))?;
        match (&mut snapshot, &record.event) {
            (None, SessionJournalEvent::SessionStarted { snapshot: seeded }) => {
                snapshot = Some(seeded.clone());
                if let Some(snapshot) = snapshot.as_mut() {
                    snapshot.last_applied_event_seq = 0;
                    apply_event_to_snapshot(snapshot, &record)?;
                }
            }
            (Some(snapshot), _) => apply_event_to_snapshot(snapshot, &record)?,
            (None, _) => {
                return Err(anyhow!(
                    "journal {} does not start with session_started",
                    events_path.display()
                ));
            }
        }
    }
    snapshot.ok_or_else(|| anyhow!("journal {} is empty", events_path.display()))
}

fn read_snapshot(path: &Path) -> Result<PersistedSessionSnapshot> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

fn sanitize_persisted_snapshot(snapshot: &mut PersistedSessionSnapshot) {
    let history = crate::app::ops::session::sanitize_session_history_messages(std::mem::take(
        &mut snapshot.runtime.session_history,
    ));
    let history = crate::app::ops::session::reduce_session_history_messages(
        history,
        snapshot.runtime.history_mode,
        snapshot.runtime.history_retained_steps,
        true,
    );
    snapshot.runtime.session_history = crate::app::ops::session::apply_current_todo_to_history(
        history,
        snapshot.runtime.current_todo.as_ref(),
    );
}

fn default_sessions_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| PathBuf::from(home).join(SESSIONS_DIR_RELATIVE_PATH))
}

fn unix_timestamp_ms() -> u64 {
    Utc::now().timestamp_millis() as u64
}

fn format_timestamp(unix_ms: u64) -> String {
    format_timestamp_relative(unix_ms, Utc::now())
}

fn format_timestamp_relative(unix_ms: u64, now: DateTime<Utc>) -> String {
    let Some(timestamp) = Utc.timestamp_millis_opt(unix_ms as i64).single() else {
        return unix_ms.to_string();
    };
    if timestamp > now {
        return timestamp.format("%b %-d, %Y at %H:%M UTC").to_string();
    }

    let day_diff = now
        .date_naive()
        .signed_duration_since(timestamp.date_naive())
        .num_days();
    let time = timestamp.format("%H:%M UTC");

    match day_diff {
        0 => format!("Today at {time}"),
        1 => format!("Yesterday at {time}"),
        2..=7 => format!("{day_diff} days ago at {time}"),
        _ => timestamp.format("%b %-d, %Y at %H:%M UTC").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{
        ChatMessage, CommandRisk, MessageStyle, PendingReply, PendingReplyKind, Speaker,
        session::test_support::{new_app, registry_app},
    };
    use crate::ask_user::{AskUserAnswer, AskUserQuestion, AskUserRequest};
    use crate::llm::history_into_rig;
    use crate::todo::{TodoSnapshot, TodoStatus, TodoTask};
    use rig::{
        OneOrMany,
        completion::{
            Message as RigMessage,
            message::{AssistantContent, ToolCall, ToolFunction},
        },
    };
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "oat-session-store-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("timestamp")
                .as_nanos()
        ))
    }

    fn store_with_root(root: &Path, app: &crate::app::App) -> SessionStore {
        let mut store = SessionStore::new_tui(app.state(), "preamble");
        store.root_dir = Some(root.to_path_buf());
        store
    }

    fn utc_datetime(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn sync_skips_unmodified_startup_state() {
        let app = new_app(true);
        let root = temp_root("startup");
        let mut store = store_with_root(&root, &app);

        store.sync_tui(app.state(), "preamble").expect("sync");

        assert!(!root.exists());
    }

    #[test]
    fn sync_persists_first_meaningful_change() {
        let mut app = new_app(true);
        let root = temp_root("first-change");
        let mut store = store_with_root(&root, &app);
        crate::app::ops::transcript::push_user_message(app.state_mut(), "hello");

        store.sync_tui(app.state(), "preamble").expect("sync");

        let session_dir = root.join(&store.live.session_id);
        assert!(session_dir.join(EVENTS_FILE_NAME).exists());
        assert!(session_dir.join(SNAPSHOT_FILE_NAME).exists());
    }

    #[test]
    fn load_session_replays_journal_tail_after_snapshot() {
        let mut app = new_app(true);
        let root = temp_root("replay-tail");
        let mut store = store_with_root(&root, &app);
        crate::app::ops::transcript::push_user_message(app.state_mut(), "hello");
        store.sync_tui(app.state(), "preamble").expect("first sync");
        crate::app::ops::transcript::push_agent_message(app.state_mut(), "world");
        store
            .sync_tui(app.state(), "preamble")
            .expect("second sync");

        let snapshot_path = root.join(&store.live.session_id).join(SNAPSHOT_FILE_NAME);
        let mut snapshot = read_snapshot(&snapshot_path).expect("snapshot");
        snapshot.transcript.pop();
        snapshot.last_applied_event_seq -= 1;
        fs::write(
            &snapshot_path,
            serde_json::to_string_pretty(&snapshot).expect("serialize"),
        )
        .expect("write stale snapshot");

        let loaded = store.load_session(&store.live.session_id).expect("load");
        assert!(matches!(
            loaded.transcript.last(),
            Some(TranscriptEntry::Message(ChatMessage {
                speaker: Speaker::Agent,
                text,
                style: MessageStyle::Plain,
                tag: None,
            })) if text == "world"
        ));
    }

    #[test]
    fn list_sessions_filters_by_workspace_root() {
        let mut app = registry_app(true);
        let root = temp_root("list");
        app.set_workspace_root(PathBuf::from("/tmp/a"));
        let mut store = store_with_root(&root, &app);
        crate::app::ops::transcript::push_user_message(app.state_mut(), "hello");
        store.sync_tui(app.state(), "preamble").expect("sync");

        let sessions = store
            .list_sessions_for_workspace(Path::new("/tmp/a"))
            .expect("list");
        assert_eq!(sessions.len(), 1);

        let none = store
            .list_sessions_for_workspace(Path::new("/tmp/b"))
            .expect("list");
        assert!(none.is_empty());
    }

    #[test]
    fn attach_resumed_session_reuses_existing_session_id() {
        let mut app = new_app(true);
        let root = temp_root("resume");
        let mut store = store_with_root(&root, &app);
        crate::app::ops::transcript::push_user_message(app.state_mut(), "hello");
        store.sync_tui(app.state(), "preamble").expect("sync");
        let session_id = store.live.session_id.clone();

        let snapshot = store.load_session(&session_id).expect("load");
        store.attach_resumed_session(snapshot.clone());
        crate::app::ops::transcript::push_agent_message(app.state_mut(), "resumed");
        store
            .sync_tui(app.state(), "preamble-2")
            .expect("sync resumed");

        assert_eq!(store.live.session_id, session_id);
        let loaded = store.load_session(&session_id).expect("load updated");
        assert_eq!(loaded.runtime.preamble_text, "preamble-2");
    }

    #[test]
    fn load_session_sanitizes_unmatched_multi_item_tool_calls_in_history() {
        let app = new_app(true);
        let root = temp_root("sanitize-history");
        let store = store_with_root(&root, &app);

        let mut snapshot = PersistedSessionSnapshot::new_from_app_state(
            app.state(),
            "sanitize-history".into(),
            PersistedSessionInterface::Tui,
            1,
            "preamble".into(),
        );
        snapshot.runtime.session_history = vec![
            SessionHistoryMessage::user("first user"),
            SessionHistoryMessage::from_rig_message(RigMessage::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-1".into(),
                        call_id: Some("call-1".into()),
                        function: ToolFunction::new(
                            "Commentary".into(),
                            json!({"message":"checking files"}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: "tool-2".into(),
                        call_id: Some("call-2".into()),
                        function: ToolFunction::new(
                            "Todo".into(),
                            json!({"operation":"create","tasks":[]}),
                        ),
                        signature: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple assistant content items"),
            })
            .expect("assistant message"),
            SessionHistoryMessage::assistant("after tool calls"),
        ];

        let session_dir = root.join("sanitize-history");
        fs::create_dir_all(&session_dir).expect("create session dir");
        fs::write(
            session_dir.join(SNAPSHOT_FILE_NAME),
            serde_json::to_string_pretty(&snapshot).expect("serialize snapshot"),
        )
        .expect("write snapshot");

        let loaded = store
            .load_session("sanitize-history")
            .expect("load session");
        let rig_history = history_into_rig(loaded.runtime.session_history).expect("rig history");

        assert_eq!(
            rig_history,
            vec![
                RigMessage::user("first user"),
                RigMessage::assistant("after tool calls"),
            ]
        );
    }

    #[test]
    fn load_session_keeps_only_latest_current_todo_summary() {
        let app = new_app(true);
        let root = temp_root("todo-summary");
        let store = store_with_root(&root, &app);

        let mut snapshot = PersistedSessionSnapshot::new_from_app_state(
            app.state(),
            "todo-summary".into(),
            PersistedSessionInterface::Tui,
            1,
            "preamble".into(),
        );
        snapshot.runtime.current_todo = Some(TodoSnapshot::new(vec![TodoTask {
            description: "Write regression coverage".into(),
            status: TodoStatus::Todo,
        }]));
        snapshot.runtime.session_history = vec![
            SessionHistoryMessage::assistant("old"),
            SessionHistoryMessage::assistant("[oat-todo] [in progress] stale task"),
            SessionHistoryMessage::assistant("[oat-todo] [done] older task"),
        ];

        let session_dir = root.join("todo-summary");
        fs::create_dir_all(&session_dir).expect("create session dir");
        fs::write(
            session_dir.join(SNAPSHOT_FILE_NAME),
            serde_json::to_string_pretty(&snapshot).expect("serialize snapshot"),
        )
        .expect("write snapshot");

        let loaded = store.load_session("todo-summary").expect("load session");
        let rig_history = history_into_rig(loaded.runtime.session_history).expect("rig history");

        assert_eq!(
            rig_history,
            vec![
                RigMessage::assistant("old"),
                RigMessage::assistant("[oat-todo] [todo] Write regression coverage"),
            ]
        );
    }

    #[test]
    fn finalize_marks_snapshot_finished() {
        let mut app = new_app(true);
        let root = temp_root("finalize");
        let mut store = store_with_root(&root, &app);
        crate::app::ops::transcript::push_user_message(app.state_mut(), "hello");
        store.sync_tui(app.state(), "preamble").expect("sync");

        store.finalize_current_session().expect("finalize");

        let loaded = store.load_session(&store.live.session_id).expect("load");
        assert_eq!(loaded.status, PersistedSessionStatus::Finalized);
        assert!(loaded.finished_at_unix_ms.is_some());
    }

    #[test]
    fn into_app_state_clears_in_flight_runtime_state() {
        let mut app = new_app(true);
        app.state_mut().session.pending_reply =
            Some(PendingReply::new(42, PendingReplyKind::Normal));
        app.state_mut().session.pending_write_approvals.push_back(
            crate::app::PendingWriteApproval {
                request_id: "write-1".into(),
                tool_name: "WriteFile".into(),
                arguments: "{}".into(),
                summary: "write".into(),
                target: Some("src/main.rs".into()),
                source_label: None,
            },
        );
        app.state_mut().session.enqueue_shell_approval(
            None,
            "shell-1".into(),
            CommandRisk::Medium,
            "command writes to disk".into(),
            "touch file.txt".into(),
            "/root/oat".into(),
            "create file".into(),
        );
        crate::app::ops::ask_user::begin_ask_user(
            app.state_mut(),
            "ask-1".into(),
            AskUserRequest {
                title: Some("Clarify".into()),
                questions: vec![AskUserQuestion {
                    id: "scope".into(),
                    prompt: "Which scope?".into(),
                    answers: vec![AskUserAnswer {
                        id: "narrow".into(),
                        label: "Narrow".into(),
                    }],
                }],
            },
        );
        app.state_mut().session.pending_side_replies.insert(
            7,
            crate::app::PendingSideReply {
                kind: crate::app::SideChannelKind::Btw,
                label: "btw #1".into(),
            },
        );
        app.state_mut()
            .session
            .queued_messages
            .push_back("queued".into());
        app.state_mut().session.active_main_request_seed = Some(crate::app::MainRequestSeed {
            history: vec![SessionHistoryMessage::user("prior")],
            visible_prompt: "continue".into(),
            model_prompt: "continue".into(),
            history_model_name: Some("gpt-5-mini".into()),
            transcript_len_before: 0,
        });
        app.state_mut().session.active_background_terminal_count = 3;
        let pending_shell = crate::app::PendingShellApproval::new(
            "shell-ui".into(),
            CommandRisk::Medium,
            "risk".into(),
            "touch file.txt".into(),
            "/root/oat".into(),
            "create file".into(),
            None,
        );
        app.state_mut().ui.pending_shell_approval =
            Some(crate::app::ui::ShellApprovalUiState::new(&pending_shell));

        let snapshot = PersistedSessionSnapshot::new_from_app_state(
            app.state(),
            "session-1".into(),
            PersistedSessionInterface::Tui,
            1,
            "preamble".into(),
        );
        let restored = snapshot.into_app_state(AccessMode::ReadOnly, ApprovalMode::Manual);

        assert!(restored.session.pending_reply.is_none());
        assert!(restored.session.pending_write_approvals.is_empty());
        assert!(restored.session.pending_shell_approvals.is_empty());
        assert!(restored.session.pending_ask_user.is_none());
        assert!(restored.session.pending_side_replies.is_empty());
        assert!(restored.session.queued_messages.is_empty());
        assert!(restored.session.active_main_request_seed.is_none());
        assert_eq!(restored.session.active_background_terminal_count, 0);
        assert!(restored.ui.pending_shell_approval.is_none());
        assert!(restored.ui.pending_ask_user.is_none());
        assert!(restored.ui.picker.is_none());
    }

    #[test]
    fn list_sessions_marks_unavailable_model_selections_as_non_resumable() {
        let mut app = registry_app(true);
        let root = temp_root("unavailable");
        app.set_workspace_root(PathBuf::from("/tmp/a"));

        let mut missing_main = PersistedSessionSnapshot::new_from_app_state(
            app.state(),
            "missing-main".into(),
            PersistedSessionInterface::Tui,
            10,
            "preamble".into(),
        );
        missing_main.runtime.title = Some("Missing main".into());
        missing_main.runtime.model_name = "missing/main-model".into();
        missing_main.updated_at_unix_ms = 20;

        let mut missing_safety = PersistedSessionSnapshot::new_from_app_state(
            app.state(),
            "missing-safety".into(),
            PersistedSessionInterface::Tui,
            30,
            "preamble".into(),
        );
        missing_safety.runtime.title = Some("Missing safety".into());
        missing_safety.runtime.safety_model_name = "missing/safety-model".into();
        missing_safety.updated_at_unix_ms = 40;

        let store = store_with_root(&root, &app);
        fs::create_dir_all(root.join("missing-main")).expect("create missing-main dir");
        fs::write(
            root.join("missing-main").join(SNAPSHOT_FILE_NAME),
            serde_json::to_string_pretty(&missing_main).expect("serialize missing main"),
        )
        .expect("write missing main snapshot");
        fs::create_dir_all(root.join("missing-safety")).expect("create missing-safety dir");
        fs::write(
            root.join("missing-safety").join(SNAPSHOT_FILE_NAME),
            serde_json::to_string_pretty(&missing_safety).expect("serialize missing safety"),
        )
        .expect("write missing safety snapshot");

        let entries = store
            .list_sessions_for_workspace(Path::new("/tmp/a"))
            .expect("list");

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| !entry.resumable));
        assert!(
            entries
                .iter()
                .all(|entry| entry.detail.contains("saved model unavailable"))
        );
    }

    #[test]
    fn format_timestamp_uses_relative_label_for_today() {
        let now = utc_datetime(2026, 3, 29, 16, 30, 0);
        let timestamp = utc_datetime(2026, 3, 29, 12, 45, 0);
        assert_eq!(
            format_timestamp_relative(timestamp.timestamp_millis() as u64, now),
            "Today at 12:45 UTC"
        );
    }

    #[test]
    fn format_timestamp_uses_relative_label_for_yesterday() {
        let now = utc_datetime(2026, 3, 29, 16, 30, 0);
        let timestamp = utc_datetime(2026, 3, 28, 15, 36, 0);
        assert_eq!(
            format_timestamp_relative(timestamp.timestamp_millis() as u64, now),
            "Yesterday at 15:36 UTC"
        );
    }

    #[test]
    fn format_timestamp_uses_relative_label_with_day_count() {
        let now = utc_datetime(2026, 3, 29, 16, 30, 0);
        let timestamp = utc_datetime(2026, 3, 26, 3, 45, 0);
        assert_eq!(
            format_timestamp_relative(timestamp.timestamp_millis() as u64, now),
            "3 days ago at 03:45 UTC"
        );
    }

    #[test]
    fn format_timestamp_uses_absolute_date_after_one_week() {
        let now = utc_datetime(2026, 3, 29, 16, 30, 0);
        let timestamp = utc_datetime(2026, 3, 20, 4, 3, 0);
        assert_eq!(
            format_timestamp_relative(timestamp.timestamp_millis() as u64, now),
            "Mar 20, 2026 at 04:03 UTC"
        );
    }
}
