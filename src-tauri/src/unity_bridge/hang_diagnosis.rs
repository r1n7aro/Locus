//! Automatic diagnosis of a wedged Unity main thread.
//!
//! When the semantic observer reports `unresponsive`, one capture runs per
//! incident (process identity + continuous unresponsive interval):
//!
//!   1. Preferred: the in-process broker (`locus_unity_native`) walks the main
//!      thread's stack with the OS unwinder and names managed frames through
//!      Mono (`main_thread_stack`, capability `broker_main_thread_stack_v1`).
//!   2. Fallback: an out-of-process stack scan (older plugin or broker
//!      failure). Native frames only, heuristic.
//!
//! Native frames are symbolized here (dbghelp + the PDBs Unity ships next to
//! the engine), classified as busy/waiting from the main thread's CPU time,
//! and reduced to one "likely location" — the first frame in user or agent
//! code when there is one. The result rides on `SemanticState` for the UI and
//! is appended to agent-facing errors (hang timeout, user restart).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::state_probe::SemanticState;
use super::unix_now_ms;

mod timeline;
pub use timeline::UnityStallAssessment;

/// Error code carried by Unity requests that were interrupted because the
/// user restarted an unresponsive Editor. Callers must not retry/reattach.
pub(crate) const UNITY_RESTART_INTERRUPT_CODE: &str = "unity_restarted_by_user";

const BROKER_STACK_CAPABILITY: &str = "broker_main_thread_stack_v1";
/// Brokers that cache managed names per domain generation. Only these get
/// timeline sampling: an older broker would attach a Mono thread per sample.
const BROKER_NAME_CACHE_CAPABILITY: &str = "broker_stack_name_cache_v1";
const TIMELINE_CAPTURE_REQUEST: &str = r#"{"samples":1,"intervalMs":10,"maxFrames":96}"#;
const BROKER_CAPTURE_TIMEOUT: Duration = Duration::from_secs(12);
const EXTERNAL_SCAN_TIMEOUT: Duration = Duration::from_secs(10);
const SYMBOLIZE_TIMEOUT: Duration = Duration::from_secs(45);
/// Re-entering `unresponsive` for the same process within this window reuses
/// the previous diagnosis instead of suspending the thread again.
const REUSE_DIAGNOSIS_MS: u64 = 60_000;
const AGENT_DIAGNOSIS_MAX_AGE_MS: u64 = 10 * 60_000;
const MAX_REPORTED_FRAMES: usize = 40;
const AGENT_STACK_FRAMES: usize = 14;
const BUSY_CPU_PERCENT: f32 = 25.0;

// ── Public model ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityHangFrame {
    pub index: u32,
    /// `managed` | `native` | `unknown`.
    pub kind: String,
    /// `user` | `agent` | `locus` | `unity` | `system` | `runtime` | `native` | `unknown`.
    pub origin: String,
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// One-line rendering for UI and agent text.
    pub display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityHangDiagnosis {
    /// `running` | `complete` | `failed`.
    pub status: String,
    pub process_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_created_at_ms: Option<u64>,
    pub started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_age_ms: Option<u64>,
    /// `native_broker` (in-process unwind + Mono names) | `external_scan`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `busy` | `waiting` | `unknown`.
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_percent: Option<f32>,
    /// Native wait primitive at the top of the stack, when waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    /// The likely location appeared in every sample.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_stable: Option<bool>,
    pub sample_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<UnityHangFrame>,
    /// `user_code` | `agent_code` | `managed` | `native` | `top`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_executions: Option<u32>,
    pub summary: String,
    pub frames: Vec<UnityHangFrame>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Timeline assessment of the incident (sampled every 5-30s while the
    /// main thread stays unresponsive). Absent until two samples exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall: Option<UnityStallAssessment>,
}

impl UnityHangDiagnosis {
    fn pending(target: &CaptureTarget, started_at_ms: u64) -> Self {
        Self {
            status: "running".to_string(),
            process_id: target.pid,
            process_created_at_ms: target.created,
            started_at_ms,
            captured_at_ms: None,
            heartbeat_age_ms: target.heartbeat_age_ms,
            source: None,
            verdict: "unknown".to_string(),
            cpu_percent: None,
            waiting_on: None,
            stack_stable: None,
            sample_count: 0,
            location: None,
            location_reason: None,
            active_executions: target.active_executions,
            summary: "Analyzing the Unity main thread".to_string(),
            frames: Vec::new(),
            notes: Vec::new(),
            error: None,
            stall: None,
        }
    }

    fn failed(target: &CaptureTarget, started_at_ms: u64, error: String, notes: Vec<String>) -> Self {
        Self {
            status: "failed".to_string(),
            captured_at_ms: Some(unix_now_ms()),
            summary: format!("Could not capture the Unity main-thread stack: {error}"),
            notes,
            error: Some(error),
            ..Self::pending(target, started_at_ms)
        }
    }
}

/// dbghelp is process-global and not thread-safe: every symbol session in
/// this process (state probe, background hook, hang symbolizer) takes this.
pub(in crate::unity_bridge) fn dbghelp_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

// ── Incident store ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CaptureTarget {
    pid: u32,
    created: Option<u64>,
    thread_id: Option<u32>,
    broker_stack: bool,
    /// The broker caches managed names, so repeated samples stay cheap.
    name_cache: bool,
    heartbeat_age_ms: Option<u64>,
    active_executions: Option<u32>,
    editor_dir: Option<String>,
}

#[derive(Debug)]
struct Incident {
    pid: u32,
    created: Option<u64>,
    active: bool,
    running: bool,
    generation: u64,
    started_at_ms: u64,
    completed_at_ms: Option<u64>,
    target: CaptureTarget,
    diagnosis: Option<UnityHangDiagnosis>,
    /// Latest timeline assessment; merged into the attached diagnosis.
    stall: Option<UnityStallAssessment>,
    /// Identifies the timeline sampler that owns `stall`; bumped whenever a
    /// new sampler starts (new incident or re-entry into `unresponsive`).
    timeline_epoch: u64,
}

fn incidents() -> &'static Mutex<HashMap<String, Incident>> {
    static STORE: OnceLock<Mutex<HashMap<String, Incident>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_unresponsive(state: &SemanticState) -> bool {
    state.phase == "unresponsive" && state.process.state == "running" && state.process.pid.is_some()
}

/// Called for every freshly fused state. Starts one capture per incident and
/// attaches the incident's diagnosis (pending or complete) to the state.
pub(super) fn note_semantic_state(
    project_path: &str,
    state: &mut SemanticState,
    broker: Option<&super::NativeBrokerStatus>,
) {
    let key = super::project_runtime_key(project_path);
    let now = unix_now_ms();
    let mut spawn = None;
    let mut spawn_timeline = None;
    if let Ok(mut store) = incidents().lock() {
        if is_unresponsive(state) {
            let pid = state.process.pid.unwrap_or_default();
            let created = state.process.pid_created_at_ms;
            let broker = broker.filter(|status| status.process_id.is_none_or(|id| id == pid));
            let target = CaptureTarget {
                pid,
                created,
                thread_id: broker.and_then(|status| status.main_thread_id).filter(|id| *id != 0),
                broker_stack: broker.is_some_and(|status| {
                    status.native_alive
                        && status
                            .broker_capabilities
                            .iter()
                            .chain(status.capabilities.iter())
                            .any(|capability| capability == BROKER_STACK_CAPABILITY)
                }),
                name_cache: broker.is_some_and(|status| {
                    status
                        .broker_capabilities
                        .iter()
                        .any(|capability| capability == BROKER_NAME_CACHE_CAPABILITY)
                }),
                heartbeat_age_ms: state.main_thread.heartbeat_age_ms,
                active_executions: state.main_thread.active_executions,
                editor_dir: state
                    .process
                    .path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).parent())
                    .map(|dir| dir.to_string_lossy().to_string()),
            };
            match store.get_mut(&key) {
                Some(entry) if entry.pid == pid && entry.created == created && entry.active => {
                    entry.target.heartbeat_age_ms = target.heartbeat_age_ms;
                }
                Some(entry)
                    if entry.pid == pid
                        && entry.created == created
                        && entry.completed_at_ms.is_some_and(|done| now.saturating_sub(done) < REUSE_DIAGNOSIS_MS) =>
                {
                    // Reuse the stack report, but measure the new stall afresh.
                    entry.active = true;
                    entry.stall = None;
                    entry.timeline_epoch += 1;
                    entry.target.heartbeat_age_ms = target.heartbeat_age_ms;
                    spawn_timeline = Some((entry.timeline_epoch, target));
                }
                existing => {
                    let generation = existing.as_ref().map(|entry| entry.generation + 1).unwrap_or(1);
                    let timeline_epoch = existing.map(|entry| entry.timeline_epoch + 1).unwrap_or(1);
                    store.insert(
                        key.clone(),
                        Incident {
                            pid,
                            created,
                            active: true,
                            running: true,
                            generation,
                            started_at_ms: now,
                            completed_at_ms: None,
                            target: target.clone(),
                            diagnosis: None,
                            stall: None,
                            timeline_epoch,
                        },
                    );
                    spawn_timeline = Some((timeline_epoch, target.clone()));
                    spawn = Some((generation, target));
                }
            }
        } else if let Some(entry) = store.get_mut(&key) {
            entry.active = false;
        }
    }
    if let Some((generation, target)) = spawn {
        let project = project_path.to_string();
        tauri::async_runtime::spawn(async move {
            run_incident(project, generation, target).await;
        });
    }
    if let Some((epoch, target)) =
        spawn_timeline.filter(|(_, target)| target.broker_stack && target.name_cache)
    {
        let project = project_path.to_string();
        tauri::async_runtime::spawn(async move {
            run_timeline(project, epoch, target).await;
        });
    }
    attach(project_path, state);
}

