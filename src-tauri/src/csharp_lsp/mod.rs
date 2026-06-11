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
        // Leave the user's project tree as we found it.
        remove_analyzer_props(&self.workspace);
    }
}

// ── Unity analyzers (Directory.Build.props injection) ────────────────

const ANALYZER_PROPS_MARKER: &str = "locus:unity-analyzers";

fn analyzer_props_path(workspace: &Path) -> PathBuf {
    workspace.join("Directory.Build.props")
}

/// Write (or refresh) the marker-tagged Directory.Build.props pointing
/// MSBuild at the downloaded Microsoft.Unity.Analyzers.dll. The file only
/// affects IDE/language-server project loads; Unity's own compilation never
/// reads it. A pre-existing user-owned file (no marker) is left untouched.
fn write_analyzer_props(workspace: &Path, dll: &Path) -> Result<(), String> {
    let path = analyzer_props_path(workspace);
    let existing = std::fs::read_to_string(&path).ok();
    if let Some(existing) = existing.as_deref() {
        if !existing.contains(ANALYZER_PROPS_MARKER) {
            return Err(format!(
                "{} already exists and was not created by Locus",
                path.display()
            ));
        }
    }
    let content = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
            "<Project>\n",
            "  <!-- {marker} : generated by Locus while its C# code analysis feature runs.\n",
            "       Adds Microsoft.Unity.Analyzers (UNT* diagnostics) to IDE / language-server\n",
            "       loads of the Unity-generated projects. Unity's own compilation ignores\n",
            "       this file. Safe to delete; Locus recreates it while the feature is on. -->\n",
            "  <ItemGroup>\n",
            "    <Analyzer Include=\"{dll}\" />\n",
            "  </ItemGroup>\n",
            "</Project>\n"
        ),
        marker = ANALYZER_PROPS_MARKER,
        dll = dll.display()
    );
    if existing.as_deref() == Some(content.as_str()) {
        return Ok(());
    }
    std::fs::write(&path, content)
        .map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

