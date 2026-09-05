#[macro_use]
mod logging;

// The Windows system heap degrades badly under multi-threaded small-object
// churn — exactly the hot paths here (rayon asset scans, tantivy indexing,
// tree-sitter parsing, serde_json streaming). mimalloc replaces it
// process-wide; bundled SQLite and the dynamically loaded onnxruntime keep
// their own internal allocators and are unaffected.
#[global_allocator]
static GLOBAL_ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, WindowEvent};

mod agent;
pub mod asset_db;
mod async_tasks;
mod auth;
pub mod binary_cache;
mod cdp_debug;
mod cli_driver;
pub mod code_tools;
mod commands;
mod compact;
mod config;
pub mod config_registry;
pub mod csharp_compile;
pub mod csharp_lsp;
pub(crate) mod diff;
pub mod dotnet_runtime;
pub(crate) mod eol;
pub mod error;
pub mod extra_workdirs;
mod feishu_docs;
pub mod file_log;
pub mod keychain;
pub mod knowledge_index;
pub mod knowledge_source_registry;
pub mod knowledge_store;
mod knowledge_watcher;
mod llm;
mod local_docs;
pub mod mcp;
pub(crate) mod merge;
pub mod model_catalog;
pub mod network;
pub mod plugin;
pub mod process_util;
pub mod prompt;
pub mod python_runtime;
pub mod resource_policy;
mod runtime_data_lock;
mod runtime_paths;
mod sdk;
mod session;
mod shared_workbench_window;
mod skill_runtime_context;
mod sqlite_maint;
mod tool;
pub mod unity_bridge;
pub mod unity_csharp;
mod unity_docs;
pub mod unity_editor_lock;
pub mod unity_hotreload;
mod unity_project_config;
pub mod unity_serialized_property;
pub mod unity_serialized_schema;
pub mod unity_type_index;
pub mod unity_type_index_selftest;
pub mod unity_yaml;
pub mod vcs;
pub mod view;
#[cfg(target_os = "windows")]
mod windows_resize_sync;
#[cfg(target_os = "windows")]
mod windows_window_frame;
mod workspace;
pub mod workspace_changes;
pub mod workspace_definition_registry;
pub mod workspace_service;
pub mod workspace_tool_registry;
mod workspace_tree;

use agent::definition::AgentDefRegistry;
use agent::instance::{AssistantStreamState, RawContextStore};
use commands::AppKnowledgeDir;

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_CLOSE_REQUESTED_EVENT: &str = "locus-main-window-close-requested";
const MAIN_TRAY_ID: &str = "locus-main-tray";
const TRAY_MENU_SHOW_ID: &str = "locus-tray-show";
const TRAY_MENU_EXIT_ID: &str = "locus-tray-exit";

fn legacy_active_session_candidates(data_dir: &std::path::Path) -> Vec<String> {
    let path = data_dir.join("active_session_selection.json");
    let Some(by_workspace) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|value| {
            value
                .get("byWorkspace")
                .or_else(|| value.get("by_workspace"))
                .and_then(serde_json::Value::as_object)
                .cloned()
        })
    else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if let Some(session_id) = by_workspace
        .get("__global__")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        candidates.push(session_id.to_string());
    }
    for session_id in by_workspace
        .values()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        if !candidates.iter().any(|candidate| candidate == session_id) {
            candidates.push(session_id.to_string());
        }
    }
    candidates
}

#[derive(Clone)]
struct StartupTrace {
    started_at: Instant,
    last_mark: Arc<Mutex<Instant>>,
}

impl StartupTrace {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_mark: Arc::new(Mutex::new(now)),
        }
    }

    fn elapsed_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }

    fn mark(&self, phase: &str) {
        let now = Instant::now();
        let mut delta_ms = 0;
        if let Ok(mut last_mark) = self.last_mark.lock() {
            delta_ms = now.duration_since(*last_mark).as_millis();
            *last_mark = now;
        }
        eprintln!(
            "[startup] phase={} total={}ms delta={}ms",
            phase,
            now.duration_since(self.started_at).as_millis(),
            delta_ms
        );
    }
}

fn emit_main_window_close_request(window: &tauri::Window) {
    if let Err(error) = window.emit(MAIN_WINDOW_CLOSE_REQUESTED_EVENT, ()) {
        eprintln!(
            "[Locus] failed to emit main window close request event: {}",
            error
        );
    }
}

fn set_main_tray_visible(app_handle: &tauri::AppHandle, visible: bool) -> bool {
    let Some(tray) = app_handle.tray_by_id(MAIN_TRAY_ID) else {
        return false;
    };
    if let Err(error) = tray.set_visible(visible) {
        eprintln!("[Locus] failed to update tray icon visibility: {}", error);
        return false;
    }
    true
}

pub(crate) fn reveal_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(error) = window.unminimize() {
            eprintln!("[Locus] failed to restore main window: {}", error);
        }
        if let Err(error) = window.show() {
            eprintln!("[Locus] failed to show main window: {}", error);
        }
        #[cfg(target_os = "windows")]
        if let Ok(hwnd) = window.hwnd() {
            unity_bridge::restore_foreground(hwnd.0 as isize);
        }
        if let Err(error) = window.set_focus() {
            eprintln!("[Locus] failed to focus main window: {}", error);
        }
    }
    let _ = set_main_tray_visible(app_handle, false);
}

fn hide_main_window_to_tray(window: &tauri::Window) {
    let app_handle = window.app_handle();
    if !set_main_tray_visible(app_handle, true) {
        emit_main_window_close_request(window);
        return;
    }

    if let Err(error) = window.hide() {
        eprintln!("[Locus] failed to hide main window to tray: {}", error);
        let _ = set_main_tray_visible(app_handle, false);
    }
}

fn tray_menu_labels() -> (&'static str, &'static str) {
    let is_zh = sys_locale::get_locale()
        .map(|locale| locale.to_ascii_lowercase().starts_with("zh"))
        .unwrap_or(false);
    if is_zh {
        ("显示 Locus", "退出")
    } else {
        ("Show Locus", "Exit")
    }
}

fn install_main_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let (show_label, exit_label) = tray_menu_labels();
    let menu = MenuBuilder::new(app)
        .text(TRAY_MENU_SHOW_ID, show_label)
        .separator()
        .text(TRAY_MENU_EXIT_ID, exit_label)
        .build()?;

    let Some(icon) = app.default_window_icon().cloned() else {
        eprintln!("[Locus] warning: default tray icon is unavailable");
        return Ok(());
    };

    let tray = TrayIconBuilder::with_id(MAIN_TRAY_ID)
        .icon(icon)
        .tooltip("Locus")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app_handle, event| match event.id().as_ref() {
            TRAY_MENU_SHOW_ID => reveal_main_window(app_handle),
            TRAY_MENU_EXIT_ID => commands::exit_app(app_handle),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => reveal_main_window(tray.app_handle()),
            _ => {}
        })
        .build(app)?;
    tray.set_visible(false)?;
    Ok(())
}

#[derive(Clone)]
pub struct AppAgentDir(pub Arc<Option<std::path::PathBuf>>);

#[derive(Clone)]
pub struct AgentDefRegistryState(pub Arc<tokio::sync::RwLock<AgentDefRegistry>>);

impl AgentDefRegistryState {
    pub async fn snapshot(&self) -> Arc<AgentDefRegistry> {
        Arc::new(self.0.read().await.clone())
    }
}

pub use asset_db::AssetDbState;
use auth::codex::CodexAuthState;
use auth::AuthState;
use commands::CodexAuthStateHandle;
use config::{AppCloseBehavior, AppConfig};

use session::store::SessionStore;
use tool::ToolRegistry;

