//! C# compilation offloaded to a CoreCLR sidecar (modern Roslyn on .NET 10).
//!
//! `unity_execute` / `unity_run_states` snippets are compiled here instead of
//! inside the Unity Editor process when the `unity_sidecar_compiler` setting
//! is on; Unity then only `Assembly.Load`s the emitted bytes
//! (`execute_loaded` / `run_states_loaded` pipe messages). Any sidecar
//! infrastructure failure falls back to the legacy in-Unity compile path —
//! compile *diagnostics* are not failures, they surface to the agent
//! directly.
//!
//! See `coreclr-compile-sidecar-plan.md` for the architecture, and
//! `locus_compile_server/` for the server side of the protocol.

mod client;
pub mod manager;
pub mod params;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value};
use tauri::Emitter;

pub const STATUS_EVENT: &str = "csharp-compile-status";

static ENABLED: AtomicBool = AtomicBool::new(false);
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

// Session counters for the phase-6 rollout: how often tool calls actually
// used the sidecar, hit deterministic compile errors, or fell back to the
// in-Unity path. Surfaced in the settings status payload.
static SIDECAR_COMPILES: AtomicU64 = AtomicU64::new(0);
static SIDECAR_COMPILE_ERRORS: AtomicU64 = AtomicU64::new(0);
static SIDECAR_FALLBACKS: AtomicU64 = AtomicU64::new(0);

fn record_outcome(outcome: &Result<CompileOutcome, String>) {
    match outcome {
        Ok(Ok(_)) => {
            SIDECAR_COMPILES.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Err(_)) => {
            SIDECAR_COMPILE_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        // Transport errors are counted at the fallback site (note_fallback),
        // which also sees the non-transport reasons (disabled plugin, etc.).
        Err(_) => {}
    }
    emit_status_in_background();
}

/// Push a fresh status snapshot to the UI (settings card subscribes), so
/// asynchronous failures — warm-up errors, runtime download problems,
/// runtime fallbacks — are visible without re-opening the page. No-op until
/// app setup provides the handle (tests, early startup).
pub(crate) fn emit_status_in_background() {
    let Some(app_handle) = APP_HANDLE.get().cloned() else {
        return;
    };
    tokio::spawn(async move {
        let _ = app_handle.emit(STATUS_EVENT, status().await);
    });
}

/// Called once from app setup with the persisted flag.
pub fn initialize(enabled: bool, app_handle: tauri::AppHandle) {
    ENABLED.store(enabled, Ordering::Relaxed);
    let _ = APP_HANDLE.set(app_handle);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Flip the feature flag. Disabling stops the running sidecar.
pub async fn set_enabled(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
    if !value {
        manager::shutdown().await;
    }
    emit_status_in_background();
}

/// Best-effort synchronous kill for app-exit paths.
pub fn kill_active_server_for_exit() {
    manager::kill_for_exit();
}

/// Record that a tool call fell back to the legacy in-Unity compile path.
/// Logged only when the reason changes so a persistent condition (sidecar
/// missing, old Unity plugin) does not spam on every call.
pub fn note_fallback(reason: &str) {
    SIDECAR_FALLBACKS.fetch_add(1, Ordering::Relaxed);
    emit_status_in_background();
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let Ok(mut guard) = LAST.lock() else { return };
    if guard.as_deref() != Some(reason) {
        eprintln!("[CsharpCompile] falling back to in-Unity compile: {reason}");
        *guard = Some(reason.to_string());
    }
}

/// Status snapshot for the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsharpCompileStatusPayload {
    pub enabled: bool,
    pub platform_supported: bool,
    /// Published sidecar binaries present on disk.
    pub server_available: bool,
    pub running: bool,
    pub roslyn_version: Option<String>,
    pub dotnet_source: Option<String>,
    pub uptime_secs: Option<u64>,
    pub last_error: Option<String>,
    /// Session counters (rollout observability): tool compiles served by the
    /// sidecar, deterministic compile errors, and fallbacks to Unity.
    pub sidecar_compiles: u64,
    pub compile_errors: u64,
    pub fallbacks: u64,
    /// Hot reload (`unity_hotreload`): feature flag and session counters.
    pub hot_reload_enabled: bool,
    pub hot_patches_applied: u64,
    pub hot_patch_failures: u64,
    pub hot_active_patches: u64,
    pub hot_cold_queued: u64,
}

pub async fn status() -> CsharpCompileStatusPayload {
    let running = manager::current_status().await;
    let hot_reload = crate::unity_hotreload::counters();
    CsharpCompileStatusPayload {
        enabled: is_enabled(),
        platform_supported: crate::dotnet_runtime::is_platform_supported(),
        server_available: manager::server_dll_available(),
        running: running.is_some(),
        roslyn_version: running.as_ref().map(|(roslyn, _, _)| roslyn.clone()),
        dotnet_source: running.as_ref().map(|(_, source, _)| source.to_string()),
        uptime_secs: running.as_ref().map(|(_, _, uptime)| uptime.as_secs()),
        last_error: manager::last_error_for_diagnostics(),
        sidecar_compiles: SIDECAR_COMPILES.load(Ordering::Relaxed),
        compile_errors: SIDECAR_COMPILE_ERRORS.load(Ordering::Relaxed),
        fallbacks: SIDECAR_FALLBACKS.load(Ordering::Relaxed),
        hot_reload_enabled: crate::unity_hotreload::is_enabled(),
        hot_patches_applied: hot_reload.patches_applied,
        hot_patch_failures: hot_reload.patch_failures,
        hot_active_patches: hot_reload.active_patches,
        hot_cold_queued: hot_reload.cold_queued,
    }
}

/// Pre-start the sidecar and JIT-warm Roslyn with a tiny self-contained
/// compile so the first real snippet does not pay the cold-start cost.
/// No-op while the feature is disabled.
pub fn warm_up_in_background() {
    if !is_enabled() {
        return;
    }
    tokio::spawn(async move {
        let warm = json!({
            "assemblyName": "__LocusWarmup",
            "sources": [{ "path": "Warmup.cs", "text": "internal static class __LocusWarmup { }" }],
            "useHostBcl": true,
        });
        match compile_raw(warm).await {
            Ok(Ok(_)) => {}
            Ok(Err(failure)) => eprintln!(
                "[CsharpCompile] warm-up compile failed unexpectedly: {}",
                failure.message
            ),
            Err(error) => eprintln!("[CsharpCompile] warm-up skipped: {error}"),
        }
        // Surface the outcome (running / lastError) in the settings card.
        emit_status_in_background();
    });
}

// ── request/response types ───────────────────────────────────────────

/// Compile parameters provided by the Unity side (`get_compile_params`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompileParams {
    pub fingerprint: String,
    pub domain_generation: String,
    #[serde(default)]
    pub lang_version: String,
    #[serde(default)]
    pub reference_paths: Vec<String>,
    #[serde(default)]
    pub defines: Vec<String>,
}

