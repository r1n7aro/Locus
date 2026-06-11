//! C# semantic code analysis backed by an external Roslyn language server.
//!
//! Optional feature, toggled from the chat composer. When disabled the
//! `code_*` agent tools are filtered out of the request tool list entirely
//! (see `AgentInstance::resolve_effective_tool_names`), so the agent context
//! carries no trace of the feature.
//!
//! Architecture: one language-server process per active workspace (a Unity
//! project root). The process is spawned lazily on first use, loads the
//! Unity-generated `.sln` / `.csproj` files via MSBuild, and is replaced when
//! the active workspace changes. Server binaries and the .NET runtime are
//! downloaded on demand (see `assets`).

mod assets;
mod client;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::watch;

pub use assets::is_platform_supported;

pub const STATUS_EVENT: &str = "csharp-lsp-status";

const PROJECT_LOAD_TIMEOUT: Duration = Duration::from_secs(600);
const QUERY_READY_TIMEOUT: Duration = Duration::from_secs(75);
const WATCH_DEBOUNCE: Duration = Duration::from_millis(400);
const STATUS_EMIT_MIN_INTERVAL: Duration = Duration::from_millis(200);
const MAX_RESULT_LOCATIONS: usize = 200;

static ENABLED: AtomicBool = AtomicBool::new(false);
static APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static LAST_STATUS_EMIT: Mutex<Option<Instant>> = Mutex::new(None);

fn active_server() -> &'static tokio::sync::Mutex<Option<Arc<WorkspaceServer>>> {
    static ACTIVE: OnceLock<tokio::sync::Mutex<Option<Arc<WorkspaceServer>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Solution or project set passed to the server after `initialize`.
#[derive(Debug, Clone)]
pub enum ProjectTarget {
    Solution(PathBuf),
    Projects(Vec<PathBuf>),
}

impl ProjectTarget {
    fn display(&self) -> String {
        match self {
            ProjectTarget::Solution(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string()),
            ProjectTarget::Projects(paths) => format!("{} csproj", paths.len()),
        }
    }
}

#[derive(Debug, Clone)]
enum Phase {
    Preparing,
    Downloading {
        component: assets::AssetComponent,
        received: u64,
        total: Option<u64>,
    },
    Starting,
    Loading {
        completed: u32,
        total: Option<u32>,
    },
    Ready,
    Error(String),
}

struct WorkspaceServer {
    workspace: PathBuf,
    phase_tx: watch::Sender<Phase>,
    phase_rx: watch::Receiver<Phase>,
    client: tokio::sync::OnceCell<Arc<client::LspClient>>,
    project_file: Mutex<Option<String>>,
    project_count: Mutex<Option<u32>>,
    dotnet_source: Mutex<Option<&'static str>>,
    started_at: Instant,
    query_references: AtomicU64,
    query_definitions: AtomicU64,
    query_symbols: AtomicU64,
    /// Keeps the filesystem watcher alive for the lifetime of the server.
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl WorkspaceServer {
    fn new(workspace: PathBuf) -> Arc<Self> {
        let (phase_tx, phase_rx) = watch::channel(Phase::Preparing);
        Arc::new(WorkspaceServer {
            workspace,
            phase_tx,
            phase_rx,
            client: tokio::sync::OnceCell::new(),
            project_file: Mutex::new(None),
            project_count: Mutex::new(None),
            dotnet_source: Mutex::new(None),
            started_at: Instant::now(),
            query_references: AtomicU64::new(0),
            query_definitions: AtomicU64::new(0),
            query_symbols: AtomicU64::new(0),
            watcher: Mutex::new(None),
        })
    }

    fn phase(&self) -> Phase {
        self.phase_rx.borrow().clone()
    }

    fn set_phase(&self, phase: Phase) {
        let _ = self.phase_tx.send(phase);
        emit_status_throttled();
    }

    fn set_phase_unthrottled(&self, phase: Phase) {
        let _ = self.phase_tx.send(phase);
        emit_status_now();
    }

    async fn wait_ready(&self, timeout: Duration) -> Result<Arc<client::LspClient>, String> {
        let mut rx = self.phase_rx.clone();
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            match &*rx.borrow() {
                Phase::Ready => break,
                Phase::Error(message) => return Err(message.clone()),
                _ => {}
            }
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return Err("C# language server task ended unexpectedly".to_string());
                    }
                }
                _ = &mut deadline => {
                    return Err(format!(
                        "C# code analysis is still warming up ({}). Retry shortly.",
                        phase_progress_text(&self.phase())
                    ));
                }
            }
        }
        let client = self
            .client
            .get()
            .cloned()
            .ok_or_else(|| "C# language server is not running".to_string())?;
        if client.has_exited() {
            return Err("C# language server exited; toggle the feature to restart it".to_string());
        }
        Ok(client)
    }

    async fn shutdown(&self) {
        if let Ok(mut watcher) = self.watcher.lock() {
            *watcher = None;
        }
        if let Some(client) = self.client.get() {
            client.shutdown().await;
        }
    }
}

