//! Hot-reload coordinator: accumulates agent .cs edits (baseline = the text
//! the loaded assemblies were compiled from), drives the sidecar
//! `compile/hotPatch`, ships the patch to Unity via `hot_patch_loaded`, and
//! keeps the cold queue that `unity_recompile` drains.
//!
//! The baseline for a file is captured at its FIRST tool write after the
//! last convergence point (recompile / reload): every later hot reload diffs
//! the current disk text against that baseline, so re-patching a method
//! always re-detours from the original — patches never stack.

use std::collections::{BTreeSet, HashMap};
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
    /// Last Unity AppDomain generation seen for this project — used to tell
    /// a real domain reload (detours died) from a transient pipe drop
    /// (detours still live) on reconnect.
    last_domain_generation: Option<String>,
    /// C0 capability matrix, probed once per domain generation (the probe
    /// assembly and the measured Mono both die with the domain).
    access_probe: Option<AccessProbeCacheEntry>,
}

fn projects() -> &'static Mutex<HashMap<String, ProjectState>> {
    static STATE: OnceLock<Mutex<HashMap<String, ProjectState>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HashMap::new()))
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
    let snapshot: Vec<PendingEdit> = {
        let projects = projects().lock().await;
        projects
            .values()
            .flat_map(|state| state.pending.values().cloned())
            .collect()
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
        // The old domain died with the recompile; the next reconnect (or
        // hot reload) re-learns the new generation.
        state.last_domain_generation = None;
    }
    drop(projects);
    on_domain_reloaded(project_path).await;
    super::set_cold_queue_depth(0);
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
        }
    }
    super::reset_active_patches();
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

/// The Unity pipe (re)connected. A reconnect does NOT always mean the domain
/// reloaded — transient pipe drops (editor stalls, focus loss) keep detours
/// alive. Fetch the current domain generation and only invalidate
/// detour-derived state (active-patch counter, TI-C rows) when the
/// generation actually moved; unknown → fail closed (treat as reloaded).
pub async fn on_pipe_reconnected(project_path: &str) {
    let current_generation = if super::is_enabled() && crate::csharp_compile::is_enabled() {
        crate::csharp_compile::params::get_params(project_path)
            .await
            .ok()
            .map(|params| params.domain_generation)
    } else {
        // Feature off: no detours/TI-C rows exist; skip the roundtrip and
        // keep the conservative cleanup path.
        None
    };

    if reconnect_requires_cleanup(project_path, current_generation).await {
        on_domain_reloaded(project_path).await;
    } else {
        eprintln!(
            "[HotReload] pipe reconnected within the same domain generation; active patches kept"
        );
    }
}