/// A successfully emitted assembly, ready to ship to Unity.
#[derive(Debug, Clone)]
pub struct CompiledAssembly {
    pub assembly_b64: String,
    pub assembly_name: String,
    pub entry_type: Option<String>,
    /// Snippet compiles: "statements" or "expression" (the mode that won).
    pub mode: Option<String>,
}

/// A compile-level failure: diagnostics / validation text for the agent.
/// Distinct from transport errors (`Err(String)`) which trigger fallback.
#[derive(Debug, Clone)]
pub struct CompileFailure {
    pub stage: String,
    pub message: String,
}

pub type CompileOutcome = Result<CompiledAssembly, CompileFailure>;

// ── compile entry points ─────────────────────────────────────────────

/// Compile a unity_execute snippet (statement mode with expression-mode
/// fallback, same semantics as the Unity-side CompileAsyncSnippet).
/// `register_image` should be true only when the result will be loaded into
/// the Unity domain (so the session image registry mirrors loaded code).
pub async fn compile_snippet(
    compile_params: &CompileParams,
    code: &str,
    reference_session_images: bool,
    register_image: bool,
) -> Result<CompileOutcome, String> {
    let request = json!({
        "code": code,
        "params": compile_params,
        "referenceSessionImages": reference_session_images,
        "registerImage": register_image,
    });
    let outcome = request_compile("compile/snippet", request).await;
    record_outcome(&outcome);
    outcome
}

