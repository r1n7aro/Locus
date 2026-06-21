//! Hot-reload coordinator: accumulates agent .cs edits (baseline = the text
//! the loaded assemblies were compiled from), drives the sidecar
//! `compile/hotPatch`, ships the patch to Unity via `hot_patch_loaded`, and
//! keeps the cold queue that `unity_recompile` drains.
//!
//! The baseline for a file is captured at its FIRST tool write after the
//! last convergence point (recompile / reload): every later hot reload diffs
//! the current disk text against that baseline, so re-patching a method
//! always re-detours from the original — patches never stack.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use tokio::sync::Mutex;

#[derive(Debug, Clone)]
struct PendingEdit {
    /// Absolute path as last seen (for reading the current text).
    absolute_path: String,
    /// Disk content when the agent first touched the file this cycle —
    /// matching what the loaded assemblies were compiled from.
    baseline: String,
    /// Last disk content that was accepted by the running Editor through
    /// `unity_hot_reload`. Pending entries stay until a real compile
    /// converges, so this lets write/edit report only the new, unapplied
    /// delta after an already-hot-applied file is edited again.
    applied_text: Option<String>,
}

/// C0 access-probe result for one domain generation (cells + primitives +
/// the raw Unity matrix kept for the diagnostic command).
#[derive(Debug, Clone)]
struct AccessProbeCacheEntry {
    domain_generation: String,
    caps: crate::csharp_compile::AccessCaps,
    matrix: serde_json::Value,
}

#[derive(Default)]
struct ProjectState {
    pending: HashMap<String, PendingEdit>,
    cold_paths: BTreeSet<String>,
    /// Last reload-state sample observed for this project (via
    /// `get_reload_state`): the editor process id, AppDomain generation and
    /// compile-convergence serial. A changed session id is a fresh editor
    /// instance (loaded assemblies match disk → converge); within one session a
    /// moved serial is a compile-driven convergence, a generation change with
    /// the serial held is a no-compile domain reload (play mode), and an
    /// all-same sample is a transient pipe drop (keep detours).
    last_session_id: Option<String>,
    last_domain_generation: Option<String>,
    last_converged_serial: Option<i64>,
    /// Set when an editor we were monitoring exited with edits still tracked.
    /// A relaunch's startup recompile loads those edits from disk, so the next
    /// new-session sample treats a moved serial (serial > 0) as their
    /// convergence. Consulted only at that first post-relaunch sample, then
    /// cleared; same-session serial moves drive convergence afterwards. Set
    /// explicitly (not inferred from `pending` being non-empty) so a fresh edit
    /// that races ahead of the connect baseline is never mistaken for a
    /// startup-compiled survivor — that would under-report.
    pending_survived_exit: bool,
    /// C0 capability matrix, probed once per domain generation (the probe
    /// assembly and the measured Mono both die with the domain).
    access_probe: Option<AccessProbeCacheEntry>,
    // ── hot-patch counters (per project; the status card aggregates) ──
    /// Monotonic this session: total hot patches applied / failed.
    patches_applied: u64,
    patch_failures: u64,
    /// Live detours and their assembly bytes / method+type count. Reset together
    /// when the domain dies (`on_domain_reloaded`). `cold_queued` is NOT stored
    /// here — it is `cold_paths.len()`, so it can never drift (the old global
    /// `store` was last-writer-wins across projects).
    active_patches: u64,
    active_patch_bytes: u64,
    active_patch_code: u64,
    // ── H6 automatic-convergence control (per project) ──
    /// Bumped on every apply; an idle watchdog fires only if its generation is
    /// still current when it wakes.
    converge_generation: u64,
    /// A convergence trigger fired during play mode: converge on play-mode exit.
    convergence_pending: bool,
    /// A convergence recompile is in flight for this project.
    convergence_running: bool,
}

fn projects() -> &'static Mutex<HashMap<String, ProjectState>> {
    static STATE: OnceLock<Mutex<HashMap<String, ProjectState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The project whose unapplied count the status card reflects. The connection
/// monitor sets it to the workspace it watches so a stale project's pending
/// (another editor that has since converged, a prior workspace) cannot inflate
/// the badge. Unset → aggregate across projects (back-compat for tests).
fn active_project() -> &'static std::sync::Mutex<Option<String>> {
    static ACTIVE: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();
    ACTIVE.get_or_init(|| std::sync::Mutex::new(None))
}

pub fn set_active_project(project_path: &str) {
    if let Ok(mut active) = active_project().lock() {
        *active = Some(project_key(project_path));
    }
}

/// Does this project still hold tracking the monitor must keep reconciling
/// (pending edits or a cold queue)? Lets the monitor keep observing reload
/// state after the user toggles hot reload off with work outstanding, so a
/// later Unity recompile still converges it instead of stranding a stale count.
pub async fn has_pending_state(project_path: &str) -> bool {
    let projects = projects().lock().await;
    projects
        .get(&project_key(project_path))
        .map(|state| !state.pending.is_empty() || !state.cold_paths.is_empty())
        .unwrap_or(false)
}

fn project_key(project_path: &str) -> String {
    project_path
        .strip_prefix(r"\\?\")
        .unwrap_or(project_path)
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn file_key(file_path: &str) -> String {
    file_path
        .strip_prefix(r"\\?\")
        .unwrap_or(file_path)
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

// ── per-project hot-patch counters & H6 automatic convergence ────────
//
// These tallies and the convergence state on `ProjectState` were once
// module-global atomics shared by every project. That is fine for the status
// card (which only DISPLAYS them) but was wrong for the convergence control
// path, which reads them to decide WHEN to recompile: with two editors open,
// one project's applies, resets and idle activity cross-contaminated the
// other's convergence — a merged active count tripped the threshold early, a
// domain reload zeroed a sibling's live count (under-converging it), and one
// project's activity disarmed the other's idle watchdog. They now live per
// project; `counters` re-aggregates them (active-project scoped) for the badge,
// while `total_active_patches` keeps the global view the shared sidecar needs.

const CONVERGE_ACTIVE_THRESHOLD: u64 = 8;
const CONVERGE_IDLE_SECS: u64 = 10 * 60;

/// Record a successful hot-patch apply against `project_path`: bumps the
/// monotonic session total and the live-detour tallies (reset together when the
/// domain dies, in `on_domain_reloaded`).
async fn record_patch_applied(project_path: &str, assembly_bytes: u64, code_entries: u64) {
    {
        let mut projects = projects().lock().await;
        let state = projects.entry(project_key(project_path)).or_default();
        state.patches_applied += 1;
        state.active_patches += 1;
        state.active_patch_bytes += assembly_bytes;
        state.active_patch_code += code_entries;
    }
    crate::csharp_compile::emit_status_in_background();
}

/// Record a hot-patch apply/compile failure against `project_path`.
async fn record_patch_failure(project_path: &str) {
    {
        let mut projects = projects().lock().await;
        let state = projects.entry(project_key(project_path)).or_default();
        state.patch_failures += 1;
    }
    crate::csharp_compile::emit_status_in_background();
}

/// Hot-patch counters for the settings status card. Scoped to the active project
/// when the monitor has set one (so a stale/background editor's tallies cannot
/// inflate the badge), else summed across all — mirroring `unapplied_change_count`'s
/// active-project discipline. `cold_queued` is the live `cold_paths` depth.
pub async fn counters() -> super::HotReloadCounters {
    let active = active_project()
        .lock()
        .ok()
        .and_then(|active| active.clone());
    let projects = projects().lock().await;
    let states: Vec<&ProjectState> = match active.as_deref() {
        Some(key) => projects.get(key).into_iter().collect(),
        None => projects.values().collect(),
    };
    let mut totals = super::HotReloadCounters::default();
    for state in states {
        totals.patches_applied += state.patches_applied;
        totals.patch_failures += state.patch_failures;
        totals.active_patches += state.active_patches;
        totals.active_patch_bytes += state.active_patch_bytes;
        totals.active_patch_code += state.active_patch_code;
        totals.cold_queued += state.cold_paths.len() as u64;
    }
    totals
}

/// Total live detours across ALL projects, ignoring active-project scope. The
/// compile sidecar is a single shared process, so its binary must not be swapped
/// (nor its lost-session gate cleared) while ANY project holds live patches.
pub async fn total_active_patches() -> u64 {
    let projects = projects().lock().await;
    projects.values().map(|state| state.active_patches).sum()
}

/// Live detours for ONE project — the self-test waits on its own count, not a
/// global a background editor could hold above zero.
pub async fn project_active_patches(project_path: &str) -> u64 {
    let projects = projects().lock().await;
    projects
        .get(&project_key(project_path))
        .map(|state| state.active_patches)
        .unwrap_or(0)
}

/// Called by the apply path after each successful hot patch. Arms this project's
/// idle watchdog and fires an immediate convergence when ITS own live-patch
/// count crosses the threshold — both keyed per project, so a second editor
/// neither trips nor disarms this one's convergence.
async fn note_patch_applied(project_path: &str) {
    let (generation, over_threshold) = {
        let mut projects = projects().lock().await;
        let state = projects.entry(project_key(project_path)).or_default();
        state.converge_generation += 1;
        (
            state.converge_generation,
            state.active_patches >= CONVERGE_ACTIVE_THRESHOLD,
        )
    };

    if over_threshold {
        let project = project_path.to_string();
        tauri::async_runtime::spawn(async move {
            try_converge(&project, "active patch threshold").await;
        });
    }

    let project = project_path.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(CONVERGE_IDLE_SECS)).await;
        let still_idle_with_patches = {
            let projects = projects().lock().await;
            projects
                .get(&project_key(&project))
                .map(|state| state.converge_generation == generation && state.active_patches > 0)
                .unwrap_or(false)
        };
        if still_idle_with_patches {
            try_converge(&project, "idle session").await;
        }
    });
}

/// Called from the connection monitor on an editor play-mode exit. Converges
/// this project when a trigger was deferred during play or live patches remain.
pub async fn on_play_mode_exited(project_path: &str) {
    if !super::is_enabled() {
        return;
    }
    let (was_pending, active) = {
        let mut projects = projects().lock().await;
        match projects.get_mut(&project_key(project_path)) {
            Some(state) => (
                std::mem::replace(&mut state.convergence_pending, false),
                state.active_patches,
            ),
            None => (false, 0),
        }
    };
    if !was_pending && active == 0 {
        return;
    }
    try_converge(project_path, "left play mode").await;
}

/// Run the silent convergence recompile for `project_path` unless its editor is
/// playing (then defer to the play-exit trigger). Reuses the unity_recompile
/// pipeline, so the cold queue, pending baselines and type index all settle. The
/// in-flight guard is per project: a second editor converges independently
/// rather than being silently skipped while this one runs.
async fn try_converge(project_path: &str, why: &str) {
    if !super::is_enabled() || !crate::csharp_compile::is_enabled() {
        return;
    }
    {
        let mut projects = projects().lock().await;
        let state = projects.entry(project_key(project_path)).or_default();
        if state.active_patches == 0 && state.cold_paths.is_empty() {
            return;
        }
        if state.convergence_running {
            return; // one at a time, per project
        }
        state.convergence_running = true;
    }

    let result = async {
        let (connected, status, _) = crate::unity_bridge::query_unity_status(project_path).await;
        if !connected {
            return Err("editor not connected".to_string());
        }
        if crate::unity_bridge::is_play_mode_status(status) {
            let mut projects = projects().lock().await;
            let state = projects.entry(project_key(project_path)).or_default();
            state.convergence_pending = true;
            return Err("editor in play mode; deferred to play-mode exit".to_string());
        }
        eprintln!("[HotReload] auto-convergence ({why}): running a silent recompile");
        crate::unity_bridge::recompile_and_wait(project_path).await
    }
    .await;

    {
        let mut projects = projects().lock().await;
        if let Some(state) = projects.get_mut(&project_key(project_path)) {
            state.convergence_running = false;
        }
    }
    match result {
        Ok(_) => eprintln!("[HotReload] auto-convergence completed ({why})"),
        Err(error) => eprintln!("[HotReload] auto-convergence skipped ({why}): {error}"),
    }
}

/// Reconstruct the `MethodKey` string the Unity plugin builds per detour
/// (`LocusBridge.HotReload.cs::MethodKey`) from the same fields the desktop
/// ships, so the inlined-method keys Unity returns can be mapped back to their
/// source files. MUST stay byte-identical to that plugin function:
/// `declaringType|name|param,types|s` (`|i` when instance).
fn unity_method_key(method: &crate::csharp_compile::HotPatchMethod) -> String {
    format!(
        "{}|{}|{}|{}",
        method.declaring_type,
        method.name,
        method.param_type_names.join(","),
        if method.is_static { "s" } else { "i" },
    )
}

/// Render each inlined `MethodKey` as `Type.name` with a confidence tag derived
/// from the parallel `InlineRiskSource` the plugin reported. Three distinct
/// labels, NOT two — a force-evaluated stub proves Mono *would* inline the callee
/// in a synthetic context, which is a strong signal but not the same as a real
/// caller having inlined it, so it is reported as `(high-confidence)`, never
/// conflated with the runtime-observed `(confirmed)`:
///   RuntimeInlined → `(confirmed)`        — Mono's own cached bit was already set
///   StubInlined    → `(high-confidence)`  — force-JIT stub made Mono set the bit
///   Predicted      → `(predicted)`        — static ≤20-IL heuristic only
/// When `sources` is absent or shorter (older plugin) the bare name is used —
/// never panics on a length mismatch.
fn annotate_inlined_names(keys: &[String], sources: &[String]) -> Vec<String> {
    keys.iter()
        .enumerate()
        .map(|(index, key)| {
            let mut parts = key.split('|');
            let base = match (parts.next(), parts.next()) {
                (Some(ty), Some(name)) if !name.is_empty() => format!("{ty}.{name}"),
                (Some(ty), _) => ty.to_string(),
                _ => key.clone(),
            };
            match sources.get(index).map(String::as_str) {
                Some("RuntimeInlined") => format!("{base} (confirmed)"),
                Some("StubInlined") => format!("{base} (high-confidence)"),
                Some("Predicted") => format!("{base} (predicted)"),
                _ => base,
            }
        })
        .collect()
}

fn normalize_project_file_path(project_path: &str, file_path: &str) -> String {
    let path = std::path::Path::new(file_path);
    if path.is_absolute() {
        return file_path.to_string();
    }
    std::path::Path::new(project_path)
        .join(path)
        .to_string_lossy()
        .to_string()
}

/// Is `file_path` a compile-relevant C# source of the Unity project at
/// `project_path` (inside Assets/ or Packages/, excluding the Locus plugin
/// package itself)?
pub fn is_trackable_cs_path(project_path: &str, file_path: &str) -> bool {
    let file = file_key(file_path);
    if !file.ends_with(".cs") {
        return false;
    }
    let project = project_key(project_path);
    if project.is_empty() {
        return false;
    }
    let Some(relative) = file.strip_prefix(&format!("{project}/")) else {
        return false;
    };
    if relative.starts_with("packages/com.farlocus.locus/") {
        // Plugin sources trigger a real recompile + reload by design.
        return false;
    }
    relative.starts_with("assets/") || relative.starts_with("packages/")
}

// ── B6: partial-type sibling part discovery ──────────────────────────

/// Cap on the sibling files shipped with one batch: beyond this the request
/// would balloon, and the sidecar's completeness gate turns the truncation
/// into a clean pointed cold instead of a wrong patch.
const MAX_PARTIAL_SIBLINGS: usize = 64;

/// The simple names of the partial types a source text declares —
/// grep-grade by design (regex over text, no namespaces): false positives
/// only cost a candidate that the sidecar's precise (metadata-name) filter
/// drops, while a miss would surface as the sidecar's fail-closed
/// "member has no source on disk" verdict.
fn collect_partial_type_names(text: &str, names: &mut BTreeSet<String>) {
    static PARTIAL_DECL: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = PARTIAL_DECL.get_or_init(|| {
        regex::Regex::new(
            r"\bpartial\s+(?:class|struct|interface|record)\s+@?([A-Za-z_][A-Za-z0-9_]*)",
        )
        .expect("static partial-decl regex")
    });
    for capture in pattern.captures_iter(text) {
        names.insert(capture[1].to_string());
    }
}

fn is_skipped_scan_dir(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    // Unity ignores hidden and `~`-suffixed folders; Library/Temp/obj/bin
    // never hold compile inputs.
    name.starts_with('.')
        || name.ends_with('~')
        || matches!(name.as_ref(), "Library" | "Temp" | "Logs" | "obj" | "bin")
}

/// Every trackable .cs file under Assets/ + Packages/ whose text contains
/// the token `partial` (cheap pre-filter), read once into memory.
fn scan_partial_candidates(project_path: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for root in ["Assets", "Packages"] {
        let dir = std::path::Path::new(project_path).join(root);
        if !dir.is_dir() {
            continue;
        }
        let walker = walkdir::WalkDir::new(&dir)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !is_skipped_scan_dir(entry));
        for entry in walker {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() {
                continue;
            }
            let path_text = entry.path().to_string_lossy().to_string();
            if !is_trackable_cs_path(project_path, &path_text) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            if text.contains("partial") {
                results.push((path_text, text));
            }
        }
    }
    results
}

