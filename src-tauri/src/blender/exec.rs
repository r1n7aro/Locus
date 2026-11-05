//! One execution in a Blender instance, shared by `blender_execute` and the
//! SDK (`locus.blender.execute`, `@bl.function`, `execute_file`): resolve
//! the target, send `exec`, stream output, classify failures (not executed /
//! state uncertain) and collect warnings.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use super::hub::{self, ExecOutcome, Hub, Instance, InstanceSnapshot, OutputChunk};
use super::target::{self, Resolved, Scope, TargetUnavailable};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
pub const MIN_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(1800);
/// Queue waits above this are reported in the result.
const WAIT_WARNING: Duration = Duration::from_secs(1);

pub fn clamp_timeout(ms: Option<u64>) -> Duration {
    ms.map(Duration::from_millis)
        .unwrap_or(DEFAULT_TIMEOUT)
        .clamp(MIN_TIMEOUT, MAX_TIMEOUT)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecSuccess {
    pub instance: InstanceSnapshot,
    /// The host's result object (`ok`, `value`, `stdout`, `error`, ...).
    pub result: Value,
    pub waited_ms: u64,
    pub elapsed_ms: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecFailure {
    /// No usable instance; lists the candidates.
    TargetUnavailable(TargetUnavailable),
    /// The code never started; safe to retry.
    NotExecuted {
        message: String,
        instance: Option<InstanceSnapshot>,
    },
    /// The code started and its outcome is unknown (timeout, lost
    /// connection, cancel mid-run).
    TimedOut {
        message: String,
        instance: Option<InstanceSnapshot>,
    },
    /// Locus-side problem before reaching an instance (service off, launch
    /// failed, ...).
    Failed { message: String },
}

impl ExecFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::TargetUnavailable(_) => "target_unavailable",
            Self::NotExecuted { .. } => "not_executed",
            Self::TimedOut { .. } => "timed_out",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn render(&self) -> String {
        match self {
            Self::TargetUnavailable(error) => error.render(),
            Self::NotExecuted { message, instance } => {
                format!("{}Not executed (safe to retry): {message}", header_prefix(instance.as_ref()))
            }
            Self::TimedOut { message, instance } => {
                format!("{}State uncertain: {message}", header_prefix(instance.as_ref()))
            }
            Self::Failed { message } => message.clone(),
        }
    }
}

fn header_prefix(instance: Option<&InstanceSnapshot>) -> String {
    instance.map(|snapshot| format!("{}\n", header_line(snapshot))).unwrap_or_default()
}

/// `[Blender 4.2.1 · PID 1234 · hero.blend · interactive]`
pub fn header_line(snapshot: &InstanceSnapshot) -> String {
    let file = snapshot
        .file
        .as_deref()
        .map(|file| {
            std::path::Path::new(file)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| file.to_string())
        })
        .unwrap_or_else(|| "(unsaved)".to_string());
    let dirty = if snapshot.dirty { "*" } else { "" };
    format!(
        "[Blender {} · PID {} · {}{} · {}]",
        if snapshot.version_string.is_empty() { &snapshot.version } else { &snapshot.version_string },
        snapshot.pid,
        file,
        dirty,
        snapshot.mode
    )
}

/// Per-call SDK context made available to `locus` inside Blender.
pub fn sdk_context(scope: &Scope) -> Value {
    let env = crate::python_runtime::locus_sdk_invocation_env();
    let lookup = |name: &str| env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
    serde_json::json!({
        "sdk_url": lookup("LOCUS_SDK_URL"),
        "sdk_token": lookup("LOCUS_SDK_TOKEN"),
        "session_id": scope.session_id,
        "workspace": scope.root.display().to_string(),
        "checkout_id": scope.checkout_id,
        "sdk_dir": crate::python_runtime::locus_python_sdk_dir(hub::app_handle())
            .map(|dir| dir.display().to_string()),
    })
}

/// Instances that took part in a `python` tool call's undo batch.
fn batches() -> &'static Mutex<HashMap<String, BTreeSet<String>>> {
    static BATCHES: OnceLock<Mutex<HashMap<String, BTreeSet<String>>>> = OnceLock::new();
    BATCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The `python` tool call ended: push one undo step in every instance it
/// changed. Instances that miss this (crash, disconnect) flush on their own
/// after a timeout.
pub async fn end_batch(batch_id: &str) {
    let keys = batches().lock().ok().and_then(|mut batches| batches.remove(batch_id));
    let (Some(keys), Some(hub)) = (keys, hub::current()) else {
        return;
    };
    for key in keys {
        if let Some(instance) = hub.instance(&key) {
            let _ = instance.request("end_batch", batch_id, Duration::from_secs(10)).await;
        }
    }
}

/// Ends the `python` tool call's undo batch when dropped (any exit path).
/// Costs nothing when the call never touched Blender.
pub struct BatchGuard(String);

impl BatchGuard {
    pub fn new(batch_id: &str) -> Self {
        Self(batch_id.to_string())
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        let used = batches().lock().map(|batches| batches.contains_key(&self.0)).unwrap_or(false);
        if used {
            let batch = std::mem::take(&mut self.0);
            tauri::async_runtime::spawn(async move { end_batch(&batch).await });
        }
    }
}

pub fn running_hub() -> Result<Arc<Hub>, ExecFailure> {
    if !super::blender_integration_enabled() {
        return Err(ExecFailure::Failed {
            message: "Blender integration is turned off (Settings > Services > Blender).".to_string(),
        });
    }
    hub::current().ok_or_else(|| ExecFailure::Failed {
        message: "The Blender service of this workspace is not running.".to_string(),
    })
}

