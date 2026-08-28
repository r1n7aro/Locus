use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::Notify;

use crate::resource_policy::ResourcePolicyStore;

#[derive(Debug, Clone)]
struct WaitingJob {
    ticket: u64,
    scope_key: String,
}

#[derive(Default)]
struct SchedulerState {
    active_jobs: usize,
    active_scopes: HashSet<String>,
    poisoned_scopes: HashSet<String>,
    waiting: VecDeque<WaitingJob>,
}

pub struct CompileScheduler {
    policy: Arc<ResourcePolicyStore>,
    state: Mutex<SchedulerState>,
    notify: Notify,
    next_ticket: AtomicU64,
}

static SCHEDULER: OnceLock<Arc<CompileScheduler>> = OnceLock::new();

pub fn initialize(policy: Arc<ResourcePolicyStore>) {
    let scheduler = Arc::new(CompileScheduler {
        policy,
        state: Mutex::new(SchedulerState::default()),
        notify: Notify::new(),
        next_ticket: AtomicU64::new(0),
    });
    if SCHEDULER.set(Arc::clone(&scheduler)).is_ok() {
        tauri::async_runtime::spawn(async move {
            let mut updates = scheduler.policy.subscribe();
            while updates.changed().await.is_ok() {
                scheduler.notify.notify_waiters();
            }
        });
    }
}

pub async fn acquire(scope_key: String) -> Result<CompileJobPermit, String> {
    acquire_inner(scope_key, false, false).await
}

/// Lifecycle control requests must remain admissible when the regular compile
/// queue is full. They still serialize behind the active job for the scope.
pub async fn acquire_control(scope_key: String) -> Result<CompileJobPermit, String> {
    acquire_inner(scope_key, true, true).await
}

async fn acquire_inner(
    scope_key: String,
    bypass_queue_limit: bool,
    allow_poisoned: bool,
) -> Result<CompileJobPermit, String> {
    let scheduler = SCHEDULER
        .get()
        .cloned()
        .ok_or_else(|| "compile scheduler is not initialized".to_string())?;
    acquire_with_scheduler(scheduler, scope_key, bypass_queue_limit, allow_poisoned).await
}

async fn acquire_with_scheduler(
    scheduler: Arc<CompileScheduler>,
    scope_key: String,
    bypass_queue_limit: bool,
    allow_poisoned: bool,
) -> Result<CompileJobPermit, String> {
    let ticket = scheduler.next_ticket.fetch_add(1, Ordering::Relaxed) + 1;
    {
        let limits = scheduler.policy.snapshot().limits;
        let mut state = scheduler
            .state
            .lock()
            .map_err(|error| format!("compile scheduler lock poisoned: {error}"))?;
        if !allow_poisoned && state.poisoned_scopes.contains(&scope_key) {
            return Err("compile scope is recovering from a timed-out request".to_string());
        }
        if !bypass_queue_limit && state.waiting.len() >= limits.max_compile_queue_depth {
            return Err(format!(
                "compile queue is busy (configured depth: {})",
                limits.max_compile_queue_depth
            ));
        }
        state.waiting.push_back(WaitingJob {
            ticket,
            scope_key: scope_key.clone(),
        });
    }
    let mut registration = WaitingRegistration {
        scheduler: Arc::clone(&scheduler),
        ticket,
        armed: true,
    };

    loop {
        // Register the waiter before inspecting state. `Notify::notify_waiters`
        // does not retain a permit, so enabling first closes the lost-wakeup
        // window between the admission check and `.await`.
        let notified = scheduler.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let admitted = {
            let limits = scheduler.policy.snapshot().limits;
            let mut state = scheduler
                .state
                .lock()
                .map_err(|error| format!("compile scheduler lock poisoned: {error}"))?;
            if !allow_poisoned && state.poisoned_scopes.contains(&scope_key) {
                return Err("compile scope is recovering from a timed-out request".to_string());
            }
            let position = state
                .waiting
                .iter()
                .position(|waiting| waiting.ticket == ticket)
                .ok_or_else(|| "compile queue registration disappeared".to_string())?;
            let earlier_same_scope = state
                .waiting
                .iter()
                .take(position)
                .any(|waiting| waiting.scope_key == scope_key);
            if state.active_jobs < limits.max_concurrent_compile_jobs
                && !state.active_scopes.contains(&scope_key)
                && !earlier_same_scope
            {
                state.waiting.remove(position);
                state.active_jobs += 1;
                state.active_scopes.insert(scope_key.clone());
                true
            } else {
                false
            }
        };
        if admitted {
            registration.armed = false;
            return Ok(CompileJobPermit {
                scheduler: Arc::clone(&scheduler),
                scope_key,
            });
        }
        notified.await;
    }
}