fn phase_progress_text(phase: &Phase) -> String {
    match phase {
        Phase::Preparing => "preparing".to_string(),
        Phase::Downloading {
            component,
            received,
            total,
        } => match total {
            Some(total) if *total > 0 => format!(
                "downloading {} {}%",
                component.as_str(),
                received * 100 / total
            ),
            _ => format!("downloading {}", component.as_str()),
        },
        Phase::Starting => "starting server".to_string(),
        Phase::Loading { completed, total } => match total {
            Some(total) if *total > 0 => format!("loading projects {completed}/{total}"),
            _ => "loading projects".to_string(),
        },
        Phase::Ready => "ready".to_string(),
        Phase::Error(message) => format!("error: {message}"),
    }
}

// ── lifecycle ────────────────────────────────────────────────────────

/// Called once from app setup with the persisted flag.
pub fn initialize(enabled: bool, app_handle: tauri::AppHandle) {
    ENABLED.store(enabled, Ordering::Relaxed);
    let _ = APP_HANDLE.set(app_handle);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Flip the feature flag. Disabling stops the running server; enabling warms
/// up the server for `workspace` in the background when one is provided.
pub async fn set_enabled(value: bool, workspace: Option<String>) {
    ENABLED.store(value, Ordering::Relaxed);
    if !value {
        let server = active_server().lock().await.take();
        if let Some(server) = server {
            server.shutdown().await;
        }
        emit_status_now();
        return;
    }
    emit_status_now();
    if let Some(workspace) = workspace.filter(|w| !w.trim().is_empty()) {
        warm_up_in_background(workspace);
    }
}

/// Start the server for `workspace` in the background so the first tool call
/// (or an app restart with the feature enabled) does not pay the full
/// download/load latency. No-op while the feature is disabled.
pub fn warm_up_in_background(workspace: String) {
    if !is_enabled() || !assets::is_platform_supported() {
        return;
    }
    tokio::spawn(async move {
        let _ = ensure_workspace_server(&workspace).await;
    });
}

/// Best-effort synchronous kill of the active server for app-exit paths.
/// The server also exits on its own when our stdin pipe closes; this just
/// avoids relying on that during MSBuild-heavy load phases.
pub fn kill_active_server_for_exit() {
    if let Ok(guard) = active_server().try_lock() {
        if let Some(server) = guard.as_ref() {
            if let Some(client) = server.client.get() {
                client.kill_process();
            }
        }
    }
}

/// Stop and restart the server for the workspace (reloads all projects).
pub async fn restart(workspace: &str) -> Result<(), String> {
    {
        let server = active_server().lock().await.take();
        if let Some(server) = server {
            server.shutdown().await;
        }
    }
    emit_status_now();
    ensure_workspace_server(workspace).await.map(|_| ())
}

/// One server for one active workspace at a time. Querying a different
/// workspace replaces the running server (full reload there). Sessions that
/// alternate between two workspaces would thrash; per-workspace instances are
/// a deliberate non-goal for now because each Roslyn server holds hundreds of
/// MB and Locus is operated against one Unity project at a time.
async fn ensure_workspace_server(workspace: &str) -> Result<Arc<WorkspaceServer>, String> {
    if !is_enabled() {
        return Err("C# code analysis is disabled".to_string());
    }
    if !assets::is_platform_supported() {
        return Err("C# code analysis is not supported on this platform yet".to_string());
    }
    let root = normalize_workspace(workspace)?;

    let mut active = active_server().lock().await;
    // Re-check under the lock: a concurrent `set_enabled(false)` may have
    // taken and shut down the active server while we were waiting.
    if !is_enabled() {
        return Err("C# code analysis is disabled".to_string());
    }
    if let Some(existing) = active.as_ref() {
        let same = paths_equal(&existing.workspace, &root);
        let dead = matches!(existing.phase(), Phase::Error(_))
            || existing
                .client
                .get()
                .map(|c| c.has_exited())
                .unwrap_or(false);
        if same && !dead {
            return Ok(Arc::clone(existing));
        }
        let old = active.take();
        if let Some(old) = old {
            tokio::spawn(async move { old.shutdown().await });
        }
    }

    let server = WorkspaceServer::new(root);
    *active = Some(Arc::clone(&server));
    drop(active);
    emit_status_now();

    let task_server = Arc::clone(&server);
    tokio::spawn(async move {
        if let Err(message) = orchestrate(&task_server).await {
            task_server.set_phase_unthrottled(Phase::Error(message));
        }
    });

    Ok(server)
}

async fn orchestrate(server: &Arc<WorkspaceServer>) -> Result<(), String> {
    server.set_phase_unthrottled(Phase::Preparing);

    let progress_server = Arc::clone(server);
    let progress = move |component, received, total| {
        progress_server.set_phase(Phase::Downloading {
            component,
            received,
            total,
        });
    };
    let resolved = assets::ensure_assets(&progress).await?;
    if let Ok(mut guard) = server.dotnet_source.lock() {
        *guard = Some(resolved.dotnet_source);
    }

    let target = discover_project_target(&server.workspace).await?;
    if let Ok(mut guard) = server.project_file.lock() {
        *guard = Some(target.display());
    }
    let project_count = match &target {
        ProjectTarget::Solution(path) => std::fs::read_to_string(path)
            .ok()
            .map(|text| text.matches("Project(\"").count() as u32),
        ProjectTarget::Projects(paths) => Some(paths.len() as u32),
    };
    if let Ok(mut guard) = server.project_count.lock() {
        *guard = project_count;
    }

    // The feature can be disabled while assets were downloading; bail before
    // spawning a process that nothing would ever shut down.
    if !is_enabled() {
        return Err("C# code analysis was disabled".to_string());
    }

    server.set_phase_unthrottled(Phase::Starting);
    let logs = assets::logs_dir()?;
    let log_tag = blake3::hash(server.workspace.to_string_lossy().as_bytes())
        .to_hex()
        .chars()
        .take(12)
        .collect::<String>();
    let stderr_log = logs.join(format!("server-{log_tag}.stderr.log"));

    let args = vec![
        resolved.server_dll.to_string_lossy().to_string(),
        "--logLevel".to_string(),
        "Information".to_string(),
        "--extensionLogDirectory".to_string(),
        logs.to_string_lossy().to_string(),
        "--stdio".to_string(),
    ];
    let lsp = client::LspClient::spawn(
        &resolved.dotnet_program,
        &args,
        &resolved.envs,
        &stderr_log,
    )
    .await?;
    server
        .client
        .set(Arc::clone(&lsp))
        .map_err(|_| "server already initialized".to_string())?;

    lsp.initialize_workspace(&server.workspace, &target).await?;
    start_file_watcher(server, &lsp);

    server.set_phase_unthrottled(Phase::Loading {
        completed: 0,
        total: project_count,
    });
    let progress_server = Arc::clone(server);
    let loaded = lsp
        .wait_project_loaded(PROJECT_LOAD_TIMEOUT, move |completed| {
            progress_server.set_phase(Phase::Loading {
                completed,
                total: project_count,
            });
        })
        .await;
    if !loaded {
        let detail = lsp
            .last_server_error()
            .map(|e| format!(": {e}"))
            .unwrap_or_default();
        lsp.shutdown().await;
        return Err(format!("Project loading did not complete{detail}"));
    }
    // Disabled mid-load: this server may already have been detached from the
    // active slot, so shut the process down ourselves.
    if !is_enabled() {
        lsp.shutdown().await;
        return Err("C# code analysis was disabled".to_string());
    }

    server.set_phase_unthrottled(Phase::Ready);
    Ok(())
}

// ── project discovery ────────────────────────────────────────────────

/// C# snippet that asks the Unity editor to (re)generate `.sln`/`.csproj`.
/// Prefers the public `CodeEditor.CurrentEditor.SyncAll`, falling back to the
/// internal-but-stable `UnityEditor.SyncVS.SyncSolution`.
const UNITY_SYNC_SOLUTION_CODE: &str = r#"
try {
    Unity.CodeEditor.CodeEditor.CurrentEditor.SyncAll();
    print("project files synced via CodeEditor");
} catch (System.Exception e) {
    var t = System.Type.GetType("UnityEditor.SyncVS,UnityEditor");
    var m = t == null ? null : t.GetMethod("SyncSolution",
        System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.NonPublic);
    if (m != null) { m.Invoke(null, null); print("project files synced via SyncVS"); }
    else { print("sync failed: " + e.Message); }
}
"#;

fn scan_project_target(root: &Path) -> Option<ProjectTarget> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut solutions: Vec<PathBuf> = Vec::new();
    let mut projects: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("sln") => solutions.push(path),
            Some(ext) if ext.eq_ignore_ascii_case("csproj") => projects.push(path),
            _ => {}
        }
    }
    if !solutions.is_empty() {
        // Unity names the solution after the project directory; prefer that
        // one when several are present.
        let dir_name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        solutions.sort();
        let preferred = solutions
            .iter()
            .find(|p| {
                p.file_stem()
                    .map(|s| s.to_string_lossy().to_lowercase() == dir_name)
                    .unwrap_or(false)
            })
            .cloned()
            .unwrap_or_else(|| solutions[0].clone());
        return Some(ProjectTarget::Solution(preferred));
    }
    if !projects.is_empty() {
        projects.sort();
        return Some(ProjectTarget::Projects(projects));
    }
    None
}

