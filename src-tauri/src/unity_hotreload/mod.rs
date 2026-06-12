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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

// Session counters, surfaced through the csharp-compile status payload
// (settings card) for rollout observability, mirroring the sidecar compiler
// counters.
static PATCHES_APPLIED: AtomicU64 = AtomicU64::new(0);
static PATCH_FAILURES: AtomicU64 = AtomicU64::new(0);
static ACTIVE_PATCHES: AtomicU64 = AtomicU64::new(0);
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
    pub cold_queued: u64,
}

pub fn counters() -> HotReloadCounters {
    HotReloadCounters {
        patches_applied: PATCHES_APPLIED.load(Ordering::Relaxed),
        patch_failures: PATCH_FAILURES.load(Ordering::Relaxed),
        active_patches: ACTIVE_PATCHES.load(Ordering::Relaxed),
        cold_queued: COLD_QUEUED.load(Ordering::Relaxed),
    }
}

pub(crate) fn record_patch_applied() {
    PATCHES_APPLIED.fetch_add(1, Ordering::Relaxed);
    ACTIVE_PATCHES.fetch_add(1, Ordering::Relaxed);
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
    crate::csharp_compile::emit_status_in_background();
}