/// Find the candidate sibling part files for every partial type the changed
/// files declare (old or new text — a part that VANISHED still names the
/// type). Closure over the candidates: a sibling can itself declare further
/// partial types whose parts must come along for its copy to compile.
/// Returns (path, current disk text) pairs; empty when no edit mentions a
/// partial declaration — the scan never runs for plain batches.
async fn discover_partial_siblings(
    project_path: &str,
    files: &[(String, String, String)],
) -> Vec<(String, String)> {
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for (_, old_text, new_text) in files {
        collect_partial_type_names(old_text, &mut needed);
        collect_partial_type_names(new_text, &mut needed);
    }
    if needed.is_empty() {
        return Vec::new();
    }

    let changed: BTreeSet<String> = files.iter().map(|(path, _, _)| file_key(path)).collect();
    let project = project_path.to_string();
    let candidates = tokio::task::spawn_blocking(move || scan_partial_candidates(&project))
        .await
        .unwrap_or_default();

    let mut remaining: Vec<(String, String, BTreeSet<String>)> = candidates
        .into_iter()
        .filter(|(path, _)| !changed.contains(&file_key(path)))
        .filter_map(|(path, text)| {
            let mut names = BTreeSet::new();
            collect_partial_type_names(&text, &mut names);
            (!names.is_empty()).then_some((path, text, names))
        })
        .collect();

    let mut selected: Vec<(String, String)> = Vec::new();
    loop {
        let mut moved = false;
        let mut index = 0;
        while index < remaining.len() {
            if remaining[index].2.iter().any(|name| needed.contains(name)) {
                let (path, text, names) = remaining.remove(index);
                needed.extend(names);
                selected.push((path, text));
                moved = true;
            } else {
                index += 1;
            }
        }
        if !moved {
            break;
        }
    }

    if selected.len() > MAX_PARTIAL_SIBLINGS {
        eprintln!(
            "[HotReload] partial sibling discovery found {} candidate files; sending the first {} \
             (the sidecar fails closed on any part it cannot see)",
            selected.len(),
            MAX_PARTIAL_SIBLINGS
        );
        selected.truncate(MAX_PARTIAL_SIBLINGS);
    }
    selected
}

/// Record a tool write to a .cs file. Called by the write/edit tools BEFORE
/// their content lands (with the prior disk text), so the baseline matches
/// what the loaded assemblies were compiled from. No-op while the feature is
/// off or the path is not a project source.
pub async fn note_cs_written(project_path: &str, file_path: &str, prior_content: String) {
    if !super::is_enabled() || !crate::csharp_compile::is_enabled() {
        return;
    }
    let absolute_path = normalize_project_file_path(project_path, file_path);
    if !is_trackable_cs_path(project_path, &absolute_path) {
        return;
    }

    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    state
        .pending
        .entry(file_key(&absolute_path))
        .or_insert_with(|| PendingEdit {
            absolute_path,
            baseline: prior_content,
            applied_text: None,
        });
    drop(projects);
    crate::csharp_compile::emit_status_in_background();
}

fn display_project_path(project_path: &str, file_path: &str) -> String {
    let project = project_path
        .strip_prefix(r"\\?\")
        .unwrap_or(project_path)
        .trim_end_matches(['/', '\\'])
        .replace('\\', "/");
    let file = file_path
        .strip_prefix(r"\\?\")
        .unwrap_or(file_path)
        .replace('\\', "/");
    let project_lower = project.to_ascii_lowercase();
    let file_lower = file.to_ascii_lowercase();
    if file_lower.starts_with(&project_lower) {
        let relative = file[project.len()..].trim_start_matches('/');
        if !relative.is_empty() {
            return relative.to_string();
        }
    }
    file
}

async fn mark_changed_keys_applied(project_path: &str, applied: &[(String, String)]) {
    if applied.is_empty() {
        return;
    }
    let mut projects = projects().lock().await;
    let Some(state) = projects.get_mut(&project_key(project_path)) else {
        return;
    };
    for (key, current_text) in applied {
        if let Some(edit) = state.pending.get_mut(key) {
            edit.applied_text = Some(current_text.clone());
        }
    }
}

async fn is_pending_edit_unapplied(edit: &PendingEdit) -> bool {
    let current = match tokio::fs::read_to_string(&edit.absolute_path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return true,
    };
    if current == edit.baseline {
        return false;
    }
    !edit
        .applied_text
        .as_ref()
        .map(|applied| applied == &current)
        .unwrap_or(false)
}

pub async fn unapplied_change_count() -> u64 {
    let active = active_project()
        .lock()
        .ok()
        .and_then(|active| active.clone());
    let snapshot: Vec<PendingEdit> = {
        let projects = projects().lock().await;
        match active.as_deref() {
            Some(key) => projects
                .get(key)
                .map(|state| state.pending.values().cloned().collect())
                .unwrap_or_default(),
            None => projects
                .values()
                .flat_map(|state| state.pending.values().cloned())
                .collect(),
        }
    };

    let mut count = 0u64;
    for edit in snapshot {
        if is_pending_edit_unapplied(&edit).await {
            count += 1;
        }
    }
    count
}

/// Agent-facing status appended after write/edit. It reports the C# changes
/// that are still not live in the running Editor and uses the sidecar's
/// cheap syntax/member diff (`analyze/hotDiff`) to hint whether the pending
/// batch can use `unity_hot_reload`.
pub async fn format_pending_edit_status(
    project_path: &str,
    touched_file_path: &str,
) -> Option<String> {
    let touched_absolute = normalize_project_file_path(project_path, touched_file_path);
    let touched_trackable = is_trackable_cs_path(project_path, &touched_absolute);
    if !touched_trackable {
        return None;
    }
    crate::csharp_compile::emit_status_in_background();

    if !super::is_enabled() {
        return Some(
            "Unity C# status:\n- This .cs change is on disk and is not applied to the running Editor yet.\n- Hot reload: disabled in Settings > Code Analysis. Use unity_recompile to apply it."
                .to_string(),
        );
    }
    if !crate::csharp_compile::is_enabled() {
        return Some(
            "Unity C# status:\n- This .cs change is on disk and is not applied to the running Editor yet.\n- Hot reload: unavailable because the sidecar compiler is disabled. Use unity_recompile to apply it."
                .to_string(),
        );
    }

    let snapshot: Vec<(String, PendingEdit, bool)> = {
        let projects = projects().lock().await;
        match projects.get(&project_key(project_path)) {
            Some(state) => state
                .pending
                .iter()
                .map(|(key, edit)| (key.clone(), edit.clone(), state.cold_paths.contains(key)))
                .collect(),
            None => Vec::new(),
        }
    };

    if snapshot.is_empty() {
        return None;
    }

    let mut unapplied: Vec<(String, String, String, bool)> = Vec::new();
    let mut hot_applied_unconverged = 0usize;

    for (_, edit, cold) in snapshot {
        let current = match tokio::fs::read_to_string(&edit.absolute_path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                let display = display_project_path(project_path, &edit.absolute_path);
                return Some(format!(
                    "Unity C# status:\n- Failed to read pending file {display}: {error}\n- Hot reload: cannot assess this edit; use unity_recompile after fixing the file access issue."
                ));
            }
        };

        if current == edit.baseline {
            continue;
        }

        if edit
            .applied_text
            .as_ref()
            .map(|applied| applied == &current)
            .unwrap_or(false)
        {
            hot_applied_unconverged += 1;
            continue;
        }

        unapplied.push((
            edit.absolute_path.clone(),
            edit.baseline.clone(),
            current,
            cold,
        ));
    }

    if unapplied.is_empty() {
        if hot_applied_unconverged == 0 {
            return Some(
                "Unity C# status:\n- No unapplied C# changes are tracked for the running Editor."
                    .to_string(),
            );
        }
        return Some(format!(
            "Unity C# status:\n- No unapplied C# changes are tracked for the running Editor.\n- {hot_applied_unconverged} hot-applied file(s) still await unity_recompile convergence."
        ));
    }

    unapplied.sort_by(|a, b| {
        display_project_path(project_path, &a.0).cmp(&display_project_path(project_path, &b.0))
    });

    let display_paths: Vec<String> = unapplied
        .iter()
        .take(6)
        .map(|(path, _, _, cold)| {
            let display = display_project_path(project_path, path);
            if *cold {
                format!("{display} (queued for unity_recompile)")
            } else {
                display
            }
        })
        .collect();
    let more = unapplied.len().saturating_sub(display_paths.len());

    let mut lines = vec![
        "Unity C# status:".to_string(),
        format!(
            "- Unapplied C# changes: {} file(s): {}{}",
            unapplied.len(),
            display_paths.join(", "),
            if more > 0 {
                format!(", +{more} more")
            } else {
                String::new()
            }
        ),
    ];
    if hot_applied_unconverged > 0 {
        lines.push(format!(
            "- {hot_applied_unconverged} hot-applied file(s) still await unity_recompile convergence."
        ));
    }

    let params = match crate::csharp_compile::params::get_params(project_path).await {
        Ok(params) => params,
        Err(error) => {
            lines.push(format!(
                "- Hot reload: cannot assess current edits ({error}). Use unity_recompile if Unity must see them now."
            ));
            return Some(lines.join("\n"));
        }
    };

    let analysis_files: Vec<(String, String, String)> = unapplied
        .iter()
        .map(|(path, old_text, new_text, _)| (path.clone(), old_text.clone(), new_text.clone()))
        .collect();

    match crate::csharp_compile::analyze_hot_diff(&params, &analysis_files).await {
        Ok(analysis) if analysis.all_hot => {
            let caller_checks: usize = analysis
                .files
                .iter()
                .map(|file| file.requires_caller_check)
                .sum();
            if caller_checks > 0 {
                lines.push(format!(
                    "- Hot reload: likely supported; {caller_checks} member surface change(s) still need call-site verification during unity_hot_reload."
                ));
            } else {
                lines.push(
                    "- Hot reload: supported for the current unapplied edits. Call unity_hot_reload to apply without a domain reload."
                        .to_string(),
                );
            }
        }
        Ok(analysis) => {
            lines.push(
                "- Hot reload: not supported for the current unapplied edits. Use unity_recompile."
                    .to_string(),
            );
            for file in analysis.files.iter().filter(|file| !file.hot).take(4) {
                let mut reasons = file.reasons.clone();
                if let Some(error) = &file.syntax_error {
                    reasons.push(error.clone());
                }
                if reasons.is_empty() {
                    reasons.push("no hot-reloadable runtime change detected".to_string());
                }
                lines.push(format!(
                    "  {}: {}",
                    display_project_path(project_path, &file.path),
                    reasons.join("; ")
                ));
            }
        }
        Err(error) => {
            lines.push(format!(
                "- Hot reload: cannot assess current edits ({error}). Use unity_recompile if Unity must see them now."
            ));
        }
    }

    Some(lines.join("\n"))
}

/// A real compile converged (recompile completed / domain reloaded): disk is
/// the new truth, detours are gone with the old domain.
pub async fn on_recompile_converged(project_path: &str) {
    let mut projects = projects().lock().await;
    if let Some(state) = projects.get_mut(&project_key(project_path)) {
        state.pending.clear();
        state.cold_paths.clear();
        // The old domain died with the recompile; the next observation
        // re-seeds the session/generation/serial trackers.
        state.last_session_id = None;
        state.last_domain_generation = None;
        state.last_converged_serial = None;
        // Survivors (if any) are now compiled in — the hint is spent.
        state.pending_survived_exit = false;
    }
    drop(projects);
    on_domain_reloaded(project_path).await;
    // The cold queue is `cold_paths`, cleared above; on_domain_reloaded emits
    // the refreshed status (no separate cold-depth counter to zero).
    // A real compile changes the type/member surface, the reference assembly
    // mtimes, and serialized schema — invalidate the cached type index (which
    // also cascades to compile params + serialized schema) so a Unity-initiated
    // recompile leaves the same consistent caches an explicit unity_recompile
    // does, not a stale window.
    crate::unity_type_index::invalidate_cached_type_index(project_path).await;
}

/// A Unity domain reload invalidates active detours and transient hot-patch
/// type-index rows. Pending source edits stay tracked until an actual
/// compile convergence confirms disk and loaded assemblies match.
pub async fn on_domain_reloaded(project_path: &str) {
    {
        let mut projects = projects().lock().await;
        if let Some(state) = projects.get_mut(&project_key(project_path)) {
            for edit in state.pending.values_mut() {
                edit.applied_text = None;
            }
            // All detours die with the AppDomain; disk already holds every
            // hot-applied edit, so the real compile converges naturally. Reset
            // only THIS project's live-detour tallies (a module-global store(0)
            // used to zero a sibling editor's live count, under-converging it)
            // and its deferred-convergence flag — the monotonic session totals
            // stay.
            state.active_patches = 0;
            state.active_patch_bytes = 0;
            state.active_patch_code = 0;
            state.convergence_pending = false;
        }
        // The shared sidecar's lost-session gate clears once NO project holds
        // live patches: with nothing left to split against the blanked
        // registries, hot-apply is safe again. Keyed on the global total (not
        // this one project) so a sibling whose patches are still split stays
        // gated.
        if projects.values().all(|state| state.active_patches == 0) {
            super::clear_session_registry_lost();
        }
    }
    crate::csharp_compile::emit_status_in_background();
    match crate::unity_type_index::drop_hot_patch_types(project_path).await {
        Ok(removed) if removed > 0 => {
            eprintln!("[HotReload] dropped {removed} hot-patch type-index row(s)");
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("[HotReload] hot-patch type-index cleanup skipped: {error}");
        }
    }
}

/// How a reload-state sample relates to the last one observed for a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReloadDecision {
    /// First sample this session: seed the trackers, take no action.
    Seed,
    /// Nothing moved (steady state / transient pipe blip): keep everything.
    Unchanged,
    /// A successful compilation advanced the convergence serial — disk is now
    /// the loaded truth (whether it reloaded the domain or compiled in place).
    Converged,
    /// The domain generation moved with no compilation behind it (e.g. entering
    /// play mode): live detours died but disk still differs from the loaded
    /// assemblies, so the edits stay pending.
    Reloaded,
}

