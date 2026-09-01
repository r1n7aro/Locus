use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};
use tokio::sync::{watch, Notify};

use crate::async_tasks::TaskProgressReporter;

const LIVENESS_POLL_INTERVAL: Duration = Duration::from_millis(500);

static ENABLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityEditorLockHolder {
    pub session_id: String,
    pub reason: String,
    pub acquired_at_unix_ms: u128,
}

impl UnityEditorLockHolder {
    fn summary(&self) -> String {
        format!(
            "session={} reason={}",
            self.session_id,
            serde_json::to_string(&self.reason).unwrap_or_else(|_| "\"<invalid>\"".to_string())
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnityEditorLockWaiter {
    id: String,
    session_id: String,
}

struct ProjectLockState {
    holder: Option<UnityEditorLockHolder>,
    waiters: VecDeque<UnityEditorLockWaiter>,
    notify: Arc<Notify>,
}

impl Default for ProjectLockState {
    fn default() -> Self {
        Self {
            holder: None,
            waiters: VecDeque::new(),
            notify: Arc::new(Notify::new()),
        }
    }
}

#[derive(Default)]
struct UnityEditorLockState {
    projects: HashMap<String, ProjectLockState>,
}

fn state() -> &'static Mutex<UnityEditorLockState> {
    static STATE: OnceLock<Mutex<UnityEditorLockState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(UnityEditorLockState::default()))
}

fn lock_state() -> std::sync::MutexGuard<'static, UnityEditorLockState> {
    state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
pub(crate) fn test_gate() -> std::sync::MutexGuard<'static, ()> {
    static GATE: OnceLock<Mutex<()>> = OnceLock::new();
    GATE.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn initialize(enabled: bool) {
    set_enabled(enabled);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        return;
    }

    let notifications = {
        let mut state = lock_state();
        let notifications = state
            .projects
            .values()
            .map(|project| project.notify.clone())
            .collect::<Vec<_>>();
        state.projects.clear();
        notifications
    };
    for notify in notifications {
        notify.notify_waiters();
    }
}