/// Decide whether a reconnect needs detour-state cleanup, recording the
/// generation for the next comparison. Split out for testability.
async fn reconnect_requires_cleanup(
    project_path: &str,
    current_generation: Option<String>,
) -> bool {
    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    match current_generation {
        Some(generation) => {
            let unchanged = state.last_domain_generation.as_deref() == Some(generation.as_str());
            state.last_domain_generation = Some(generation);
            !unchanged
        }
        None => {
            state.last_domain_generation = None;
            true
        }
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

    if probe.code_optimization != "debug" {
        return Err(
            "Hot reload requires Unity Editor Code Optimization = Debug (currently Release; \
             switch it in the Unity status bar or Preferences > General), or use unity_recompile."
                .to_string(),
        );
    }
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

    let params = crate::csharp_compile::params::get_params(project_path)
        .await
        .map_err(|error| {
            format!("Could not get compile params from Unity ({error}); use unity_recompile.")
        })?;

    // Remember the generation so a later transient pipe reconnect is not
    // mistaken for a domain reload (which would wrongly clear patch state).
    {
        let mut projects = projects().lock().await;
        let state = projects.entry(project_key(project_path)).or_default();
        state.last_domain_generation = Some(params.domain_generation.clone());
    }

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
    let outcome = crate::csharp_compile::compile_hot_patch(
        &params,
        &files,
        &baseline_siblings,
        &extra_references,
        Some(&access_caps),
    )
    .await
    .map_err(|error| {
        super::record_patch_failure();
        format!("Compile server unavailable ({error}); use unity_recompile.")
    })?;

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
            super::set_cold_queue_depth(queued as u64);
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
            methods,
            new_types,
            caller_scan_note,
        } => {
            if methods.is_empty() && new_types.is_empty() {
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

            // New-types-only patches skip the detour message: loading the
            // assembly is enough... except nothing would load it. Ship it
            // through the same pipe message with an empty method list NOT
            // allowed (Unity rejects) — so require methods OR load via
            // execute path. Simplest correct path: send hot_patch_loaded
            // whenever there are methods; for pure new-type patches send it
            // too — Unity loads the assembly and applies zero detours.
            let payload = serde_json::json!({
                "patch_id": assembly_name,
                "assembly_b64": assembly_b64,
                "domain_generation": params.domain_generation,
                "methods": methods.iter().map(|m| serde_json::json!({
                    "declaring_type": m.declaring_type,
                    "patch_declaring_type": m.patch_declaring_type,
                    "name": m.name,
                    "param_type_names": m.param_type_names,
                    "is_static": m.is_static,
                    "is_ctor": m.is_ctor,
                    // Older plugins ignore the unknown field and then fail
                    // resolution → whole-patch rollback + update hint (the
                    // established compatibility discipline).
                    "original_assembly": m.original_assembly.as_deref().unwrap_or(""),
                })).collect::<Vec<_>>(),
            })
            .to_string();

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
                    super::record_patch_failure();
                    // The patch never applied: queue the files so the
                    // convergence pass (and the status card) covers them.
                    let queued = queue_cold_paths(project_path, &changed_keys).await;
                    super::set_cold_queue_depth(queued as u64);
                    return Err(format!(
                        "Unity did not accept the hot patch ({error}); use unity_recompile."
                    ));
                }
            };

            if !resp.ok {
                super::record_patch_failure();
                let queued = queue_cold_paths(project_path, &changed_keys).await;
                super::set_cold_queue_depth(queued as u64);
                let error = resp
                    .error
                    .unwrap_or_else(|| "hot patch rejected".to_string());
                if error.starts_with("unknown message type") {
                    return Err(
                        "The Unity plugin in this project predates hot reload; update the Locus \
                         plugin or use unity_recompile."
                            .to_string(),
                    );
                }
                return Err(format!(
                    "Hot patch failed in Unity: {error}\nRun unity_recompile to converge."
                ));
            }

            mark_changed_keys_applied(project_path, &changed_current_texts).await;

            let image_register_error = match crate::csharp_compile::register_session_image(
                &params.domain_generation,
                &assembly_name,
                &assembly_b64,
            )
            .await
            {
                Ok(()) => None,
                Err(error) => Some(error),
            };

            let assembly_bytes = base64_decoded_len(&assembly_b64);
            let code_entries = methods.len().saturating_add(new_types.len()) as u64;
            super::record_patch_applied(assembly_bytes, code_entries);
            // H6: arm the convergence scheduler (threshold / idle / play exit).
            super::note_patch_applied(project_path);

            let engine = resp
                .message
                .as_deref()
                .and_then(|message| serde_json::from_str::<serde_json::Value>(message).ok())
                .and_then(|value| {
                    value
                        .get("detour_engine")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();

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
            summary.push_str(
                ".\nChanges are live in the running Editor — no recompile, no domain reload, \
                 state preserved. The files are on disk, so the next unity_recompile or domain \
                 reload makes them permanent automatically.",
            );

            // TI-C: layer new public top-level types into the cached type
            // index so auto-usings resolve them immediately.
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
                    assembly: assembly_name.clone(),
                })
                .collect();
            if image_register_error.is_none() && !index_types.is_empty() {
                if let Err(error) = crate::unity_type_index::append_hot_patch_types(
                    project_path,
                    &assembly_name,
                    index_types,
                )
                .await
                {
                    eprintln!("[HotReload] type index increment skipped: {error}");
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
        super::super::initialize(true);
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

        super::super::initialize(false);
        crate::csharp_compile::initialize_enabled_for_tests(false);
    }

    #[tokio::test]
    async fn reconnect_cleanup_tracks_domain_generation() {
        let project = r"C:\HotReloadTest\Reconnect";

        // First sighting of a generation: unknown → cleanup required.
        assert!(reconnect_requires_cleanup(project, Some("gen-1".to_string())).await);
        // Transient pipe drop within the same generation: keep detours.
        assert!(!reconnect_requires_cleanup(project, Some("gen-1".to_string())).await);
        // Real domain reload (new generation): cleanup.
        assert!(reconnect_requires_cleanup(project, Some("gen-2".to_string())).await);
        // Unknown generation (params fetch failed): fail closed.
        assert!(reconnect_requires_cleanup(project, None).await);
        assert!(reconnect_requires_cleanup(project, Some("gen-2".to_string())).await);

        // Convergence forgets the generation: next reconnect cleans again.
        {
            let mut projects = projects().lock().await;
            let state = projects.entry(project_key(project)).or_default();
            state.last_domain_generation = Some("gen-3".to_string());
        }
        on_recompile_converged(project).await;
        assert!(reconnect_requires_cleanup(project, Some("gen-3".to_string())).await);
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
    async fn relative_project_paths_are_tracked_against_project_root() {
        let project = r"C:\HotReloadTest\Relative";
        super::super::initialize(true);
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
        super::super::initialize(false);
        crate::csharp_compile::initialize_enabled_for_tests(false);
    }
}
