#[macro_use]
mod logging;

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
mod auth;
pub mod binary_cache;
mod commands;
mod compact;
mod config;
pub mod config_registry;
pub(crate) mod diff;
pub(crate) mod eol;
pub mod error;
mod feishu_docs;
pub mod keychain;
pub mod knowledge_index;
pub mod knowledge_store;
mod knowledge_watcher;
mod llm;
pub(crate) mod merge;
pub mod network;
pub mod plugin;
pub mod process_util;
pub mod prompt;
pub mod python_runtime;
mod session;
mod tool;
pub mod unity_bridge;
pub mod unity_csharp;
mod unity_docs;
pub mod unity_serialized_property;
pub mod unity_type_index;
pub mod unity_yaml;
pub mod vcs;
pub mod view;
#[cfg(target_os = "windows")]
mod windows_resize_sync;
#[cfg(target_os = "windows")]
mod windows_window_frame;
mod workspace;

use agent::definition::AgentDefRegistry;
use agent::instance::{AssistantStreamState, RawContextStore};
use commands::AppKnowledgeDir;

const MAIN_WINDOW_LABEL: &str = "main";
const MAIN_WINDOW_CLOSE_REQUESTED_EVENT: &str = "locus-main-window-close-requested";
const MAIN_TRAY_ID: &str = "locus-main-tray";
const TRAY_MENU_SHOW_ID: &str = "locus-tray-show";
const TRAY_MENU_EXIT_ID: &str = "locus-tray-exit";

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