/// Classify a `(session, generation, serial)` sample against the last one.
///
/// A changed session id is a fresh editor instance: its detours are dead, but a
/// clean disk load is NOT proven in general — startup compilation can fail and
/// leave the editor running the last good assemblies, not the current sources.
/// The exception is edits that survived the previous editor's exit
/// (`survived_exit`): they are on disk, so the relaunch's startup recompile
/// loads them — proven once that instance reports a successful compile
/// (serial > 0). So a new session with survivors and a moved serial converges;
/// a new session whose serial is still 0 (not compiled yet, or startup compile
/// failed) keeps pending, as does any new session without recorded survivors.
/// Within one session a moved serial is a real compile (covers compile-in-place),
/// a generation change with the serial held is a no-compile domain reload, and
/// an all-same sample is a transient pipe drop.
fn classify_reload(
    last_session: Option<&str>,
    last_generation: Option<&str>,
    last_serial: Option<i64>,
    current_session: &str,
    current_generation: &str,
    current_serial: i64,
    survived_exit: bool,
) -> ReloadDecision {
    let Some(last_session) = last_session else {
        // First sample for this project this app session: nothing to compare
        // against. If edits survived a prior editor's exit and this instance has
        // already compiled (serial > 0), its startup recompile loaded them —
        // converge. Otherwise just seed the trackers and keep any pending
        // uncleared. (The monitor seeds a baseline on connect, before any edit,
        // so a fresh edit is never the first sample and so never mistaken here
        // for a survivor.)
        if survived_exit && current_serial > 0 {
            return ReloadDecision::Converged;
        }
        return ReloadDecision::Seed;
    };
    if last_session != current_session {
        // Fresh editor instance. Converge only the edits we know outlived the
        // previous editor, and only once this instance has compiled them in
        // (serial > 0); otherwise keep evidence until it reports a compile.
        if survived_exit && current_serial > 0 {
            return ReloadDecision::Converged;
        }
        return ReloadDecision::Reloaded;
    }
    if last_serial != Some(current_serial) {
        ReloadDecision::Converged
    } else if last_generation != Some(current_generation) {
        ReloadDecision::Reloaded
    } else {
        ReloadDecision::Unchanged
    }
}

/// Reconcile pending edits against the editor's reload lifecycle. The
/// connection monitor calls this every poll while connected (and right after a
/// reconnect), feeding the `(domain_generation, converged_serial)` pair read
/// from Unity via `get_reload_state`.
///
/// This is what makes a Unity-initiated recompile (manual Ctrl+R, save, focus
/// auto-refresh, startup) converge the unapplied set exactly like a Locus
/// `unity_recompile`: convergence keys on Unity's own compilation serial, not
/// on who asked for the compile — and it works whether or not the pipe dropped
/// across the reload (the native broker survives it). A transient pipe drop
/// within one domain (same generation, same serial) keeps active detours.
pub async fn observe_reload_state(
    project_path: &str,
    session_id: String,
    domain_generation: String,
    converged_serial: i64,
) {
    let decision = {
        let projects = projects().lock().await;
        let state = projects.get(&project_key(project_path));
        let survived_exit = state
            .map(|state| state.pending_survived_exit)
            .unwrap_or(false);
        classify_reload(
            state.and_then(|state| state.last_session_id.as_deref()),
            state.and_then(|state| state.last_domain_generation.as_deref()),
            state.and_then(|state| state.last_converged_serial),
            &session_id,
            &domain_generation,
            converged_serial,
            survived_exit,
        )
    };

    match decision {
        ReloadDecision::Converged => {
            eprintln!(
                "[HotReload] editor converged (session {session_id}, serial {converged_serial}); clearing tracked edits"
            );
            on_recompile_converged(project_path).await;
        }
        ReloadDecision::Reloaded => {
            eprintln!(
                "[HotReload] editor domain reloaded with no recompile; detours dropped, edits stay pending"
            );
            on_domain_reloaded(project_path).await;
        }
        ReloadDecision::Seed | ReloadDecision::Unchanged => {}
    }

    // Always record the latest sample so the next one is judged against it
    // (convergence/reload may have reset the trackers above).
    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    state.last_session_id = Some(session_id);
    state.last_domain_generation = Some(domain_generation);
    state.last_converged_serial = Some(converged_serial);
    // The survived-exit hint is for the first sample of the relaunched instance
    // only (consulted above). Past it, this instance's own serial moves drive
    // convergence, so clear it — otherwise a later edit made before the next
    // sample could be swept up by a stale hint.
    state.pending_survived_exit = false;
}

/// The editor process is gone (quit or crash). Its detours died with it, so
/// re-mark hot-applied edits as unapplied and zero the active-patch counters —
/// but KEEP pending/cold. Those edits are still not in any running editor, and a
/// relaunch is NOT proof they are: startup compilation can fail and leave the
/// editor running the last good assemblies. They converge only when a session
/// reports a successful compile (a moved serial) or an explicit unity_recompile.
/// The trackers reset so the next connect re-seeds a baseline.
pub async fn on_editor_exited(project_path: &str) {
    // Reuse the domain-reload cleanup: drops active detours, re-marks
    // hot-applied edits unapplied, and clears hot-patch type-index rows — all
    // without touching pending/cold.
    on_domain_reloaded(project_path).await;
    let mut projects = projects().lock().await;
    if let Some(state) = projects.get_mut(&project_key(project_path)) {
        // Remember that these edits outlived the editor: the relaunch's startup
        // recompile will load them, so the next new-session sample converges on
        // a moved serial (serial > 0) rather than stranding a stale count. A
        // failed startup compile leaves serial 0, so they correctly stay pending.
        state.pending_survived_exit = !state.pending.is_empty() || !state.cold_paths.is_empty();
        state.last_session_id = None;
        state.last_domain_generation = None;
        state.last_converged_serial = None;
    }
}

/// Queue file keys for the `unity_recompile` convergence pass and return the
/// queue depth. Used for cold classifications AND Unity-side patch
/// rejections — a rejected patch leaves the files un-applied, so they need a
/// real compile exactly like cold files do.
async fn queue_cold_paths(project_path: &str, keys: &[String]) -> usize {
    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    for key in keys {
        state.cold_paths.insert(key.clone());
    }
    state.cold_paths.len()
}

/// The compile-server sidecar restarted while hot patches were live, losing its
/// in-memory hot-reload session registries (field stores, session images, member
/// surfaces). Hot-applying now would let the sidecar's empty field-store registry
/// mint a fresh store for an already-virtualized field and silently split its
/// value from the live detours' copy. Route the batch to the `unity_recompile`
/// convergence path instead — it rebuilds consistent state and clears the
/// lost-session flag once no project holds live patches (`on_domain_reloaded`).
/// Returns the agent-facing message and queues the changed files so the
/// recompile drains them.
async fn converge_after_session_loss(project_path: &str, changed_keys: &[String]) -> String {
    let queued = queue_cold_paths(project_path, changed_keys).await;
    crate::csharp_compile::emit_status_in_background();
    format!(
        "Hot reload unavailable: the compile server restarted while patches were live, losing the \
         hot-reload session state. Applying these edits hot now could split a hot-added field's \
         value, so they are queued for a real compile ({queued} file(s)). Run unity_recompile to \
         converge; hot reload resumes cleanly afterward."
    )
}

fn base64_decoded_len(value: &str) -> u64 {
    let trimmed = value.trim();
    let len = trimmed.len();
    if len == 0 {
        return 0;
    }
    let padding = trimmed
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count();
    ((len / 4) * 3).saturating_sub(padding) as u64
}

fn assembly_artifact_len(assembly_b64: &str, assembly_path: Option<&str>) -> u64 {
    if let Some(path) = assembly_path.filter(|path| !path.is_empty()) {
        if let Ok(metadata) = std::fs::metadata(path) {
            return metadata.len();
        }
    }
    base64_decoded_len(assembly_b64)
}

/// Decide whether a freshly applied hot patch that declared added Unity messages
/// must fail closed (route the caller to a recompile) rather than be reported live.
/// Two hard cases: a plugin that can't drive added messages at all (`pump_supported`
/// false — an older plugin that ignored `message_drivers`), and a plugin that
/// reported messages it could NOT wire (`pump_failed_count > 0`). Both become live
/// natively after a recompile, so the file is not marked applied here. Returns the
/// agent-facing error when it must fail closed, `Ok(())` when the patch may stand.
fn message_driver_gate(
    has_message_drivers: bool,
    pump_supported: bool,
    pump_failed_count: u64,
) -> Result<(), String> {
    if has_message_drivers && !pump_supported {
        return Err(
            "This project's Unity plugin can't drive newly added Unity messages \
             (Update/LateUpdate/FixedUpdate). Update the Locus plugin, or run \
             unity_recompile to make them live."
                .to_string(),
        );
    }
    if pump_failed_count > 0 {
        return Err(format!(
            "{pump_failed_count} newly added Unity message(s) could not be wired in Unity. \
             Run unity_recompile to make them live."
        ));
    }
    Ok(())
}

// Session capability cache: the plugin's last-echoed `pump_supported`, keyed by
// domain generation (the plugin — and thus its capability — is fixed within a
// generation; a recompile / domain reload re-learns it). Lets a message-driver
// patch PREFLIGHT a plugin already known unable to drive added messages and route
// straight to a recompile, instead of applying method detours we'd immediately fail
// closed and discard. The post-response gate still backstops the first, unlearned
// patch (and records the capability for the next one).
static PUMP_CAPABILITY: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, bool>>,
> = std::sync::OnceLock::new();

fn remember_pump_capability(domain_generation: &str, supported: bool) {
    let map =
        PUMP_CAPABILITY.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(mut guard) = map.lock() {
        guard.insert(domain_generation.to_string(), supported);
    }
}

/// `Some(false)` = known unable to drive added messages, `Some(true)` = known able,
/// `None` = not learned for this generation yet.
fn known_pump_capability(domain_generation: &str) -> Option<bool> {
    PUMP_CAPABILITY.get().and_then(|map| {
        map.lock()
            .ok()
            .and_then(|guard| guard.get(domain_generation).copied())
    })
}

/// Build the message-driver portion of the apply summary (appended after the
/// redirect / new-type counts). The `total` added messages split into `driven`
/// (total − skipped) and the skipped ones; when the plugin sent the skipped
/// messages by name (`skipped_messages`, each a "message — reason") they are listed
/// — capped so a large batch does not flood — else only the count is shown (older
/// plugins). Each DISTINCT non-empty note is appended once, in first-seen order, so
/// lifecycle / ordering / side-effect caveats reach the agent without repetition.
/// Returns "" when there were no added messages.
fn message_driver_summary(
    total: usize,
    skipped_count: u64,
    skipped_messages: &[String],
    notes: &[&str],
) -> String {
    if total == 0 {
        return String::new();
    }
    let mut out = String::new();
    let driven = (total as u64).saturating_sub(skipped_count);
    if driven > 0 {
        out.push_str(&format!(
            ", {driven} added Unity message(s) wired to a runtime driver (see notes for timing/order caveats)"
        ));
    }
    if skipped_count > 0 {
        if skipped_messages.is_empty() {
            out.push_str(&format!(
                ", {skipped_count} added Unity message(s) not driven (not an engine message, edit-time only, or no live instance)"
            ));
        } else {
            const MAX_NAMED: usize = 6;
            let shown = skipped_messages.len().min(MAX_NAMED);
            let names = skipped_messages[..shown].join("; ");
            out.push_str(&format!(
                ", {skipped_count} added Unity message(s) not driven: {names}"
            ));
            if skipped_messages.len() > shown {
                out.push_str(&format!(" (+{} more)", skipped_messages.len() - shown));
            }
        }
    }
    let mut seen_notes: Vec<&str> = Vec::new();
    for &note in notes {
        if !note.is_empty() && !seen_notes.contains(&note) {
            seen_notes.push(note);
            out.push_str(&format!(".\n{note}"));
        }
    }
    out
}

#[derive(Debug)]
struct AppliedHotPatch {
    engine: String,
    inlined_method_keys: Vec<String>,
    /// Parallel to `inlined_method_keys`: the InlineRiskSource name the plugin
    /// reported for each ("RuntimeInlined" / "StubInlined" / "Predicted"). Empty
    /// from older plugins.
    inlined_sources: Vec<String>,
    image_register_error: Option<String>,
    /// Added Unity messages the plugin received but did not drive (not a
    /// MonoBehaviour, parameter not the real engine type, edit-time-only, or a
    /// catch-up with no live instance). Surfaced as a note, not an error.
    pump_skipped_count: u64,
    /// Each skipped message's "message — reason", when the plugin sent them, so the
    /// summary can name them instead of showing a bare count. Empty from older
    /// plugins (then only the count is reported).
    pump_skipped_messages: Vec<String>,
}