async fn discover_project_target(root: &Path) -> Result<ProjectTarget, String> {
    if let Some(target) = scan_project_target(root) {
        return Ok(target);
    }

    // No project files yet — ask a connected Unity editor to generate them.
    let workspace = root.to_string_lossy().to_string();
    if crate::unity_bridge::is_unity_project(&workspace) {
        let (connected, _, _) = crate::unity_bridge::query_unity_status(&workspace).await;
        if connected {
            let _ = crate::unity_bridge::unity_execute_code(&workspace, UNITY_SYNC_SOLUTION_CODE)
                .await;
            for _ in 0..10 {
                if let Some(target) = scan_project_target(root) {
                    return Ok(target);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        return Err(
            "No .sln/.csproj found in the workspace. Open the Unity project (with an external \
             script editor configured) so the project files can be generated, then retry."
                .to_string(),
        );
    }
    Err("No .sln/.csproj found in the workspace".to_string())
}

// ── file watching ────────────────────────────────────────────────────

fn watched_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some(ext) if ext.eq_ignore_ascii_case("cs")
            || ext.eq_ignore_ascii_case("csproj")
            || ext.eq_ignore_ascii_case("sln")
    )
}

fn start_file_watcher(server: &Arc<WorkspaceServer>, lsp: &Arc<client::LspClient>) {
    use notify::Watcher;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(PathBuf, u8)>();
    let event_tx = tx.clone();
    let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else { return };
        let kind: u8 = match event.kind {
            notify::EventKind::Create(_) => 1,
            notify::EventKind::Modify(_) => 2,
            notify::EventKind::Remove(_) => 3,
            _ => return,
        };
        for path in event.paths {
            if watched_extension(&path) {
                let _ = event_tx.send((path, kind));
            }
        }
    });

    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            eprintln!("[CsharpLsp] file watcher unavailable: {error}");
            return;
        }
    };

    let root = server.workspace.clone();
    // Project files live at the root; sources under Assets/ and Packages/.
    // Library/ churns constantly and its PackageCache is effectively
    // immutable during a session, so it is deliberately not watched.
    let _ = watcher.watch(&root, notify::RecursiveMode::NonRecursive);
    for sub in ["Assets", "Packages"] {
        let dir = root.join(sub);
        if dir.is_dir() {
            let _ = watcher.watch(&dir, notify::RecursiveMode::Recursive);
        }
    }
    if let Ok(mut guard) = server.watcher.lock() {
        *guard = Some(watcher);
    }

    let lsp = Arc::clone(lsp);
    tokio::spawn(async move {
        loop {
            let Some(first) = rx.recv().await else { return };
            let mut batch: HashMap<PathBuf, u8> = HashMap::new();
            batch.insert(first.0, first.1);
            let window = tokio::time::sleep(WATCH_DEBOUNCE);
            tokio::pin!(window);
            loop {
                tokio::select! {
                    more = rx.recv() => match more {
                        Some((path, kind)) => { batch.insert(path, kind); }
                        None => break,
                    },
                    _ = &mut window => break,
                }
            }
            let mut changes = Vec::with_capacity(batch.len());
            for (path, kind) in batch {
                if let Ok(uri) = client::path_to_uri(&path) {
                    changes.push((uri, kind));
                }
                // Keep documents we opened in sync with external edits and
                // release them on deletion; never open new ones from here.
                if kind == 2 && path.is_file() {
                    let _ = lsp.sync_document_if_open(&path).await;
                } else if kind == 3 {
                    let _ = lsp.close_document_if_open(&path).await;
                }
            }
            let _ = lsp.notify_watched_files(changes).await;
            if lsp.has_exited() {
                return;
            }
        }
    });
}