/// Resolve `target` (or the session/workspace default) to a connected
/// instance, launching a background instance when asked for one.
pub async fn resolve_instance(hub: &Arc<Hub>, scope: &Scope, explicit: Option<&str>) -> Result<Arc<Instance>, ExecFailure> {
    let instance = match target::resolve(hub, scope, explicit).map_err(ExecFailure::TargetUnavailable)? {
        Resolved::Instance(instance) => instance,
        Resolved::Background { version } => target::background_instance(hub, scope, version.as_deref(), None)
            .await
            .map_err(|message| ExecFailure::Failed { message })?,
    };
    match instance.connection() {
        hub::ConnectionState::Connected => Ok(instance),
        state => Err(ExecFailure::NotExecuted {
            message: format!("the instance is {}", serde_json::to_value(state).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()),
            instance: Some(instance.snapshot()),
        }),
    }
}

/// Send one `exec` payload. `payload` carries the mode-specific fields
/// (`code` / `function` / `path`, `args`, `namespace`, `readonly`,
/// `lenient`, `label`, `batch`); `context` is filled in here.
pub async fn execute(
    scope: &Scope,
    explicit_target: Option<&str>,
    payload: Value,
    timeout: Duration,
    on_output: impl FnMut(OutputChunk) + Send,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<ExecSuccess, ExecFailure> {
    let hub = running_hub()?;
    let instance = resolve_instance(&hub, scope, explicit_target).await?;
    execute_on(&hub, scope, &instance, payload, timeout, on_output, cancel).await
}

/// Send one `exec` payload to an already resolved instance.
pub async fn execute_on(
    hub: &Arc<Hub>,
    scope: &Scope,
    instance: &Arc<Instance>,
    mut payload: Value,
    timeout: Duration,
    on_output: impl FnMut(OutputChunk) + Send,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<ExecSuccess, ExecFailure> {
    if let Some(object) = payload.as_object_mut() {
        object.insert("context".to_string(), sdk_context(scope));
        if !object.contains_key("session") {
            if let Some(session) = scope.session_id.as_deref() {
                object.insert("session".to_string(), Value::from(session));
            }
        }
    }
    if let Some(batch) = payload.get("batch").and_then(Value::as_str).filter(|b| !b.is_empty()) {
        if payload.get("readonly").and_then(Value::as_bool) != Some(true) {
            if let Ok(mut batches) = batches().lock() {
                batches.entry(batch.to_string()).or_default().insert(instance.key.clone());
            }
        }
    }
    let started = Instant::now();
    let outcome = instance.exec(&payload, timeout, on_output, cancel).await;
    let elapsed = started.elapsed();
    let snapshot = instance.snapshot();
    match outcome {
        ExecOutcome::Completed(result) => {
            let duration_ms = result.get("duration_ms").and_then(Value::as_u64).unwrap_or(0);
            let elapsed_ms = elapsed.as_millis() as u64;
            let waited_ms = elapsed_ms.saturating_sub(duration_ms);
            let warnings = warnings(hub, scope, instance, &snapshot, Duration::from_millis(waited_ms));
            Ok(ExecSuccess { instance: snapshot, result, waited_ms, elapsed_ms, warnings })
        }
        ExecOutcome::NotExecuted(message) => Err(ExecFailure::NotExecuted { message, instance: Some(snapshot) }),
        ExecOutcome::Unknown(message) => Err(ExecFailure::TimedOut { message, instance: Some(snapshot) }),
    }
}

fn warnings(hub: &Hub, scope: &Scope, instance: &Instance, snapshot: &InstanceSnapshot, waited: Duration) -> Vec<String> {
    let mut warnings = Vec::new();
    let version = target::pinned_version(&scope.checkout_id);
    if let Some(version) = version {
        let wanted = super::blend_header::parse_version(&version);
        let actual = super::blend_header::parse_version(&snapshot.version);
        if (wanted.0, wanted.1) != (actual.0, actual.1) {
            warnings.push(format!(
                "this workspace is pinned to Blender {version}; the instance runs {}",
                snapshot.version_string
            ));
        }
    }
    if let Some(saved) = snapshot.saved_version.as_deref().filter(|v| !v.is_empty()) {
        let saved_v = super::blend_header::parse_version(saved);
        let actual = super::blend_header::parse_version(&snapshot.version);
        if (saved_v.0, saved_v.1) > (actual.0, actual.1) {
            warnings.push(format!(
                "the open file was saved by Blender {saved}, newer than this instance ({}); saving may lose data",
                snapshot.version_string
            ));
        }
    }
    if let Some(file) = snapshot.file.as_deref() {
        let key = super::installations::normalize_path(std::path::Path::new(file));
        let others = hub
            .instances()
            .iter()
            .filter(|other| other.key != instance.key)
            .filter(|other| {
                other
                    .registration()
                    .file
                    .as_deref()
                    .is_some_and(|f| super::installations::normalize_path(std::path::Path::new(f)) == key)
            })
            .map(|other| other.registration().pid.to_string())
            .collect::<Vec<_>>();
        if !others.is_empty() {
            warnings.push(format!(
                "the same file is open in other Blender instances (PID {}); saving in one overwrites the other",
                others.join(", ")
            ));
        }
    }
    if target::targeted_by_other_session(scope, instance) {
        warnings.push(
            "this instance is another session's Blender target; changes here can collide with that session's work"
                .to_string(),
        );
    }
    if waited >= WAIT_WARNING {
        warnings.push(format!("waited {} ms before Blender started the code", waited.as_millis()));
    }
    if let Some(stall) = snapshot.main_thread_stall_ms.filter(|ms| *ms >= hub::MAIN_THREAD_STUCK_MS) {
        warnings.push(format!("Blender's main thread had been busy for {stall} ms"));
    }
    warnings
}