async fn apply_compiled_hot_patch(
    project_path: &str,
    params: &crate::csharp_compile::CompileParams,
    assembly_name: &str,
    assembly_b64: &str,
    assembly_path: Option<&String>,
    methods: &[crate::csharp_compile::HotPatchMethod],
    new_types: &[crate::csharp_compile::HotPatchNewType],
    message_drivers: &[crate::csharp_compile::HotPatchMessageDriver],
) -> Result<AppliedHotPatch, String> {
    // Preflight (P1-b): if a prior apply in this domain generation already showed the
    // plugin cannot drive added messages, do NOT apply method detours we would
    // immediately fail closed and discard — fail closed now, before the Unity
    // round-trip, so the caller routes straight to a recompile. The first, unlearned
    // message-driver patch still hits the post-response gate below (which records the
    // capability), so this only short-circuits once the answer is known.
    if !message_drivers.is_empty()
        && known_pump_capability(&params.domain_generation) == Some(false)
    {
        message_driver_gate(true, false, 0)?;
    }

    let mut payload = serde_json::json!({
        "patch_id": assembly_name,
        "domain_generation": params.domain_generation,
        // Experimental Phase B gate (default off): ask the plugin to force-JIT a
        // synthetic caller stub to evaluate inline risk. Older plugins ignore it.
        "inline_force_evaluate": super::inline_force_evaluate_enabled(),
        "methods": methods.iter().map(|m| serde_json::json!({
            "declaring_type": m.declaring_type,
            "patch_declaring_type": m.patch_declaring_type,
            "name": m.name,
            "param_type_names": m.param_type_names,
            "param_type_sigs": m.param_type_sigs,
            "is_static": m.is_static,
            "is_ctor": m.is_ctor,
            // The edited file this detour came from. The plugin uses it to clear
            // a file's stale pump registrations before re-adding (replace-by-source).
            "source_path": m.source_path,
            // Older plugins ignore the unknown field and then fail
            // resolution → whole-patch rollback + update hint (the
            // established compatibility discipline).
            "original_assembly": m.original_assembly.as_deref().unwrap_or(""),
        })).collect::<Vec<_>>(),
        // Newly added Unity messages the engine never discovers after load: the
        // plugin wires a driver by `kind` (a PlayerLoop pump or a forwarding proxy
        // MonoBehaviour). A plugin that supports this echoes pump_supported in its
        // response; if it does not, the caller fails closed to a recompile rather
        // than reporting a false "live" (see below).
        "message_drivers": message_drivers.iter().map(|d| serde_json::json!({
            "kind": d.kind,
            "declaring_type": d.declaring_type,
            "shim_type": d.shim_type,
            "shim_method": d.shim_method,
            "message": d.message,
            "param_type": d.param_type,
            "source_path": d.source_path,
        })).collect::<Vec<_>>(),
    });
    if let Some(object) = payload.as_object_mut() {
        if let Some(path) = assembly_path {
            object.insert(
                "assembly_path".to_string(),
                serde_json::Value::String(path.clone()),
            );
        } else {
            object.insert(
                "assembly_b64".to_string(),
                serde_json::Value::String(assembly_b64.to_string()),
            );
        }
    }
    let payload = payload.to_string();

    let resp = match crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_patch_loaded",
        &payload,
        std::time::Duration::from_secs(30),
    )
    .await
    {
        Ok(resp) => resp,
        Err(error) => {
            record_patch_failure(project_path).await;
            return Err(format!(
                "Unity did not accept the hot patch ({error}); use unity_recompile."
            ));
        }
    };

    if !resp.ok {
        record_patch_failure(project_path).await;
        let error = resp
            .error
            .unwrap_or_else(|| "hot patch rejected".to_string());
        if error.starts_with("unknown message type") {
            return Err(
                "The Unity plugin in this project predates hot reload; update the Locus plugin \
                 or use unity_recompile."
                    .to_string(),
            );
        }
        return Err(format!(
            "Hot patch failed in Unity: {error}\nRun unity_recompile to converge."
        ));
    }

    #[derive(serde::Deserialize, Default)]
    #[serde(default)]
    struct HotPatchLoadedResponse {
        detour_engine: String,
        inlined_method_keys: Vec<String>,
        inlined_sources: Vec<String>,
        // Pump capability echo. Absent (→ false) from plugins that predate the
        // PlayerLoop message pump: such a plugin silently load-onlys an
        // add-a-message patch without driving it, so we must NOT report it live.
        pump_supported: bool,
        pumped_count: u64,
        pump_skipped_count: u64,
        // Per-message "message — reason" for each skipped driver (empty from older
        // plugins). Lets the summary name them rather than show a bare count.
        pump_skipped_messages: Vec<String>,
        // Hard wiring failures (type/shim missing, unbindable shim). > 0 means the
        // patch claimed a driver it could not honor — fail closed to a recompile.
        pump_failed_count: u64,
    }
    let loaded = resp
        .message
        .as_deref()
        .and_then(|message| serde_json::from_str::<HotPatchLoadedResponse>(message).ok())
        .unwrap_or_default();

    // Learn the plugin's pump capability for this generation so the next
    // message-driver patch can preflight instead of applying-then-failing-closed.
    remember_pump_capability(&params.domain_generation, loaded.pump_supported);

    // Fail closed if this patch adds Unity messages the plugin cannot honor: an
    // older plugin that ignores `message_drivers` and load-onlys the patch, or one
    // that reported messages it could not wire. A recompile makes them live
    // natively; returning Err routes the caller to queue_cold_paths instead of
    // marking the file applied / live.
    message_driver_gate(
        !message_drivers.is_empty(),
        loaded.pump_supported,
        loaded.pump_failed_count,
    )?;

    let assembly_bytes = assembly_artifact_len(assembly_b64, assembly_path.map(|p| p.as_str()));
    let code_entries = methods.len().saturating_add(new_types.len()) as u64;
    record_patch_applied(project_path, assembly_bytes, code_entries).await;
    note_patch_applied(project_path).await;

    let image_register_error = match crate::csharp_compile::register_session_image(
        &params.domain_generation,
        assembly_name,
        assembly_b64,
        assembly_path.map(|p| p.as_str()),
    )
    .await
    {
        Ok(()) => None,
        Err(error) => Some(error),
    };

    let index_types: Vec<crate::unity_type_index::UnityTypeIndexEntry> = new_types
        .iter()
        .filter(|t| t.is_top_level && t.is_public)
        .map(|t| crate::unity_type_index::UnityTypeIndexEntry {
            simple_name: t.simple_name.clone(),
            namespace: t.ns.clone(),
            full_name: if t.ns.is_empty() {
                t.simple_name.clone()
            } else {
                format!("{}.{}", t.ns, t.simple_name)
            },
            assembly: assembly_name.to_string(),
        })
        .collect();
    if image_register_error.is_none() && !index_types.is_empty() {
        if let Err(error) = crate::unity_type_index::append_hot_patch_types(
            project_path,
            assembly_name,
            index_types,
        )
        .await
        {
            eprintln!("[HotReload] type index increment skipped: {error}");
        }
    }

    Ok(AppliedHotPatch {
        engine: loaded.detour_engine,
        inlined_method_keys: loaded.inlined_method_keys,
        inlined_sources: loaded.inlined_sources,
        image_register_error,
        pump_skipped_count: loaded.pump_skipped_count,
        pump_skipped_messages: loaded.pump_skipped_messages,
    })
}

const INLINE_REFRESH_MAX_DEPTH: usize = 2;
const INLINE_REFRESH_MAX_CALLERS_PER_TARGET: usize = 8;
const INLINE_REFRESH_MAX_METHODS_TOTAL: usize = 16;
const INLINE_REFRESH_MAX_FILES_TOTAL: usize = 16;

#[derive(Debug, Default)]
struct InlineCallerRefreshReport {
    rounds: usize,
    files: usize,
    methods: usize,
    notes: Vec<String>,
}

#[derive(Clone, Debug)]
struct InlineRefreshFile {
    path: String,
    old_text: String,
    new_text: String,
    force_methods: BTreeSet<String>,
}

fn caller_query_target_from_unity_key(
    key: &str,
) -> Option<crate::csharp_compile::CallerQueryTarget> {
    let parts: Vec<&str> = key.split('|').collect();
    if parts.len() != 4 {
        return None;
    }
    let declaring_type = parts[0].trim();
    let member_name = parts[1].trim();
    if declaring_type.is_empty()
        || member_name.is_empty()
        || member_name == ".ctor"
        || declaring_type.contains('<')
        || member_name.contains('<')
    {
        return None;
    }
    Some(crate::csharp_compile::CallerQueryTarget {
        declaring_type: declaring_type.to_string(),
        member_name: member_name.to_string(),
    })
}

fn resolve_caller_source_path(project_path: &str, file: &str) -> String {
    let normalized = file.replace('\\', "/");
    let path = std::path::Path::new(file);
    if path.is_absolute() {
        return file.to_string();
    }
    std::path::Path::new(project_path)
        .join(normalized.trim_start_matches('/'))
        .to_string_lossy()
        .to_string()
}

fn squash_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn try_inline_caller_refresh(
    project_path: &str,
    params: &crate::csharp_compile::CompileParams,
    extra_references: &[String],
    access_caps: &crate::csharp_compile::AccessCaps,
    initial_patch_files: &[(String, String, String)],
    initial_inlined_keys: &[String],
) -> InlineCallerRefreshReport {
    let mut report = InlineCallerRefreshReport::default();
    let mut frontier: Vec<String> = initial_inlined_keys.to_vec();
    let mut seen_targets = BTreeSet::<String>::new();
    let mut refreshed_methods = BTreeSet::<String>::new();
    let mut refreshed_files = BTreeSet::<String>::new();
    // Carry the WHOLE initial batch — every changed file with its diff — into the
    // refresh compile, not just the inlined callees' own source files. A
    // force-detoured caller is recompiled here, so the compilation must see every
    // sibling the (still un-converged) batch introduced: e.g. an enum value or a
    // type/member appended in ANOTHER file of the same batch. Subsetting to the
    // inlined methods' files dropped those siblings, so the caller refresh failed
    // to compile (CS0117: 'Type' has no definition for the appended member) — which
    // silently defeated the un-inline and fell back to a queued recompile. The
    // full set is self-consistent (the main apply just compiled it); the extra
    // files only carry references — only the queried callers are force-detoured.
    let mut carry_files = {
        let mut files = BTreeMap::<String, InlineRefreshFile>::new();
        for (path, old_text, new_text) in initial_patch_files {
            files.insert(
                file_key(path),
                InlineRefreshFile {
                    path: path.clone(),
                    old_text: old_text.clone(),
                    new_text: new_text.clone(),
                    force_methods: BTreeSet::new(),
                },
            );
        }
        files
    };

    for depth in 0..INLINE_REFRESH_MAX_DEPTH {
        // A sidecar restart during refresh blanks the session registries; a
        // force-detoured caller recompiled against them could split a virtualized
        // field. Stop here and let the next hot reload converge via recompile.
        if super::session_registry_lost() {
            report.notes.push(
                "caller refresh stopped: compile-server session lost to a restart".to_string(),
            );
            break;
        }
        let targets: Vec<crate::csharp_compile::CallerQueryTarget> = frontier
            .iter()
            .filter(|key| seen_targets.insert((*key).clone()))
            .filter_map(|key| caller_query_target_from_unity_key(key))
            .collect();
        if targets.is_empty() {
            break;
        }

        let query = match crate::csharp_compile::query_callers(params, &targets).await {
            Ok(query) => query,
            Err(error) => {
                report.notes.push(format!(
                    "caller refresh skipped: caller index unavailable ({error})"
                ));
                break;
            }
        };

        let mut force_by_file: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut method_limit_hit = false;
        for target in &query.targets {
            if method_limit_hit {
                break;
            }
            if target.callers.len() > INLINE_REFRESH_MAX_CALLERS_PER_TARGET {
                report.notes.push(format!(
                    "caller refresh skipped {} caller(s) for {}.{} (limit {})",
                    target.callers.len(),
                    target.declaring_type,
                    target.member_name,
                    INLINE_REFRESH_MAX_CALLERS_PER_TARGET
                ));
                continue;
            }
            for caller in &target.callers {
                if caller.file.trim().is_empty() || caller.method_key.trim().is_empty() {
                    continue;
                }
                if refreshed_methods.len() >= INLINE_REFRESH_MAX_METHODS_TOTAL {
                    if !method_limit_hit {
                        report.notes.push(format!(
                            "caller refresh stopped at {} method(s) (limit {})",
                            refreshed_methods.len(),
                            INLINE_REFRESH_MAX_METHODS_TOTAL
                        ));
                    }
                    method_limit_hit = true;
                    break;
                }
                let absolute = resolve_caller_source_path(project_path, &caller.file);
                let fkey = file_key(&absolute);
                // Respect the file budget BEFORE claiming the method: a method
                // skipped for the file cap must not be recorded as refreshed,
                // or it would wrongly consume the method budget and suppress a
                // later, in-budget refresh of the same method.
                if !refreshed_files.contains(&fkey)
                    && refreshed_files.len() >= INLINE_REFRESH_MAX_FILES_TOTAL
                {
                    report.notes.push(format!(
                        "caller refresh stopped at {} file(s) (limit {})",
                        refreshed_files.len(),
                        INLINE_REFRESH_MAX_FILES_TOTAL
                    ));
                    continue;
                }
                let method_id = format!("{}::{}", fkey, caller.method_key);
                if !refreshed_methods.insert(method_id) {
                    continue;
                }
                refreshed_files.insert(fkey);
                force_by_file
                    .entry(absolute)
                    .or_default()
                    .insert(caller.method_key.clone());
            }
        }

        if force_by_file.is_empty() {
            break;
        }

        let forced_file_count = force_by_file.len();
        let forced_method_count: usize = force_by_file.values().map(BTreeSet::len).sum();
        let mut round_files = carry_files.clone();
        for (path, methods) in force_by_file {
            let current = match tokio::fs::read_to_string(&path).await {
                Ok(text) => text,
                Err(error) => {
                    report.notes.push(format!(
                        "caller refresh skipped {}: failed to read source ({error})",
                        display_project_path(project_path, &path)
                    ));
                    continue;
                }
            };
            let key = file_key(&path);
            let entry = round_files.entry(key).or_insert_with(|| InlineRefreshFile {
                path: path.clone(),
                old_text: current.clone(),
                new_text: current,
                force_methods: BTreeSet::new(),
            });
            entry.force_methods.extend(methods);
        }

        let files: Vec<(String, String, String)> = round_files
            .values()
            .map(|file| {
                (
                    file.path.clone(),
                    file.old_text.clone(),
                    file.new_text.clone(),
                )
            })
            .collect();
        let force_detours: Vec<crate::csharp_compile::ForceDetour> = round_files
            .values()
            .filter(|file| !file.force_methods.is_empty())
            .map(|file| crate::csharp_compile::ForceDetour {
                path: file.path.clone(),
                method_keys: file.force_methods.iter().cloned().collect(),
            })
            .collect();

        if files.is_empty() || force_detours.is_empty() {
            break;
        }

        let baseline_siblings = discover_partial_siblings(project_path, &files).await;
        let outcome = match crate::csharp_compile::compile_hot_patch(
            params,
            &files,
            &baseline_siblings,
            extra_references,
            Some(access_caps),
            &force_detours,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                report
                    .notes
                    .push(format!("caller refresh compile unavailable: {error}"));
                break;
            }
        };

        match outcome {
            crate::csharp_compile::HotPatchOutcome::Compiled {
                assembly_name,
                assembly_b64,
                assembly_path,
                methods,
                new_types,
                message_drivers,
                ..
            } => {
                if methods.is_empty() {
                    report
                        .notes
                        .push("caller refresh produced no detourable methods".to_string());
                    break;
                }
                match apply_compiled_hot_patch(
                    project_path,
                    params,
                    &assembly_name,
                    &assembly_b64,
                    assembly_path.as_ref(),
                    &methods,
                    &new_types,
                    &message_drivers,
                )
                .await
                {
                    Ok(applied) => {
                        report.rounds = depth + 1;
                        report.files += forced_file_count;
                        report.methods += forced_method_count;
                        carry_files = round_files;
                        if let Some(error) = applied.image_register_error {
                            report.notes.push(format!(
                                "caller refresh image registration failed: {error}; run unity_recompile before the next hot reload"
                            ));
                            break;
                        }
                        frontier = applied.inlined_method_keys;
                        if frontier.is_empty() {
                            break;
                        }
                    }
                    Err(error) => {
                        report.notes.push(format!(
                            "caller refresh patch failed: {}",
                            squash_line(&error)
                        ));
                        break;
                    }
                }
            }
            crate::csharp_compile::HotPatchOutcome::Cold { files } => {
                let reasons: Vec<String> = files
                    .iter()
                    .map(|(path, reasons)| {
                        format!(
                            "{}: {}",
                            display_project_path(project_path, path),
                            reasons.join("; ")
                        )
                    })
                    .collect();
                report.notes.push(format!(
                    "caller refresh stopped at cold verdict: {}",
                    reasons.join(" | ")
                ));
                break;
            }
            crate::csharp_compile::HotPatchOutcome::Noop { .. } => {
                report
                    .notes
                    .push("caller refresh found no effective caller detours".to_string());
                break;
            }
            crate::csharp_compile::HotPatchOutcome::CompileError(message) => {
                report.notes.push(format!(
                    "caller refresh compile error: {}",
                    squash_line(&message)
                ));
                break;
            }
        }
    }

    if !frontier.is_empty() && report.rounds >= INLINE_REFRESH_MAX_DEPTH {
        report.notes.push(format!(
            "caller refresh stopped at recursion depth {}",
            INLINE_REFRESH_MAX_DEPTH
        ));
    }
    report
}

