//! Unity hot reload: method-body level C# edits applied to the running
//! Editor in seconds, without a Unity recompile or domain reload.
//!
//! The sidecar (`locus_compile_server`) classifies each edited file
//! (`analyze/hotDiff`) and compiles a rewritten patch assembly
//! (`compile/hotPatch`); the Unity plugin loads it and redirects the original
//! methods with MonoMod detours (`hot_patch_loaded`). Anything not provably
//! hot-safe — signature/field/type-shape changes — queues for the existing
//! `unity_recompile` path instead. See `unity-hotreload-plan.md`.

pub mod coordinator;
pub mod selftest;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

// Session counters, surfaced through the csharp-compile status payload
// (settings card) for rollout observability, mirroring the sidecar compiler
// counters.
static PATCHES_APPLIED: AtomicU64 = AtomicU64::new(0);
static PATCH_FAILURES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PATCHES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PATCH_BYTES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PATCH_CODE: AtomicU64 = AtomicU64::new(0);
static COLD_QUEUED: AtomicU64 = AtomicU64::new(0);

/// Called once from app setup with the persisted flag.
pub fn initialize(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

/// Counter snapshot for the settings status payload.
#[derive(Debug, Clone, Copy, Default)]
pub struct HotReloadCounters {
    pub patches_applied: u64,
    pub patch_failures: u64,
    pub active_patches: u64,
    pub active_patch_bytes: u64,
    pub active_patch_code: u64,
    pub cold_queued: u64,
}

pub fn counters() -> HotReloadCounters {
    HotReloadCounters {
        patches_applied: PATCHES_APPLIED.load(Ordering::Relaxed),
        patch_failures: PATCH_FAILURES.load(Ordering::Relaxed),
        active_patches: ACTIVE_PATCHES.load(Ordering::Relaxed),
        active_patch_bytes: ACTIVE_PATCH_BYTES.load(Ordering::Relaxed),
        active_patch_code: ACTIVE_PATCH_CODE.load(Ordering::Relaxed),
        cold_queued: COLD_QUEUED.load(Ordering::Relaxed),
    }
}

pub(crate) fn record_patch_applied(assembly_bytes: u64, code_entries: u64) {
    PATCHES_APPLIED.fetch_add(1, Ordering::Relaxed);
    ACTIVE_PATCHES.fetch_add(1, Ordering::Relaxed);
    ACTIVE_PATCH_BYTES.fetch_add(assembly_bytes, Ordering::Relaxed);
    ACTIVE_PATCH_CODE.fetch_add(code_entries, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

pub(crate) fn record_patch_failure() {
    PATCH_FAILURES.fetch_add(1, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

pub(crate) fn set_cold_queue_depth(depth: u64) {
    COLD_QUEUED.store(depth, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

/// All detours die with the AppDomain (recompile / reload); the disk already
/// holds every hot-applied edit, so the real compile converges naturally.
pub(crate) fn reset_active_patches() {
    ACTIVE_PATCHES.store(0, Ordering::Relaxed);
    ACTIVE_PATCH_BYTES.store(0, Ordering::Relaxed);
    ACTIVE_PATCH_CODE.store(0, Ordering::Relaxed);
    CONVERGENCE_PENDING.store(false, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

// ── H6: automatic convergence ────────────────────────────────────────
//
// Field virtualization (M4) makes the hot/real state gap grow with every
// patch — convergence stops being an optimization and becomes part of the
// mechanism. A silent real recompile runs when any of:
//   • active patches reach the threshold,
//   • the session goes idle with patches live,
//   • the editor leaves play mode with patches live (deferred triggers
//     also land here: recompiling mid-play would kill the play session).

const CONVERGE_ACTIVE_THRESHOLD: u64 = 8;
const CONVERGE_IDLE_SECS: u64 = 10 * 60;

/// Bumped on every hot-reload apply; an idle watchdog only fires if its
/// generation is still current when it wakes.
static CONVERGE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// A trigger fired during play mode: converge on play-mode exit.
static CONVERGENCE_PENDING: AtomicBool = AtomicBool::new(false);
/// A convergence recompile is currently running.
static CONVERGENCE_RUNNING: AtomicBool = AtomicBool::new(false);

/// Called by the coordinator after each successful hot patch.
pub(crate) fn note_patch_applied(project_path: &str) {
    let generation = CONVERGE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    let project = project_path.to_string();

    if counters().active_patches >= CONVERGE_ACTIVE_THRESHOLD {
        let threshold_project = project.clone();
        tauri::async_runtime::spawn(async move {
            try_converge(&threshold_project, "active patch threshold").await;
        });
    }

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(CONVERGE_IDLE_SECS)).await;
        if CONVERGE_GENERATION.load(Ordering::Relaxed) != generation {
            return; // newer activity rearmed the watchdog
        }
        if counters().active_patches == 0 {
            return;
        }
        try_converge(&project, "idle session").await;
    });
}

/// Called from the connection monitor on an editor play-mode transition.
pub(crate) async fn on_play_mode_exited(project_path: &str) {
    if !is_enabled() {
        return;
    }
    let pending = CONVERGENCE_PENDING.swap(false, Ordering::Relaxed);
    if !pending && counters().active_patches == 0 {
        return;
    }
    try_converge(project_path, "left play mode").await;
}

/// Run the silent convergence recompile unless the editor is playing (then
/// defer to the play-exit trigger). Reuses the unity_recompile pipeline, so
/// the cold queue, pending baselines and type index all settle.
async fn try_converge(project_path: &str, why: &str) {
    if !is_enabled() || !crate::csharp_compile::is_enabled() {
        return;
    }
    if counters().active_patches == 0 && counters().cold_queued == 0 {
        return;
    }
    if CONVERGENCE_RUNNING.swap(true, Ordering::Relaxed) {
        return; // one at a time
    }

    let result = async {
        let (connected, status, _) = crate::unity_bridge::query_unity_status(project_path).await;
        if !connected {
            return Err("editor not connected".to_string());
        }
        if crate::unity_bridge::is_play_mode_status(status) {
            CONVERGENCE_PENDING.store(true, Ordering::Relaxed);
            return Err("editor in play mode; deferred to play-mode exit".to_string());
        }
        eprintln!("[HotReload] auto-convergence ({why}): running a silent recompile");
        crate::unity_bridge::recompile_and_wait(project_path).await
    }
    .await;

    CONVERGENCE_RUNNING.store(false, Ordering::Relaxed);
    match result {
        Ok(_) => eprintln!("[HotReload] auto-convergence completed ({why})"),
        Err(error) => eprintln!("[HotReload] auto-convergence skipped ({why}): {error}"),
    }
}
