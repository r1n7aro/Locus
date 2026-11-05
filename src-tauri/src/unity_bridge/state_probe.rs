//! Out-of-process native Unity editor-state probe.
//!
//! The named pipe (a managed object in Unity's Mono domain) dies during a
//! domain reload and goes silent when the editor's main thread wedges — the
//! exact moments we most want a state read. This module reads the editor's
//! state from OUTSIDE the process, with no cooperation from the C# layer, so
//! it keeps reporting through reloads and hangs.
//!
//! Normal monitoring reads the broker's heartbeat from shared memory once per
//! second. A live pipe or advancing CPU is not proof of main-thread progress.
//! After a sustained missing heartbeat, at most three passive CPU samples help
//! describe the failure. The automatic observer never suspends the Editor.
//!
//! Explicit diagnostics/self-tests can additionally use these tiers:
//!
//!   T1  main-thread stack classification — suspend the editor main thread for
//!       ~one bulk stack read, scan the live frames for a return address inside
//!       a known `MonoManager::*` domain-reload function, and report the reload
//!       sub-phase. Survives the pipe-dead window. Needs `unity_x64.pdb`
//!       (already required by the background hook) to resolve the ~10 reload
//!       symbols ONCE at init; the hot path is then pure Rust (a binary search
//!       over a cached address table) plus Win32 thread/RPM calls — no dbghelp.
//!   T2  CPU/liveness — main-thread CPU delta (`GetThreadTimes`) + whether the
//!       instruction pointer sits inside the Unity module. Separates "busy /
//!       executing" from "waiting"; neither proves forward progress.
//!   T3  process liveness only — alive / gone (handled by the caller via the
//!       existing `process` probe).
//!
//! Symbol names verified identical across Unity 2022.3 and Unity 6 (engine
//! module renamed `Unity.exe` → `Unity.dll`, already handled by the module
//! search). Resolution is by NAME, so per-build address differences are moot.
//!
//! Everything here is Windows-only; other platforms get inert stubs.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::unix_now_ms;

pub mod selftest;

// ── Public state model ───────────────────────────────────────────────

/// Sub-phase of a domain reload, derived from which `MonoManager` function
/// the main thread is executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadPhase {
    /// Tearing down the old AppDomain.
    Unloading,
    /// Building the fresh child domain.
    CreatingDomain,
    /// Loading the recompiled assemblies into the new domain.
    LoadingAssemblies,
    /// Rebuilding managed caches / running initializers.
    Finalizing,
    /// In `ReloadAssembly`/`BeginReloadAssembly` but not a finer sub-phase.
    Reloading,
}

impl ReloadPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ReloadPhase::Unloading => "unloading",
            ReloadPhase::CreatingDomain => "creating_domain",
            ReloadPhase::LoadingAssemblies => "loading_assemblies",
            ReloadPhase::Finalizing => "finalizing",
            ReloadPhase::Reloading => "reloading",
        }
    }
}

/// One out-of-process native sample of the editor main thread.
#[derive(Debug, Clone)]
pub struct NativeSample {
    /// `Some` when a live stack frame is inside a domain-reload function.
    /// Only the (opt-in) stack tier sets this; the passive tier leaves `None`.
    pub reloading: Option<ReloadPhase>,
    /// Instruction pointer is inside the Unity engine module (stack tier only).
    pub rip_in_unity: bool,
    /// Main-thread CPU advanced measurably since the previous sample.
    pub cpu_active: bool,
    /// Milliseconds the main thread has shown no CPU progress (0 if active).
    /// Works for both tiers, so hang detection needs no suspension.
    pub quiescent_for_ms: u64,
    /// The main thread was briefly suspended for a stack read this sample.
    pub suspended: bool,
    /// Microseconds the main thread was suspended (0 for the passive tier) —
    /// surfaced so the self-test can report the real freeze window.
    pub suspend_window_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedProcessState {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_created_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedChannelState {
    pub control_pipe: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedDomainState {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload_sub_phase: Option<String>,
    pub source: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedEditorMode {
    pub value: String,
    pub source: String,
    pub confidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedMainThreadState {
    pub state: String,
    pub cpu_active: bool,
    pub quiescent_for_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suspend_window_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_age_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_executions: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedSafetyState {
    pub can_call_unity_api: bool,
    pub can_modify_assets_safely: bool,
    pub recommended_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedStatePlane {
    pub observer: String,
    pub data_plane: String,
    pub native_broker: String,
    pub native_hook: String,
    pub history_samples: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_observed_at_ms: Option<u64>,
}

fn editor_mode_from_phase(phase: SemanticPhase) -> &'static str {
    match phase {
        SemanticPhase::Editing => "editing",
        SemanticPhase::Playing => "playing",
        SemanticPhase::Paused => "paused",
        _ => "unknown",
    }
}

fn phase_from_editor_mode(mode: &str) -> SemanticPhase {
    match mode {
        "editing" => SemanticPhase::Editing,
        "playing" => SemanticPhase::Playing,
        "paused" => SemanticPhase::Paused,
        _ => SemanticPhase::Unknown,
    }
}

fn editor_mode_from_status(status: &str) -> &'static str {
    match super::normalize_editor_status(status) {
        super::UNITY_EDITOR_STATUS_PLAYING => "playing",
        super::UNITY_EDITOR_STATUS_PLAYING_PAUSED => "paused",
        super::UNITY_EDITOR_STATUS_EDITING => "editing",
        _ => "unknown",
    }
}

fn status_from_editor_mode(mode: &str) -> &'static str {
    match mode {
        "playing" => super::UNITY_EDITOR_STATUS_PLAYING,
        "paused" => super::UNITY_EDITOR_STATUS_PLAYING_PAUSED,
        "editing" => super::UNITY_EDITOR_STATUS_EDITING,
        _ => super::UNITY_EDITOR_STATUS_DISCONNECTED,
    }
}

/// Fused, consumer-facing semantic phase. Priority-ordered when several
/// conditions hold (lifecycle/problem dominate activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticPhase {
    /// Process gone, editor instance file removed → clean shutdown.
    Quit,
    /// Process gone while the editor instance file lingered → abnormal exit.
    Crashed,
    /// Process up, control channel not yet ready (first load / project open).
    Starting,
    /// Alive but the main thread is wedged and not in a known reload.
    Unresponsive,
    /// Domain reload in progress (the pipe-dead window).
    Reloading,
    /// Unity Safe Mode: compiler errors are present and project/package managed code is disabled.
    SafeMode,
    /// Connected, edit mode, idle.
    Editing,
    /// Play mode running.
    Playing,
    /// Play mode paused.
    Paused,
    /// Alive but no usable signal (degraded: native off + pipe silent).
    Unknown,
}

impl SemanticPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            SemanticPhase::Quit => "quit",
            SemanticPhase::Crashed => "crashed",
            SemanticPhase::Starting => "starting",
            SemanticPhase::Unresponsive => "unresponsive",
            SemanticPhase::Reloading => "reloading",
            SemanticPhase::SafeMode => "safe_mode",
            SemanticPhase::Editing => "editing",
            SemanticPhase::Playing => "playing",
            SemanticPhase::Paused => "paused",
            SemanticPhase::Unknown => "unknown",
        }
    }

    /// Will this state resolve itself without user intervention?
    pub fn transient(self) -> bool {
        matches!(self, SemanticPhase::Starting | SemanticPhase::Reloading)
    }
}

/// The fused semantic state handed to the UI / agent / sidecar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticState {
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload_phase: Option<String>,
    /// Which tier produced the phase: `pipe` | `native_stack` | `native_cpu`
    /// | `process` | `inference`.
    pub source: String,
    /// `high` | `medium` | `low`.
    pub confidence: String,
    pub transient: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub checked_at_ms: u64,
    pub process: ObservedProcessState,
    pub channel: ObservedChannelState,
    pub domain: ObservedDomainState,
    pub editor_mode: ObservedEditorMode,
    pub main_thread: ObservedMainThreadState,
    pub safety: ObservedSafetyState,
    pub state_plane: ObservedStatePlane,
    pub editor_log: super::ObservedEditorLogState,
    /// Where the main thread is stuck — present while `phase` is
    /// `unresponsive` (status `running` until the capture completes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hang_diagnosis: Option<super::hang_diagnosis::UnityHangDiagnosis>,
}

impl SemanticState {
    pub(crate) fn can_restart_process(&self, pid: u32, created_at_ms: u64) -> bool {
        self.phase == "unresponsive"
            && self.safety.recommended_action == "diagnose_hang"
            && self.process.state == "running"
            && self.process.pid == Some(pid)
            && self.process.pid_created_at_ms == Some(created_at_ms)
    }