async fn queue_inlined_method_files(
    project_path: &str,
    changed_keys: &[String],
    methods: &[crate::csharp_compile::HotPatchMethod],
    inlined_method_keys: &[String],
) -> usize {
    if inlined_method_keys.is_empty() {
        return 0;
    }
    let method_key_to_file: BTreeMap<String, String> = methods
        .iter()
        .filter(|method| !method.source_path.is_empty())
        .map(|method| (unity_method_key(method), file_key(&method.source_path)))
        .collect();
    let mut inlined_files: BTreeSet<String> = BTreeSet::new();
    let mut unmapped = false;
    for key in inlined_method_keys {
        match method_key_to_file.get(key) {
            Some(file) if !file.is_empty() => {
                inlined_files.insert(file.clone());
            }
            _ => unmapped = true,
        }
    }
    if unmapped || inlined_files.is_empty() {
        queue_cold_paths(project_path, changed_keys).await
    } else {
        let files: Vec<String> = inlined_files.into_iter().collect();
        queue_cold_paths(project_path, &files).await
    }
}

pub async fn pending_paths(project_path: &str) -> Vec<String> {
    let projects = projects().lock().await;
    match projects.get(&project_key(project_path)) {
        Some(state) => {
            let mut paths: Vec<String> = state
                .pending
                .values()
                .map(|edit| edit.absolute_path.clone())
                .collect();
            paths.sort();
            paths
        }
        None => Vec::new(),
    }
}

/// Locate the plugin's Locus.HotReload.Runtime.dll (field-store runtime,
/// M4) across the known install roots. Missing file → no extra reference;
/// field-store patches then fail with a deterministic compile diagnostic
/// that names the missing type, pointing at a plugin update.
fn hotreload_runtime_references(project_path: &str) -> Vec<String> {
    const INSTALL_DIRS: &[&str] = &[
        "Packages/com.farlocus.locus",
        "Assets/Locus",
        "Assets/Plugins/Locus",
    ];
    for dir in INSTALL_DIRS {
        let candidate = std::path::Path::new(project_path)
            .join(dir)
            .join("Editor/HotReload/Locus.HotReload.Runtime.dll");
        if candidate.is_file() {
            return vec![candidate.to_string_lossy().to_string()];
        }
    }
    Vec::new()
}

// ── probe ────────────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct HotReloadProbeResponse {
    #[serde(default)]
    detour_ok: bool,
    #[serde(default)]
    code_optimization: String,
    /// Whether entering Play Mode reloads the domain. `None` when the plugin
    /// predates the toggle (the field is absent), so the UI shows "unknown"
    /// rather than mistaking a missing field for "no reload".
    #[serde(default)]
    domain_reload_on_play: Option<bool>,
    #[serde(default)]
    error: String,
}

async fn run_probe(project_path: &str) -> Result<(), String> {
    let resp = crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_reload_probe",
        "",
        std::time::Duration::from_secs(10),
    )
    .await
    .map_err(|error| format!("Unity probe failed: {error}. Use unity_recompile instead."))?;

    if !resp.ok {
        let error = resp.error.unwrap_or_else(|| "probe failed".to_string());
        if error.starts_with("unknown message type") {
            return Err(
                "The Unity plugin in this project predates hot reload; update the Locus plugin \
                 (reopen the project from Locus) or use unity_recompile."
                    .to_string(),
            );
        }
        return Err(format!(
            "Unity probe failed: {error}. Use unity_recompile instead."
        ));
    }

    let message = resp.message.unwrap_or_default();
    let probe: HotReloadProbeResponse = serde_json::from_str(&message)
        .map_err(|error| format!("Unity probe response parse failed: {error}"))?;

    // Code Optimization is informational now (Release-first). The editor may be
    // in Release, where Mono inlines some small methods past the detour; the
    // apply path detects those per method and converges them with a recompile,
    // so the probe no longer blocks on Release. The detour self-test below is
    // the real capability gate.
    if !probe.detour_ok {
        return Err(format!(
            "The detour engine self-test failed in this editor ({}); use unity_recompile.",
            if probe.error.is_empty() {
                "no detail"
            } else {
                &probe.error
            }
        ));
    }
    Ok(())
}

// ── enable-time Code Optimization gate ───────────────────────────────

/// Read the connected editor's Code Optimization for the toggle-time gate
/// (the icon above the chat input and the Settings switch both call this
/// before turning hot reload on). Reuses `hot_reload_probe`, which every
/// hot-reload-capable plugin already answers, so the warning works even
/// before the in-project plugin is updated.
///
/// Returns `(connected, code_optimization)`. The editor being unreachable →
/// `(false, None)`; a connected editor whose probe could not be parsed (an
/// older plugin shape) → `(true, None)`. The caller only blocks the toggle on
/// a positive `Some("release")`; every other case enables directly, because
/// the execution-time `run_probe` still gates real hot reloads.
pub async fn detect_code_optimization(project_path: &str) -> (bool, Option<String>) {
    let resp = match crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_reload_probe",
        "",
        std::time::Duration::from_secs(10),
    )
    .await
    {
        Ok(resp) => resp,
        // No connected editor (or the pipe timed out): nothing to gate on now.
        Err(_) => return (false, None),
    };

    if !resp.ok {
        // Connected, but the probe errored (e.g. a plugin predating it).
        return (true, None);
    }

    let message = resp.message.unwrap_or_default();
    match serde_json::from_str::<HotReloadProbeResponse>(&message) {
        Ok(probe) if !probe.code_optimization.is_empty() => (true, Some(probe.code_optimization)),
        _ => (true, None),
    }
}

fn normalize_code_optimization(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "debug" => Some("debug"),
        "release" => Some("release"),
        _ => None,
    }
}

async fn send_code_optimization_request(
    project_path: &str,
    message_type: &str,
    payload: &str,
    desired: &str,
) -> Result<String, String> {
    let resp = crate::unity_bridge::send_message_with_timeout(
        project_path,
        message_type,
        payload,
        std::time::Duration::from_secs(15),
    )
    .await
    .map_err(|error| {
        format!("Could not reach the Unity editor to change Code Optimization: {error}")
    })?;

    if !resp.ok {
        let error = resp
            .error
            .unwrap_or_else(|| "Unity rejected the Code Optimization change".to_string());
        if error == "domain_reload_interrupted" {
            return Ok(desired.to_string());
        }
        if error.starts_with("unknown message type") {
            return Err(format!(
                "The Unity plugin in this project predates the Code Optimization auto-switch. \
                 Update the Locus plugin (reopen the project from Locus), or set Code \
                 Optimization to {desired} yourself from the Unity status bar."
            ));
        }
        return Err(format!(
            "Unity could not switch Code Optimization to {desired}: {error}"
        ));
    }

    let message = resp.message.unwrap_or_default();
    let parsed: HotReloadProbeResponse = serde_json::from_str(&message)
        .map_err(|error| format!("Code Optimization response parse failed: {error}"))?;
    Ok(if parsed.code_optimization.is_empty() {
        desired.to_string()
    } else {
        parsed.code_optimization
    })
}

/// Switch the connected editor to the requested Code Optimization. Triggers a
/// script recompile in Unity, exactly like flipping the status-bar bug icon.
/// Returns the resulting value reported by Unity.
pub async fn set_code_optimization(project_path: &str, desired: &str) -> Result<String, String> {
    let desired = normalize_code_optimization(desired)
        .ok_or_else(|| "Code Optimization must be 'debug' or 'release'".to_string())?;
    match send_code_optimization_request(
        project_path,
        "hot_reload_set_code_optimization",
        desired,
        desired,
    )
    .await
    {
        Ok(value) => Ok(value),
        Err(error) if desired == "debug" && error.contains("predates") => {
            send_code_optimization_request(project_path, "hot_reload_set_debug", "", desired).await
        }
        Err(error) => Err(error),
    }
}

/// Switch the connected editor to Code Optimization = Debug (the optional
/// toggle-time auto-fix the user confirmed).
pub async fn set_code_optimization_debug(project_path: &str) -> Result<String, String> {
    set_code_optimization(project_path, "debug").await
}

// ── Enter-Play-Mode domain reload (manual popover toggle) ────────────

/// One probe, both editor settings the hot-reload popover surfaces: Code
/// Optimization and the Play-Mode domain-reload flag. Mirrors
/// `detect_code_optimization`'s lenient contract — an unreachable editor →
/// `(false, None, None)`; a connected editor whose probe could not be parsed
/// (an older plugin) → `(true, None, None)`. Both settings are read off the
/// single `hot_reload_probe` round-trip so the popover never double-probes.
pub async fn detect_hot_reload_editor_settings(
    project_path: &str,
) -> (bool, Option<String>, Option<bool>) {
    let resp = match crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_reload_probe",
        "",
        std::time::Duration::from_secs(10),
    )
    .await
    {
        Ok(resp) => resp,
        Err(_) => return (false, None, None),
    };

    if !resp.ok {
        return (true, None, None);
    }

    let message = resp.message.unwrap_or_default();
    match serde_json::from_str::<HotReloadProbeResponse>(&message) {
        Ok(probe) => (
            true,
            if probe.code_optimization.is_empty() {
                None
            } else {
                Some(probe.code_optimization)
            },
            probe.domain_reload_on_play,
        ),
        _ => (true, None, None),
    }
}

/// Set whether entering Play Mode reloads the managed domain
/// (EditorSettings.enterPlayModeOptions / DisableDomainReload). Unlike a Code
/// Optimization switch this does NOT trigger a recompile — it only flips the
/// editor setting. Returns the resulting effective value reported by Unity.
pub async fn set_play_mode_reload(project_path: &str, domain_reload: bool) -> Result<bool, String> {
    let payload = serde_json::json!({ "domain_reload_on_play": domain_reload }).to_string();
    let resp = crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_reload_set_play_mode_reload",
        &payload,
        std::time::Duration::from_secs(10),
    )
    .await
    .map_err(|error| {
        format!("Could not reach the Unity editor to change Play Mode domain reload: {error}")
    })?;

    if !resp.ok {
        let error = resp
            .error
            .unwrap_or_else(|| "Unity rejected the Play Mode domain-reload change".to_string());
        if error.starts_with("unknown message type") {
            return Err(
                "The Unity plugin in this project predates the Play Mode domain-reload toggle. \
                 Update the Locus plugin (reopen the project from Locus)."
                    .to_string(),
            );
        }
        return Err(format!(
            "Unity could not change Play Mode domain reload: {error}"
        ));
    }

    let message = resp.message.unwrap_or_default();
    let parsed: HotReloadProbeResponse = serde_json::from_str(&message)
        .map_err(|error| format!("Play Mode domain-reload response parse failed: {error}"))?;
    Ok(parsed.domain_reload_on_play.unwrap_or(domain_reload))
}

// ── access probe (C0 runtime capability matrix) ─────────────────────

