//! Lifecycle of the compile-server sidecar process: locate the published
//! DLL, resolve a .NET host (shared `dotnet_runtime` module), spawn,
//! handshake, and respawn after a crash.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use serde_json::json;

use super::client::CompileClient;

/// Protocol + wrapper contract versions this build of Locus expects. Must
/// match `CompileService.ProtocolVersion` / `WrapperContractVersion` in
/// `locus_compile_server` (the sidecar ships inside the same bundle, so a
/// mismatch means a corrupted or foreign install — fall back to Unity).
const EXPECTED_PROTOCOL_VERSION: i64 = 3;
const EXPECTED_WRAPPER_CONTRACT_VERSION: i64 = 1;

const SERVER_DLL_NAME: &str = "LocusCompileServer.dll";

pub struct ServerHandle {
    pub client: Arc<CompileClient>,
    pub roslyn_version: String,
    pub dotnet_source: &'static str,
    pub started_at: Instant,
}

fn active_server() -> &'static tokio::sync::Mutex<Option<Arc<ServerHandle>>> {
    static ACTIVE: OnceLock<tokio::sync::Mutex<Option<Arc<ServerHandle>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Last startup failure, logged only when the message changes so a broken
/// install does not spam the log on every tool call.
fn last_start_error() -> &'static Mutex<Option<String>> {
    static LAST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(None))
}

fn note_start_error(error: &str) {
    let mut guard = match last_start_error().lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if guard.as_deref() != Some(error) {
        eprintln!("[CsharpCompile] compile server unavailable: {error}");
        *guard = Some(error.to_string());
    }
}

fn clear_start_error() {
    if let Ok(mut guard) = last_start_error().lock() {
        *guard = None;
    }
}

/// Locate the published sidecar directory (`LocusCompileServer.dll` + deps).
/// Dev builds publish to `src-tauri/gen/compile-server` via
/// `bun run compile-server:bundle`; bundles ship it as the `compile-server/`
/// resource directory next to the executable.
pub fn find_server_dll() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("compile-server"));
            candidates.push(exe_dir.join("resources").join("compile-server"));
            // target/debug or target/release -> src-tauri/gen/compile-server
            candidates.push(exe_dir.join("../../gen/compile-server"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("gen").join("compile-server"));
        candidates.push(cwd.join("src-tauri").join("gen").join("compile-server"));
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("gen")
            .join("compile-server"),
    );

    candidates
        .into_iter()
        .map(|dir| dir.join(SERVER_DLL_NAME))
        .find(|dll| dll.is_file())
}

fn logs_dir() -> Result<PathBuf, String> {
    let dir = crate::commands::persistent_config_dir()?
        .join("csharp-compile")
        .join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log dir: {e}"))?;
    Ok(dir)
}

async fn spawn_server() -> Result<Arc<ServerHandle>, String> {
    if !crate::dotnet_runtime::is_platform_supported() {
        return Err("C# compile server is not supported on this platform yet".to_string());
    }

    let server_dll = find_server_dll().ok_or_else(|| {
        "compile server binaries not found (run `bun run compile-server:bundle`)".to_string()
    })?;

    // No interactive progress here: the shared runtime is almost always
    // already present (csharp_lsp uses the same cache); a first-time
    // download happens in the background before the first compile.
    let dotnet = crate::dotnet_runtime::ensure_dotnet(&|_received, _total| {}).await?;

    let stderr_log = logs_dir()?.join("compile-server.log");
    let args = vec![server_dll.to_string_lossy().to_string()];
    let client = CompileClient::spawn(&dotnet.program, &args, &dotnet.envs, &stderr_log).await?;

    let init = client
        .request(
            "initialize",
            json!({ "protocolVersion": EXPECTED_PROTOCOL_VERSION }),
        )
        .await
        .map_err(|e| {
            client.kill_process();
            format!("compile server initialize failed: {e}")
        })?;

    let protocol = init
        .get("protocolVersion")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    let contract = init
        .get("wrapperContractVersion")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);
    if protocol != EXPECTED_PROTOCOL_VERSION || contract != EXPECTED_WRAPPER_CONTRACT_VERSION {
        client.kill_process();
        return Err(format!(
            "compile server version mismatch (protocol {protocol}, wrapper contract {contract})"
        ));
    }

    let roslyn_version = init
        .get("roslynVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    eprintln!(
        "[CsharpCompile] compile server ready (dotnet: {}, roslyn: {})",
        dotnet.source, roslyn_version
    );

    Ok(Arc::new(ServerHandle {
        client,
        roslyn_version,
        dotnet_source: dotnet.source,
        started_at: Instant::now(),
    }))
}

/// Get a live client, spawning or respawning the sidecar when needed.
pub async fn ensure_client() -> Result<Arc<CompileClient>, String> {
    let mut guard = active_server().lock().await;
    if let Some(server) = guard.as_ref() {
        if !server.client.has_exited() {
            return Ok(Arc::clone(&server.client));
        }
        eprintln!("[CsharpCompile] compile server exited; restarting");
        *guard = None;
    }

    match spawn_server().await {
        Ok(server) => {
            clear_start_error();
            let client = Arc::clone(&server.client);
            *guard = Some(server);
            super::emit_status_in_background();
            Ok(client)
        }
        Err(error) => {
            note_start_error(&error);
            super::emit_status_in_background();
            Err(error)
        }
    }
}

/// Snapshot of the running server (for status surfaces); None when stopped.
pub async fn current_status() -> Option<(String, &'static str, std::time::Duration)> {
    let guard = active_server().lock().await;
    guard.as_ref().filter(|s| !s.client.has_exited()).map(|s| {
        (
            s.roslyn_version.clone(),
            s.dotnet_source,
            s.started_at.elapsed(),
        )
    })
}

/// Stop the sidecar (feature toggled off, or app shutdown with runtime).
pub async fn shutdown() {
    let server = active_server().lock().await.take();
    if let Some(server) = server {
        server.client.shutdown().await;
    }
}

/// Best-effort synchronous kill for app-exit paths without a runtime.
pub fn kill_for_exit() {
    if let Ok(guard) = active_server().try_lock() {
        if let Some(server) = guard.as_ref() {
            server.client.kill_process();
        }
    }
}

/// Kill the current process without clearing the slot — `ensure_client`
/// notices the exit and respawns. Test hook for the crash-recovery path.
#[cfg(test)]
pub async fn kill_current_for_test() -> bool {
    let guard = active_server().lock().await;
    match guard.as_ref() {
        Some(server) => {
            server.client.kill_process();
            true
        }
        None => false,
    }
}

#[allow(dead_code)]
pub fn server_dll_available() -> bool {
    find_server_dll().is_some()
}

#[allow(dead_code)]
pub fn dll_path_for_diagnostics() -> Option<String> {
    find_server_dll().map(|p| p.to_string_lossy().to_string())
}

#[allow(dead_code)]
pub fn last_error_for_diagnostics() -> Option<String> {
    last_start_error().lock().ok().and_then(|g| g.clone())
}

#[allow(dead_code)]
pub fn server_dll_dir(dll: &Path) -> Option<&Path> {
    dll.parent()
}