    fn new(
        phase: SemanticPhase,
        source: &str,
        confidence: &str,
        reload_phase: Option<ReloadPhase>,
        detail: Option<String>,
    ) -> Self {
        Self {
            phase: phase.as_str().to_string(),
            reload_phase: reload_phase.map(|p| p.as_str().to_string()),
            source: source.to_string(),
            confidence: confidence.to_string(),
            transient: phase.transient(),
            detail,
            checked_at_ms: unix_now_ms(),
            process: ObservedProcessState {
                state: "unknown".to_string(),
                pid: None,
                pid_created_at_ms: None,
                path: None,
            },
            channel: ObservedChannelState {
                control_pipe: "not_checked".to_string(),
                last_latency_ms: None,
                last_error: None,
                stale_ms: None,
            },
            domain: ObservedDomainState {
                phase: reload_phase
                    .map(|_| "reloading".to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
                reload_sub_phase: reload_phase.map(|p| p.as_str().to_string()),
                source: source.to_string(),
                confidence: confidence.to_string(),
            },
            editor_mode: ObservedEditorMode {
                value: editor_mode_from_phase(phase).to_string(),
                source: "unknown".to_string(),
                confidence: "low".to_string(),
                stale_ms: None,
                detail: None,
            },
            main_thread: ObservedMainThreadState {
                state: "unknown".to_string(),
                cpu_active: false,
                quiescent_for_ms: 0,
                stack_class: None,
                suspend_window_us: None,
                heartbeat_age_ms: None,
                active_executions: None,
            },
            safety: ObservedSafetyState {
                can_call_unity_api: false,
                can_modify_assets_safely: false,
                recommended_action: "avoid_unity_api".to_string(),
            },
            state_plane: ObservedStatePlane {
                observer: "on_demand".to_string(),
                data_plane: "direct".to_string(),
                native_broker: "not_checked".to_string(),
                native_hook: "not_checked".to_string(),
                history_samples: 0,
                last_observed_at_ms: None,
            },
            editor_log: super::ObservedEditorLogState::default(),
            hang_diagnosis: None,
        }
    }
}

/// Inputs to the fusion, gathered by the caller from the cheap signals.
#[derive(Debug, Clone)]
struct FusionInputs {
    /// Pipe answered `status` (richest channel; authoritative for the
    /// interactive modes it can see).
    pub pipe_connected: bool,
    /// Canonical pipe status when connected: editing | playing |
    /// playing_paused | disconnected.
    pub pipe_status: String,
    /// Editor process state from the probe. Only an EXPLICIT `NotRunning`
    /// triggers quit/crash; `Unknown` (probe failure) must fall through to
    /// unknown/inference rather than be misreported as a crash.
    pub process_state: super::UnityEditorProcessState,
    /// `EditorInstance.json` is present (distinguishes crash from clean quit
    /// once the process is gone).
    pub editor_instance_present: bool,
    /// Locus initiated a recompile and is waiting for the editor to come back.
    pub recompile_inflight: bool,
    /// The editor process appeared very recently (still in first load).
    pub recently_launched: bool,
    /// Native sample, when the native tier is enabled and succeeded.
    pub native: Option<NativeSample>,
    pub native_broker_status: Option<super::NativeBrokerStatus>,
    pub pipe_latency_ms: Option<u64>,
    pub pipe_error: Option<String>,
    pub control_channel_state: String,
    pub process_id: Option<u32>,
    pub process_created_at_ms: Option<u64>,
    pub process_path: Option<String>,
    pub last_known_editor_mode: Option<LastKnownEditorMode>,
    pub pending_editor_intent: Option<PendingEditorIntent>,
    pub native_hook: NativeHookObservation,
    pub observer: Option<ObserverObservation>,
    /// This process announced its shutdown through the broker. The broker and
    /// pipe disappear before the process does; the quit must survive that.
    pub editor_quitting: bool,
}

#[derive(Debug, Clone)]
struct LastKnownEditorMode {
    pid: Option<u32>,
    pid_created_at_ms: Option<u64>,
    mode: String,
    source: String,
    observed_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingEditorIntent {
    desired_mode: String,
    requested_at_ms: u64,
    acked_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct NativeHookObservation {
    state: String,
    patched: bool,
    source_available: bool,
    error: Option<String>,
}

impl Default for NativeHookObservation {
    fn default() -> Self {
        Self {
            state: "not_checked".to_string(),
            patched: false,
            source_available: false,
            error: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ObserverObservation {
    history_samples: u32,
    last_observed_at_ms: Option<u64>,
}

// ── Tunables ─────────────────────────────────────────────────────────

/// Bytes of stack read in the single suspended RPM. 64 KiB covers dozens of
/// frames while keeping the suspend window to one cross-process read.
const STACK_SCAN_BYTES: usize = 64 * 1024;
/// Instruction-pointer parked this long with no CPU + pipe silent + not
/// reloading ⇒ wedged.
const WEDGED_AFTER_MS: u64 = 4_000;
const LAST_KNOWN_EDITOR_MODE_MAX_STALE_MS: u64 = 5 * 60_000;
const LAST_KNOWN_EDITOR_MODE_MEDIUM_CONFIDENCE_MS: u64 = 60_000;
const PENDING_EDITOR_INTENT_MAX_STALE_MS: u64 = 60_000;
const NATIVE_BROKER_STATE_PROBE_CONSUMER: &str = "unity_state_probe";
const OBSERVER_HISTORY_LIMIT: usize = 120;
const OBSERVER_NORMAL_INTERVAL_MS: u64 = 1_000;
const OBSERVER_FAST_INTERVAL_MS: u64 = 1_000;
const MAIN_THREAD_DELAYED_MS: u64 = 10_000;
const MAIN_THREAD_UNRESPONSIVE_MS: u64 = 30_000;
const BROKER_SNAPSHOT_MAX_AGE_MS: u64 = 5_000;
const PROCESS_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1_000);
const PASSIVE_SAMPLE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

// ── Status surface (settings card) ───────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnityStateProbeTier {
    /// Disabled by the user.
    Disabled,
    /// Enabled, not yet exercised against an editor.
    Inactive,
    /// Running passively (CPU/liveness, no suspension) — the default; the
    /// stack tier attaches on demand.
    Passive,
    /// Full native stack classification attached (symbols resolved).
    Stack,
    /// Native init failed (no PDB / symbol resolve) → CPU/liveness only.
    CpuOnly,
    /// Native unavailable; relies on pipe + process inference.
    Inference,
    /// Not supported on this platform.
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityStateProbeStatus {
    pub enabled: bool,
    pub supported: bool,
    pub tier: UnityStateProbeTier,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    pub reload_symbols: u32,
    pub total_symbols: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub updated_at_ms: u64,
}

impl UnityStateProbeStatus {
    fn base(enabled: bool, tier: UnityStateProbeTier) -> Self {
        Self {
            enabled,
            supported: cfg!(target_os = "windows"),
            tier,
            process_id: None,
            reload_symbols: 0,
            total_symbols: 0,
            last_phase: None,
            error: None,
            updated_at_ms: unix_now_ms(),
        }
    }
}

struct ProbeRuntime {
    enabled: bool,
    project_scopes: HashMap<String, WorkspaceStatusKey>,
    workspace_statuses: HashMap<WorkspaceStatusKey, UnityStateProbeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkspaceStatusKey {
    checkout_id: String,
    workspace_generation: u64,
    service_instance_id: Option<String>,
    service_generation: Option<u64>,
}

impl From<&crate::workspace_service::event::WorkspaceEventScope> for WorkspaceStatusKey {
    fn from(scope: &crate::workspace_service::event::WorkspaceEventScope) -> Self {
        Self {
            checkout_id: scope.checkout_id.to_string(),
            workspace_generation: scope.workspace_generation,
            service_instance_id: scope.service_instance_id.as_ref().map(ToString::to_string),
            service_generation: scope.service_generation,
        }
    }
}

fn runtime() -> &'static Mutex<ProbeRuntime> {
    static RT: OnceLock<Mutex<ProbeRuntime>> = OnceLock::new();
    RT.get_or_init(|| {
        Mutex::new(ProbeRuntime {
            enabled: true,
            project_scopes: HashMap::new(),
            workspace_statuses: HashMap::new(),
        })
    })
}

fn base_status(enabled: bool) -> UnityStateProbeStatus {
    if enabled {
        UnityStateProbeStatus::base(true, UnityStateProbeTier::Inactive)
    } else {
        UnityStateProbeStatus::base(false, UnityStateProbeTier::Disabled)
    }
}

pub fn bind_workspace_scope(
    project_path: &str,
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) {
    let Ok(mut rt) = runtime().lock() else {
        return;
    };
    let key = WorkspaceStatusKey::from(scope);
    if let Some(previous) = rt
        .project_scopes
        .insert(project_key(project_path), key.clone())
    {
        if previous != key {
            rt.workspace_statuses.remove(&previous);
        }
    }
    let enabled = rt.enabled;
    rt.workspace_statuses
        .entry(key)
        .or_insert_with(|| base_status(enabled));
}

pub fn unbind_workspace_scope(
    project_path: &str,
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) {
    let Ok(mut rt) = runtime().lock() else {
        return;
    };
    let expected = WorkspaceStatusKey::from(scope);
    let project = project_key(project_path);
    if rt.project_scopes.get(&project) == Some(&expected) {
        rt.project_scopes.remove(&project);
        rt.workspace_statuses.remove(&expected);
    }
}

pub fn status_for_scope(
    scope: &crate::workspace_service::event::WorkspaceEventScope,
) -> UnityStateProbeStatus {
    runtime()
        .lock()
        .map(|rt| {
            rt.workspace_statuses
                .get(&WorkspaceStatusKey::from(scope))
                .cloned()
                .unwrap_or_else(|| base_status(rt.enabled))
        })
        .unwrap_or_else(|_| UnityStateProbeStatus::base(false, UnityStateProbeTier::Unsupported))
}

fn status_for_project(project_path: &str) -> UnityStateProbeStatus {
    runtime()
        .lock()
        .map(|rt| {
            rt.project_scopes
                .get(&project_key(project_path))
                .and_then(|key| rt.workspace_statuses.get(key))
                .cloned()
                .unwrap_or_else(|| base_status(rt.enabled))
        })
        .unwrap_or_else(|_| UnityStateProbeStatus::base(false, UnityStateProbeTier::Unsupported))
}

fn record_project_status(project_path: &str, update: impl FnOnce(&mut UnityStateProbeStatus)) {
    let Ok(mut rt) = runtime().lock() else {
        return;
    };
    let Some(key) = rt.project_scopes.get(&project_key(project_path)).cloned() else {
        return;
    };
    let enabled = rt.enabled;
    let status = rt
        .workspace_statuses
        .entry(key)
        .or_insert_with(|| base_status(enabled));
    update(status);
    status.updated_at_ms = unix_now_ms();
}

pub fn initialize(enabled: bool) {
    let mut rt = runtime().lock().expect("state probe runtime poisoned");
    rt.enabled = enabled;
    let status = base_status(enabled);
    for current in rt.workspace_statuses.values_mut() {
        *current = status.clone();
    }
    if let Ok(mut memory) = state_memory().lock() {
        *memory = StateMemory::default();
    }
    reset_observer_runtime();
}

pub fn enabled() -> bool {
    runtime().lock().map(|rt| rt.enabled).unwrap_or(false)
}

pub fn set_enabled(value: bool) -> UnityStateProbeStatus {
    let mut rt = runtime().lock().expect("state probe runtime poisoned");
    rt.enabled = value;
    let status = if value {
        UnityStateProbeStatus::base(true, UnityStateProbeTier::Inactive)
    } else {
        // Drop any cached per-process symbol tables so a re-enable re-resolves.
        imp::clear_cache();
        UnityStateProbeStatus::base(false, UnityStateProbeTier::Disabled)
    };
    for current in rt.workspace_statuses.values_mut() {
        *current = status.clone();
    }
    if let Ok(mut memory) = state_memory().lock() {
        *memory = StateMemory::default();
    }
    reset_observer_runtime();
    status
}

#[derive(Default)]
struct StateMemory {
    last_known: HashMap<String, LastKnownEditorMode>,
    pending_intents: HashMap<String, PendingEditorIntent>,
    quitting: HashMap<String, QuittingEditor>,
}

#[derive(Debug, Clone, Copy)]
struct QuittingEditor {
    pid: u32,
    pid_created_at_ms: Option<u64>,
}

fn note_editor_quitting(project_path: &str, pid: u32, pid_created_at_ms: Option<u64>) {
    if let Ok(mut memory) = state_memory().lock() {
        memory
            .quitting
            .insert(project_key(project_path), QuittingEditor { pid, pid_created_at_ms });
    }
}

/// Whether the live process is the one that announced its shutdown. A gone or
/// replaced process drops the marker; an unknown probe result keeps it.
fn editor_quitting(
    project_path: &str,
    process_state: &super::UnityEditorProcessState,
    process_id: Option<u32>,
    process_created_at_ms: Option<u64>,
) -> bool {
    let key = project_key(project_path);
    let Ok(mut memory) = state_memory().lock() else {
        return false;
    };
    let Some(marker) = memory.quitting.get(&key).copied() else {
        return false;
    };
    match process_state {
        super::UnityEditorProcessState::Unknown => false,
        super::UnityEditorProcessState::Running
            if process_id == Some(marker.pid)
                && process_identity_matches(
                    Some(marker.pid),
                    marker.pid_created_at_ms,
                    process_id,
                    process_created_at_ms,
                ) =>
        {
            true
        }
        _ => {
            memory.quitting.remove(&key);
            false
        }
    }
}

fn state_memory() -> &'static Mutex<StateMemory> {
    static MEMORY: OnceLock<Mutex<StateMemory>> = OnceLock::new();
    MEMORY.get_or_init(|| Mutex::new(StateMemory::default()))
}

#[derive(Default)]
struct ObserverRuntime {
    projects: HashMap<String, ObserverEntry>,
}

struct ObserverEntry {
    abort_handle: Option<tokio::task::AbortHandle>,
    current: Option<SemanticState>,
    history: VecDeque<SemanticState>,
    last_observed_at_ms: Option<u64>,
    last_accessed_at_ms: u64,
    /// Cuts the interval short when an event (process exit, dialog) makes the
    /// cached state stale.
    wake: std::sync::Arc<tokio::sync::Notify>,
}

impl ObserverEntry {
    fn new(now_ms: u64) -> Self {
        Self {
            abort_handle: None,
            current: None,
            history: VecDeque::with_capacity(OBSERVER_HISTORY_LIMIT),
            last_observed_at_ms: None,
            last_accessed_at_ms: now_ms,
            wake: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

fn observer_runtime() -> &'static Mutex<ObserverRuntime> {
    static OBSERVERS: OnceLock<Mutex<ObserverRuntime>> = OnceLock::new();
    OBSERVERS.get_or_init(|| Mutex::new(ObserverRuntime::default()))
}

fn reset_observer_runtime() {
    if let Ok(mut runtime) = observer_runtime().lock() {
        for entry in runtime.projects.values_mut() {
            if let Some(handle) = entry.abort_handle.take() {
                handle.abort();
            }
        }
        runtime.projects.clear();
    }
}

pub fn start_observer(project_path: &str) {
    ensure_observer(project_path);
}

pub fn stop_all_observers() {
    reset_observer_runtime();
}

/// Stop only the observer associated with one checkout. Workspace selection
/// changes keep every other checkout observer alive.
pub fn stop_observer(project_path: &str) {
    let key = project_key(project_path);
    if let Ok(mut runtime) = observer_runtime().lock() {
        if let Some(mut entry) = runtime.projects.remove(&key) {
            if let Some(handle) = entry.abort_handle.take() {
                handle.abort();
            }
        }
    }
}

fn ensure_observer(project_path: &str) {
    if project_path.trim().is_empty() || !enabled() {
        return;
    }

    let key = project_key(project_path);
    let now_ms = unix_now_ms();
    let mut spawn_wake = None;
    if let Ok(mut runtime) = observer_runtime().lock() {
        let entry = runtime
            .projects
            .entry(key)
            .or_insert_with(|| ObserverEntry::new(now_ms));
        entry.last_accessed_at_ms = now_ms;
        if entry.abort_handle.is_none() {
            spawn_wake = Some(entry.wake.clone());
        }
    }

    if let Some(wake) = spawn_wake {
        let project = project_path.to_string();
        let handle = tokio::spawn(observer_loop(project.clone(), wake)).abort_handle();
        if let Ok(mut runtime) = observer_runtime().lock() {
            if let Some(entry) = runtime.projects.get_mut(&project_key(&project)) {
                entry.abort_handle = Some(handle);
            }
        }
    }
}

fn cached_observer_state(project_path: &str, now_ms: u64) -> Option<SemanticState> {
    let mut runtime = observer_runtime().lock().ok()?;
    let entry = runtime.projects.get_mut(&project_key(project_path))?;
    entry.last_accessed_at_ms = now_ms;
    entry.current.clone()
}

pub(crate) fn clear_project_observer_state(project_path: &str) {
    if let Ok(mut runtime) = observer_runtime().lock() {
        if let Some(entry) = runtime.projects.get_mut(&project_key(project_path)) {
            entry.current = None;
            entry.history.clear();
            entry.last_observed_at_ms = None;
        }
    }
}

fn observer_observation(project_path: &str) -> Option<ObserverObservation> {
    let runtime = observer_runtime().lock().ok()?;
    let entry = runtime.projects.get(&project_key(project_path))?;
    Some(ObserverObservation {
        history_samples: entry.history.len().min(u32::MAX as usize) as u32,
        last_observed_at_ms: entry.last_observed_at_ms,
    })
}

fn store_observer_state(project_path: &str, mut state: SemanticState) -> SemanticState {
    let key = project_key(project_path);
    let observed_at_ms = state.checked_at_ms;
    let mut observation = ObserverObservation {
        history_samples: 0,
        last_observed_at_ms: Some(observed_at_ms),
    };

    if let Ok(mut runtime) = observer_runtime().lock() {
        let entry = runtime
            .projects
            .entry(key)
            .or_insert_with(|| ObserverEntry::new(observed_at_ms));
        entry.last_accessed_at_ms = observed_at_ms;
        entry.last_observed_at_ms = Some(observed_at_ms);
        if entry.history.len() >= OBSERVER_HISTORY_LIMIT {
            entry.history.pop_front();
        }
        entry.history.push_back(state.clone());
        observation.history_samples = entry.history.len().min(u32::MAX as usize) as u32;
    }

    state.state_plane.history_samples = observation.history_samples;
    state.state_plane.last_observed_at_ms = observation.last_observed_at_ms;
    state.state_plane.observer = "cache".to_string();
    if let Ok(mut runtime) = observer_runtime().lock() {
        if let Some(entry) = runtime.projects.get_mut(&project_key(project_path)) {
            if let Some(last) = entry.history.back_mut() {
                *last = state.clone();
            }
            entry.current = Some(state.clone());
        }
    }
    state
}

fn observer_interval_ms(state: &SemanticState) -> u64 {
    if matches!(
        state.channel.control_pipe.as_str(),
        "timeout" | "busy" | "disconnected" | "error"
    ) || state.domain.phase == "reloading"
        || state.safety.recommended_action != "proceed"
    {
        OBSERVER_FAST_INTERVAL_MS
    } else {
        OBSERVER_NORMAL_INTERVAL_MS
    }
}

async fn observer_loop(project_path: String, wake: std::sync::Arc<tokio::sync::Notify>) {
    let mut dialog_events = super::dialog::subscribe();
    let mut dialog_events_open = true;
    let mut publisher = super::SemanticStatePublisher::default();
    loop {
        if !enabled() {
            break;
        }
        let state = observe_project_once(&project_path, observer_observation(&project_path)).await;
        let interval_ms = observer_interval_ms(&state);
        let state = store_observer_state(&project_path, state);
        publisher.publish(&project_path, state);
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(interval_ms)) => {}
            _ = wake.notified() => {}
            changed = dialog_events.changed(), if dialog_events_open => {
                dialog_events_open = changed.is_ok();
            }
        }
    }

    if let Ok(mut runtime) = observer_runtime().lock() {
        if let Some(entry) = runtime.projects.get_mut(&project_key(&project_path)) {
            entry.abort_handle = None;
        }
    }
}

fn project_key(project_path: &str) -> String {
    super::project_runtime_key(project_path)
}

/// Re-sample now. Matches path spellings that differ only in case, slashes or
/// a trailing separator, since callers reach this from the process cache.
pub(crate) fn wake_observer(project_path: &str) {
    let wanted = super::normalize_project_path_for_state_plane(project_path);
    let Ok(runtime) = observer_runtime().lock() else {
        return;
    };
    for (key, entry) in &runtime.projects {
        if super::normalize_project_path_for_state_plane(key) == wanted {
            // `notify_one` keeps a permit when the loop is mid-sample.
            entry.wake.notify_one();
        }
    }
}

fn process_identity_matches(
    cached_pid: Option<u32>,
    cached_created_at_ms: Option<u64>,
    current_pid: Option<u32>,
    current_created_at_ms: Option<u64>,
) -> bool {
    match (cached_pid, current_pid) {
        (Some(cached), Some(current)) if cached != current => return false,
        _ => {}
    }
    match (cached_created_at_ms, current_created_at_ms) {
        (Some(cached), Some(current)) => cached == current,
        _ => true,
    }
}

fn lookup_last_known_editor_mode(
    project_path: &str,
    process_id: Option<u32>,
    process_created_at_ms: Option<u64>,
    now_ms: u64,
) -> Option<LastKnownEditorMode> {
    let key = project_key(project_path);
    let mut memory = state_memory().lock().ok()?;
    let item = memory.last_known.get(&key).cloned()?;
    let stale_ms = now_ms.saturating_sub(item.observed_at_ms);
    if stale_ms > LAST_KNOWN_EDITOR_MODE_MAX_STALE_MS
        || !process_identity_matches(
            item.pid,
            item.pid_created_at_ms,
            process_id,
            process_created_at_ms,
        )
    {
        memory.last_known.remove(&key);
        return None;
    }
    Some(item)
}

fn lookup_pending_editor_intent(project_path: &str, now_ms: u64) -> Option<PendingEditorIntent> {
    let key = project_key(project_path);
    let mut memory = state_memory().lock().ok()?;
    let intent = memory.pending_intents.get(&key).cloned()?;
    if now_ms.saturating_sub(intent.requested_at_ms) > PENDING_EDITOR_INTENT_MAX_STALE_MS {
        memory.pending_intents.remove(&key);
        return None;
    }
    Some(intent)
}

fn note_editor_mode_observation(
    project_path: &str,
    mode: &str,
    source: &str,
    process_id: Option<u32>,
    pid_created_at_ms: Option<u64>,
    observed_at_ms: u64,
) {
    if !matches!(mode, "editing" | "playing" | "paused") {
        return;
    }

    if let Ok(mut memory) = state_memory().lock() {
        let key = project_key(project_path);
        memory.last_known.insert(
            key.clone(),
            LastKnownEditorMode {
                pid: process_id,
                pid_created_at_ms,
                mode: mode.to_string(),
                source: source.to_string(),
                observed_at_ms,
            },
        );

        let should_clear_intent = memory
            .pending_intents
            .get(&key)
            .map(|intent| intent.desired_mode == mode)
            .unwrap_or(false);
        if should_clear_intent {
            memory.pending_intents.remove(&key);
        }
    }
}

pub(crate) fn note_pipe_editor_status(
    project_path: &str,
    status: &str,
    process_id: Option<u32>,
    observed_at_ms: u64,
) {
    let mode = editor_mode_from_status(status);
    if mode == "unknown" {
        return;
    }

    let pid_created_at_ms = process_id.and_then(super::process::process_created_at_unix_ms);
    note_editor_mode_observation(
        project_path,
        mode,
        "pipe",
        process_id,
        pid_created_at_ms,
        observed_at_ms,
    );
}

pub(crate) fn note_editor_status_intent(project_path: &str, desired_status: &str) {
    let desired_mode = editor_mode_from_status(desired_status);
    if desired_mode == "unknown" {
        return;
    }
    if let Ok(mut memory) = state_memory().lock() {
        memory.pending_intents.insert(
            project_key(project_path),
            PendingEditorIntent {
                desired_mode: desired_mode.to_string(),
                requested_at_ms: unix_now_ms(),
                acked_at_ms: None,
            },
        );
    }
}

pub(crate) fn note_editor_status_intent_acked(project_path: &str) {
    if let Ok(mut memory) = state_memory().lock() {
        if let Some(intent) = memory.pending_intents.get_mut(&project_key(project_path)) {
            intent.acked_at_ms = Some(unix_now_ms());
        }
    }
}

pub(crate) fn clear_editor_status_intent(project_path: &str) {
    if let Ok(mut memory) = state_memory().lock() {
        memory.pending_intents.remove(&project_key(project_path));
    }
}

pub(crate) fn fallback_editor_status_for_project(
    project_path: &str,
    process_id: Option<u32>,
    process_created_at_ms: Option<u64>,
) -> Option<String> {
    let now_ms = unix_now_ms();
    if let Some(intent) = lookup_pending_editor_intent(project_path, now_ms) {
        return Some(status_from_editor_mode(&intent.desired_mode).to_string());
    }
    lookup_last_known_editor_mode(project_path, process_id, process_created_at_ms, now_ms)
        .map(|mode| status_from_editor_mode(&mode.mode).to_string())
}

// ── Native sampling (blocking; call via spawn_blocking) ───────────────

/// Take one native sample of the editor main thread. Returns `Ok(None)` when
/// the native tier is unavailable for a non-fatal reason (no PDB, symbols not
/// resolvable, thread gone) — the caller then falls back to CPU/inference.
/// `allow_suspend = false` (the default for normal operation) takes the
/// passive tier: CPU/liveness only, the main thread is never suspended.
/// `allow_suspend = true` additionally suspends the main thread for one bulk
/// stack read to classify the domain-reload sub-phase — higher fidelity, used
/// on demand (e.g. the self-test).
pub fn sample_blocking(
    project_path: &str,
    process_id: u32,
    module_path: &str,
    allow_suspend: bool,
) -> Result<Option<NativeSample>, String> {
    if !enabled() {
        return Ok(None);
    }
    // Process creation time keys the per-PID cache so a restarted editor that
    // reuses the same PID never reads stale symbols / a stale main-thread id.
    let created = super::process::process_created_at_unix_ms(process_id);
    imp::sample(
        project_path,
        process_id,
        module_path,
        allow_suspend,
        created,
    )
}

// ── Fusion ───────────────────────────────────────────────────────────

fn process_state_name(state: &super::UnityEditorProcessState) -> &'static str {
    match state {
        super::UnityEditorProcessState::Running => "running",
        super::UnityEditorProcessState::NotRunning => "not_running",
        super::UnityEditorProcessState::Unknown => "unknown",
    }
}

async fn native_hook_observation_for_process(
    project_path: &str,
    process_id: Option<u32>,
    process_path: Option<String>,
) -> NativeHookObservation {
    if !super::background_hook::enabled() {
        return NativeHookObservation {
            state: "disabled".to_string(),
            patched: false,
            source_available: false,
            error: None,
        };
    }

    let Some(pid) = process_id else {
        let status = super::background_hook::status_for_project(project_path);
        return NativeHookObservation {
            state: format!("{:?}", status.state).to_ascii_lowercase(),
            patched: status.patched,
            source_available: false,
            error: status.error,
        };
    };

    let Some(path) = process_path.filter(|value| !value.trim().is_empty()) else {
        return NativeHookObservation {
            state: "failed".to_string(),
            patched: false,
            source_available: false,
            error: Some("Unity process path is unavailable".to_string()),
        };
    };

    let hook_project_path = project_path.to_string();
    let result = tauri::async_runtime::spawn_blocking(move || {
        super::background_hook::sync_for_project(&hook_project_path, pid, &path)
    })
    .await
    .map_err(|error| format!("native hook sync task failed: {error}"))
    .and_then(|result| result);

    match result {
        Ok(status) => NativeHookObservation {
            state: format!("{:?}", status.state).to_ascii_lowercase(),
            patched: status.patched,
            source_available: status.patched,
            error: status.error,
        },
        Err(error) => NativeHookObservation {
            state: "failed".to_string(),
            patched: false,
            source_available: false,
            error: Some(error),
        },
    }
}

fn broker_heartbeat_age(status: &super::NativeBrokerStatus, now_ms: u64) -> Option<u64> {
    let observed = u64::try_from(status.observed_at_ms).ok().filter(|value| *value > 0)?;
    if observed > now_ms.saturating_add(1_000)
        || now_ms.saturating_sub(observed) > BROKER_SNAPSHOT_MAX_AGE_MS {
        return None;
    }
    if status.broker_capabilities.iter().any(|cap| cap == "broker_main_thread_liveness_v1") {
        // Age was measured by the broker's monotonic clock. Do not turn a
        // cached snapshot's delivery time into a new main-thread heartbeat.
        return status.main_thread_heartbeat_age_ms;
    }
    let heartbeat = u64::try_from(status.last_managed_heartbeat_ms).ok().filter(|value| *value > 0)?;
    (heartbeat <= now_ms).then(|| now_ms.saturating_sub(heartbeat))
}

fn input_heartbeat_age(inputs: &FusionInputs) -> Option<u64> {
    inputs.native_broker_status.as_ref().and_then(|status| broker_heartbeat_age(status, unix_now_ms()))
}

fn main_thread_unresponsive(inputs: &FusionInputs) -> bool {
    inputs.native_broker_status.as_ref().is_some_and(|status| status.managed_state == "ready")
        && input_heartbeat_age(inputs).is_some_and(|age| age >= MAIN_THREAD_UNRESPONSIVE_MS)
}

/// A Locus execution that keeps publishing progress while it holds the main
/// thread: the Editor is busy on purpose, not hung. The publication age is
/// measured at the broker snapshot, so add the snapshot's own age.
fn live_locus_execution(inputs: &FusionInputs) -> Option<super::NativeBrokerExecution> {
    let status = inputs.native_broker_status.as_ref()?;
    let snapshot_age = u64::try_from(status.observed_at_ms)
        .ok()
        .map(|observed| unix_now_ms().saturating_sub(observed))
        .unwrap_or(u64::MAX);
    status
        .executions
        .iter()
        .map(|execution| super::NativeBrokerExecution {
            published_age_ms: execution.published_age_ms.saturating_add(snapshot_age),
            ..execution.clone()
        })
        .find(super::NativeBrokerExecution::is_live)
}

fn live_execution_detail(execution: &super::NativeBrokerExecution, away_secs: u64) -> String {
    let mut detail = format!(
        "Unity main thread is running Locus execution {} and has not returned to the editor loop for {away_secs}s",
        execution.execution_id
    );
    if let Some(idle) = execution.idle_ms {
        detail.push_str(&format!("; last output {}s ago", idle / 1_000));
    }
    detail
}

fn select_editor_mode(inputs: &FusionInputs, now_ms: u64) -> ObservedEditorMode {
    if let Some(status) = inputs.native_broker_status.as_ref().filter(|status| status.managed_state == "ready") {
        let age = broker_heartbeat_age(status, now_ms);
        let fresh = age.is_some_and(|age| age < MAIN_THREAD_DELAYED_MS);
        return ObservedEditorMode {
            value: editor_mode_from_status(&status.editor_status).to_string(),
            source: if fresh { "native_broker" } else { "last_known" }.to_string(),
            confidence: if fresh { "high" } else { "low" }.to_string(),
            stale_ms: age,
            detail: (!fresh).then(|| "Last reported editor mode; the main thread has not confirmed availability".to_string()),
        };
    }
    if inputs.pipe_connected {
        return ObservedEditorMode {
            value: editor_mode_from_status(&inputs.pipe_status).to_string(),
            source: "pipe".to_string(),
            confidence: "high".to_string(),
            stale_ms: None,
            detail: None,
        };
    }

    if matches!(
        inputs.process_state,
        super::UnityEditorProcessState::NotRunning | super::UnityEditorProcessState::Unknown
    ) {
        return ObservedEditorMode {
            value: "unknown".to_string(),
            source: "unknown".to_string(),
            confidence: "low".to_string(),
            stale_ms: None,
            detail: None,
        };
    }

    if let Some(status) = &inputs.native_broker_status {
        if status.managed_state == "ready" {
            let mode = editor_mode_from_status(&status.editor_status);
            if mode != "unknown" {
                return ObservedEditorMode {
                    value: mode.to_string(),
                    source: "native_broker".to_string(),
                    confidence: "high".to_string(),
                    stale_ms: None,
                    detail: Some(format!(
                        "native broker generation {}",
                        status.domain_generation
                    )),
                };
            }
        }
    }

    if let Some(intent) = &inputs.pending_editor_intent {
        let stale_ms = now_ms.saturating_sub(intent.requested_at_ms);
        if stale_ms <= PENDING_EDITOR_INTENT_MAX_STALE_MS {
            let confidence = if intent.acked_at_ms.is_some() {
                "medium"
            } else {
                "low"
            };
            return ObservedEditorMode {
                value: intent.desired_mode.clone(),
                source: "intent".to_string(),
                confidence: confidence.to_string(),
                stale_ms: Some(stale_ms),
                detail: Some(
                    "editor status request is awaiting control-channel confirmation".to_string(),
                ),
            };
        }
    }

    if let Some(last) = &inputs.last_known_editor_mode {
        let stale_ms = now_ms.saturating_sub(last.observed_at_ms);
        if stale_ms <= LAST_KNOWN_EDITOR_MODE_MAX_STALE_MS
            && process_identity_matches(
                last.pid,
                last.pid_created_at_ms,
                inputs.process_id,
                inputs.process_created_at_ms,
            )
        {
            let confidence = if stale_ms <= LAST_KNOWN_EDITOR_MODE_MEDIUM_CONFIDENCE_MS {
                "medium"
            } else {
                "low"
            };
            return ObservedEditorMode {
                value: last.mode.clone(),
                source: if last.source.is_empty() {
                    "last_known".to_string()
                } else {
                    "last_known".to_string()
                },
                confidence: confidence.to_string(),
                stale_ms: Some(stale_ms),
                detail: Some(format!(
                    "last high-confidence editor mode from {}",
                    last.source
                )),
            };
        }
    }

    ObservedEditorMode {
        value: "unknown".to_string(),
        source: "unknown".to_string(),
        confidence: "low".to_string(),
        stale_ms: None,
        detail: None,
    }
}

fn domain_state_for_inputs(inputs: &FusionInputs) -> ObservedDomainState {
    if inputs.process_state == super::UnityEditorProcessState::NotRunning {
        return ObservedDomainState {
            phase: "none".to_string(),
            reload_sub_phase: None,
            source: "process".to_string(),
            confidence: "high".to_string(),
        };
    }

    if let Some(status) = &inputs.native_broker_status {
        if status.managed_state == "reloading" {
            return ObservedDomainState {
                phase: "reloading".to_string(),
                reload_sub_phase: Some("reloading".to_string()),
                source: "native_broker".to_string(),
                confidence: "high".to_string(),
            };
        }
    }

    if let Some(native) = &inputs.native {
        if let Some(reload) = native.reloading {
            return ObservedDomainState {
                phase: "reloading".to_string(),
                reload_sub_phase: Some(reload.as_str().to_string()),
                source: "native_stack".to_string(),
                confidence: "high".to_string(),
            };
        }
    }

    // A responding managed pipe proves that the new domain is loaded. Keep
    // recompile_inflight as a fallback only for the pipe-disconnected reload
    // window; otherwise the operation's own wait guard would permanently make
    // its post-compile readiness probe observe a synthetic reload.
    if inputs.pipe_connected {
        return ObservedDomainState {
            phase: "none".to_string(),
            reload_sub_phase: None,
            source: "pipe".to_string(),
            confidence: "high".to_string(),
        };
    }

    if inputs
        .native_broker_status
        .as_ref()
        .map(|status| status.managed_state == "ready")
        .unwrap_or(false)
    {
        return ObservedDomainState {
            phase: "none".to_string(),
            reload_sub_phase: None,
            source: "native_broker".to_string(),
            confidence: "high".to_string(),
        };
    }

    if inputs.recompile_inflight {
        return ObservedDomainState {
            phase: "reloading".to_string(),
            reload_sub_phase: None,
            source: "inference".to_string(),
            confidence: "medium".to_string(),
        };
    }

    if let Some(native) = &inputs.native {
        return ObservedDomainState {
            phase: "none".to_string(),
            reload_sub_phase: None,
            source: if native.suspended {
                "native_stack".to_string()
            } else {
                "native_cpu".to_string()
            },
            confidence: "medium".to_string(),
        };
    }

    ObservedDomainState {
        phase: if inputs.pipe_connected {
            "none"
        } else {
            "unknown"
        }
        .to_string(),
        reload_sub_phase: None,
        source: if inputs.pipe_connected {
            "pipe"
        } else {
            "unknown"
        }
        .to_string(),
        confidence: if inputs.pipe_connected {
            "medium"
        } else {
            "low"
        }
        .to_string(),
    }
}

fn main_thread_state_for_inputs(
    inputs: &FusionInputs,
    editor_mode: &ObservedEditorMode,
) -> ObservedMainThreadState {
    if let Some(broker) = &inputs.native_broker_status {
        let age = input_heartbeat_age(inputs);
        let cpu = inputs.native.as_ref().map(|sample| sample.cpu_active);
        let state = if main_thread_unresponsive(inputs) {
            if live_locus_execution(inputs).is_some() {
                "unresponsive_executing"
            } else {
                match cpu { Some(true) => "unresponsive_busy", Some(false) => "unresponsive_waiting", None => "unresponsive" }
            }
        } else if age.is_some_and(|age| age >= MAIN_THREAD_DELAYED_MS) {
            "stalled"
        } else if age.is_some() && broker.managed_state == "ready" {
            "responsive"
        } else { "unknown" };
        return ObservedMainThreadState {
            state: state.to_string(), cpu_active: cpu.unwrap_or(false),
            quiescent_for_ms: inputs.native.as_ref().map(|sample| sample.quiescent_for_ms).unwrap_or(0),
            stack_class: None, suspend_window_us: None,
            heartbeat_age_ms: age, active_executions: Some(broker.active_executions),
        };
    }
    let Some(native) = &inputs.native else {
        return ObservedMainThreadState {
            state: "unknown".to_string(),
            cpu_active: false,
            quiescent_for_ms: 0,
            stack_class: None,
            suspend_window_us: None,
            heartbeat_age_ms: None,
            active_executions: None,
        };
    };

    let state = if native.cpu_active {
        "active"
    } else if !inputs.pipe_connected && native.quiescent_for_ms >= WEDGED_AFTER_MS {
        "hung"
    } else if !inputs.pipe_connected && editor_mode.value == "unknown" {
        "stalled"
    } else {
        "idle"
    };

    ObservedMainThreadState {
        state: state.to_string(),
        cpu_active: native.cpu_active,
        quiescent_for_ms: native.quiescent_for_ms,
        stack_class: native.reloading.map(|_| "reload".to_string()).or_else(|| {
            if native.cpu_active {
                Some("unknown".to_string())
            } else {
                None
            }
        }),
        suspend_window_us: if native.suspended {
            Some(native.suspend_window_us)
        } else {
            None
        },
        heartbeat_age_ms: None,
        active_executions: None,
    }
}

fn safety_for_state(
    inputs: &FusionInputs,
    domain: &ObservedDomainState,
    editor_mode: &ObservedEditorMode,
    main_thread: &ObservedMainThreadState,
) -> ObservedSafetyState {
    let process_running = inputs.process_state == super::UnityEditorProcessState::Running;
    let domain_ready = domain.phase == "none";
    let main_thread_usable = if inputs.native_broker_status.is_some() {
        main_thread.state == "responsive"
    } else {
        !matches!(main_thread.state.as_str(), "hung" | "stalled")
    };
    let can_call_unity_api =
        process_running && inputs.pipe_connected && domain_ready && main_thread_usable;
    let can_modify_assets_safely = can_call_unity_api && editor_mode.value == "editing";
    let recommended_action = if domain.phase == "reloading" {
        "wait_reload"
    } else if main_thread.state == "unresponsive_executing" {
        // Restarting would kill healthy agent work; wait or cancel it instead.
        "wait_execution"
    } else if main_thread.state == "hung" || main_thread.state.starts_with("unresponsive") {
        "diagnose_hang"
    } else if !main_thread_usable {
        "avoid_unity_api"
    } else if !inputs.pipe_connected && process_running {
        "avoid_unity_api"
    } else if !inputs.pipe_connected {
        "reconnect_control_pipe"
    } else {
        "proceed"
    };

    ObservedSafetyState {
        can_call_unity_api,
        can_modify_assets_safely,
        recommended_action: recommended_action.to_string(),
    }
}

fn decorate_state(mut state: SemanticState, inputs: &FusionInputs, now_ms: u64) -> SemanticState {
    let editor_mode = select_editor_mode(inputs, now_ms);
    let domain = domain_state_for_inputs(inputs);
    let main_thread = main_thread_state_for_inputs(inputs, &editor_mode);
    let safety = safety_for_state(inputs, &domain, &editor_mode, &main_thread);

    state.process = ObservedProcessState {
        state: process_state_name(&inputs.process_state).to_string(),
        pid: inputs.process_id,
        pid_created_at_ms: inputs.process_created_at_ms,
        path: inputs.process_path.clone(),
    };
    state.channel = ObservedChannelState {
        control_pipe: inputs.control_channel_state.clone(),
        last_latency_ms: inputs.pipe_latency_ms,
        last_error: inputs.pipe_error.clone(),
        stale_ms: editor_mode.stale_ms,
    };
    state.domain = domain;
    state.editor_mode = editor_mode;
    state.main_thread = main_thread;
    state.safety = safety;
    state.state_plane = ObservedStatePlane {
        observer: inputs
            .observer
            .as_ref()
            .map(|_| "cache".to_string())
            .unwrap_or_else(|| "direct".to_string()),
        data_plane: if inputs.native_broker_status.is_some() {
            "native_broker+native".to_string()
        } else {
            "native".to_string()
        },
        native_broker: inputs
            .native_broker_status
            .as_ref()
            .map(|status| {
                format!(
                    "{}:generation:{}:observed:{}",
                    status.managed_state, status.domain_generation, status.observed_at_ms
                )
            })
            .unwrap_or_else(|| "unavailable".to_string()),
        native_hook: if inputs.native_hook.source_available {
            if inputs.native_hook.patched {
                "available:patched".to_string()
            } else {
                "available".to_string()
            }
        } else if let Some(error) = &inputs.native_hook.error {
            format!("{}:{}", inputs.native_hook.state, error)
        } else {
            inputs.native_hook.state.clone()
        },
        history_samples: inputs
            .observer
            .as_ref()
            .map(|observer| observer.history_samples)
            .unwrap_or(0),
        last_observed_at_ms: inputs
            .observer
            .as_ref()
            .and_then(|observer| observer.last_observed_at_ms),
    };
    state
}

/// Combine the cheap signals into one semantic state. Pure and deterministic;
/// the priority order puts lifecycle/problem states above activity, and lets
/// the native tier override the pipe for the two conditions the pipe cannot
/// see (reloading, wedged).
fn fuse(inputs: &FusionInputs) -> SemanticState {
    let now_ms = unix_now_ms();
    let editor_mode = select_editor_mode(inputs, now_ms);

    // 1. EXPLICIT NotRunning → the editor is gone, and that dominates even a
    //    recompile we thought was in flight (a process that vanished mid-reload
    //    crashed). An `Unknown` process state (probe failure) must NOT land
    //    here — it falls through to native/pipe/inference below.
    if inputs.process_state == super::UnityEditorProcessState::NotRunning {
        let phase = if inputs.editor_instance_present {
            SemanticPhase::Crashed
        } else {
            SemanticPhase::Quit
        };
        return decorate_state(
            SemanticState::new(phase, "process", "high", None, None),
            inputs,
            now_ms,
        );
    }

    // 1b. The editor announced its shutdown. Its broker and pipe go away
    //     before the process exits; neither that silence nor the stopped
    //     heartbeat may turn the quit back into the last known mode or a hang.
    if inputs.editor_quitting {
        return decorate_state(
            SemanticState::new(
                SemanticPhase::Quit,
                "native_broker",
                "high",
                None,
                Some("Unity editor is shutting down".to_string()),
            ),
            inputs,
            now_ms,
        );
    }

    if main_thread_unresponsive(inputs) {
        let away_secs = input_heartbeat_age(inputs).unwrap_or(0) / 1_000;
        let detail = match live_locus_execution(inputs) {
            Some(execution) => live_execution_detail(&execution, away_secs),
            None => format!("Unity main thread has not returned to the editor loop for {away_secs}s"),
        };
        return decorate_state(
            SemanticState::new(SemanticPhase::Unresponsive, "native_broker", "high", None, Some(detail)),
            inputs, now_ms,
        );
    }

    // 2. Native broker lifecycle is the authoritative managed-domain state.
    //    Stack classification stays as a fallback when the broker has not
    //    published the lifecycle yet.
    if let Some(status) = &inputs.native_broker_status {
        match status.managed_state.as_str() {
            "reloading" => {
                return decorate_state(
                    SemanticState::new(
                        SemanticPhase::Reloading,
                        "native_broker",
                        "high",
                        None,
                        Some(format!(
                            "managed domain generation {} is reloading",
                            status.domain_generation
                        )),
                    ),
                    inputs,
                    now_ms,
                );
            }
            "initializing" => {
                return decorate_state(
                    SemanticState::new(
                        SemanticPhase::Starting,
                        "native_broker",
                        "medium",
                        None,
                        Some(format!(
                            "managed domain generation {} is initializing",
                            status.domain_generation
                        )),
                    ),
                    inputs,
                    now_ms,
                );
            }
            "quitting" => {
                return decorate_state(
                    SemanticState::new(
                        SemanticPhase::Quit,
                        "native_broker",
                        "high",
                        None,
                        Some("managed editor domain is quitting".to_string()),
                    ),
                    inputs,
                    now_ms,
                );
            }
            _ => {}
        }
    }

    if let Some(native) = &inputs.native {
        if let Some(reload) = native.reloading {
            return decorate_state(
                SemanticState::new(
                    SemanticPhase::Reloading,
                    "native_stack",
                    "high",
                    Some(reload),
                    Some(format!("domain reload: {}", reload.as_str())),
                ),
                inputs,
                now_ms,
            );
        }
    }

    // 3. Pipe is the authority for the interactive modes it can report.
    if inputs.pipe_connected {
        let phase = match super::normalize_editor_status(&inputs.pipe_status) {
            super::UNITY_EDITOR_STATUS_PLAYING => SemanticPhase::Playing,
            super::UNITY_EDITOR_STATUS_PLAYING_PAUSED => SemanticPhase::Paused,
            _ => SemanticPhase::Editing,
        };
        return decorate_state(
            SemanticState::new(phase, "pipe", "high", None, None),
            inputs,
            now_ms,
        );
    }

    // 4. Pipe silent but process alive — lean on native, then inference.
    if inputs.recompile_inflight {
        // We asked for the recompile; the pipe drop is the expected reload
        // window even if the native tier did not catch a frame this sample.
        return decorate_state(
            SemanticState::new(
                SemanticPhase::Reloading,
                "inference",
                "medium",
                None,
                Some("recompile in flight".to_string()),
            ),
            inputs,
            now_ms,
        );
    }

    if let Some(native) = &inputs.native {
        if !native.cpu_active && native.quiescent_for_ms >= WEDGED_AFTER_MS {
            // No CPU progress, not reloading, pipe unreachable, process alive →
            // wedged. The stack tier corroborates with the IP inside the engine
            // module (raising confidence over the passive CPU-only read).
            let confidence = if native.rip_in_unity {
                "high"
            } else {
                "medium"
            };
            let source = if native.suspended {
                "native_stack"
            } else {
                "native_cpu"
            };
            return decorate_state(
                SemanticState::new(
                    SemanticPhase::Unresponsive,
                    source,
                    confidence,
                    None,
                    Some(format!(
                        "main thread no CPU progress for {}ms, channel down",
                        native.quiescent_for_ms
                    )),
                ),
                inputs,
                now_ms,
            );
        }

        if !inputs.pipe_connected && editor_mode.value != "unknown" {
            let phase = phase_from_editor_mode(&editor_mode.value);
            return decorate_state(
                SemanticState::new(
                    phase,
                    &editor_mode.source,
                    &editor_mode.confidence,
                    None,
                    Some(
                        editor_mode
                            .detail
                            .clone()
                            .unwrap_or_else(|| "control channel is not ready".to_string()),
                    ),
                ),
                inputs,
                now_ms,
            );
        }

        if inputs.recently_launched {
            return decorate_state(
                SemanticState::new(SemanticPhase::Starting, "native_cpu", "medium", None, None),
                inputs,
                now_ms,
            );
        }
        if native.cpu_active {
            // Busy with the pipe down and the process alive — in a dev loop this
            // is almost always a domain reload (or a long import); both are
            // "transient, wait and retry". Low confidence without a stack frame
            // proving the reload sub-phase.
            return decorate_state(
                SemanticState::new(
                    SemanticPhase::Reloading,
                    "native_cpu",
                    "low",
                    None,
                    Some("main thread busy, channel down (reload or import)".to_string()),
                ),
                inputs,
                now_ms,
            );
        }
        // Quiescent but not long enough to call wedged.
        return decorate_state(
            SemanticState::new(
                SemanticPhase::Starting,
                "native_cpu",
                "low",
                None,
                Some("alive, channel not ready".to_string()),
            ),
            inputs,
            now_ms,
        );
    }

    if !inputs.pipe_connected && editor_mode.value != "unknown" {
        let phase = phase_from_editor_mode(&editor_mode.value);
        return decorate_state(
            SemanticState::new(
                phase,
                &editor_mode.source,
                &editor_mode.confidence,
                None,
                editor_mode.detail.clone(),
            ),
            inputs,
            now_ms,
        );
    }

    // 5. Degraded: no native tier, pipe silent.
    if inputs.recently_launched {
        return decorate_state(
            SemanticState::new(SemanticPhase::Starting, "inference", "low", None, None),
            inputs,
            now_ms,
        );
    }
    decorate_state(
        SemanticState::new(SemanticPhase::Unknown, "inference", "low", None, None),
        inputs,
        now_ms,
    )
}

/// Reflect a freshly fused state into the settings status surface. The native
/// tier (Stack / CpuOnly) is set authoritatively inside `sample`; here we only
/// record the latest phase and, when no native tier ever engaged, mark the
/// degraded inference tier.
pub fn note_fused(project_path: &str, state: &SemanticState, process_id: Option<u32>) {
    record_project_status(project_path, |status| {
        status.last_phase = Some(state.phase.clone());
        if process_id.is_some() {
            status.process_id = process_id;
        }
        if status.enabled
            && matches!(status.tier, UnityStateProbeTier::Inactive)
            && state.source == "inference"
        {
            status.tier = UnityStateProbeTier::Inference;
        }
    });
}

async fn query_process_bounded(project_path: &str) -> super::UnityEditorProcessInfo {
    match tokio::time::timeout(
        PROCESS_PROBE_TIMEOUT,
        super::query_current_project_editor_process(project_path),
    )
    .await
    {
        Ok(process) => process,
        Err(_) => super::UnityEditorProcessInfo {
            state: super::UnityEditorProcessState::Unknown,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms: unix_now_ms(),
            last_error: Some("Unity process probe timed out".to_string()),
        },
    }
}

async fn native_sample_with_timeout(
    project_path: &str,
    process_id: u32,
    module_hint: String,
    allow_suspend: bool,
    timeout: std::time::Duration,
) -> Option<NativeSample> {
    let status_project_path = project_path.to_string();
    let sample_project_path = status_project_path.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        sample_blocking(
            &sample_project_path,
            process_id,
            &module_hint,
            allow_suspend,
        )
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(Ok(sample))) => sample,
        Ok(Ok(Err(error))) => {
            record_project_status(&status_project_path, |s| s.error = Some(error));
            None
        }
        Ok(Err(error)) => {
            record_project_status(&status_project_path, |s| {
                s.error = Some(format!("native sample task failed: {error}"));
            });
            None
        }
        Err(_) => {
            record_project_status(&status_project_path, |s| {
                s.error = Some(format!(
                    "native {} sample timed out after {}ms",
                    if allow_suspend { "stack" } else { "passive" },
                    timeout.as_millis()
                ));
            });
            None
        }
    }
}

/// Main-thread stack candidates read from outside the Editor process.
#[derive(Debug, Clone)]
pub(crate) struct ExternalStackScan {
    /// Per sample: the instruction pointer, then call-site-validated return
    /// addresses in stack order. Heuristic: may contain stale frames.
    pub samples: Vec<Vec<u64>>,
    pub cpu_start_100ns: Option<u64>,
    pub cpu_end_100ns: Option<u64>,
    pub wall_ms: u64,
}

/// Hang-diagnosis fallback: scan the main thread's stack from outside the
/// process. Blocking; call from a blocking task.
pub(crate) fn scan_main_thread_stack(
    pid: u32,
    thread_hint: Option<u32>,
) -> Result<ExternalStackScan, String> {
    imp::scan_main_thread(pid, thread_hint, 3, std::time::Duration::from_millis(250), 48)
}

/// `bytes` are the 8 bytes preceding a candidate return address. True when
/// they end with a `call rel32` or an `FF /2` indirect call whose encoded
/// length lands exactly on the candidate.
fn looks_like_call_site(bytes: &[u8; 8]) -> bool {
    if bytes[3] == 0xE8 {
        return true;
    }
    for length in 2..=7usize {
        let start = 8 - length;
        if bytes[start] != 0xFF {
            continue;
        }
        let modrm = bytes[start + 1];
        if (modrm >> 3) & 7 != 2 {
            continue;
        }
        let rm = modrm & 7;
        let sib = bytes.get(start + 2).copied();
        let operand = match modrm >> 6 {
            3 => Some(1),
            0 if rm == 5 => Some(5),
            0 if rm == 4 => sib.map(|sib| if sib & 7 == 5 { 6 } else { 2 }),
            0 => Some(1),
            1 => Some(if rm == 4 { 3 } else { 2 }),
            _ => Some(if rm == 4 { 6 } else { 5 }),
        };
        if operand.is_some_and(|operand| 1 + operand == length) {
            return true;
        }
    }
    false
}

struct PassiveIncident {
    pid: u32,
    created: Option<u64>,
    samples: u8,
}

/// At most three passive CPU samples per stalled interval. Healthy Editors
/// need no thread inspection; automatic observers never suspend a thread.
fn should_sample_stalled_thread(project: &str, pid: u32, created: Option<u64>, age: Option<u64>) -> bool {
    static INCIDENTS: OnceLock<Mutex<HashMap<String, PassiveIncident>>> = OnceLock::new();
    let Ok(mut incidents) = INCIDENTS.get_or_init(|| Mutex::new(HashMap::new())).lock() else { return false; };
    let key = project_key(project);
    if !age.is_some_and(|age| age >= MAIN_THREAD_UNRESPONSIVE_MS) {
        incidents.remove(&key);
        return false;
    }
    let incident = incidents.entry(key).or_insert(PassiveIncident { pid, created, samples: 0 });
    if incident.pid != pid || incident.created != created {
        *incident = PassiveIncident { pid, created, samples: 0 };
    }
    if incident.samples >= 3 { return false; }
    incident.samples += 1;
    true
}

/// Gather the cheap signals for a project and fuse them into one semantic
/// state. This is the observer actor's single-sample path; public callers go
/// through `semantic_state_for_project` so they can use the observer cache.
async fn observe_project_once(
    project_path: &str,
    observer: Option<ObserverObservation>,
) -> SemanticState {
    observe_project_once_with_native_broker_status(project_path, observer, None).await
}

async fn observe_project_once_with_native_broker_status(
    project_path: &str,
    observer: Option<ObserverObservation>,
    native_broker_status_override: Option<super::NativeBrokerStatus>,
) -> SemanticState {
    // No / non-Unity workspace: answer immediately, never touch the pipe.
    if project_path.trim().is_empty() {
        return SemanticState::new(
            SemanticPhase::Unknown,
            "inference",
            "low",
            None,
            Some("no workspace".to_string()),
        );
    }

    // Native shared memory is the normal observation path. It never queues
    // work on the Editor; only legacy/unavailable brokers need a pipe probe.
    let mut native_broker_status = match native_broker_status_override {
        Some(status) => Some(status),
        None => super::query_native_broker_observation(project_path, NATIVE_BROKER_STATE_PROBE_CONSUMER)
            .await.map(native_broker_status_for_semantic),
    };
    let (pipe_probe, process) = tokio::join!(
        async {
            if native_broker_status.is_some() { Ok(None) } else {
                super::try_query_unity_status_response_with_timeout(project_path, std::time::Duration::from_millis(800)).await
            }
        },
        query_process_bounded(project_path),
    );

    let process_state = process.state.clone();
    let process_alive = matches!(process_state, super::UnityEditorProcessState::Running);
    let process_id = process.process_id;
    let module_hint = process.executable_path.clone().unwrap_or_default();
    let process_created_at_ms = process_id.and_then(super::process::process_created_at_unix_ms);
    super::dialog::sync_project_process(project_path, process_id, process_created_at_ms).await;

    let mut pipe_connected = false;
    let mut pipe_status = super::UNITY_EDITOR_STATUS_DISCONNECTED.to_string();
    let mut pipe_latency_ms = None;
    let mut pipe_error = None;
    let mut control_channel_state: String;

    match pipe_probe {
        Ok(Some((resp, latency_ms))) if resp.ok => {
            pipe_latency_ms = Some(latency_ms);
            control_channel_state = "ready".to_string();
            pipe_connected = true;
            let message = resp.message.unwrap_or_default();
            let (status, _, _) = super::parse_unity_status_message(&message);
            pipe_status = status.to_string();
            note_pipe_editor_status(
                project_path,
                status,
                resp.process_id.or(process_id),
                unix_now_ms(),
            );
        }
        Ok(Some((resp, latency_ms))) => {
            pipe_latency_ms = Some(latency_ms);
            control_channel_state = "error".to_string();
            pipe_error = Some(
                resp.error
                    .unwrap_or_else(|| "Unity status returned ok=false".to_string()),
            );
        }
        Ok(None) => {
            // Reaching the writer try-lock requires a successfully resolved
            // pipe connection. Preserve liveness while recording that the
            // control channel is momentarily occupied.
            pipe_connected = true;
            control_channel_state = "busy".to_string();
            pipe_error =
                Some("Unity status poll skipped because the pipe writer is busy".to_string());
        }
        Err(error) => {
            control_channel_state = if error.contains("timed out") {
                "timeout".to_string()
            } else {
                "disconnected".to_string()
            };
            pipe_error = Some(error);
        }
    }

    if native_broker_status.as_ref().is_some_and(|status|
        status.process_id.is_some() && status.process_id != process_id) {
        native_broker_status = None;
        pipe_connected = false;
        control_channel_state = "disconnected".to_string();
    }
    if let (Some(status), Some(pid)) = (&native_broker_status, process_id) {
        if status.managed_state == "quitting" && process_alive {
            note_editor_quitting(project_path, pid, process_created_at_ms);
        }
    }
    let editor_quitting =
        editor_quitting(project_path, &process_state, process_id, process_created_at_ms);
    if let Some(status) = &native_broker_status {
        match status.managed_state.as_str() {
            "ready" => {
                pipe_connected = status.native_alive;
                control_channel_state = "ready".to_string();
                pipe_status = super::normalize_editor_status(&status.editor_status).to_string();
                pipe_error = None;
                if broker_heartbeat_age(status, unix_now_ms()).is_some_and(|age| age < MAIN_THREAD_DELAYED_MS)
                    && !status.editor_status.trim().is_empty() {
                    note_pipe_editor_status(
                        project_path,
                        super::normalize_editor_status(&status.editor_status),
                        process_id,
                        unix_now_ms(),
                    );
                }
            }
            "reloading" => {
                pipe_connected = false;
                control_channel_state = "reloading".to_string();
                pipe_error = Some(format!(
                    "Native broker reports managed domain generation {} is reloading",
                    status.domain_generation
                ));
            }
            "initializing" => {
                pipe_connected = false;
                control_channel_state = "starting".to_string();
                pipe_error = Some(format!(
                    "Native broker reports managed domain generation {} is initializing",
                    status.domain_generation
                ));
            }
            "quitting" => {
                pipe_connected = false;
                control_channel_state = "quitting".to_string();
                pipe_error = Some("Native broker reports managed domain is quitting".to_string());
            }
            _ => {}
        }
    }

    let recompile_inflight = super::unity_recompile_waiting(project_path);
    let editor_instance_present =
        std::path::Path::new(super::strip_extended_path_prefix(project_path))
            .join("Library")
            .join("EditorInstance.json")
            .is_file();

    // Live "recently launched" from the process creation time (replaces the
    // dead `false`): a just-spawned editor whose bridge is not up yet reads as
    // `starting` rather than `unknown`.
    let recently_launched = process_created_at_ms
        .map(|created| unix_now_ms().saturating_sub(created) < 30_000)
        .unwrap_or(false);

    let heartbeat_age = native_broker_status.as_ref().and_then(|status| broker_heartbeat_age(status, unix_now_ms()));
    let native = if let Some(pid) = process_id.filter(|_| enabled()) {
        if should_sample_stalled_thread(project_path, pid, process_created_at_ms, heartbeat_age) {
            native_sample_with_timeout(project_path, pid, module_hint.clone(), false, PASSIVE_SAMPLE_TIMEOUT).await
        } else { None }
    } else { None };
    let native_hook = native_hook_observation_for_process(
        project_path,
        process_id,
        process.executable_path.clone(),
    )
    .await;

    let inputs = FusionInputs {
        pipe_connected,
        pipe_status,
        process_state,
        editor_instance_present,
        recompile_inflight,
        recently_launched,
        native,
        native_broker_status,
        pipe_latency_ms,
        pipe_error,
        control_channel_state,
        process_id,
        process_created_at_ms,
        process_path: process.executable_path.clone(),
        last_known_editor_mode: lookup_last_known_editor_mode(
            project_path,
            process_id,
            process_created_at_ms,
            unix_now_ms(),
        ),
        pending_editor_intent: lookup_pending_editor_intent(project_path, unix_now_ms()),
        native_hook,
        observer,
        editor_quitting,
    };
    let mut state = fuse(&inputs);
    let allow_log_safe_mode_fallback = process_alive
        && !pipe_connected
        && !inputs
            .native_broker_status
            .as_ref()
            .is_some_and(|status| status.managed_state == "ready");
    state.editor_log = super::editor_log::observe(
        project_path,
        process_id,
        process_created_at_ms,
        allow_log_safe_mode_fallback,
    );
    if process_alive && state.editor_log.safe_mode {
        state.phase = SemanticPhase::SafeMode.as_str().to_string();
        state.reload_phase = None;
        state.source = state
            .editor_log
            .safe_mode_source
            .clone()
            .unwrap_or_else(|| "editor_log".to_string());
        state.confidence = if state.source == "window_title" {
            "high"
        } else {
            "medium"
        }
        .to_string();
        state.transient = false;
        state.detail = Some(
            "Unity Safe Mode is active; fix script compilation errors with file tools before using Unity APIs"
                .to_string(),
        );
        state.domain.phase = "compilation_failed".to_string();
        state.domain.reload_sub_phase = None;
        state.domain.source = state.source.clone();
        state.domain.confidence = state.confidence.clone();
        state.editor_mode.value = "unknown".to_string();
        state.editor_mode.source = state.source.clone();
        state.editor_mode.confidence = state.confidence.clone();
        state.editor_mode.detail = Some("project/package managed code is disabled".to_string());
        state.safety.can_call_unity_api = false;
        state.safety.can_modify_assets_safely = false;
        state.safety.recommended_action = "fix_compile_errors".to_string();
    }
    super::hang_diagnosis::note_semantic_state(
        project_path,
        &mut state,
        inputs.native_broker_status.as_ref(),
    );
    note_fused(project_path, &state, process_id);
    state
}

/// Public state read for commands, UI, and self-test. The observation actor is
/// the primary plane; this call falls back to one immediate sample only when
/// the cache is cold.
pub async fn semantic_state_for_project(project_path: &str) -> SemanticState {
    if project_path.trim().is_empty() {
        return observe_project_once(project_path, None).await;
    }

    ensure_observer(project_path);
    let now_ms = unix_now_ms();
    if let Some(state) = cached_observer_state(project_path, now_ms) {
        match cached_native_broker_decision(project_path, &state).await {
            CachedNativeBrokerDecision::UseCached => return state,
            CachedNativeBrokerDecision::Refresh(status) => {
                let state = observe_project_once_with_native_broker_status(
                    project_path,
                    observer_observation(project_path),
                    status,
                )
                .await;
                return if enabled() {
                    store_observer_state(project_path, state)
                } else {
                    state
                };
            }
        }
    }

    let state = observe_project_once(project_path, observer_observation(project_path)).await;
    if enabled() {
        store_observer_state(project_path, state)
    } else {
        state
    }
}

enum CachedNativeBrokerDecision {
    UseCached,
    Refresh(Option<super::NativeBrokerStatus>),
}

async fn cached_native_broker_decision(
    project_path: &str,
    cached: &SemanticState,
) -> CachedNativeBrokerDecision {
    if !super::native_bridge_enabled() {
        return CachedNativeBrokerDecision::UseCached;
    }

    match super::query_native_broker_observation(project_path, NATIVE_BROKER_STATE_PROBE_CONSUMER)
        .await
        .map(native_broker_status_for_semantic)
    {
        Some(status) if native_broker_status_is_lifecycle(&status) => {
            CachedNativeBrokerDecision::Refresh(Some(status))
        }
        Some(status) if semantic_state_is_native_broker_lifecycle(cached) => {
            CachedNativeBrokerDecision::Refresh(Some(status))
        }
        Some(_) => CachedNativeBrokerDecision::UseCached,
        None if semantic_state_is_native_broker_lifecycle(cached) => {
            CachedNativeBrokerDecision::UseCached
        }
        None => CachedNativeBrokerDecision::Refresh(None),
    }
}

fn native_broker_status_for_semantic(
    observation: super::NativeBrokerObservation,
) -> super::NativeBrokerStatus {
    let mut status = observation.current;
    if let Some(event) = observation
        .events
        .iter()
        .rev()
        .find(|event| native_broker_event_is_lifecycle(event))
    {
        status.managed_state = event.to.clone();
        if event.domain_generation > 0 {
            status.domain_generation = event.domain_generation;
        }
        if !event.editor_status.trim().is_empty() {
            status.editor_status = event.editor_status.clone();
        }
    }
    status
}

fn native_broker_event_is_lifecycle(event: &super::NativeBrokerEvent) -> bool {
    event.kind == "managed_state_changed"
        && matches!(event.to.as_str(), "reloading" | "initializing" | "quitting")
}

fn native_broker_status_is_lifecycle(status: &super::NativeBrokerStatus) -> bool {
    match status.managed_state.as_str() {
        "reloading" | "initializing" | "quitting" => true,
        _ => false,
    }
}

fn semantic_state_is_native_broker_lifecycle(state: &SemanticState) -> bool {
    state.source == "native_broker"
        && matches!(state.phase.as_str(), "reloading" | "starting" | "quit")
}

// ── Windows implementation ───────────────────────────────────────────

#[cfg(target_os = "windows")]
mod imp {
    use std::collections::HashMap;
    use std::ffi::{c_void, CString, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use std::sync::Mutex;
    use std::time::Instant;

    use super::{
        record_project_status, NativeSample, ReloadPhase, UnityStateProbeTier, STACK_SCAN_BYTES,
    };

    type Bool = i32;
    type Dword = u32;
    type Dword64 = u64;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

    const TH32CS_SNAPTHREAD: Dword = 0x0000_0004;

    const THREAD_SUSPEND_RESUME: Dword = 0x0002;
    const THREAD_GET_CONTEXT: Dword = 0x0008;
    const THREAD_QUERY_INFORMATION: Dword = 0x0040;
    const PROCESS_VM_READ: Dword = 0x0010;
    const PROCESS_QUERY_INFORMATION: Dword = 0x0400;

    const CONTEXT_AMD64: Dword = 0x0010_0000;
    const CONTEXT_CONTROL: Dword = CONTEXT_AMD64 | 0x1;

    const SYMOPT_UNDNAME: Dword = 0x0000_0002;
    const SYMOPT_DEFERRED_LOADS: Dword = 0x0000_0004;
    const SYMOPT_FAIL_CRITICAL_ERRORS: Dword = 0x0000_0200;
    const SYMOPT_AUTO_PUBLICS: Dword = 0x0001_0000;
    const SYMOPT_NO_PROMPTS: Dword = 0x0008_0000;
    const MAX_SYM_NAME: usize = 2048;

    /// (symbol name, sub-phase). Verified present in 2022.3 and Unity 6.
    const RELOAD_SYMBOLS: &[(&str, ReloadPhase)] = &[
        (
            "MonoManagerProfiling::MonoUnityDomainUnload",
            ReloadPhase::Unloading,
        ),
        ("MonoManagerProfiling::UnloadDomain", ReloadPhase::Unloading),
        (
            "MonoManager::CreateAndSetChildDomain",
            ReloadPhase::CreatingDomain,
        ),
        (
            "MonoManager::LoadAllAssembliesAndSetupDomain",
            ReloadPhase::LoadingAssemblies,
        ),
        (
            "MonoManager::LoadAssemblies",
            ReloadPhase::LoadingAssemblies,
        ),
        (
            "MonoManager::SetupLoadedEditorAssemblies",
            ReloadPhase::LoadingAssemblies,
        ),
        ("MonoManager::FinalizeReload", ReloadPhase::Finalizing),
        (
            "MonoManager::RebuildCommonMonoClasses",
            ReloadPhase::Finalizing,
        ),
        ("MonoManager::BeginReloadAssembly", ReloadPhase::Reloading),
        ("MonoManager::ReloadAssembly", ReloadPhase::Reloading),
    ];

    // x64 CONTEXT. Only the control fields are requested; offsets are checked
    // at compile time so a layout slip fails the build instead of corrupting.
    #[allow(dead_code)]
    #[repr(C, align(16))]
    struct Context {
        p1_home: u64,
        p2_home: u64,
        p3_home: u64,
        p4_home: u64,
        p5_home: u64,
        p6_home: u64,
        context_flags: u32,
        mx_csr: u32,
        seg_cs: u16,
        seg_ds: u16,
        seg_es: u16,
        seg_fs: u16,
        seg_gs: u16,
        seg_ss: u16,
        eflags: u32,
        dr0: u64,
        dr1: u64,
        dr2: u64,
        dr3: u64,
        dr6: u64,
        dr7: u64,
        rax: u64,
        rcx: u64,
        rdx: u64,
        rbx: u64,
        rsp: u64,
        rbp: u64,
        rsi: u64,
        rdi: u64,
        r8: u64,
        r9: u64,
        r10: u64,
        r11: u64,
        r12: u64,
        r13: u64,
        r14: u64,
        r15: u64,
        rip: u64,
        flt_save: [u8; 512],
        vector_register: [u8; 16 * 26],
        vector_control: u64,
        debug_control: u64,
        last_branch_to_rip: u64,
        last_branch_from_rip: u64,
        last_exception_to_rip: u64,
        last_exception_from_rip: u64,
    }

    const _: () = {
        assert!(core::mem::offset_of!(Context, context_flags) == 0x30);
        assert!(core::mem::offset_of!(Context, rsp) == 0x98);
        assert!(core::mem::offset_of!(Context, rbp) == 0xA0);
        assert!(core::mem::offset_of!(Context, rip) == 0xF8);
        assert!(size_of::<Context>() == 1232);
    };

    #[allow(non_snake_case)]
    #[repr(C)]
    struct ThreadEntry32 {
        dwSize: Dword,
        cntUsage: Dword,
        th32ThreadID: Dword,
        th32OwnerProcessID: Dword,
        tpBasePri: i32,
        tpDeltaPri: i32,
        dwFlags: Dword,
    }

    #[repr(C)]
    struct Filetime {
        low: Dword,
        high: Dword,
    }

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

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfoA {
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
        Name: [u8; 1],
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfoBufferA {
        Symbol: SymbolInfoA,
        NameBuffer: [u8; MAX_SYM_NAME],
    }

    type EnumCb = extern "system" fn(*const SymbolInfoW, Dword, *mut c_void) -> Bool;

    unsafe extern "system" {
        fn CloseHandle(h: Handle) -> Bool;
        fn CreateToolhelp32Snapshot(flags: Dword, pid: Dword) -> Handle;
        fn Thread32First(snap: Handle, te: *mut ThreadEntry32) -> Bool;
        fn Thread32Next(snap: Handle, te: *mut ThreadEntry32) -> Bool;
        fn OpenThread(access: Dword, inherit: Bool, tid: Dword) -> Handle;
        fn OpenProcess(access: Dword, inherit: Bool, pid: Dword) -> Handle;
        fn SuspendThread(h: Handle) -> Dword;
        fn ResumeThread(h: Handle) -> Dword;
        fn GetThreadContext(h: Handle, ctx: *mut Context) -> Bool;
        fn GetThreadTimes(
            h: Handle,
            creation: *mut Filetime,
            exit: *mut Filetime,
            kernel: *mut Filetime,
            user: *mut Filetime,
        ) -> Bool;
        fn ReadProcessMemory(
            h: Handle,
            base: *const c_void,
            buf: *mut c_void,
            size: usize,
            read: *mut usize,
        ) -> Bool;
        fn VirtualQueryEx(
            h: Handle,
            address: *const c_void,
            buffer: *mut MemoryBasicInformation,
            length: usize,
        ) -> usize;
    }

    #[repr(C)]
    struct MemoryBasicInformation {
        base_address: *mut c_void,
        allocation_base: *mut c_void,
        allocation_protect: u32,
        partition_id: u16,
        region_size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }

    const _: () = assert!(size_of::<MemoryBasicInformation>() == 48);

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
        fn SymFromName(h: Handle, name: *const u8, sym: *mut SymbolInfoA) -> Bool;
        fn SymEnumSymbolsW(
            h: Handle,
            base: Dword64,
            mask: *const u16,
            cb: EnumCb,
            ctx: *mut c_void,
        ) -> Bool;
        fn SymCleanup(h: Handle) -> Bool;
    }

    struct OwnedHandle(Handle);
    impl OwnedHandle {
        fn new(h: Handle) -> Option<Self> {
            if h.is_null() || h == INVALID_HANDLE_VALUE {
                None
            } else {
                Some(Self(h))
            }
        }
    }
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Resumes the suspended thread on drop — even on early return / panic —
    /// so a probe failure can never leave the editor frozen.
    struct SuspendGuard(Handle);
    impl Drop for SuspendGuard {
        fn drop(&mut self) {
            unsafe {
                ResumeThread(self.0);
            }
        }
    }

    /// Per-process resolved symbol table. Built once, then the hot path is a
    /// binary search — no dbghelp, no persistent symbol session.
    #[derive(Clone)]
    struct ModuleSymbols {
        module_base: u64,
        module_end: u64,
        /// All symbol addresses, sorted — predecessor lookup yields the
        /// containing function.
        sorted_addrs: Vec<u64>,
        /// Reload-function start addresses → sub-phase.
        reload_targets: HashMap<u64, ReloadPhase>,
    }

    struct ThreadHistory {
        tid: u32,
        last_cpu_100ns: u64,
        /// When the main-thread CPU total last stopped advancing — drives the
        /// quiescence duration without needing a suspended IP read.
        cpu_flat_since: Instant,
    }

    #[derive(Default)]
    struct Cache {
        symbols: HashMap<u32, Option<ModuleSymbols>>,
        history: HashMap<u32, ThreadHistory>,
        /// Process creation time per PID — a change means the PID was reused by
        /// a new editor instance, so the symbol table (stale module base) and
        /// the cached main-thread id must be dropped.
        created: HashMap<u32, u64>,
    }

    fn cache() -> &'static Mutex<Cache> {
        static C: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
        C.get_or_init(|| Mutex::new(Cache::default()))
    }