/// Measure the editor's Mono access-check matrix: the sidecar compiles the
/// fixed probe assembly against the project's reference set, Unity loads it,
/// force-JITs every cell, and runs the three reflection/emit primitives.
/// Returns the parsed caps plus the raw Unity matrix (per-cell errors).
async fn run_access_probe(
    project_path: &str,
    params: &crate::csharp_compile::CompileParams,
) -> Result<(crate::csharp_compile::AccessCaps, serde_json::Value), String> {
    let (assembly_b64, cells) = crate::csharp_compile::compile_access_probe(params).await?;

    // Cell descriptors pass through verbatim (the sidecar already emits the
    // lowercase method/op/visibility keys JsonUtility expects).
    let payload = serde_json::json!({
        "assembly_b64": assembly_b64,
        "cells": cells,
    })
    .to_string();

    let resp = crate::unity_bridge::send_message_with_timeout(
        project_path,
        "hot_reload_access_probe",
        &payload,
        std::time::Duration::from_secs(30),
    )
    .await
    .map_err(|error| format!("Unity access probe failed: {error}"))?;

    if !resp.ok {
        let error = resp
            .error
            .unwrap_or_else(|| "access probe failed".to_string());
        if error.starts_with("unknown message type") {
            return Err(
                "the Unity plugin in this project predates the access probe; update the Locus plugin"
                    .to_string(),
            );
        }
        return Err(format!("Unity access probe failed: {error}"));
    }

    let message = resp.message.unwrap_or_default();
    let matrix: serde_json::Value = serde_json::from_str(&message)
        .map_err(|error| format!("access probe response parse failed: {error}"))?;
    Ok((parse_access_caps(&matrix), matrix))
}