fn reveal_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window(MAIN_WINDOW_LABEL) {
        if let Err(error) = window.show() {
            eprintln!("[Locus] failed to show main window from tray: {}", error);
        }
        if let Err(error) = window.set_focus() {
            eprintln!("[Locus] failed to focus main window from tray: {}", error);
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

use asset_db::watcher::AssetDbWatcher;
pub use asset_db::AssetDbState;
use auth::codex::CodexAuthState;
use auth::AuthState;
use commands::CodexAuthStateHandle;
use config::{AppCloseBehavior, AppConfig};

pub type AssetDbWatcherHandle = Arc<std::sync::Mutex<Option<AssetDbWatcher>>>;
pub type KnowledgeFsWatcherHandle =
    Arc<std::sync::Mutex<Option<knowledge_watcher::KnowledgeFsWatcher>>>;
use session::store::SessionStore;
use tool::ToolRegistry;
use unity_bridge::UnityMonitorHandle;
use workspace::Workspace;

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

    let shared_debug_flag = Arc::new(AtomicBool::new(
        std::env::var("LOCUS_DEBUG")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "True"))
            .unwrap_or(false),
    ));
    let log_store = Arc::new(logging::AppLogStore::new(logging::DEFAULT_LOG_CAPACITY));
    logging::init_tracing(shared_debug_flag.clone(), log_store.clone());
    startup_trace.mark("tracing_ready");
    let binary_cache: Arc<binary_cache::BinaryCache> = Arc::new(binary_cache::BinaryCache::new());
    let cache_for_protocol = binary_cache.clone();
    let debug_flag_for_setup = shared_debug_flag.clone();
    let log_store_for_setup = log_store.clone();
    let startup_for_page_load = startup_trace.clone();
    let startup_for_setup = startup_trace.clone();

    tauri::Builder::default()
        .on_page_load(move |webview, payload| {
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
            commands::handle_agent_graph_tool_window_event(window, event);
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
            if let Err(error) = commands::ensure_windows_notification_identity(&app.handle().clone())
            {
                eprintln!(
                    "[Locus] warning: failed to prepare Windows notification identity: {}",
                    error
                );
            }
            let data_dir = commands::prepare_runtime_storage_dir(&app.handle().clone())
                .map_err(|e| format!("Failed to prepare app storage dir: {}", e))?;
            if let Ok(resource_dir) = app.path().resource_dir() {
                process_util::set_managed_git_resource_dir(resource_dir);
            }
            commands::restore_saved_git_override(&app.handle().clone());

            println!("[Locus] data_dir: {:?}", data_dir);
            startup_for_setup.mark("setup_storage_ready");

            let mut loaded_config = AppConfig::load(&data_dir);
            debug_flag_for_setup.store(loaded_config.debug_enabled(), Ordering::Relaxed);
            loaded_config.debug = debug_flag_for_setup.clone();
            let config = Arc::new(loaded_config);
            unity_bridge::initialize_background_hook(config.unity_background_hook_enabled());
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
            startup_for_setup.mark("setup_session_store_ready");

            let working_dir_file = data_dir.join("working_dir.txt");
            let initial_working_dir = std::fs::read_to_string(&working_dir_file)
                .ok()
                .and_then(|s| {
                    let trimmed = s.trim().to_string();
                    if std::path::Path::new(&trimmed).is_dir() {
                        Some(trimmed)
                    } else {
                        None
                    }
                })
                .unwrap_or_default();
            println!("[Locus] working_dir: {}", initial_working_dir);

            if !initial_working_dir.is_empty() {
                commands::save_recent_dir_pub(&data_dir, &initial_working_dir);
            }

            let initial_workspace_id = if !initial_working_dir.is_empty() {
                workspace::load_or_create_workspace(&initial_working_dir).ok()
            } else {
                None
            };
            println!("[Locus] workspace_id: {:?}", initial_workspace_id);
            startup_for_setup.mark("setup_workspace_ready");

            let initial_working_dir_copy = initial_working_dir.clone();
            let workspace = Arc::new(Workspace::new(initial_working_dir, initial_workspace_id));

            let mut app_agent_dir_candidates = vec![
                std::path::PathBuf::from("../agent"), // dev: src-tauri/../agent
                std::path::PathBuf::from("agent"),    // cwd
                data_dir.join("agent"),               // production: app_data_dir/agent
            ];
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

            let project_agent_dir = std::path::Path::new(&initial_working_dir_copy)
                .join("Locus")
                .join("agent");
            let project_agent_opt = if project_agent_dir.is_dir() {
                println!("[Locus] project agent dir: {:?}", project_agent_dir);
                Some(project_agent_dir.as_path())
            } else {
                None
            };

            let initial_registry = AgentDefRegistry::load_with_plugins(
                app_agent_dir.0.as_deref(),
                project_agent_opt,
                &crate::plugin::installed_agent_sources(&initial_working_dir_copy),
            );
            let initial_subagents = initial_registry.list_task_agent_descriptions();
            let registry = AgentDefRegistryState(Arc::new(tokio::sync::RwLock::new(initial_registry)));
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

            let knowledge_library_dir = if initial_working_dir_copy.trim().is_empty() {
                knowledge_index::no_workspace_library_dir()
            } else {
                knowledge_index::library_dir_for_working_dir(&initial_working_dir_copy)
            };
            let knowledge_runtime =
                knowledge_index::KnowledgeRuntime::open(&knowledge_library_dir, &data_dir)
                    .map_err(|e| format!("Failed to initialize knowledge index runtime: {}", e))?;
            let knowledge_index_state: Arc<knowledge_index::KnowledgeIndexState> =
                Arc::new(knowledge_index::KnowledgeIndexState::new_with_app_handle(
                    knowledge_runtime.db,
                    knowledge_runtime.tantivy,
                    knowledge_runtime.embedding_mgr,
                    app.handle().clone(),
                ));
            let unity_reference_import_state = unity_docs::UnityReferenceImportState::default();
            let feishu_reference_import_state = feishu_docs::FeishuReferenceImportState::default();
            startup_for_setup.mark("setup_knowledge_runtime_ready");

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
                tool_registry.register_task_tool(&subagents);
                println!(
                    "[Locus] task tool registered with {} subagent(s): {}",
                    subagents.len(),
                    subagents
                        .iter()
                        .map(|(id, _)| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let tool_registry = Arc::new(tool_registry);
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
            let pending_input_queue: PendingInputQueueHandle =
                Arc::new(std::sync::Mutex::new(session::pending_inputs::PendingInputQueue::default()));

            let question_store: QuestionStore = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let agent_graph_tool_store: commands::AgentGraphToolStore =
                Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let knowledge_proposal_drafts: KnowledgeProposalDraftStore =
                Arc::new(tokio::sync::Mutex::new(HashMap::new()));

            let undo_manager: UndoManagerHandle = Arc::new(vcs::UndoManager::new(vcs::GitProvider));
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
            let initial_tool_perms: HashMap<String, String> =
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
            println!("[Locus] tool_permissions: {:?}", initial_tool_perms);
            let tool_permissions: ToolPermissions =
                ToolPermissions(Arc::new(tokio::sync::RwLock::new(initial_tool_perms)));
            startup_for_setup.mark("setup_permissions_ready");

            let unity_monitor: UnityMonitorHandle = Arc::new(tokio::sync::Mutex::new(None));

            let last_scan_info_state = commands::asset::LastScanInfoState::new();
            let scan_phase_state = commands::asset::ScanPhaseState::new();
            let preview_cache = commands::asset::WorkspacePreviewCache::new();
            let dir_entries_cache = commands::DirEntriesPageCache::new();

            let mut startup_ref_graph_reconcile: Option<(
                std::path::PathBuf,
                Arc<std::sync::Mutex<Option<asset_db::AssetDb>>>,
            )> = None;
            let ref_graph_state = match asset_db::AssetDb::load_existing(std::path::Path::new(
                &initial_working_dir_copy,
            )) {
                asset_db::LoadExistingAssetDb::Ready(graph) => {
                    let project_root = std::path::Path::new(&initial_working_dir_copy);
                    match commands::asset::read_persisted_last_scan_info(
                        std::path::Path::new(&initial_working_dir_copy),
                    ) {
                        Ok(Some(info)) => last_scan_info_state.set(info),
                        Ok(None) => {}
                        Err(err) => {
                            eprintln!(
                                "[Locus] warning: failed to load persisted asset scan info: {}",
                                err
                            );
                        }
                    };
                    let db_path = project_root.join("Library").join("Locus").join("locus.db");
                    eprintln!("[Locus] existing ref_graph DB loaded: {}", db_path.display());
                    let graph_state = Arc::new(std::sync::Mutex::new(Some(graph)));
                    startup_ref_graph_reconcile =
                        Some((project_root.to_path_buf(), graph_state.clone()));
                    AssetDbState(graph_state)
                }
                asset_db::LoadExistingAssetDb::NeedsRescan(issue) => {
                    if let Err(err) = commands::asset::delete_persisted_last_scan_info(
                        std::path::Path::new(&initial_working_dir_copy),
                    ) {
                        eprintln!(
                            "[Locus] warning: failed to clear stale asset scan info: {}",
                            err
                        );
                    }
                    eprintln!(
                        "[Locus] existing ref_graph DB invalidated, rescan required: {}",
                        issue.message
                    );
                    scan_phase_state.set(Some(asset_db::types::ScanPhase::Error {
                        error: issue.to_app_error(),
                    }));
                    AssetDbState(Arc::new(std::sync::Mutex::new(None)))
                }
                asset_db::LoadExistingAssetDb::Missing => {
                    if let Err(err) = commands::asset::delete_persisted_last_scan_info(
                        std::path::Path::new(&initial_working_dir_copy),
                    ) {
                        eprintln!(
                            "[Locus] warning: failed to clear stale asset scan info: {}",
                            err
                        );
                    }
                    eprintln!("[Locus] no existing ref_graph DB, waiting for manual scan");
                    AssetDbState(Arc::new(std::sync::Mutex::new(None)))
                }
            };
            startup_for_setup.mark("setup_asset_db_ready");

            let watcher_handle: AssetDbWatcherHandle = Arc::new(std::sync::Mutex::new(None));
            let knowledge_watcher_handle: KnowledgeFsWatcherHandle =
                Arc::new(std::sync::Mutex::new(None));
            let watcher_tuning = Arc::new(crate::asset_db::watcher::WatcherTuning::new());
            let ref_graph_scan_task_state = commands::RefGraphScanTaskState::new();
            let asset_reconcile_task_state = commands::asset::AssetDbReconcileTaskState::new();

            if ref_graph_state.0.lock().unwrap().is_some() {
                let graph_arc = ref_graph_state.0.clone();
                let watcher_root = std::path::PathBuf::from(&initial_working_dir_copy);
                match AssetDbWatcher::start(watcher_root, graph_arc, watcher_tuning.clone()) {
                    Ok(w) => {
                        *watcher_handle.lock().unwrap() = Some(w);
                        eprintln!("[Locus] ref_graph watcher started (from existing DB)");
                    }
                    Err(e) => {
                        eprintln!("[Locus] warning: failed to start ref_graph watcher: {}", e);
                    }
                }
            }

            if let Some((project_root, graph_state)) = startup_ref_graph_reconcile.take() {
                let startup_for_reconcile = startup_for_setup.clone();
                let workspace_generation = workspace.generation();
                let registration = asset_reconcile_task_state.register(
                    project_root.display().to_string(),
                    workspace_generation,
                );
                let cancel_token = registration.cancel_token();
                scan_phase_state.set(Some(asset_db::types::ScanPhase::reconcile_started(true)));
                let app_handle_for_reconcile = app.handle().clone();
                let scan_phase_state_for_reconcile = scan_phase_state.clone();
                let workspace_for_reconcile = workspace.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let _registration = registration;
                    startup_for_reconcile.mark("asset_reconcile_task_start");
                    let started_at = Instant::now();
                    let app_handle_for_progress = app_handle_for_reconcile.clone();
                    let scan_phase_state_for_progress = scan_phase_state_for_reconcile.clone();
                    let workspace_for_progress = workspace_for_reconcile.clone();
                    match asset_db::watcher::reconcile_graph_state_with_cancel_and_progress(
                        &project_root,
                        graph_state,
                        &cancel_token,
                        true,
                        |progress| {
                            if workspace_for_progress.generation() != workspace_generation {
                                return;
                            }
                            let phase = progress.to_scan_phase();
                            let _ = app_handle_for_progress.emit("ref-graph-scan", &phase);
                            scan_phase_state_for_progress.set(Some(phase));
                        },
                    ) {
                        Ok(stats) => {
                            if cancel_token.load(std::sync::atomic::Ordering::Relaxed)
                                || workspace_for_reconcile.generation() != workspace_generation
                            {
                                eprintln!(
                                    "[Locus] existing ref_graph DB background reconcile cancelled: elapsed={}ms",
                                    started_at.elapsed().as_millis()
                                );
                            } else {
                                tracing::info!(
                                    log_module = "Locus",
                                    "existing ref_graph DB reconciled in background: queued={}, processed={}, failed={}, elapsed={}ms",
                                    stats.queued,
                                    stats.processed,
                                    stats.failed,
                                    started_at.elapsed().as_millis()
                                );
                                let phase = asset_db::types::ScanPhase::ReconcileDone;
                                let _ = app_handle_for_reconcile.emit("ref-graph-scan", &phase);
                                if scan_phase_state_for_reconcile
                                    .snapshot()
                                    .as_ref()
                                    .map(|phase| {
                                        matches!(phase, asset_db::types::ScanPhase::Reconcile { .. })
                                    })
                                    .unwrap_or(false)
                                {
                                    scan_phase_state_for_reconcile.clear();
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "[Locus] existing ref_graph DB background reconcile failed: {}",
                                err
                            );
                            if !cancel_token.load(std::sync::atomic::Ordering::Relaxed)
                                && workspace_for_reconcile.generation() == workspace_generation
                            {
                                let phase = asset_db::types::ScanPhase::Error {
                                    error: crate::error::AppError::new(
                                        "ref_graph.rescan_required.reconcile_failed",
                                        "Persisted asset database could not be verified. Run a rescan to rebuild it.",
                                    )
                                    .detail(err)
                                    .retryable(true),
                                };
                                let _ = app_handle_for_reconcile.emit("ref-graph-scan", &phase);
                                scan_phase_state_for_reconcile.set(Some(phase));
                            }
                        }
                    }
                    startup_for_reconcile.mark("asset_reconcile_task_done");
                });
            }

            if !initial_working_dir_copy.trim().is_empty() {
                if let Err(error) =
                    crate::knowledge_store::ensure_knowledge_roots(&initial_working_dir_copy)
                {
                    eprintln!(
                        "[Locus] warning: failed to prepare knowledge roots before watcher start: {}",
                        error
                    );
                }
                match knowledge_watcher::KnowledgeFsWatcher::start(
                    app.handle().clone(),
                    initial_working_dir_copy.clone(),
                    app_knowledge_dir.0.as_ref().as_ref().cloned(),
                    knowledge_index_state.clone(),
                ) {
                    Ok(watcher) => {
                        *knowledge_watcher_handle.lock().unwrap() = Some(watcher);
                        eprintln!("[Locus] knowledge fs watcher started");
                    }
                    Err(error) => {
                        eprintln!(
                            "[Locus] warning: failed to start knowledge fs watcher: {}",
                            error
                        );
                    }
                }
            }
            startup_for_setup.mark("setup_watchers_ready");

            app.manage(config);
            app.manage(auth_state);
            app.manage(codex_state);
            app.manage(api_key_state);
            app.manage(app_knowledge_dir);
            app.manage(app_agent_dir);
            app.manage(provider_keys);
            app.manage(store);
            app.manage(registry);
            app.manage(tool_registry);
            app.manage(workspace.clone());
            app.manage(raw_context_store);
            app.manage(active_tasks);
            app.manage(pending_input_queue);
            app.manage(unity_monitor.clone());
            app.manage(ref_graph_state);
            app.manage(watcher_handle);
            app.manage(knowledge_watcher_handle);
            app.manage(crate::asset_db::watcher::WatcherTuningState(watcher_tuning));
            app.manage(ref_graph_scan_task_state);
            app.manage(asset_reconcile_task_state);
            app.manage(last_scan_info_state);
            app.manage(scan_phase_state);
            app.manage(preview_cache);
            app.manage(dir_entries_cache);
            app.manage(question_store);
            app.manage(agent_graph_tool_store);
            app.manage(knowledge_proposal_drafts);
            app.manage(undo_manager);
            app.manage(view_automation_store);
            app.manage(tool_permission_mode);
            app.manage(tool_permissions);
            app.manage(binary_cache);
            app.manage(knowledge_index_state.clone());
            app.manage(unity_reference_import_state);
            app.manage(feishu_reference_import_state);
            app.manage(log_store_for_setup.clone());
            startup_for_setup.mark("setup_state_managed");
            startup_for_setup.mark("setup_backend_ready");

            let main_window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == MAIN_WINDOW_LABEL)
                .ok_or_else(|| format!("Missing '{}' window config", MAIN_WINDOW_LABEL))?;
            startup_for_setup.mark("main_window_build_start");
            tauri::WebviewWindowBuilder::from_config(app.handle(), main_window_config)?.build()?;
            startup_for_setup.mark("main_window_build_done");
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

            let app_handle = app.handle().clone();
            let workspace_for_unity = workspace.clone();
            let startup_for_unity = startup_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                startup_for_unity.mark("unity_monitor_task_start");
                let wd = workspace_for_unity.path.read().await.clone();
                let is_unity = unity_bridge::is_unity_project(&wd);
                eprintln!(
                    "[Locus] working_dir='{}', is_unity_project={}",
                    wd, is_unity
                );
                if is_unity {
                    unity_bridge::start_unity_monitor(
                        app_handle.clone(),
                        wd.clone(),
                        &unity_monitor,
                    )
                    .await;
                    unity_bridge::emit_plugin_status(&app_handle, &wd);
                }
                startup_for_unity.mark("unity_monitor_task_done");
            });

            let knowledge_startup_state = knowledge_index_state.clone();
            let workspace_for_knowledge = workspace.clone();
            let app_handle_for_knowledge = app.handle().clone();
            let startup_for_knowledge = startup_for_setup.clone();
            tauri::async_runtime::spawn(async move {
                startup_for_knowledge.mark("knowledge_startup_task_start");
                let wd = workspace_for_knowledge.path.read().await.clone();
                if wd.trim().is_empty() {
                    startup_for_knowledge.mark("knowledge_startup_task_skipped");
                    return;
                }
                let app_knowledge_dir: tauri::State<'_, AppKnowledgeDir> =
                    app_handle_for_knowledge.state();
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
            commands::insert_pending_chat_input,
            commands::delete_pending_chat_input,
            commands::list_agents,
            commands::list_subagent_defs,
            commands::get_agent_system_prompt,
            commands::get_agent_env_template,
            commands::get_agent_rendered_env_prompt,
            commands::get_agent_system_prompt_stats,
            commands::list_agent_injected_items,
            commands::set_agent_tool_direct_load,
            commands::load_session,
            commands::list_sessions,
            commands::list_archived_sessions,
            commands::get_active_session_selection,
            commands::save_active_session_selection,
            commands::rename_session,
            commands::archive_session,
            commands::unarchive_session,
            commands::delete_session,
            commands::get_session_usage,
            commands::get_session_active_run,
            commands::list_session_events,
            commands::get_auth_status,
            commands::get_auth_url,
            commands::exchange_auth_code,
            commands::auth_logout,
            commands::save_api_key,
            commands::clear_api_key,
            commands::get_providers,
            commands::save_provider_key,
            commands::delete_provider_key,
            commands::get_app_storage_info,
            commands::get_app_temp_info,
            commands::clear_app_temp_dir,
            commands::open_app_storage_dir,
            commands::open_app_temp_dir,
            commands::schedule_app_storage_migration,
            commands::clear_app_storage_migration,
            commands::get_working_dir,
            commands::set_working_dir,
            commands::list_recent_dirs,
            commands::remove_recent_dir,
            commands::open_dir_in_file_explorer,
            commands::list_dir_entries,
            commands::list_dir_entries_page,
            commands::search_workspace_entries,
            commands::stat_workspace_entries,
            commands::save_raw_context,
            commands::get_todos,
            commands::cancel_chat,
            commands::stale_knowledge_proposals,
            commands::ignore_knowledge_proposal,
            commands::apply_knowledge_proposal,
            commands::check_unity_connection,
            commands::check_unity_connection_status,
            commands::get_unity_console_text,
            commands::check_unity_plugin,
            commands::install_unity_plugin,
            commands::launch_unity_project,
            commands::send_unity_log,
            commands::select_unity_asset,
            commands::open_unity_asset_inspector,
            commands::select_unity_scene_object,
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
            commands::knowledge_close_download_progress_window,
            commands::knowledge_close_lexical_progress_window,
            commands::knowledge_close_unity_reference_import_progress_window,
            commands::knowledge_close_feishu_reference_import_progress_window,
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
            commands::knowledge_list_page,
            commands::knowledge_list_directories,
            commands::knowledge_list_directory_documents,
            commands::knowledge_list_directory_documents_page,
            commands::knowledge_list_external_reference_directories,
            commands::knowledge_list_unity_managed_directory_stats,
            commands::knowledge_query,
            commands::knowledge_read,
            commands::knowledge_import_unity_reference_docs,
            commands::knowledge_import_feishu_reference_docs,
            commands::knowledge_delete_unity_reference_docs,
            commands::knowledge_delete_feishu_reference_docs,
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
            commands::plugin_list_installed,
            commands::plugin_install_from_path,
            commands::plugin_uninstall,
            commands::plugin_export,
            commands::open_file_external,
            commands::reveal_workspace_file,
            commands::knowledge_reveal_target,
            commands::resolve_markdown_image,
            commands::preview_workspace_file,
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
            commands::get_model_defaults,
            commands::save_model_defaults,
            commands::get_codex_model_config,
            commands::get_codex_available_models,
            commands::save_codex_model_config,
            commands::get_custom_endpoints,
            commands::save_custom_endpoints,
            commands::test_custom_endpoint,
            commands::codex_status,
            commands::codex_start_login,
            commands::codex_poll_login,
            commands::codex_logout,
            commands::codex_retry_auth,
            commands::codex_rate_limits,
            commands::diff_single_file,
            commands::diff_semantic_target,
            commands::diff_text_for_large,
            commands::diff_strings,
            commands::undo_latest_conversation_turn,
            commands::rollback_session_to_message,
            commands::undo_perform,
            commands::undo_perform_to_message,
            commands::undo_preview,
            commands::undo_list,
            commands::undo_check_conflicts,
            commands::get_debug_mode,
            commands::set_debug_mode,
            commands::get_file_tool_workspace_boundary,
            commands::set_file_tool_workspace_boundary,
            commands::get_tool_permission_mode,
            commands::save_tool_permission_mode,
            commands::get_tool_permissions,
            commands::save_tool_permissions,
            commands::reset_all_config,
            commands::save_plan_artifact,
            commands::get_system_fonts,
            commands::get_system_locale,
            commands::get_close_behavior,
            commands::set_close_behavior,
            commands::get_dynamic_tool_loading_mode,
            commands::set_dynamic_tool_loading_mode,
            commands::get_unity_background_hook_enabled,
            commands::set_unity_background_hook_enabled,
            commands::get_unity_background_hook_status,
            commands::get_view_windows_above_main,
            commands::set_view_windows_above_main,
            commands::get_view_open_in_existing_window,
            commands::set_view_open_in_existing_window,
            commands::get_proxy_status,
            commands::save_proxy_config,
            commands::get_python_runtime_state,
            commands::save_python_runtime_selection,
            commands::send_system_notification,
            commands::play_custom_notification_sound,
            commands::request_app_exit,
            commands::get_config_registry,
            commands::get_log_entries,
            commands::clear_log_entries,
            commands::save_log_export,
            commands::unity_embed_status,
            commands::unity_embed_open_frontend_window,
            commands::unity_embed_set_mouse_activation_suppressed,
            commands::unity_embed_activate_for_input,
            commands::unity_embed_set_drag_passthrough,
            commands::unity_embed_focus_debug_snapshot,
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
            commands::view_detach_tab,
            commands::view_host_pool_prepare,
            commands::view_host_pool_ready,
            commands::view_host_revealed,
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
            commands::agent_graph_tool_request,
            commands::agent_graph_tool_submit,
            commands::agent_graph_tool_cancel,
            commands::agent_graph_tool_reopen,
            commands::fetch_app_update_manifest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