    /// dbghelp is not thread-safe; serialize the (rare, init-only) symbol work
    /// with every other dbghelp user in the process.
    fn dbghelp_lock() -> &'static Mutex<()> {
        crate::unity_bridge::hang_diagnosis::dbghelp_lock()
    }

    pub(super) fn clear_cache() {
        if let Ok(mut c) = cache().lock() {
            c.symbols.clear();
            c.history.clear();
            c.created.clear();
        }
    }

    fn wide_nul(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    extern "system" fn collect_addr(
        info: *const SymbolInfoW,
        _size: Dword,
        ctx: *mut c_void,
    ) -> Bool {
        unsafe {
            let vec = &mut *(ctx as *mut Vec<u64>);
            if vec.len() < 4_000_000 {
                vec.push((*info).Address);
            }
            1
        }
    }

    fn build_symbols(pid: u32, module_path_hint: &str) -> Result<ModuleSymbols, String> {
        // Reuse the background hook's Toolhelp module finder (Unity.dll first,
        // then Unity.exe) instead of re-declaring Module32* bindings here.
        let (base, end, module_path) =
            crate::unity_bridge::background_hook::engine_module_bounds(pid)?;
        let module_path = if module_path.trim().is_empty() {
            module_path_hint.to_string()
        } else {
            module_path
        };
        let symbol_dir = Path::new(&module_path)
            .parent()
            .ok_or("engine module has no parent dir")?;
        // Unity ships `unity_x64.pdb`; Tuanjie ships `Tuanjie_x64.pdb` next to its
        // renamed engine module. Derive the name from the resolved module so Unity
        // keeps resolving exactly `unity_x64.pdb`.
        let pdb_name = Path::new(&module_path)
            .file_name()
            .and_then(|value| value.to_str())
            .map(crate::unity_bridge::flavor::engine_pdb_name_for_module)
            .unwrap_or("unity_x64.pdb");
        let pdb = symbol_dir.join(pdb_name);
        if !pdb.is_file() {
            return Err(format!("{pdb_name} missing: {}", pdb.display()));
        }

        let _guard = dbghelp_lock().lock().map_err(|_| "dbghelp lock poisoned")?;
        // A private pseudo-handle keeps this session distinct from the
        // background hook's (-1) session.
        let sym_handle = 0x5052_4F42usize as Handle; // 'PROB'
        let search = wide_nul(symbol_dir.as_os_str());
        unsafe {
            SymSetOptions(
                SYMOPT_UNDNAME
                    | SYMOPT_DEFERRED_LOADS
                    | SYMOPT_FAIL_CRITICAL_ERRORS
                    | SYMOPT_AUTO_PUBLICS
                    | SYMOPT_NO_PROMPTS,
            );
            if SymInitializeW(sym_handle, search.as_ptr(), FALSE) == 0 {
                return Err(format!("SymInitializeW failed: {}", last_error()));
            }
        }
        struct SymSession(Handle);
        impl Drop for SymSession {
            fn drop(&mut self) {
                unsafe {
                    SymCleanup(self.0);
                }
            }
        }
        let _session = SymSession(sym_handle);

        let image = wide_nul(Path::new(&module_path).as_os_str());
        let module_name = wide_nul(OsStr::new("Unity"));
        let loaded = unsafe {
            SymLoadModuleExW(
                sym_handle,
                null_mut(),
                image.as_ptr(),
                module_name.as_ptr(),
                base,
                (end - base) as u32,
                null_mut(),
                0,
            )
        };
        if loaded == 0 {
            return Err(format!("SymLoadModuleExW failed: {}", last_error()));
        }

        // Resolve the reload functions by name (a handful, unique symbols).
        let mut reload_targets = HashMap::new();
        for (name, phase) in RELOAD_SYMBOLS {
            if let Some(addr) = resolve_by_name(sym_handle, name) {
                reload_targets.insert(addr, *phase);
            }
        }
        if reload_targets.is_empty() {
            return Err("no MonoManager reload symbols resolved".to_string());
        }

        // Enumerate the `MonoManager*` compiland only (both MonoManager:: and
        // MonoManagerProfiling:: — a few hundred symbols, sub-second) to build
        // a DENSE predecessor table over exactly the region the reload targets
        // live in. Avoids the multi-second full-module ("*") enumeration that
        // would otherwise run on the first stack sample. `containing_reload`
        // adds a span backstop for any foreign function in the region.
        let mut sorted_addrs: Vec<u64> = Vec::with_capacity(1024);
        let mask = wide_nul(OsStr::new("MonoManager*"));
        unsafe {
            SymEnumSymbolsW(
                sym_handle,
                base,
                mask.as_ptr(),
                collect_addr,
                &mut sorted_addrs as *mut Vec<u64> as *mut c_void,
            );
        }
        sorted_addrs.retain(|a| *a >= base && *a < end);
        sorted_addrs.sort_unstable();
        sorted_addrs.dedup();
        if sorted_addrs.is_empty() {
            return Err("symbol enumeration produced no addresses".to_string());
        }

        let _ = module_path;
        Ok(ModuleSymbols {
            module_base: base,
            module_end: end,
            sorted_addrs,
            reload_targets,
        })
    }

    fn resolve_by_name(sym_handle: Handle, name: &str) -> Option<u64> {
        let mut storage: Box<SymbolInfoBufferA> = unsafe { Box::new(zeroed()) };
        storage.Symbol.SizeOfStruct = size_of::<SymbolInfoA>() as u32;
        storage.Symbol.MaxNameLen = MAX_SYM_NAME as u32;
        let cname = CString::new(name).ok()?;
        let ok = unsafe {
            SymFromName(
                sym_handle,
                cname.as_ptr() as *const u8,
                &mut storage.Symbol as *mut SymbolInfoA,
            ) != 0
        };
        if ok {
            Some(storage.Symbol.Address)
        } else {
            None
        }
    }

    /// Greatest symbol address ≤ `addr`; returns the reload sub-phase only when
    /// that containing symbol is a reload target AND `addr` is within a bounded
    /// span of its start (backstop against a foreign function placed after a
    /// target in the densely-enumerated MonoManager region).
    fn containing_reload(symbols: &ModuleSymbols, addr: u64) -> Option<ReloadPhase> {
        const MAX_RELOAD_FN_BYTES: u64 = 256 * 1024;
        if addr < symbols.module_base || addr >= symbols.module_end {
            return None;
        }
        let idx = match symbols.sorted_addrs.binary_search(&addr) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let start = symbols.sorted_addrs[idx];
        let phase = symbols.reload_targets.get(&start).copied()?;
        (addr - start < MAX_RELOAD_FN_BYTES).then_some(phase)
    }

    /// Pick the editor main thread: the earliest-created thread of the
    /// process (Unity creates it first). Cached per process.
    fn find_main_thread(pid: u32) -> Result<u32, String> {
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        let snap = OwnedHandle::new(snap).ok_or("thread snapshot failed")?;
        let mut entry: ThreadEntry32 = unsafe { zeroed() };
        entry.dwSize = size_of::<ThreadEntry32>() as u32;
        let mut ok = unsafe { Thread32First(snap.0, &mut entry) != 0 };
        let mut best: Option<(u32, u64)> = None;
        while ok {
            if entry.th32OwnerProcessID == pid {
                let tid = entry.th32ThreadID;
                if let Some(created) = thread_creation_100ns(tid) {
                    if best.map(|(_, c)| created < c).unwrap_or(true) {
                        best = Some((tid, created));
                    }
                }
            }
            ok = unsafe { Thread32Next(snap.0, &mut entry) != 0 };
        }
        best.map(|(tid, _)| tid)
            .ok_or_else(|| format!("no threads for pid {pid}"))
    }

    fn thread_creation_100ns(tid: u32) -> Option<u64> {
        let h = OwnedHandle::new(unsafe { OpenThread(THREAD_QUERY_INFORMATION, FALSE, tid) })?;
        let (mut c, mut e, mut k, mut u): (Filetime, Filetime, Filetime, Filetime) =
            unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
        let ok = unsafe { GetThreadTimes(h.0, &mut c, &mut e, &mut k, &mut u) != 0 };
        if ok {
            Some(((c.high as u64) << 32) | c.low as u64)
        } else {
            None
        }
    }

    fn thread_cpu_100ns(h: Handle) -> Option<u64> {
        let (mut c, mut e, mut k, mut u): (Filetime, Filetime, Filetime, Filetime) =
            unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
        let ok = unsafe { GetThreadTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 };
        if ok {
            let kernel = ((k.high as u64) << 32) | k.low as u64;
            let user = ((u.high as u64) << 32) | u.low as u64;
            Some(kernel + user)
        } else {
            None
        }
    }

    fn last_error() -> String {
        std::io::Error::last_os_error().to_string()
    }

    /// Build + cache the symbol table for `pid` if absent. The expensive
    /// dbghelp work runs WITHOUT the cache lock held. A failure is NOT cached
    /// (it fails fast when there's no PDB, and a transient module read during a
    /// reload should be retried), so the next escalation tries again.
    pub(super) fn ensure_symbols(project_path: &str, pid: u32, module_path_hint: &str) {
        match cache().lock() {
            Ok(cache) => {
                if cache.symbols.contains_key(&pid) {
                    return;
                }
            }
            Err(_) => return,
        }
        match build_symbols(pid, module_path_hint) {
            Ok(syms) => {
                let total = syms.sorted_addrs.len() as u32;
                let reloads = syms.reload_targets.len() as u32;
                if let Ok(mut cache) = cache().lock() {
                    cache.symbols.entry(pid).or_insert(Some(syms));
                }
                record_project_status(project_path, |s| {
                    s.tier = UnityStateProbeTier::Stack;
                    s.process_id = Some(pid);
                    s.total_symbols = total;
                    s.reload_symbols = reloads;
                    s.error = None;
                });
            }
            Err(error) => {
                record_project_status(project_path, |s| {
                    if matches!(
                        s.tier,
                        UnityStateProbeTier::Inactive
                            | UnityStateProbeTier::Passive
                            | UnityStateProbeTier::Inference
                    ) {
                        s.tier = UnityStateProbeTier::CpuOnly;
                    }
                    s.process_id = Some(pid);
                    s.error = Some(error);
                });
            }
        }
    }

    /// Out-of-process fallback for hang diagnosis when the in-process broker
    /// cannot walk the stack (older plugin, broker unreachable). Each sample
    /// freezes the main thread for one context read plus one stack read; the
    /// candidate return addresses are then filtered after resuming: they must
    /// point into a loaded module and directly follow a `call` instruction.
    pub(super) fn scan_main_thread(
        pid: u32,
        thread_hint: Option<u32>,
        samples: u32,
        interval: std::time::Duration,
        max_frames: usize,
    ) -> Result<super::ExternalStackScan, String> {
        let tid = match thread_hint {
            Some(tid) if tid != 0 => tid,
            _ => find_main_thread(pid)?,
        };
        let thread = OwnedHandle::new(unsafe {
            OpenThread(
                THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                FALSE,
                tid,
            )
        })
        .ok_or_else(|| format!("OpenThread {tid} failed: {}", last_error()))?;
        let process = OwnedHandle::new(unsafe {
            OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, FALSE, pid)
        })
        .ok_or_else(|| format!("OpenProcess {pid} failed: {}", last_error()))?;
        let mut modules: Vec<(u64, u64)> =
            crate::unity_bridge::background_hook::process_modules(pid)?
                .into_iter()
                .map(|module| (module.base, module.base.saturating_add(module.size)))
                .collect();
        modules.sort_unstable();
        let in_module = |address: u64| match modules.binary_search_by(|(base, _)| base.cmp(&address)) {
            Ok(_) => true,
            Err(0) => false,
            Err(index) => address < modules[index - 1].1,
        };
        let read_remote = |address: u64, buffer: &mut [u8]| -> usize {
            let mut read = 0usize;
            let ok = unsafe {
                ReadProcessMemory(
                    process.0,
                    address as *const c_void,
                    buffer.as_mut_ptr() as *mut c_void,
                    buffer.len(),
                    &mut read,
                ) != 0
            };
            if ok { buffer.len() } else { read }
        };

        let started = Instant::now();
        let cpu_start_100ns = thread_cpu_100ns(thread.0);
        let mut buffer = vec![0u8; STACK_SCAN_BYTES];
        let mut out = Vec::with_capacity(samples as usize);
        for index in 0..samples.max(1) {
            if index > 0 {
                std::thread::sleep(interval);
            }
            if unsafe { SuspendThread(thread.0) } == u32::MAX {
                return Err(format!("SuspendThread failed: {}", last_error()));
            }
            let (rip, read) = {
                let _resume = SuspendGuard(thread.0);
                let mut ctx: Context = unsafe { zeroed() };
                ctx.context_flags = CONTEXT_CONTROL;
                if unsafe { GetThreadContext(thread.0, &mut ctx) } == 0 {
                    return Err(format!("GetThreadContext failed: {}", last_error()));
                }
                // Read only the committed stack above RSP: the stack top can
                // be much closer than the scan window on a shallow stack.
                let mut info: MemoryBasicInformation = unsafe { zeroed() };
                let queried = unsafe {
                    VirtualQueryEx(
                        process.0,
                        ctx.rsp as *const c_void,
                        &mut info,
                        size_of::<MemoryBasicInformation>(),
                    ) != 0
                };
                let region_end = (info.base_address as u64).saturating_add(info.region_size as u64);
                let len = if queried && region_end > ctx.rsp {
                    ((region_end - ctx.rsp) as usize).min(buffer.len())
                } else {
                    buffer.len()
                };
                (ctx.rip, read_remote(ctx.rsp, &mut buffer[..len]))
            };

            let mut frames = vec![rip];
            for word in buffer[..read - read % 8].chunks_exact(8) {
                if frames.len() >= max_frames {
                    break;
                }
                let value = u64::from_le_bytes(word.try_into().unwrap());
                if value < 0x10000 || !in_module(value) || frames.last() == Some(&value) {
                    continue;
                }
                let mut site = [0u8; 8];
                if read_remote(value - 8, &mut site) == 8 && super::looks_like_call_site(&site) {
                    frames.push(value);
                }
            }
            out.push(frames);
        }
        Ok(super::ExternalStackScan {
            samples: out,
            cpu_start_100ns,
            cpu_end_100ns: thread_cpu_100ns(thread.0),
            wall_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
    }

    pub(super) fn sample(
        project_path: &str,
        pid: u32,
        module_path_hint: &str,
        allow_suspend: bool,
        expected_created: Option<u64>,
    ) -> Result<Option<NativeSample>, String> {
        // PID-reuse guard FIRST: if the process creation time changed, a new
        // editor reused this PID — drop the stale symbol table (wrong module
        // base) and main-thread id before anything reads them.
        if let Some(created) = expected_created {
            if let Ok(mut cache) = cache().lock() {
                let stale = matches!(cache.created.get(&pid), Some(prev) if *prev != created);
                if stale {
                    cache.symbols.remove(&pid);
                    cache.history.remove(&pid);
                }
                cache.created.insert(pid, created);
            }
        }

        // Build the per-process symbol table WITHOUT holding the cache lock, so
        // the first (sub-second) build never blocks a concurrent sampler. Only
        // the stack tier needs symbols; the passive tier skips dbghelp.
        if allow_suspend {
            ensure_symbols(project_path, pid, module_path_hint);
        }

        // Main thread, cached in history.
        let cached_tid = {
            let cache = cache().lock().map_err(|_| "probe cache poisoned")?;
            cache.history.get(&pid).map(|h| h.tid)
        };
        let tid = match cached_tid {
            Some(tid) => tid,
            None => find_main_thread(pid)?,
        };

        // Passive tier only needs query access; the stack tier additionally
        // needs suspend + get-context rights.
        let mut access = THREAD_QUERY_INFORMATION;
        if allow_suspend {
            access |= THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT;
        }
        let thread = OwnedHandle::new(unsafe { OpenThread(access, FALSE, tid) })
            .ok_or_else(|| format!("OpenThread {tid} failed: {}", last_error()))?;

        // ── optional stack tier: suspend for ONE bulk stack read ──
        let mut reload = None;
        let mut rip_in_unity = false;
        let mut suspended = false;
        let mut suspend_window_us = 0u64;
        if allow_suspend {
            let symbols = {
                let cache = cache().lock().map_err(|_| "probe cache poisoned")?;
                cache.symbols.get(&pid).and_then(|s| s.as_ref()).cloned()
            };
            if let Some(syms) = symbols.as_ref() {
                let proc = OwnedHandle::new(unsafe {
                    OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, FALSE, pid)
                })
                .ok_or_else(|| format!("OpenProcess {pid} failed: {}", last_error()))?;

                // Allocate the read buffer BEFORE the suspend, so the frozen
                // window holds only GetThreadContext + one ReadProcessMemory.
                let mut buf = vec![0u8; STACK_SCAN_BYTES];
                let window_start = Instant::now();
                if unsafe { SuspendThread(thread.0) } == u32::MAX {
                    return Err(format!("SuspendThread failed: {}", last_error()));
                }
                let (rip, read, read_ok) = {
                    let _resume = SuspendGuard(thread.0);
                    let mut ctx: Context = unsafe { zeroed() };
                    ctx.context_flags = CONTEXT_CONTROL;
                    if unsafe { GetThreadContext(thread.0, &mut ctx) } == 0 {
                        return Err(format!("GetThreadContext failed: {}", last_error()));
                    }
                    let mut read = 0usize;
                    let read_ok = unsafe {
                        ReadProcessMemory(
                            proc.0,
                            ctx.rsp as *const c_void,
                            buf.as_mut_ptr() as *mut c_void,
                            buf.len(),
                            &mut read,
                        ) != 0
                    };
                    (ctx.rip, read, read_ok)
                    // _resume drops here → thread resumes BEFORE classification.
                };
                suspend_window_us = window_start.elapsed().as_micros() as u64;
                suspended = true;

                rip_in_unity = rip >= syms.module_base && rip < syms.module_end;
                // The current instruction first, then the nearest live return
                // address up the stack.
                let mut found = containing_reload(syms, rip);
                if found.is_none() && read_ok {
                    let words = read / 8;
                    for i in 0..words {
                        let off = i * 8;
                        let val = u64::from_le_bytes(buf[off..off + 8].try_into().unwrap());
                        if let Some(phase) = containing_reload(syms, val) {
                            found = Some(phase);
                            break;
                        }
                    }
                }
                reload = found;
            }
        }

        // ── CPU delta + quiescence history (no suspension needed) ──
        let cpu_now = thread_cpu_100ns(thread.0).unwrap_or(0);
        let now = Instant::now();
        let mut cache = cache().lock().map_err(|_| "probe cache poisoned")?;
        let (cpu_active, quiescent_for_ms) = match cache.history.get_mut(&pid) {
            Some(h) => {
                let cpu_active = cpu_now > h.last_cpu_100ns.saturating_add(5_000); // >0.5ms
                if cpu_active {
                    h.cpu_flat_since = now;
                }
                let quiescent_for_ms = if cpu_active {
                    0
                } else {
                    now.duration_since(h.cpu_flat_since).as_millis() as u64
                };
                h.last_cpu_100ns = cpu_now;
                h.tid = tid;
                (cpu_active, quiescent_for_ms)
            }
            None => {
                cache.history.insert(
                    pid,
                    ThreadHistory {
                        tid,
                        last_cpu_100ns: cpu_now,
                        cpu_flat_since: now,
                    },
                );
                (true, 0)
            }
        };

        // Reflect the passive tier in the status surface (without clobbering a
        // higher-fidelity tier a prior stack sample established).
        if !allow_suspend {
            record_project_status(project_path, |s| {
                s.process_id = Some(pid);
                if matches!(
                    s.tier,
                    UnityStateProbeTier::Inactive | UnityStateProbeTier::Inference
                ) {
                    s.tier = UnityStateProbeTier::Passive;
                }
            });
        }

        Ok(Some(NativeSample {
            reloading: reload,
            rip_in_unity,
            cpu_active,
            quiescent_for_ms,
            suspended,
            suspend_window_us,
        }))
    }
}

#[cfg(test)]
mod workspace_scope_tests {
    use super::*;
    use crate::workspace_service::event::WorkspaceEventScope;
    use crate::workspace_service::{CheckoutId, ProjectId, ServiceInstanceId};