/// Unity matrix (`{cells:[{op,visibility,ok,error}], primitives:{...}}`,
/// snake_case from JsonUtility) → AccessCaps. Anything missing reads as
/// false — conservative by construction.
fn parse_access_caps(matrix: &serde_json::Value) -> crate::csharp_compile::AccessCaps {
    let mut caps = crate::csharp_compile::AccessCaps::default();
    let primitive = |name: &str| {
        matrix
            .get("primitives")
            .and_then(|p| p.get(name))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    caps.create_delegate_non_public = primitive("create_delegate_non_public");
    caps.dynamic_method_skip_visibility = primitive("dynamic_method_skip_visibility");
    caps.dynamic_method_byref_return = primitive("dynamic_method_byref_return");

    if let Some(cells) = matrix.get("cells").and_then(|v| v.as_array()) {
        for cell in cells {
            let op = cell.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let visibility = cell
                .get("visibility")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if op.is_empty() || visibility.is_empty() {
                continue;
            }
            let ok = cell.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            caps.cells.insert(format!("{op}_{visibility}"), ok);
        }
    }
    caps
}

async fn cached_access_probe(
    project_path: &str,
    domain_generation: &str,
) -> Option<AccessProbeCacheEntry> {
    let projects = projects().lock().await;
    projects.get(&project_key(project_path)).and_then(|state| {
        state
            .access_probe
            .as_ref()
            .filter(|entry| entry.domain_generation == domain_generation)
            .cloned()
    })
}

async fn store_access_probe(project_path: &str, entry: AccessProbeCacheEntry) {
    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    state.access_probe = Some(entry);
}

/// Caps for this (project, domain generation), probing at most once per
/// generation. NEVER fails: any probe error logs, caches conservative
/// all-false caps (so a persistently failing probe — old plugin, busy
/// editor — cannot retry-storm every hot reload) and lets the hot path
/// proceed exactly as today.
async fn ensure_access_caps(
    project_path: &str,
    params: &crate::csharp_compile::CompileParams,
) -> crate::csharp_compile::AccessCaps {
    if let Some(entry) = cached_access_probe(project_path, &params.domain_generation).await {
        return entry.caps;
    }
    match run_access_probe(project_path, params).await {
        Ok((caps, matrix)) => {
            let summary = caps.cells.values().filter(|ok| **ok).count();
            eprintln!(
                "[HotReload] access probe: {}/{} cells pass, primitives [{}, {}, {}]",
                summary,
                caps.cells.len(),
                caps.create_delegate_non_public,
                caps.dynamic_method_skip_visibility,
                caps.dynamic_method_byref_return,
            );
            store_access_probe(
                project_path,
                AccessProbeCacheEntry {
                    domain_generation: params.domain_generation.clone(),
                    caps: caps.clone(),
                    matrix,
                },
            )
            .await;
            caps
        }
        Err(error) => {
            eprintln!("[HotReload] access probe failed; using conservative caps: {error}");
            let caps = crate::csharp_compile::AccessCaps::default();
            store_access_probe(
                project_path,
                AccessProbeCacheEntry {
                    domain_generation: params.domain_generation.clone(),
                    caps: caps.clone(),
                    matrix: serde_json::json!({ "error": error }),
                },
            )
            .await;
            caps
        }
    }
}

/// Diagnostic entry for the `unity_hot_reload_access_probe_run` command:
/// the full matrix for the project's CURRENT domain generation. Unlike the
/// hot-reload path this propagates probe errors (and does not cache them),
/// so a verification run always sees the real failure.
pub async fn access_probe_run(project_path: &str) -> Result<serde_json::Value, String> {
    if !crate::csharp_compile::is_enabled() {
        return Err(
            "the sidecar compiler is disabled; enable it in Settings > Code Analysis".to_string(),
        );
    }

    let op_lock = crate::unity_bridge::project_unity_op_lock(project_path).await;
    let _op_guard = op_lock.lock().await;

    let params = crate::csharp_compile::params::get_params(project_path)
        .await
        .map_err(|error| format!("could not get compile params from Unity: {error}"))?;

    if let Some(entry) = cached_access_probe(project_path, &params.domain_generation).await {
        return Ok(serde_json::json!({
            "cached": true,
            "domainGeneration": entry.domain_generation,
            "caps": entry.caps,
            "matrix": entry.matrix,
        }));
    }

    let (caps, matrix) = run_access_probe(project_path, &params).await?;
    store_access_probe(
        project_path,
        AccessProbeCacheEntry {
            domain_generation: params.domain_generation.clone(),
            caps: caps.clone(),
            matrix: matrix.clone(),
        },
    )
    .await;
    Ok(serde_json::json!({
        "cached": false,
        "domainGeneration": params.domain_generation,
        "caps": caps,
        "matrix": matrix,
    }))
}

// ── hot reload orchestration ─────────────────────────────────────────

/// Outcome text for the `unity_hot_reload` tool. `Err` carries agent-facing
/// errors (compile diagnostics, gating guidance).
pub async fn hot_reload(
    project_path: &str,
    path_filter: Option<Vec<String>>,
) -> Result<String, String> {
    if !super::is_enabled() {
        return Err(
            "Unity hot reload is disabled. Enable it in Settings > Code Analysis (requires the \
             sidecar compiler), or use unity_recompile."
                .to_string(),
        );
    }
    if !crate::csharp_compile::is_enabled() {
        return Err(
            "Unity hot reload requires the sidecar compiler, which is disabled. Enable it in \
             Settings > Code Analysis, or use unity_recompile."
                .to_string(),
        );
    }

    let op_lock = crate::unity_bridge::project_unity_op_lock(project_path).await;
    let _op_guard = op_lock.lock().await;

    // Snapshot the pending set for this project (filtered when asked).
    let filter: Option<BTreeSet<String>> = path_filter.map(|paths| {
        paths
            .iter()
            .map(|path| {
                if std::path::Path::new(path).is_absolute() {
                    file_key(path)
                } else {
                    file_key(&format!(
                        "{}/{}",
                        project_path.trim_end_matches(['/', '\\']),
                        path
                    ))
                }
            })
            .collect()
    });

    let edits: Vec<(String, PendingEdit)> = {
        let projects = projects().lock().await;
        match projects.get(&project_key(project_path)) {
            Some(state) => state
                .pending
                .iter()
                .filter(|(key, _)| filter.as_ref().map_or(true, |f| f.contains(*key)))
                .map(|(key, edit)| (key.clone(), edit.clone()))
                .collect(),
            None => Vec::new(),
        }
    };

    if edits.is_empty() {
        return Ok(
            "No pending .cs edits tracked for this session. Edit files with the write/edit tools \
             first; for changes made outside this session use unity_recompile."
                .to_string(),
        );
    }

    run_probe(project_path).await?;

    // Read current disk text; skip files that returned to their baseline.
    let mut files: Vec<(String, String, String)> = Vec::new(); // (path, old, new)
    let mut changed_keys: Vec<String> = Vec::new();
    let mut changed_current_texts: Vec<(String, String)> = Vec::new();
    for (key, edit) in &edits {
        let current = match tokio::fs::read_to_string(&edit.absolute_path).await {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Deleted since the edit: enter the batch as an empty file —
                // the sidecar classifies the type deletions (M5/H7e) and
                // either hot-deletes or queues a precise cold reason.
                String::new()
            }
            Err(error) => {
                return Err(format!("Failed to read {}: {error}", edit.absolute_path));
            }
        };
        if current == edit.baseline {
            continue;
        }
        files.push((edit.absolute_path.clone(), edit.baseline.clone(), current));
        changed_keys.push(key.clone());
        changed_current_texts.push((key.clone(), files.last().unwrap().2.clone()));
    }

    if files.is_empty() {
        return Ok(
            "All tracked edits are back at their compiled baseline; nothing to hot reload."
                .to_string(),
        );
    }

    // A prior request already saw the sidecar restart with patches live: its
    // session registries are gone, so a hot apply could split a virtualized
    // field's value. Converge through unity_recompile instead of compiling
    // against the blank registry (which would also fail confusingly on any
    // cross-patch reference the lost ImageRegistry can no longer resolve).
    if super::session_registry_lost() {
        return Ok(converge_after_session_loss(project_path, &changed_keys).await);
    }

    let params = crate::csharp_compile::params::get_params(project_path)
        .await
        .map_err(|error| {
            format!("Could not get compile params from Unity ({error}); use unity_recompile.")
        })?;

    // The generation/serial trackers are maintained solely by
    // `observe_reload_state` (the monitor samples every poll while connected),
    // so a transient pipe drop within one domain is judged "unchanged" there
    // and never mistaken for a reload — no need to record the generation here.

    // C0: per-domain-generation runtime capability matrix (Mono JIT access
    // checks + emit primitives), probed once before the generation's first
    // compile. Non-fatal: failures degrade to conservative all-false caps.
    // C2′ gates non-public lowering on it; this stage only feeds it through
    // (the sidecar echoes it in the verdict).
    let access_caps = ensure_access_caps(project_path, &params).await;

    // M4: patch assemblies reference the field-store runtime shipped with
    // the plugin (Unity's own reference set only carries script assemblies).
    let extra_references = hotreload_runtime_references(project_path);

    // B6: when the batch declares partial types, ship the candidate sibling
    // part files (grep-grade match; the sidecar parses and keeps only real
    // parts) so the patch can re-declare the COMPLETE type.
    let baseline_siblings = discover_partial_siblings(project_path, &files).await;

    let started = std::time::Instant::now();
    let outcome = match crate::csharp_compile::compile_hot_patch(
        &params,
        &files,
        &baseline_siblings,
        &extra_references,
        Some(&access_caps),
        &[],
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            record_patch_failure(project_path).await;
            return Err(format!(
                "Compile server unavailable ({error}); use unity_recompile."
            ));
        }
    };

    match outcome {
        crate::csharp_compile::HotPatchOutcome::Cold { files: cold_files } => {
            let mut lines = vec![
                "Hot reload not applicable — these edits change structure (signatures, fields, \
                 types), which needs a real compile:"
                    .to_string(),
            ];
            for (path, reasons) in &cold_files {
                lines.push(format!("  {}: {}", path, reasons.join("; ")));
            }
            let queued = queue_cold_paths(project_path, &changed_keys).await;
            crate::csharp_compile::emit_status_in_background();
            lines.push(format!(
                "Run unity_recompile to apply them ({queued} file(s) queued). Hot-applied edits \
                 from earlier patches stay live until then."
            ));
            Ok(lines.join("\n"))
        }
        crate::csharp_compile::HotPatchOutcome::Noop {
            deletions_noted,
            caller_scan_note,
        } => {
            mark_changed_keys_applied(project_path, &changed_current_texts).await;
            crate::csharp_compile::emit_status_in_background();
            if deletions_noted == 0 {
                return Ok(
                    "No effective code change (comments/formatting only); nothing to hot reload."
                        .to_string(),
                );
            }
            let mut summary = format!(
                "Deletion applied: {deletions_noted} removed member(s) recorded. The loaded code \
                 was already correct (the members are unreachable); later hot patches referencing \
                 them will fail with a pointed error until unity_recompile converges."
            );
            if let Some(note) = caller_scan_note {
                summary.push_str(&format!("\nCall-site check: {note}."));
            }
            Ok(summary)
        }
        crate::csharp_compile::HotPatchOutcome::CompileError(message) => Err(message),
        crate::csharp_compile::HotPatchOutcome::Compiled {
            assembly_name,
            assembly_b64,
            assembly_path,
            methods,
            new_types,
            message_drivers,
            caller_scan_note,
        } => {
            // A patch that ONLY adds a new Unity PlayerLoop message has no
            // detour and no new type, but it must still be loaded so the pump
            // shim is in the domain and can be driven — so it is not a no-op.
            if methods.is_empty() && new_types.is_empty() && message_drivers.is_empty() {
                // Compiled-but-nothing-detourable means the batch ONLY adds
                // new surface (methods / enum members): the patch is not
                // loaded, because nothing in the running domain can reach it
                // yet. Comment-only edits never get here (sidecar noop).
                return Ok(
                    "No detourable change: the edit only adds new surface. It becomes live when \
                     a later hot edit references it (edit a call site and hot reload again — the \
                     batch re-sends the addition together with the caller), or at the next \
                     unity_recompile."
                        .to_string(),
                );
            }

            // The sidecar can have crashed and respawned during this very call's
            // get_params / probe / compile (after the gate above), blanking the
            // registries — so this freshly built assembly may carry a new,
            // value-splitting field store. Re-check before shipping it to Unity.
            if super::session_registry_lost() {
                return Ok(converge_after_session_loss(project_path, &changed_keys).await);
            }

            let applied = match apply_compiled_hot_patch(
                project_path,
                &params,
                &assembly_name,
                &assembly_b64,
                assembly_path.as_ref(),
                &methods,
                &new_types,
                &message_drivers,
            )
            .await
            {
                Ok(applied) => applied,
                Err(error) => {
                    queue_cold_paths(project_path, &changed_keys).await;
                    crate::csharp_compile::emit_status_in_background();
                    return Err(error);
                }
            };

            mark_changed_keys_applied(project_path, &changed_current_texts).await;

            let engine = applied.engine;
            let inlined_method_keys = applied.inlined_method_keys;
            let inlined_sources = applied.inlined_sources;
            let image_register_error = applied.image_register_error;
            let pump_skipped_count = applied.pump_skipped_count;
            let pump_skipped_messages = applied.pump_skipped_messages;

            // Phase D rollout observability: count the force-evaluate-attributable
            // classifications (StubInlined = the force-JIT stub moved Mono's bit;
            // without the feature these fall to the static heuristic). An upper
            // bound on the over-refresh the feature adds — logged so a gray rollout
            // can weigh that cost against the tighter "is it live" reporting it buys.
            let stub_inlined = inlined_sources
                .iter()
                .filter(|source| source.as_str() == "StubInlined")
                .count();
            if stub_inlined > 0 {
                eprintln!(
                    "[HotReload] inline force-evaluate: {stub_inlined}/{} inlined method(s) classified via the force-JIT stub this apply",
                    inlined_method_keys.len()
                );
            }

            // Route inlined methods to the same convergence path Locus uses for
            // any non-hot-safe change, but queue only the source file(s) whose
            // methods Unity reported as inlined. Fall back to the batch if a
            // method key cannot be mapped.
            if !inlined_method_keys.is_empty() {
                queue_inlined_method_files(
                    project_path,
                    &changed_keys,
                    &methods,
                    &inlined_method_keys,
                )
                .await;
                crate::csharp_compile::emit_status_in_background();
            }

            let inline_refresh = if !inlined_method_keys.is_empty() {
                Some(
                    try_inline_caller_refresh(
                        project_path,
                        &params,
                        &extra_references,
                        &access_caps,
                        &files,
                        &inlined_method_keys,
                    )
                    .await,
                )
            } else {
                None
            };

            let stub_count = methods.iter().filter(|m| m.is_stub).count();
            let mut summary = format!(
                "Hot reload applied in {} ms: {} method(s) redirected across {} file(s)",
                started.elapsed().as_millis(),
                methods.len(),
                files.len(),
            );
            if !new_types.is_empty() {
                summary.push_str(&format!(", {} new type(s) loaded", new_types.len()));
            }
            if !message_drivers.is_empty() {
                // Hard failures already fail closed above, so the remainder is driven
                // (pump/proxy/once-off catch-up) or benign-skipped. Report each
                // skipped message by name (when the plugin sent them), and surface
                // each distinct caveat note once so the agent understands why a driven
                // message may not match native behavior.
                let notes: Vec<&str> = message_drivers.iter().map(|d| d.note.as_str()).collect();
                summary.push_str(&message_driver_summary(
                    message_drivers.len(),
                    pump_skipped_count,
                    &pump_skipped_messages,
                    &notes,
                ));
            }
            if !engine.is_empty() {
                summary.push_str(&format!(" (engine: {engine})"));
            }
            if stub_count > 0 {
                summary.push_str(&format!(
                    ".\n{stub_count} deleted Unity message method(s) now detour to empty stubs — \
                     the behavior stops immediately; reflection/SendMessage/UnityEvent string \
                     bindings to deleted members cannot be verified and only converge at \
                     unity_recompile"
                ));
            }
            if let Some(note) = &caller_scan_note {
                summary.push_str(&format!(".\nCall-site check: {note}"));
            }
            if let Some(error) = &image_register_error {
                summary.push_str(&format!(
                    ".\nSidecar image registration failed: {error}. Run unity_recompile before the next hot reload."
                ));
            }
            if let Some(refresh) = &inline_refresh {
                if refresh.methods > 0 {
                    summary.push_str(&format!(
                        ".\nInline caller refresh patched {} caller method(s) across {} file(s) in {} round(s); unity_recompile is still queued for convergence.",
                        refresh.methods, refresh.files, refresh.rounds
                    ));
                }
                for note in &refresh.notes {
                    summary.push_str(&format!("\nInline caller refresh: {note}."));
                }
            }
            if inlined_method_keys.is_empty() {
                summary.push_str(
                    ".\nChanges are live in the running Editor — no recompile, no domain reload, \
                     state preserved. The files are on disk, so the next unity_recompile or domain \
                     reload makes them permanent automatically.",
                );
            } else {
                // Release-first honesty: Mono inlined some originals, so the detour
                // is bypassed at their inlined call sites and those edits are NOT
                // live yet. Report tersely — names (tagged by the confidence the
                // plugin reported) + the one action that matters. (Keep the exact
                // phrase "inlined in Release"; the self-test keys on it.)
                let names = annotate_inlined_names(&inlined_method_keys, &inlined_sources);
                let playing = {
                    let (connected, status, _) =
                        crate::unity_bridge::query_unity_status(project_path).await;
                    connected && crate::unity_bridge::is_play_mode_status(status)
                };
                let action = if playing {
                    "exit Play Mode or run unity_recompile (exit reloads the domain, dropping \
                     play-mode state), or switch Code Optimization to Debug"
                } else {
                    "run unity_recompile, or switch Code Optimization to Debug"
                };
                if inline_refresh
                    .as_ref()
                    .map(|refresh| refresh.methods > 0)
                    .unwrap_or(false)
                {
                    summary.push_str(&format!(
                        ".\n{} method(s) inlined in Release; project caller refresh was attempted for: {}. To converge fully: {}.",
                        names.len(),
                        names.join(", "),
                        action,
                    ));
                } else {
                    summary.push_str(&format!(
                        ".\n{} method(s) inlined in Release — NOT live yet: {}. To apply: {}.",
                        names.len(),
                        names.join(", "),
                        action,
                    ));
                }
            }

            Ok(summary)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_driver_gate_passes_when_supported_and_clean() {
        // No added messages → always Ok, regardless of support / failures.
        assert!(message_driver_gate(false, false, 0).is_ok());
        // Added messages, plugin supports them, none failed → Ok.
        assert!(message_driver_gate(true, true, 0).is_ok());
    }

    #[test]
    fn message_driver_gate_fails_closed_when_plugin_cannot_drive() {
        // Drivers sent but the plugin predates pump support → fail closed.
        let err = message_driver_gate(true, false, 0).unwrap_err();
        assert!(err.contains("can't drive"), "{err}");
        assert!(err.contains("unity_recompile"), "{err}");
    }

    #[test]
    fn message_driver_gate_fails_closed_on_reported_wiring_failures() {
        // pump_failed_count > 0 fails closed even when the plugin supports driving.
        let err = message_driver_gate(true, true, 2).unwrap_err();
        assert!(err.contains('2'), "{err}");
        assert!(err.contains("could not be wired"), "{err}");
    }

    #[test]
    fn pump_capability_cache_roundtrips_and_overwrites() {
        // Unique key so the process-global cache can't collide with other tests.
        let gen = "test-gen-pump-capability-roundtrip";
        assert_eq!(known_pump_capability(gen), None); // not learned yet
        remember_pump_capability(gen, false);
        assert_eq!(known_pump_capability(gen), Some(false)); // learned: unsupported → preflight fails closed
        remember_pump_capability(gen, true);
        assert_eq!(known_pump_capability(gen), Some(true)); // re-learned: supported
    }

    #[test]
    fn message_driver_summary_reports_driven_and_named_skips() {
        let skipped = vec![
            "OnTriggerEnter — parameter is not UnityEngine.Collider (a same-named custom type) — left as a plain method".to_string(),
            "Awake — no live instance to catch up (new instances need a recompile)".to_string(),
        ];
        // A note duplicated across drivers must appear once; the empty note is dropped.
        let notes = vec![
            "Update runs after native scripts.",
            "Update runs after native scripts.",
            "",
        ];
        let s = message_driver_summary(3, 2, &skipped, &notes); // 3 total, 2 skipped → 1 driven
        assert!(
            s.contains("1 added Unity message(s) wired to a runtime driver"),
            "{s}"
        );
        assert!(s.contains("2 added Unity message(s) not driven"), "{s}");
        assert!(s.contains("OnTriggerEnter"), "{s}");
        assert!(s.contains("Awake"), "{s}");
        assert_eq!(
            s.matches("Update runs after native scripts.").count(),
            1,
            "{s}"
        );
    }

    #[test]
    fn message_driver_summary_all_driven_has_no_skip_clause() {
        let s = message_driver_summary(2, 0, &[], &["", ""]);
        assert!(
            s.contains("2 added Unity message(s) wired to a runtime driver"),
            "{s}"
        );
        assert!(!s.contains("not driven"), "{s}");
    }

    #[test]
    fn message_driver_summary_falls_back_to_count_without_names() {
        // Older plugin: count > 0 but no names → the generic reason clause, no driven.
        let s = message_driver_summary(1, 1, &[], &[""]);
        assert!(
            s.contains("1 added Unity message(s) not driven (not an engine message"),
            "{s}"
        );
        assert!(!s.contains("wired to a runtime driver"), "{s}");
    }

    #[test]
    fn message_driver_summary_caps_named_skips() {
        let skipped: Vec<String> = (0..9).map(|i| format!("OnMsg{i} — reason")).collect();
        let s = message_driver_summary(9, 9, &skipped, &[]);
        assert!(s.contains("OnMsg0") && s.contains("OnMsg5"), "{s}"); // first 6 shown
        assert!(!s.contains("OnMsg6"), "{s}"); // 7th onward capped
        assert!(s.contains("(+3 more)"), "{s}");
    }

    #[test]
    fn message_driver_summary_empty_when_no_messages() {
        assert_eq!(message_driver_summary(0, 0, &[], &[]), "");
    }

    #[test]
    fn partial_decl_names_are_grep_grade() {
        let mut names = BTreeSet::new();
        collect_partial_type_names(
            "public partial class Foo { }\n\
             internal sealed partial   struct Bar { }\n\
             partial interface IBaz<T> { }\n\
             partial class @Quux { }\n\
             // a comment that says partial class NotADecl is fine: regex still grabs it\n\
             class Plain { }",
            &mut names,
        );
        assert!(names.contains("Foo"));
        assert!(names.contains("Bar"));
        assert!(names.contains("IBaz"));
        assert!(names.contains("Quux"));
        assert!(!names.contains("Plain"));

        let mut none = BTreeSet::new();
        collect_partial_type_names(
            "class A { int partial; } // identifier, not a decl",
            &mut none,
        );
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn inlined_names_annotate_by_reported_confidence() {
        let keys = vec![
            "Game.Foo|Bar|Int32|s".to_string(),
            "Game.Foo|Baz||i".to_string(),
            "Game.Foo|Qux|String|s".to_string(),
        ];
        let sources = vec![
            "RuntimeInlined".to_string(),
            "StubInlined".to_string(),
            "Predicted".to_string(),
        ];
        let names = annotate_inlined_names(&keys, &sources);
        assert_eq!(names[0], "Game.Foo.Bar (confirmed)");
        assert_eq!(names[1], "Game.Foo.Baz (high-confidence)");
        assert_eq!(names[2], "Game.Foo.Qux (predicted)");

        // Older plugin: no sources → bare names, no panic on the length gap.
        let bare = annotate_inlined_names(&keys, &[]);
        assert_eq!(bare, vec!["Game.Foo.Bar", "Game.Foo.Baz", "Game.Foo.Qux"]);

        // Unknown/empty source string also falls back to the bare name.
        let unknown = annotate_inlined_names(&keys[..1], &["Mystery".to_string()]);
        assert_eq!(unknown, vec!["Game.Foo.Bar"]);
    }

    #[test]
    fn inline_method_keys_roundtrip_to_caller_query_targets() {
        let method = crate::csharp_compile::HotPatchMethod {
            declaring_type: "Game.Runtime.Foo+Bar".to_string(),
            patch_declaring_type: "__LocusHotPatch.Foo_Bar".to_string(),
            name: "Answer".to_string(),
            param_type_names: vec!["Int32".to_string(), "String".to_string()],
            param_type_sigs: vec!["System.Int32".to_string(), "System.String".to_string()],
            is_static: true,
            is_ctor: false,
            source_path: r"F:\Game\Assets\Foo.cs".to_string(),
            original_assembly: Some("Assembly-CSharp".to_string()),
            is_stub: false,
        };

        let key = unity_method_key(&method);
        assert_eq!(key, "Game.Runtime.Foo+Bar|Answer|Int32,String|s");

        let target = caller_query_target_from_unity_key(&key).expect("target");
        assert_eq!(target.declaring_type, "Game.Runtime.Foo+Bar");
        assert_eq!(target.member_name, "Answer");
        assert!(caller_query_target_from_unity_key("Game.Foo|.ctor||i").is_none());
        assert!(caller_query_target_from_unity_key("Game.Foo|<Generated>||s").is_none());
    }

    #[tokio::test]
    async fn partial_sibling_discovery_scans_closure_and_skips_changed_files() {
        let dir = std::env::temp_dir().join(format!(
            "locus-partial-discovery-{}",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos() as u64
        ));
        let assets = dir.join("Assets");
        std::fs::create_dir_all(assets.join("Sub")).unwrap();

        let changed_path = assets.join("PartA.cs");
        std::fs::write(
            &changed_path,
            "public partial class Split { int A() { return 1; } }",
        )
        .unwrap();
        // Direct sibling of the edited type.
        std::fs::write(
            assets.join("Sub").join("PartB.cs"),
            "public partial class Split { int B() { return 2; } }\npublic partial class Chained { }",
        )
        .unwrap();
        // Pulled in only through the closure (Chained is declared by PartB).
        std::fs::write(
            assets.join("ChainedPart.cs"),
            "public partial class Chained { int C() { return 3; } }",
        )
        .unwrap();
        // Unrelated partial type: stays out.
        std::fs::write(
            assets.join("Other.cs"),
            "public partial class Unrelated { }",
        )
        .unwrap();
        // No partial at all: never a candidate.
        std::fs::write(assets.join("Plain.cs"), "public class Plain { }").unwrap();

        let project = dir.to_string_lossy().to_string();
        let files = vec![(
            changed_path.to_string_lossy().to_string(),
            "public partial class Split { int A() { return 0; } }".to_string(),
            "public partial class Split { int A() { return 1; } }".to_string(),
        )];

        let siblings = discover_partial_siblings(&project, &files).await;
        let names: BTreeSet<String> = siblings
            .iter()
            .map(|(path, _)| {
                std::path::Path::new(path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert!(names.contains("PartB.cs"), "{names:?}");
        assert!(
            names.contains("ChainedPart.cs"),
            "closure must follow Chained: {names:?}"
        );
        assert!(!names.contains("Other.cs"), "{names:?}");
        assert!(!names.contains("Plain.cs"), "{names:?}");
        assert!(
            !names.contains("PartA.cs"),
            "the changed file is not its own sibling: {names:?}"
        );

        // A batch with no partial declarations never scans.
        let plain_files = vec![(
            changed_path.to_string_lossy().to_string(),
            "class X { }".to_string(),
            "class X { int a; }".to_string(),
        )];
        assert!(discover_partial_siblings(&project, &plain_files)
            .await
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trackable_paths_require_unity_source_dirs() {
        let project = r"C:\Proj\Game";
        assert!(is_trackable_cs_path(
            project,
            r"C:\Proj\Game\Assets\Scripts\Player.cs"
        ));
        assert!(is_trackable_cs_path(
            project,
            "c:/proj/game/Packages/com.example/Runtime/A.cs"
        ));
        assert!(!is_trackable_cs_path(
            project,
            r"C:\Proj\Game\Assets\Data\table.csv"
        ));
        assert!(!is_trackable_cs_path(
            project,
            r"C:\Proj\Game\Library\Temp\X.cs"
        ));
        assert!(!is_trackable_cs_path(project, r"C:\Other\Assets\X.cs"));
        assert!(!is_trackable_cs_path(
            project,
            r"C:\Proj\Game\Packages\com.farlocus.locus\Editor\LocusBridge.cs"
        ));
        assert!(!is_trackable_cs_path("", r"C:\Proj\Game\Assets\X.cs"));
    }

    #[tokio::test]
    async fn baseline_is_captured_once_and_cleared_on_convergence() {
        // Isolated project key for this test.
        let project = r"C:\HotReloadTest\Baseline";
        super::super::initialize(true, false);
        crate::csharp_compile::initialize_enabled_for_tests(true);

        note_cs_written(
            project,
            r"C:\HotReloadTest\Baseline\Assets\A.cs",
            "v1".to_string(),
        )
        .await;
        note_cs_written(
            project,
            r"C:\HotReloadTest\Baseline\Assets\A.cs",
            "v2".to_string(),
        )
        .await;

        {
            let projects = projects().lock().await;
            let state = projects.get(&project_key(project)).expect("state");
            let edit = state.pending.values().next().expect("edit");
            assert_eq!(edit.baseline, "v1", "first write wins the baseline");
        }

        on_recompile_converged(project).await;
        {
            let projects = projects().lock().await;
            let state = projects.get(&project_key(project)).expect("state");
            assert!(state.pending.is_empty());
            assert!(state.cold_paths.is_empty());
        }

        super::super::initialize(false, false);
        crate::csharp_compile::initialize_enabled_for_tests(false);
    }

    #[test]
    fn classify_reload_distinguishes_compile_from_bare_reload() {
        use ReloadDecision::*;
        // First sample: seed only (pending may be uncompiled — never clear).
        assert_eq!(
            classify_reload(None, None, None, "s1", "g1", 0, false),
            Seed
        );
        // Nothing moved (steady state / transient pipe blip).
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(0), "s1", "g1", 0, false),
            Unchanged
        );
        // Compile + reload: generation and serial both moved.
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(0), "s1", "g2", 1, false),
            Converged
        );
        // Bare domain reload (entered play mode): generation moved, serial held.
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(2), "s1", "g2", 2, false),
            Reloaded
        );
        // Compile in place (no reload): serial moved without a generation change.
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(2), "s1", "g1", 3, false),
            Converged
        );
        // Editor restart (NEW session) WITHOUT recorded survivors: keep evidence.
        // A fresh editor's startup compile can fail, leaving the sources
        // unloaded, and without the survived-exit hint we cannot prove the
        // moved serial compiled THESE edits — so do not converge (safe
        // over-report) until the instance reports a same-session compile.
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(0), "s2", "gNew", 0, false),
            Reloaded
        );
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(5), "s2", "gNew", 7, false),
            Reloaded
        );
        // ...but once that new instance reports a successful compile, it converges.
        assert_eq!(
            classify_reload(Some("s2"), Some("gNew"), Some(0), "s2", "gX", 1, false),
            Converged
        );
        // First sample with no baseline only seeds, even if the serial looks
        // advanced — documents the race the monitor avoids by seeding a baseline
        // on connect before any edit can be the first sample.
        assert_eq!(
            classify_reload(None, None, None, "s1", "g2", 5, false),
            Seed
        );

        // ── Edits that survived the previous editor's exit ──
        // Relaunch's startup recompile loaded them (serial advanced past 0):
        // converge, whether the first sample lands on the seed path (trackers
        // blanked on exit)...
        assert_eq!(
            classify_reload(None, None, None, "s2", "gNew", 1, true),
            Converged
        );
        // ...or on the new-session path (exit not observed, trackers stale).
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(3), "s2", "gNew", 4, true),
            Converged
        );
        // Survivors but the relaunch has not compiled yet (serial still 0): keep
        // pending — startup compile may have failed, leaving last-good loaded.
        assert_eq!(
            classify_reload(None, None, None, "s2", "gNew", 0, true),
            Seed
        );
        assert_eq!(
            classify_reload(Some("s1"), Some("g1"), Some(3), "s2", "gNew", 0, true),
            Reloaded
        );
    }

    async fn unapplied_in_project(project: &str) -> u64 {
        let snapshot: Vec<PendingEdit> = {
            let projects = projects().lock().await;
            projects
                .get(&project_key(project))
                .map(|state| state.pending.values().cloned().collect())
                .unwrap_or_default()
        };
        let mut count = 0u64;
        for edit in snapshot {
            if is_pending_edit_unapplied(&edit).await {
                count += 1;
            }
        }
        count
    }

    #[tokio::test]
    async fn observe_converges_on_compile_and_keeps_on_bare_reload() {
        let dir = std::env::temp_dir().join(format!(
            "locus-observe-reload-{}",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos() as u64
        ));
        let assets = dir.join("Assets");
        std::fs::create_dir_all(&assets).unwrap();
        let file = assets.join("Obs.cs");
        let file_path = file.to_string_lossy().to_string();
        let project = dir.to_string_lossy().to_string();

        // Seed pending directly rather than through note_cs_written, so the test
        // does not race other tests on the global hot-reload enabled flag (the
        // reconciliation paths under test do not read that flag).
        async fn seed(project: &str, path: &str, baseline: &str) {
            let mut projects = projects().lock().await;
            let state = projects.entry(project_key(project)).or_default();
            state.pending.insert(
                file_key(path),
                PendingEdit {
                    absolute_path: path.to_string(),
                    baseline: baseline.to_string(),
                    applied_text: None,
                },
            );
        }

        // An edit on disk that differs from its compiled baseline is unapplied.
        std::fs::write(&file, "v1").unwrap();
        seed(&project, &file_path, "v0").await;
        assert_eq!(unapplied_in_project(&project).await, 1);

        // Seed the reload tracker, then a Unity recompile (serial moves) converges.
        observe_reload_state(&project, "s1".to_string(), "g1".to_string(), 0).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            1,
            "seeding takes no action"
        );
        observe_reload_state(&project, "s1".to_string(), "g2".to_string(), 1).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            0,
            "a compile-driven reload converges pending"
        );

        // A fresh edit, then a bare domain reload (play mode) keeps it pending.
        std::fs::write(&file, "v2").unwrap();
        seed(&project, &file_path, "v1").await;
        observe_reload_state(&project, "s1".to_string(), "g2".to_string(), 1).await; // record current sample
        observe_reload_state(&project, "s1".to_string(), "g3".to_string(), 1).await; // generation moved, serial held
        assert_eq!(
            unapplied_in_project(&project).await,
            1,
            "a bare reload keeps edits pending"
        );

        // A fresh editor instance (new session id) does NOT auto-converge — the
        // startup compile is unproven — so edits stay pending until the new
        // instance reports a successful compile.
        observe_reload_state(&project, "s2".to_string(), "g9".to_string(), 0).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            1,
            "a new editor instance keeps pending until a confirmed compile"
        );
        observe_reload_state(&project, "s2".to_string(), "g10".to_string(), 1).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            0,
            "a confirmed compile in the new instance converges"
        );

        // Editor exit PRESERVES pending (a relaunch is not proof of a clean
        // compile) — it only resets the dead detour state, never deletes the
        // evidence. It does, however, record that the edits outlived the editor.
        std::fs::write(&file, "v3").unwrap();
        seed(&project, &file_path, "v2").await;
        assert_eq!(unapplied_in_project(&project).await, 1);
        on_editor_exited(&project).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            1,
            "exit preserves pending as evidence"
        );

        // Relaunch whose startup recompile loaded the survivors (serial > 0):
        // the first new-session sample converges them instead of stranding the
        // count — the original "still unapplied after restarting Unity" bug.
        observe_reload_state(&project, "s3".to_string(), "gReboot".to_string(), 1).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            0,
            "a relaunch that recompiled the survivors converges them"
        );

        // Now the failed-startup path: exit with fresh pending, relaunch reports
        // serial 0 (startup compile failed / not run) — keep pending until a
        // real compile in the new instance advances the serial.
        std::fs::write(&file, "v4").unwrap();
        seed(&project, &file_path, "v3").await;
        on_editor_exited(&project).await;
        observe_reload_state(&project, "s4".to_string(), "gBoot".to_string(), 0).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            1,
            "a relaunch that has not compiled (serial 0) keeps survivors pending"
        );
        observe_reload_state(&project, "s4".to_string(), "gBoot2".to_string(), 1).await;
        assert_eq!(
            unapplied_in_project(&project).await,
            0,
            "the new instance's first successful compile converges them"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn access_caps_parse_from_unity_matrix() {
        let matrix = serde_json::json!({
            "cells": [
                { "op": "ldfld", "visibility": "private", "ok": true, "error": "" },
                { "op": "ldsfld", "visibility": "private", "ok": false, "error": "FieldAccessException: x" },
                { "op": "", "visibility": "private", "ok": true, "error": "" },
            ],
            "primitives": {
                "create_delegate_non_public": true,
                "dynamic_method_skip_visibility": false,
                "dynamic_method_byref_return": true,
            },
            "errors": [],
        });
        let caps = parse_access_caps(&matrix);
        assert!(caps.create_delegate_non_public);
        assert!(!caps.dynamic_method_skip_visibility);
        assert!(caps.dynamic_method_byref_return);
        assert_eq!(caps.cells.get("ldfld_private"), Some(&true));
        assert_eq!(caps.cells.get("ldsfld_private"), Some(&false));
        assert_eq!(caps.cells.len(), 2, "malformed cell must be dropped");

        // Missing pieces (old plugin shapes) read conservative.
        let caps = parse_access_caps(&serde_json::json!({}));
        assert_eq!(caps, crate::csharp_compile::AccessCaps::default());
    }

    #[tokio::test]
    async fn access_probe_cache_is_keyed_by_domain_generation() {
        let project = r"C:\HotReloadTest\AccessProbeCache";
        let mut caps = crate::csharp_compile::AccessCaps::default();
        caps.cells.insert("ldfld_private".to_string(), true);

        store_access_probe(
            project,
            AccessProbeCacheEntry {
                domain_generation: "gen-1".to_string(),
                caps: caps.clone(),
                matrix: serde_json::json!({ "cells": [] }),
            },
        )
        .await;

        let hit = cached_access_probe(project, "gen-1").await.expect("hit");
        assert_eq!(hit.caps, caps);
        assert!(cached_access_probe(project, "gen-2").await.is_none());

        // Convergence keeps the entry (the domain is unchanged until a real
        // reload mints a new generation, which simply misses the cache).
        on_recompile_converged(project).await;
        assert!(cached_access_probe(project, "gen-1").await.is_some());
    }

    #[tokio::test]
    async fn rejected_patches_queue_for_convergence() {
        let project = r"C:\HotReloadTest\RejectQueue";

        let queued = queue_cold_paths(project, &["a.cs".to_string(), "b.cs".to_string()]).await;
        assert_eq!(queued, 2);

        // Re-queueing the same key does not double-count.
        let queued = queue_cold_paths(project, &["a.cs".to_string()]).await;
        assert_eq!(queued, 2);

        on_recompile_converged(project).await;
        {
            let projects = projects().lock().await;
            let state = projects.get(&project_key(project)).expect("state");
            assert!(state.cold_paths.is_empty());
        }
    }

    #[tokio::test]
    async fn active_patch_counters_are_isolated_per_project() {
        // P2-a: the live-detour tallies and convergence state are per project, so
        // two open editors never cross-contaminate (they were once module-global
        // atomics shared by every project). Distinct keys keep this test isolated
        // from the others sharing the process-global map.
        let a = r"C:\HotReloadTest\CrosstalkA";
        let b = r"C:\HotReloadTest\CrosstalkB";

        record_patch_applied(a, 128, 3).await;
        record_patch_applied(b, 64, 1).await;
        record_patch_applied(b, 64, 1).await;
        assert_eq!(project_active_patches(a).await, 1);
        assert_eq!(project_active_patches(b).await, 2);

        // The crux: a domain reload in A drops A's detours but must leave B's live
        // count untouched. The old global `ACTIVE_PATCHES.store(0)` zeroed everyone,
        // so B believed it had no active patch and under-converged.
        on_domain_reloaded(a).await;
        assert_eq!(
            project_active_patches(a).await,
            0,
            "A's detours die with its domain"
        );
        assert_eq!(
            project_active_patches(b).await,
            2,
            "B's active patches must survive A's domain reload"
        );

        // The monotonic session total is independent of the live-detour reset, and
        // the active byte/code tallies reset alongside the patch count.
        let (applied_a, bytes_a) = {
            let projects = projects().lock().await;
            let state = projects.get(&project_key(a)).expect("state");
            (state.patches_applied, state.active_patch_bytes)
        };
        assert_eq!(
            applied_a, 1,
            "patches_applied is monotonic across the reload"
        );
        assert_eq!(bytes_a, 0, "active bytes reset with the domain");

        on_recompile_converged(a).await;
        on_recompile_converged(b).await;
    }

    #[tokio::test]
    async fn relative_project_paths_are_tracked_against_project_root() {
        let project = r"C:\HotReloadTest\Relative";
        super::super::initialize(true, false);
        crate::csharp_compile::initialize_enabled_for_tests(true);

        note_cs_written(project, r"Assets\Scripts\B.cs", "old".to_string()).await;

        {
            let projects = projects().lock().await;
            let state = projects.get(&project_key(project)).expect("state");
            let edit = state.pending.values().next().expect("edit");
            assert!(file_key(&edit.absolute_path).ends_with("/assets/scripts/b.cs"));
            assert_eq!(edit.baseline, "old");
        }

        on_recompile_converged(project).await;
        super::super::initialize(false, false);
        crate::csharp_compile::initialize_enabled_for_tests(false);
    }
}