pub struct ActiveTaskHandle {
    pub run_id: String,
    pub cancel_tx: tokio::sync::watch::Sender<bool>,
    pub done_rx: tokio::sync::watch::Receiver<bool>,
    pub partial_assistant: Arc<AssistantStreamState>,
    pub join_handle: tauri::async_runtime::JoinHandle<()>,
}

pub type ActiveTasks = Arc<tokio::sync::Mutex<HashMap<String, ActiveTaskHandle>>>;

pub type PendingInputQueueHandle =
    Arc<std::sync::Mutex<session::pending_inputs::PendingInputQueue>>;

pub type ApiKeyState = Arc<tokio::sync::RwLock<String>>;

pub type ProviderKeysState = Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>;

pub struct PendingQuestionResponse {
    pub session_id: String,
    pub run_id: String,
    pub tx: tokio::sync::oneshot::Sender<String>,
}

pub type QuestionStore = Arc<tokio::sync::Mutex<HashMap<String, PendingQuestionResponse>>>;

#[derive(Debug, Clone)]
pub struct PendingKnowledgeProposalDraft {
    pub run_id: String,
    pub proposal: session::models::KnowledgeProposal,
}

pub type KnowledgeProposalDraftStore =
    Arc<tokio::sync::Mutex<HashMap<String, PendingKnowledgeProposalDraft>>>;

pub type UndoManagerHandle = Arc<vcs::UndoManager>;

#[derive(Clone)]
pub struct ToolPermissionMode(pub Arc<tokio::sync::RwLock<String>>);

#[derive(Clone)]
pub struct ToolPermissions(pub Arc<tokio::sync::RwLock<HashMap<String, String>>>);

#[cfg(test)]
mod state_type_tests {
    use super::{ApiKeyState, ProviderKeysState, ToolPermissionMode, ToolPermissions};
    use std::any::TypeId;

    #[test]
    fn permission_state_types_do_not_alias_key_state_types() {
        assert_ne!(
            TypeId::of::<ToolPermissionMode>(),
            TypeId::of::<ApiKeyState>()
        );
        assert_ne!(
            TypeId::of::<ToolPermissions>(),
            TypeId::of::<ProviderKeysState>()
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let startup_trace = StartupTrace::new();
    std::eprintln!("[startup] phase=run_enter total=0ms delta=0ms");
    let runtime_launch_options =
        match runtime_paths::RuntimeLaunchOptions::configure_from_env_args() {
            Ok(options) => options,
            Err(error) => {
                eprintln!("[Locus CLI] {error}");
                std::process::exit(2);
            }
        };
    let external_script_open_request = unity_bridge::external_script_open_request_from_env_args();
    let cli_driver_config = match cli_driver::CliDriverConfig::from_env_args() {
        Some(Ok(config)) => Some(config),
        Some(Err(error)) => {
            eprintln!("[Locus CLI] {error}");
            std::process::exit(2);
        }
        None => None,
    };

    let shared_debug_flag = Arc::new(AtomicBool::new(
        std::env::var("LOCUS_DEBUG")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "True"))
            .unwrap_or(false),
    ));
    let log_store = Arc::new(logging::AppLogStore::new(logging::DEFAULT_LOG_CAPACITY));
    match file_log::FileLogSink::init_default() {
        Ok(sink) => {
            log_store.attach_file_sink(sink.clone());
            file_log::install_panic_hook(sink);
        }
        Err(error) => {
            std::eprintln!("[FileLog] persistent file logging disabled: {error}");
        }
    }
    logging::init_tracing(shared_debug_flag.clone(), log_store.clone());
    startup_trace.mark("tracing_ready");
    let binary_cache: Arc<binary_cache::BinaryCache> = Arc::new(binary_cache::BinaryCache::new());
    let cache_for_protocol = binary_cache.clone();
    let debug_flag_for_setup = shared_debug_flag.clone();
    let log_store_for_setup = log_store.clone();
    let startup_for_page_load = startup_trace.clone();
    let startup_for_setup = startup_trace.clone();
    let cli_driver_for_setup = cli_driver_config.clone();
    let external_script_open_for_setup = external_script_open_request.clone();
    let runtime_workspace_for_setup = runtime_launch_options.workspace_dir.clone();
    let skip_onboarding_for_setup = runtime_launch_options.skip_onboarding;

