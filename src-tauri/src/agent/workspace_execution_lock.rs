use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{
    watch, Mutex as AsyncMutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard,
    RwLock,
};

const WAIT_LOG_INTERVAL: Duration = Duration::from_secs(5);
const POSSIBLE_DEADLOCK_AFTER: Duration = Duration::from_secs(30);
pub(crate) const WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT: &str =
    "workspace-execution-lock-diagnostic";

static PROCESS_WORKSPACE_EXECUTION_LOCKS: LazyLock<Mutex<HashMap<String, WorkspaceExecutionLock>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceExecutionLockRequest {
    PathWrite(Vec<String>),
    Exclusive,
}

impl WorkspaceExecutionLockRequest {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::PathWrite(_) => "path_write",
            Self::Exclusive => "write",
        }
    }

    pub(crate) fn path_keys(&self) -> Option<&[String]> {
        match self {
            Self::PathWrite(paths) => Some(paths),
            Self::Exclusive => None,
        }
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Exclusive, _) | (_, Self::Exclusive) => Self::Exclusive,
            (Self::PathWrite(mut left), Self::PathWrite(right)) => {
                left.extend(right);
                left.sort();
                left.dedup();
                Self::PathWrite(left)
            }
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceExecutionLockBlocker {
    pub session_id: String,
    pub run_id: String,
    pub mode: String,
    pub held_ms: u64,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceExecutionLockDiagnostic {
    pub active: bool,
    pub session_id: String,
    pub run_id: String,
    pub iteration: usize,
    pub workspace: String,
    pub mode: String,
    pub waited_ms: u64,
    pub tools: Vec<String>,
    pub blockers: Vec<WorkspaceExecutionLockBlocker>,
}

type WorkspaceExecutionLockDiagnosticReporter =
    Arc<dyn Fn(WorkspaceExecutionLockDiagnostic) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceExecutionLockAcquireError {
    Cancelled,
}

#[derive(Debug, Clone)]
struct TraceHolder {
    owner: WorkspaceExecutionLockOwner,
    request: WorkspaceExecutionLockRequest,
    acquired_at: Instant,
}

#[derive(Debug, Clone)]
struct TraceWaiter {
    request: WorkspaceExecutionLockRequest,
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
    path_gates: Mutex<HashMap<String, std::sync::Weak<AsyncMutex<()>>>>,
    trace: Mutex<TraceState>,
    next_lease_id: AtomicU64,
    wait_log_interval: Duration,
    possible_deadlock_after: Duration,
}

struct WorkspaceExecutionWaitRegistration {
    lock: WorkspaceExecutionLock,
    lease_id: u64,
    request: WorkspaceExecutionLockRequest,
    owner: WorkspaceExecutionLockOwner,
    requested_at: Instant,
    active: bool,
    diagnostic_active: bool,
    diagnostic_reporter: Option<WorkspaceExecutionLockDiagnosticReporter>,
}

#[derive(Clone)]
pub(crate) struct WorkspaceExecutionLock {
    inner: Arc<WorkspaceExecutionLockInner>,
}

enum OwnedWorkspaceExecutionGuard {
    PathWrite {
        _mutation_guard: OwnedRwLockReadGuard<()>,
        _path_guards: Vec<OwnedMutexGuard<()>>,
    },
    Exclusive(OwnedRwLockWriteGuard<()>),
}

pub(crate) struct WorkspaceExecutionGuard {
    lock: WorkspaceExecutionLock,
    lease_id: u64,
    request: WorkspaceExecutionLockRequest,
    owner: WorkspaceExecutionLockOwner,
    acquired_at: Instant,
    guard: Option<OwnedWorkspaceExecutionGuard>,
}

impl WorkspaceExecutionLock {
    fn new() -> Self {
        Self {
            inner: Arc::new(WorkspaceExecutionLockInner {
                gate: Arc::new(RwLock::new(())),
                path_gates: Mutex::new(HashMap::new()),
                trace: Mutex::new(TraceState::default()),
                next_lease_id: AtomicU64::new(1),
                wait_log_interval: WAIT_LOG_INTERVAL,
                possible_deadlock_after: POSSIBLE_DEADLOCK_AFTER,
            }),
        }
    }

    #[cfg(test)]
    fn new_with_timings(wait_log_interval: Duration, possible_deadlock_after: Duration) -> Self {
        Self {
            inner: Arc::new(WorkspaceExecutionLockInner {
                gate: Arc::new(RwLock::new(())),
                path_gates: Mutex::new(HashMap::new()),
                trace: Mutex::new(TraceState::default()),
                next_lease_id: AtomicU64::new(1),
                wait_log_interval,
                possible_deadlock_after,
            }),
        }
    }

    fn trace(&self) -> MutexGuard<'_, TraceState> {
        self.inner
            .trace
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn path_gates(&self, paths: &[String]) -> Vec<Arc<AsyncMutex<()>>> {
        let mut registry = self
            .inner
            .path_gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        registry.retain(|_, gate| gate.strong_count() > 0);
        paths
            .iter()
            .map(|path| {
                if let Some(gate) = registry.get(path).and_then(std::sync::Weak::upgrade) {
                    return gate;
                }
                let gate = Arc::new(AsyncMutex::new(()));
                registry.insert(path.clone(), Arc::downgrade(&gate));
                gate
            })
            .collect()
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
        let path_writers = trace
            .readers
            .iter()
            .take(4)
            .map(|(lease_id, holder)| {
                format!(
                    "path_writer#{} paths={} held_ms={} {}",
                    lease_id,
                    holder.request.path_keys().map_or(0, <[String]>::len),
                    holder.acquired_at.elapsed().as_millis(),
                    holder.owner.summary()
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let path_writers = if path_writers.is_empty() {
            "path_writers=none".to_string()
        } else if trace.readers.len() > 4 {
            format!(
                "path_writers={} [{} | ...]",
                trace.readers.len(),
                path_writers
            )
        } else {
            format!("path_writers={} [{}]", trace.readers.len(), path_writers)
        };
        let waiters = trace
            .waiters
            .iter()
            .take(4)
            .map(|(lease_id, waiter)| {
                format!(
                    "waiter#{} mode={} wait_ms={} {}",
                    lease_id,
                    waiter.request.label(),
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
        format!("{}; {}; {}", writer, path_writers, waiters)
    }

    fn blockers(
        &self,
        request: &WorkspaceExecutionLockRequest,
    ) -> Vec<WorkspaceExecutionLockBlocker> {
        let trace = self.trace();
        let mut blockers = Vec::new();
        if let Some((_, holder)) = trace.writer.as_ref() {
            blockers.push(WorkspaceExecutionLockBlocker {
                session_id: holder.owner.session_id.clone(),
                run_id: holder.owner.run_id.clone(),
                mode: holder.request.label().to_string(),
                held_ms: duration_millis(holder.acquired_at.elapsed()),
                tools: holder.owner.tools.clone(),
            });
        }
        let requested_paths = request.path_keys();
        if matches!(request, WorkspaceExecutionLockRequest::Exclusive) || requested_paths.is_some()
        {
            blockers.extend(
                trace
                    .readers
                    .values()
                    .filter(|holder| {
                        matches!(request, WorkspaceExecutionLockRequest::Exclusive)
                            || paths_overlap(
                                requested_paths.unwrap_or_default(),
                                holder.request.path_keys().unwrap_or_default(),
                            )
                    })
                    .map(|holder| WorkspaceExecutionLockBlocker {
                        session_id: holder.owner.session_id.clone(),
                        run_id: holder.owner.run_id.clone(),
                        mode: holder.request.label().to_string(),
                        held_ms: duration_millis(holder.acquired_at.elapsed()),
                        tools: holder.owner.tools.clone(),
                    }),
            );
        }
        blockers
    }

    fn diagnostic(
        &self,
        active: bool,
        request: &WorkspaceExecutionLockRequest,
        owner: &WorkspaceExecutionLockOwner,
        waited: Duration,
    ) -> WorkspaceExecutionLockDiagnostic {
        WorkspaceExecutionLockDiagnostic {
            active,
            session_id: owner.session_id.clone(),
            run_id: owner.run_id.clone(),
            iteration: owner.iteration,
            workspace: owner.workspace.clone(),
            mode: request.label().to_string(),
            waited_ms: duration_millis(waited),
            tools: owner.tools.clone(),
            blockers: self.blockers(request),
        }
    }

    #[cfg(test)]
    async fn acquire(
        &self,
        request: WorkspaceExecutionLockRequest,
        owner: WorkspaceExecutionLockOwner,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError> {
        self.acquire_inner(request, owner, cancel_rx, None).await
    }

    pub(crate) async fn acquire_with_diagnostics(
        &self,
        request: WorkspaceExecutionLockRequest,
        owner: WorkspaceExecutionLockOwner,
        cancel_rx: watch::Receiver<bool>,
        app_handle: &AppHandle,
    ) -> Result<WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError> {
        let app_handle = app_handle.clone();
        let reporter: WorkspaceExecutionLockDiagnosticReporter = Arc::new(move |diagnostic| {
            if let Err(error) =
                app_handle.emit(WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT, diagnostic)
            {
                eprintln!(
                    "[WorkspaceExecutionLock] failed to emit frontend diagnostic: {}",
                    error
                );
            }
        });
        self.acquire_inner(request, owner, cancel_rx, Some(reporter))
            .await
    }

    #[cfg(test)]
    async fn acquire_with_reporter(
        &self,
        request: WorkspaceExecutionLockRequest,
        owner: WorkspaceExecutionLockOwner,
        cancel_rx: watch::Receiver<bool>,
        reporter: WorkspaceExecutionLockDiagnosticReporter,
    ) -> Result<WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError> {
        self.acquire_inner(request, owner, cancel_rx, Some(reporter))
            .await
    }

    async fn acquire_inner(
        &self,
        mut request: WorkspaceExecutionLockRequest,
        owner: WorkspaceExecutionLockOwner,
        mut cancel_rx: watch::Receiver<bool>,
        diagnostic_reporter: Option<WorkspaceExecutionLockDiagnosticReporter>,
    ) -> Result<WorkspaceExecutionGuard, WorkspaceExecutionLockAcquireError> {
        if *cancel_rx.borrow() {
            return Err(WorkspaceExecutionLockAcquireError::Cancelled);
        }
        if let WorkspaceExecutionLockRequest::PathWrite(paths) = &mut request {
            paths.sort();
            paths.dedup();
            if paths.is_empty() {
                request = WorkspaceExecutionLockRequest::Exclusive;
            }
        }

        let lease_id = self.inner.next_lease_id.fetch_add(1, Ordering::Relaxed);
        let requested_at = Instant::now();
        {
            self.trace().waiters.insert(
                lease_id,
                TraceWaiter {
                    request: request.clone(),
                    owner: owner.clone(),
                    requested_at,
                },
            );
        }
        let mut wait_registration = WorkspaceExecutionWaitRegistration {
            lock: self.clone(),
            lease_id,
            request: request.clone(),
            owner: owner.clone(),
            requested_at,
            active: true,
            diagnostic_active: false,
            diagnostic_reporter,
        };
        eprintln!(
            "[WorkspaceExecutionLock] requested lease={} mode={} {} holders=({})",
            lease_id,
            request.label(),
            owner.summary(),
            self.holder_summary()
        );

        let gate = self.inner.gate.clone();
        let path_gates = match &request {
            WorkspaceExecutionLockRequest::PathWrite(paths) => self.path_gates(paths),
            WorkspaceExecutionLockRequest::Exclusive => Vec::new(),
        };
        let mut acquire_future: Pin<Box<dyn Future<Output = OwnedWorkspaceExecutionGuard> + Send>> =
            match &request {
                WorkspaceExecutionLockRequest::PathWrite(_) => Box::pin(async move {
                    let mutation_guard = gate.read_owned().await;
                    let mut path_guards = Vec::with_capacity(path_gates.len());
                    for path_gate in path_gates {
                        path_guards.push(path_gate.lock_owned().await);
                    }
                    OwnedWorkspaceExecutionGuard::PathWrite {
                        _mutation_guard: mutation_guard,
                        _path_guards: path_guards,
                    }
                }),
                WorkspaceExecutionLockRequest::Exclusive => Box::pin(async move {
                    OwnedWorkspaceExecutionGuard::Exclusive(gate.write_owned().await)
                }),
            };
        let mut wait_log = tokio::time::interval_at(
            tokio::time::Instant::now() + self.inner.wait_log_interval,
            self.inner.wait_log_interval,
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
                            request: request.clone(),
                            acquired_at,
                        };
                        match &request {
                            WorkspaceExecutionLockRequest::PathWrite(_) => {
                                trace.readers.insert(lease_id, holder);
                            }
                            WorkspaceExecutionLockRequest::Exclusive => {
                                trace.writer = Some((lease_id, holder));
                            }
                        }
                    }
                    wait_registration.active = false;
                    wait_registration.clear_diagnostic(requested_at.elapsed());
                    eprintln!(
                        "[WorkspaceExecutionLock] acquired lease={} mode={} wait_ms={} {}",
                        lease_id,
                        request.label(),
                        requested_at.elapsed().as_millis(),
                        owner.summary()
                    );
                    return Ok(WorkspaceExecutionGuard {
                        lock: self.clone(),
                        lease_id,
                        request,
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
                    wait_registration.clear_diagnostic(requested_at.elapsed());
                    eprintln!(
                        "[WorkspaceExecutionLock] cancelled lease={} mode={} wait_ms={} {} holders=({})",
                        lease_id,
                        request.label(),
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
                        request.label(),
                        waited.as_millis(),
                        waited >= self.inner.possible_deadlock_after,
                        owner.summary(),
                        self.holder_summary()
                    );
                    if waited >= self.inner.possible_deadlock_after {
                        wait_registration.emit_diagnostic(waited);
                    }
                }
            }
        }
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

fn paths_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|path| right.binary_search(path).is_ok())
}

impl WorkspaceExecutionGuard {
    pub(crate) fn path_keys(&self) -> Option<&[String]> {
        self.request.path_keys()
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

    fn emit_diagnostic(&mut self, waited: Duration) {
        if self.diagnostic_active {
            return;
        }
        let Some(reporter) = self.diagnostic_reporter.as_ref() else {
            return;
        };
        self.diagnostic_active = true;
        reporter(
            self.lock
                .diagnostic(true, &self.request, &self.owner, waited),
        );
    }

    fn clear_diagnostic(&mut self, waited: Duration) {
        if !self.diagnostic_active {
            return;
        }
        self.diagnostic_active = false;
        if let Some(reporter) = self.diagnostic_reporter.as_ref() {
            reporter(
                self.lock
                    .diagnostic(false, &self.request, &self.owner, waited),
            );
        }
    }
}

impl Drop for WorkspaceExecutionWaitRegistration {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.lock.trace().waiters.remove(&self.lease_id);
        self.active = false;
        self.clear_diagnostic(self.requested_at.elapsed());
        eprintln!(
            "[WorkspaceExecutionLock] abandoned lease={} mode={} wait_ms={} {} holders=({})",
            self.lease_id,
            self.request.label(),
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
                OwnedWorkspaceExecutionGuard::PathWrite {
                    _mutation_guard,
                    _path_guards,
                } => {
                    drop(_path_guards);
                    drop(_mutation_guard);
                }
                OwnedWorkspaceExecutionGuard::Exclusive(guard) => drop(guard),
            }
        }
        match &self.request {
            WorkspaceExecutionLockRequest::PathWrite(_) => {
                trace.readers.remove(&self.lease_id);
            }
            WorkspaceExecutionLockRequest::Exclusive => {
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
            self.request.label(),
            self.acquired_at.elapsed().as_millis(),
            self.owner.summary()
        );
    }
}

fn normalize_workspace_key(workspace: &str) -> String {
    let trimmed = workspace.trim();
    if trimmed.is_empty() {
        return "<default-workspace>".to_string();
    }

    let path = std::path::PathBuf::from(trimmed);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map(|current| current.join(&path))
            .unwrap_or(path)
    };
    let resolved = dunce::canonicalize(&absolute).unwrap_or(absolute);
    let mut key = dunce::simplified(&resolved)
        .to_string_lossy()
        .replace('\\', "/");
    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }
    #[cfg(windows)]
    key.make_ascii_lowercase();
    key
}

pub(crate) fn normalize_workspace_path_key(workspace: &str, raw_path: &str) -> String {
    use std::path::Component;

    let requested = std::path::PathBuf::from(raw_path.trim());
    let absolute = if requested.is_absolute() {
        requested
    } else {
        std::path::Path::new(workspace).join(requested)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normalized
                    .components()
                    .next_back()
                    .is_some_and(|part| matches!(part, Component::Normal(_)))
                {
                    normalized.pop();
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    let mut anchor = normalized.clone();
    let mut suffix = Vec::new();
    while !anchor.exists() {
        let Some(name) = anchor.file_name() else {
            break;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = anchor.parent() else {
            break;
        };
        anchor = parent.to_path_buf();
    }
    let mut resolved = dunce::canonicalize(&anchor).unwrap_or(anchor);
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    let key = dunce::simplified(&resolved)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

pub(crate) fn process_workspace_execution_lock(workspace: &str) -> WorkspaceExecutionLock {
    let key = normalize_workspace_key(workspace);
    let mut locks = PROCESS_WORKSPACE_EXECUTION_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(key)
        .or_insert_with(WorkspaceExecutionLock::new)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::{
        process_workspace_execution_lock, WorkspaceExecutionLock,
        WorkspaceExecutionLockAcquireError, WorkspaceExecutionLockDiagnostic,
        WorkspaceExecutionLockDiagnosticReporter, WorkspaceExecutionLockOwner,
        WorkspaceExecutionLockRequest,
    };
    use std::sync::{Arc, Mutex};
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
    async fn process_registry_isolates_distinct_workspaces() {
        let root = tempfile::tempdir().expect("workspace root");
        let workspace_a = root.path().join("workspace-a");
        let workspace_b = root.path().join("workspace-b");
        std::fs::create_dir_all(&workspace_a).expect("workspace a");
        std::fs::create_dir_all(&workspace_b).expect("workspace b");
        let workspace_a = workspace_a.to_string_lossy().to_string();
        let workspace_b = workspace_b.to_string_lossy().to_string();

        let lock_a = process_workspace_execution_lock(&workspace_a);
        let same_workspace_lock = process_workspace_execution_lock(&workspace_a);
        let lock_b = process_workspace_execution_lock(&workspace_b);
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let writer = lock_a
            .acquire(
                WorkspaceExecutionLockRequest::Exclusive,
                owner("workspace-a-writer"),
                cancel_rx.clone(),
            )
            .await
            .expect("workspace a writer");

        let mut same_workspace_reader = tokio::spawn({
            let cancel_rx = cancel_rx.clone();
            async move {
                same_workspace_lock
                    .acquire(
                        WorkspaceExecutionLockRequest::PathWrite(vec!["a.txt".to_string()]),
                        owner("workspace-a-reader"),
                        cancel_rx,
                    )
                    .await
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut same_workspace_reader)
                .await
                .is_err()
        );

        let other_workspace_reader = tokio::time::timeout(
            Duration::from_millis(100),
            lock_b.acquire(
                WorkspaceExecutionLockRequest::PathWrite(vec!["b.txt".to_string()]),
                owner("workspace-b-reader"),
                cancel_rx,
            ),
        )
        .await
        .expect("different workspace must not block")
        .expect("workspace b reader");
        drop(other_workspace_reader);
        drop(writer);
        same_workspace_reader.abort();
        let _ = same_workspace_reader.await;
    }

    #[tokio::test]
    async fn distinct_path_writes_overlap_and_exclusive_waits_for_all() {
        let lock = WorkspaceExecutionLock::new();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let first_reader = lock
            .acquire(
                WorkspaceExecutionLockRequest::PathWrite(vec!["a.txt".to_string()]),
                owner("reader-1"),
                cancel_rx.clone(),
            )
            .await
            .expect("first reader");
        let second_reader = tokio::time::timeout(
            Duration::from_millis(100),
            lock.acquire(
                WorkspaceExecutionLockRequest::PathWrite(vec!["b.txt".to_string()]),
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
                    WorkspaceExecutionLockRequest::Exclusive,
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
    async fn same_path_writes_are_serialized() {
        let lock = WorkspaceExecutionLock::new();
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let first = lock
            .acquire(
                WorkspaceExecutionLockRequest::PathWrite(vec!["same.txt".to_string()]),
                owner("path-writer-1"),
                cancel_rx.clone(),
            )
            .await
            .expect("first path writer");

        let waiting_lock = lock.clone();
        let mut second = tokio::spawn(async move {
            waiting_lock
                .acquire(
                    WorkspaceExecutionLockRequest::PathWrite(vec!["same.txt".to_string()]),
                    owner("path-writer-2"),
                    cancel_rx,
                )
                .await
        });
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut second)
            .await
            .is_err());

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("same-path writer should acquire after release")
            .expect("same-path writer task")
            .expect("same-path writer guard");
        drop(second);
    }

    #[tokio::test]
    async fn waiting_acquisition_is_cancellable() {
        let lock = WorkspaceExecutionLock::new();
        let (_holder_cancel_tx, holder_cancel) = tokio::sync::watch::channel(false);
        let holder = lock
            .acquire(
                WorkspaceExecutionLockRequest::Exclusive,
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
                    WorkspaceExecutionLockRequest::PathWrite(vec!["a.txt".to_string()]),
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
                WorkspaceExecutionLockRequest::Exclusive,
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
                    WorkspaceExecutionLockRequest::PathWrite(vec!["a.txt".to_string()]),
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

    #[tokio::test]
    async fn long_wait_emits_one_active_diagnostic_and_clears_after_acquire() {
        let lock = WorkspaceExecutionLock::new_with_timings(
            Duration::from_millis(5),
            Duration::from_millis(15),
        );
        let (_holder_cancel_tx, holder_cancel) = tokio::sync::watch::channel(false);
        let holder = lock
            .acquire(
                WorkspaceExecutionLockRequest::Exclusive,
                owner("holder"),
                holder_cancel,
            )
            .await
            .expect("holder");

        let reports = Arc::new(Mutex::new(Vec::<WorkspaceExecutionLockDiagnostic>::new()));
        let report_sink = reports.clone();
        let reporter: WorkspaceExecutionLockDiagnosticReporter = Arc::new(move |diagnostic| {
            report_sink
                .lock()
                .expect("diagnostic reports")
                .push(diagnostic);
        });
        let (_waiter_cancel_tx, waiter_cancel) = tokio::sync::watch::channel(false);
        let waiting_lock = lock.clone();
        let mut waiting = tokio::spawn(async move {
            waiting_lock
                .acquire_with_reporter(
                    WorkspaceExecutionLockRequest::Exclusive,
                    owner("waiting"),
                    waiter_cancel,
                    reporter,
                )
                .await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if reports
                    .lock()
                    .expect("diagnostic reports")
                    .iter()
                    .any(|diagnostic| diagnostic.active)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("active diagnostic");

        {
            let reports = reports.lock().expect("diagnostic reports");
            let active = reports
                .iter()
                .filter(|diagnostic| diagnostic.active)
                .collect::<Vec<_>>();
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].session_id, "session-test");
            assert_eq!(active[0].run_id, "waiting");
            assert_eq!(active[0].mode, "write");
            assert_eq!(active[0].blockers.len(), 1);
            assert_eq!(active[0].blockers[0].run_id, "holder");
        }

        drop(holder);
        let waiting_guard = tokio::time::timeout(Duration::from_secs(1), &mut waiting)
            .await
            .expect("waiter should acquire")
            .expect("waiter task")
            .expect("waiter guard");
        drop(waiting_guard);

        let reports = reports.lock().expect("diagnostic reports");
        assert_eq!(
            reports
                .iter()
                .filter(|diagnostic| !diagnostic.active)
                .count(),
            1
        );
    }
}