/// Remove the marker-tagged props file; user-owned files are left alone.
fn remove_analyzer_props(workspace: &Path) {
    let path = analyzer_props_path(workspace);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return;
    };
    if existing.contains(ANALYZER_PROPS_MARKER) {
        let _ = std::fs::remove_file(&path);
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
            remove_analyzer_props(&server.workspace);
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

    // Unity-specific analyzers ride into the workspace via a marker-tagged
    // Directory.Build.props so the Unity-generated csproj pick them up when
    // MSBuild evaluates them below. Must happen before solution/open.
    // Failures are soft: the server is still useful without UNT* rules.
    if crate::code_tools::unity_analyzers_enabled() {
        match assets::ensure_unity_analyzers(&progress).await {
            Ok(dll) => {
                if let Err(error) = write_analyzer_props(&server.workspace, &dll) {
                    eprintln!("[CsharpLsp] Unity analyzers not injected: {error}");
                }
            }
            Err(error) => eprintln!("[CsharpLsp] Unity analyzers unavailable: {error}"),
        }
    } else {
        remove_analyzer_props(&server.workspace);
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
    pub anchor: SymbolAnchor,
}

async fn ready_client(
    workspace: &str,
) -> Result<(Arc<WorkspaceServer>, Arc<client::LspClient>), String> {
    let server = ensure_workspace_server(workspace).await?;
    let client = server.wait_ready(QUERY_READY_TIMEOUT).await?;
    Ok((server, client))
}

/// Failure shapes that mean the server process died under the query (it has
/// been observed exiting silently right after rapid file edits).
fn is_server_exit_error(error: &str) -> bool {
    error.contains("language server exited")
        || error.contains("write failed")
        || error.contains("flush failed")
        || error.contains("dropped the request")
}

/// Run a query; when it fails because the server process died mid-flight,
/// retry once. The retried call goes through `ensure_workspace_server`, which
/// detects the dead client and spawns a replacement (the retry then waits up
/// to `QUERY_READY_TIMEOUT` for it to load).
async fn retry_once_on_server_exit<T, Fut>(make: impl Fn() -> Fut) -> Result<T, String>
where
    Fut: std::future::Future<Output = Result<T, String>>,
{
    match make().await {
        Err(error) if is_server_exit_error(&error) => {
            eprintln!("[CsharpLsp] query hit a dead server; restarting and retrying once ({error})");
            make().await
        }
        result => result,
    }
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

/// Where a position-based symbol query was actually anchored after resolving
/// the caller's (forgiving) line hint.
#[derive(Debug, Clone, Copy)]
pub struct SymbolAnchor {
    /// 1-based line the query ran against.
    pub line: u32,
    /// 1-based line hint the caller supplied, if any.
    pub requested_line: Option<u32>,
}

impl SymbolAnchor {
    pub fn adjusted(&self) -> bool {
        self.requested_line
            .map(|requested| requested != self.line)
            .unwrap_or(false)
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// First whole-word occurrence of `symbol` in `line_text` as a UTF-16 column.
/// With `allow_substring` a plain substring hit is accepted as fallback.
fn symbol_column_in_line(line_text: &str, symbol: &str, allow_substring: bool) -> Option<u32> {
    let mut fallback: Option<usize> = None;
    let mut search_from = 0;
    while let Some(found) = line_text[search_from..].find(symbol) {
        let start = search_from + found;
        let end = start + symbol.len();
        let before_ok = line_text[..start]
            .chars()
            .next_back()
            .map(|c| !is_ident_char(c))
            .unwrap_or(true);
        let after_ok = line_text[end..]
            .chars()
            .next()
            .map(|c| !is_ident_char(c))
            .unwrap_or(true);
        if before_ok && after_ok {
            return Some(line_text[..start].encode_utf16().count() as u32);
        }
        if fallback.is_none() {
            fallback = Some(start);
        }
        search_from = end;
    }
    match fallback {
        Some(start) if allow_substring => Some(line_text[..start].encode_utf16().count() as u32),
        _ => None,
    }
}

/// Resolve `symbol` to an LSP position (0-based line, UTF-16 column).
///
/// The line hint is forgiving — agents rarely know exact line numbers. When
/// the symbol is not on the hinted line, the nearest whole-word occurrence in
/// the file is used instead. Without a hint the occurrence must be unique;
/// multiple candidates are listed back so one retry can disambiguate.
fn locate_symbol_position(
    text: &str,
    line_hint: Option<u32>,
    symbol: &str,
) -> Result<(u32, u32, SymbolAnchor), String> {
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();

    if let Some(hint) = line_hint {
        if hint == 0 {
            return Err("line is 1-based".to_string());
        }
        if let Some(line_text) = lines.get((hint - 1) as usize) {
            if let Some(column) = symbol_column_in_line(line_text, symbol, true) {
                return Ok((
                    hint - 1,
                    column,
                    SymbolAnchor {
                        line: hint,
                        requested_line: Some(hint),
                    },
                ));
            }
        }
        // Not on the hinted line (or hint beyond EOF) — fall through to the
        // file-wide scan.
    }

    // Whole-word occurrences across the file. Comment-looking lines are
    // skipped: the LSP resolves nothing useful at a comment position.
    let mut occurrences: Vec<(usize, u32)> = Vec::new();
    for (index, line_text) in lines.iter().enumerate() {
        let trimmed = line_text.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            continue;
        }
        if let Some(column) = symbol_column_in_line(line_text, symbol, false) {
            occurrences.push((index, column));
        }
    }

    match (occurrences.len(), line_hint) {
        (0, hint) => {
            let detail = hint
                .and_then(|h| lines.get((h - 1) as usize).map(|l| (h, l.trim())))
                .map(|(h, content)| {
                    let mut preview = content.to_string();
                    if preview.chars().count() > 120 {
                        preview = preview.chars().take(120).collect::<String>() + "…";
                    }
                    format!(" Line {h} content: {preview}")
                })
                .unwrap_or_default();
            Err(format!("Symbol '{symbol}' not found in the file.{detail}"))
        }
        (_, Some(hint)) => {
            let hint_index = (hint - 1) as usize;
            let (index, column) = occurrences
                .iter()
                .min_by_key(|(index, _)| index.abs_diff(hint_index))
                .copied()
                .unwrap();
            Ok((
                index as u32,
                column,
                SymbolAnchor {
                    line: index as u32 + 1,
                    requested_line: Some(hint),
                },
            ))
        }
        (1, None) => {
            let (index, column) = occurrences[0];
            Ok((
                index as u32,
                column,
                SymbolAnchor {
                    line: index as u32 + 1,
                    requested_line: None,
                },
            ))
        }
        (count, None) => {
            let mut listing = String::new();
            for (index, _) in occurrences.iter().take(10) {
                listing.push_str(&format!("\n  {}: {}", index + 1, lines[*index].trim()));
            }
            if count > 10 {
                listing.push_str(&format!("\n  … and {} more", count - 10));
            }
            Err(format!(
                "Symbol '{symbol}' appears on {count} lines; pass `line` to pick one:{listing}"
            ))
        }
    }
}

fn display_path(workspace: &Path, path: &Path) -> String {
    if let Some(relative) = relative_to(workspace, path) {
        return relative.to_string_lossy().replace('\\', "/");
    }
    let text = path.to_string_lossy();
    // Roslyn materializes decompiled sources in a temp MetadataAsSource tree;
    // the raw path is noise — the (well-known) file name is what matters.
    if text.contains("MetadataAsSource") {
        if let Some(name) = path.file_name() {
            return format!("[decompiled] {}", name.to_string_lossy());
        }
    }
    text.replace('\\', "/")
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
    line: Option<u32>,
    symbol: &str,
    include_declaration: bool,
) -> Result<ReferencesResult, String> {
    retry_once_on_server_exit(|| {
        find_references_attempt(workspace, file_path, line, symbol, include_declaration)
    })
    .await
}

async fn find_references_attempt(
    workspace: &str,
    file_path: &str,
    line: Option<u32>,
    symbol: &str,
    include_declaration: bool,
) -> Result<ReferencesResult, String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let text = client::read_text_lossy(&absolute)?;
    let (line0, column, anchor) = locate_symbol_position(&text, line, symbol)?;
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
        anchor,
    })
}

