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

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

// Experimental (Phase B, default off): when set, the desktop tells the Unity
// plugin (via the hot_patch_loaded payload) it may force-JIT a synthetic caller
// stub to evaluate a not-yet-evaluated method's inline risk, instead of relying
// only on the plugin's static heuristic. Correctness is unaffected — every
// inline-risk verdict still converges through recompile.
static INLINE_FORCE_EVALUATE: AtomicBool = AtomicBool::new(false);

// Set when the compile-server sidecar that held the live hot-reload session
// registries — field stores, session images, member surfaces, all in-memory
// instance state on `CompileService` — died and was respawned while patches
// were still active. The fresh process starts with EMPTY registries, so a later
// hot patch touching an already-virtualized field would miss its original store
// (`FieldStoreRegistry.SnapshotFor` returns nothing), mint a new one, and
// silently split the field's value from the copy the live detours read. While
// this is set the coordinator refuses to hot-apply and routes edits to
// `unity_recompile`, which rebuilds consistent state. The sidecar is shared by
// every project, so the loss is global: it is cleared (by the coordinator's
// `on_domain_reloaded`) only once NO project holds live patches — with nothing
// left to split, the lost registries are moot. This is the crash/timeout-path
// analogue of the `active_patches == 0` gate that stops a binary-change restart
// from splitting the same state.
static SESSION_REGISTRY_LOST: AtomicBool = AtomicBool::new(false);

/// Called once from app setup with the persisted flags.
pub fn initialize(enabled: bool, inline_force_evaluate: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    INLINE_FORCE_EVALUATE.store(inline_force_evaluate, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

/// Whether the desktop should ask the plugin to force-evaluate inline risk
/// (Phase B). Shipped to Unity in each hot_patch_loaded payload.
pub fn inline_force_evaluate_enabled() -> bool {
    INLINE_FORCE_EVALUATE.load(Ordering::Relaxed)
}

pub fn set_inline_force_evaluate_enabled(value: bool) {
    INLINE_FORCE_EVALUATE.store(value, Ordering::Relaxed);
    crate::csharp_compile::emit_status_in_background();
}

/// Counter snapshot for the settings status payload (rollout observability,
/// mirroring the sidecar compiler counters). The per-project tallies live in the
/// coordinator's `ProjectState` — the convergence control path reads them per
/// project so two open editors never cross-contaminate — and
/// `coordinator::counters` aggregates them here for the status card.
#[derive(Debug, Clone, Copy, Default)]
pub struct HotReloadCounters {
    pub patches_applied: u64,
    pub patch_failures: u64,
    pub active_patches: u64,
    pub active_patch_bytes: u64,
    pub active_patch_code: u64,
    pub cold_queued: u64,
}

/// Flagged by the compile-server manager when it respawns the sidecar after a
/// crash/timeout with hot patches still live. See `SESSION_REGISTRY_LOST`.
pub(crate) fn note_sidecar_session_lost() {
    // Only log on the 0→1 edge so a burst of post-crash requests does not spam.
    if !SESSION_REGISTRY_LOST.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[HotReload] compile server restarted with live patches; hot-reload session state \
             lost — routing further edits to unity_recompile until convergence"
        );
        crate::csharp_compile::emit_status_in_background();
    }
}

/// Whether a sidecar restart invalidated the live hot-reload session, so the
/// next batch must converge through a real compile instead of hot-applying.
pub fn session_registry_lost() -> bool {
    SESSION_REGISTRY_LOST.load(Ordering::Relaxed)
}

/// Clear the lost-session gate. The coordinator calls this once no project holds
/// live patches (a detour-killing convergence makes the blanked registries
/// moot); keying on the global total — not a single project — keeps a sibling
/// editor whose patches are still split correctly gated.
pub(crate) fn clear_session_registry_lost() {
    SESSION_REGISTRY_LOST.store(false, Ordering::Relaxed);
}