// ── queries ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeLocation {
    /// Workspace-relative display path (forward slashes) or a descriptive
    /// label for non-file locations (e.g. decompiled metadata).
    pub path: String,
    /// 1-based line number; 0 when unknown.
    pub line: u32,
    /// Trimmed source line text, when resolvable.
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSymbol {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub path: String,
    pub line: u32,
}

pub struct ReferencesResult {
    pub locations: Vec<CodeLocation>,
    pub truncated: bool,
}

async fn ready_client(
    workspace: &str,
) -> Result<(Arc<WorkspaceServer>, Arc<client::LspClient>), String> {
    let server = ensure_workspace_server(workspace).await?;
    let client = server.wait_ready(QUERY_READY_TIMEOUT).await?;
    Ok((server, client))
}

fn resolve_file_path(workspace: &Path, file_path: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(file_path);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        workspace.join(candidate)
    };
    let absolute = dunce::simplified(&absolute).to_path_buf();
    if !absolute.is_file() {
        return Err(format!("File not found: {}", absolute.display()));
    }
    Ok(absolute)
}

/// Find `symbol` on the given 1-based line and return the LSP position
/// (0-based line, UTF-16 column) of its first whole-word occurrence.
fn locate_symbol_position(text: &str, line: u32, symbol: &str) -> Result<(u32, u32), String> {
    if line == 0 {
        return Err("line is 1-based".to_string());
    }
    let line_index = (line - 1) as usize;
    let line_text = text
        .split('\n')
        .nth(line_index)
        .ok_or_else(|| format!("File has fewer than {line} lines"))?
        .trim_end_matches('\r');

    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut fallback: Option<usize> = None;
    let mut search_from = 0;
    while let Some(found) = line_text[search_from..].find(symbol) {
        let start = search_from + found;
        let end = start + symbol.len();
        let before_ok = line_text[..start]
            .chars()
            .next_back()
            .map(|c| !is_ident(c))
            .unwrap_or(true);
        let after_ok = line_text[end..]
            .chars()
            .next()
            .map(|c| !is_ident(c))
            .unwrap_or(true);
        if before_ok && after_ok {
            let column = line_text[..start].encode_utf16().count() as u32;
            return Ok(((line - 1), column));
        }
        if fallback.is_none() {
            fallback = Some(start);
        }
        search_from = end;
    }
    if let Some(start) = fallback {
        let column = line_text[..start].encode_utf16().count() as u32;
        return Ok(((line - 1), column));
    }
    let mut preview = line_text.trim().to_string();
    if preview.chars().count() > 120 {
        preview = preview.chars().take(120).collect::<String>() + "…";
    }
    Err(format!(
        "Symbol '{symbol}' not found on line {line}. Line content: {preview}"
    ))
}