pub fn mark_scope_poisoned(scope_key: &str) {
    let Some(scheduler) = SCHEDULER.get() else {
        return;
    };
    if let Ok(mut state) = scheduler.state.lock() {
        state.poisoned_scopes.insert(scope_key.to_string());
    }
    scheduler.notify.notify_waiters();
}

pub fn clear_scope_poisoned(scope_key: &str) {
    let Some(scheduler) = SCHEDULER.get() else {
        return;
    };
    if let Ok(mut state) = scheduler.state.lock() {
        state.poisoned_scopes.remove(scope_key);
    }
    scheduler.notify.notify_waiters();
}

pub fn clear_all_poisoned() {
    let Some(scheduler) = SCHEDULER.get() else {
        return;
    };
    if let Ok(mut state) = scheduler.state.lock() {
        state.poisoned_scopes.clear();
    }
    scheduler.notify.notify_waiters();
}

struct WaitingRegistration {
    scheduler: Arc<CompileScheduler>,
    ticket: u64,
    armed: bool,
}

impl Drop for WaitingRegistration {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut state) = self.scheduler.state.lock() {
            state
                .waiting
                .retain(|waiting| waiting.ticket != self.ticket);
        }
        self.scheduler.notify.notify_waiters();
    }
}

pub struct CompileJobPermit {
    scheduler: Arc<CompileScheduler>,
    scope_key: String,
}

impl Drop for CompileJobPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scheduler.state.lock() {
            state.active_jobs = state.active_jobs.saturating_sub(1);
            state.active_scopes.remove(&self.scope_key);
        }
        self.scheduler.notify.notify_waiters();
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileSchedulerMetrics {
    pub configured_max_concurrent_jobs: usize,
    pub configured_max_queue_depth: usize,
    pub active_jobs: usize,
    pub waiting_jobs: usize,
    pub active_scopes: usize,
    pub poisoned_scopes: usize,
}