pub async fn goto_definition(
    workspace: &str,
    file_path: &str,
    line: Option<u32>,
    symbol: &str,
) -> Result<(Vec<CodeLocation>, SymbolAnchor), String> {
    retry_once_on_server_exit(|| goto_definition_attempt(workspace, file_path, line, symbol))
        .await
}

async fn goto_definition_attempt(
    workspace: &str,
    file_path: &str,
    line: Option<u32>,
    symbol: &str,
) -> Result<(Vec<CodeLocation>, SymbolAnchor), String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let text = client::read_text_lossy(&absolute)?;
    let (line0, column, anchor) = locate_symbol_position(&text, line, symbol)?;
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
    Ok((locations, anchor))
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
    retry_once_on_server_exit(|| workspace_symbols_attempt(workspace, query, limit)).await
}

async fn workspace_symbols_attempt(
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

// ── hover ────────────────────────────────────────────────────────────

/// Flatten the LSP hover `contents` union (MarkedString | MarkedString[] |
/// MarkupContent) into plain text.
fn hover_contents_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Object(map) => map
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(hover_contents_to_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// Signature/type/documentation for the symbol at a position, as the server's
/// (markdown-ish) hover text. `None` when the server has nothing to say.
pub async fn hover(
    workspace: &str,
    file_path: &str,
    line: Option<u32>,
    symbol: &str,
) -> Result<(Option<String>, SymbolAnchor), String> {
    retry_once_on_server_exit(|| hover_attempt(workspace, file_path, line, symbol)).await
}

async fn hover_attempt(
    workspace: &str,
    file_path: &str,
    line: Option<u32>,
    symbol: &str,
) -> Result<(Option<String>, SymbolAnchor), String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let text = client::read_text_lossy(&absolute)?;
    let (line0, column, anchor) = locate_symbol_position(&text, line, symbol)?;
    let uri = lsp.sync_document(&absolute).await?;

    let result = lsp
        .request(
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line0, "character": column }
            }),
        )
        .await?;
    emit_status_throttled();
    if result.is_null() {
        return Ok((None, anchor));
    }
    let contents = result
        .get("contents")
        .map(hover_contents_to_text)
        .unwrap_or_default();
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        Ok((None, anchor))
    } else {
        Ok((Some(trimmed.to_string()), anchor))
    }
}

// ── diagnostics ──────────────────────────────────────────────────────

const WORKSPACE_DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub struct CodeDiagnostic {
    /// Workspace-relative display path (forward slashes).
    pub path: String,
    /// 1-based line; 0 when unknown.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
    /// LSP severity: 1 error, 2 warning, 3 info, 4 hint.
    pub severity: u8,
    /// Diagnostic id, e.g. `CS0103` or `UNT0010`.
    pub code: Option<String>,
    pub message: String,
}

pub fn severity_label(severity: u8) -> &'static str {
    match severity {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    }
}