fn display_path(workspace: &Path, path: &Path) -> String {
    let relative = relative_to(workspace, path).unwrap_or_else(|| path.to_path_buf());
    relative.to_string_lossy().replace('\\', "/")
}

fn relative_to(base: &Path, path: &Path) -> Option<PathBuf> {
    if cfg!(windows) {
        let base_lower = base.to_string_lossy().to_lowercase();
        let path_text = path.to_string_lossy().to_string();
        let path_lower = path_text.to_lowercase();
        let stripped = path_lower.strip_prefix(&base_lower)?;
        // The match must end on a path-component boundary, otherwise
        // `F:\Game` would "contain" `F:\GameTools\A.cs`.
        if !stripped.starts_with(['\\', '/']) && !base_lower.ends_with(['\\', '/']) {
            return None;
        }
        let stripped = stripped.trim_start_matches(['\\', '/']);
        if stripped.is_empty() {
            return None;
        }
        let offset = path_text.len() - stripped.len();
        Some(PathBuf::from(&path_text[offset..]))
    } else {
        path.strip_prefix(base).ok().map(|p| p.to_path_buf())
    }
}

/// Convert raw LSP locations into display entries with line text, grouped and
/// capped for tool output.
fn collect_locations(
    workspace: &Path,
    raw: &[serde_json::Value],
) -> (Vec<CodeLocation>, bool) {
    let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut seen: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
    let mut locations = Vec::new();
    let mut truncated = false;

    for item in raw {
        let Some(uri) = item.get("uri").and_then(|u| u.as_str()) else {
            continue;
        };
        let line0 = item
            .pointer("/range/start/line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32;
        let (path_display, display_line, text) = match client::uri_to_path(uri) {
            Some(path) => {
                let lines = file_cache.entry(path.clone()).or_insert_with(|| {
                    client::read_text_lossy(&path)
                        .map(|t| t.split('\n').map(|l| l.trim_end_matches('\r').to_string()).collect())
                        .unwrap_or_default()
                });
                let text = lines
                    .get(line0 as usize)
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                (display_path(workspace, &path), line0 + 1, text)
            }
            // Decompiled metadata / non-file schemes: no meaningful line.
            None => (format!("[external] {uri}"), 0, String::new()),
        };
        let key = (path_display.to_lowercase(), display_line);
        if !seen.insert(key) {
            continue;
        }
        if locations.len() >= MAX_RESULT_LOCATIONS {
            truncated = true;
            break;
        }
        locations.push(CodeLocation {
            path: path_display,
            line: display_line,
            text,
        });
    }

    locations.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    (locations, truncated)
}

pub async fn find_references(
    workspace: &str,
    file_path: &str,
    line: u32,
    symbol: &str,
    include_declaration: bool,
) -> Result<ReferencesResult, String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let text = client::read_text_lossy(&absolute)?;
    let (line0, column) = locate_symbol_position(&text, line, symbol)?;
    let uri = lsp.sync_document(&absolute).await?;

    let result = lsp
        .request(
            "textDocument/references",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line0, "character": column },
                "context": { "includeDeclaration": include_declaration }
            }),
        )
        .await?;
    let raw = result.as_array().cloned().unwrap_or_default();
    let (locations, truncated) = collect_locations(&server.workspace, &raw);
    server.query_references.fetch_add(1, Ordering::Relaxed);
    emit_status_throttled();
    Ok(ReferencesResult {
        locations,
        truncated,
    })
}