/// Compile a unity_run_states request (also serves as the
/// `compile_run_states` pre-check: validation errors come back as
/// `CompileFailure { stage: "validation" }`).
pub async fn compile_run_states(
    compile_params: &CompileParams,
    run_states_request: &Value,
    reference_session_images: bool,
    register_image: bool,
) -> Result<CompileOutcome, String> {
    let request = json!({
        "request": run_states_request,
        "params": compile_params,
        "referenceSessionImages": reference_session_images,
        "registerImage": register_image,
    });
    let outcome = request_compile("compile/runStates", request).await;
    record_outcome(&outcome);
    outcome
}

/// Compile a View Script (compile_named / invoke_named). The result rides
/// inside the legacy pipe message as optional `assembly_b64` / `assembly_id`
/// fields: a current Unity plugin loads the bytes on a cache miss, an older
/// plugin ignores them and compiles from source as before.
pub async fn compile_view_script(
    compile_params: &CompileParams,
    source: &str,
    source_path: &str,
    script_name: &str,
) -> Result<CompileOutcome, String> {
    let request = json!({
        "source": source,
        "path": source_path,
        "scriptName": script_name,
        "params": compile_params,
    });
    let outcome = request_compile("compile/viewScript", request).await;
    record_outcome(&outcome);
    outcome
}

/// Compile an arbitrary source set (tests and the warm-up).
pub async fn compile_raw(request: Value) -> Result<CompileOutcome, String> {
    request_compile("compile/raw", request).await
}

// ── hot patch (unity_hot_reload) ─────────────────────────────────────

/// One original→patch method redirection from `compile/hotPatch`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotPatchMethod {
    pub declaring_type: String,
    #[serde(default)]
    pub patch_declaring_type: String,
    pub name: String,
    #[serde(default)]
    pub param_type_names: Vec<String>,
    #[serde(default)]
    pub is_static: bool,
    #[serde(default)]
    pub is_ctor: bool,
}

/// A type that only exists in the edited text (TI-C / snippet visibility).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotPatchNewType {
    #[serde(default)]
    pub metadata_name: String,
    #[serde(default)]
    pub ns: String,
    #[serde(default)]
    pub simple_name: String,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub is_top_level: bool,
}

/// Parsed `compile/hotPatch` response. Transport failures stay `Err(String)`
/// at the call site (no legacy fallback exists for hot patches — the
/// recompile path is the recovery).
#[derive(Debug, Clone)]
pub enum HotPatchOutcome {
    /// At least one file needs a real compile; per-file reasons.
    Cold { files: Vec<(String, Vec<String>)> },
    /// Only comments/formatting changed.
    Noop,
    /// Deterministic compiler diagnostics for the agent.
    CompileError(String),
    Compiled {
        assembly_name: String,
        assembly_b64: String,
        methods: Vec<HotPatchMethod>,
        new_types: Vec<HotPatchNewType>,
    },
}

/// Diff + rewrite + compile edited files into a hot-patch assembly.
/// `files` entries are (path, baselineText, currentText).
pub async fn compile_hot_patch(
    compile_params: &CompileParams,
    files: &[(String, String, String)],
) -> Result<HotPatchOutcome, String> {
    let request = json!({
        "files": files
            .iter()
            .map(|(path, old_text, new_text)| json!({
                "path": path,
                "oldText": old_text,
                "newText": new_text,
            }))
            .collect::<Vec<_>>(),
        "params": compile_params,
        "referenceSessionImages": true,
        "registerImage": true,
    });

    let client = manager::ensure_client().await?;
    let value = client
        .request_with_timeout("compile/hotPatch", request, client::COMPILE_REQUEST_TIMEOUT)
        .await?;
    parse_hot_patch_result(value)
}

