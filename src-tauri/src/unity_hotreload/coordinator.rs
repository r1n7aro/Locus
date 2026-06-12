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
    if !is_trackable_cs_path(project_path, file_path) {
        return;
    }

    let mut projects = projects().lock().await;
    let state = projects.entry(project_key(project_path)).or_default();
    state
        .pending
        .entry(file_key(file_path))
        .or_insert_with(|| PendingEdit {
            absolute_path: file_path.to_string(),
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
    }
    drop(projects);
    super::reset_active_patches();
    super::set_cold_queue_depth(0);
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
        return Err(format!("Unity probe failed: {error}. Use unity_recompile instead."));
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
            if probe.error.is_empty() { "no detail" } else { &probe.error }
        ));
    }
    Ok(())
}

// ── hot reload orchestration ─────────────────────────────────────────

/// Outcome text for the `unity_hot_reload` tool. `Err` carries agent-facing
/// errors (compile diagnostics, gating guidance).
pub async fn hot_reload(project_path: &str, path_filter: Option<Vec<String>>) -> Result<String, String> {
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

    // Snapshot the pending set for this project (filtered when asked).
    let filter: Option<BTreeSet<String>> = path_filter.map(|paths| {
        paths
            .iter()
            .map(|path| {
                if std::path::Path::new(path).is_absolute() {
                    file_key(path)
                } else {
                    file_key(&format!("{}/{}", project_path.trim_end_matches(['/', '\\']), path))
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
                // Deleted since the edit — deletion is never hot.
                return Err(format!(
                    "{} was deleted after being edited; file deletions need unity_recompile.",
                    edit.absolute_path
                ));
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
        return Ok("All tracked edits are back at their compiled baseline; nothing to hot reload.".to_string());
    }

    let params = crate::csharp_compile::params::get_params(project_path)
        .await
        .map_err(|error| {
            format!("Could not get compile params from Unity ({error}); use unity_recompile.")
        })?;

    let started = std::time::Instant::now();
    let outcome = crate::csharp_compile::compile_hot_patch(&params, &files)
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
            let queued = {
                let mut projects = projects().lock().await;
                let state = projects.entry(project_key(project_path)).or_default();
                for key in &changed_keys {
                    state.cold_paths.insert(key.clone());
                }
                state.cold_paths.len()
            };
            super::set_cold_queue_depth(queued as u64);
            lines.push(format!(
                "Run unity_recompile to apply them ({queued} file(s) queued). Hot-applied edits \
                 from earlier patches stay live until then."
            ));
            Ok(lines.join("\n"))
        }
        crate::csharp_compile::HotPatchOutcome::Noop => Ok(
            "No effective code change (comments/formatting only); nothing to hot reload.".to_string(),
        ),
        crate::csharp_compile::HotPatchOutcome::CompileError(message) => Err(message),
        crate::csharp_compile::HotPatchOutcome::Compiled {
            assembly_name,
            assembly_b64,
            methods,
            new_types,
        } => {
            if methods.is_empty() && new_types.is_empty() {
                return Ok(
                    "No effective code change (comments/formatting only); nothing to hot reload."
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
                })).collect::<Vec<_>>(),
            })
            .to_string();

            let resp = crate::unity_bridge::send_message_with_timeout(
                project_path,
                "hot_patch_loaded",
                &payload,
                std::time::Duration::from_secs(30),
            )
            .await
            .map_err(|error| {
                super::record_patch_failure();
                format!("Unity did not accept the hot patch ({error}); use unity_recompile.")
            })?;

            if !resp.ok {
                super::record_patch_failure();
                let error = resp.error.unwrap_or_else(|| "hot patch rejected".to_string());
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

            super::record_patch_applied();

            let engine = resp
                .message
                .as_deref()
                .and_then(|message| serde_json::from_str::<serde_json::Value>(message).ok())
                .and_then(|value| value.get("detour_engine").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_default();

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
            if !index_types.is_empty() {
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
        assert!(is_trackable_cs_path(project, r"C:\Proj\Game\Assets\Scripts\Player.cs"));
        assert!(is_trackable_cs_path(project, "c:/proj/game/Packages/com.example/Runtime/A.cs"));
        assert!(!is_trackable_cs_path(project, r"C:\Proj\Game\Assets\Data\table.csv"));
        assert!(!is_trackable_cs_path(project, r"C:\Proj\Game\Library\Temp\X.cs"));
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

        note_cs_written(project, r"C:\HotReloadTest\Baseline\Assets\A.cs", "v1".to_string()).await;
        note_cs_written(project, r"C:\HotReloadTest\Baseline\Assets\A.cs", "v2".to_string()).await;

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
}