    tauri::Builder::default()
        .on_page_load(move |webview, payload| {
            let page_finished = matches!(payload.event(), PageLoadEvent::Finished);
            let event = match payload.event() {
                PageLoadEvent::Started => "started",
                PageLoadEvent::Finished => "finished",
            };
            eprintln!(
                "[startup] phase=webview_page_load label={} event={} url={} total={}ms",
                webview.label(),
                event,
                payload.url(),
                startup_for_page_load.elapsed_ms()
            );
            #[cfg(target_os = "windows")]
            if page_finished {
                if let Err(error) = windows_resize_sync::sync_after_page_load(webview) {
                    eprintln!("[Locus] warning: failed to sync WebView2 after page load: {error}");
                }
            }
        })
        .register_uri_scheme_protocol("locus-binary", move |_ctx, request| {
            let request_start = Instant::now();
            let path = request.uri().path(); // "/blob/{uuid}"
            let blob_id = path.strip_prefix("/blob/").unwrap_or("");
            match cache_for_protocol.get(blob_id) {
                Some((bytes, mime)) => {
                    let byte_len = bytes.len();
                    let response = tauri::http::Response::builder()
                        .header("Content-Type", &mime)
                        .header("Access-Control-Allow-Origin", "*")
                        .body(bytes)
                        .unwrap();
                    if mime == "application/octet-stream" {
                        eprintln!(
                            "[perf:locus-binary] blob={} status=200 bytes={} total={}ms",
                            blob_id,
                            byte_len,
                            request_start.elapsed().as_millis()
                        );
                    }
                    response
                }
                None => {
                    eprintln!(
                        "[perf:locus-binary] blob={} status=404 total={}ms",
                        blob_id,
                        request_start.elapsed().as_millis()
                    );
                    tauri::http::Response::builder()
                        .status(404)
                        .body(Vec::new())
                        .unwrap()
                }
            }
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_webview_event(|webview, event| {
            commands::handle_unity_embed_webview_event(webview, event);
        })
        .on_window_event(|window, event| {
            commands::handle_locus_window_event(window, event);
            commands::handle_sub_window_event(window, event);
            // Process exit destroys every native window. Preserve the last
            // durable pane projection so the next process can restore all
            // windows/workspaces; interactive closes still detach normally.
            if matches!(event, WindowEvent::Destroyed) && !commands::app_exit_started() {
                if let Some(contexts) = window
                    .app_handle()
                    .try_state::<Arc<workspace_service::WindowContextRegistry>>()
                {
                    if let Some(persistence) = window
                        .app_handle()
                        .try_state::<Arc<commands::WindowContextPersistence>>()
                    {
                        if let Ok(_mutation) = persistence.mutation.lock() {
                            if let Ok(intent_epoch) =
                                contexts.next_window_intent_epoch(window.label())
                            {
                                let _ = contexts.remove_window(window.label(), intent_epoch);
                            }
                            let _ = commands::persist_window_context_recovery(
                                window.app_handle(),
                                &contexts,
                            );
                        }
                    } else {
                        if let Ok(intent_epoch) = contexts.next_window_intent_epoch(window.label())
                        {
                            let _ = contexts.remove_window(window.label(), intent_epoch);
                        }
                    }
                }
            }
            if window.label() != MAIN_WINDOW_LABEL {
                return;
            }

            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let config = window.app_handle().state::<Arc<AppConfig>>();
                if config.close_behavior() == AppCloseBehavior::MinimizeToTray {
                    hide_main_window_to_tray(window);
                    return;
                }
                emit_main_window_close_request(window);
            }
        })
        .setup(move |app| {
            startup_for_setup.mark("setup_start");
            log_store_for_setup.attach_app_handle(app.handle().clone());
            if let Err(error) =
                commands::ensure_windows_notification_identity(&app.handle().clone())
            {
                eprintln!(
                    "[Locus] warning: failed to prepare Windows notification identity: {}",
                    error
                );
            }
            let data_dir = commands::prepare_runtime_storage_dir(&app.handle().clone())
                .map_err(|e| format!("Failed to prepare app storage dir: {}", e))?;
            let runtime_data_lock = runtime_data_lock::RuntimeDataDirLock::acquire(&data_dir)
                .map_err(|e| format!("Failed to acquire app storage lock: {}", e))?;
            println!(
                "[Locus] runtime data lock acquired: {}",
                runtime_data_lock.path().display()
            );
            app.manage(runtime_data_lock);
            if let Ok(resource_dir) = app.path().resource_dir() {
                process_util::set_managed_git_resource_dir(resource_dir.clone());
                process_util::set_managed_github_cli_resource_dir(resource_dir);
            }
            commands::restore_saved_git_override(&app.handle().clone());

            println!("[Locus] data_dir: {:?}", data_dir);
            startup_for_setup.mark("setup_storage_ready");

            let mut loaded_config = AppConfig::load(&data_dir);
            // CLI integration drivers exercise the bridge and process state
            // directly. Unity embed windows retain native HWNDs across editor
            // restarts and can panic the desktop event loop while a driver is
            // intentionally killing/relaunching Unity, so keep embed disabled
            // for this isolated test runtime.
            if cli_driver_for_setup.is_some() {
                loaded_config
                    .unity_embed_enabled
                    .store(false, Ordering::Relaxed);
            }
            debug_flag_for_setup.store(loaded_config.debug_enabled(), Ordering::Relaxed);
            loaded_config.debug = debug_flag_for_setup.clone();
            let config = Arc::new(loaded_config);
            let resource_policy = Arc::new(
                resource_policy::ResourcePolicyStore::from_config(config.clone())
                    .map_err(|error| format!("Invalid workspace resource policy: {error}"))?,
            );
            let workspace_service_factories: Vec<
                Arc<dyn workspace_service::service::WorkspaceServiceFactory>,
            > = vec![Arc::new(
                workspace_service::unity::UnityServiceFactory::new(
                    app.handle().clone(),
                    config.clone(),
                ),
            )];
            let workspace_registry = workspace_service::ProjectRegistry::new(
                resource_policy.clone(),
                workspace_service_factories,
            );
            let window_contexts = Arc::new(workspace_service::WindowContextRegistry::new());
            let window_context_persistence =
                Arc::new(commands::WindowContextPersistence::default());
            unity_bridge::initialize_background_hook(config.unity_background_hook_enabled());
            unity_bridge::initialize_state_probe(config.unity_state_probe_enabled());
            unity_bridge::initialize_native_bridge(config.unity_native_bridge_enabled());
            unity_editor_lock::initialize(config.unity_multi_agent_editor_enabled());
            unity_bridge::initialize_external_editor_default(
                config.unity_external_editor_default_enabled(),
            );
            csharp_lsp::initialize(
                config.csharp_lsp_enabled(),
                app.handle().clone(),
                resource_policy.clone(),
            );
            csharp_compile::initialize(
                config.unity_sidecar_compiler_enabled(),
                config.unity_non_public_access_enabled(),
                app.handle().clone(),
                resource_policy.clone(),
            );
            csharp_compile::set_in_process_fallback(
                config.unity_in_process_compile_fallback_enabled(),
            );
            unity_hotreload::initialize(
                config.unity_hot_reload_enabled(),
                config.unity_inline_force_evaluate_enabled(),
            );
            code_tools::initialize(config.code_analysis_tools());
            llm::retry::initialize(config.llm_retry_max_attempts());
            llm::think_tag_filter::initialize(config.llm_strip_inline_think_tags());
            startup_for_setup.mark("setup_config_ready");

            // Load OpenRouter API key from OS keychain only.
            let initial_key = keychain::get_secret(keychain::KEY_OPENROUTER)
                .ok()
                .flatten()
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            let api_key_state: ApiKeyState =
                Arc::new(tokio::sync::RwLock::new(initial_key.clone()));
            println!("[Locus] api_key present: {}", !initial_key.is_empty());

            let auth_state = Arc::new(tokio::sync::Mutex::new(AuthState::new(&data_dir)));
            println!("[Locus] auth state initialized");

            let codex_state: CodexAuthStateHandle =
                Arc::new(tokio::sync::Mutex::new(CodexAuthState::new(&data_dir)));
            println!("[Locus] codex auth state initialized");
            startup_for_setup.mark("setup_auth_state_ready");

            let app_temp_dir = commands::set_app_temp_dir_override(data_dir.join("temp"))
                .map_err(|e| format!("Failed to prepare app temp dir: {}", e))?;
            let tool_results_root = app_temp_dir.join("tool-results");
            let store = Arc::new(
                SessionStore::new_with_tool_results_root(&data_dir, tool_results_root)
                    .map_err(|e| format!("Failed to initialize SessionStore: {}", e))?,
            );
            workspace_registry
                .attach_session_store(&store)
                .map_err(|error| format!("Failed to attach project session catalog: {error}"))?;
            startup_for_setup.mark("setup_session_store_ready");
            // Deleted sessions leave locus.db at its high-water mark (SQLite
            // never returns freelist pages to the OS on its own); reclaim in
            // the background when most of the file is dead space.
            store.clone().spawn_vacuum_if_fragmented();

            let watcher_tuning = Arc::new(crate::asset_db::watcher::WatcherTuning::new());
            // Runtime registration is the single initialization boundary for
            // every open path, including startup recovery and session-driven
            // activation of persisted checkouts.
            app.manage(workspace_registry.clone());
            {
                let registration_store = Arc::clone(&store);
                let registration_app_handle = app.handle().clone();
                let registration_watcher_tuning = Arc::clone(&watcher_tuning);
                workspace_registry
                    .add_runtime_registration_hook(Arc::new(move |runtime| {
                        let opened_at = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs() as i64;
                        registration_store.upsert_workspace_checkout(
                            &session::models::WorkspaceCheckoutRecord {
                                checkout_id: runtime.checkout_id().to_string(),
                                project_id: runtime.project_id().to_string(),
                                root_path: runtime.root().display().to_string(),
                                normalized_root: runtime.normalized_root().to_string(),
                                last_opened_at: opened_at,
                            },
                        )?;
                        workspace_service::restore_or_persist_service_settings(
                            registration_store.as_ref(),
                            runtime,
                        )?;
                        runtime.core().start_background_watchers(
                            runtime,
                            &registration_app_handle,
                            Arc::clone(&registration_watcher_tuning),
                        )
                    }))
                    .map_err(|error| {
                        format!("Failed to register workspace initialization hook: {error}")
                    })?;
            }
            startup_for_setup.mark("setup_workspace_runtime_data_plane_ready");

            let recovered_window_contexts = commands::load_window_context_recovery(&data_dir);
            let legacy_active_session_candidates = legacy_active_session_candidates(&data_dir);
            let recovered_main_context = recovered_window_contexts
                .iter()
                .find(|context| context.window_id == MAIN_WINDOW_LABEL && context.pane_id == "main")
                .cloned();
            let recovered_main_root = recovered_main_context
                .as_ref()
                .and_then(|context| {
                    store
                        .get_workspace_checkout(context.focused_checkout_id.as_str())
                        .ok()
                        .flatten()
                })
                .map(|checkout| checkout.root_path)
                .filter(|root| std::path::Path::new(root).is_dir());

            // `working_dir.txt` is a one-time upgrade input. It is converted
            // into the durable main/main recovery context below and never
            // participates in backend request routing.
            let working_dir_file = data_dir.join("working_dir.txt");
            let driver_working_dir = cli_driver_for_setup
                .as_ref()
                .and_then(|driver| driver.project_path.as_ref())
                .map(|path| path.trim().to_string())
                .or_else(|| {
                    external_script_open_for_setup
                        .as_ref()
                        .map(|request| request.project_path.trim().to_string())
                })
                .filter(|path| {
                    let root = std::path::Path::new(path);
                    root.is_dir() && root.join("Assets").is_dir()
                })
                .map(|path| {
                    dunce::canonicalize(&path)
                        .map(|value| value.display().to_string())
                        .unwrap_or(path)
                });
            let explicit_main_root = runtime_workspace_for_setup
                .as_ref()
                .map(|path| path.display().to_string())
                .or(driver_working_dir);
            let legacy_migration_root = (explicit_main_root.is_none()
                && recovered_main_root.is_none())
            .then(|| {
                std::fs::read_to_string(&working_dir_file)
                    .ok()
                    .and_then(|s| {
                        let trimmed = s.trim().to_string();
                        if std::path::Path::new(&trimmed).is_dir() {
                            Some(trimmed)
                        } else {
                            None
                        }
                    })
            })
            .flatten();
            let requested_main_root = explicit_main_root.or(legacy_migration_root);
            let restored_window_contexts = commands::restore_persisted_window_contexts(
                recovered_window_contexts,
                requested_main_root.as_deref(),
                &legacy_active_session_candidates,
                workspace_registry.as_ref(),
                window_contexts.as_ref(),
                store.as_ref(),
            )
            .map_err(|error| format!("Failed to restore workspace contexts: {error}"))?;
            for warning in &restored_window_contexts.warnings {
                eprintln!("[Locus] warning: {warning}");
            }
            println!(
                "[Locus] restored {} workspace pane context(s)",
                restored_window_contexts.restored_panes
            );
            let initial_workspace_runtime = restored_window_contexts.main_runtime;
            let main_workspace_root = initial_workspace_runtime
                .as_ref()
                .map(|runtime| runtime.root().display().to_string())
                .unwrap_or_default();
            println!("[Locus] main workspace: {}", main_workspace_root);

            if let Some(runtime) = initial_workspace_runtime.as_ref() {
                commands::save_recent_dir_pub(&data_dir, &runtime.root().to_string_lossy());
            }

            if let Some(runtime) = initial_workspace_runtime
                .as_ref()
                .filter(|runtime| unity_bridge::is_unity_project(&runtime.root().to_string_lossy()))
            {
                let runtime_root = runtime.root().to_string_lossy();
                if let Err(error) = unity_bridge::sync_native_bridge_marker(
                    &runtime_root,
                    config.unity_native_bridge_enabled(),
                ) {
                    eprintln!(
                        "[Locus] warning: failed to sync native bridge marker on startup: {}",
                        error
                    );
                }
                if let Err(error) = unity_bridge::sync_background_hook_marker(
                    &runtime_root,
                    config.unity_background_hook_enabled(),
                ) {
                    eprintln!(
                        "[Locus] warning: failed to sync background hook marker on startup: {}",
                        error
                    );
                }
                if let Err(error) = unity_bridge::sync_unity_embed_enabled_marker(
                    &runtime_root,
                    config.unity_embed_enabled(),
                ) {
                    eprintln!(
                        "[Locus] warning: failed to sync Unity embed marker on startup: {}",
                        error
                    );
                }
            }
            startup_for_setup.mark("setup_workspace_ready");

            commands::persist_window_context_recovery(app.handle(), &window_contexts).map_err(
                |error| format!("Failed to persist restored workspace contexts: {error}"),
            )?;
            if working_dir_file.exists() {
                let _ = std::fs::remove_file(&working_dir_file);
            }
            let legacy_active_session_file = data_dir.join("active_session_selection.json");
            if legacy_active_session_file.exists() {
                let _ = std::fs::remove_file(legacy_active_session_file);
            }
            let pending_external_script_open = unity_bridge::PendingExternalScriptOpenRequest::new(
                external_script_open_for_setup.clone(),
            );

            let mut app_agent_dir_candidates = Vec::new();
            #[cfg(debug_assertions)]
            app_agent_dir_candidates.extend([
                std::path::PathBuf::from("../agent"), // dev: src-tauri/../agent
                std::path::PathBuf::from("agent"),    // dev: cwd/agent
            ]);
            app_agent_dir_candidates.push(data_dir.join("agent"));
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    app_agent_dir_candidates.push(exe_dir.join("agent"));
                }
            }
            let app_agent_dir = AppAgentDir(Arc::new(
                app_agent_dir_candidates
                    .iter()
                    .find(|p| p.is_dir())
                    .map(|p| {
                        let canonical = dunce::canonicalize(p).unwrap_or(p.clone());
                        println!("[Locus] app agent dir: {:?}", canonical);
                        canonical
                    }),
            ));
            if app_agent_dir.0.is_none() {
                println!("[Locus] no app agent dir found");
            }
            let workspace_definitions = Arc::new(
                workspace_definition_registry::WorkspaceDefinitionRegistry::new(
                    app_agent_dir.0.as_ref().clone(),
                ),
            );
            {
                let definitions = Arc::downgrade(&workspace_definitions);
                workspace_registry
                    .add_runtime_retirement_hook(Arc::new(move |runtime| {
                        if let Some(definitions) = definitions.upgrade() {
                            let _ = definitions
                                .remove_generation(runtime.checkout_id(), runtime.generation());
                        }
                        crate::csharp_compile::retire_workspace_generation(
                            runtime.checkout_id(),
                            runtime.generation(),
                        );
                        crate::view::retire_workspace_runtime(runtime);
                    }))
                    .map_err(|error| {
                        format!("Failed to register workspace retirement cleanup: {error}")
                    })?;
            }
            workspace_registry.spawn_idle_reaper();

            // The process-level registry is the immutable app base. Checkout
            // Agent and plugin overlays are resolved through
            // WorkspaceDefinitionRegistry at request time.
            let initial_registry = AgentDefRegistry::load_with_plugins(
                app_agent_dir.0.as_deref(),
                None,
                &crate::plugin::installed_agent_sources(""),
            );
            let initial_subagents = initial_registry.list_subagent_descriptions();
            let registry =
                AgentDefRegistryState(Arc::new(tokio::sync::RwLock::new(initial_registry)));
            startup_for_setup.mark("setup_agents_ready");

            let app_knowledge_dir = AppKnowledgeDir(Arc::new(
                commands::resolve_app_knowledge_dir(&data_dir).map(|p| {
                    let canonical = dunce::canonicalize(&p).unwrap_or(p);
                    println!("[Locus] app knowledge dir: {:?}", canonical);
                    canonical
                }),
            ));
            if app_knowledge_dir.0.is_none() {
                println!("[Locus] no app knowledge dir found");
            }

            let mut tool_registry = ToolRegistry::with_builtins();
            let skill_tool_count = commands::register_skill_package_tools(&mut tool_registry);
            if skill_tool_count > 0 {
                println!(
                    "[Locus] registered {} Skill package tool(s)",
                    skill_tool_count
                );
            }
            let subagents = initial_subagents;
            if !subagents.is_empty() {
                tool_registry.register_subagent_tool(&subagents);
                println!(
                    "[Locus] subagent tool registered with {} subagent(s): {}",
                    subagents.len(),
                    subagents
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let tool_registry = Arc::new(tool_registry);
            let workspace_tool_registry = Arc::new(
                workspace_tool_registry::WorkspaceToolRegistry::new(tool_registry.clone()),
            );
            println!("[Locus] tool registry initialized with built-in tools");
            startup_for_setup.mark("setup_tool_registry_ready");

            let provider_keys: ProviderKeysState = Arc::new(tokio::sync::RwLock::new(
                commands::load_provider_keys_from_keychain(&data_dir),
            ));
            println!("[Locus] provider keys loaded from keychain");
            startup_for_setup.mark("setup_provider_keys_ready");

            let raw_context_store: RawContextStore =
                Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

            let active_tasks: ActiveTasks = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let pending_input_queue: PendingInputQueueHandle = Arc::new(std::sync::Mutex::new(
                session::pending_inputs::PendingInputQueue::default(),
            ));
            let async_task_manager = Arc::new(async_tasks::AsyncTaskManager::default());

            let question_store: QuestionStore = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let knowledge_proposal_drafts: KnowledgeProposalDraftStore =
                Arc::new(tokio::sync::Mutex::new(HashMap::new()));

            // Persistent undo stacks: survive restarts; entries for sessions
            // deleted while the app was closed are dropped at load.
            let undo_valid_sessions = match store.list_all_session_ids() {
                Ok(ids) => Some(ids.into_iter().collect::<std::collections::HashSet<_>>()),
                Err(e) => {
                    eprintln!(
                        "[Locus] failed to list session ids for undo reconcile: {}",
                        e
                    );
                    None
                }
            };
            let undo_manager: UndoManagerHandle = Arc::new(vcs::UndoManager::with_persistence(
                vcs::GitProvider,
                data_dir.join("undo_stacks.json"),
                undo_valid_sessions,
            ));
            let view_automation_store = Arc::new(view::ViewAutomationStore::default());

            let tool_mode_path = data_dir.join("tool_permission_mode.txt");
            let initial_tool_mode = std::fs::read_to_string(&tool_mode_path)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| s == "ask")
                .unwrap_or_else(|| "auto".to_string());
            println!("[Locus] tool_permission_mode: {}", initial_tool_mode);
            let tool_permission_mode: ToolPermissionMode =
                ToolPermissionMode(Arc::new(tokio::sync::RwLock::new(initial_tool_mode)));

            let tool_perm_path = data_dir.join("tool_permissions.json");
            let mut initial_tool_perms: HashMap<String, String> =
                std::fs::read_to_string(&tool_perm_path)
                    .ok()
                    .and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok())
                    .map(|raw| {
                        raw.into_iter()
                            .map(|(key, value)| {
                                let normalized = if value.trim().eq_ignore_ascii_case("ask") {
                                    "ask".to_string()
                                } else {
                                    "auto".to_string()
                                };
                                (key, normalized)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            if !initial_tool_perms.contains_key("subagent") {
                if let Some(mode) = initial_tool_perms.get("task").cloned() {
                    initial_tool_perms.insert("subagent".to_string(), mode);
                }
            }
            initial_tool_perms.remove("task");
            println!("[Locus] tool_permissions: {:?}", initial_tool_perms);
            let tool_permissions: ToolPermissions =
                ToolPermissions(Arc::new(tokio::sync::RwLock::new(initial_tool_perms)));
            startup_for_setup.mark("setup_permissions_ready");

            app.manage(config);
            app.manage(resource_policy);
            app.manage(window_contexts);
            app.manage(window_context_persistence);
            app.manage(pending_external_script_open);
            app.manage(auth_state);
            app.manage(codex_state);
            app.manage(api_key_state);
            app.manage(app_knowledge_dir);
            app.manage(app_agent_dir);
            app.manage(provider_keys);
            app.manage(store.clone());
            app.manage(registry);
            app.manage(workspace_definitions);
            app.manage(tool_registry);
            app.manage(workspace_tool_registry);
            app.manage(std::sync::Arc::new(
                cdp_debug::CdpDebugServerHandle::default(),
            ));
            app.manage(std::sync::Arc::new(mcp::server::McpServerHandle::default()));
            app.manage(std::sync::Arc::new(sdk::SdkServerHandle::default()));
            app.manage(raw_context_store);
            app.manage(active_tasks);
            app.manage(pending_input_queue);
            app.manage(async_task_manager);
            app.manage(crate::asset_db::watcher::WatcherTuningState(Arc::clone(
                &watcher_tuning,
            )));
            app.manage(question_store);
            app.manage(knowledge_proposal_drafts);
            app.manage(undo_manager);
            app.manage(view_automation_store);
            app.manage(tool_permission_mode);
            app.manage(tool_permissions);
            app.manage(binary_cache);
            app.manage(log_store_for_setup.clone());
            startup_for_setup.mark("setup_state_managed");
            startup_for_setup.mark("setup_backend_ready");

            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    match sdk::start(app_handle).await {
                        Ok(address) => eprintln!("[LocusSdk] listening on http://{address}/sdk"),
                        Err(error) => {
                            python_runtime::clear_locus_sdk_connection();
                            eprintln!("[LocusSdk] failed to start: {error}");
                        }
                    }
                });
            }

            if let Some(cli_driver_config) = cli_driver_for_setup.clone() {
                if !cli_driver_config.requires_frontend() {
                    cli_driver::spawn(app.handle().clone(), cli_driver_config);
                    startup_for_setup.mark("setup_cli_driver_scheduled");
                    startup_for_setup.mark("setup_done");
                    return Ok(());
                }
            }

            let main_window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == MAIN_WINDOW_LABEL)
                .ok_or_else(|| format!("Missing '{}' window config", MAIN_WINDOW_LABEL))?;
            startup_for_setup.mark("main_window_build_start");
            let mut main_window_builder =
                tauri::WebviewWindowBuilder::from_config(app.handle(), main_window_config)?;
            let app_handle_for_shared_workbench = app.handle().clone();
            main_window_builder = main_window_builder.on_new_window(move |url, features| {
                shared_workbench_window::handle_new_window(
                    &app_handle_for_shared_workbench,
                    url,
                    features,
                )
            });
            if skip_onboarding_for_setup
                || cli_driver_for_setup
                    .as_ref()
                    .is_some_and(cli_driver::CliDriverConfig::requires_frontend)
            {
                main_window_builder = main_window_builder.initialization_script(
                    "try { localStorage.setItem('locus-onboarding-completed', '1'); } catch (_) {}",
                );
            }
            let debug_initialization_script = if app.state::<Arc<AppConfig>>().debug_enabled() {
                "window.__LOCUS_DEBUG_ENABLED__ = true; try { localStorage.setItem('locus:webview-bridge:debug-enabled:v1', '1'); } catch (_) {}"
            } else {
                "window.__LOCUS_DEBUG_ENABLED__ = false; try { localStorage.removeItem('locus:webview-bridge:debug-enabled:v1'); } catch (_) {}"
            };
            main_window_builder =
                main_window_builder.initialization_script(debug_initialization_script);
            main_window_builder.build()?;
            startup_for_setup.mark("main_window_build_done");
            if let Some(cli_driver_config) = cli_driver_for_setup.clone() {
                cli_driver::spawn(app.handle().clone(), cli_driver_config);
                startup_for_setup.mark("setup_cli_driver_scheduled");
                startup_for_setup.mark("setup_done");
                return Ok(());
            }
            if app.state::<Arc<AppConfig>>().debug_enabled() {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = cdp_debug::reconcile(app_handle, true).await {
                        eprintln!("[CdpDebug] startup failed: {error}");
                    }
                });
            }
            if let Err(error) = install_main_tray(app) {
                eprintln!("[Locus] warning: failed to install tray icon: {}", error);
            }

            commands::start_unity_embed_control_server(app.handle().clone());
            #[cfg(target_os = "windows")]
            if let Err(error) = windows_window_frame::restore_main_window_frame(app) {
                eprintln!("[Locus] warning: failed to restore main window frame: {error}");
            }
            #[cfg(target_os = "windows")]
            if let Err(error) = windows_resize_sync::install_for_main_window(app) {
                eprintln!("[Locus] warning: failed to install WebView2 resize sync: {error}");
            }
            startup_for_setup.mark("setup_native_window_hooks_ready");

            let initial_runtime_for_unity = initial_workspace_runtime.clone();
            let workspace_registry_for_unity = workspace_registry.clone();
            let startup_for_unity = startup_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                startup_for_unity.mark("unity_monitor_task_start");
                if let Some(runtime) = initial_runtime_for_unity.filter(|runtime| {
                    unity_bridge::is_unity_project(&runtime.root().to_string_lossy())
                }) {
                    if let Err(error) = workspace_registry_for_unity
                        .execution_context(
                            runtime.checkout_id(),
                            &[workspace_service::service::ServiceKind::Unity],
                        )
                        .await
                    {
                        eprintln!("[Locus] failed to start initial Unity service: {error}");
                    }
                }
                startup_for_unity.mark("unity_monitor_task_done");
            });

            tauri::async_runtime::spawn(model_catalog::background_refresh());

            // Connect enabled MCP servers in the background; the agent tool
            // snapshot stays empty until this (or a later mcp_reload /
            // settings write) completes, so startup is never blocked on an
            // external server.
            mcp::manager::set_event_app_handle(app.handle().clone());
            tauri::async_runtime::spawn(async {
                let reports = mcp::manager::reconcile().await;
                for report in &reports {
                    match &report.error {
                        Some(error) => {
                            eprintln!("[Mcp] startup connect failed for '{}': {error}", report.id)
                        }
                        None => eprintln!(
                            "[Mcp] startup connected '{}' ({} tools)",
                            report.id,
                            report.tool_names.len()
                        ),
                    }
                }
            });

            // Locus-as-MCP-server: start the localhost endpoint when the
            // feature is enabled (no-op otherwise). Lives in the pre-window
            // service section so a future headless mode inherits it.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    mcp::server::reconcile(app_handle).await;
                });
            }

            let initial_runtime_for_knowledge = initial_workspace_runtime.clone();
            let app_handle_for_knowledge = app.handle().clone();
            let startup_for_knowledge = startup_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                startup_for_knowledge.mark("knowledge_startup_task_start");
                let Some(runtime) = initial_runtime_for_knowledge else {
                    startup_for_knowledge.mark("knowledge_startup_task_skipped");
                    return;
                };
                let wd = runtime.root().to_string_lossy().to_string();
                let app_knowledge_dir: tauri::State<'_, AppKnowledgeDir> =
                    app_handle_for_knowledge.state();
                let knowledge_startup_state =
                    match runtime.knowledge_index(&app_handle_for_knowledge) {
                        Ok(state) => state,
                        Err(error) => {
                            eprintln!("[Locus] knowledge index startup error: {error}");
                            return;
                        }
                    };
                if let Err(e) = knowledge_index::maybe_auto_activate_embedding_runtime(
                    knowledge_startup_state.clone(),
                    &wd,
                    app_knowledge_dir.0.as_ref().as_ref(),
                )
                .await
                {
                    eprintln!("[Locus] knowledge embedding auto-activate error: {}", e);
                }
                if let Err(e) = knowledge_index::reconcile_workspace(
                    &wd,
                    app_knowledge_dir.0.as_ref().as_ref(),
                    knowledge_startup_state.clone(),
                )
                .await
                {
                    eprintln!("[Locus] knowledge reconcile error: {}", e);
                }
                startup_for_knowledge.mark("knowledge_startup_task_done");
            });
            startup_for_setup.mark("setup_background_tasks_scheduled");
            startup_for_setup.mark("setup_done");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_session,
            commands::fork_session,
            commands::fork_session_from_message,
            commands::chat,
            commands::queue_chat_input,
            commands::queue_session_compact,
            commands::insert_pending_chat_input,
            commands::delete_pending_chat_input,
            commands::list_agents,
            commands::list_workspace_agents,
            commands::list_subagent_defs,
            commands::list_workspace_subagent_defs,
            commands::get_agent_system_prompt,
            commands::get_workspace_agent_system_prompt,
            commands::get_agent_env_template,
            commands::get_workspace_agent_env_template,
            commands::get_agent_rendered_env_prompt,
            commands::get_workspace_agent_rendered_env_prompt,
            commands::get_agent_system_prompt_stats,
            commands::get_workspace_agent_system_prompt_stats,
            commands::list_agent_injected_items,
            commands::list_workspace_agent_injected_items,
            commands::set_agent_injection_enabled,
            commands::set_agent_tool_direct_load,
            commands::set_agent_tool_enabled,
            commands::load_session,
            commands::load_session_view,
            commands::load_session_message_page,
            commands::load_session_message_images,
            commands::load_session_turn_preview,
            commands::save_session_execution_state,
            commands::get_compacted_context_output,
            commands::list_sessions,
            commands::list_checkout_sessions,
            commands::list_project_sessions,
            commands::list_archived_sessions,
            commands::list_archived_checkout_sessions,
            commands::rename_session,
            commands::archive_session,
            commands::unarchive_session,
            commands::delete_session,
            commands::get_session_usage,
            commands::get_session_context_usage_report,
            commands::get_model_usage_stats,
            commands::get_session_active_run,
            commands::get_session_resume_available,
            commands::list_session_events,
            commands::get_auth_status,
            commands::get_auth_url,
            commands::exchange_auth_code,
            commands::auth_logout,
            commands::import_claude_code_oauth,
            commands::anthropic_rate_limits,
            commands::save_api_key,
            commands::clear_api_key,
            commands::get_providers,
            commands::test_claude_code_cli,
            commands::save_provider_key,
            commands::delete_provider_key,
            commands::get_app_storage_info,
            commands::get_app_temp_info,
            commands::clear_app_temp_dir,
            commands::open_app_storage_dir,
            commands::open_app_temp_dir,
            commands::schedule_app_storage_migration,
            commands::clear_app_storage_migration,
            commands::get_workspace_service_resource_limits,
            commands::set_workspace_service_resource_limits,
            commands::get_workspace_service_resource_metrics,
            commands::list_workspace_runtimes,
            commands::list_project_contexts,
            commands::open_workspace,
            commands::remove_workspace,
            commands::focus_workspace,
            commands::set_active_session,
            commands::detach_workspace_pane,
            commands::detach_workspace_window,
            commands::list_window_workspace_contexts,
            commands::list_window_workspace_intent_epochs,
            commands::start_workspace_unity_service,
            commands::get_workspace_service_states,
            commands::list_recent_dirs,
            commands::remove_recent_dir,
            commands::extra_workdirs_get,
            commands::extra_workdirs_set,
            commands::extra_workdirs_map,
            commands::mcp_servers_get,
            commands::mcp_servers_upsert,
            commands::mcp_servers_remove,
            commands::mcp_server_test,
            commands::mcp_get_status,
            commands::mcp_server_set_enabled,
            commands::mcp_import_scan,
            commands::mcp_import_apply,
            commands::mcp_server_wire_tools,
            commands::mcp_server_tools_inventory,
            commands::mcp_server_get_state,
            commands::mcp_server_update_settings,
            commands::mcp_server_regenerate_token,
            commands::mcp_server_tool_inventory,
            commands::mcp_server_integrations,
            commands::mcp_server_integration_apply,
            commands::mcp_server_integration_remove,
            commands::open_dir_in_file_explorer,
            commands::list_dir_entries,
            commands::list_dir_entries_page,
            commands::search_workspace_entries,
            commands::stat_workspace_entries,
            commands::export_session_context,
            commands::get_todos,
            commands::cancel_chat,
            commands::stale_knowledge_proposals,
            commands::ignore_knowledge_proposal,
            commands::apply_knowledge_proposal,
            commands::check_unity_connection,
            commands::check_unity_connection_status,
            commands::get_unity_console_text,
            commands::check_unity_plugin,
            commands::check_unity_plugin_install_plan,
            commands::install_unity_plugin,
            commands::launch_unity_project,
            commands::close_headless_unity_project,
            commands::unity_recompile_run,
            commands::unity_recompile_probe_run,
            commands::unity_execute_snippet_run,
            commands::send_unity_log,
            commands::select_unity_asset,
            commands::open_unity_asset_inspector,
            commands::select_unity_scene_object,
            commands::validate_unity_scene_object,
            commands::open_unity_scene_object_inspector,
            commands::ref_graph_status,
            commands::ref_graph_scan,
            commands::ref_graph_scan_start,
            commands::asset_db_overview,
            commands::asset_db_light_status,
            commands::asset_risk_report,
            commands::get_watcher_tuning,
            commands::set_watcher_tuning,
            commands::search_workspace_assets,
            commands::search_workspace_scene_objects,
            commands::preview_workspace_asset,
            commands::preview_workspace_asset_thumbnail,
            commands::read_workspace_asset_preview_frame_cache,
            commands::cache_workspace_asset_preview_frame,
            commands::render_workspace_asset_preview_frame,
            commands::preview_workspace_asset_target,
            commands::unity_serialized_property_read,
            commands::unity_serialized_property_discover,
            commands::unity_serialized_property_write,
            commands::unity_serialized_property_apply,
            commands::ref_graph_deps,
            commands::ref_graph_refs,
            commands::ref_graph_resolve_guid,
            commands::ref_graph_resolve_path,
            commands::ref_graph_walk_deps,
            commands::ref_graph_walk_refs,
            commands::search_assets,
            commands::answer_question,
            commands::git_log,
            commands::git_history_snapshot,
            commands::project_collaboration_snapshot,
            commands::git_history_search,
            commands::git_commit_body,
            commands::git_probe,
            commands::git_runtime_state,
            commands::git_save_runtime_selection,
            commands::git_head_hash,
            commands::git_install_help,
            commands::git_install_via,
            commands::git_set_override,
            commands::git_clear_override,
            commands::git_status,
            commands::git_commit_files,
            commands::git_compare_files,
            commands::git_branches,
            commands::git_stashes,
            commands::git_submodules,
            commands::git_init_unity,
            commands::git_check_user_config,
            commands::git_config_snapshot,
            commands::git_save_config,
            commands::git_set_user_config,
            commands::git_stage,
            commands::git_stage_paths,
            commands::git_unstage,
            commands::git_unstage_paths,
            commands::git_stage_all,
            commands::git_unstage_all,
            commands::git_discard_file,
            commands::git_commit,
            commands::git_merge_file,
            commands::git_merge_apply,
            commands::git_merge_action,
            commands::git_merge_semantic_session,
            commands::git_merge_semantic_target,
            commands::git_merge_semantic_validate,
            commands::git_merge_semantic_apply,
            commands::git_generate_commit_message,
            commands::git_commit_action,
            commands::git_branch_action,
            commands::git_stash_action,
            commands::run_command,
            commands::get_skill_config,
            commands::set_skill_config,
            commands::get_all_skill_configs,
            commands::knowledge_get_general_config,
            commands::knowledge_save_general_config,
            commands::knowledge_get_embedding_config,
            commands::knowledge_save_embedding_config,
            commands::knowledge_activate_embedding,
            commands::knowledge_deactivate_embedding,
            commands::knowledge_get_embedding_status,
            commands::knowledge_test_embedding_runtime,
            commands::knowledge_get_local_embedding_model_catalog,
            commands::knowledge_download_local_embedding_model,
            commands::knowledge_cancel_local_embedding_model_download,
            commands::knowledge_inspect_local_embedding_model_directory,
            commands::knowledge_rebuild_lexical_index,
            commands::knowledge_get_lexical_rebuild_status,
            commands::knowledge_get_overview,
            commands::knowledge_get_unity_reference_import_status,
            commands::knowledge_find_unity_reference_directory,
            commands::knowledge_get_feishu_reference_import_status,
            commands::knowledge_save_feishu_reference_config,
            commands::knowledge_test_feishu_reference_connection,
            commands::knowledge_start_feishu_reference_oauth,
            commands::knowledge_cancel_feishu_reference_oauth_wait,
            commands::knowledge_list_feishu_reference_space_nodes,
            commands::knowledge_cancel_unity_reference_import,
            commands::knowledge_cancel_feishu_reference_import,
            commands::knowledge_list,
            commands::project_knowledge_list,
            commands::project_explorer_snapshot,
            commands::project_explorer_apply_operations,
            commands::project_explorer_list_presets,
            commands::project_explorer_create_preset,
            commands::project_explorer_switch_preset,
            commands::project_explorer_rename_preset,
            commands::project_explorer_delete_preset,
            commands::project_explorer_list_mount,
            commands::project_explorer_preview_file,
            commands::project_explorer_file_revision,
            commands::project_explorer_write_file,
            commands::workspace_file_preview,
            commands::workspace_file_revision,
            commands::workspace_file_write,
            commands::knowledge_list_scoped,
            commands::knowledge_list_page,
            commands::knowledge_list_page_scoped,
            commands::knowledge_list_directories,
            commands::knowledge_list_directories_scoped,
            commands::knowledge_list_directory_documents,
            commands::knowledge_list_directory_documents_page,
            commands::knowledge_list_external_reference_directories,
            commands::knowledge_list_unity_managed_directory_stats,
            commands::knowledge_query,
            commands::knowledge_read,
            commands::knowledge_read_scoped,
            commands::knowledge_import_unity_reference_docs,
            commands::knowledge_import_feishu_reference_docs,
            commands::knowledge_delete_unity_reference_docs,
            commands::knowledge_delete_feishu_reference_docs,
            commands::knowledge_preview_local_reference_import,
            commands::knowledge_import_local_reference_docs,
            commands::knowledge_get_local_reference_import_status,
            commands::knowledge_cancel_local_reference_import,
            commands::knowledge_sync_local_reference_docs,
            commands::knowledge_delete_local_reference_docs,
            commands::knowledge_delete_external_reference_directory,
            commands::knowledge_create,
            commands::knowledge_delete,
            commands::knowledge_move,
            commands::knowledge_edit,
            commands::list_skills,
            commands::read_skill_manifest,
            commands::get_default_skill_package_namespace,
            commands::set_default_skill_package_namespace,
            commands::create_skill_scaffold,
            commands::delete_skill_package,
            commands::import_skill_package,
            commands::export_skill_package,
            commands::get_skill_unity_install_status,
            commands::install_skill_unity_files,
            commands::remove_skill_unity_files,
            commands::refresh_external_skills,
            commands::plugin_registry_sources_get,
            commands::plugin_registry_sources_set,
            commands::plugin_registry_fetch_manifest,
            commands::plugin_registry_fetch_shard,
            commands::plugin_registry_fetch_search_index,
            commands::plugin_registry_fetch_plugin,
            commands::plugin_registry_fetch_description,
            commands::plugin_list_installed,
            commands::plugin_inspector_drawer_packages,
            commands::plugin_install_from_path,
            commands::plugin_install_from_registry,
            commands::plugin_install_from_source,
            commands::plugin_set_enabled,
            commands::plugin_github_auth_status,
            commands::plugin_github_repo_star_status,
            commands::plugin_github_repo_set_starred,
            commands::plugin_github_auth_save_token,
            commands::plugin_github_oauth_start,
            commands::plugin_github_oauth_poll,
            commands::plugin_github_auth_logout,
            commands::plugin_uninstall,
            commands::plugin_export,
            commands::open_file_external,
            commands::reveal_workspace_file,
            commands::knowledge_reveal_target,
            commands::resolve_markdown_image,
            commands::preview_workspace_file,
            commands::list_app_rules,
            commands::read_app_rule,
            commands::list_rules,
            commands::save_rule,
            commands::read_rule,
            commands::delete_rule,
            commands::set_rule_enabled,
            commands::set_rule_order,
            commands::get_last_model,
            commands::save_last_model,
            commands::get_last_effort,
            commands::save_last_effort,
            commands::get_agent_model_preferences,
            commands::save_agent_model_preference,
            commands::get_codex_fast_mode,
            commands::save_codex_fast_mode,
            commands::get_model_defaults,
            commands::save_model_defaults,
            commands::get_codex_model_config,
            commands::get_codex_available_models,
            commands::save_codex_model_config,
            commands::test_custom_endpoint,
            commands::get_custom_providers,
            commands::save_custom_providers,
            model_catalog::get_model_catalog,
            model_catalog::refresh_model_catalog,
            commands::codex_status,
            commands::codex_start_login,
            commands::codex_poll_login,
            commands::codex_logout,
            commands::import_codex_cli,
            commands::codex_retry_auth,
            commands::codex_rate_limits,
            commands::codex_consume_rate_limit_reset_credit,
            commands::diff_single_file,
            commands::diff_semantic_target,
            commands::diff_text_for_large,
            commands::diff_strings,
            commands::undo_latest_conversation_turn,
            commands::rollback_session_to_message,
            commands::undo_perform,
            commands::undo_perform_to_message,
            commands::undo_revert_file,
            commands::undo_preview,
            commands::undo_list,
            commands::undo_check_conflicts,
            commands::undo_check_dirty,
            commands::get_debug_mode,
            commands::debug_webview_bridge_heartbeat,
            commands::set_debug_mode,
            commands::get_tool_failure_log_enabled,
            commands::set_tool_failure_log_enabled,
            commands::get_session_undo_enabled,
            commands::set_session_undo_enabled,
            commands::get_llm_retry_max_attempts,
            commands::set_llm_retry_max_attempts,
            commands::get_subagent_max_depth,
            commands::set_subagent_max_depth,
            commands::get_subagent_max_concurrent,
            commands::set_subagent_max_concurrent,
            commands::get_file_tool_workspace_boundary,
            commands::set_file_tool_workspace_boundary,
            commands::get_unity_test_tools_workspace_status,
            commands::set_unity_test_tools_workspace_enabled,
            commands::get_tool_permission_mode,
            commands::save_tool_permission_mode,
            commands::get_tool_permissions,
            commands::save_tool_permissions,
            commands::reset_all_config,
            commands::get_session_plan_state,
            commands::set_session_plan_mode,
            commands::get_plan_file_content,
            commands::get_system_fonts,
            commands::get_system_locale,
            commands::get_close_behavior,
            commands::set_close_behavior,
            commands::get_dynamic_tool_loading_mode,
            commands::set_dynamic_tool_loading_mode,
            commands::get_anthropic_native_lazy_enabled,
            commands::set_anthropic_native_lazy_enabled,
            commands::get_async_tasks_enabled,
            commands::set_async_tasks_enabled,
            commands::get_unity_multi_agent_editor_enabled,
            commands::set_unity_multi_agent_editor_enabled,
            commands::get_unity_background_hook_enabled,
            commands::set_unity_background_hook_enabled,
            commands::get_unity_background_hook_status,
            commands::get_unity_external_editor_default_enabled,
            commands::set_unity_external_editor_default_enabled,
            commands::take_external_script_open_request,
            commands::get_unity_state_probe_enabled,
            commands::set_unity_state_probe_enabled,
            commands::get_unity_state_probe_status,
            commands::get_unity_native_bridge_enabled,
            commands::set_unity_native_bridge_enabled,
            commands::get_unity_native_broker_status,
            commands::get_unity_semantic_state,
            commands::unity_state_probe_selftest_run,
            commands::unity_native_bridge_selftest_run,
            commands::unity_integration_test_run,
            commands::unity_integration_test_cancel,
            commands::csharp_lsp_get_status,
            commands::csharp_lsp_set_enabled,
            commands::csharp_lsp_restart,
            commands::unity_sidecar_compiler_get_status,
            commands::unity_sidecar_compiler_set_enabled,
            commands::unity_non_public_access_set_enabled,
            commands::unity_in_process_compile_fallback_get_enabled,
            commands::unity_in_process_compile_fallback_set_enabled,
            commands::unity_hot_reload_set_enabled,
            commands::unity_inline_force_evaluate_set_enabled,
            commands::unity_hot_reload_selftest_run,
            commands::unity_hot_reload_access_probe_run,
            commands::unity_hot_reload_preflight,
            commands::unity_hot_reload_set_code_optimization_debug,
            commands::unity_hot_reload_set_code_optimization,
            commands::unity_hot_reload_set_play_mode_reload,
            commands::code_analysis_tools_get_config,
            commands::code_analysis_tools_set_config,
            commands::get_proxy_status,
            commands::save_proxy_config,
            commands::get_python_runtime_state,
            commands::save_python_runtime_selection,
            commands::send_system_notification,
            commands::play_custom_notification_sound,
            commands::get_running_task_count,
            commands::request_app_exit,
            commands::get_config_registry,
            commands::get_workspace_config_registry,
            commands::get_log_entries,
            commands::clear_log_entries,
            commands::save_log_export,
            commands::append_frontend_logs,
            commands::reveal_log_file,
            commands::unity_embed_status,
            commands::get_unity_embed_enabled,
            commands::set_unity_embed_enabled,
            commands::unity_embed_open_frontend_window,
            commands::unity_embed_set_mouse_activation_suppressed,
            commands::unity_embed_activate_for_input,
            commands::unity_embed_set_drag_passthrough,
            commands::unity_embed_focus_debug_snapshot,
            shared_workbench_window::start_shared_workbench_drag_tracking,
            shared_workbench_window::stop_shared_workbench_drag_tracking,
            commands::unity_embed_commit_asset_drop,
            commands::unity_embed_start_asset_drag,
            commands::unity_embed_cancel_asset_drag,
            commands::unity_embed_start_native_asset_file_drag,
            commands::locus_start_native_file_drag,
            commands::locus_start_drag_preview,
            commands::locus_stop_drag_preview,
            commands::view_templates,
            commands::view_list,
            commands::view_tree,
            commands::view_create,
            commands::view_create_folder,
            commands::view_delete_entry,
            commands::view_rename_entry,
            commands::view_move_entry,
            commands::view_export_package,
            commands::view_import_package,
            commands::view_read,
            commands::view_reload,
            commands::view_run,
            commands::view_run_in_unity,
            commands::view_set_tab_host,
            commands::sub_window_open,
            commands::sub_window_pool_prepare,
            commands::sub_window_pool_ready,
            commands::sub_window_claimed_query,
            commands::view_content_mount,
            commands::view_content_hide,
            commands::view_content_destroy,
            commands::view_compile_script,
            commands::view_call_script,
            commands::view_append_frontend_log,
            commands::view_read_frontend_log,
            commands::view_open_frontend_log,
            commands::view_storage_get,
            commands::view_storage_set,
            commands::view_storage_remove,
            commands::view_fs_read_file,
            commands::view_fs_write_file,
            commands::view_fs_append_file,
            commands::view_fs_mkdir,
            commands::view_fs_readdir,
            commands::view_fs_stat,
            commands::view_fs_lstat,
            commands::view_fs_access,
            commands::view_fs_unlink,
            commands::view_fs_rm,
            commands::view_fs_rename,
            commands::view_fs_copy_file,
            commands::view_automation_respond,
            commands::fetch_app_update_manifest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