fn parse_hot_patch_result(value: Value) -> Result<HotPatchOutcome, String> {
    let hot = value
        .get("hot")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "malformed compile server response (missing hot)".to_string())?;

    if !hot {
        let mut files = Vec::new();
        if let Some(entries) = value.get("files").and_then(|v| v.as_array()) {
            for entry in entries {
                let path = entry
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let reasons = entry
                    .get("reasons")
                    .and_then(|v| v.as_array())
                    .map(|reasons| {
                        reasons
                            .iter()
                            .filter_map(|r| r.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                files.push((path, reasons));
            }
        }
        return Ok(HotPatchOutcome::Cold { files });
    }

    if value.get("noop").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Ok(HotPatchOutcome::Noop);
    }

    let success = value
        .get("success")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "malformed compile server response (missing success)".to_string())?;
    if !success {
        let message = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown compile failure")
            .to_string();
        return Ok(HotPatchOutcome::CompileError(message));
    }

    let assembly_b64 = value
        .get("assemblyB64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "malformed compile server response (missing assemblyB64)".to_string())?
        .to_string();
    let assembly_name = value
        .get("assemblyName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let methods: Vec<HotPatchMethod> =
        serde_json::from_value(value.get("methods").cloned().unwrap_or_else(|| json!([])))
            .map_err(|e| format!("malformed hot patch methods: {e}"))?;
    let new_types: Vec<HotPatchNewType> =
        serde_json::from_value(value.get("newTypes").cloned().unwrap_or_else(|| json!([])))
            .map_err(|e| format!("malformed hot patch newTypes: {e}"))?;

    Ok(HotPatchOutcome::Compiled {
        assembly_name,
        assembly_b64,
        methods,
        new_types,
    })
}

// ── type index (TI-B) ────────────────────────────────────────────────

/// Build the Unity type index from reference metadata in the sidecar
/// (`index/types`). Returns the parsed entry set; the caller pairs it with a
/// Unity-side fingerprint. Transport errors route back to the Unity export.
pub async fn index_types(
    compile_params: &CompileParams,
) -> Result<Vec<crate::unity_type_index::UnityTypeIndexEntry>, String> {
    let client = manager::ensure_client().await?;
    let value = client
        .request_with_timeout(
            "index/types",
            json!({ "params": compile_params }),
            client::DEFAULT_REQUEST_TIMEOUT,
        )
        .await?;

    let types = value
        .get("types")
        .cloned()
        .ok_or_else(|| "malformed index/types response (missing types)".to_string())?;
    serde_json::from_value(types).map_err(|e| format!("malformed index/types entries: {e}"))
}

/// Test-only flag control (`initialize` needs an AppHandle).
#[cfg(test)]
pub fn initialize_enabled_for_tests(value: bool) {
    ENABLED.store(value, Ordering::Relaxed);
}

async fn request_compile(method: &str, request: Value) -> Result<CompileOutcome, String> {
    let client = manager::ensure_client().await?;
    let result = client
        .request_with_timeout(method, request, client::COMPILE_REQUEST_TIMEOUT)
        .await?;
    parse_compile_result(result)
}

fn parse_compile_result(value: Value) -> Result<CompileOutcome, String> {
    let success = value
        .get("success")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| "malformed compile server response (missing success)".to_string())?;

    if !success {
        let message = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown compile failure")
            .to_string();
        let stage = value
            .get("errorStage")
            .and_then(|v| v.as_str())
            .unwrap_or("compile")
            .to_string();
        return Ok(Err(CompileFailure { stage, message }));
    }

    let assembly_b64 = value
        .get("assemblyB64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "malformed compile server response (missing assemblyB64)".to_string())?
        .to_string();
    let assembly_name = value
        .get("assemblyName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let entry_type = value
        .get("entryType")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mode = value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(Ok(CompiledAssembly {
        assembly_b64,
        assembly_name,
        entry_type,
        mode,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_compile_result_success() {
        let value = json!({
            "success": true,
            "assemblyName": "__LocusRuntimeAsync_00000000_00000001",
            "assemblyB64": "TVo=",
            "entryType": "Locus.RuntimeSnippets.__LocusAsyncSnippetHost",
            "mode": "statements",
            "durationMs": 12,
        });
        let outcome = parse_compile_result(value).expect("parse");
        let assembly = outcome.expect("success");
        assert_eq!(assembly.assembly_b64, "TVo=");
        assert_eq!(assembly.assembly_name, "__LocusRuntimeAsync_00000000_00000001");
        assert_eq!(
            assembly.entry_type.as_deref(),
            Some("Locus.RuntimeSnippets.__LocusAsyncSnippetHost")
        );
        assert_eq!(assembly.mode.as_deref(), Some("statements"));
    }

    #[test]
    fn parse_compile_result_failure() {
        let value = json!({
            "success": false,
            "error": "compilation failed:\n  CS0103 at 1:1: The name 'x' does not exist in the current context\n",
            "errorStage": "compile",
        });
        let outcome = parse_compile_result(value).expect("parse");
        let failure = outcome.expect_err("failure");
        assert_eq!(failure.stage, "compile");
        assert!(failure.message.starts_with("compilation failed:\n"));
    }

    #[test]
    fn parse_compile_result_malformed() {
        assert!(parse_compile_result(json!({ "bogus": true })).is_err());
    }

    #[test]
    fn parse_hot_patch_result_cold() {
        let value = json!({
            "hot": false,
            "files": [
                { "path": "A.cs", "hot": false, "reasons": ["field layout changed: A"] }
            ],
        });
        match parse_hot_patch_result(value).expect("parse") {
            HotPatchOutcome::Cold { files } => {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].0, "A.cs");
                assert_eq!(files[0].1, vec!["field layout changed: A".to_string()]);
            }
            other => panic!("expected Cold, got {other:?}"),
        }
    }

    #[test]
    fn parse_hot_patch_result_noop_and_error() {
        match parse_hot_patch_result(json!({ "hot": true, "success": true, "noop": true })).expect("parse") {
            HotPatchOutcome::Noop => {}
            other => panic!("expected Noop, got {other:?}"),
        }
        match parse_hot_patch_result(json!({
            "hot": true,
            "success": false,
            "error": "compilation failed:\n  CS0103 at 1:1: nope\n",
            "errorStage": "compile",
        }))
        .expect("parse")
        {
            HotPatchOutcome::CompileError(message) => {
                assert!(message.starts_with("compilation failed:"));
            }
            other => panic!("expected CompileError, got {other:?}"),
        }
    }

    #[test]
    fn parse_hot_patch_result_compiled() {
        let value = json!({
            "hot": true,
            "success": true,
            "assemblyName": "__LocusHotPatch_00000000_00000001",
            "assemblyB64": "TVo=",
            "methods": [{
                "declaringType": "Game.Player",
                "patchDeclaringType": "Game.Player__LocusPatch",
                "name": "Update",
                "paramTypeNames": [],
                "isStatic": false,
                "isCtor": false,
            }],
            "newTypes": [{
                "metadataName": "Game.Spawner",
                "ns": "Game",
                "simpleName": "Spawner",
                "isPublic": true,
                "isTopLevel": true,
            }],
        });
        match parse_hot_patch_result(value).expect("parse") {
            HotPatchOutcome::Compiled {
                assembly_name,
                assembly_b64,
                methods,
                new_types,
            } => {
                assert_eq!(assembly_name, "__LocusHotPatch_00000000_00000001");
                assert_eq!(assembly_b64, "TVo=");
                assert_eq!(methods.len(), 1);
                assert_eq!(methods[0].declaring_type, "Game.Player");
                assert_eq!(methods[0].patch_declaring_type, "Game.Player__LocusPatch");
                assert_eq!(methods[0].name, "Update");
                assert!(!methods[0].is_static);
                assert_eq!(new_types.len(), 1);
                assert!(new_types[0].is_public && new_types[0].is_top_level);
                assert_eq!(new_types[0].ns, "Game");
            }
            other => panic!("expected Compiled, got {other:?}"),
        }
    }

    #[test]
    fn compile_params_serialize_camel_case() {
        let params = CompileParams {
            fingerprint: "fp".to_string(),
            domain_generation: "gen".to_string(),
            lang_version: "9".to_string(),
            reference_paths: vec!["a.dll".to_string()],
            defines: vec!["UNITY_EDITOR".to_string()],
        };
        let value = serde_json::to_value(&params).expect("serialize");
        assert_eq!(value["domainGeneration"], "gen");
        assert_eq!(value["langVersion"], "9");
        assert_eq!(value["referencePaths"][0], "a.dll");
    }

    /// End-to-end sidecar smoke: spawn, BCL-only compile, crash recovery,
    /// and a 5 MB request frame. Skips (passing) when the published server
    /// DLL or a non-downloading dotnet host is unavailable so the suite
    /// stays green on machines without the sidecar toolchain.
    #[tokio::test]
    async fn compile_raw_bcl_smoke_and_crash_recovery() {
        if manager::find_server_dll().is_none() {
            eprintln!("skip: compile server dll not built (bun run compile-server:bundle)");
            return;
        }
        if crate::dotnet_runtime::try_resolve_cached_dotnet().await.is_none() {
            eprintln!("skip: no cached/system dotnet runtime available");
            return;
        }

        let compile_class_a = || async {
            compile_raw(json!({
                "assemblyName": "LocusSmokeA",
                "sources": [{ "path": "A.cs", "text": "class A { }" }],
                "useHostBcl": true,
            }))
            .await
        };

        let assembly = compile_class_a()
            .await
            .expect("sidecar transport")
            .expect("compile success");
        let bytes = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(assembly.assembly_b64.as_bytes())
                .expect("valid base64")
        };
        assert!(bytes.len() > 512, "suspiciously small assembly");
        assert_eq!(&bytes[..2], b"MZ", "missing PE header");
        assert_eq!(assembly.assembly_name, "LocusSmokeA");

        // Diagnostics keep the legacy "compilation failed:" framing.
        let failure = compile_raw(json!({
            "sources": [{ "path": "B.cs", "text": "class B { void M() { int x = \"oops\"; } }" }],
            "useHostBcl": true,
        }))
        .await
        .expect("sidecar transport")
        .expect_err("compile failure");
        assert!(
            failure.message.starts_with("compilation failed:\n"),
            "unexpected diagnostics framing: {}",
            failure.message
        );
        assert!(failure.message.contains("CS0029"), "{}", failure.message);

        // Crash recovery: kill the process, the next call must respawn.
        assert!(manager::kill_current_for_test().await, "server should be running");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        compile_class_a()
            .await
            .expect("sidecar transport after crash")
            .expect("compile success after restart");

        // 5 MB request frame round-trips (plan §6 transport-size check).
        let padding = "// padding\n".repeat(5 * 1024 * 1024 / 11);
        let big_source = format!("{padding}class Big {{ }}");
        compile_raw(json!({
            "assemblyName": "LocusSmokeBig",
            "sources": [{ "path": "Big.cs", "text": big_source }],
            "useHostBcl": true,
        }))
        .await
        .expect("sidecar transport for 5MB frame")
        .expect("compile success for 5MB frame");

        // Session image registry: assembly A registers under a domain
        // generation, B compiles against it via referenceSessionImages, and
        // a new generation invalidates the old images.
        compile_raw(json!({
            "assemblyName": "LocusSessionA",
            "sources": [{ "path": "A.cs", "text": "public class LocusSessionTypeA { public int Value = 7; }" }],
            "useHostBcl": true,
            "params": { "domainGeneration": "11112222333344445555666677778888" },
            "registerImage": true,
        }))
        .await
        .expect("sidecar transport")
        .expect("session image A compiles");

        let b_source = "public class LocusSessionTypeB { public int Read() { return new LocusSessionTypeA().Value; } }";
        compile_raw(json!({
            "assemblyName": "LocusSessionB",
            "sources": [{ "path": "B.cs", "text": b_source }],
            "useHostBcl": true,
            "params": { "domainGeneration": "11112222333344445555666677778888" },
            "referenceSessionImages": true,
        }))
        .await
        .expect("sidecar transport")
        .expect("B resolves A through the session image registry");

        // A different generation must not see the old generation's images.
        let stale = compile_raw(json!({
            "assemblyName": "LocusSessionB2",
            "sources": [{ "path": "B2.cs", "text": b_source }],
            "useHostBcl": true,
            "params": { "domainGeneration": "99990000999900009999000099990000" },
            "referenceSessionImages": true,
        }))
        .await
        .expect("sidecar transport")
        .expect_err("stale generation should not resolve LocusSessionTypeA");
        assert!(
            stale.message.contains("CS0246"),
            "expected unknown-type diagnostics, got: {}",
            stale.message
        );

        manager::shutdown().await;
    }
}
