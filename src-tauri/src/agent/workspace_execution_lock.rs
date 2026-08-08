use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tokio::sync::{watch, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

const WAIT_LOG_INTERVAL: Duration = Duration::from_secs(5);
const POSSIBLE_DEADLOCK_AFTER: Duration = Duration::from_secs(30);

static PROCESS_WORKSPACE_EXECUTION_LOCK: LazyLock<WorkspaceExecutionLock> =
    LazyLock::new(WorkspaceExecutionLock::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceExecutionLockMode {
    Read,
    Write,
}

impl WorkspaceExecutionLockMode {
    fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceExecutionLockOwner {
    pub session_id: String,
    pub run_id: String,
    pub iteration: usize,
    pub workspace: String,
    pub tools: Vec<String>,
}

impl WorkspaceExecutionLockOwner {
    fn summary(&self) -> String {
        let tools = self
            .tools
            .iter()
            .take(12)
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        let suffix = if self.tools.len() > 12 {
            format!(",+{}", self.tools.len() - 12)
        } else {
            String::new()
        };
        format!(
            "session={} run={} iteration={} workspace={} tools=[{}{}]",
            self.session_id, self.run_id, self.iteration, self.workspace, tools, suffix
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceExecutionLockAcquireError {
    Cancelled,
}

#[derive(Debug, Clone)]
struct TraceHolder {
    owner: WorkspaceExecutionLockOwner,
    acquired_at: Instant,
}

#[derive(Debug, Clone)]
struct TraceWaiter {
    mode: WorkspaceExecutionLockMode,
    owner: WorkspaceExecutionLockOwner,
    requested_at: Instant,
}

#[derive(Default)]
struct TraceState {
    writer: Option<(u64, TraceHolder)>,
    readers: HashMap<u64, TraceHolder>,
    waiters: HashMap<u64, TraceWaiter>,
}

struct WorkspaceExecutionLockInner {
    gate: Arc<RwLock<()>>,
    trace: Mutex<TraceState>,
    next_lease_id: AtomicU64,
}

struct WorkspaceExecutionWaitRegistration {
    lock: WorkspaceExecutionLock,
    lease_id: u64,
    mode: WorkspaceExecutionLockMode,
    owner: WorkspaceExecutionLockOwner,
    requested_at: Instant,
    active: bool,
}

#[derive(Clone)]
pub(crate) struct WorkspaceExecutionLock {
    inner: Arc<WorkspaceExecutionLockInner>,
}

enum OwnedWorkspaceExecutionGuard {
    Read(OwnedRwLockReadGuard<()>),
    Write(OwnedRwLockWriteGuard<()>),
}

pub(crate) struct WorkspaceExecutionGuard {
    lock: WorkspaceExecutionLock,
    lease_id: u64,
    mode: WorkspaceExecutionLockMode,
    owner: WorkspaceExecutionLockOwner,
    acquired_at: Instant,
    guard: Option<OwnedWorkspaceExecutionGuard>,
}

impl WorkspaceExecutionLock {
    fn new() -> Self {
        Self {
            inner: Arc::new(WorkspaceExecutionLockInner {
                gate: Arc::new(RwLock::new(())),
                trace: Mutex::new(TraceState::default()),
                next_lease_id: AtomicU64::new(1),
            }),
        }
    }

    fn trace(&self) -> MutexGuard<'_, TraceState> {
        self.inner
            .trace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn holder_summary(&self) -> String {
        let trace = self.trace();
        let writer = trace
            .writer
            .as_ref()
            .map(|(lease_id, holder)| {
                format!(
                    "writer#{} held_ms={} {}",
                    lease_id,
                    holder.acquired_at.elapsed().as_millis(),
                    holder.owner.summary()
                )
            })
            .unwrap_or_else(|| "writer=none".to_string());
        let readers = trace
            .readers
            .iter()
            .take(4)
            .map(|(lease_id, holder)| {
                format!(
                    "reader#{} held_ms={} {}",
                    lease_id,
                    holder.acquired_at.elapsed().as_millis(),
                    holder.owner.summary()
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let readers = if readers.is_empty() {
            "readers=none".to_string()
        } else if trace.readers.len() > 4 {
            format!("readers={} [{} | ...]", trace.readers.len(), readers)
        } else {
            format!("readers={} [{}]", trace.readers.len(), readers)
        };
        let waiters = trace
            .waiters
            .iter()
            .take(4)
            .map(|(lease_id, waiter)| {
                format!(
                    "waiter#{} mode={} wait_ms={} {}",
                    lease_id,
                    waiter.mode.label(),
                    waiter.requested_at.elapsed().as_millis(),
                    waiter.owner.summary()
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let waiters = if waiters.is_empty() {
            "waiters=none".to_string()
        } else if trace.waiters.len() > 4 {
            format!("waiters={} [{} | ...]", trace.waiters.len(), waiters)
        } else {
            format!("waiters={} [{}]", trace.waiters.len(), waiters)
        };
        format!("{}; {}; {}", writer, readers, waiters)
    }

    pub(crate) async fn acquire(
        &self,
        mode: WorkspaceExecutionLockMode,
        owner: WorkspaceExecutionLockOwner,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Result<WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError> {
        if *cancel_rx.borrow() {
            return Err(WorkspaceExecutionLockAcquireError::Cancelled);
        }

        let lease_id = self.inner.next_lease_id.fetch_add(1, Ordering::Relaxed);
        let requested_at = Instant::now();
        {
            self.trace().waiters.insert(
                lease_id,
                TraceWaiter {
                    mode,
                    owner: owner.clone(),
                    requested_at,
                },
            );
        }
        let mut wait_registration = WorkspaceExecutionWaitRegistration {
            lock: self.clone(),
            lease_id,
            mode,
            owner: owner.clone(),
            requested_at,
            active: true,
        };
        eprintln!(
            "[WorkspaceExecutionLock] requested lease={} mode={} {} holders=({})",
            lease_id,
            mode.label(),
            owner.summary(),
            self.holder_summary()
        );

        let gate = self.inner.gate.clone();
        let mut acquire_future: Pin<Box<dyn Future<Output = OwnedWorkspaceExecutionGuard> + Send>> =
            match mode {
                WorkspaceExecutionLockMode::Read => {
                    Box::pin(
                        async move { OwnedWorkspaceExecutionGuard::Read(gate.read_owned().await) },
                    )
                }
                WorkspaceExecutionLockMode::Write => Box::pin(async move {
                    OwnedWorkspaceExecutionGuard::Write(gate.write_owned().await)
                }),
            };
        let mut wait_log = tokio::time::interval_at(
            tokio::time::Instant::now() + WAIT_LOG_INTERVAL,
            WAIT_LOG_INTERVAL,
        );
        wait_log.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                guard = &mut acquire_future => {
                    let acquired_at = Instant::now();
                    {
                        let mut trace = self.trace();
                        trace.waiters.remove(&lease_id);
                        let holder = TraceHolder {
                            owner: owner.clone(),
                            acquired_at,
                        };
                        match mode {
                            WorkspaceExecutionLockMode::Read => {
                                trace.readers.insert(lease_id, holder);
                            }
                            WorkspaceExecutionLockMode::Write => {
                                trace.writer = Some((lease_id, holder));
                            }
                        }
                    }
                    wait_registration.active = false;
                    eprintln!(
                        "[WorkspaceExecutionLock] acquired lease={} mode={} wait_ms={} {}",
                        lease_id,
                        mode.label(),
                        requested_at.elapsed().as_millis(),
                        owner.summary()
                    );
                    return Ok(WorkspaceExecutionGuard {
                        lock: self.clone(),
                        lease_id,
                        mode,
                        owner,
                        acquired_at,
                        guard: Some(guard),
                    });
                }
                changed = cancel_rx.changed() => {
                    if changed.is_ok() && !*cancel_rx.borrow() {
                        continue;
                    }
                    wait_registration.remove();
                    eprintln!(
                        "[WorkspaceExecutionLock] cancelled lease={} mode={} wait_ms={} {} holders=({})",
                        lease_id,
                        mode.label(),
                        requested_at.elapsed().as_millis(),
                        owner.summary(),
                        self.holder_summary()
                    );
                    return Err(WorkspaceExecutionLockAcquireError::Cancelled);
                }
                _ = wait_log.tick() => {
                    let waited = requested_at.elapsed();
                    eprintln!(
                        "[WorkspaceExecutionLock] waiting lease={} mode={} wait_ms={} possible_deadlock={} {} holders=({})",
                        lease_id,
                        mode.label(),
                        waited.as_millis(),
                        waited >= POSSIBLE_DEADLOCK_AFTER,
                        owner.summary(),
                        self.holder_summary()
                    );
                }
            }
        }
    }
}

impl WorkspaceExecutionWaitRegistration {
    fn remove(&mut self) {
        if !self.active {
            return;
        }
        self.lock.trace().waiters.remove(&self.lease_id);
        self.active = false;
    }
}

impl Drop for WorkspaceExecutionWaitRegistration {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.lock.trace().waiters.remove(&self.lease_id);
        self.active = false;
        eprintln!(
            "[WorkspaceExecutionLock] abandoned lease={} mode={} wait_ms={} {} holders=({})",
            self.lease_id,
            self.mode.label(),
            self.requested_at.elapsed().as_millis(),
            self.owner.summary(),
            self.lock.holder_summary()
        );
    }
}

impl Drop for WorkspaceExecutionGuard {
    fn drop(&mut self) {
        // Serialize the real gate release and trace transition under the small
        // trace mutex. A newly awakened holder cannot publish `acquired`
        // before this lease publishes `released`, and lease-id checks keep an
        // older writer from clearing a newer trace owner.
        let mut trace = self.lock.trace();
        if let Some(guard) = self.guard.take() {
            match guard {
                OwnedWorkspaceExecutionGuard::Read(guard) => drop(guard),
                OwnedWorkspaceExecutionGuard::Write(guard) => drop(guard),
            }
        }
        match self.mode {
            WorkspaceExecutionLockMode::Read => {
                trace.readers.remove(&self.lease_id);
            }
            WorkspaceExecutionLockMode::Write => {
                if trace
                    .writer
                    .as_ref()
                    .is_some_and(|(lease_id, _)| *lease_id == self.lease_id)
                {
                    trace.writer = None;
                }
            }
        }
        eprintln!(
            "[WorkspaceExecutionLock] released lease={} mode={} held_ms={} {}",
            self.lease_id,
            self.mode.label(),
            self.acquired_at.elapsed().as_millis(),
            self.owner.summary()
        );
    }
}

pub(crate) fn process_workspace_execution_lock() -> WorkspaceExecutionLock {
    PROCESS_WORKSPACE_EXECUTION_LOCK.clone()
}

#[cfg(test)]
mod tests {
    use super::{
        WorkspaceExecutionLock, WorkspaceExecutionLockAcquireError, WorkspaceExecutionLockMode,
        WorkspaceExecutionLockOwner,
    };
    use std::time::Duration;

    fn owner(run_id: &str) -> WorkspaceExecutionLockOwner {
        WorkspaceExecutionLockOwner {
            session_id: "session-test".to_string(),
            run_id: run_id.to_string(),
            iteration: 1,
            workspace: "test-workspace".to_string(),
            tools: vec!["test".to_string()],
        }
    }

    #[tokio::test]
    async fn readers_overlap_and_writer_waits_for_all_readers() {
        let lock = WorkspaceExecutionLock::new();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let first_reader = lock
            .acquire(
                WorkspaceExecutionLockMode::Read,
                owner("reader-1"),
                cancel_rx.clone(),
            )
            .await
            .expect("first reader");
        let second_reader = tokio::time::timeout(
            Duration::from_millis(100),
            lock.acquire(
                WorkspaceExecutionLockMode::Read,
                owner("reader-2"),
                cancel_rx.clone(),
            ),
        )
        .await
        .expect("second reader must not block")
        .expect("second reader");

        let writer_lock = lock.clone();
        let writer_cancel = cancel_rx.clone();
        let mut writer = tokio::spawn(async move {
            writer_lock
                .acquire(
                    WorkspaceExecutionLockMode::Write,
                    owner("writer"),
                    writer_cancel,
                )
                .await
        });
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut writer)
            .await
            .is_err());

        drop(first_reader);
        drop(second_reader);
        let writer = tokio::time::timeout(Duration::from_secs(1), writer)
            .await
            .expect("writer should acquire after readers release")
            .expect("writer task")
            .expect("writer guard");
        drop(writer);
    }

    #[tokio::test]
    async fn waiting_acquisition_is_cancellable() {
        let lock = WorkspaceExecutionLock::new();
        let (_holder_cancel_tx, holder_cancel) = tokio::sync::watch::channel(false);
        let holder = lock
            .acquire(
                WorkspaceExecutionLockMode::Write,
                owner("holder"),
                holder_cancel,
            )
            .await
            .expect("holder");

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let waiting_lock = lock.clone();
        let waiting = tokio::spawn(async move {
            waiting_lock
                .acquire(
                    WorkspaceExecutionLockMode::Read,
                    owner("waiting"),
                    cancel_rx,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel_tx.send(true).expect("cancel waiter");
        assert!(matches!(
            waiting.await.expect("waiting task"),
            Err(WorkspaceExecutionLockAcquireError::Cancelled)
        ));
        drop(holder);
    }

    #[tokio::test]
    async fn aborted_waiter_is_removed_from_trace_state() {
        let lock = WorkspaceExecutionLock::new();
        let (_holder_cancel_tx, holder_cancel) = tokio::sync::watch::channel(false);
        let holder = lock
            .acquire(
                WorkspaceExecutionLockMode::Write,
                owner("holder"),
                holder_cancel,
            )
            .await
            .expect("holder");

        let (_waiter_cancel_tx, waiter_cancel) = tokio::sync::watch::channel(false);
        let waiting_lock = lock.clone();
        let waiting = tokio::spawn(async move {
            waiting_lock
                .acquire(
                    WorkspaceExecutionLockMode::Read,
                    owner("aborted-waiter"),
                    waiter_cancel,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        waiting.abort();
        let _ = waiting.await;

        assert!(lock.holder_summary().contains("waiters=none"));
        drop(holder);
    }
}