pub async fn goto_definition(
    workspace: &str,
    file_path: &str,
    line: u32,
    symbol: &str,
) -> Result<Vec<CodeLocation>, String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let text = client::read_text_lossy(&absolute)?;
    let (line0, column) = locate_symbol_position(&text, line, symbol)?;
    let uri = lsp.sync_document(&absolute).await?;

    let result = lsp
        .request(
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line0, "character": column }
            }),
        )
        .await?;
    // Definition responses may be Location, Location[] or LocationLink[].
    let mut raw: Vec<serde_json::Value> = Vec::new();
    match result {
        serde_json::Value::Array(items) => {
            for item in items {
                if item.get("targetUri").is_some() {
                    raw.push(serde_json::json!({
                        "uri": item.get("targetUri").cloned().unwrap_or_default(),
                        "range": item.get("targetSelectionRange").or(item.get("targetRange")).cloned().unwrap_or_default(),
                    }));
                } else {
                    raw.push(item);
                }
            }
        }
        serde_json::Value::Object(_) => raw.push(result),
        _ => {}
    }
    let (locations, _) = collect_locations(&server.workspace, &raw);
    server.query_definitions.fetch_add(1, Ordering::Relaxed);
    emit_status_throttled();
    Ok(locations)
}

fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        3 => "Namespace",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        _ => "Symbol",
    }
}