fn push_diagnostic_items(
    workspace: &Path,
    uri: &str,
    items: &[serde_json::Value],
    out: &mut Vec<CodeDiagnostic>,
) {
    let path = match client::uri_to_path(uri) {
        Some(path) => display_path(workspace, &path),
        None => format!("[external] {uri}"),
    };
    for item in items {
        let line = item
            .pointer("/range/start/line")
            .and_then(|l| l.as_u64())
            .map(|l| l as u32 + 1)
            .unwrap_or(0);
        let column = item
            .pointer("/range/start/character")
            .and_then(|c| c.as_u64())
            .map(|c| c as u32 + 1)
            .unwrap_or(0);
        let severity = item
            .get("severity")
            .and_then(|s| s.as_u64())
            .unwrap_or(3)
            .clamp(1, 4) as u8;
        let code = item
            .get("code")
            .map(|code| match code {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .filter(|code| !code.is_empty());
        let message = item
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if message.is_empty() {
            continue;
        }
        out.push(CodeDiagnostic {
            path: path.clone(),
            line,
            column,
            severity,
            code,
            message,
        });
    }
}

fn dedupe_and_sort_diagnostics(diagnostics: &mut Vec<CodeDiagnostic>) {
    let mut seen: std::collections::HashSet<(String, u32, u32, String, String)> =
        std::collections::HashSet::new();
    diagnostics.retain(|d| {
        seen.insert((
            d.path.to_lowercase(),
            d.line,
            d.column,
            d.code.clone().unwrap_or_default(),
            d.message.clone(),
        ))
    });
    diagnostics.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
    });
}

/// Pull diagnostics (compiler + loaded analyzers) for a single document.
pub async fn document_diagnostics(
    workspace: &str,
    file_path: &str,
) -> Result<Vec<CodeDiagnostic>, String> {
    retry_once_on_server_exit(|| document_diagnostics_attempt(workspace, file_path)).await
}

async fn document_diagnostics_attempt(
    workspace: &str,
    file_path: &str,
) -> Result<Vec<CodeDiagnostic>, String> {
    let (server, lsp) = ready_client(workspace).await?;
    let absolute = resolve_file_path(&server.workspace, file_path)?;
    let uri = lsp.sync_document(&absolute).await?;

    let registrations = lsp.diagnostic_registrations();
    let identifiers: Vec<Option<String>> = if registrations.is_empty() {
        vec![None]
    } else {
        registrations
            .into_iter()
            .map(|registration| registration.identifier)
            .collect()
    };

    let mut diagnostics = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut succeeded = false;
    for identifier in identifiers {
        let mut params = serde_json::json!({ "textDocument": { "uri": uri } });
        if let Some(identifier) = identifier {
            params["identifier"] = serde_json::json!(identifier);
        }
        match lsp.request("textDocument/diagnostic", params).await {
            Ok(result) => {
                succeeded = true;
                if let Some(items) = result.get("items").and_then(|i| i.as_array()) {
                    push_diagnostic_items(&server.workspace, &uri, items, &mut diagnostics);
                }
            }
            Err(error) => errors.push(error),
        }
    }
    if !succeeded {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "diagnostic request failed".to_string()));
    }
    dedupe_and_sort_diagnostics(&mut diagnostics);
    emit_status_throttled();
    Ok(diagnostics)
}

/// Pull diagnostics for the whole solution. Requires full-solution background
/// analysis scope (answered in the configuration handler) and can be slow on
/// first call while the server warms its caches.
pub async fn workspace_diagnostics(workspace: &str) -> Result<Vec<CodeDiagnostic>, String> {
    retry_once_on_server_exit(|| workspace_diagnostics_attempt(workspace)).await
}

async fn workspace_diagnostics_attempt(workspace: &str) -> Result<Vec<CodeDiagnostic>, String> {
    let (server, lsp) = ready_client(workspace).await?;

    let registrations: Vec<_> = lsp
        .diagnostic_registrations()
        .into_iter()
        .filter(|registration| registration.workspace_diagnostics)
        .collect();
    let identifiers: Vec<Option<String>> = if registrations.is_empty() {
        vec![None]
    } else {
        registrations
            .into_iter()
            .map(|registration| registration.identifier)
            .collect()
    };

    let mut diagnostics = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut succeeded = false;
    for identifier in identifiers {
        let mut params = serde_json::json!({ "previousResultIds": [] });
        if let Some(identifier) = identifier {
            params["identifier"] = serde_json::json!(identifier);
        }
        match lsp
            .request_with_timeout("workspace/diagnostic", params, WORKSPACE_DIAGNOSTIC_TIMEOUT)
            .await
        {
            Ok(result) => {
                succeeded = true;
                let reports = result
                    .get("items")
                    .and_then(|i| i.as_array())
                    .cloned()
                    .unwrap_or_default();
                for report in &reports {
                    let Some(uri) = report.get("uri").and_then(|u| u.as_str()) else {
                        continue;
                    };
                    if let Some(items) = report.get("items").and_then(|i| i.as_array()) {
                        push_diagnostic_items(&server.workspace, uri, items, &mut diagnostics);
                    }
                }
            }
            Err(error) => errors.push(error),
        }
    }
    if !succeeded {
        return Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| "workspace diagnostic request failed".to_string()));
    }
    dedupe_and_sort_diagnostics(&mut diagnostics);
    emit_status_throttled();
    Ok(diagnostics)
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
