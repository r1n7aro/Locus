#[macro_use]
mod logging;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::{Manager, WindowEvent};

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
pub mod process_util;
pub mod prompt;
mod session;
mod tool;
pub mod unity_bridge;
pub mod unity_csharp;
mod unity_docs;
pub mod unity_yaml;
pub mod vcs;
#[cfg(target_os = "windows")]
mod windows_resize_sync;
#[cfg(target_os = "windows")]
mod windows_window_frame;
mod workspace;

use agent::definition::AgentDefRegistry;
use agent::instance::{AssistantStreamState, RawContextStore};
use commands::AppKnowledgeDir;

const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Clone)]
pub struct AppAgentDir(pub Arc<Option<std::path::PathBuf>>);
use asset_db::watcher::AssetDbWatcher;
pub use asset_db::AssetDbState;
use auth::codex::CodexAuthState;
use auth::AuthState;
use commands::{CanvasSpecStore, CodexAuthStateHandle};
use config::AppConfig;

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

pub type ToolPermissionMode = Arc<tokio::sync::RwLock<String>>;

pub type ToolPermissions = Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared_debug_flag = Arc::new(AtomicBool::new(
        std::env::var("LOCUS_DEBUG")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "True"))
            .unwrap_or(false),
    ));
    let log_store = Arc::new(logging::AppLogStore::new(logging::DEFAULT_LOG_CAPACITY));
    logging::init_tracing(shared_debug_flag.clone(), log_store.clone());
    #[cfg(windows)]
    knowledge_index::embedding::preload_ort_helpers();

    let binary_cache: Arc<binary_cache::BinaryCache> = Arc::new(binary_cache::BinaryCache::new());
    let cache_for_protocol = binary_cache.clone();
    let debug_flag_for_setup = shared_debug_flag.clone();
    let log_store_for_setup = log_store.clone();

    tauri::Builder::default()
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
        .on_window_event(|window, event| {
            if window.label() != MAIN_WINDOW_LABEL {
                return;
            }

            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app_handle = window.app_handle().clone();
                commands::destroy_unity_embed_control_window_on_main(&app_handle);
                app_handle.exit(0);
            }
        })
        .setup(move |app| {
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
            commands::restore_saved_git_override(&app.handle().clone());

            println!("[Locus] data_dir: {:?}", data_dir);

            let mut loaded_config = AppConfig::load(&data_dir);
            debug_flag_for_setup.store(loaded_config.debug_enabled(), Ordering::Relaxed);
            loaded_config.debug = debug_flag_for_setup.clone();
            let config = Arc::new(loaded_config);

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

            let store = Arc::new(
                SessionStore::new(&data_dir)
                    .map_err(|e| format!("Failed to initialize SessionStore: {}", e))?,
            );

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

            let initial_working_dir_copy = initial_working_dir.clone();
            let workspace = Arc::new(Workspace {
                path: tokio::sync::RwLock::new(initial_working_dir),
                workspace_id: tokio::sync::RwLock::new(initial_workspace_id),
            });

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

            let registry = Arc::new(AgentDefRegistry::load(
                app_agent_dir.0.as_deref(),
                project_agent_opt,
            ));

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

            let mut tool_registry = ToolRegistry::with_builtins();
            let subagents = registry.list_task_agent_descriptions();
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

            let provider_keys: ProviderKeysState = Arc::new(tokio::sync::RwLock::new(
                commands::load_provider_keys_from_keychain(&data_dir),
            ));
            println!("[Locus] provider keys loaded from keychain");

            let raw_context_store: RawContextStore =
                Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

            let active_tasks: ActiveTasks = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

            let question_store: QuestionStore = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let knowledge_proposal_drafts: KnowledgeProposalDraftStore =
                Arc::new(tokio::sync::Mutex::new(HashMap::new()));

            let undo_manager: UndoManagerHandle = Arc::new(vcs::UndoManager::new(vcs::GitProvider));

            let tool_mode_path = data_dir.join("tool_permission_mode.txt");
            let initial_tool_mode = std::fs::read_to_string(&tool_mode_path)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| s == "ask")
                .unwrap_or_else(|| "auto".to_string());
            println!("[Locus] tool_permission_mode: {}", initial_tool_mode);
            let tool_permission_mode: ToolPermissionMode =
                Arc::new(tokio::sync::RwLock::new(initial_tool_mode));

            let tool_perm_path = data_dir.join("tool_permissions.json");
            let initial_tool_perms: std::collections::HashMap<String, String> =
                std::fs::read_to_string(&tool_perm_path)
                    .ok()
                    .and_then(|s| {
                        serde_json::from_str::<std::collections::HashMap<String, String>>(&s).ok()
                    })
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
                Arc::new(tokio::sync::RwLock::new(initial_tool_perms));

            let canvas_spec_store: CanvasSpecStore =
                Arc::new(tokio::sync::Mutex::new(HashMap::new()));

            let unity_monitor: UnityMonitorHandle = Arc::new(tokio::sync::Mutex::new(None));

            let last_scan_info_state = commands::asset::LastScanInfoState::new();
            let scan_phase_state = commands::asset::ScanPhaseState::new();
            let preview_cache = commands::asset::WorkspacePreviewCache::new();
            let dir_entries_cache = commands::DirEntriesPageCache::new();

            let ref_graph_state = match asset_db::AssetDb::load_existing(std::path::Path::new(
                &initial_working_dir_copy,
            )) {
                asset_db::LoadExistingAssetDb::Ready(graph) => {
                    let project_root = std::path::Path::new(&initial_working_dir_copy);
                    match asset_db::watcher::reconcile_loaded_db(project_root, graph) {
                        Ok((graph, stats)) => {
                            eprintln!(
                                "[Locus] existing ref_graph DB reconciled: queued={}, processed={}, failed={}",
                                stats.queued, stats.processed, stats.failed
                            );
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
                            }
                            let db_path = std::path::Path::new(&initial_working_dir_copy)
                                .join("Library")
                                .join("Locus")
                                .join("locus.db");
                            eprintln!(
                                "[Locus] existing ref_graph DB loaded: {}",
                                db_path.display()
                            );
                            AssetDbState(Arc::new(std::sync::Mutex::new(Some(graph))))
                        }
                        Err(err) => {
                            if let Err(clear_err) =
                                commands::asset::delete_persisted_last_scan_info(project_root)
                            {
                                eprintln!(
                                    "[Locus] warning: failed to clear stale asset scan info: {}",
                                    clear_err
                                );
                            }
                            eprintln!(
                                "[Locus] existing ref_graph DB reconcile failed, rescan required: {}",
                                err
                            );
                            scan_phase_state.set(Some(asset_db::types::ScanPhase::Error {
                                error: error::AppError::new(
                                    "ref_graph.rescan_required.reconcile_failed",
                                    "Persisted asset database could not be reconciled. Run a rescan to rebuild it.",
                                )
                                .detail(err)
                                .retryable(true),
                            }));
                            AssetDbState(Arc::new(std::sync::Mutex::new(None)))
                        }
                    }
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

            let watcher_handle: AssetDbWatcherHandle = Arc::new(std::sync::Mutex::new(None));
            let knowledge_watcher_handle: KnowledgeFsWatcherHandle =
                Arc::new(std::sync::Mutex::new(None));
            let watcher_tuning = Arc::new(crate::asset_db::watcher::WatcherTuning::new());

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
            app.manage(unity_monitor.clone());
            app.manage(ref_graph_state);
            app.manage(watcher_handle);
            app.manage(knowledge_watcher_handle);
            app.manage(crate::asset_db::watcher::WatcherTuningState(watcher_tuning));
            app.manage(last_scan_info_state);
            app.manage(scan_phase_state);
            app.manage(preview_cache);
            app.manage(dir_entries_cache);
            app.manage(question_store);
            app.manage(knowledge_proposal_drafts);
            app.manage(canvas_spec_store);
            app.manage(undo_manager);
            app.manage(tool_permission_mode);
            app.manage(tool_permissions);
            app.manage(binary_cache);
            app.manage(knowledge_index_state.clone());
            app.manage(unity_reference_import_state);
            app.manage(feishu_reference_import_state);
            app.manage(log_store_for_setup.clone());
            commands::start_unity_embed_control_server(app.handle().clone());
            #[cfg(target_os = "windows")]
            if let Err(error) = windows_window_frame::restore_main_window_frame(app) {
                eprintln!("[Locus] warning: failed to restore main window frame: {error}");
            }
            #[cfg(target_os = "windows")]
            if let Err(error) = windows_resize_sync::install_for_main_window(app) {
                eprintln!("[Locus] warning: failed to install WebView2 resize sync: {error}");
            }

            let app_handle = app.handle().clone();
            let workspace_for_unity = workspace.clone();
            tauri::async_runtime::spawn(async move {
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
            });

            let knowledge_startup_state = knowledge_index_state.clone();
            let workspace_for_knowledge = workspace.clone();
            let app_handle_for_knowledge = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let wd = workspace_for_knowledge.path.read().await.clone();
                if wd.trim().is_empty() {
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
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::create_session,
            commands::chat,
            commands::list_agents,
            commands::list_subagent_defs,
            commands::get_agent_system_prompt,
            commands::get_agent_env_template,
            commands::get_agent_system_prompt_stats,
            commands::list_agent_injected_items,
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
            commands::open_app_storage_dir,
            commands::schedule_app_storage_migration,
            commands::clear_app_storage_migration,
            commands::get_working_dir,
            commands::set_working_dir,
            commands::list_recent_dirs,
            commands::list_dir_entries,
            commands::list_dir_entries_page,
            commands::search_workspace_entries,
            commands::save_raw_context,
            commands::get_todos,
            commands::cancel_chat,
            commands::stale_knowledge_proposals,
            commands::ignore_knowledge_proposal,
            commands::apply_knowledge_proposal,
            commands::check_unity_connection,
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
            commands::asset_db_overview,
            commands::asset_risk_report,
            commands::get_watcher_tuning,
            commands::set_watcher_tuning,
            commands::search_workspace_assets,
            commands::preview_workspace_asset,
            commands::preview_workspace_asset_target,
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
            commands::git_commit_body,
            commands::git_probe,
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
            commands::create_skill_scaffold,
            commands::open_file_external,
            commands::reveal_workspace_file,
            commands::knowledge_reveal_target,
            commands::preview_workspace_file,
            commands::list_rules,
            commands::save_rule,
            commands::read_rule,
            commands::delete_rule,
            commands::set_rule_enabled,
            commands::set_rule_order,
            commands::canvas_set_spec,
            commands::canvas_get_spec,
            commands::canvas_update_field,
            commands::canvas_refresh,
            commands::canvas_save,
            commands::canvas_load,
            commands::canvas_list,
            commands::canvas_delete,
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
            commands::diff_single_file,
            commands::diff_semantic_target,
            commands::diff_text_for_large,
            commands::diff_strings,
            commands::undo_perform,
            commands::undo_preview,
            commands::undo_list,
            commands::undo_check_conflicts,
            commands::get_debug_mode,
            commands::set_debug_mode,
            commands::get_tool_permission_mode,
            commands::save_tool_permission_mode,
            commands::get_tool_permissions,
            commands::save_tool_permissions,
            commands::reset_all_config,
            commands::save_plan_artifact,
            commands::get_system_fonts,
            commands::get_system_locale,
            commands::send_system_notification,
            commands::get_config_registry,
            commands::get_log_entries,
            commands::clear_log_entries,
            commands::unity_embed_status,
            commands::unity_embed_set_mouse_activation_suppressed,
            commands::unity_embed_activate_for_input,
            commands::unity_embed_focus_debug_snapshot,
            commands::fetch_app_update_manifest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