pub async fn workspace_symbols(
    workspace: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<CodeSymbol>, String> {
    let (server, lsp) = ready_client(workspace).await?;
    let result = lsp
        .request(
            "workspace/symbol",
            serde_json::json!({ "query": query }),
        )
        .await?;
    let items = result.as_array().cloned().unwrap_or_default();
    let mut symbols = Vec::new();
    for item in items.iter().take(limit.max(1)) {
        let name = item
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .to_string();
        let kind = item.get("kind").and_then(|k| k.as_u64()).unwrap_or(0);
        let container = item
            .get("containerName")
            .and_then(|c| c.as_str())
            .filter(|c| !c.is_empty())
            .map(|c| c.to_string());
        let uri = item
            .pointer("/location/uri")
            .and_then(|u| u.as_str())
            .unwrap_or_default();
        let line0 = item
            .pointer("/location/range/start/line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32;
        let (path, line) = match client::uri_to_path(uri) {
            Some(path) => (display_path(&server.workspace, &path), line0 + 1),
            None => (format!("[external] {uri}"), 0),
        };
        symbols.push(CodeSymbol {
            name,
            kind: symbol_kind_name(kind).to_string(),
            container,
            path,
            line,
        });
    }
    server.query_symbols.fetch_add(1, Ordering::Relaxed);
    emit_status_throttled();
    Ok(symbols)
}

// ── status / events ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsharpLspStatusPayload {
    pub enabled: bool,
    pub supported: bool,
    pub phase: String,
    pub message: Option<String>,
    pub download_component: Option<String>,
    pub download_received: Option<u64>,
    pub download_total: Option<u64>,
    pub workspace: Option<String>,
    pub project_file: Option<String>,
    pub server_version: String,
    pub dotnet_source: Option<String>,
    pub project_count: Option<u32>,
    pub loaded_projects: Option<u32>,
    pub open_documents: Option<u32>,
    pub query_references: u64,
    pub query_definitions: u64,
    pub query_symbols: u64,
    pub uptime_secs: Option<u64>,
}

fn build_status(server: Option<&Arc<WorkspaceServer>>) -> CsharpLspStatusPayload {
    let enabled = is_enabled();
    let supported = assets::is_platform_supported();
    let mut payload = CsharpLspStatusPayload {
        enabled,
        supported,
        phase: if !enabled {
            "disabled".to_string()
        } else {
            "idle".to_string()
        },
        message: None,
        download_component: None,
        download_received: None,
        download_total: None,
        workspace: None,
        project_file: None,
        server_version: assets::SERVER_VERSION.to_string(),
        dotnet_source: None,
        project_count: None,
        loaded_projects: None,
        open_documents: None,
        query_references: 0,
        query_definitions: 0,
        query_symbols: 0,
        uptime_secs: None,
    };
    if !enabled {
        return payload;
    }
    let Some(server) = server else {
        return payload;
    };
    payload.workspace = Some(server.workspace.to_string_lossy().to_string());
    payload.project_file = server.project_file.lock().ok().and_then(|g| g.clone());
    payload.dotnet_source = server
        .dotnet_source
        .lock()
        .ok()
        .and_then(|g| g.map(|s| s.to_string()));
    payload.project_count = server.project_count.lock().ok().and_then(|g| *g);
    payload.open_documents = server
        .client
        .get()
        .map(|client| client.open_document_count() as u32);
    payload.query_references = server.query_references.load(Ordering::Relaxed);
    payload.query_definitions = server.query_definitions.load(Ordering::Relaxed);
    payload.query_symbols = server.query_symbols.load(Ordering::Relaxed);
    payload.uptime_secs = Some(server.started_at.elapsed().as_secs());
    match server.phase() {
        Phase::Preparing => payload.phase = "preparing".to_string(),
        Phase::Downloading {
            component,
            received,
            total,
        } => {
            payload.phase = "downloading".to_string();
            payload.download_component = Some(component.as_str().to_string());
            payload.download_received = Some(received);
            payload.download_total = total;
        }
        Phase::Starting => payload.phase = "starting".to_string(),
        Phase::Loading { completed, total } => {
            payload.phase = "loading".to_string();
            payload.loaded_projects = Some(completed);
            if payload.project_count.is_none() {
                payload.project_count = total;
            }
        }
        Phase::Ready => payload.phase = "ready".to_string(),
        Phase::Error(message) => {
            payload.phase = "error".to_string();
            payload.message = Some(message);
        }
    }
    payload
}

/// Snapshot of the current feature status for the UI.
pub async fn status() -> CsharpLspStatusPayload {
    let active = active_server().lock().await;
    build_status(active.as_ref())
}

fn emit_status_with(payload: CsharpLspStatusPayload) {
    if let Some(app) = APP_HANDLE.get() {
        let _ = app.emit(STATUS_EVENT, payload);
    }
}

fn emit_status_now() {
    if let Ok(mut last) = LAST_STATUS_EMIT.lock() {
        *last = Some(Instant::now());
    }
    tokio::spawn(async {
        let payload = status().await;
        emit_status_with(payload);
    });
}

fn emit_status_throttled() {
    if let Ok(mut last) = LAST_STATUS_EMIT.lock() {
        if let Some(previous) = *last {
            if previous.elapsed() < STATUS_EMIT_MIN_INTERVAL {
                return;
            }
        }
        *last = Some(Instant::now());
    }
    tokio::spawn(async {
        let payload = status().await;
        emit_status_with(payload);
    });
}

// ── helpers ──────────────────────────────────────────────────────────

fn normalize_workspace(workspace: &str) -> Result<PathBuf, String> {
    let trimmed = workspace.trim();
    if trimmed.is_empty() {
        return Err("A workspace directory is required for C# code analysis".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_dir() {
        return Err(format!("Workspace directory not found: {trimmed}"));
    }
    Ok(dunce::simplified(&path).to_path_buf())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}