/// Attach the current incident's diagnosis to an `unresponsive` state.
pub(super) fn attach(project_path: &str, state: &mut SemanticState) {
    if !is_unresponsive(state) {
        state.hang_diagnosis = None;
        return;
    }
    let key = super::project_runtime_key(project_path);
    let Ok(store) = incidents().lock() else { return };
    state.hang_diagnosis = store.get(&key).and_then(|entry| {
        (entry.active
            && Some(entry.pid) == state.process.pid
            && entry.created == state.process.pid_created_at_ms)
            .then(|| current_diagnosis(entry))
    });
}

fn current_diagnosis(entry: &Incident) -> UnityHangDiagnosis {
    let mut diagnosis = entry
        .diagnosis
        .clone()
        .unwrap_or_else(|| UnityHangDiagnosis::pending(&entry.target, entry.started_at_ms));
    // The capture time is fixed; the hang keeps growing.
    if entry.target.heartbeat_age_ms.is_some() {
        diagnosis.heartbeat_age_ms = entry.target.heartbeat_age_ms;
    }
    diagnosis.stall = entry.stall.clone();
    diagnosis
}

// ── Timeline sampling ───────────────────────────────────────────────────────

fn timeline_is_current(project_path: &str, epoch: u64) -> bool {
    let key = super::project_runtime_key(project_path);
    incidents()
        .lock()
        .ok()
        .and_then(|store| {
            store
                .get(&key)
                .map(|entry| entry.active && entry.timeline_epoch == epoch)
        })
        .unwrap_or(false)
}

fn store_timeline_assessment(project_path: &str, epoch: u64, assessment: UnityStallAssessment) {
    let key = super::project_runtime_key(project_path);
    if let Ok(mut store) = incidents().lock() {
        if let Some(entry) = store.get_mut(&key).filter(|entry| entry.timeline_epoch == epoch) {
            if entry.stall.as_ref().map(|stall| &stall.kind) != Some(&assessment.kind) {
                tracing::info!(
                    log_module = "Locus",
                    "Unity stall assessment for pid {}: {} ({} samples, max suspend {}us)",
                    entry.pid,
                    timeline::describe(&assessment),
                    assessment.samples,
                    assessment.max_suspend_us.unwrap_or(0)
                );
            }
            entry.stall = Some(assessment);
        }
    }
}

/// Sample the main thread while this incident stays unresponsive. Stops as
/// soon as the observer sees the editor loop again (checked every second).
async fn run_timeline(project_path: String, epoch: u64, target: CaptureTarget) {
    let mut samples: std::collections::VecDeque<timeline::TimelineSample> =
        std::collections::VecDeque::with_capacity(timeline::MAX_RETAINED);
    let mut taken = 0u32;
    while timeline_is_current(&project_path, epoch) {
        match capture_timeline_sample(&project_path, target.pid).await {
            Ok(sample) => {
                if samples.len() >= timeline::MAX_RETAINED {
                    samples.pop_front();
                }
                samples.push_back(sample);
                let ordered: Vec<_> = samples.iter().cloned().collect();
                if let Some(assessment) = timeline::assess(&ordered) {
                    store_timeline_assessment(&project_path, epoch, assessment);
                }
            }
            // A concurrent full capture ("capture_in_progress") or a broker
            // hiccup only costs this sample.
            Err(error) => {
                tracing::debug!(log_module = "Locus", "Unity stall sample skipped: {error}")
            }
        }
        taken = taken.saturating_add(1);
        let wait = Duration::from_millis(timeline::next_interval_ms(taken));
        let started = std::time::Instant::now();
        while started.elapsed() < wait {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !timeline_is_current(&project_path, epoch) {
                return;
            }
        }
    }
}

async fn capture_timeline_sample(
    project_path: &str,
    pid: u32,
) -> Result<timeline::TimelineSample, String> {
    let response = super::transport::send_message_with_timeout(
        project_path,
        "main_thread_stack",
        TIMELINE_CAPTURE_REQUEST,
        BROKER_CAPTURE_TIMEOUT,
    )
    .await?;
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "main_thread_stack returned ok=false".to_string()));
    }
    let capture: BrokerCapture = serde_json::from_str(response.message.as_deref().unwrap_or(""))
        .map_err(|error| format!("invalid main_thread_stack payload: {error}"))?;
    if capture.process_id != 0 && capture.process_id != pid {
        return Err(format!("stack was captured from Unity pid {}", capture.process_id));
    }
    timeline_sample_from_capture(project_path, &capture)
}

fn timeline_sample_from_capture(
    project_path: &str,
    capture: &BrokerCapture,
) -> Result<timeline::TimelineSample, String> {
    let sample = capture
        .samples
        .iter()
        .find(|sample| !sample.frames.is_empty())
        .ok_or_else(|| "the main-thread stack was empty".to_string())?;
    let names: HashMap<u64, &ManagedName> =
        capture.managed.iter().map(|name| (name.ip, name)).collect();
    let frames = sample
        .frames
        .iter()
        .map(|frame| match names.get(&frame.ip) {
            Some(name) => timeline::TimelineFrame {
                ip: frame.ip,
                sp: frame.sp,
                method: Some(name.method.clone()),
                origin: managed_origin(&name.method, name.assembly.as_deref()),
                display: match (name.file.as_deref(), name.line) {
                    (Some(file), Some(line)) => format!(
                        "{} (at {}:{line})",
                        name.method,
                        relative_source_path(project_path, file)
                    ),
                    _ => name.method.clone(),
                },
            },
            None => timeline::TimelineFrame {
                ip: frame.ip,
                sp: frame.sp,
                method: None,
                origin: "native",
                display: format!("0x{:016X}", frame.ip),
            },
        })
        .collect();
    let at_ms = u64::try_from(capture.started_at_ms)
        .ok()
        .filter(|ms| *ms > 0)
        .unwrap_or_else(unix_now_ms);
    Ok(timeline::TimelineSample {
        at_ms,
        cpu_100ns: capture.cpu_end_100ns.or(capture.cpu_start_100ns),
        frames,
        names_ok: matches!(capture.managed_status.as_str(), "ok" | "skipped"),
        suspend_us: sample.suspend_us,
    })
}

async fn run_incident(project_path: String, generation: u64, target: CaptureTarget) {
    let started_at_ms = unix_now_ms();
    let diagnosis = diagnose(&project_path, &target, started_at_ms).await;
    tracing::info!(
        log_module = "Locus",
        "Unity hang diagnosis for pid {} ({}): {}",
        target.pid,
        diagnosis.source.as_deref().unwrap_or("none"),
        diagnosis.summary
    );
    let key = super::project_runtime_key(&project_path);
    if let Ok(mut store) = incidents().lock() {
        if let Some(entry) = store.get_mut(&key) {
            if entry.generation == generation {
                entry.running = false;
                entry.completed_at_ms = Some(unix_now_ms());
                entry.diagnosis = Some(diagnosis);
            }
        }
    }
}

