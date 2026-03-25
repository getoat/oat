use anyhow::{Result, anyhow};
use tokio::time::Instant;

use super::store::SubagentRecord;
use super::{SubagentManager, SubagentSnapshot, SubagentStatus, WaitResult};

pub(crate) struct WaitStateSnapshot {
    pub(crate) subagents: Vec<SubagentSnapshot>,
    pub(crate) completed_id: Option<String>,
    pub(crate) failed_id: Option<String>,
    pub(crate) cancelled_id: Option<String>,
    pub(crate) inactive_id: Option<String>,
    pub(crate) deadline: Option<Instant>,
}

impl SubagentManager {
    pub async fn wait(
        &self,
        ids: &[String],
        timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult> {
        if ids.is_empty() {
            anyhow::bail!("ids must contain at least one subagent id");
        }

        let inactivity_timeout = timeout
            .unwrap_or_else(|| std::time::Duration::from_millis(super::DEFAULT_WAIT_TIMEOUT_MS));
        if inactivity_timeout.is_zero() {
            anyhow::bail!("timeout_ms must be greater than 0");
        }

        let mut rx = self.inner.notify_tx.subscribe();
        loop {
            let snapshot = self.wait_state_snapshot(ids, inactivity_timeout)?;
            if snapshot.is_terminal() {
                return Ok(snapshot.into_result());
            }

            let deadline = snapshot.deadline.expect("deadline for running subagents");
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Ok(self.wait_state_snapshot(ids, inactivity_timeout)?.into_result());
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let timed_out = self.wait_state_snapshot(ids, inactivity_timeout)?;
                    if let Some(inactive_id) = timed_out.inactive_id.clone() {
                        return Ok(timed_out.into_result_with_inactive(inactive_id));
                    }
                }
            }
        }
    }

    pub async fn wait_all(
        &self,
        ids: &[String],
        timeout: Option<std::time::Duration>,
    ) -> Result<WaitResult> {
        if ids.is_empty() {
            anyhow::bail!("ids must contain at least one subagent id");
        }

        let inactivity_timeout = timeout
            .unwrap_or_else(|| std::time::Duration::from_millis(super::DEFAULT_WAIT_TIMEOUT_MS));
        if inactivity_timeout.is_zero() {
            anyhow::bail!("timeout_ms must be greater than 0");
        }

        let mut rx = self.inner.notify_tx.subscribe();
        loop {
            let snapshot = self.wait_state_snapshot(ids, inactivity_timeout)?;
            if snapshot.all_terminal() {
                return Ok(snapshot.into_result());
            }

            let deadline = snapshot.deadline.expect("deadline for running subagents");
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Ok(self.wait_state_snapshot(ids, inactivity_timeout)?.into_result());
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    let timed_out = self.wait_state_snapshot(ids, inactivity_timeout)?;
                    if let Some(inactive_id) = timed_out.inactive_id.clone() {
                        return Ok(timed_out.into_result_with_inactive(inactive_id));
                    }
                }
            }
        }
    }

    pub(crate) fn wait_state_snapshot(
        &self,
        ids: &[String],
        inactivity_timeout: std::time::Duration,
    ) -> Result<WaitStateSnapshot> {
        let state = self.inner.state.lock().expect("subagent state lock");
        let mut snapshots = Vec::with_capacity(ids.len());
        let now = Instant::now();
        let mut completed_id = None;
        let mut failed_id = None;
        let mut cancelled_id = None;
        let mut inactive_id = None;
        let mut deadline = None;

        for id in ids {
            let record = state
                .records
                .get(id)
                .cloned()
                .ok_or_else(|| anyhow!("Unknown subagent `{id}`."))?;
            match record.status {
                SubagentStatus::Completed if completed_id.is_none() => {
                    completed_id = Some(record.id.clone());
                }
                SubagentStatus::Failed if failed_id.is_none() => {
                    failed_id = Some(record.id.clone());
                }
                SubagentStatus::Cancelled if cancelled_id.is_none() => {
                    cancelled_id = Some(record.id.clone());
                }
                SubagentStatus::Running => {
                    let record_deadline = record.last_activity_at + inactivity_timeout;
                    if record_deadline <= now && inactive_id.is_none() {
                        inactive_id = Some(record.id.clone());
                    }
                    deadline = Some(match deadline {
                        Some(current) if current <= record_deadline => current,
                        _ => record_deadline,
                    });
                }
                _ => {}
            }
            snapshots.push(snapshot_from_record(record));
        }

        Ok(WaitStateSnapshot {
            subagents: snapshots,
            completed_id,
            failed_id,
            cancelled_id,
            inactive_id,
            deadline,
        })
    }
}

impl WaitStateSnapshot {
    pub(crate) fn is_terminal(&self) -> bool {
        self.completed_id.is_some() || self.failed_id.is_some() || self.cancelled_id.is_some()
    }

    pub(crate) fn all_terminal(&self) -> bool {
        self.deadline.is_none() && self.inactive_id.is_none()
    }

    pub(crate) fn into_result(self) -> WaitResult {
        WaitResult {
            completed_id: self.completed_id,
            failed_id: self.failed_id,
            cancelled_id: self.cancelled_id,
            inactive_id: None,
            timed_out_on_inactivity: false,
            subagents: self.subagents,
        }
    }

    pub(crate) fn into_result_with_inactive(self, inactive_id: String) -> WaitResult {
        WaitResult {
            completed_id: self.completed_id,
            failed_id: self.failed_id,
            cancelled_id: self.cancelled_id,
            inactive_id: Some(inactive_id),
            timed_out_on_inactivity: true,
            subagents: self.subagents,
        }
    }
}

pub(crate) fn snapshot_from_record(record: SubagentRecord) -> SubagentSnapshot {
    SubagentSnapshot {
        id: record.id,
        status: record.status,
        access_mode: record.access_mode.label().to_ascii_lowercase(),
        latest_tool_name: record.latest_tool_name,
        output: record.output,
        error: record.error,
        failure_log_path: record.failure_log_path,
    }
}
