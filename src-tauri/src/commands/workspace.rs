use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::asset_db::{AssetDb, AssetDbState, LoadExistingAssetDb};
use crate::commands::asset::{
    delete_persisted_last_scan_info, read_persisted_last_scan_info, LastScanInfoState,
    ScanPhaseState, WorkspacePreviewCache,
};
use crate::error::AppError;
use crate::keychain;
use crate::unity_bridge::UnityMonitorHandle;
use crate::workspace::Workspace;
use crate::AssetDbWatcherHandle;
use crate::KnowledgeFsWatcherHandle;

const ENDPOINT_TEST_HTML_RESPONSE_CODE: &str = "endpoint_test.html_response";

/// Returns a stable app config directory inside the OS config root.
/// On Windows this resolves under `%APPDATA%\\locus`, which keeps model config
/// under the app-data tree while staying outside Tauri's bundle-specific
/// `app_data_dir` that may be cleared during reinstall.
pub(crate) fn persistent_config_dir() -> Result<std::path::PathBuf, String> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Failed to get config directory".to_string())?;
    let dir = config_dir.join("locus");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create persistent config dir: {}", e))?;
    Ok(dir)
}

fn read_nonempty_string(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(crate) fn custom_endpoints_path(_app_handle: &AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("custom_endpoints.json"))
}

#[tauri::command]
pub async fn get_working_dir(workspace: State<'_, Arc<Workspace>>) -> Result<String, AppError> {
    let dir = workspace.path.read().await.clone();
    Ok(dir)
}

