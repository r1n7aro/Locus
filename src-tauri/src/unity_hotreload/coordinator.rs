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
}

#[derive(Default)]
struct ProjectState {
    pending: HashMap<String, PendingEdit>,
    cold_paths: BTreeSet<String>,
    /// Last Unity AppDomain generation seen for this project — used to tell
    /// a real domain reload (detours died) from a transient pipe drop
    /// (detours still live) on reconnect.
    last_domain_generation: Option<String>,
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
        });
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
        eprintln!("[HotReload] pipe reconnected within the same domain generation; active patches kept");
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

    // M4: patch assemblies reference the field-store runtime shipped with
    // the plugin (Unity's own reference set only carries script assemblies).
    let extra_references = hotreload_runtime_references(project_path);

    let started = std::time::Instant::now();
    let outcome = crate::csharp_compile::compile_hot_patch(&params, &files, &extra_references)
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

            super::record_patch_applied();
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

    #[tokio::test]
    async fn rejected_patches_queue_for_convergence() {
        let project = r"C:\HotReloadTest\RejectQueue";

        let queued = queue_cold_paths(
            project,
            &["a.cs".to_string(), "b.cs".to_string()],
        )
        .await;
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