pub fn metrics() -> CompileSchedulerMetrics {
    let Some(scheduler) = SCHEDULER.get() else {
        return CompileSchedulerMetrics {
            configured_max_concurrent_jobs: 0,
            configured_max_queue_depth: 0,
            active_jobs: 0,
            waiting_jobs: 0,
            active_scopes: 0,
            poisoned_scopes: 0,
        };
    };
    let limits = scheduler.policy.snapshot().limits;
    let state = scheduler.state.lock().ok();
    CompileSchedulerMetrics {
        configured_max_concurrent_jobs: limits.max_concurrent_compile_jobs,
        configured_max_queue_depth: limits.max_compile_queue_depth,
        active_jobs: state.as_ref().map(|state| state.active_jobs).unwrap_or(0),
        waiting_jobs: state.as_ref().map(|state| state.waiting.len()).unwrap_or(0),
        active_scopes: state
            .as_ref()
            .map(|state| state.active_scopes.len())
            .unwrap_or(0),
        poisoned_scopes: state
            .as_ref()
            .map(|state| state.poisoned_scopes.len())
            .unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scheduler(
        max_jobs: usize,
        queue_depth: usize,
    ) -> (Arc<CompileScheduler>, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().expect("config dir");
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.path().join("config.json"),
        ));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("policy"));
        let mut limits = policy.snapshot().limits;
        limits.max_concurrent_compile_jobs = max_jobs;
        limits.max_compile_queue_depth = queue_depth;
        policy.update(limits).expect("configure policy");
        (
            Arc::new(CompileScheduler {
                policy,
                state: Mutex::new(SchedulerState::default()),
                notify: Notify::new(),
                next_ticket: AtomicU64::new(0),
            }),
            config_dir,
        )
    }

    async fn wait_for_waiting_jobs(scheduler: &CompileScheduler, expected: usize) {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if scheduler
                    .state
                    .lock()
                    .map(|state| state.waiting.len() == expected)
                    .unwrap_or(false)
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("waiting job count");
    }

    #[test]
    fn waiting_registration_drop_releases_queue_slot() {
        let config_dir = tempfile::tempdir().expect("config dir");
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.path().join("config.json"),
        ));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("policy"));
        let scheduler = Arc::new(CompileScheduler {
            policy,
            state: Mutex::new(SchedulerState {
                active_jobs: 0,
                active_scopes: HashSet::new(),
                poisoned_scopes: HashSet::new(),
                waiting: VecDeque::from([WaitingJob {
                    ticket: 7,
                    scope_key: "scope".to_string(),
                }]),
            }),
            notify: Notify::new(),
            next_ticket: AtomicU64::new(7),
        });
        drop(WaitingRegistration {
            scheduler: Arc::clone(&scheduler),
            ticket: 7,
            armed: true,
        });
        assert!(scheduler.state.lock().unwrap().waiting.is_empty());
    }

    #[tokio::test]
    async fn different_scopes_use_configured_parallelism_and_same_scope_stays_ordered() {
        let (scheduler, _config_dir) = test_scheduler(2, 8);
        let left = acquire_with_scheduler(Arc::clone(&scheduler), "left".to_string(), false, false)
            .await
            .expect("left permit");
        let right =
            acquire_with_scheduler(Arc::clone(&scheduler), "right".to_string(), false, false)
                .await
                .expect("right permit");

        let scheduler_for_waiter = Arc::clone(&scheduler);
        let same_scope = tokio::spawn(async move {
            acquire_with_scheduler(scheduler_for_waiter, "left".to_string(), false, false).await
        });
        wait_for_waiting_jobs(&scheduler, 1).await;
        assert_eq!(scheduler.state.lock().unwrap().active_jobs, 2);
        drop(left);
        let same_scope = tokio::time::timeout(std::time::Duration::from_secs(1), same_scope)
            .await
            .expect("same scope wakes")
            .expect("same scope task")
            .expect("same scope permit");
        drop(same_scope);
        drop(right);
    }

    #[tokio::test]
    async fn queue_depth_rejects_new_jobs_and_cancelled_waiters_release_capacity() {
        let (scheduler, _config_dir) = test_scheduler(1, 1);
        let active =
            acquire_with_scheduler(Arc::clone(&scheduler), "active".to_string(), false, false)
                .await
                .expect("active permit");
        let scheduler_for_waiter = Arc::clone(&scheduler);
        let waiter = tokio::spawn(async move {
            acquire_with_scheduler(scheduler_for_waiter, "waiting".to_string(), false, false).await
        });
        wait_for_waiting_jobs(&scheduler, 1).await;
        let busy =
            acquire_with_scheduler(Arc::clone(&scheduler), "overflow".to_string(), false, false)
                .await
                .err()
                .expect("queue must reject overflow");
        assert!(busy.contains("configured depth: 1"));
        waiter.abort();
        let _ = waiter.await;
        wait_for_waiting_jobs(&scheduler, 0).await;
        drop(active);
    }

    #[tokio::test]
    async fn policy_shrink_preserves_active_jobs_and_blocks_new_admission_until_converged() {
        let (scheduler, _config_dir) = test_scheduler(2, 8);
        let left = acquire_with_scheduler(Arc::clone(&scheduler), "left".to_string(), false, false)
            .await
            .expect("left permit");
        let right =
            acquire_with_scheduler(Arc::clone(&scheduler), "right".to_string(), false, false)
                .await
                .expect("right permit");
        let mut limits = scheduler.policy.snapshot().limits;
        limits.max_concurrent_compile_jobs = 1;
        scheduler.policy.update(limits).expect("shrink policy");
        scheduler.notify.notify_waiters();

        let scheduler_for_waiter = Arc::clone(&scheduler);
        let next = tokio::spawn(async move {
            acquire_with_scheduler(scheduler_for_waiter, "next".to_string(), false, false).await
        });
        wait_for_waiting_jobs(&scheduler, 1).await;
        drop(left);
        tokio::task::yield_now().await;
        assert_eq!(scheduler.state.lock().unwrap().active_jobs, 1);
        assert_eq!(scheduler.state.lock().unwrap().waiting.len(), 1);
        drop(right);
        let next = tokio::time::timeout(std::time::Duration::from_secs(1), next)
            .await
            .expect("next wakes after convergence")
            .expect("next task")
            .expect("next permit");
        drop(next);
    }
}