    #[test]
    fn call_site_heuristic_accepts_only_call_encodings_ending_at_the_candidate() {
        // call rel32
        assert!(looks_like_call_site(&[0x90, 0x90, 0x90, 0xE8, 1, 2, 3, 4]));
        // call qword ptr [rip+disp32]
        assert!(looks_like_call_site(&[0x90, 0x90, 0xFF, 0x15, 1, 2, 3, 4]));
        // call rax
        assert!(looks_like_call_site(&[0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0xFF, 0xD0]));
        // call qword ptr [rax+8]
        assert!(looks_like_call_site(&[0x90, 0x90, 0x90, 0x90, 0x90, 0xFF, 0x50, 0x08]));
        // call qword ptr [rsp+0x20] (SIB + disp8)
        assert!(looks_like_call_site(&[0x90, 0x90, 0x90, 0x90, 0xFF, 0x54, 0x24, 0x20]));
        // jmp rax (FF /4) and plain data are rejected
        assert!(!looks_like_call_site(&[0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0xFF, 0xE0]));
        assert!(!looks_like_call_site(&[0, 0, 0, 0, 0, 0, 0, 0]));
    }

    fn connected_recompile_inputs(pipe_connected: bool) -> FusionInputs {
        FusionInputs {
            pipe_connected,
            pipe_status: if pipe_connected {
                "editing"
            } else {
                "disconnected"
            }
            .to_string(),
            process_state: super::super::UnityEditorProcessState::Running,
            editor_instance_present: true,
            recompile_inflight: true,
            recently_launched: false,
            native: None,
            native_broker_status: None,
            pipe_latency_ms: Some(1),
            pipe_error: None,
            control_channel_state: if pipe_connected { "ready" } else { "reloading" }.to_string(),
            process_id: Some(42),
            process_created_at_ms: Some(1),
            process_path: None,
            last_known_editor_mode: None,
            pending_editor_intent: None,
            native_hook: NativeHookObservation::default(),
            observer: None,
            editor_quitting: false,
        }
    }