/// Latest diagnosis for the project's (optionally given) process, waiting for
/// a capture that is still running. Drives an observation so a hang the
/// observer has not reported yet still starts its capture.
pub(crate) async fn wait_for_diagnosis(
    project_path: &str,
    pid: Option<u32>,
    timeout: Duration,
) -> Option<UnityHangDiagnosis> {
    let key = super::project_runtime_key(project_path);
    let started = std::time::Instant::now();
    let grace = Duration::from_secs(3).min(timeout);
    let mut last_observation: Option<std::time::Instant> = None;
    loop {
        let (done, running) = {
            let store = incidents().lock().ok()?;
            match store.get(&key) {
                Some(entry) if pid.is_none_or(|pid| pid == entry.pid) => {
                    let fresh = entry.completed_at_ms.is_some_and(|done| {
                        unix_now_ms().saturating_sub(done) < AGENT_DIAGNOSIS_MAX_AGE_MS
                    });
                    (
                        entry.diagnosis.clone().filter(|_| !entry.running && fresh),
                        entry.running,
                    )
                }
                _ => (None, false),
            }
        };
        if done.is_some() {
            return done;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout || (!running && elapsed >= grace) {
            return None;
        }
        if last_observation.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            last_observation = Some(std::time::Instant::now());
            let _ = super::unity_semantic_state(project_path).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Append the hang diagnosis to an agent-facing "main thread unresponsive"
/// error so the agent learns where the Editor is stuck.
pub(crate) async fn append_to_unresponsive_error(project_path: &str, error: String) -> String {
    match wait_for_diagnosis(project_path, None, Duration::from_secs(10)).await {
        Some(diagnosis) => format!("{error}\n\n{}", format_for_agent(&diagnosis)),
        None => error,
    }
}

/// The user chose to restart an unresponsive Editor: fail every Unity request
/// still waiting on that Editor with an explanation and the diagnosis, so
/// agents do not hang or silently reattach to the replacement process.
pub(crate) async fn interrupt_requests_for_user_restart(project_path: &str, pid: u32) {
    let diagnosis = wait_for_diagnosis(project_path, Some(pid), Duration::from_secs(5)).await;
    let mut message = format!(
        "{UNITY_RESTART_INTERRUPT_CODE}: The user restarted the unresponsive Unity Editor (pid {pid}) from Locus. \
This Unity request was interrupted and did not complete; anything it changed before the hang may be partially applied, \
and unsaved Editor state is lost. Do not resend the same operation unchanged."
    );
    if let Some(diagnosis) = diagnosis {
        message.push_str("\n\n");
        message.push_str(&format_for_agent(&diagnosis));
    }
    // Tool calls that are not waiting on the pipe (recompile/test polling,
    // calls queued behind the hung one) end through the restart notice.
    let project_key = super::project_runtime_key(project_path);
    let notice: Arc<str> = Arc::from(message.as_str());
    restart_notices().send_modify(|current| {
        current.seq += 1;
        current.project_key = project_key;
        current.message = notice;
    });
    super::transport::disconnect_with_reason(project_path, &message).await;
}

#[derive(Debug, Clone, Default)]
struct RestartNotice {
    seq: u64,
    project_key: String,
    message: Arc<str>,
}

fn restart_notices() -> &'static tokio::sync::watch::Sender<RestartNotice> {
    static NOTICES: OnceLock<tokio::sync::watch::Sender<RestartNotice>> = OnceLock::new();
    NOTICES.get_or_init(|| tokio::sync::watch::channel(RestartNotice::default()).0)
}

/// Resolves with the interruption message once the user restarts this
/// project's unresponsive Editor *after* this call. Create it before the work
/// it guards starts, so a restart during the work is never missed.
pub(crate) fn user_restart_interruption(
    project_path: &str,
) -> impl std::future::Future<Output = String> + Send + 'static {
    let project_key = super::project_runtime_key(project_path);
    let mut notices = restart_notices().subscribe();
    async move {
        loop {
            if notices.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
            let notice = notices.borrow_and_update().clone();
            if notice.project_key == project_key {
                return notice.message.to_string();
            }
        }
    }
}

pub(crate) fn is_user_restart_interrupt(error: &str) -> bool {
    error.contains(UNITY_RESTART_INTERRUPT_CODE)
}

// ── Capture ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BrokerCapture {
    thread_id: u32,
    process_id: u32,
    started_at_ms: i64,
    wall_ms: u64,
    cpu_start_100ns: Option<u64>,
    cpu_end_100ns: Option<u64>,
    samples: Vec<BrokerSample>,
    managed: Vec<ManagedName>,
    managed_status: String,
    managed_error: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BrokerSample {
    frames: Vec<BrokerFrame>,
    suspend_us: u64,
    unwind_stop: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct BrokerFrame {
    ip: u64,
    sp: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ManagedName {
    ip: u64,
    method: String,
    assembly: Option<String>,
    file: Option<String>,
    line: Option<u32>,
}

#[derive(Debug)]
struct RawCapture {
    source: &'static str,
    samples: Vec<Vec<u64>>,
    cpu_percent: Option<f32>,
    managed: HashMap<u64, ManagedName>,
    /// Why the walk of the latest sample stopped early, if it did.
    unwind_stop: Option<String>,
    notes: Vec<String>,
}

fn cpu_percent(start: Option<u64>, end: Option<u64>, wall_ms: u64) -> Option<f32> {
    let (start, end) = (start?, end?);
    if wall_ms == 0 || end < start {
        return None;
    }
    let percent = (end - start) as f64 / (wall_ms as f64 * 10_000.0) * 100.0;
    Some(percent.clamp(0.0, 100.0) as f32)
}

async fn capture_via_broker(project_path: &str, pid: u32) -> Result<RawCapture, String> {
    let response = super::transport::send_message_with_timeout(
        project_path,
        "main_thread_stack",
        r#"{"samples":3,"intervalMs":250,"maxFrames":96}"#,
        BROKER_CAPTURE_TIMEOUT,
    )
    .await?;
    if !response.ok {
        return Err(response
            .error
            .unwrap_or_else(|| "main_thread_stack returned ok=false".to_string()));
    }
    let capture: BrokerCapture = serde_json::from_str(response.message.as_deref().unwrap_or(""))
        .map_err(|error| format!("invalid main_thread_stack payload: {error}"))?;
    if capture.process_id != 0 && capture.process_id != pid {
        return Err(format!(
            "stack was captured from Unity pid {} instead of {pid}",
            capture.process_id
        ));
    }
    let mut notes = Vec::new();
    match capture.managed_status.as_str() {
        "ok" | "skipped" => {}
        "timeout" | "blocked" => notes.push(
            "Mono did not answer (it is likely held by the stuck main thread); managed frames are unnamed"
                .to_string(),
        ),
        other => notes.push(format!(
            "managed frame names unavailable ({other}{})",
            capture
                .managed_error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )),
    }
    let latest = capture.samples.iter().rev().find(|sample| !sample.frames.is_empty());
    let unwind_stop = latest.and_then(|sample| sample.unwind_stop.clone());
    if let Some(error) = capture.samples.iter().find_map(|sample| sample.error.clone()) {
        notes.push(format!("a stack sample failed: {error}"));
    }
    let _ = capture.thread_id;
    Ok(RawCapture {
        source: "native_broker",
        samples: capture
            .samples
            .iter()
            .filter(|sample| !sample.frames.is_empty())
            .map(|sample| sample.frames.iter().map(|frame| frame.ip).collect())
            .collect(),
        cpu_percent: cpu_percent(capture.cpu_start_100ns, capture.cpu_end_100ns, capture.wall_ms),
        managed: capture.managed.into_iter().map(|name| (name.ip, name)).collect(),
        unwind_stop,
        notes,
    })
}

async fn capture_external(target: &CaptureTarget) -> Result<RawCapture, String> {
    if target.created.is_some()
        && super::process::process_created_at_unix_ms(target.pid) != target.created
    {
        return Err(format!("Unity pid {} is no longer the hung process", target.pid));
    }
    let pid = target.pid;
    let thread = target.thread_id;
    let scan = tokio::time::timeout(
        EXTERNAL_SCAN_TIMEOUT,
        tauri::async_runtime::spawn_blocking(move || {
            super::state_probe::scan_main_thread_stack(pid, thread)
        }),
    )
    .await
    .map_err(|_| "external stack scan timed out".to_string())?
    .map_err(|error| format!("external stack scan task failed: {error}"))??;
    Ok(RawCapture {
        source: "external_scan",
        samples: scan.samples.into_iter().filter(|frames| !frames.is_empty()).collect(),
        cpu_percent: cpu_percent(scan.cpu_start_100ns, scan.cpu_end_100ns, scan.wall_ms),
        managed: HashMap::new(),
        unwind_stop: None,
        notes: vec![
            "stack reconstructed by scanning from outside Unity (heuristic); managed frames cannot be named"
                .to_string(),
        ],
    })
}

async fn diagnose(project_path: &str, target: &CaptureTarget, started_at_ms: u64) -> UnityHangDiagnosis {
    let mut notes = Vec::new();
    let mut raw = None;
    if target.broker_stack {
        match capture_via_broker(project_path, target.pid).await {
            Ok(capture) => raw = Some(capture),
            Err(error) => notes.push(format!("in-process stack capture failed: {error}")),
        }
    } else {
        notes.push(
            "the Locus Unity plugin cannot capture the stack in-process; update the plugin to see managed (C#) frames"
                .to_string(),
        );
    }
    if raw.is_none() {
        match capture_external(target).await {
            Ok(capture) => raw = Some(capture),
            Err(error) => return UnityHangDiagnosis::failed(target, started_at_ms, error, notes),
        }
    }
    let Some(mut raw) = raw else {
        return UnityHangDiagnosis::failed(target, started_at_ms, "no capture".to_string(), notes);
    };
    if raw.samples.is_empty() {
        return UnityHangDiagnosis::failed(
            target,
            started_at_ms,
            "the main-thread stack was empty".to_string(),
            notes,
        );
    }
    notes.append(&mut raw.notes);

    let symbols = match symbolize_capture(target, &raw).await {
        Ok(symbols) => symbols,
        Err(error) => {
            notes.push(format!("native symbols unavailable: {error}"));
            HashMap::new()
        }
    };
    build_diagnosis(project_path, target, started_at_ms, raw, &symbols, notes)
}

// ── Symbolization ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct NativeSymbol {
    module: String,
    symbol: Option<String>,
    displacement: u64,
    module_offset: u64,
}

/// `(frame ip, address to resolve)` for every frame Mono did not name.
/// Return addresses resolve one byte back so a call that ends its function
/// is attributed to that function.
fn native_lookups(raw: &RawCapture) -> Vec<(u64, u64)> {
    let mut seen = HashSet::new();
    let mut lookups = Vec::new();
    for frames in &raw.samples {
        for (index, ip) in frames.iter().copied().enumerate() {
            if raw.managed.contains_key(&ip) || !seen.insert(ip) {
                continue;
            }
            lookups.push((ip, if index == 0 { ip } else { ip.saturating_sub(1) }));
        }
    }
    lookups
}

async fn symbolize_capture(
    target: &CaptureTarget,
    raw: &RawCapture,
) -> Result<HashMap<u64, NativeSymbol>, String> {
    let lookups = native_lookups(raw);
    if lookups.is_empty() {
        return Ok(HashMap::new());
    }
    let pid = target.pid;
    let editor_dir = target.editor_dir.clone();
    tokio::time::timeout(
        SYMBOLIZE_TIMEOUT,
        tauri::async_runtime::spawn_blocking(move || {
            let modules = super::background_hook::process_modules(pid)?;
            Ok::<_, String>(symbols_imp::symbolize(&modules, &lookups, editor_dir.as_deref()))
        }),
    )
    .await
    .map_err(|_| "symbol lookup timed out".to_string())?
    .map_err(|error| format!("symbol task failed: {error}"))?
}

/// Symbolize native frame addresses of another process. Blender reuses the
/// Unity symbolizer: PDBs beside the loaded modules (`blender.pdb` next to
/// `blender.exe` in builds that ship one) are found through the module
/// directories. Frames without symbols fall back to `module+offset`.
pub(crate) async fn symbolize_native_frames(pid: u32, frames: Vec<u64>) -> Vec<String> {
    let lookups = frames
        .iter()
        .enumerate()
        .map(|(index, ip)| (*ip, if index == 0 { *ip } else { ip.saturating_sub(1) }))
        .collect::<Vec<_>>();
    let symbols = tokio::time::timeout(
        SYMBOLIZE_TIMEOUT,
        tauri::async_runtime::spawn_blocking(move || {
            let modules = super::background_hook::process_modules(pid)?;
            Ok::<_, String>(symbols_imp::symbolize(&modules, &lookups, None))
        }),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(Result::ok)
    .unwrap_or_default();
    frames
        .iter()
        .map(|ip| match symbols.get(ip) {
            Some(NativeSymbol { module, symbol: Some(name), displacement, .. }) => {
                format!("{module}!{name}+0x{displacement:x}")
            }
            Some(NativeSymbol { module, module_offset, .. }) => format!("{module}+0x{module_offset:x}"),
            None => format!("0x{ip:x}"),
        })
        .collect()
}

fn find_module<'a>(
    modules: &'a [super::background_hook::ProcessModule],
    address: u64,
) -> Option<&'a super::background_hook::ProcessModule> {
    modules
        .iter()
        .find(|module| address >= module.base && address < module.base.saturating_add(module.size))
}

#[cfg(target_os = "windows")]
mod symbols_imp {
    use std::collections::{HashMap, HashSet};
    use std::ffi::{c_void, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    use super::super::background_hook::ProcessModule;
    use super::NativeSymbol;

    type Bool = i32;
    type Dword = u32;
    type Dword64 = u64;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const SYMOPT_UNDNAME: Dword = 0x0000_0002;
    const SYMOPT_DEFERRED_LOADS: Dword = 0x0000_0004;
    const SYMOPT_FAIL_CRITICAL_ERRORS: Dword = 0x0000_0200;
    const SYMOPT_AUTO_PUBLICS: Dword = 0x0001_0000;
    const SYMOPT_NO_PROMPTS: Dword = 0x0008_0000;
    const MAX_SYM_NAME: usize = 2000;

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfoW {
        SizeOfStruct: Dword,
        TypeIndex: Dword,
        Reserved: [Dword64; 2],
        Index: Dword,
        Size: Dword,
        ModBase: Dword64,
        Flags: Dword,
        Value: Dword64,
        Address: Dword64,
        Register: Dword,
        Scope: Dword,
        Tag: Dword,
        NameLen: Dword,
        MaxNameLen: Dword,
        Name: [u16; 1],
    }

    #[repr(C)]
    struct SymbolInfoBufferW {
        symbol: SymbolInfoW,
        name: [u16; MAX_SYM_NAME],
    }

    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn SymSetOptions(o: Dword) -> Dword;
        fn SymInitializeW(h: Handle, path: *const u16, invade: Bool) -> Bool;
        fn SymLoadModuleExW(
            h: Handle,
            hf: Handle,
            img: *const u16,
            module: *const u16,
            base: Dword64,
            size: Dword,
            data: *mut c_void,
            flags: Dword,
        ) -> Dword64;
        fn SymFromAddrW(
            h: Handle,
            address: Dword64,
            displacement: *mut Dword64,
            symbol: *mut SymbolInfoW,
        ) -> Bool;
        fn SymCleanup(h: Handle) -> Bool;
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn symbolize(
        modules: &[ProcessModule],
        lookups: &[(u64, u64)],
        editor_dir: Option<&str>,
    ) -> HashMap<u64, NativeSymbol> {
        let mut out = HashMap::new();
        let mut involved: Vec<&ProcessModule> = Vec::new();
        let mut seen_bases = HashSet::new();
        for (ip, lookup) in lookups {
            if let Some(module) = super::find_module(modules, *lookup) {
                out.insert(
                    *ip,
                    NativeSymbol {
                        module: module.name.clone(),
                        symbol: None,
                        displacement: 0,
                        module_offset: lookup.saturating_sub(module.base),
                    },
                );
                if seen_bases.insert(module.base) {
                    involved.push(module);
                }
            }
        }
        if involved.is_empty() {
            return out;
        }

        // Unity ships its PDBs next to the engine (Mono's beside Unity.exe), so
        // the module directories are the whole search path. No symbol server.
        let mut dirs: Vec<String> = Vec::new();
        for dir in involved
            .iter()
            .filter_map(|module| Path::new(&module.path).parent().map(|dir| dir.to_string_lossy().to_string()))
            .chain(editor_dir.map(str::to_string))
        {
            if !dirs.iter().any(|known| known.eq_ignore_ascii_case(&dir)) {
                dirs.push(dir);
            }
        }
        let search = wide(OsStr::new(&dirs.join(";")));

        let Ok(_guard) = super::dbghelp_lock().lock() else { return out };
        let session = 0x4C48_4E47usize as Handle; // 'LHNG'
        unsafe {
            SymSetOptions(
                SYMOPT_UNDNAME
                    | SYMOPT_DEFERRED_LOADS
                    | SYMOPT_FAIL_CRITICAL_ERRORS
                    | SYMOPT_AUTO_PUBLICS
                    | SYMOPT_NO_PROMPTS,
            );
            if SymInitializeW(session, search.as_ptr(), FALSE) == 0 {
                return out;
            }
        }
        struct Session(Handle);
        impl Drop for Session {
            fn drop(&mut self) {
                unsafe {
                    SymCleanup(self.0);
                }
            }
        }
        let _session = Session(session);

        for module in &involved {
            let image = wide(OsStr::new(&module.path));
            unsafe {
                SymLoadModuleExW(
                    session,
                    null_mut(),
                    image.as_ptr(),
                    null_mut(),
                    module.base,
                    module.size.min(u64::from(u32::MAX)) as u32,
                    null_mut(),
                    0,
                );
            }
        }

        let mut storage: Box<SymbolInfoBufferW> = Box::new(unsafe { zeroed() });
        for (ip, lookup) in lookups {
            let Some(entry) = out.get_mut(ip) else { continue };
            storage.symbol.SizeOfStruct = size_of::<SymbolInfoW>() as u32;
            storage.symbol.MaxNameLen = MAX_SYM_NAME as u32;
            let mut displacement = 0u64;
            let ok = unsafe {
                SymFromAddrW(session, *lookup, &mut displacement, &mut storage.symbol) != 0
            };
            if !ok {
                continue;
            }
            let len = (storage.symbol.NameLen as usize).min(MAX_SYM_NAME - 1);
            let name_ptr = storage.symbol.Name.as_ptr();
            let name = unsafe { std::slice::from_raw_parts(name_ptr, len) };
            let name = String::from_utf16_lossy(name).trim().to_string();
            if !name.is_empty() {
                entry.symbol = Some(name);
                // Report against the frame address, not the adjusted lookup.
                entry.displacement = displacement + ip.saturating_sub(*lookup);
            }
        }
        out
    }
}

#[cfg(not(target_os = "windows"))]
mod symbols_imp {
    use std::collections::HashMap;

    use super::super::background_hook::ProcessModule;
    use super::NativeSymbol;

    pub(super) fn symbolize(
        _modules: &[ProcessModule],
        _lookups: &[(u64, u64)],
        _editor_dir: Option<&str>,
    ) -> HashMap<u64, NativeSymbol> {
        HashMap::new()
    }
}

// ── Analysis ────────────────────────────────────────────────────────────────

const WAIT_SYMBOLS: &[&str] = &[
    "NtWaitFor",
    "ZwWaitFor",
    "NtDelayExecution",
    "ZwDelayExecution",
    "NtRemoveIoCompletion",
    "NtSignalAndWaitForSingleObject",
    "NtUserMsgWaitForMultipleObjectsEx",
    "NtUserGetMessage",
    "NtUserWaitMessage",
    "NtWaitForAlertByThreadId",
    "NtWaitForKeyedEvent",
    "WaitForSingleObject",
    "WaitForMultipleObjects",
    "SleepConditionVariable",
    "RtlSleepConditionVariable",
    "SleepEx",
    "MsgWaitForMultipleObjects",
];

const SYSTEM_MODULES: &[&str] = &[
    "ntdll.dll",
    "kernelbase.dll",
    "kernel32.dll",
    "win32u.dll",
    "user32.dll",
    "ucrtbase.dll",
    "msvcrt.dll",
    "combase.dll",
];

fn managed_origin(method: &str, assembly: Option<&str>) -> &'static str {
    if method.starts_with("(wrapper") {
        return "runtime";
    }
    let Some(assembly) = assembly.map(str::trim).filter(|name| !name.is_empty()) else {
        return "unknown";
    };
    let lower = assembly.to_ascii_lowercase();
    if lower.starts_with("__locus") {
        "agent"
    } else if lower == "locus.editor" || lower.starts_with("locus.") {
        "locus"
    } else if lower.starts_with("unityengine") || lower.starts_with("unityeditor") || lower.starts_with("unity.") {
        "unity"
    } else if lower == "mscorlib"
        || lower == "netstandard"
        || lower == "system"
        || lower.starts_with("system.")
        || lower.starts_with("mono.")
        || lower.starts_with("microsoft.")
    {
        "system"
    } else {
        "user"
    }
}

fn relative_source_path(project_path: &str, file: &str) -> String {
    let normalize = |value: &str| value.replace('\\', "/");
    let project = normalize(super::strip_extended_path_prefix(project_path)).trim_end_matches('/').to_string();
    let file_norm = normalize(file);
    if !project.is_empty()
        && file_norm.len() > project.len() + 1
        && file_norm[..project.len()].eq_ignore_ascii_case(&project)
        && file_norm.as_bytes()[project.len()] == b'/'
    {
        file_norm[project.len() + 1..].to_string()
    } else {
        file_norm
    }
}

fn build_frame(
    project_path: &str,
    index: usize,
    ip: u64,
    raw: &RawCapture,
    symbols: &HashMap<u64, NativeSymbol>,
) -> UnityHangFrame {
    let ip_text = format!("0x{ip:016X}");
    if let Some(name) = raw.managed.get(&ip) {
        let origin = managed_origin(&name.method, name.assembly.as_deref());
        let file = name.file.as_deref().map(|file| relative_source_path(project_path, file));
        let display = match (&file, name.line) {
            (Some(file), Some(line)) => format!("{} (at {file}:{line})", name.method),
            (Some(file), None) => format!("{} (in {file})", name.method),
            _ => name.method.clone(),
        };
        return UnityHangFrame {
            index: index as u32,
            kind: "managed".to_string(),
            origin: origin.to_string(),
            ip: ip_text,
            module: None,
            symbol: Some(name.method.clone()),
            assembly: name.assembly.clone(),
            file,
            line: name.line,
            display,
        };
    }
    if let Some(native) = symbols.get(&ip) {
        let display = match &native.symbol {
            Some(symbol) if native.displacement > 0 => {
                format!("{}!{symbol}+0x{:X}", native.module, native.displacement)
            }
            Some(symbol) => format!("{}!{symbol}", native.module),
            None => format!("{}+0x{:X}", native.module, native.module_offset),
        };
        return UnityHangFrame {
            index: index as u32,
            kind: "native".to_string(),
            origin: "native".to_string(),
            ip: ip_text,
            module: Some(native.module.clone()),
            symbol: native.symbol.clone(),
            assembly: None,
            file: None,
            line: None,
            display,
        };
    }
    UnityHangFrame {
        index: index as u32,
        kind: "unknown".to_string(),
        origin: "unknown".to_string(),
        display: format!("{ip_text} (JIT or dynamic code)"),
        ip: ip_text,
        module: None,
        symbol: None,
        assembly: None,
        file: None,
        line: None,
    }
}

/// Identity of the function a frame is in (stable across samples).
fn frame_key(frame: &UnityHangFrame) -> String {
    match (&frame.module, &frame.symbol) {
        (_, Some(symbol)) if frame.kind == "managed" => symbol.clone(),
        (Some(module), Some(symbol)) => format!("{module}!{symbol}"),
        _ => frame.ip.clone(),
    }
}

fn is_system_module(module: Option<&str>) -> bool {
    let Some(module) = module else { return false };
    let lower = module.to_ascii_lowercase();
    SYSTEM_MODULES.contains(&lower.as_str())
        || lower.starts_with("msvcp")
        || lower.starts_with("vcruntime")
        || lower.starts_with("mono-")
}

/// Only the instruction pointer of a scanned stack is certain; deeper
/// candidates may be stale return addresses. Stay at the top, stepping once
/// past a system wait/syscall stub to its (most likely genuine) caller.
fn pick_scanned_location(frames: &[UnityHangFrame]) -> Option<(usize, &'static str)> {
    let top = frames.first()?;
    if top.kind == "unknown" {
        return Some((0, "managed_unnamed"));
    }
    if top.kind == "native" && is_system_module(top.module.as_deref()) {
        if let Some(caller) = frames.get(1) {
            if caller.kind == "native" && caller.symbol.is_some() && !is_system_module(caller.module.as_deref()) {
                return Some((1, "native"));
            }
        }
    }
    Some((0, "top"))
}

fn pick_location(frames: &[UnityHangFrame]) -> Option<(usize, &'static str)> {
    if let Some(index) = frames.iter().position(|frame| frame.origin == "user") {
        return Some((index, "user_code"));
    }
    if let Some(index) = frames.iter().position(|frame| frame.origin == "agent") {
        return Some((index, "agent_code"));
    }
    if let Some(index) = frames
        .iter()
        .position(|frame| frame.kind == "managed" && matches!(frame.origin.as_str(), "unity" | "locus" | "system"))
    {
        return Some((index, "managed"));
    }
    if let Some(index) = frames
        .iter()
        .position(|frame| frame.kind == "native" && frame.symbol.is_some() && !is_system_module(frame.module.as_deref()))
    {
        return Some((index, "native"));
    }
    (!frames.is_empty()).then_some((0, "top"))
}

fn waiting_primitive(frames: &[UnityHangFrame], depth: usize) -> Option<String> {
    frames.iter().take(depth).find_map(|frame| {
        let symbol = frame.symbol.as_deref()?;
        (frame.kind == "native" && WAIT_SYMBOLS.iter().any(|wait| symbol.contains(wait)))
            .then(|| symbol.to_string())
    })
}

fn build_diagnosis(
    project_path: &str,
    target: &CaptureTarget,
    started_at_ms: u64,
    raw: RawCapture,
    symbols: &HashMap<u64, NativeSymbol>,
    mut notes: Vec<String>,
) -> UnityHangDiagnosis {
    let sample_frames: Vec<Vec<UnityHangFrame>> = raw
        .samples
        .iter()
        .map(|ips| {
            ips.iter()
                .enumerate()
                .map(|(index, ip)| build_frame(project_path, index, *ip, &raw, symbols))
                .collect()
        })
        .collect();
    // The latest sample is the most current picture of the hang.
    let mut frames = sample_frames.last().cloned().unwrap_or_default();
    let scanned = raw.source == "external_scan";
    let location = if scanned { pick_scanned_location(&frames) } else { pick_location(&frames) }
        .map(|(index, reason)| (frames[index].clone(), reason));
    // An unnamed JIT frame has no function identity; its IP moves within a loop.
    let stack_stable = location
        .as_ref()
        .filter(|(_, reason)| sample_frames.len() > 1 && *reason != "managed_unnamed")
        .map(|(frame, _)| {
        let key = frame_key(frame);
        sample_frames
            .iter()
            .all(|sample| sample.iter().any(|candidate| frame_key(candidate) == key))
    });
    let waiting_on = waiting_primitive(&frames, if scanned { 2 } else { 6 });
    let verdict = match (raw.cpu_percent, &waiting_on) {
        (Some(percent), _) if percent >= BUSY_CPU_PERCENT => "busy",
        (Some(_), _) => "waiting",
        (None, Some(_)) => "waiting",
        (None, None) => "unknown",
    };
    if let Some(stop) = raw.unwind_stop.as_deref() {
        if !matches!(stop, "max_frames" | "stack_copy_end") {
            notes.push(format!("the stack walk stopped early ({stop})"));
        }
    }
    if target.active_executions.is_some_and(|count| count > 0) {
        notes.push("a Locus execution (agent C# code) was running on the main thread".to_string());
    }

    let location_text = match location.as_ref() {
        Some((_, "managed_unnamed")) => {
            "managed (C#) code whose method could not be named (update the Locus Unity plugin)".to_string()
        }
        Some((frame, _)) => frame.display.clone(),
        None => "an unknown location".to_string(),
    };
    let cpu_text = raw
        .cpu_percent
        .map(|percent| format!(" (main-thread CPU {percent:.0}%)"))
        .unwrap_or_default();
    let stability_text = match stack_stable {
        Some(true) => format!("; same location in all {} samples", sample_frames.len()),
        Some(false) => "; the location changed between samples".to_string(),
        None => String::new(),
    };
    let summary = match verdict {
        "busy" => format!("Main thread is busy executing code{cpu_text}, stuck in {location_text}{stability_text}."),
        "waiting" => format!(
            "Main thread is blocked waiting{}{cpu_text}, from {location_text}{stability_text}.",
            waiting_on
                .as_deref()
                .map(|wait| format!(" in {wait}"))
                .unwrap_or_default()
        ),
        _ => format!("Main thread is stuck in {location_text}{stability_text}."),
    };

    frames.truncate(MAX_REPORTED_FRAMES);
    UnityHangDiagnosis {
        status: "complete".to_string(),
        process_id: target.pid,
        process_created_at_ms: target.created,
        started_at_ms,
        captured_at_ms: Some(unix_now_ms()),
        heartbeat_age_ms: target.heartbeat_age_ms,
        source: Some(raw.source.to_string()),
        verdict: verdict.to_string(),
        cpu_percent: raw.cpu_percent,
        waiting_on,
        stack_stable,
        sample_count: sample_frames.len() as u32,
        location_reason: location.as_ref().map(|(_, reason)| reason.to_string()),
        location: location.map(|(frame, _)| frame),
        active_executions: target.active_executions,
        summary,
        frames,
        notes,
        error: None,
        stall: None,
    }
}

/// Agent code is running, the thread is busy and its location moves: progress,
/// not a wedge.
fn is_long_running_locus_execution(diagnosis: &UnityHangDiagnosis) -> bool {
    diagnosis.active_executions.is_some_and(|count| count > 0)
        && diagnosis.verdict == "busy"
        && diagnosis.stack_stable == Some(false)
}

/// Plain-text report for agents (tool errors). English, compact, actionable.
pub(crate) fn format_for_agent(diagnosis: &UnityHangDiagnosis) -> String {
    let mut lines = Vec::new();
    let age = diagnosis
        .heartbeat_age_ms
        .map(|age| format!(" (no editor-loop heartbeat for {}s)", age / 1_000))
        .unwrap_or_default();
    lines.push(format!(
        "Unity main-thread hang diagnosis, pid {}{age}:",
        diagnosis.process_id
    ));
    lines.push(format!("- Verdict: {}", diagnosis.summary));
    if let Some(stall) = &diagnosis.stall {
        lines.push(format!(
            "- Over time ({} samples): {}",
            stall.samples,
            timeline::describe(stall)
        ));
    }
    if let Some(location) = &diagnosis.location {
        let owner = match location.origin.as_str() {
            "agent" => " — code executed by the agent (unity_execute / run_states snippet)",
            "user" => " — project code",
            "locus" => " — Locus editor bridge",
            "unity" => " — Unity editor code",
            _ => "",
        };
        if diagnosis.location_reason.as_deref() == Some("managed_unnamed") {
            lines.push(format!(
                "- Likely location: managed (C#) code at {} whose method could not be named",
                location.ip
            ));
        } else {
            lines.push(format!("- Likely location: {}{owner}", location.display));
        }
    }
    if !diagnosis.frames.is_empty() {
        lines.push(format!(
            "- Main-thread stack, top first ({}):",
            diagnosis.source.as_deref().unwrap_or("capture")
        ));
        let location_index = diagnosis.location.as_ref().map(|frame| frame.index as usize);
        for frame in diagnosis.frames.iter().take(AGENT_STACK_FRAMES) {
            lines.push(format!("    #{} {}", frame.index, frame.display));
        }
        if let Some(index) = location_index.filter(|index| *index >= AGENT_STACK_FRAMES) {
            if let Some(frame) = diagnosis.frames.get(index) {
                lines.push("    ...".to_string());
                lines.push(format!("    #{} {}", frame.index, frame.display));
            }
        }
    }
    for note in &diagnosis.notes {
        lines.push(format!("- Note: {note}"));
    }
    if is_long_running_locus_execution(diagnosis) {
        lines.push(
            "- Hint: a Locus execution is still computing on the main thread (CPU busy, the location changes between \
samples), so this is most likely long synchronous work rather than a deadlock. Read its result later with \
`locus.wait_unity_execution`, or stop it with `locus.cancel_unity_execution` (loops stop at their next iteration). \
For long batches, print or call ctx.Progress per chunk and `await ctx.WaitFrame()` between chunks so the Editor keeps \
pumping."
                .to_string(),
        );
    } else if matches!(
        diagnosis.location.as_ref().map(|frame| frame.origin.as_str()),
        Some("agent" | "user")
    ) {
        lines.push(
            "- Hint: this code never returned to the Unity editor loop. Look for an endless loop or a blocking wait \
on the main thread (Task.Result/Wait(), Thread.Sleep, synchronous I/O, waiting for a frame/callback that can only \
run after it returns). Fix it before running it again."
                .to_string(),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed(ip: u64, method: &str, assembly: &str, file: Option<&str>, line: Option<u32>) -> ManagedName {
        ManagedName {
            ip,
            method: method.to_string(),
            assembly: Some(assembly.to_string()),
            file: file.map(str::to_string),
            line,
        }
    }

    fn target() -> CaptureTarget {
        CaptureTarget {
            pid: 42,
            created: Some(7),
            thread_id: Some(9),
            broker_stack: true,
            name_cache: true,
            heartbeat_age_ms: Some(31_000),
            active_executions: Some(0),
            editor_dir: None,
        }
    }

    fn native(module: &str, symbol: &str) -> NativeSymbol {
        NativeSymbol {
            module: module.to_string(),
            symbol: Some(symbol.to_string()),
            displacement: 0x10,
            module_offset: 0x1000,
        }
    }

    #[test]
    fn timeline_sample_keeps_stack_pointers_names_and_suspend_cost() {
        let payload = r#"{
            "version": 1, "threadId": 9, "processId": 42, "startedAtMs": 1700000000000,
            "wallMs": 0, "cpuStart100ns": 50000000, "cpuEnd100ns": 50000100,
            "samples": [{"capturedAtMs": 1700000000000, "suspendUs": 140, "stackBytes": 4096,
                         "frames": [{"ip": 16, "sp": 4096}, {"ip": 32, "sp": 4200}, {"ip": 48, "sp": 4300}]}],
            "managed": [
                {"ip": 32, "method": "Game.Boot:Init ()", "assembly": "Assembly-CSharp",
                 "file": "F:/Proj/Assets/Boot.cs", "line": 12, "nativeOffset": 4},
                {"ip": 48, "method": "__LocusSnippet:ExecuteSync ()", "assembly": "__LocusRaw_1", "nativeOffset": 8}
            ],
            "managedStatus": "ok"
        }"#;
        let capture: BrokerCapture = serde_json::from_str(payload).unwrap();
        let sample = timeline_sample_from_capture("F:/Proj", &capture).unwrap();
        assert_eq!(sample.at_ms, 1_700_000_000_000);
        assert_eq!(sample.cpu_100ns, Some(50_000_100));
        assert_eq!(sample.suspend_us, 140);
        assert!(sample.names_ok);
        assert_eq!(sample.frames.len(), 3);
        assert_eq!(sample.frames[0].origin, "native");
        assert_eq!(sample.frames[1].sp, 4200);
        assert_eq!(sample.frames[1].origin, "user");
        assert_eq!(sample.frames[1].display, "Game.Boot:Init () (at Assets/Boot.cs:12)");
        assert_eq!(sample.frames[2].origin, "agent");

        let timed_out: BrokerCapture = serde_json::from_str(
            &payload.replace(r#""managedStatus": "ok""#, r#""managedStatus": "timeout""#),
        )
        .unwrap();
        assert!(!timeline_sample_from_capture("F:/Proj", &timed_out).unwrap().names_ok);
    }

    #[test]
    fn agent_report_includes_the_timeline_assessment() {
        let raw = RawCapture {
            source: "native_broker",
            samples: vec![vec![0x10]],
            cpu_percent: Some(99.0),
            managed: HashMap::new(),
            unwind_stop: None,
            notes: Vec::new(),
        };
        let mut diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &HashMap::new(), Vec::new());
        diagnosis.stall = Some(UnityStallAssessment {
            kind: "suspected_infinite_loop".to_string(),
            owner: "user".to_string(),
            for_ms: 125_000,
            samples: 26,
            cpu_percent: Some(98.0),
            frame: Some("Game.Hash:Run () (at Assets/Hash.cs:9)".to_string()),
            blocks_progress: true,
            max_suspend_us: Some(210),
        });
        let text = format_for_agent(&diagnosis);
        assert!(text.contains("Over time (26 samples)"), "{text}");
        assert!(text.contains("running for 125s without returning"), "{text}");
        assert!(text.contains("Game.Hash:Run ()"), "{text}");
    }

    #[test]
    fn busy_loop_in_project_code_is_the_location() {
        let mut managed_names = HashMap::new();
        for name in [
            managed(0x20, "Game.Spawner:Update ()", "Assembly-CSharp", Some("F:/Proj/Assets/Spawner.cs"), Some(42)),
            managed(0x30, "UnityEditor.EditorApplication:Internal_CallUpdateFunctions ()", "UnityEditor.CoreModule", None, None),
        ] {
            managed_names.insert(name.ip, name);
        }
        let raw = RawCapture {
            source: "native_broker",
            samples: vec![vec![0x10, 0x20, 0x30, 0x40], vec![0x11, 0x20, 0x30, 0x40]],
            cpu_percent: Some(97.0),
            managed: managed_names,
            unwind_stop: None,
            notes: Vec::new(),
        };
        let mut symbols = HashMap::new();
        symbols.insert(0x10, native("Unity.dll", "LeakDetection::Record"));
        symbols.insert(0x11, native("Unity.dll", "LeakDetection::Record"));
        symbols.insert(0x40, native("mono-2.0-bdwgc.dll", "mono_jit_runtime_invoke"));
        let diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &symbols, Vec::new());
        assert_eq!(diagnosis.verdict, "busy");
        assert_eq!(diagnosis.location_reason.as_deref(), Some("user_code"));
        let location = diagnosis.location.as_ref().unwrap();
        assert_eq!(location.display, "Game.Spawner:Update () (at Assets/Spawner.cs:42)");
        assert_eq!(diagnosis.stack_stable, Some(true));
        assert!(diagnosis.summary.contains("busy"), "{}", diagnosis.summary);
        assert_eq!(diagnosis.frames[0].display, "Unity.dll!LeakDetection::Record+0x10");
        let text = format_for_agent(&diagnosis);
        assert!(text.contains("Likely location: Game.Spawner:Update () (at Assets/Spawner.cs:42) — project code"), "{text}");
        assert!(text.contains("Hint:"));
    }

    #[test]
    fn moving_locus_execution_is_reported_as_long_running_work() {
        let mut managed_names = HashMap::new();
        for name in [
            managed(0x20, "DustEcho.ECS.CleanupSystem:OnUpdate ()", "Assembly-CSharp", Some("F:/Proj/Assets/Cleanup.cs"), Some(64)),
            managed(0x21, "DustEcho.ECS.SimulationPipeline:OnUpdate ()", "Assembly-CSharp", Some("F:/Proj/Assets/Pipeline.cs"), Some(55)),
            managed(0x50, "Locus.RuntimeSnippets.__LocusAsyncSnippetHost:ExecuteSync ()", "__LocusRuntimeAsync_00000001", None, None),
        ] {
            managed_names.insert(name.ip, name);
        }
        let raw = RawCapture {
            source: "native_broker",
            samples: vec![vec![0x10, 0x20, 0x50], vec![0x10, 0x21, 0x50]],
            cpu_percent: Some(99.0),
            managed: managed_names,
            unwind_stop: None,
            notes: Vec::new(),
        };
        let mut symbols = HashMap::new();
        symbols.insert(0x10, native("mono-2.0-bdwgc.dll", "mono_jit_runtime_invoke"));
        let mut executing = target();
        executing.active_executions = Some(1);
        let diagnosis = build_diagnosis("F:/Proj", &executing, 1, raw, &symbols, Vec::new());
        assert_eq!(diagnosis.verdict, "busy");
        assert_eq!(diagnosis.stack_stable, Some(false));
        let text = format_for_agent(&diagnosis);
        assert!(text.contains("most likely long synchronous work"), "{text}");
        assert!(text.contains("locus.wait_unity_execution"), "{text}");
        assert!(!text.contains("Fix it before running it again"), "{text}");

        // The same busy stack without a Locus execution keeps the loop hint.
        let mut managed_names = HashMap::new();
        let spawner = managed(0x20, "Game.Spawner:Update ()", "Assembly-CSharp", Some("F:/Proj/Assets/Spawner.cs"), Some(42));
        managed_names.insert(spawner.ip, spawner);
        let raw = RawCapture {
            source: "native_broker",
            samples: vec![vec![0x10, 0x20], vec![0x11, 0x20]],
            cpu_percent: Some(99.0),
            managed: managed_names,
            unwind_stop: None,
            notes: Vec::new(),
        };
        let diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &symbols, Vec::new());
        assert!(format_for_agent(&diagnosis).contains("Fix it before running it again"));
    }

    #[test]
    fn idle_wait_reports_the_wait_primitive_and_agent_code() {
        let mut managed_names = HashMap::new();
        let snippet = managed(0x50, "__LocusSnippet:Run ()", "__LocusRaw_00000001_00000003", None, None);
        managed_names.insert(snippet.ip, snippet);
        let raw = RawCapture {
            source: "native_broker",
            samples: vec![vec![0x10, 0x20, 0x50]],
            cpu_percent: Some(0.4),
            managed: managed_names,
            unwind_stop: Some("invalid_return_address".to_string()),
            notes: Vec::new(),
        };
        let mut symbols = HashMap::new();
        symbols.insert(0x10, native("ntdll.dll", "NtWaitForSingleObject"));
        symbols.insert(0x20, native("KERNELBASE.dll", "WaitForSingleObjectEx"));
        let diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &symbols, Vec::new());
        assert_eq!(diagnosis.verdict, "waiting");
        assert_eq!(diagnosis.waiting_on.as_deref(), Some("NtWaitForSingleObject"));
        assert_eq!(diagnosis.location_reason.as_deref(), Some("agent_code"));
        assert!(diagnosis.notes.iter().any(|note| note.contains("invalid_return_address")));
        assert!(format_for_agent(&diagnosis).contains("code executed by the agent"));
    }

    #[test]
    fn native_only_capture_skips_system_modules_for_location() {
        let raw = RawCapture {
            source: "external_scan",
            samples: vec![vec![0x10, 0x20]],
            cpu_percent: None,
            managed: HashMap::new(),
            unwind_stop: None,
            notes: Vec::new(),
        };
        let mut symbols = HashMap::new();
        symbols.insert(0x10, native("ntdll.dll", "NtDelayExecution"));
        symbols.insert(0x20, native("Unity.dll", "AssetDatabase::Refresh"));
        let diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &symbols, Vec::new());
        assert_eq!(diagnosis.verdict, "waiting");
        assert_eq!(diagnosis.location.unwrap().display, "Unity.dll!AssetDatabase::Refresh+0x10");
        assert_eq!(diagnosis.stack_stable, None);
    }

    #[test]
    fn scanned_stack_trusts_only_the_top_frame() {
        let raw = RawCapture {
            source: "external_scan",
            samples: vec![vec![0x10, 0x20, 0x30], vec![0x11, 0x20, 0x30]],
            cpu_percent: Some(96.0),
            managed: HashMap::new(),
            unwind_stop: None,
            notes: Vec::new(),
        };
        let mut symbols = HashMap::new();
        // 0x10/0x11 are JIT code (no module); deeper candidates may be stale.
        symbols.insert(0x20, native("mono-2.0-bdwgc.dll", "mono_jit_runtime_invoke"));
        symbols.insert(0x30, native("Unity.dll", "scripting_thread_is_attached"));
        let diagnosis = build_diagnosis("F:/Proj", &target(), 1, raw, &symbols, Vec::new());
        assert_eq!(diagnosis.verdict, "busy");
        assert_eq!(diagnosis.location_reason.as_deref(), Some("managed_unnamed"));
        assert!(diagnosis.summary.contains("managed (C#) code"), "{}", diagnosis.summary);
        assert!(!diagnosis.summary.contains("scripting_thread_is_attached"));
        assert!(format_for_agent(&diagnosis).contains("whose method could not be named"));
    }

    #[test]
    fn managed_origins() {
        assert_eq!(managed_origin("A:B ()", Some("Assembly-CSharp-Editor")), "user");
        assert_eq!(managed_origin("A:B ()", Some("__LocusRunStates_1_2")), "agent");
        assert_eq!(managed_origin("A:B ()", Some("Locus.Editor")), "locus");
        assert_eq!(managed_origin("A:B ()", Some("UnityEditor.CoreModule")), "unity");
        assert_eq!(managed_origin("A:B ()", Some("Unity.Burst")), "unity");
        assert_eq!(managed_origin("A:B ()", Some("mscorlib")), "system");
        assert_eq!(managed_origin("(wrapper managed-to-native) X:Y ()", Some("UnityEngine")), "runtime");
    }

    #[test]
    fn cpu_percent_uses_100ns_units() {
        assert_eq!(cpu_percent(Some(0), Some(5_000_000), 500), Some(100.0));
        assert_eq!(cpu_percent(Some(0), Some(250_000), 500), Some(5.0));
        assert_eq!(cpu_percent(None, Some(1), 500), None);
    }

    #[cfg(target_os = "windows")]
    #[inline(never)]
    fn hang_diagnosis_test_spin(stop: &std::sync::atomic::AtomicBool) {
        let mut counter = 0u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            counter = std::hint::black_box(counter.wrapping_add(1));
        }
    }

    /// End to end on a live thread of this process: the out-of-process scan
    /// must find the spinning function and the symbolizer must name it.
    #[cfg(target_os = "windows")]
    #[test]
    fn external_scan_and_symbolizer_name_a_spinning_thread() {
        use std::sync::atomic::{AtomicBool, Ordering};
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
        }
        static STOP: AtomicBool = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            tx.send(unsafe { GetCurrentThreadId() }).unwrap();
            hang_diagnosis_test_spin(&STOP);
        });
        let tid = rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let pid = std::process::id();
        let scan = super::super::state_probe::scan_main_thread_stack(pid, Some(tid));
        STOP.store(true, Ordering::Relaxed);
        worker.join().unwrap();
        let scan = scan.expect("scan");
        assert_eq!(scan.samples.len(), 3);
        let raw = RawCapture {
            source: "external_scan",
            samples: scan.samples,
            cpu_percent: cpu_percent(scan.cpu_start_100ns, scan.cpu_end_100ns, scan.wall_ms),
            managed: HashMap::new(),
            unwind_stop: None,
            notes: Vec::new(),
        };
        assert!(raw.cpu_percent.unwrap_or(0.0) > 50.0, "spinning thread CPU: {:?}", raw.cpu_percent);
        let modules = super::super::background_hook::process_modules(pid).expect("modules");
        let symbols = symbols_imp::symbolize(&modules, &native_lookups(&raw), None);
        let names: Vec<String> = raw.samples[0]
            .iter()
            .filter_map(|ip| symbols.get(ip).and_then(|symbol| symbol.symbol.clone()))
            .collect();
        assert!(
            names.iter().any(|name| name.contains("hang_diagnosis_test_spin")
                || name.contains("AtomicBool")
                || name.contains("black_box")),
            "symbols: {names:?}"
        );
        // Frames above the spin come from the thread start-up path.
        assert!(names.iter().any(|name| name.contains("BaseThreadInitThunk") || name.contains("RtlUserThreadStart")),
            "symbols: {names:?}");
    }

    #[tokio::test]
    async fn user_restart_notice_reaches_only_calls_for_that_project_started_before_it() {
        let stuck = user_restart_interruption("F:/RestartNoticeA");
        let other = user_restart_interruption("F:/RestartNoticeB");
        tokio::pin!(stuck, other);
        restart_notices().send_modify(|notice| {
            notice.seq += 1;
            notice.project_key = super::super::project_runtime_key("F:/RestartNoticeA");
            notice.message = Arc::from("unity_restarted_by_user: test");
        });
        let message = tokio::time::timeout(Duration::from_secs(1), &mut stuck)
            .await
            .expect("notified");
        assert!(is_user_restart_interrupt(&message));
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut other).await.is_err());
        // A call that starts after the restart is not interrupted by it.
        let later = user_restart_interruption("F:/RestartNoticeA");
        assert!(tokio::time::timeout(Duration::from_millis(50), later).await.is_err());
    }

    #[test]
    fn source_paths_are_project_relative() {
        assert_eq!(relative_source_path(r"F:\Proj", r"F:\Proj\Assets\A.cs"), "Assets/A.cs");
        assert_eq!(relative_source_path("F:/Proj", "f:/proj/Packages/p/B.cs"), "Packages/p/B.cs");
        assert_eq!(relative_source_path("F:/Proj", "D:/Other/C.cs"), "D:/Other/C.cs");
    }

    /// Live check against a running Unity Editor whose main thread is stuck:
    /// `LOCUS_HANG_TEST_PROJECT=<project> LOCUS_HANG_TEST_PID=<pid>
    ///  [LOCUS_HANG_TEST_EXPECT=<location substring>] [LOCUS_HANG_TEST_EXTERNAL=1]
    ///  cargo test --lib live_unity_hang_diagnosis -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_unity_hang_diagnosis() {
        let project = std::env::var("LOCUS_HANG_TEST_PROJECT").expect("LOCUS_HANG_TEST_PROJECT");
        let pid: u32 = std::env::var("LOCUS_HANG_TEST_PID")
            .expect("LOCUS_HANG_TEST_PID")
            .parse()
            .expect("pid");
        let external = std::env::var("LOCUS_HANG_TEST_EXTERNAL").is_ok_and(|value| value == "1");
        super::super::initialize_native_bridge(true);
        let editor_dir = super::super::background_hook::process_modules(pid)
            .ok()
            .and_then(|modules| {
                modules
                    .into_iter()
                    .find(|module| module.name.eq_ignore_ascii_case("Unity.exe"))
            })
            .and_then(|module| std::path::Path::new(&module.path).parent().map(|dir| dir.to_string_lossy().to_string()));
        let target = CaptureTarget {
            pid,
            created: super::super::process::process_created_at_unix_ms(pid),
            thread_id: None,
            broker_stack: !external,
            name_cache: false,
            heartbeat_age_ms: None,
            active_executions: None,
            editor_dir,
        };
        let started = std::time::Instant::now();
        let diagnosis = diagnose(&project, &target, unix_now_ms()).await;
        let report = format!(
            "diagnosis took {:?}\n{}\n----\n{}\n",
            started.elapsed(),
            serde_json::to_string_pretty(&diagnosis).unwrap(),
            format_for_agent(&diagnosis)
        );
        eprintln!("{report}");
        if let Ok(path) = std::env::var("LOCUS_HANG_TEST_OUT") {
            std::fs::write(path, &report).unwrap();
        }
        assert_eq!(diagnosis.status, "complete", "{:?}", diagnosis.error);
        assert_eq!(
            diagnosis.source.as_deref(),
            Some(if external { "external_scan" } else { "native_broker" })
        );
        if let Ok(expected) = std::env::var("LOCUS_HANG_TEST_EXPECT") {
            let location = diagnosis.location.as_ref().map(|frame| frame.display.as_str()).unwrap_or("");
            assert!(location.contains(&expected), "location: {location}");
        }
    }
}