fn project_key(project_path: &str) -> String {
    let path = Path::new(project_path.trim());
    let resolved = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let key = resolved.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn remove_waiter(project_key: &str, waiter_id: &str) {
    let notify = {
        let mut state = lock_state();
        let Some(project) = state.projects.get_mut(project_key) else {
            return;
        };
        let previous_len = project.waiters.len();
        project.waiters.retain(|waiter| waiter.id != waiter_id);
        let changed = previous_len != project.waiters.len();
        let notify = changed.then(|| project.notify.clone());
        if project.holder.is_none() && project.waiters.is_empty() {
            state.projects.remove(project_key);
        }
        notify
    };
    if let Some(notify) = notify {
        notify.notify_waiters();
    }
}

struct WaiterRegistration {
    project_key: String,
    waiter_id: String,
    active: bool,
}

impl WaiterRegistration {
    fn new(project_key: String, waiter_id: String) -> Self {
        Self {
            project_key,
            waiter_id,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for WaiterRegistration {
    fn drop(&mut self) {
        if self.active {
            remove_waiter(&self.project_key, &self.waiter_id);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityEditorLockAcquireOutcome {
    pub already_owned: bool,
    pub waited_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnityEditorLockAcquireMode {
    Wait,
    Try,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnityEditorLockAcquireError {
    Busy {
        holder: Option<UnityEditorLockHolder>,
        next_waiter_session_id: Option<String>,
    },
    Cancelled {
        holder: Option<UnityEditorLockHolder>,
    },
    TimedOut {
        waited_ms: u128,
        holder: Option<UnityEditorLockHolder>,
    },
    Disabled,
}

impl UnityEditorLockAcquireError {
    pub fn message(&self) -> String {
        match self {
            Self::Busy {
                holder,
                next_waiter_session_id,
            } => {
                if let Some(holder) = holder.as_ref() {
                    format!(
                        "Unity Editor cooperative lock: status=busy current holder: {}.",
                        holder.summary()
                    )
                } else if let Some(session_id) = next_waiter_session_id.as_deref() {
                    format!(
                        "Unity Editor cooperative lock: status=busy reserved for queued session={session_id}; try mode did not jump the queue."
                    )
                } else {
                    "Unity Editor cooperative lock: status=busy transitioning between owners; try again after other asynchronous work."
                        .to_string()
                }
            }
            Self::Cancelled { holder } => {
                let holder = holder
                    .as_ref()
                    .map(UnityEditorLockHolder::summary)
                    .unwrap_or_else(|| "none".to_string());
                format!(
                    "Unity Editor cooperative lock wait was cancelled; current holder: {holder}."
                )
            }
            Self::Disabled => {
                "Unity Editor cooperative locking is disabled in Settings > Experimental."
                    .to_string()
            }
            Self::TimedOut { waited_ms, holder } => {
                let holder = holder
                    .as_ref()
                    .map(UnityEditorLockHolder::summary)
                    .unwrap_or_else(|| "transitioning to the next waiter".to_string());
                format!(
                    "Timed out after {}ms waiting for the Unity Editor cooperative lock; current holder: {holder}.",
                    waited_ms
                )
            }
        }
    }
}

trait SessionActivity {
    fn is_active<'a>(
        &'a self,
        session_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
}

struct AppSessionActivity<'a>(&'a AppHandle);

impl SessionActivity for AppSessionActivity<'_> {
    fn is_active<'a>(
        &'a self,
        session_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            let Some(active_tasks) = self.0.try_state::<crate::ActiveTasks>() else {
                return true;
            };
            let tasks = active_tasks.lock().await;
            tasks
                .get(session_id)
                .is_some_and(|task| !*task.done_rx.borrow())
        })
    }
}

fn current_holder(project_key: &str) -> Option<UnityEditorLockHolder> {
    lock_state()
        .projects
        .get(project_key)
        .and_then(|project| project.holder.clone())
}

fn release_if_holder_matches(project_key: &str, expected_session_id: &str) -> bool {
    let notify = {
        let mut state = lock_state();
        let Some(project) = state.projects.get_mut(project_key) else {
            return false;
        };
        if !project
            .holder
            .as_ref()
            .is_some_and(|holder| holder.session_id == expected_session_id)
        {
            return false;
        }
        project.holder = None;
        let notify = project.notify.clone();
        if project.waiters.is_empty() {
            state.projects.remove(project_key);
        }
        notify
    };
    notify.notify_waiters();
    true
}

pub async fn acquire(
    app: &AppHandle,
    project_path: &str,
    session_id: &str,
    reason: &str,
    mode: UnityEditorLockAcquireMode,
    timeout: Duration,
    mut cancel_rx: Option<watch::Receiver<bool>>,
    progress: Option<TaskProgressReporter>,
) -> Result<UnityEditorLockAcquireOutcome, UnityEditorLockAcquireError> {
    acquire_with_activity(
        &AppSessionActivity(app),
        project_path,
        session_id,
        reason,
        mode,
        timeout,
        cancel_rx.take(),
        progress,
    )
    .await
}

async fn acquire_with_activity(
    activity: &impl SessionActivity,
    project_path: &str,
    session_id: &str,
    reason: &str,
    mode: UnityEditorLockAcquireMode,
    timeout: Duration,
    mut cancel_rx: Option<watch::Receiver<bool>>,
    progress: Option<TaskProgressReporter>,
) -> Result<UnityEditorLockAcquireOutcome, UnityEditorLockAcquireError> {
    if !is_enabled() {
        return Err(UnityEditorLockAcquireError::Disabled);
    }

    let project_key = project_key(project_path);
    let waiter_id = uuid::Uuid::new_v4().simple().to_string();
    let mut registration = WaiterRegistration::new(project_key.clone(), waiter_id.clone());
    let started = Instant::now();
    let deadline = started + timeout;
    let reason = reason.trim().chars().take(240).collect::<String>();
    let mut last_progress_second = None;

    loop {
        if !is_enabled() {
            return Err(UnityEditorLockAcquireError::Disabled);
        }
        if cancel_rx
            .as_ref()
            .is_some_and(|receiver| *receiver.borrow())
        {
            return Err(UnityEditorLockAcquireError::Cancelled {
                holder: current_holder(&project_key),
            });
        }

        let (outcome, holder, next_waiter_session_id, notify) = {
            let mut state = lock_state();
            let project = state.projects.entry(project_key.clone()).or_default();

            if let Some(holder) = project.holder.as_mut() {
                if holder.session_id == session_id {
                    holder.reason = reason.clone();
                    project.waiters.retain(|waiter| waiter.id != waiter_id);
                    (
                        Some(UnityEditorLockAcquireOutcome {
                            already_owned: true,
                            waited_ms: started.elapsed().as_millis(),
                        }),
                        None,
                        None,
                        project.notify.clone(),
                    )
                } else {
                    if mode == UnityEditorLockAcquireMode::Wait
                        && !project.waiters.iter().any(|waiter| waiter.id == waiter_id)
                    {
                        project.waiters.push_back(UnityEditorLockWaiter {
                            id: waiter_id.clone(),
                            session_id: session_id.to_string(),
                        });
                    }
                    (None, Some(holder.clone()), None, project.notify.clone())
                }
            } else {
                if mode == UnityEditorLockAcquireMode::Try && !project.waiters.is_empty() {
                    let next_waiter_session_id = project
                        .waiters
                        .front()
                        .map(|waiter| waiter.session_id.clone());
                    (None, None, next_waiter_session_id, project.notify.clone())
                } else if mode == UnityEditorLockAcquireMode::Try {
                    project.holder = Some(UnityEditorLockHolder {
                        session_id: session_id.to_string(),
                        reason: reason.clone(),
                        acquired_at_unix_ms: now_unix_ms(),
                    });
                    (
                        Some(UnityEditorLockAcquireOutcome {
                            already_owned: false,
                            waited_ms: started.elapsed().as_millis(),
                        }),
                        None,
                        None,
                        project.notify.clone(),
                    )
                } else {
                    if !project.waiters.iter().any(|waiter| waiter.id == waiter_id) {
                        project.waiters.push_back(UnityEditorLockWaiter {
                            id: waiter_id.clone(),
                            session_id: session_id.to_string(),
                        });
                    }
                    let at_front = project
                        .waiters
                        .front()
                        .is_some_and(|waiter| waiter.id == waiter_id);
                    if at_front {
                        project.waiters.pop_front();
                        project.holder = Some(UnityEditorLockHolder {
                            session_id: session_id.to_string(),
                            reason: reason.clone(),
                            acquired_at_unix_ms: now_unix_ms(),
                        });
                        (
                            Some(UnityEditorLockAcquireOutcome {
                                already_owned: false,
                                waited_ms: started.elapsed().as_millis(),
                            }),
                            None,
                            None,
                            project.notify.clone(),
                        )
                    } else {
                        (None, None, None, project.notify.clone())
                    }
                }
            }
        };

        if let Some(outcome) = outcome {
            registration.disarm();
            return Ok(outcome);
        }

        if let Some(holder) = holder.as_ref() {
            if !activity.is_active(&holder.session_id).await {
                release_if_holder_matches(&project_key, &holder.session_id);
                continue;
            }
        }

        if mode == UnityEditorLockAcquireMode::Try {
            return Err(UnityEditorLockAcquireError::Busy {
                holder,
                next_waiter_session_id,
            });
        }

        let waited = started.elapsed();
        let waited_second = waited.as_secs();
        if last_progress_second != Some(waited_second) {
            if let Some(report) = progress.as_ref() {
                let holder = holder
                    .as_ref()
                    .map(UnityEditorLockHolder::summary)
                    .unwrap_or_else(|| "waiting for the previous release".to_string());
                report(format!(
                    "Waiting for Unity Editor cooperative lock: {holder}; waited={}s",
                    waited_second
                ));
            }
            last_progress_second = Some(waited_second);
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(UnityEditorLockAcquireError::TimedOut {
                waited_ms: started.elapsed().as_millis(),
                holder,
            });
        }
        let poll_delay = LIVENESS_POLL_INTERVAL.min(deadline.saturating_duration_since(now));
        if let Some(receiver) = cancel_rx.as_mut() {
            tokio::select! {
                _ = notify.notified() => {}
                _ = tokio::time::sleep(poll_delay) => {}
                changed = receiver.changed() => {
                    if changed.is_err() || *receiver.borrow() {
                        return Err(UnityEditorLockAcquireError::Cancelled {
                            holder: holder.clone(),
                        });
                    }
                }
            }
        } else {
            tokio::select! {
                _ = notify.notified() => {}
                _ = tokio::time::sleep(poll_delay) => {}
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnityEditorLockReleaseOutcome {
    Released,
    AlreadyFree,
}

pub fn release(
    project_path: &str,
    session_id: &str,
) -> Result<UnityEditorLockReleaseOutcome, UnityEditorLockHolder> {
    let project_key = project_key(project_path);
    let mut state = lock_state();
    let Some(project) = state.projects.get_mut(&project_key) else {
        return Ok(UnityEditorLockReleaseOutcome::AlreadyFree);
    };
    let Some(holder) = project.holder.as_ref() else {
        return Ok(UnityEditorLockReleaseOutcome::AlreadyFree);
    };
    if holder.session_id != session_id {
        return Err(holder.clone());
    }

    project.holder = None;
    let notify = project.notify.clone();
    if project.waiters.is_empty() {
        state.projects.remove(&project_key);
    }
    drop(state);
    notify.notify_waiters();
    Ok(UnityEditorLockReleaseOutcome::Released)
}

pub fn release_for_session(project_path: &str, session_id: &str) -> bool {
    release_if_holder_matches(&project_key(project_path), session_id)
}

pub fn holder_summary(holder: &UnityEditorLockHolder) -> String {
    holder.summary()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Default)]
    struct FakeSessionActivity {
        active: HashSet<String>,
    }

    impl SessionActivity for FakeSessionActivity {
        fn is_active<'a>(
            &'a self,
            session_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
            Box::pin(async move { self.active.contains(session_id) })
        }
    }

    fn reset() {
        set_enabled(false);
        set_enabled(true);
    }

    fn install_holder(project_path: &str, session_id: &str, reason: &str) {
        let mut state = lock_state();
        state.projects.insert(
            project_key(project_path),
            ProjectLockState {
                holder: Some(UnityEditorLockHolder {
                    session_id: session_id.to_string(),
                    reason: reason.to_string(),
                    acquired_at_unix_ms: 1,
                }),
                ..Default::default()
            },
        );
    }

    #[test]
    fn owner_release_is_scoped_to_project_and_session() {
        let _gate = test_gate();
        reset();
        install_holder("C:/ProjectA", "session-a", "play mode");
        install_holder("C:/ProjectB", "session-b", "compile");

        let foreign = release("C:/ProjectA", "session-b").expect_err("foreign release");
        assert_eq!(foreign.session_id, "session-a");
        assert_eq!(
            release("C:/ProjectA", "session-a"),
            Ok(UnityEditorLockReleaseOutcome::Released)
        );
        assert_eq!(
            release("C:/ProjectA", "session-a"),
            Ok(UnityEditorLockReleaseOutcome::AlreadyFree)
        );
        assert_eq!(
            release("C:/ProjectB", "session-b"),
            Ok(UnityEditorLockReleaseOutcome::Released)
        );
        set_enabled(false);
    }

    #[test]
    fn disabling_clears_cooperative_state() {
        let _gate = test_gate();
        reset();
        install_holder("C:/Project", "session-a", "reload");
        set_enabled(false);
        assert_eq!(
            release("C:/Project", "session-a"),
            Ok(UnityEditorLockReleaseOutcome::AlreadyFree)
        );
    }

    #[test]
    fn run_cleanup_only_releases_the_current_owner() {
        let _gate = test_gate();
        reset();
        install_holder("C:/Project", "session-a", "active scene");
        assert!(!release_for_session("C:/Project", "session-b"));
        assert!(release_for_session("C:/Project", "session-a"));
        set_enabled(false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn try_mode_returns_the_complete_active_holder_immediately() {
        let _gate = test_gate();
        reset();
        install_holder("C:/TryBusy", "session-active-owner", "enter Play Mode");
        let activity = FakeSessionActivity {
            active: HashSet::from(["session-active-owner".to_string()]),
        };

        let error = acquire_with_activity(
            &activity,
            "C:/TryBusy",
            "session-caller",
            "compile",
            UnityEditorLockAcquireMode::Try,
            Duration::from_secs(5),
            None,
            None,
        )
        .await
        .expect_err("active holder should make try mode fail");

        let UnityEditorLockAcquireError::Busy {
            holder: Some(holder),
            ..
        } = error
        else {
            panic!("expected busy error with holder");
        };
        assert_eq!(holder.session_id, "session-active-owner");
        assert!(holder.summary().contains("session=session-active-owner"));
        assert!(holder.summary().contains("enter Play Mode"));
        release("C:/TryBusy", "session-active-owner").expect("release active owner");
        set_enabled(false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn try_mode_reclaims_a_holder_whose_session_is_not_running() {
        let _gate = test_gate();
        reset();
        install_holder("C:/TryStale", "session-old", "stale compile");
        let activity = FakeSessionActivity::default();

        let outcome = acquire_with_activity(
            &activity,
            "C:/TryStale",
            "session-new",
            "enter Play Mode",
            UnityEditorLockAcquireMode::Try,
            Duration::from_secs(5),
            None,
            None,
        )
        .await
        .expect("stale owner should be reclaimed");

        assert!(!outcome.already_owned);
        assert_eq!(
            release("C:/TryStale", "session-new"),
            Ok(UnityEditorLockReleaseOutcome::Released)
        );
        set_enabled(false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn wait_mode_acquires_after_the_active_owner_releases() {
        let _gate = test_gate();
        reset();
        install_holder("C:/Wait", "session-owner", "active scene switch");
        let activity = FakeSessionActivity {
            active: HashSet::from(["session-owner".to_string()]),
        };

        let acquire = acquire_with_activity(
            &activity,
            "C:/Wait",
            "session-waiter",
            "recompile",
            UnityEditorLockAcquireMode::Wait,
            Duration::from_secs(2),
            None,
            None,
        );
        let release_owner = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            release("C:/Wait", "session-owner").expect("release owner")
        };
        let (outcome, release_outcome) = tokio::join!(acquire, release_owner);

        assert_eq!(release_outcome, UnityEditorLockReleaseOutcome::Released);
        assert!(!outcome.expect("waiter should acquire").already_owned);
        release("C:/Wait", "session-waiter").expect("release waiter");
        set_enabled(false);
    }
}