    fn heartbeat_inputs(age: Option<u64>) -> FusionInputs {
        let mut inputs = connected_recompile_inputs(true);
        inputs.native_broker_status = Some(super::super::NativeBrokerStatus {
            native_alive: true, observed_at_ms: unix_now_ms() as i64,
            managed_state: "ready".to_string(), editor_status: "editing".to_string(),
            main_thread_heartbeat_age_ms: age,
            broker_capabilities: vec!["broker_main_thread_liveness_v1".to_string()],
            active_executions: 1, ..Default::default()
        });
        inputs
    }

    #[test]
    fn stopped_heartbeat_overrides_a_healthy_pipe_high_cpu_and_pending_recompile() {
        let mut inputs = heartbeat_inputs(Some(45_000));
        inputs.native = Some(NativeSample { reloading: None, rip_in_unity: true, cpu_active: true,
            quiescent_for_ms: 0, suspended: false, suspend_window_us: 0 });
        let state = fuse(&inputs);
        assert_eq!(state.phase, "unresponsive");
        assert_eq!(state.main_thread.state, "unresponsive_busy");
        assert_eq!(state.main_thread.active_executions, Some(1));
        assert!(!state.safety.can_call_unity_api);
        assert!(!state.safety.can_modify_assets_safely);
        assert_eq!(state.safety.recommended_action, "diagnose_hang");
        assert_eq!(state.editor_mode.source, "last_known");
    }

