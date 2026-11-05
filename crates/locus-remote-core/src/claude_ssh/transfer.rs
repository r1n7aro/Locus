//! Deadline leases for bulk history, not for ordinary SSH/CLI startup.
//! Only matching payload bytes renew a lease. One begin notification switches
//! the timer to transfer mode; later bytes merely update the existing deadline.
use serde_json::Value;
use std::{future::Future, sync::{Arc, Mutex}, time::Duration};
use tokio::{sync::Notify, time::Instant};

const HISTORY_IDLE: Duration = Duration::from_secs(45);
const ARCHIVE_IDLE: Duration = Duration::from_secs(120);
const TRANSFER_TOTAL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub initial: Duration,
    pub idle: Duration,
    pub total: Duration,
}
impl Limits {
    pub fn history(initial: Duration) -> Self { Self { initial, idle: HISTORY_IDLE, total: TRANSFER_TOTAL } }
    pub fn archive() -> Self { Self { initial: ARCHIVE_IDLE, idle: ARCHIVE_IDLE, total: TRANSFER_TOTAL } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Timeout {
    Initial(Duration),
    Idle(Duration),
    Total(Duration),
}
impl Timeout {
    pub fn message(self, operation: &str) -> String {
        match self {
            Self::Initial(limit) => format!("{operation} timed out after {}s waiting for transfer data", limit.as_secs()),
            Self::Idle(limit) => format!("{operation} stalled: no new transfer data for {}s", limit.as_secs()),
            Self::Total(limit) => format!("{operation} exceeded the {}s total transfer limit", limit.as_secs()),
        }
    }
}

struct Active { started: Instant, received: Instant }
#[derive(Default)]
pub(super) struct Progress {
    active: Mutex<Option<Active>>,
    began: Notify,
}
impl Progress {
    /// Repeated announcements do not count as data or restart either deadline.
    pub fn begin(&self) { self.update(false); }
    pub fn received(&self) { self.update(true); }
    fn update(&self, data: bool) {
        let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_mut() {
            if data { active.received = Instant::now(); }
        } else {
            let now = Instant::now();
            *active = Some(Active { started: now, received: now });
            self.began.notify_one();
        }
    }
    fn deadline(&self, initial: Instant, limits: Limits) -> (Instant, Timeout) {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        let Some(active) = active.as_ref() else { return (initial, Timeout::Initial(limits.initial)); };
        let idle = active.received + limits.idle;
        let total = active.started + limits.total;
        if total <= idle { (total, Timeout::Total(limits.total)) } else { (idle, Timeout::Idle(limits.idle)) }
    }
    pub async fn wait<F: Future>(&self, future: F, limits: Limits) -> Result<F::Output, Timeout> {
        let initial = Instant::now() + limits.initial;
        tokio::pin!(future);
        loop {
            let (deadline, _) = self.deadline(initial, limits);
            tokio::select! {
                biased;
                result = &mut future => return Ok(result),
                _ = self.began.notified() => {},
                _ = tokio::time::sleep_until(deadline) => {
                    // Bytes arriving while this timer slept may have renewed it.
                    let (deadline, reason) = self.deadline(initial, limits);
                    if Instant::now() >= deadline { return Err(reason); }
                }
            }
        }
    }
}

/// The helper emits a size marker immediately before its payload while holding
/// the SAME stdout lock. Thus even an unfinished JSON line can renew only its
/// own request. Completed frames must still match the marker's type/id/size.
pub(super) struct FrameProgress {
    progress: Arc<Progress>,
    expected: usize,
    received: usize,
    archive_id: Option<String>,
}
impl FrameProgress {
    pub fn new(progress: Arc<Progress>, expected: usize, archive_id: Option<String>) -> Result<Self, String> {
        if expected == 0 || expected > super::MAX_FRAME { return Err("Invalid history transfer frame size".into()); }
        progress.begin();
        Ok(Self { progress, expected, received: 0, archive_id })
    }
    pub fn received(&mut self, count: usize) -> Result<(), String> {
        self.received = self.received.saturating_add(count);
        if self.received > self.expected { return Err("History transfer frame exceeds its announced size".into()); }
        if count > 0 { self.progress.received(); }
        Ok(())
    }
    pub fn complete(&self, value: &Value) -> Result<(), String> {
        let matches = if let Some(id) = &self.archive_id {
            value["id"].as_str() == Some(id.as_str())
                && matches!(value["type"].as_str(), Some("archive_chunk" | "archive_result"))
        } else { value["type"] == "hello" && value["protocol"] == 1 };
        if self.received != self.expected || !matches {
            return Err("History transfer frame does not match its announcement".into());
        }
        Ok(())
    }
}

pub(super) fn announced_size(value: &Value) -> Result<usize, String> {
    value["bytes"].as_u64().filter(|size| *size > 0 && *size <= super::MAX_FRAME as u64)
        .map(|size| size as usize).ok_or_else(|| "Invalid history transfer frame size".into())
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