#[tauri::command]
pub async fn set_working_dir(
    path: String,
    workspace: State<'_, Arc<Workspace>>,
    unity_monitor: State<'_, UnityMonitorHandle>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    knowledge_watcher_handle: State<'_, KnowledgeFsWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    preview_cache: State<'_, WorkspacePreviewCache>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
    watcher_tuning: State<'_, crate::asset_db::watcher::WatcherTuningState>,
    knowledge_index_state: State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>>,
    app_knowledge_dir: State<'_, crate::commands::AppKnowledgeDir>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let p = std::path::Path::new(&path);
    if !p.is_dir() {
        return Err(format!("Directory not found: {}", path).into());
    }

    if !p.join("Assets").is_dir() {
        return Err(
            "Selected directory is not a Unity project (Assets/ folder not found)"
                .to_string()
                .into(),
        );
    }

    let canonical = dunce::canonicalize(p)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.clone());

    let ws_id = crate::workspace::load_or_create_workspace(&canonical)?;

    // Decide whether the workspace is actually changing. We compare the
    // canonical form against the currently-stored cwd. If unchanged, we keep
    // the previous `LastScanInfo` so the asset page status row stays accurate;
    // a re-`set_working_dir` of the same project should not erase its history.
    let prev_cwd = workspace.path.read().await.clone();
    let is_real_switch = prev_cwd != canonical;

    {
        let mut dir = workspace.path.write().await;
        *dir = canonical.clone();
    }
    if is_real_switch {
        // Only now is it safe to clear: the new workspace path is committed,
        // all prior validation passed, and it differs from the previous one.
        // Both the cached scan timestamp and any sticky scan-phase state from
        // the previous project belong to a workspace we are no longer in.
        last_scan_info.clear();
        scan_phase_state.clear();
        // Drop any preview sessions parsed against the previous workspace —
        // they hold owned YAML docs that would otherwise be paired with the
        // new project's AssetDb in `preview_workspace_asset_target`.
        preview_cache.clear();
        dir_entries_cache.clear();
    }
    {
        let mut wid = workspace.workspace_id.write().await;
        *wid = Some(ws_id.clone());
    }

    if is_real_switch {
        super::reset_unity_embed_control_window(&app_handle);
        super::refresh_unity_embed_control_server(app_handle.clone());
    }

    if let Ok(data_dir) = super::resolve_runtime_storage_dir(&app_handle) {
        let file: std::path::PathBuf = data_dir.join("working_dir.txt");
        let _ = std::fs::write(&file, &canonical);
        save_recent_dir(&data_dir, &canonical);
    }

    if is_real_switch {
        let library_dir = crate::knowledge_index::library_dir_for_working_dir(&canonical);
        let model_storage_dir = super::resolve_runtime_storage_dir(&app_handle)?;
        knowledge_index_state
            .rebuild(&library_dir, &model_storage_dir)
            .await?;
        let knowledge_state = knowledge_index_state.inner().clone();
        let working_dir_for_index = canonical.clone();
        let app_handle_for_index = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            let app_knowledge_dir: tauri::State<'_, crate::commands::AppKnowledgeDir> =
                app_handle_for_index.state();
            if let Err(e) = crate::knowledge_index::maybe_auto_activate_embedding_runtime(
                knowledge_state.clone(),
                &working_dir_for_index,
                app_knowledge_dir.0.as_ref().as_ref(),
            )
            .await
            {
                eprintln!("[Locus] knowledge embedding auto-activate error: {}", e);
            }
            if let Err(e) = crate::knowledge_index::reconcile_workspace(
                &working_dir_for_index,
                app_knowledge_dir.0.as_ref().as_ref(),
                knowledge_state,
            )
            .await
            {
                eprintln!("[Locus] knowledge reconcile error: {}", e);
            }
        });
    }

    if is_real_switch {
        if let Err(error) = crate::knowledge_store::ensure_knowledge_roots(&canonical) {
            eprintln!(
                "[Locus] warning: failed to prepare knowledge roots for new working dir: {}",
                error
            );
        }
        let mut knowledge_watcher = knowledge_watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = knowledge_watcher.take() {
            old.stop();
            eprintln!("[Locus] stopped knowledge watcher (working dir changed)");
        }
        match crate::knowledge_watcher::KnowledgeFsWatcher::start(
            app_handle.clone(),
            canonical.clone(),
            app_knowledge_dir.0.as_ref().as_ref().cloned(),
            knowledge_index_state.inner().clone(),
        ) {
            Ok(watcher) => {
                *knowledge_watcher = Some(watcher);
                eprintln!("[Locus] knowledge watcher started for new working dir");
            }
            Err(error) => {
                eprintln!(
                    "[Locus] warning: failed to start knowledge watcher: {}",
                    error
                );
            }
        }
    }

    {
        let mut wh = watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = wh.take() {
            old.stop_and_join();
            eprintln!("[Locus] stopped ref_graph watcher (working dir changed)");
        }
    }

    match AssetDb::load_existing(std::path::Path::new(&canonical)) {
        LoadExistingAssetDb::Ready(graph) => {
            match crate::asset_db::watcher::reconcile_loaded_db(
                std::path::Path::new(&canonical),
                graph,
            ) {
                Ok((graph, stats)) => {
                    eprintln!(
                        "[Locus] ref_graph DB reconciled for new working dir: queued={}, processed={}, failed={}",
                        stats.queued, stats.processed, stats.failed
                    );
                    let db_path = std::path::Path::new(&canonical)
                        .join("Library")
                        .join("Locus")
                        .join("locus.db");
                    eprintln!(
                        "[Locus] ref_graph DB loaded for new working dir: {}",
                        db_path.display()
                    );
                    *ref_graph_state
                        .0
                        .lock()
                        .map_err(|e| format!("Lock error: {}", e))? = Some(graph);
                    match read_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                        Ok(Some(info)) => last_scan_info.set(info),
                        Ok(None) => {
                            if is_real_switch {
                                last_scan_info.clear();
                            }
                        }
                        Err(err) => {
                            eprintln!(
                                "[Locus] warning: failed to load persisted asset scan info: {}",
                                err
                            );
                            if is_real_switch {
                                last_scan_info.clear();
                            }
                        }
                    }

                    let graph_arc = ref_graph_state.0.clone();
                    let watcher_root = std::path::PathBuf::from(&canonical);
                    match crate::asset_db::watcher::AssetDbWatcher::start(
                        watcher_root,
                        graph_arc,
                        watcher_tuning.0.clone(),
                    ) {
                        Ok(w) => {
                            *watcher_handle
                                .lock()
                                .map_err(|e| format!("Lock error: {}", e))? = Some(w);
                            eprintln!("[Locus] ref_graph watcher started for new working dir");
                        }
                        Err(e) => {
                            eprintln!("[Locus] warning: failed to start ref_graph watcher: {}", e);
                        }
                    }
                }
                Err(err) => {
                    eprintln!(
                        "[Locus] ref_graph DB reconcile failed for new working dir, rescan required: {}",
                        err
                    );
                    last_scan_info.clear();
                    if let Err(clear_err) =
                        delete_persisted_last_scan_info(std::path::Path::new(&canonical))
                    {
                        eprintln!(
                            "[Locus] warning: failed to clear stale asset scan info: {}",
                            clear_err
                        );
                    }
                    *ref_graph_state
                        .0
                        .lock()
                        .map_err(|e| format!("Lock error: {}", e))? = None;
                    scan_phase_state.set(Some(crate::asset_db::types::ScanPhase::Error {
                        error: crate::error::AppError::new(
                            "ref_graph.rescan_required.reconcile_failed",
                            "Persisted asset database could not be reconciled. Run a rescan to rebuild it.",
                        )
                        .detail(err)
                        .retryable(true),
                    }));
                }
            }
        }
        LoadExistingAssetDb::NeedsRescan(issue) => {
            eprintln!(
                "[Locus] ref_graph DB invalidated for new working dir, rescan required: {}",
                issue.message
            );
            last_scan_info.clear();
            if let Err(err) = delete_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                eprintln!(
                    "[Locus] warning: failed to clear stale asset scan info: {}",
                    err
                );
            }
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = None;
            scan_phase_state.set(Some(crate::asset_db::types::ScanPhase::Error {
                error: issue.to_app_error(),
            }));
        }
        LoadExistingAssetDb::Missing => {
            eprintln!("[Locus] no ref_graph DB in new working dir, clearing state");
            last_scan_info.clear();
            if let Err(err) = delete_persisted_last_scan_info(std::path::Path::new(&canonical)) {
                eprintln!(
                    "[Locus] warning: failed to clear stale asset scan info: {}",
                    err
                );
            }
            *ref_graph_state
                .0
                .lock()
                .map_err(|e| format!("Lock error: {}", e))? = None;
        }
    }

    if crate::unity_bridge::is_unity_project(&canonical) {
        crate::unity_bridge::start_unity_monitor(
            app_handle.clone(),
            canonical.clone(),
            &unity_monitor,
        )
        .await;
        crate::unity_bridge::emit_plugin_status(&app_handle, &canonical);
    } else {
        crate::unity_bridge::stop_unity_monitor(&unity_monitor).await;
        let _ = app_handle.emit("unity-connection-status", false);
    }

    eprintln!(
        "[Locus] working_dir changed to: {}, workspace_id: {}",
        canonical, ws_id
    );
    Ok(canonical)
}

const MAX_RECENT_DIRS: usize = 8;