    #[test]
    fn live_locus_execution_is_busy_work_not_a_restartable_hang() {
        let execution = |published_age_ms: u64, idle_ms: u64, stop: bool| super::super::NativeBrokerExecution {
            execution_id: "exec-live".to_string(),
            published_age_ms,
            idle_ms: Some(idle_ms),
            idle_limit_ms: Some(30_000),
            main_thread_blocked_ms: Some(44_000),
            stop_requested: Some(stop),
        };
        let mut inputs = heartbeat_inputs(Some(45_000));
        inputs.native = Some(NativeSample { reloading: None, rip_in_unity: true, cpu_active: true,
            quiescent_for_ms: 0, suspended: false, suspend_window_us: 0 });
        inputs.native_broker_status.as_mut().unwrap().executions = vec![execution(250, 1_200, false)];
        let state = fuse(&inputs);
        assert_eq!(state.phase, "unresponsive");
        assert_eq!(state.main_thread.state, "unresponsive_executing");
        assert_eq!(state.safety.recommended_action, "wait_execution");
        assert!(!state.safety.can_call_unity_api);
        assert!(!state.can_restart_process(42, 1));
        let detail = state.detail.clone().unwrap_or_default();
        assert!(detail.contains("exec-live"), "{detail}");
        assert!(detail.contains("last output 1s ago"), "{detail}");

        // A silent, stopping, or no longer publishing execution is a hang again.
        for stale in [execution(250, 31_000, false), execution(250, 1_000, true), execution(9_000, 1_000, false)] {
            inputs.native_broker_status.as_mut().unwrap().executions = vec![stale];
            let state = fuse(&inputs);
            assert_eq!(state.main_thread.state, "unresponsive_busy");
            assert_eq!(state.safety.recommended_action, "diagnose_hang");
            assert!(state.can_restart_process(42, 1));
        }

        // A responsive Editor with a live execution is simply responsive.
        let mut responsive = heartbeat_inputs(Some(0));
        responsive.native_broker_status.as_mut().unwrap().executions = vec![execution(250, 0, false)];
        assert_eq!(fuse(&responsive).main_thread.state, "responsive");
    }

    #[test]
    fn fresh_heartbeat_recovers_without_cpu_or_stack_sampling() {
        let state = fuse(&heartbeat_inputs(Some(0)));
        assert_eq!(state.phase, "editing");
        assert_eq!(state.main_thread.state, "responsive");
        assert!(state.safety.can_call_unity_api);
        assert_eq!(state.main_thread.suspend_window_us, None);
    }

    #[test]
    fn announced_shutdown_stays_quit_after_the_broker_and_pipe_go_away() {
        // The broker is gone, the pipe is down, and the last known mode says
        // editing: without the quit marker this fused to "editing" until the
        // process cache noticed the exit.
        let mut inputs = connected_recompile_inputs(false);
        inputs.recompile_inflight = false;
        inputs.last_known_editor_mode = Some(LastKnownEditorMode {
            pid: Some(42), pid_created_at_ms: Some(1), mode: "editing".to_string(),
            source: "pipe".to_string(), observed_at_ms: unix_now_ms(),
        });
        assert_eq!(fuse(&inputs).phase, "editing");
        inputs.editor_quitting = true;
        let state = fuse(&inputs);
        assert_eq!(state.phase, "quit");
        assert!(!state.safety.can_call_unity_api);
        // A stopped heartbeat during shutdown is not reported as a hang.
        let mut stalled = heartbeat_inputs(Some(45_000));
        stalled.editor_quitting = true;
        assert_eq!(fuse(&stalled).phase, "quit");
    }