pub fn save_recent_dir_pub(data_dir: &std::path::Path, dir: &str) {
    save_recent_dir(data_dir, dir);
}

fn save_recent_dir(data_dir: &std::path::Path, dir: &str) {
    let file = data_dir.join("recent_dirs.json");
    let mut dirs: Vec<String> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    dirs.retain(|d| d != dir);
    dirs.insert(0, dir.to_string());
    dirs.truncate(MAX_RECENT_DIRS);

    let _ = std::fs::write(&file, serde_json::to_string(&dirs).unwrap_or_default());
}

#[tauri::command]
pub async fn list_recent_dirs(app_handle: AppHandle) -> Result<Vec<String>, AppError> {
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let file = data_dir.join("recent_dirs.json");
    let dirs: Vec<String> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Ok(dirs
        .into_iter()
        .filter(|d| std::path::Path::new(d).is_dir())
        .collect())
}

#[tauri::command]
pub async fn get_last_model(_app_handle: AppHandle) -> Result<String, AppError> {
    let primary_path = persistent_config_dir()?.join("last_model.txt");
    if let Some(val) = read_nonempty_string(&primary_path) {
        return Ok(val);
    }
    Ok(String::new())
}

#[tauri::command]
pub async fn save_last_model(model_id: String, _app_handle: AppHandle) -> Result<(), AppError> {
    let trimmed = model_id.trim();
    // Save to persistent location (~/.locus/) — survives reinstalls
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("last_model.txt"), trimmed)
        .map_err(|e| format!("Failed to save last model: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_last_effort(_app_handle: AppHandle) -> Result<String, AppError> {
    let primary_path = persistent_config_dir()?.join("last_effort.txt");
    if let Some(val) = read_nonempty_string(&primary_path) {
        return Ok(val);
    }
    Ok(String::new())
}

#[tauri::command]
pub async fn save_last_effort(effort: String, _app_handle: AppHandle) -> Result<(), AppError> {
    let trimmed = effort.trim();
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("last_effort.txt"), trimmed)
        .map_err(|e| format!("Failed to save last effort: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefaults {
    #[serde(default)]
    pub main_model: String,
    #[serde(default)]
    pub plan_model: String,
    #[serde(default)]
    pub subagent_models: std::collections::HashMap<String, String>,
}

impl Default for ModelDefaults {
    fn default() -> Self {
        ModelDefaults {
            main_model: String::new(),
            plan_model: String::new(),
            subagent_models: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexTransportMode {
    Http,
    Websocket,
}

impl Default for CodexTransportMode {
    fn default() -> Self {
        Self::Websocket
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelConfig {
    #[serde(default)]
    pub transport: CodexTransportMode,
}

fn codex_model_config_path() -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("codex_model_config.json"))
}

pub(crate) fn load_codex_model_config() -> Result<CodexModelConfig, String> {
    let path = codex_model_config_path()?;
    Ok(std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<CodexModelConfig>(&s).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn get_model_defaults(_app_handle: AppHandle) -> Result<ModelDefaults, AppError> {
    let primary_path = persistent_config_dir()?.join("model_defaults.json");
    if let Some(defaults) = std::fs::read_to_string(&primary_path)
        .ok()
        .and_then(|s| serde_json::from_str::<ModelDefaults>(&s).ok())
    {
        return Ok(defaults);
    }
    Ok(ModelDefaults::default())
}

#[tauri::command]
pub async fn save_model_defaults(
    defaults: ModelDefaults,
    _app_handle: AppHandle,
) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(&defaults)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    // Save to persistent location
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {}", e))?;
    std::fs::write(dir.join("model_defaults.json"), &json)
        .map_err(|e| format!("Failed to save model defaults: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_codex_model_config() -> Result<CodexModelConfig, AppError> {
    load_codex_model_config().map_err(AppError::from)
}

#[tauri::command]
pub async fn get_codex_available_models(
    codex: State<'_, crate::commands::auth::CodexAuthStateHandle>,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<Vec<crate::llm::codex_models::CodexAvailableModel>, AppError> {
    let cache_dir = persistent_config_dir().map_err(AppError::from)?;
    let (access_token, account_id) = {
        let mut codex_guard = codex.lock().await;
        let access_token = codex_guard.access_token().await.map_err(AppError::from)?;
        let account_id = codex_guard.account_id();
        (access_token, account_id)
    };

    crate::llm::codex_models::list_codex_available_models(
        &access_token,
        account_id.as_deref(),
        config.base_url.as_deref(),
        &cache_dir,
    )
    .await
    .map_err(AppError::from)
}

#[tauri::command]
pub async fn save_codex_model_config(config: CodexModelConfig) -> Result<(), AppError> {
    let path = codex_model_config_path().map_err(AppError::from)?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize codex model config: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save codex model config: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomReasoningParamFormat {
    None,
    OpenaiChatReasoningEffort,
    OpenaiResponsesReasoningEffort,
    AnthropicThinking,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpoint {
    pub id: String,
    pub name: String,
    pub api_model: String,
    pub endpoint: String,
    pub api_format: ApiFormat,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_context_length")]
    pub context_length: u32,
    #[serde(default)]
    pub beta_flags: Vec<String>,
    #[serde(default = "default_supported_reasoning_efforts")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning_param_format: Option<CustomReasoningParamFormat>,
}

fn default_context_length() -> u32 {
    128_000
}

fn default_supported_reasoning_efforts() -> Vec<String> {
    ["low", "medium", "high", "max"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn default_reasoning_param_format(api_format: &ApiFormat) -> CustomReasoningParamFormat {
    match api_format {
        ApiFormat::OpenaiResponses => CustomReasoningParamFormat::OpenaiResponsesReasoningEffort,
        ApiFormat::AnthropicMessages => CustomReasoningParamFormat::AnthropicThinking,
        ApiFormat::OpenaiChat => CustomReasoningParamFormat::OpenaiChatReasoningEffort,
    }
}

fn normalize_reasoning_effort(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Some(trimmed),
        _ => None,
    }
}

pub(crate) fn normalize_custom_endpoint_config(endpoint: &mut CustomEndpoint) {
    endpoint.supported_reasoning_efforts = endpoint
        .supported_reasoning_efforts
        .iter()
        .filter_map(|value| normalize_reasoning_effort(value))
        .collect();
    if endpoint.supported_reasoning_efforts.is_empty() {
        endpoint.supported_reasoning_efforts = default_supported_reasoning_efforts();
    }
    if endpoint.reasoning_param_format.is_none() {
        endpoint.reasoning_param_format =
            Some(default_reasoning_param_format(&endpoint.api_format));
    }
}

#[tauri::command]
pub async fn get_custom_endpoints(app_handle: AppHandle) -> Result<Vec<CustomEndpoint>, AppError> {
    let path = custom_endpoints_path(&app_handle)?;
    let mut endpoints: Vec<CustomEndpoint> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    for ep in &mut endpoints {
        normalize_custom_endpoint_config(ep);
        if let Ok(Some(key)) = keychain::get_secret(&keychain::endpoint_key_name(&ep.id)) {
            ep.api_key = key;
        }
    }

    Ok(endpoints)
}

#[tauri::command]
pub async fn save_custom_endpoints(
    endpoints: Vec<CustomEndpoint>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    // Save api_key to keychain, strip from JSON file
    for ep in &endpoints {
        if !ep.api_key.is_empty() {
            keychain::set_secret(&keychain::endpoint_key_name(&ep.id), &ep.api_key)?;
        } else {
            let _ = keychain::delete_secret(&keychain::endpoint_key_name(&ep.id));
        }
    }

    let mut stripped = endpoints;
    for ep in &mut stripped {
        normalize_custom_endpoint_config(ep);
        ep.api_key = String::new();
    }

    let path = custom_endpoints_path(&app_handle)?;
    let json = serde_json::to_string_pretty(&stripped)
        .map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save custom endpoints: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn test_custom_endpoint(endpoint: CustomEndpoint) -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(30))
        .gzip(true)
        .deflate(true)
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    match endpoint.api_format {
        ApiFormat::OpenaiChat => {
            let url = format!(
                "{}/chat/completions",
                endpoint.endpoint.trim_end_matches('/')
            );
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "messages": [{"role": "user", "content": "Hi"}],
                "max_tokens": 16,
                "stream": false,
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !endpoint.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                    return Ok(content.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
        ApiFormat::OpenaiResponses => {
            let url = format!("{}/responses", endpoint.endpoint.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "input": "Hi",
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            if !endpoint.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                // Responses API: output[].content[].text
                if let Some(output) = json["output"].as_array() {
                    for item in output {
                        if let Some(content) = item["content"].as_array() {
                            for block in content {
                                if let Some(t) = block["text"].as_str() {
                                    return Ok(t.to_string());
                                }
                            }
                        }
                        if let Some(t) = item["text"].as_str() {
                            return Ok(t.to_string());
                        }
                    }
                }
                if let Some(t) = json["output_text"].as_str() {
                    return Ok(t.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
        ApiFormat::AnthropicMessages => {
            let url = format!("{}/messages", endpoint.endpoint.trim_end_matches('/'));
            let body = serde_json::json!({
                "model": endpoint.api_model,
                "messages": [{"role": "user", "content": "Hi"}],
                "max_tokens": 16,
            });
            let mut req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("anthropic-version", "2023-06-01");
            if !endpoint.beta_flags.is_empty() {
                req = req.header("anthropic-beta", endpoint.beta_flags.join(","));
            }
            if !endpoint.api_key.is_empty() {
                req = req
                    .header("x-api-key", &endpoint.api_key)
                    .header("Authorization", format!("Bearer {}", endpoint.api_key));
            }
            let resp = req
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("Request failed: {}", e))?;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                if let Some(msg) = maybe_html_fallback(&text) {
                    return Err(endpoint_html_response_error(msg, Some(status)));
                }
                return Err(
                    format!("HTTP {} — {}", status.as_u16(), truncate_str(&text, 200)).into(),
                );
            }
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(content) = json["content"][0]["text"].as_str() {
                    return Ok(content.to_string());
                }
            }
            if let Some(msg) = maybe_html_fallback(&text) {
                return Err(endpoint_html_response_error(msg, None));
            }
            Ok(truncate_str(&text, 120).to_string())
        }
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

/// If the response body looks like HTML (e.g. a CDN challenge page),
/// save it to a temp file and return a message with `[OPEN_HTML:filepath]` marker.
fn maybe_html_fallback(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let head = trimmed.chars().take(32).collect::<String>().to_ascii_lowercase();
    if head.starts_with("<!") || head.starts_with("<html") {
        let tmp =
            std::env::temp_dir().join(format!("locus_endpoint_test_{}.html", std::process::id()));
        if std::fs::write(&tmp, text).is_ok() {
            Some(format!(
                "Server returned an HTML page instead of JSON (possible verification/challenge page). [OPEN_HTML:{}]",
                tmp.display()
            ))
        } else {
            Some("Server returned an HTML page instead of JSON.".to_string())
        }
    } else {
        None
    }
}

fn endpoint_html_response_error(
    message: String,
    status: Option<reqwest::StatusCode>,
) -> AppError {
    let message = match status {
        Some(status) => format!("HTTP {} — {}", status.as_u16(), message),
        None => message,
    };
    AppError::new(ENDPOINT_TEST_HTML_RESPONSE_CODE, message)
}

#[tauri::command]
pub async fn get_debug_mode(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.debug_enabled())
}

#[tauri::command]
pub async fn set_debug_mode(
    value: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config.set_debug_enabled(value).map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn get_tool_permission_mode(
    mode: State<'_, crate::ToolPermissionMode>,
) -> Result<String, AppError> {
    Ok(mode.read().await.clone())
}

fn normalize_tool_permission_mode_request(value: Option<&str>, mode: Option<&str>) -> &'static str {
    let requested = value.or(mode).unwrap_or_default().trim();
    if requested.eq_ignore_ascii_case("ask") {
        "ask"
    } else {
        "auto"
    }
}

#[tauri::command]
pub async fn save_tool_permission_mode(
    value: Option<String>,
    mode: Option<String>,
    mode_state: State<'_, crate::ToolPermissionMode>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    // Accept both `value` and the legacy `mode` argument to keep older frontends working.
    let normalized =
        normalize_tool_permission_mode_request(value.as_deref(), mode.as_deref()).to_string();
    *mode_state.write().await = normalized.clone();
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let path = data_dir.join("tool_permission_mode.txt");
    std::fs::write(&path, &normalized)
        .map_err(|e| format!("Failed to save tool permission mode: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn get_tool_permissions(
    perms: State<'_, crate::ToolPermissions>,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    Ok(perms.read().await.clone())
}

#[tauri::command]
pub async fn save_tool_permissions(
    value: std::collections::HashMap<String, String>,
    perms: State<'_, crate::ToolPermissions>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let normalized: std::collections::HashMap<String, String> = value
        .into_iter()
        .map(|(k, v)| {
            let mode = normalize_tool_permission_mode_request(Some(v.as_str()), None).to_string();
            (k, mode)
        })
        .collect();
    *perms.write().await = normalized.clone();
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;
    let path = data_dir.join("tool_permissions.json");
    let json = serde_json::to_string_pretty(&normalized)
        .map_err(|e| format!("Failed to serialize tool permissions: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save tool permissions: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub rel_path: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchEntry {
    pub rel_path: String,
    pub name: String,
    pub parent_path: String,
    pub is_dir: bool,
    pub match_score: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntriesPage {
    pub entries: Vec<DirEntry>,
    pub total_count: usize,
    pub next_offset: usize,
    pub has_more: bool,
}

#[derive(Default)]
struct DirEntriesPageCacheInner {
    order: VecDeque<String>,
    listings: HashMap<String, Arc<[DirEntry]>>,
}

#[derive(Clone, Default)]
pub struct DirEntriesPageCache(Arc<Mutex<DirEntriesPageCacheInner>>);

impl DirEntriesPageCache {
    const MAX_ENTRIES: usize = 24;

    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(DirEntriesPageCacheInner::default())))
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.0.lock() {
            guard.order.clear();
            guard.listings.clear();
        }
    }

    fn get(&self, key: &str) -> Option<Arc<[DirEntry]>> {
        let mut guard = self.0.lock().ok()?;
        let listing = guard.listings.get(key).cloned()?;
        if let Some(index) = guard.order.iter().position(|existing| existing == key) {
            guard.order.remove(index);
        }
        guard.order.push_back(key.to_string());
        Some(listing)
    }

    fn insert(&self, key: String, entries: Vec<DirEntry>) -> Arc<[DirEntry]> {
        let listing: Arc<[DirEntry]> = Arc::from(entries.into_boxed_slice());
        if let Ok(mut guard) = self.0.lock() {
            if let Some(index) = guard.order.iter().position(|existing| existing == &key) {
                guard.order.remove(index);
            }
            guard.order.push_back(key.clone());
            guard.listings.insert(key, listing.clone());

            while guard.order.len() > Self::MAX_ENTRIES {
                if let Some(stale_key) = guard.order.pop_front() {
                    guard.listings.remove(&stale_key);
                }
            }
        }
        listing
    }
}

const WORKSPACE_HIDDEN_DIRS: &[&str] = &[
    ".git",
    ".vs",
    ".vscode",
    ".idea",
    "node_modules",
    "__pycache__",
    ".next",
    "dist",
    "build",
    "Library",
    "Temp",
    "Logs",
    "obj",
];

const ASSET_ROOT_DIRS: &[&str] = &["Assets", "Packages", "ProjectSettings"];

fn resolve_workspace_dir_target(
    cwd: &str,
    sub_path: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf), AppError> {
    let base = std::path::Path::new(cwd);
    let target = if sub_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(sub_path)
    };

    if !target.is_dir() {
        return Ok((target, std::path::PathBuf::new()));
    }

    let canonical_base = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let canonical_target = dunce::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if !canonical_target.starts_with(&canonical_base) {
        return Err("Path is not within the working directory"
            .to_string()
            .into());
    }

    Ok((target, canonical_target))
}

fn should_skip_workspace_entry(file_name: &str, is_dir: bool, exclude_meta: bool) -> bool {
    if file_name.starts_with('.') {
        return true;
    }

    if exclude_meta && file_name.ends_with(".meta") {
        return true;
    }

    is_dir && WORKSPACE_HIDDEN_DIRS.contains(&file_name)
}

fn join_workspace_rel_path(sub_path: &str, file_name: &str) -> String {
    if sub_path.is_empty() {
        file_name.to_string()
    } else {
        format!("{}/{}", sub_path.trim_end_matches('/'), file_name)
    }
}

fn collect_dir_entries(
    target: &std::path::Path,
    sub_path: &str,
    exclude_meta: bool,
) -> Result<Vec<DirEntry>, AppError> {
    let mut entries: Vec<DirEntry> = Vec::new();
    let read_dir =
        std::fs::read_dir(target).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if should_skip_workspace_entry(&file_name, is_dir, exclude_meta) {
            continue;
        }

        entries.push(DirEntry {
            rel_path: join_workspace_rel_path(sub_path, &file_name),
            name: file_name,
            is_dir,
        });
    }

    entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));

    Ok(entries)
}

fn workspace_search_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn compact_workspace_search(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn workspace_search_score(query: &str, name: &str, rel_path: &str, is_dir: bool) -> Option<i32> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let query_lower = trimmed.to_ascii_lowercase();
    let name_lower = name.to_ascii_lowercase();
    let rel_lower = rel_path.to_ascii_lowercase();
    let query_tokens = workspace_search_tokens(&query_lower);
    if !query_tokens.is_empty()
        && query_tokens
            .iter()
            .any(|token| !name_lower.contains(token) && !rel_lower.contains(token))
    {
        return None;
    }

    let compact_query = compact_workspace_search(&query_lower);
    let compact_name = compact_workspace_search(&name_lower);
    let compact_rel = compact_workspace_search(&rel_lower);

    let mut score = if name_lower == query_lower {
        1240
    } else if rel_lower == query_lower {
        1200
    } else if name_lower.starts_with(&query_lower) {
        1140 - name_lower.len().min(48) as i32
    } else if rel_lower.starts_with(&query_lower) {
        1080 - rel_lower.len().min(72) as i32
    } else if let Some(index) = name_lower.find(&query_lower) {
        1020 - index as i32 * 8
    } else if let Some(index) = rel_lower.find(&query_lower) {
        960 - index as i32 * 5
    } else if !compact_query.is_empty() && compact_name.starts_with(&compact_query) {
        920 - compact_name.len().min(48) as i32
    } else if !compact_query.is_empty() && compact_rel.contains(&compact_query) {
        let index = compact_rel.find(&compact_query).unwrap_or(0) as i32;
        860 - index * 4
    } else {
        return None;
    };

    score -= rel_path.matches('/').count() as i32 * 3;
    if is_dir {
        score += 12;
    }
    Some(score)
}

fn build_workspace_search_entry(
    rel_path: String,
    name: String,
    is_dir: bool,
    match_score: i32,
) -> WorkspaceSearchEntry {
    let parent_path = rel_path
        .rsplit_once('/')
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default();
    WorkspaceSearchEntry {
        rel_path,
        name,
        parent_path,
        is_dir,
        match_score,
    }
}

fn collect_workspace_search_entries(
    root_dir: &std::path::Path,
    root_rel_path: &str,
    include_files: bool,
    query: &str,
    results: &mut Vec<WorkspaceSearchEntry>,
) -> Result<(), AppError> {
    let mut stack = vec![(root_dir.to_path_buf(), root_rel_path.to_string())];

    while let Some((dir_path, dir_rel_path)) = stack.pop() {
        let read_dir =
            std::fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?;
        let mut child_dirs: Vec<(std::path::PathBuf, String)> = Vec::new();

        for entry in read_dir.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if should_skip_workspace_entry(&file_name, is_dir, false) {
                continue;
            }

            let rel_path = join_workspace_rel_path(&dir_rel_path, &file_name);
            if !is_dir && !include_files {
                continue;
            }
            if let Some(match_score) = workspace_search_score(query, &file_name, &rel_path, is_dir)
            {
                results.push(build_workspace_search_entry(
                    rel_path.clone(),
                    file_name.clone(),
                    is_dir,
                    match_score,
                ));
            }

            if is_dir {
                child_dirs.push((entry.path(), rel_path));
            }
        }

        child_dirs.sort_by(|left, right| right.1.cmp(&left.1));
        stack.extend(child_dirs);
    }

    Ok(())
}

fn search_workspace_entries_in_dir(
    workspace_root: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    if !workspace_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let read_dir = std::fs::read_dir(workspace_root)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if should_skip_workspace_entry(&file_name, is_dir, false) {
            continue;
        }

        if let Some(match_score) = workspace_search_score(query, &file_name, &file_name, is_dir) {
            results.push(build_workspace_search_entry(
                file_name.clone(),
                file_name.clone(),
                is_dir,
                match_score,
            ));
        }

        if !is_dir {
            continue;
        }

        let include_files = !ASSET_ROOT_DIRS.contains(&file_name.as_str());
        collect_workspace_search_entries(
            &entry.path(),
            &file_name,
            include_files,
            query,
            &mut results,
        )?;
    }

    results.sort_by(|left, right| {
        right
            .match_score
            .cmp(&left.match_score)
            .then_with(|| right.is_dir.cmp(&left.is_dir))
            .then_with(|| left.rel_path.len().cmp(&right.rel_path.len()))
            .then_with(|| left.rel_path.cmp(&right.rel_path))
    });

    if results.len() > limit {
        results.truncate(limit);
    }

    Ok(results)
}

#[tauri::command]
pub async fn list_dir_entries(
    sub_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<DirEntry>, AppError> {
    let cwd = workspace.path.read().await.clone();
    let (target, _canonical_target) = resolve_workspace_dir_target(&cwd, &sub_path)?;
    if !target.is_dir() {
        return Ok(vec![]);
    }

    collect_dir_entries(&target, &sub_path, false)
}

#[tauri::command]
pub async fn list_dir_entries_page(
    sub_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    exclude_meta: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
) -> Result<DirEntriesPage, AppError> {
    let cwd = workspace.path.read().await.clone();
    let (target, canonical_target) = resolve_workspace_dir_target(&cwd, &sub_path)?;
    if !target.is_dir() {
        return Ok(DirEntriesPage {
            entries: Vec::new(),
            total_count: 0,
            next_offset: 0,
            has_more: false,
        });
    }

    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(200).clamp(1, 2_000);
    let exclude_meta = exclude_meta.unwrap_or(false);
    let cache_key = format!("{}::{}", canonical_target.display(), u8::from(exclude_meta));

    let listing = if offset == 0 {
        let entries = collect_dir_entries(&target, &sub_path, exclude_meta)?;
        dir_entries_cache.insert(cache_key.clone(), entries)
    } else if let Some(cached) = dir_entries_cache.get(&cache_key) {
        cached
    } else {
        let entries = collect_dir_entries(&target, &sub_path, exclude_meta)?;
        dir_entries_cache.insert(cache_key.clone(), entries)
    };

    let total_count = listing.len();
    let start = offset.min(total_count);
    let end = (start + limit).min(total_count);

    Ok(DirEntriesPage {
        entries: listing[start..end].to_vec(),
        total_count,
        next_offset: end,
        has_more: end < total_count,
    })
}

#[tauri::command]
pub async fn search_workspace_entries(
    query: String,
    limit: Option<usize>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.unwrap_or(200).clamp(1, 500);
    search_workspace_entries_in_dir(std::path::Path::new(&cwd), trimmed, limit)
}

#[tauri::command]
pub async fn check_unity_connection(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<bool, AppError> {
    let cwd = workspace.path.read().await.clone();
    Ok(crate::unity_bridge::is_unity_connected(&cwd).await)
}

#[tauri::command]
pub async fn check_unity_plugin(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<crate::unity_bridge::PluginStatus, AppError> {
    let cwd = workspace.path.read().await.clone();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(crate::unity_bridge::PluginStatus::UpToDate);
    }
    crate::unity_bridge::check_plugin_status(&cwd).map_err(Into::into)
}

#[tauri::command]
pub async fn install_unity_plugin(
    workspace: State<'_, Arc<Workspace>>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Err("Current working directory is not a Unity project"
            .to_string()
            .into());
    }
    let hash = crate::unity_bridge::install_or_update_plugin(&cwd)?;
    crate::unity_bridge::emit_plugin_status(&app_handle, &cwd);
    Ok(hash)
}

#[tauri::command]
pub async fn launch_unity_project(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<crate::unity_bridge::UnityLaunchResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::launch_project(&cwd).map_err(Into::into)
}

#[tauri::command]
pub async fn send_unity_log(
    message: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    let resp = crate::unity_bridge::send_message(&cwd, "log", &message).await?;
    if resp.ok {
        Ok(format!("Unity log sent: {}", message))
    } else {
        Err(resp
            .error
            .unwrap_or_else(|| "unknown error".to_string())
            .into())
    }
}

#[tauri::command]
pub async fn select_unity_asset(
    asset_path: String,
    focus_project_window: Option<bool>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::select_asset(&cwd, &asset_path, focus_project_window.unwrap_or(true))
        .await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_asset_inspector(
    asset_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::open_asset_inspector(&cwd, &asset_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn select_unity_scene_object(
    scene_path: String,
    object_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::select_scene_object(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_scene_object_inspector(
    scene_path: String,
    object_path: String,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    crate::unity_bridge::open_scene_object_inspector(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn reset_all_config(
    workspace: State<'_, Arc<Workspace>>,
    unity_monitor: State<'_, UnityMonitorHandle>,
    ref_graph_state: State<'_, AssetDbState>,
    watcher_handle: State<'_, AssetDbWatcherHandle>,
    last_scan_info: State<'_, LastScanInfoState>,
    scan_phase_state: State<'_, ScanPhaseState>,
    preview_cache: State<'_, WorkspacePreviewCache>,
    dir_entries_cache: State<'_, DirEntriesPageCache>,
    knowledge_index_state: State<'_, Arc<crate::knowledge_index::KnowledgeIndexState>>,
    mode: State<'_, crate::ToolPermissionMode>,
    perms: State<'_, crate::ToolPermissions>,
    api_key_state: State<'_, crate::ApiKeyState>,
    provider_keys: State<'_, crate::ProviderKeysState>,
    auth: State<'_, Arc<tokio::sync::Mutex<crate::auth::AuthState>>>,
    codex: State<'_, crate::commands::auth::CodexAuthStateHandle>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get data dir: {}", e))?;

    // Clear keychain secrets: OpenRouter key
    let _ = keychain::delete_secret(keychain::KEY_OPENROUTER);

    // Clear keychain secrets: all provider keys
    {
        let keys = provider_keys.read().await;
        for id in keys.keys() {
            let _ = keychain::delete_secret(&keychain::provider_key_name(id));
        }
    }

    // Clear keychain secrets: custom endpoint API keys
    let ep_path = custom_endpoints_path(&app_handle)
        .unwrap_or_else(|_| data_dir.join("custom_endpoints.json"));
    if let Ok(content) = std::fs::read_to_string(&ep_path) {
        if let Ok(endpoints) = serde_json::from_str::<Vec<CustomEndpoint>>(&content) {
            for ep in &endpoints {
                let _ = keychain::delete_secret(&keychain::endpoint_key_name(&ep.id));
            }
        }
    }

    // OAuth/Codex tokens are cleared by .logout() which now uses keychain

    let config_files = [
        "provider_key_ids.json",
        "working_dir.txt",
        "recent_dirs.json",
        "active_session_selection.json",
        "tool_permission_mode.txt",
        "tool_permissions.json",
        "git_path_override.txt",
        "config.json",
    ];

    for file in &config_files {
        let path = data_dir.join(file);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }

    // Also clear the stable config dir.
    if let Ok(pdir) = persistent_config_dir() {
        for file in [
            "config.json",
            "last_model.txt",
            "last_effort.txt",
            "model_defaults.json",
            "custom_endpoints.json",
            "codex_model_config.json",
            crate::python_runtime::config_file_name(),
        ] {
            let path = pdir.join(file);
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    if let Some(webview) = app_handle.webview_windows().values().next() {
        let _ = webview.clear_all_browsing_data();
    }

    {
        let mut wh = watcher_handle
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(old) = wh.take() {
            old.stop_and_join();
            eprintln!("[Locus] stopped ref_graph watcher during reset");
        }
    }
    {
        *ref_graph_state
            .0
            .lock()
            .map_err(|e| format!("Lock error: {}", e))? = None;
    }
    last_scan_info.clear();
    scan_phase_state.clear();
    preview_cache.clear();
    dir_entries_cache.clear();

    crate::unity_bridge::stop_unity_monitor(&unity_monitor).await;
    let _ = app_handle.emit("unity-connection-status", false);

    *workspace.path.write().await = String::new();
    *workspace.workspace_id.write().await = None;
    super::reset_unity_embed_control_window(&app_handle);
    super::refresh_unity_embed_control_server(app_handle.clone());
    let no_workspace_library_dir = crate::knowledge_index::no_workspace_library_dir();
    knowledge_index_state
        .rebuild(&no_workspace_library_dir, &data_dir)
        .await?;
    *mode.write().await = "auto".to_string();
    *perms.write().await = std::collections::HashMap::new();
    *api_key_state.write().await = String::new();
    *provider_keys.write().await = std::collections::HashMap::new();
    auth.lock().await.logout();
    codex.lock().await.logout();

    eprintln!("[Locus] All config reset (keychain + config files + runtime state + WebView browsing data)");
    Ok(())
}

// ── Config registry ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_config_registry(
    category: Option<String>,
    app_handle: AppHandle,
) -> Result<Vec<crate::config_registry::ConfigEntry>, AppError> {
    match category.as_deref() {
        Some(cat) => crate::config_registry::collect_by_category(&app_handle, cat),
        None => crate::config_registry::collect_all(&app_handle),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_tool_permission_mode_request, search_workspace_entries_in_dir,
        workspace_search_score,
    };
    use tempfile::tempdir;

    #[test]
    fn normalize_tool_permission_mode_accepts_primary_value_arg() {
        assert_eq!(
            normalize_tool_permission_mode_request(Some("ask"), Some("auto")),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(Some("auto"), Some("ask")),
            "auto"
        );
    }

    #[test]
    fn normalize_tool_permission_mode_accepts_legacy_mode_arg() {
        assert_eq!(
            normalize_tool_permission_mode_request(None, Some("ask")),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(None, Some("auto")),
            "auto"
        );
        assert_eq!(normalize_tool_permission_mode_request(None, None), "auto");
    }

    #[test]
    fn normalize_tool_permission_mode_trims_and_normalizes_case() {
        assert_eq!(
            normalize_tool_permission_mode_request(Some(" Ask "), None),
            "ask"
        );
        assert_eq!(
            normalize_tool_permission_mode_request(Some(" AUTO "), None),
            "auto"
        );
    }

    #[test]
    fn workspace_search_score_matches_compact_path_queries() {
        let score = workspace_search_score(
            "UIElementsSchema/UnityEditor.Overlays",
            "UnityEditor.Overlays.xsd",
            "UIElementsSchema/UnityEditor.Overlays.xsd",
            false,
        );

        assert!(score.is_some());
    }

    #[test]
    fn search_workspace_entries_in_dir_returns_generic_files_and_directories() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("UIElementsSchema"))
            .expect("create workspace folder");
        std::fs::write(
            temp.path()
                .join("UIElementsSchema/UnityEditor.Overlays.xsd"),
            "schema",
        )
        .expect("write workspace file");
        std::fs::create_dir_all(temp.path().join("Assets/Scripts/UI")).expect("create assets dir");
        std::fs::write(temp.path().join("Assets/Scripts/UI/Hud.prefab"), "prefab")
            .expect("write asset file");

        let generic_results =
            search_workspace_entries_in_dir(temp.path(), "UnityEditor.Overlays", 100)
                .expect("search generic workspace");
        assert!(generic_results.iter().any(|entry| {
            entry.rel_path == "UIElementsSchema/UnityEditor.Overlays.xsd" && !entry.is_dir
        }));

        let folder_results = search_workspace_entries_in_dir(temp.path(), "Scripts", 100)
            .expect("search workspace folders");
        assert!(folder_results
            .iter()
            .any(|entry| { entry.rel_path == "Assets/Scripts" && entry.is_dir }));

        assert!(!folder_results
            .iter()
            .any(|entry| { entry.rel_path == "Assets/Scripts/UI/Hud.prefab" }));
    }
}