    #[test]
    fn quit_marker_follows_the_announcing_process_identity() {
        use super::super::UnityEditorProcessState as Process;
        let project = format!("F:/QuitMarker/{}", uuid::Uuid::new_v4());
        note_editor_quitting(&project, 42, Some(1_000));
        assert!(editor_quitting(&project, &Process::Running, Some(42), Some(1_000)));
        // An unknown probe result keeps the marker.
        assert!(!editor_quitting(&project, &Process::Unknown, None, None));
        assert!(editor_quitting(&project, &Process::Running, Some(42), Some(1_000)));
        // A replacement process clears it; it does not inherit the quit.
        assert!(!editor_quitting(&project, &Process::Running, Some(43), Some(2_000)));
        assert!(!editor_quitting(&project, &Process::Running, Some(42), Some(1_000)));
        note_editor_quitting(&project, 42, Some(1_000));
        assert!(!editor_quitting(&project, &Process::NotRunning, None, None));
        assert!(!editor_quitting(&project, &Process::Running, Some(42), Some(1_000)));
    }

    #[test]
    fn restart_requires_the_same_unresponsive_process_and_no_modal_dialog() {
        let mut state = fuse(&heartbeat_inputs(Some(45_000)));
        assert!(state.can_restart_process(42, 1));
        assert!(!state.can_restart_process(43, 1));
        assert!(!state.can_restart_process(42, 2));
        state.safety.recommended_action = "resolve_dialog".to_string();
        assert!(!state.can_restart_process(42, 1));
        assert!(!fuse(&heartbeat_inputs(Some(0))).can_restart_process(42, 1));
    }

    #[test]
    fn unavailable_or_stale_broker_evidence_does_not_authorize_unity_api() {
        let mut inputs = heartbeat_inputs(None);
        assert!(!fuse(&inputs).safety.can_call_unity_api);
        inputs.native_broker_status.as_mut().unwrap().main_thread_heartbeat_age_ms = Some(0);
        inputs.native_broker_status.as_mut().unwrap().observed_at_ms -= 60_000;
        let state = fuse(&inputs);
        assert!(!state.safety.can_call_unity_api);
        assert_eq!(state.main_thread.state, "unknown");
        assert_ne!(state.phase, "unresponsive");
    }

    #[test]
    fn explicit_reload_is_not_misreported_as_a_hang() {
        let mut inputs = heartbeat_inputs(Some(45_000));
        inputs.native_broker_status.as_mut().unwrap().managed_state = "reloading".to_string();
        let state = fuse(&inputs);
        assert_eq!(state.phase, "reloading");
        assert!(!state.safety.can_call_unity_api);
    }

    #[test]
    fn legacy_heartbeat_and_passive_sampling_budget_are_bounded() {
        let mut inputs = heartbeat_inputs(None);
        let broker = inputs.native_broker_status.as_mut().unwrap();
        broker.broker_capabilities.clear();
        broker.last_managed_heartbeat_ms = unix_now_ms() as i64 - 45_000;
        assert_eq!(fuse(&inputs).phase, "unresponsive");
        let project = "F:/liveness-budget-unit-test";
        assert!(!should_sample_stalled_thread(project, 1, Some(2), Some(0)));
        for _ in 0..3 { assert!(should_sample_stalled_thread(project, 1, Some(2), Some(45_000))); }
        assert!(!should_sample_stalled_thread(project, 1, Some(2), Some(45_000)));
        assert!(!should_sample_stalled_thread(project, 1, Some(2), Some(0)));
        assert!(should_sample_stalled_thread(project, 2, Some(3), Some(45_000)));
    }

    #[test]
    fn connected_pipe_ends_recompile_reload_inference() {
        let connected = domain_state_for_inputs(&connected_recompile_inputs(true));
        assert_eq!(connected.phase, "none");
        assert_eq!(connected.source, "pipe");

        let disconnected = domain_state_for_inputs(&connected_recompile_inputs(false));
        assert_eq!(disconnected.phase, "reloading");
        assert_eq!(disconnected.source, "inference");
    }

    fn scope(
        checkout: &str,
        workspace_generation: u64,
        service_generation: u64,
    ) -> WorkspaceEventScope {
        let checkout_id = CheckoutId::new(checkout).expect("checkout id");
        WorkspaceEventScope {
            project_id: ProjectId::new("project-state-probe-test").expect("project id"),
            checkout_id: checkout_id.clone(),
            workspace_generation,
            materialization_epoch: None,
            service_instance_id: Some(ServiceInstanceId::for_service(&checkout_id, "unity")),
            service_generation: Some(service_generation),
        }
    }

    #[test]
    fn checkout_statuses_do_not_overwrite_each_other_or_leak_across_generations() {
        let root_a = r"F:\scope-tests\state-probe-a";
        let root_b = r"F:\scope-tests\state-probe-b";
        let scope_a = scope("checkout-state-probe-a", 11, 101);
        let scope_b = scope("checkout-state-probe-b", 12, 102);
        bind_workspace_scope(root_a, &scope_a);
        bind_workspace_scope(root_b, &scope_b);

        record_project_status(root_a, |status| {
            status.enabled = true;
            status.tier = UnityStateProbeTier::Stack;
            status.process_id = Some(1101);
        });
        record_project_status(root_b, |status| {
            status.enabled = true;
            status.tier = UnityStateProbeTier::Passive;
            status.process_id = Some(1202);
        });

        let status_a = status_for_scope(&scope_a);
        let status_b = status_for_scope(&scope_b);
        assert_eq!(status_a.process_id, Some(1101));
        assert_eq!(status_a.tier, UnityStateProbeTier::Stack);
        assert_eq!(status_b.process_id, Some(1202));
        assert_eq!(status_b.tier, UnityStateProbeTier::Passive);

        let replacement = scope("checkout-state-probe-a", 13, 103);
        bind_workspace_scope(root_a, &replacement);
        assert_eq!(status_for_scope(&scope_a).process_id, None);
        assert_eq!(status_for_scope(&replacement).process_id, None);
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::NativeSample;

    pub(super) fn clear_cache() {}

    pub(super) fn scan_main_thread(
        _pid: u32,
        _thread_hint: Option<u32>,
        _samples: u32,
        _interval: std::time::Duration,
        _max_frames: usize,
    ) -> Result<super::ExternalStackScan, String> {
        Err("main-thread stack scan is only supported on Windows".to_string())
    }

    pub(super) fn sample(
        _project_path: &str,
        _pid: u32,
        _module_path: &str,
        _allow_suspend: bool,
        _expected_created: Option<u64>,
    ) -> Result<Option<NativeSample>, String> {
        Ok(None)
    }
}
