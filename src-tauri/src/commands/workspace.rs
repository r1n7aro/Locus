use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::error::AppError;
use crate::keychain;
use crate::workspace_service::service::{
    ResolvedServiceBinding, ServiceBindingError, ServiceKind, ServiceReadinessPhase,
    ServiceReadinessSnapshot, ServiceStatus, WorkspaceServiceStateSnapshot,
};
use crate::workspace_service::{AgentExecutionContext, ProjectRegistry, WorkspaceRef};

const ENDPOINT_TEST_HTML_RESPONSE_CODE: &str = "endpoint_test.html_response";
pub const SESSION_UNDO_ENABLED_CHANGED_EVENT: &str = "session-undo-enabled-changed";

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionUndoEnabledChangedEvent {
    enabled: bool,
}

/// Returns a stable app config directory inside the OS config root.
/// On Windows this resolves under `%APPDATA%\\locus`, which keeps model config
/// under the app-data tree while staying outside Tauri's bundle-specific
/// `app_data_dir` that may be cleared during reinstall.
pub(crate) fn persistent_config_dir() -> Result<std::path::PathBuf, String> {
    if let Some(dir) = crate::runtime_paths::runtime_config_dir_from_env()? {
        return Ok(dir);
    }
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Failed to get config directory".to_string())?;
    let dir = config_dir.join("locus");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create persistent config dir: {}", e))?;
    Ok(dir)
}

fn app_temp_dir_override() -> &'static Mutex<Option<std::path::PathBuf>> {
    static OVERRIDE: OnceLock<Mutex<Option<std::path::PathBuf>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn set_app_temp_dir_override(
    dir: std::path::PathBuf,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
    let dir = dunce::canonicalize(&dir).unwrap_or(dir);
    let mut guard = app_temp_dir_override()
        .lock()
        .map_err(|e| format!("Failed to lock app temp directory override: {}", e))?;
    *guard = Some(dir.clone());
    Ok(dir)
}

pub(crate) fn app_temp_dir() -> Result<std::path::PathBuf, String> {
    if let Some(dir) = app_temp_dir_override()
        .lock()
        .map_err(|e| format!("Failed to lock app temp directory override: {}", e))?
        .clone()
    {
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
        return Ok(dir);
    }

    let dir = persistent_config_dir()?.join("temp");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create app temp directory: {}", e))?;
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

const MAX_RECENT_DIRS: usize = 8;

pub fn save_recent_dir_pub(data_dir: &std::path::Path, dir: &str) {
    save_recent_dir(data_dir, dir);
}

pub fn remove_recent_dirs_pub(
    data_dir: &std::path::Path,
    removed_paths: &[String],
) -> Result<Vec<String>, AppError> {
    let removed_paths = removed_paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .collect::<HashSet<_>>();
    let mut dirs = read_recent_dirs(data_dir);
    dirs.retain(|dir| !removed_paths.contains(dir.trim()));
    write_recent_dirs(data_dir, &dirs)?;
    Ok(existing_recent_dirs(dirs))
}

fn recent_dirs_file(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("recent_dirs.json")
}

fn read_recent_dirs(data_dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(recent_dirs_file(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_recent_dirs(data_dir: &std::path::Path, dirs: &[String]) -> Result<(), AppError> {
    let file = recent_dirs_file(data_dir);
    let text = serde_json::to_string(dirs)
        .map_err(|e| AppError::new("workspace.recent_dirs_serialize_failed", e.to_string()))?;
    std::fs::write(&file, text).map_err(|e| {
        AppError::new(
            "workspace.recent_dirs_write_failed",
            format!("Failed to save recent directories: {}", e),
        )
    })
}

fn existing_recent_dirs(dirs: Vec<String>) -> Vec<String> {
    dirs.into_iter()
        .filter(|d| std::path::Path::new(d).is_dir())
        .collect()
}

fn save_recent_dir(data_dir: &std::path::Path, dir: &str) {
    let mut dirs = read_recent_dirs(data_dir);

    dirs.retain(|d| d != dir);
    dirs.insert(0, dir.to_string());
    dirs.truncate(MAX_RECENT_DIRS);

    let _ = write_recent_dirs(data_dir, &dirs);
}

#[tauri::command]
pub async fn list_recent_dirs(app_handle: AppHandle) -> Result<Vec<String>, AppError> {
    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    Ok(existing_recent_dirs(read_recent_dirs(&data_dir)))
}

#[tauri::command]
pub async fn remove_recent_dir(
    path: String,
    app_handle: AppHandle,
) -> Result<Vec<String>, AppError> {
    let target = path.trim();
    if target.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let data_dir = super::resolve_runtime_storage_dir(&app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    let mut dirs = read_recent_dirs(&data_dir);
    dirs.retain(|d| d != target);
    write_recent_dirs(&data_dir, &dirs)?;
    Ok(existing_recent_dirs(dirs))
}

#[tauri::command]
pub async fn open_dir_in_file_explorer(path: String) -> Result<(), AppError> {
    let target = path.trim();
    if target.is_empty() {
        return Err("Path cannot be empty".to_string().into());
    }

    let path = std::path::Path::new(target);
    if !path.is_dir() {
        return Err(format!("Directory not found: {}", target).into());
    }

    let canonical =
        dunce::canonicalize(path).map_err(|e| format!("Failed to resolve path: {}", e))?;
    crate::commands::knowledge::reveal_path_native(&canonical).map_err(Into::into)
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

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentModelPreference {
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub effort: String,
}

fn agent_model_preferences_path() -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("agent_model_preferences.json"))
}

fn load_agent_model_preferences() -> HashMap<String, AgentModelPreference> {
    let Ok(path) = agent_model_preferences_path() else {
        return HashMap::new();
    };
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<HashMap<String, AgentModelPreference>>(&content) else {
        return HashMap::new();
    };

    let mut normalized = HashMap::new();
    for (agent_id, preference) in raw.iter().filter(|(agent_id, _)| {
        crate::agent::definition::canonical_agent_id(agent_id) != agent_id.as_str()
    }) {
        normalized.insert(
            crate::agent::definition::canonical_agent_id(agent_id).to_string(),
            preference.clone(),
        );
    }
    for (agent_id, preference) in raw.iter().filter(|(agent_id, _)| {
        crate::agent::definition::canonical_agent_id(agent_id) == agent_id.as_str()
    }) {
        normalized.insert(agent_id.clone(), preference.clone());
    }
    normalized
}

#[tauri::command]
pub async fn get_agent_model_preferences(
    _app_handle: AppHandle,
) -> Result<HashMap<String, AgentModelPreference>, AppError> {
    Ok(load_agent_model_preferences())
}

#[tauri::command]
pub async fn save_agent_model_preference(
    agent_id: String,
    model_id: String,
    effort: String,
    _app_handle: AppHandle,
) -> Result<(), AppError> {
    let agent_id = crate::agent::definition::canonical_agent_id(agent_id.trim());
    if agent_id.is_empty() {
        return Err("Agent id cannot be empty".to_string().into());
    }
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err("Model id cannot be empty".to_string().into());
    }
    let effort = effort.trim();
    if !matches!(effort, "none" | "low" | "medium" | "high" | "xhigh" | "max") {
        return Err(format!("Unsupported reasoning effort: {}", effort).into());
    }

    let mut preferences = load_agent_model_preferences();
    preferences.insert(
        agent_id.to_string(),
        AgentModelPreference {
            model_id: model_id.to_string(),
            effort: effort.to_string(),
        },
    );
    let json = serde_json::to_string_pretty(&preferences)
        .map_err(|error| format!("Failed to serialize Agent model preferences: {}", error))?;
    std::fs::write(agent_model_preferences_path()?, json)
        .map_err(|error| format!("Failed to save Agent model preferences: {}", error))?;
    Ok(())
}

#[tauri::command]
pub async fn get_codex_fast_mode(_app_handle: AppHandle) -> Result<bool, AppError> {
    let path = persistent_config_dir()?.join("codex_fast_mode.txt");
    Ok(read_nonempty_string(&path).is_some_and(|value| value.eq_ignore_ascii_case("true")))
}

#[tauri::command]
pub async fn save_codex_fast_mode(enabled: bool, _app_handle: AppHandle) -> Result<(), AppError> {
    let dir = persistent_config_dir().map_err(|e| format!("Failed to get config dir: {e}"))?;
    std::fs::write(dir.join("codex_fast_mode.txt"), enabled.to_string())
        .map_err(|e| format!("Failed to save Codex Fast mode: {e}"))?;
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
    #[serde(default)]
    pub subagent_efforts: std::collections::HashMap<String, String>,
    /// Missing keys inherit the current session; explicit false selects the
    /// standard Codex service tier for that sub-agent.
    #[serde(default)]
    pub subagent_fast_modes: std::collections::HashMap<String, bool>,
    /// Opt-in flag: Claude Code CLI models only join the model list after the
    /// user explicitly enables them in model configuration.
    #[serde(default)]
    pub claude_code_enabled: bool,
}

impl Default for ModelDefaults {
    fn default() -> Self {
        ModelDefaults {
            main_model: String::new(),
            plan_model: String::new(),
            subagent_models: std::collections::HashMap::new(),
            subagent_efforts: std::collections::HashMap::new(),
            subagent_fast_modes: std::collections::HashMap::new(),
            claude_code_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexTransportMode {
    Http,
    Websocket,
}

pub const DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS: u32 = 5 * 60;
pub const DEFAULT_CODEX_PREFIX_CACHE_TTL_SECONDS: u32 = 30 * 60;
pub const DEFAULT_CODEX_CONTEXT_WINDOW: u32 = 272_000;
pub const LEGACY_CODEX_EXTENDED_CONTEXT_WINDOW: u32 = 372_000;
pub const MIN_CODEX_CONTEXT_WINDOW: u32 = 16_000;
pub const MAX_CODEX_CONTEXT_WINDOW: u32 = 1_000_000;

fn default_provider_prefix_cache_ttl_seconds() -> u32 {
    DEFAULT_PROVIDER_PREFIX_CACHE_TTL_SECONDS
}

fn default_codex_prefix_cache_ttl_seconds() -> u32 {
    DEFAULT_CODEX_PREFIX_CACHE_TTL_SECONDS
}

impl Default for CodexTransportMode {
    fn default() -> Self {
        Self::Websocket
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelConfig {
    #[serde(default)]
    pub transport: CodexTransportMode,
    /// Raw GPT-5.6 context window requested by Locus. Missing values retain
    /// the standard 272K default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    /// Legacy 272K/372K switch. New saves migrate this into `context_window`.
    #[serde(default)]
    pub extended_context: bool,
    /// Generate a concise title for new chat sessions with Codex OAuth.
    #[serde(default)]
    pub generate_session_titles: bool,
    /// Route approval requests through the Codex auto-review model.
    #[serde(default)]
    pub auto_review: bool,
    /// How long Locus keeps a Codex session's composed prompt prefix stable
    /// after the most recent successful remote response.
    #[serde(default = "default_codex_prefix_cache_ttl_seconds")]
    pub prefix_cache_ttl_seconds: u32,
}

impl Default for CodexModelConfig {
    fn default() -> Self {
        Self {
            transport: CodexTransportMode::default(),
            context_window: None,
            extended_context: false,
            generate_session_titles: false,
            auto_review: false,
            prefix_cache_ttl_seconds: default_codex_prefix_cache_ttl_seconds(),
        }
    }
}

impl CodexModelConfig {
    pub(crate) fn resolved_context_window(&self) -> u32 {
        self.context_window
            .map(|window| window.clamp(MIN_CODEX_CONTEXT_WINDOW, MAX_CODEX_CONTEXT_WINDOW))
            .unwrap_or_else(|| {
                if self.extended_context {
                    LEGACY_CODEX_EXTENDED_CONTEXT_WINDOW
                } else {
                    DEFAULT_CODEX_CONTEXT_WINDOW
                }
            })
    }

    fn normalized_for_save(mut self) -> Self {
        let context_window = self.resolved_context_window();
        self.context_window = Some(context_window);
        self.extended_context = false;
        self
    }
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
    let config = config.normalized_for_save();
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
    /// DashScope/Qwen style: `enable_thinking: true` in the chat body
    /// (required for hosted Qwen/QwQ to emit reasoning_content at all).
    OpenaiChatEnableThinking,
    /// Zhipu GLM style: `thinking: {"type": "enabled"|"disabled"}` in the
    /// chat body.
    OpenaiChatThinkingType,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct CustomEndpointServerTools {
    #[serde(default)]
    pub web_search: bool,
}

fn default_supports_tool_lazy_loading() -> bool {
    false
}

fn default_supports_vision() -> bool {
    true
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
    #[serde(default = "default_supported_reasoning_efforts")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning_param_format: Option<CustomReasoningParamFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_reasoning_content: Option<bool>,
    #[serde(default)]
    pub server_tools: CustomEndpointServerTools,
    #[serde(default = "default_supports_tool_lazy_loading")]
    pub supports_tool_lazy_loading: bool,
    #[serde(default = "default_supports_vision")]
    pub supports_vision: bool,
}

const DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH: u32 = 256_000;

fn default_context_length() -> u32 {
    DEFAULT_CUSTOM_ENDPOINT_CONTEXT_LENGTH
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

fn default_replay_reasoning_content(endpoint: &CustomEndpoint) -> bool {
    endpoint.api_format == ApiFormat::OpenaiChat
}

fn normalize_reasoning_effort(value: &str) -> Option<String> {
    let trimmed = value.trim().to_ascii_lowercase();
    match trimmed.as_str() {
        "low" | "medium" | "high" | "xhigh" | "max" => Some(trimmed),
        _ => None,
    }
}

fn is_stale_custom_model_ref(model_id: &str, valid_endpoint_ids: &HashSet<String>) -> bool {
    if let Some(endpoint_id) = model_id.trim().strip_prefix("custom/") {
        return !endpoint_id.is_empty() && !valid_endpoint_ids.contains(endpoint_id);
    }
    false
}

fn prune_stale_custom_model_refs(valid_endpoint_ids: &HashSet<String>) -> Result<(), String> {
    let dir = persistent_config_dir()?;
    let last_model_path = dir.join("last_model.txt");
    if let Some(last_model) = read_nonempty_string(&last_model_path) {
        if is_stale_custom_model_ref(&last_model, valid_endpoint_ids) {
            let _ = std::fs::remove_file(&last_model_path);
        }
    }

    let defaults_path = dir.join("model_defaults.json");
    let Some(mut defaults) = std::fs::read_to_string(&defaults_path)
        .ok()
        .and_then(|s| serde_json::from_str::<ModelDefaults>(&s).ok())
    else {
        return Ok(());
    };

    let mut changed = false;
    if is_stale_custom_model_ref(&defaults.main_model, valid_endpoint_ids) {
        defaults.main_model.clear();
        changed = true;
    }
    if is_stale_custom_model_ref(&defaults.plan_model, valid_endpoint_ids) {
        defaults.plan_model.clear();
        changed = true;
    }
    defaults.subagent_models.retain(|_, model_id| {
        let keep = !is_stale_custom_model_ref(model_id, valid_endpoint_ids);
        if !keep {
            changed = true;
        }
        keep
    });

    if changed {
        let json = serde_json::to_string_pretty(&defaults)
            .map_err(|e| format!("Failed to serialize model defaults: {}", e))?;
        std::fs::write(&defaults_path, json)
            .map_err(|e| format!("Failed to save model defaults: {}", e))?;
    }
    Ok(())
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
    if endpoint.context_length == 0 {
        endpoint.context_length = default_context_length();
    }
    if endpoint.reasoning_param_format.is_none() {
        endpoint.reasoning_param_format =
            Some(default_reasoning_param_format(&endpoint.api_format));
    }
    if endpoint.replay_reasoning_content.is_none() {
        endpoint.replay_reasoning_content = Some(default_replay_reasoning_content(endpoint));
    }
    endpoint.supports_tool_lazy_loading = false;
}

#[tauri::command]
pub async fn test_custom_endpoint(endpoint: CustomEndpoint) -> Result<String, AppError> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(30))
            .gzip(true)
            .deflate(true),
    )
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

// ===== Custom providers (v3: model-level remote compaction capability) =====
//
// Stored in custom_providers.json; api keys stay in the keychain under the
// legacy "endpoint/{provider_id}" name so migrated endpoints keep their
// secret without a keychain rewrite. Model ids are addressed as
// "custom/<provider_id>/<model_row_id>"; the legacy single-segment
// "custom/<endpoint_id>" keeps resolving to the provider's first model.

/// Which message-level field carries replayed reasoning text on OpenAI-chat
/// requests (models.dev `interleaved.field`).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningReplayField {
    ReasoningContent,
    ReasoningDetails,
    Reasoning,
}

/// Remote context-compaction protocol supported by one custom model route.
/// Kept as an enum so later protocols can be added without replacing the
/// persisted capability shape.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCompactionMode {
    #[default]
    Disabled,
    CodexV2,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProviderModel {
    /// Row id, unique within the provider, never contains '/'.
    pub id: String,
    pub api_model: String,
    pub name: String,
    #[serde(default = "default_context_length")]
    pub context_length: u32,
    #[serde(default)]
    pub remote_compaction_mode: RemoteCompactionMode,
    /// Protocol-native lazy tool loading (`defer_loading`/`tool_reference`)
    /// for Anthropic-format endpoints; the endpoint must support it.
    #[serde(default)]
    pub supports_tool_lazy_loading: bool,
    #[serde(default = "default_supported_reasoning_efforts")]
    pub supported_reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning_param_format: Option<CustomReasoningParamFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_reasoning_content: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_replay_field: Option<ReasoningReplayField>,
    #[serde(default)]
    pub server_tools: CustomEndpointServerTools,
    #[serde(default = "default_supports_vision")]
    pub supports_vision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_model_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub api_format: ApiFormat,
    /// Keychain-only; stripped before the JSON file is written.
    #[serde(default)]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    /// How long Locus keeps a session's composed prompt prefix stable after
    /// the provider's most recent successful response. Zero disables reuse.
    #[serde(default = "default_provider_prefix_cache_ttl_seconds")]
    pub prefix_cache_ttl_seconds: u32,
    #[serde(default)]
    pub models: Vec<CustomProviderModel>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct CustomProvidersFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    providers: Vec<CustomProvider>,
}

const CUSTOM_PROVIDERS_FILE_VERSION: u32 = 3;

pub(crate) fn custom_providers_path() -> Result<std::path::PathBuf, String> {
    Ok(persistent_config_dir()?.join("custom_providers.json"))
}

fn sanitize_id_segment(value: &str, fallback: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c.is_whitespace() {
                '-'
            } else {
                c
            }
        })
        .collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

pub(crate) fn model_row_id_from_api_model(api_model: &str) -> String {
    sanitize_id_segment(api_model, "model")
}

fn default_model_replay_reasoning_content(api_format: &ApiFormat) -> bool {
    *api_format == ApiFormat::OpenaiChat
}

fn is_deepseek_v4_model(model: &CustomProviderModel) -> bool {
    [
        model.api_model.as_str(),
        model.catalog_model_id.as_deref().unwrap_or(""),
    ]
    .iter()
    .any(|value| {
        value
            .trim()
            .to_ascii_lowercase()
            .starts_with("deepseek-v4-")
    })
}

pub(crate) fn normalize_custom_provider_config(provider: &mut CustomProvider) {
    provider.id = sanitize_id_segment(&provider.id, "provider");
    let api_format = provider.api_format.clone();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for model in &mut provider.models {
        if model.id.trim().is_empty() {
            model.id = model_row_id_from_api_model(&model.api_model);
        } else {
            model.id = sanitize_id_segment(&model.id, "model");
        }
        if seen_ids.contains(&model.id) {
            let base = model.id.clone();
            let mut n = 2usize;
            while seen_ids.contains(&format!("{base}-{n}")) {
                n += 1;
            }
            model.id = format!("{base}-{n}");
        }
        seen_ids.insert(model.id.clone());

        if model.name.trim().is_empty() {
            model.name = model.api_model.clone();
        }
        model.supported_reasoning_efforts = model
            .supported_reasoning_efforts
            .iter()
            .filter_map(|value| normalize_reasoning_effort(value))
            .collect();
        if model.supported_reasoning_efforts.is_empty() {
            model.supported_reasoning_efforts = default_supported_reasoning_efforts();
        }
        if model.context_length == 0 {
            model.context_length = default_context_length();
        }
        if model.reasoning_param_format.is_none() {
            model.reasoning_param_format = Some(default_reasoning_param_format(&api_format));
        }
        if is_deepseek_v4_model(model) {
            model.replay_reasoning_content = Some(true);
            if model.reasoning_replay_field.is_none() {
                model.reasoning_replay_field = Some(ReasoningReplayField::ReasoningContent);
            }
        } else if model.replay_reasoning_content.is_none() {
            model.replay_reasoning_content =
                Some(default_model_replay_reasoning_content(&api_format));
        }
    }
}

fn migrate_endpoint_to_provider(mut endpoint: CustomEndpoint) -> CustomProvider {
    normalize_custom_endpoint_config(&mut endpoint);
    let mut provider = CustomProvider {
        id: endpoint.id,
        name: endpoint.name,
        endpoint: endpoint.endpoint,
        api_format: endpoint.api_format,
        api_key: String::new(),
        catalog_id: None,
        prefix_cache_ttl_seconds: default_provider_prefix_cache_ttl_seconds(),
        models: vec![CustomProviderModel {
            id: model_row_id_from_api_model(&endpoint.api_model),
            name: endpoint.api_model.clone(),
            api_model: endpoint.api_model,
            context_length: endpoint.context_length,
            remote_compaction_mode: RemoteCompactionMode::Disabled,
            supports_tool_lazy_loading: endpoint.supports_tool_lazy_loading,
            supported_reasoning_efforts: endpoint.supported_reasoning_efforts,
            reasoning_param_format: endpoint.reasoning_param_format,
            replay_reasoning_content: endpoint.replay_reasoning_content,
            reasoning_replay_field: None,
            server_tools: endpoint.server_tools,
            supports_vision: endpoint.supports_vision,
            catalog_model_id: None,
        }],
    };
    normalize_custom_provider_config(&mut provider);
    provider
}

/// Rewrite a legacy `custom/<endpoint_id>` reference to the migrated
/// `custom/<provider_id>/<model_row_id>` form. Returns None when the value is
/// not a legacy custom reference or the provider is unknown.
fn rewrite_legacy_custom_model_ref(model_id: &str, providers: &[CustomProvider]) -> Option<String> {
    let endpoint_id = model_id.trim().strip_prefix("custom/")?;
    if endpoint_id.is_empty() || endpoint_id.contains('/') {
        return None;
    }
    let provider = providers.iter().find(|p| p.id == endpoint_id)?;
    let model = provider.models.first()?;
    Some(format!("custom/{}/{}", provider.id, model.id))
}

fn rewrite_legacy_custom_model_refs(providers: &[CustomProvider]) {
    let Ok(dir) = persistent_config_dir() else {
        return;
    };

    let last_model_path = dir.join("last_model.txt");
    if let Some(last_model) = read_nonempty_string(&last_model_path) {
        if let Some(rewritten) = rewrite_legacy_custom_model_ref(&last_model, providers) {
            let _ = std::fs::write(&last_model_path, rewritten);
        }
    }

    let defaults_path = dir.join("model_defaults.json");
    let Some(mut defaults) = std::fs::read_to_string(&defaults_path)
        .ok()
        .and_then(|s| serde_json::from_str::<ModelDefaults>(&s).ok())
    else {
        return;
    };
    let mut changed = false;
    if let Some(rewritten) = rewrite_legacy_custom_model_ref(&defaults.main_model, providers) {
        defaults.main_model = rewritten;
        changed = true;
    }
    if let Some(rewritten) = rewrite_legacy_custom_model_ref(&defaults.plan_model, providers) {
        defaults.plan_model = rewritten;
        changed = true;
    }
    for model_id in defaults.subagent_models.values_mut() {
        if let Some(rewritten) = rewrite_legacy_custom_model_ref(model_id, providers) {
            *model_id = rewritten;
            changed = true;
        }
    }
    if changed {
        if let Ok(json) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(&defaults_path, json);
        }
    }
}

fn write_custom_providers_file(providers: &[CustomProvider]) -> Result<(), String> {
    let path = custom_providers_path()?;
    let file = CustomProvidersFile {
        version: CUSTOM_PROVIDERS_FILE_VERSION,
        providers: providers.to_vec(),
    };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("Failed to serialize custom providers: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to save custom providers: {e}"))
}

fn migrate_custom_providers_file(file: &mut CustomProvidersFile) -> bool {
    if file.version >= CUSTOM_PROVIDERS_FILE_VERSION {
        return false;
    }

    // v2 had no remote-compaction capability. Persist the explicit disabled
    // value so the migration is repeatable and does not depend on serde's
    // in-memory default after the first successful load.
    for provider in &mut file.providers {
        for model in &mut provider.models {
            model.remote_compaction_mode = RemoteCompactionMode::Disabled;
        }
    }
    file.version = CUSTOM_PROVIDERS_FILE_VERSION;
    true
}

/// Load providers (keys NOT filled in). Lazily migrates custom_endpoints.json
/// on first read; the legacy file is left in place so downgrades still work.
pub(crate) fn load_custom_providers() -> Result<Vec<CustomProvider>, String> {
    let path = custom_providers_path()?;
    if path.exists() {
        let json = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read custom providers: {e}"))?;
        let mut file: CustomProvidersFile = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse custom providers: {e}"))?;
        let migrated = migrate_custom_providers_file(&mut file);
        for provider in &mut file.providers {
            normalize_custom_provider_config(provider);
        }
        if migrated {
            write_custom_providers_file(&file.providers)?;
        }
        return Ok(file.providers);
    }

    let legacy_path = persistent_config_dir()?.join("custom_endpoints.json");
    let endpoints: Vec<CustomEndpoint> = std::fs::read_to_string(&legacy_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let providers: Vec<CustomProvider> = endpoints
        .into_iter()
        .map(migrate_endpoint_to_provider)
        .collect();
    if !providers.is_empty() {
        if let Err(error) = write_custom_providers_file(&providers) {
            eprintln!("[Locus] custom provider migration write failed: {error}");
        } else {
            rewrite_legacy_custom_model_refs(&providers);
        }
    }
    Ok(providers)
}

/// Resolve `custom/<provider_id>/<model_row_id>` (or the legacy
/// `custom/<endpoint_id>`, which maps to the provider's first model).
pub(crate) fn find_custom_provider_model(
    model_id: &str,
) -> Result<Option<(CustomProvider, CustomProviderModel)>, String> {
    let Some(rest) = model_id.trim().strip_prefix("custom/") else {
        return Ok(None);
    };
    let (provider_id, model_row_id) = match rest.split_once('/') {
        Some((pid, mid)) => (pid, Some(mid)),
        None => (rest, None),
    };
    let providers = load_custom_providers()?;
    let Some(provider) = providers.into_iter().find(|p| p.id == provider_id) else {
        return Ok(None);
    };
    let model = match model_row_id {
        Some(mid) => provider.models.iter().find(|m| m.id == mid).cloned(),
        None => provider.models.first().cloned(),
    };
    Ok(model.map(|m| (provider, m)))
}

fn valid_custom_model_refs(providers: &[CustomProvider]) -> HashSet<String> {
    let mut refs = HashSet::new();
    for provider in providers {
        // Legacy single-segment references stay valid while the provider exists.
        refs.insert(provider.id.clone());
        for model in &provider.models {
            refs.insert(format!("{}/{}", provider.id, model.id));
        }
    }
    refs
}

#[tauri::command]
pub async fn get_custom_providers() -> Result<Vec<CustomProvider>, AppError> {
    let mut providers = load_custom_providers()?;
    for provider in &mut providers {
        if let Ok(Some(key)) = keychain::get_secret(&keychain::endpoint_key_name(&provider.id)) {
            provider.api_key = key;
        }
    }
    Ok(providers)
}

#[tauri::command]
pub async fn save_custom_providers(providers: Vec<CustomProvider>) -> Result<(), AppError> {
    let previous_ids: HashSet<String> = load_custom_providers()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.id)
        .collect();

    let mut normalized = providers;
    for provider in &mut normalized {
        normalize_custom_provider_config(provider);
        if !provider.api_key.is_empty() {
            keychain::set_secret(
                &keychain::endpoint_key_name(&provider.id),
                &provider.api_key,
            )?;
        } else {
            let _ = keychain::delete_secret(&keychain::endpoint_key_name(&provider.id));
        }
        provider.api_key = String::new();
    }

    write_custom_providers_file(&normalized)?;

    let next_ids: HashSet<String> = normalized.iter().map(|p| p.id.clone()).collect();
    for stale in previous_ids.difference(&next_ids) {
        let _ = keychain::delete_secret(&keychain::endpoint_key_name(stale));
    }
    prune_stale_custom_model_refs(&valid_custom_model_refs(&normalized))?;
    Ok(())
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
    let head = trimmed
        .chars()
        .take(32)
        .collect::<String>()
        .to_ascii_lowercase();
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

fn endpoint_html_response_error(message: String, status: Option<reqwest::StatusCode>) -> AppError {
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
pub fn debug_webview_bridge_heartbeat(
    heartbeat: crate::cdp_debug::FrontendBridgeHeartbeat,
    config: State<'_, Arc<crate::config::AppConfig>>,
    diagnostics: State<'_, Arc<crate::cdp_debug::CdpDebugServerHandle>>,
) -> Result<(), AppError> {
    if config.debug_enabled() {
        diagnostics.record_frontend_heartbeat(heartbeat);
    }
    Ok(())
}

#[tauri::command]
pub async fn set_debug_mode(
    value: bool,
    app_handle: AppHandle,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config.set_debug_enabled(value).map_err(AppError::from)?;
    if let Err(error) = crate::cdp_debug::reconcile(app_handle, value).await {
        if value {
            let rollback_error = config.set_debug_enabled(false).err();
            let message = match rollback_error {
                Some(rollback_error) => {
                    format!("{error}; failed to roll back debug mode: {rollback_error}")
                }
                None => error,
            };
            return Err(AppError::from(message));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn get_tool_failure_log_enabled(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.tool_failure_log_enabled())
}

#[tauri::command]
pub async fn set_tool_failure_log_enabled(
    value: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config
        .set_tool_failure_log_enabled(value)
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn get_session_undo_enabled(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.session_undo_enabled())
}

#[tauri::command]
pub async fn set_session_undo_enabled(
    value: bool,
    app_handle: AppHandle,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config
        .set_session_undo_enabled(value)
        .map_err(AppError::from)?;
    if let Err(error) = app_handle.emit(
        SESSION_UNDO_ENABLED_CHANGED_EVENT,
        SessionUndoEnabledChangedEvent { enabled: value },
    ) {
        eprintln!("[Locus] failed to publish session undo setting: {error}");
    }
    Ok(())
}

#[tauri::command]
pub async fn get_llm_retry_max_attempts(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    Ok(config.llm_retry_max_attempts())
}

/// Persist the automatic LLM retry count (0 = disabled, clamped to 10) and
/// mirror it into the live `llm::retry` global the transports read.
#[tauri::command]
pub async fn set_llm_retry_max_attempts(
    value: u32,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    config
        .set_llm_retry_max_attempts(value)
        .map_err(AppError::from)?;
    crate::llm::retry::set_max_retries(value);
    Ok(config.llm_retry_max_attempts())
}

#[tauri::command]
pub async fn get_subagent_max_depth(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    Ok(config.subagent_max_depth())
}

/// Persist the `subagent` nesting-depth cap (clamped to 1..=8; 1 means
/// subagents cannot spawn further subagents).
#[tauri::command]
pub async fn set_subagent_max_depth(
    value: u32,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    config
        .set_subagent_max_depth(value)
        .map_err(AppError::from)?;
    Ok(config.subagent_max_depth())
}

#[tauri::command]
pub async fn get_subagent_max_concurrent(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    Ok(config.subagent_max_concurrent())
}

/// Persist the concurrent `subagent` cap per top-level agent tree
/// (clamped to 1..=16).
#[tauri::command]
pub async fn set_subagent_max_concurrent(
    value: u32,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<u32, AppError> {
    config
        .set_subagent_max_concurrent(value)
        .map_err(AppError::from)?;
    Ok(config.subagent_max_concurrent())
}

#[tauri::command]
pub async fn get_file_tool_workspace_boundary(
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<bool, AppError> {
    Ok(config.file_tool_workspace_boundary_enabled())
}

#[tauri::command]
pub async fn set_file_tool_workspace_boundary(
    value: bool,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), AppError> {
    config
        .set_file_tool_workspace_boundary_enabled(value)
        .map_err(AppError::from)?;
    Ok(())
}

#[tauri::command]
pub async fn get_unity_test_tools_workspace_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::workspace::UnityTestToolsWorkspaceStatus, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_test_tools_workspace_status",
    )?;
    let working_dir = scope.runtime().root().to_string_lossy().to_string();
    Ok(crate::workspace::unity_test_tools_workspace_status(
        &working_dir,
    ))
}

#[tauri::command]
pub async fn set_unity_test_tools_workspace_enabled(
    value: bool,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::workspace::UnityTestToolsWorkspaceStatus, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "set_unity_test_tools_workspace_enabled",
    )?;
    let working_dir = scope.runtime().root().to_string_lossy().to_string();
    crate::workspace::set_unity_test_tools_enabled(&working_dir, value).map_err(AppError::from)?;
    Ok(crate::workspace::unity_test_tools_workspace_status(
        &working_dir,
    ))
}

#[tauri::command]
pub async fn get_tool_permission_mode(
    mode: State<'_, crate::ToolPermissionMode>,
) -> Result<String, AppError> {
    Ok(mode.0.read().await.clone())
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
    *mode_state.0.write().await = normalized.clone();
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
    Ok(perms.0.read().await.clone())
}

#[tauri::command]
pub async fn save_tool_permissions(
    value: std::collections::HashMap<String, String>,
    perms: State<'_, crate::ToolPermissions>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    let mut normalized: std::collections::HashMap<String, String> = value
        .into_iter()
        .map(|(k, v)| {
            let mode = normalize_tool_permission_mode_request(Some(v.as_str()), None).to_string();
            (k, mode)
        })
        .collect();
    if !normalized.contains_key("subagent") {
        if let Some(mode) = normalized.get("task").cloned() {
            normalized.insert("subagent".to_string(), mode);
        }
    }
    normalized.remove("task");
    *perms.0.write().await = normalized.clone();
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
pub struct WorkspaceEntryStat {
    pub path: String,
    pub exists: bool,
    pub entry_kind: String,
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
const LINKED_ASSET_ROOT_DIRS: &[&str] = &["Assets", "Packages"];
const WORKSPACE_SEARCH_MAX_DEPTH: usize = 64;

pub(crate) fn normalize_workspace_sub_path(sub_path: &str) -> Result<String, AppError> {
    let unified = sub_path.replace('\\', "/");
    if unified.contains('\0')
        || unified.starts_with('/')
        || unified
            .split('/')
            .next()
            .map(|head| {
                head.len() >= 2
                    && head.as_bytes()[1] == b':'
                    && head.as_bytes()[0].is_ascii_alphabetic()
            })
            .unwrap_or(false)
    {
        return Err("Path is not within the working directory"
            .to_string()
            .into());
    }

    let mut parts = Vec::new();

    for component in std::path::Path::new(&unified).components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err("Path is not within the working directory"
                    .to_string()
                    .into());
            }
        }
    }

    Ok(parts.join("/"))
}

fn resolve_workspace_dir_target(
    cwd: &str,
    sub_path: &str,
) -> Result<(std::path::PathBuf, String), AppError> {
    let base = std::path::Path::new(cwd);
    let normalized_sub_path = normalize_workspace_sub_path(sub_path)?;
    let target = if normalized_sub_path.is_empty() {
        base.to_path_buf()
    } else {
        base.join(&normalized_sub_path)
    };

    if !target.is_dir() {
        return Ok((target, normalized_sub_path));
    }

    let canonical_base = dunce::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let canonical_target = dunce::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if canonical_target.starts_with(&canonical_base)
        || path_reaches_allowed_linked_asset_dir(base, &normalized_sub_path)
    {
        return Ok((target, normalized_sub_path));
    }

    Err("Path is not within the working directory"
        .to_string()
        .into())
}

fn normalized_hidden_directory_set(hidden_dirs: Vec<String>) -> HashSet<String> {
    hidden_dirs
        .into_iter()
        .filter_map(|name| {
            let trimmed = name.trim().trim_end_matches(['/', '\\']);
            if trimmed.is_empty()
                || trimmed == "."
                || trimmed == ".."
                || trimmed.contains('/')
                || trimmed.contains('\\')
            {
                None
            } else {
                Some(trimmed.to_lowercase())
            }
        })
        .collect()
}

fn should_skip_workspace_entry_with_hidden(
    file_name: &str,
    is_dir: bool,
    exclude_meta: bool,
    hidden_dirs: Option<&HashSet<String>>,
) -> bool {
    if exclude_meta && file_name.ends_with(".meta") {
        return true;
    }

    if !is_dir {
        return false;
    }

    match hidden_dirs {
        Some(names) => names.contains(&file_name.to_lowercase()),
        None => {
            file_name.starts_with('.')
                || WORKSPACE_HIDDEN_DIRS
                    .iter()
                    .any(|hidden| hidden.eq_ignore_ascii_case(file_name))
        }
    }
}

fn join_workspace_rel_path(sub_path: &str, file_name: &str) -> String {
    if sub_path.is_empty() {
        file_name.to_string()
    } else {
        format!("{}/{}", sub_path.trim_end_matches('/'), file_name)
    }
}

fn is_allowed_linked_asset_rel_path(rel_path: &str) -> bool {
    LINKED_ASSET_ROOT_DIRS.iter().any(|root| {
        rel_path == *root
            || rel_path
                .strip_prefix(root)
                .map(|rest| rest.starts_with('/'))
                .unwrap_or(false)
    })
}

fn path_reaches_allowed_linked_asset_dir(base: &std::path::Path, rel_path: &str) -> bool {
    let mut current = base.to_path_buf();
    let mut rel_parts = Vec::new();
    let mut saw_allowed_linked_dir = false;

    for part in rel_path.split('/').filter(|part| !part.is_empty()) {
        current.push(part);
        rel_parts.push(part);
        if path_is_symlink_dir(&current) {
            let current_rel_path = rel_parts.join("/");
            if !is_allowed_linked_asset_rel_path(&current_rel_path) {
                return false;
            }
            saw_allowed_linked_dir = true;
        }
    }

    saw_allowed_linked_dir
}

fn entry_is_dir(entry: &std::fs::DirEntry, rel_path: &str) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_dir() => true,
        Ok(file_type) if file_type.is_symlink() => {
            is_allowed_linked_asset_rel_path(rel_path) && entry.path().is_dir()
        }
        Ok(_) => false,
        Err(_) => {
            let path = entry.path();
            if path_is_symlink_dir(&path) {
                is_allowed_linked_asset_rel_path(rel_path)
            } else {
                path.is_dir()
            }
        }
    }
}

fn entry_is_file(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_file() => true,
        Ok(file_type) if file_type.is_symlink() => entry.path().is_file(),
        Ok(_) => false,
        Err(_) => entry.path().is_file(),
    }
}

fn entry_is_symlink_dir(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(file_type) if file_type.is_symlink() => entry.path().is_dir(),
        _ => false,
    }
}

fn entry_is_disallowed_symlink_dir(entry: &std::fs::DirEntry, rel_path: &str) -> bool {
    entry_is_symlink_dir(entry) && !is_allowed_linked_asset_rel_path(rel_path)
}

fn path_is_symlink_dir(path: &std::path::Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => path.is_dir(),
        _ => false,
    }
}

fn collect_dir_entries(
    target: &std::path::Path,
    sub_path: &str,
    exclude_meta: bool,
) -> Result<Vec<DirEntry>, AppError> {
    collect_dir_entries_with_hidden(target, sub_path, exclude_meta, None)
}

fn collect_dir_entries_with_hidden(
    target: &std::path::Path,
    sub_path: &str,
    exclude_meta: bool,
    hidden_dirs: Option<&HashSet<String>>,
) -> Result<Vec<DirEntry>, AppError> {
    let mut entries: Vec<DirEntry> = Vec::new();
    let read_dir =
        std::fs::read_dir(target).map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let rel_path = join_workspace_rel_path(sub_path, &file_name);

        if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
            continue;
        }

        let is_dir = entry_is_dir(&entry, &rel_path);

        if should_skip_workspace_entry_with_hidden(&file_name, is_dir, exclude_meta, hidden_dirs) {
            continue;
        }

        entries.push(DirEntry {
            rel_path,
            name: file_name,
            is_dir,
        });
    }

    entries.sort_by_cached_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));

    Ok(entries)
}

fn normalize_workspace_entry_stat_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') && !normalized.ends_with(":/") {
        normalized.pop();
    }
    normalized
}

fn missing_workspace_entry_stat(path: &str) -> WorkspaceEntryStat {
    WorkspaceEntryStat {
        path: normalize_workspace_entry_stat_path(path),
        exists: false,
        entry_kind: "missing".to_string(),
    }
}

fn workspace_entry_stat_from_target(path: String, target: &std::path::Path) -> WorkspaceEntryStat {
    let is_dir = target.is_dir();
    let is_file = target.is_file();
    let entry_kind = if is_dir {
        "folder"
    } else if is_file {
        "file"
    } else {
        "other"
    };

    WorkspaceEntryStat {
        path,
        exists: target.exists(),
        entry_kind: entry_kind.to_string(),
    }
}

fn is_workspace_entry_stat_absolute_path(path: &str) -> bool {
    let normalized = path.trim();
    std::path::Path::new(normalized).is_absolute()
        || normalized.starts_with("\\\\")
        || normalized.starts_with("//")
}

pub(crate) fn workspace_entry_target_allowed(
    workspace_root: &std::path::Path,
    rel_path: &str,
    target: &std::path::Path,
) -> bool {
    let canonical_base =
        dunce::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    let canonical_target = match dunce::canonicalize(target) {
        Ok(path) => path,
        Err(_) => return false,
    };
    canonical_target.starts_with(&canonical_base)
        || path_reaches_allowed_linked_asset_dir(workspace_root, rel_path)
}

fn workspace_entry_stat_for_path(cwd: &str, raw_path: &str) -> WorkspaceEntryStat {
    let display_path = normalize_workspace_entry_stat_path(raw_path);
    if display_path.is_empty() {
        return missing_workspace_entry_stat(raw_path);
    }

    if is_workspace_entry_stat_absolute_path(&display_path) {
        let target =
            std::path::PathBuf::from(display_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !target.exists() {
            return missing_workspace_entry_stat(&display_path);
        }
        return workspace_entry_stat_from_target(display_path, &target);
    }

    if cwd.trim().is_empty() {
        return missing_workspace_entry_stat(&display_path);
    }

    let rel_path = match normalize_workspace_sub_path(&display_path) {
        Ok(path) if !path.is_empty() => path,
        _ => return missing_workspace_entry_stat(&display_path),
    };
    let workspace_root = std::path::Path::new(cwd);
    let target = workspace_root.join(&rel_path);
    if !target.exists() {
        return missing_workspace_entry_stat(&rel_path);
    }
    if !workspace_entry_target_allowed(workspace_root, &rel_path, &target) {
        return missing_workspace_entry_stat(&rel_path);
    }

    workspace_entry_stat_from_target(rel_path, &target)
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
    hidden_dirs: Option<&HashSet<String>>,
) -> Result<(), AppError> {
    let initial_linked_visit_keys = path_is_symlink_dir(root_dir).then(|| Arc::new(HashSet::new()));
    let mut stack = vec![(
        root_dir.to_path_buf(),
        root_rel_path.to_string(),
        0usize,
        initial_linked_visit_keys,
    )];

    while let Some((dir_path, dir_rel_path, depth, linked_visit_keys)) = stack.pop() {
        let current_linked_visit_keys = if let Some(keys) = linked_visit_keys {
            let visit_key = dunce::canonicalize(&dir_path).unwrap_or_else(|_| dir_path.clone());
            if keys.contains(&visit_key) {
                continue;
            }
            let mut updated = (*keys).clone();
            updated.insert(visit_key);
            Some(Arc::new(updated))
        } else {
            None
        };

        let read_dir =
            std::fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?;
        let mut child_dirs: Vec<(
            std::path::PathBuf,
            String,
            Option<Arc<HashSet<std::path::PathBuf>>>,
        )> = Vec::new();

        for entry in read_dir.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();
            let rel_path = join_workspace_rel_path(&dir_rel_path, &file_name);
            if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
                continue;
            }
            let is_dir = entry_is_dir(&entry, &rel_path);
            if should_skip_workspace_entry_with_hidden(&file_name, is_dir, false, hidden_dirs) {
                continue;
            }

            let is_file = entry_is_file(&entry);
            if !is_dir && (!include_files || !is_file) {
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

            if is_dir && depth < WORKSPACE_SEARCH_MAX_DEPTH {
                let child_linked_visit_keys = current_linked_visit_keys
                    .clone()
                    .or_else(|| entry_is_symlink_dir(&entry).then(|| Arc::new(HashSet::new())));
                child_dirs.push((entry.path(), rel_path, child_linked_visit_keys));
            }
        }

        child_dirs.sort_by(|left, right| right.1.cmp(&left.1));
        stack.extend(
            child_dirs
                .into_iter()
                .map(|(path, rel_path, linked_visit_keys)| {
                    (path, rel_path, depth + 1, linked_visit_keys)
                }),
        );
    }

    Ok(())
}

#[cfg(test)]
fn search_workspace_entries_in_dir(
    workspace_root: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    search_workspace_entries_in_dir_with_hidden(workspace_root, query, limit, None)
}

fn search_workspace_entries_in_dir_with_hidden(
    workspace_root: &std::path::Path,
    query: &str,
    limit: usize,
    hidden_dirs: Option<&HashSet<String>>,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    if !workspace_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let read_dir = std::fs::read_dir(workspace_root)
        .map_err(|e| format!("Failed to read directory: {}", e))?;

    for entry in read_dir.flatten() {
        let file_name = entry.file_name().to_string_lossy().to_string();
        let rel_path = file_name.clone();
        if entry_is_disallowed_symlink_dir(&entry, &rel_path) {
            continue;
        }
        let is_dir = entry_is_dir(&entry, &rel_path);
        if should_skip_workspace_entry_with_hidden(&file_name, is_dir, false, hidden_dirs) {
            continue;
        }

        if let Some(match_score) = workspace_search_score(query, &file_name, &rel_path, is_dir) {
            results.push(build_workspace_search_entry(
                rel_path.clone(),
                file_name.clone(),
                is_dir,
                match_score,
            ));
        }

        if !is_dir {
            continue;
        }

        let include_files = hidden_dirs.is_some() || !ASSET_ROOT_DIRS.contains(&rel_path.as_str());
        collect_workspace_search_entries(
            &entry.path(),
            &rel_path,
            include_files,
            query,
            &mut results,
            hidden_dirs,
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
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<DirEntry>, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "list_dir_entries",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    let (target, normalized_sub_path) = resolve_workspace_dir_target(&cwd, &sub_path)?;
    if !target.is_dir() {
        return Ok(vec![]);
    }

    collect_dir_entries(&target, &normalized_sub_path, false)
}

#[tauri::command]
pub async fn list_dir_entries_page(
    sub_path: String,
    offset: Option<usize>,
    limit: Option<usize>,
    exclude_meta: Option<bool>,
    hidden_dirs: Option<Vec<String>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<DirEntriesPage, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "list_dir_entries_page",
    )?;
    let runtime = scope.runtime();
    let dir_entries_cache = runtime.core().dir_entries_page_cache();
    let cwd = runtime.root().to_string_lossy().to_string();
    let (target, normalized_sub_path) = resolve_workspace_dir_target(&cwd, &sub_path)?;
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
    let uses_configured_hidden_dirs = hidden_dirs.is_some();
    let hidden_dirs = hidden_dirs.map(normalized_hidden_directory_set);
    let mut hidden_cache_key = hidden_dirs
        .as_ref()
        .map(|names| names.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    hidden_cache_key.sort();
    let cache_key = format!(
        "{}::{}::{}::{}::{}",
        runtime.checkout_id(),
        runtime.generation(),
        normalized_sub_path,
        u8::from(exclude_meta),
        format!(
            "{}:{}",
            if uses_configured_hidden_dirs {
                "configured"
            } else {
                "default"
            },
            hidden_cache_key.join("\u{1f}")
        )
    );

    let listing = if offset == 0 {
        let entries = collect_dir_entries_with_hidden(
            &target,
            &normalized_sub_path,
            exclude_meta,
            hidden_dirs.as_ref(),
        )?;
        dir_entries_cache.insert(cache_key.clone(), entries)
    } else if let Some(cached) = dir_entries_cache.get(&cache_key) {
        cached
    } else {
        let entries = collect_dir_entries_with_hidden(
            &target,
            &normalized_sub_path,
            exclude_meta,
            hidden_dirs.as_ref(),
        )?;
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
    hidden_dirs: Option<Vec<String>>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<WorkspaceSearchEntry>, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "search_workspace_entries",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    if cwd.trim().is_empty() {
        return Ok(Vec::new());
    }

    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let limit = limit.unwrap_or(200).clamp(1, 500);
    let hidden_dirs = hidden_dirs.map(normalized_hidden_directory_set);
    search_workspace_entries_in_dir_with_hidden(
        std::path::Path::new(&cwd),
        trimmed,
        limit,
        hidden_dirs.as_ref(),
    )
}

#[tauri::command]
pub async fn stat_workspace_entries(
    paths: Vec<String>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<WorkspaceEntryStat>, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "stat_workspace_entries",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    let mut stats = Vec::new();
    let mut seen = HashSet::new();

    for path in paths.into_iter().take(300) {
        let normalized = normalize_workspace_entry_stat_path(&path);
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        stats.push(workspace_entry_stat_for_path(&cwd, &normalized));
    }

    Ok(stats)
}

const UNITY_IPC_READY_TIMEOUT: Duration = Duration::from_secs(45);

pub(crate) struct UnityReadyIpcScope {
    execution: Arc<AgentExecutionContext>,
    _binding: ResolvedServiceBinding,
}

impl UnityReadyIpcScope {
    pub(crate) fn root_text(&self) -> String {
        self.execution.root().to_string_lossy().to_string()
    }

    pub(crate) fn checkout_event_scope(
        &self,
    ) -> crate::workspace_service::event::WorkspaceEventScope {
        crate::workspace_service::event::WorkspaceEventScope {
            project_id: self.execution.project_id.clone(),
            checkout_id: self.execution.checkout_id.clone(),
            workspace_generation: self.execution.workspace_generation,
            service_instance_id: None,
            service_generation: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityCheckoutConnectionStatus {
    pub checkout_id: String,
    pub workspace_generation: u64,
    pub connected: bool,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_status: Option<ServiceStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<ServiceReadinessSnapshot>,
}

fn unity_connection_flags(readiness: Option<&ServiceReadinessSnapshot>) -> (bool, bool) {
    match readiness.map(|snapshot| snapshot.phase) {
        Some(ServiceReadinessPhase::Connected | ServiceReadinessPhase::Ready) => (
            true,
            readiness.is_some_and(|snapshot| snapshot.phase == ServiceReadinessPhase::Ready),
        ),
        // The native broker can remain connected while the managed domain is
        // reloading. Keep the connection indicator visible while commands wait.
        Some(ServiceReadinessPhase::Reloading) => (true, false),
        _ => (false, false),
    }
}

fn unity_service_operation_error(
    operation: &'static str,
    error: impl std::fmt::Display,
) -> AppError {
    AppError::new(
        "unity.checkout_service_unavailable",
        "The Unity service for this checkout is unavailable.",
    )
    .detail(error.to_string())
    .operation(operation)
    .retryable(true)
}

fn unity_ready_binding_error(operation: &'static str, error: ServiceBindingError) -> AppError {
    let detail = error.diagnostic_json().unwrap_or_else(|| error.to_string());
    AppError::new(
        "unity.checkout_not_ready",
        "Unity is connected to this checkout but is not ready to execute commands.",
    )
    .detail(detail)
    .operation(operation)
    .retryable(true)
}

async fn unity_execution_context(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<Arc<AgentExecutionContext>, AppError> {
    let scope =
        super::session::resolve_workspace_scope(workspace_registry, workspace_ref, operation)?;
    let expected_checkout = scope.runtime().checkout_id().clone();
    let expected_generation = scope.runtime().generation();
    let execution = workspace_registry
        .execution_context(&expected_checkout, &[ServiceKind::Unity])
        .await
        .map_err(|error| unity_service_operation_error(operation, error))?;
    if execution.checkout_id != expected_checkout
        || execution.workspace_generation != expected_generation
    {
        return Err(unity_service_operation_error(
            operation,
            format!(
                "Unity execution scope changed: checkout={} generation={}, actualCheckout={} actualGeneration={}",
                expected_checkout,
                expected_generation,
                execution.checkout_id,
                execution.workspace_generation
            ),
        ));
    }
    Ok(execution)
}

pub(crate) async fn resolve_unity_ready_ipc_scope(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<UnityReadyIpcScope, AppError> {
    let execution = unity_execution_context(workspace_registry, workspace_ref, operation).await?;
    let binding = execution
        .resolve_service_ready(ServiceKind::Unity, UNITY_IPC_READY_TIMEOUT)
        .await
        .map_err(|error| unity_ready_binding_error(operation, error))?;
    Ok(UnityReadyIpcScope {
        execution,
        _binding: binding,
    })
}

async fn unity_checkout_connection_status(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<UnityCheckoutConnectionStatus, AppError> {
    let scope =
        super::session::resolve_workspace_scope(workspace_registry, workspace_ref, operation)?;
    let runtime = scope.runtime();
    let state: Option<WorkspaceServiceStateSnapshot> =
        runtime.services().state_snapshot(ServiceKind::Unity).await;
    let readiness = state
        .as_ref()
        .and_then(|snapshot| snapshot.readiness.clone());
    let (connected, ready) = unity_connection_flags(readiness.as_ref());
    Ok(UnityCheckoutConnectionStatus {
        checkout_id: runtime.checkout_id().to_string(),
        workspace_generation: runtime.generation(),
        connected,
        ready,
        service_status: state.map(|snapshot| snapshot.status),
        readiness,
    })
}

#[tauri::command]
pub async fn check_unity_connection(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<bool, AppError> {
    Ok(unity_checkout_connection_status(
        workspace_registry.inner(),
        &workspace_ref,
        "check_unity_connection",
    )
    .await?
    .connected)
}

#[tauri::command]
pub async fn check_unity_connection_status(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnityCheckoutConnectionStatus, AppError> {
    unity_checkout_connection_status(
        workspace_registry.inner(),
        &workspace_ref,
        "check_unity_connection_status",
    )
    .await
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConsoleTextEntry {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityConsoleTextPayload {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub entries: Vec<UnityConsoleTextEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[tauri::command]
pub async fn get_unity_console_text(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<UnityConsoleTextPayload, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_unity_console_text",
    )
    .await?;
    let cwd = ready.root_text();
    let resp = crate::unity_bridge::send_message(&cwd, "get_console_text", "").await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "Failed to read Unity Console".to_string())
            .into());
    }

    let message = resp.message.unwrap_or_default();
    let mut payload: UnityConsoleTextPayload = serde_json::from_str(&message).map_err(|error| {
        AppError::from(format!("Failed to parse Unity Console response: {error}"))
    })?;
    payload
        .entries
        .retain(|entry| !entry.text.trim().is_empty());
    Ok(payload)
}

#[tauri::command]
pub async fn check_unity_plugin(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::unity_bridge::PluginStatus, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "check_unity_plugin",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(crate::unity_bridge::PluginStatus::UpToDate);
    }
    crate::unity_bridge::check_plugin_status(&cwd).map_err(Into::into)
}

#[tauri::command]
pub async fn check_unity_plugin_install_plan(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::unity_bridge::PluginInstallPlan, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "check_unity_plugin_install_plan",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Ok(crate::unity_bridge::PluginInstallPlan {
            status: crate::unity_bridge::PluginStatus::UpToDate,
            dll_update_required: false,
            unity_running: false,
            unity_process_ids: Vec::new(),
        });
    }
    crate::unity_bridge::check_plugin_install_plan(&cwd)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn install_unity_plugin(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
    force_close_unity: Option<bool>,
) -> Result<String, AppError> {
    let scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "install_unity_plugin",
    )?;
    let runtime = scope.runtime();
    let cwd = runtime.root().to_string_lossy().to_string();
    if !crate::unity_bridge::is_unity_project(&cwd) {
        return Err("Current working directory is not a Unity project"
            .to_string()
            .into());
    }

    // The Unity-hosted Locus window is a cross-process WS_CHILD. Detach it on
    // the GUI thread before terminating Unity so WebView2 keeps a valid host
    // HWND throughout the plugin replacement and editor restart.
    let unity_embed_quiesce =
        super::quiesce_unity_embed_control_windows(&app_handle, &workspace_ref).await?;
    let install_result = crate::unity_bridge::install_or_update_plugin_with_force_close(
        &cwd,
        force_close_unity.unwrap_or(false),
    )
    .await;
    drop(unity_embed_quiesce);
    let hash = install_result?;
    let event_scope = crate::workspace_service::event::WorkspaceEventScope {
        project_id: runtime.project_id().clone(),
        checkout_id: runtime.checkout_id().clone(),
        workspace_generation: runtime.generation(),
        service_instance_id: None,
        service_generation: None,
    };
    crate::unity_bridge::emit_plugin_status_scoped(&app_handle, &cwd, &event_scope);
    Ok(hash)
}

#[tauri::command]
pub async fn launch_unity_project(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<crate::unity_bridge::UnityLaunchResult, AppError> {
    // Launching is a process operation and does not require an already-ready
    // editor. Starting the checkout service first establishes its monitor and
    // readiness barrier before the new process begins connecting.
    let execution = unity_execution_context(
        workspace_registry.inner(),
        &workspace_ref,
        "launch_unity_project",
    )
    .await?;
    let _monitor_binding = execution
        .resolve_service(ServiceKind::Unity)
        .map_err(|error| unity_service_operation_error("launch_unity_project", error))?;
    let cwd = execution.root().to_string_lossy().to_string();
    crate::unity_bridge::launch_project(&cwd)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn close_headless_unity_project(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<(), AppError> {
    let scope = workspace_registry
        .resolve_workspace_ref(&workspace_ref)
        .map_err(|error| AppError::from(error.to_string()))?;
    let project = scope.runtime().root().to_string_lossy().to_string();
    let status = crate::unity_bridge::query_unity_connection_status(&project).await;
    if status.editor_process_state == crate::unity_bridge::UnityEditorProcessState::NotRunning {
        return Ok(());
    }
    if !status.headless {
        return Err(AppError::from(
            "The running Unity editor is interactive; close_headless_unity_project only closes a headless editor"
                .to_string(),
        ));
    }
    crate::unity_bridge::close_current_project_unity_processes(&project, Duration::from_secs(60))
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// Drive a Unity recompile + domain reload and wait for it to settle. Thin
/// wrapper over `recompile_and_wait` so plugin pushes and corpus changes can
/// be converged deterministically from a host driver (the same flow the
/// `unity_recompile` agent tool uses).
#[tauri::command]
pub async fn unity_recompile_run(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_recompile_run",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::recompile_and_wait(&cwd)
        .await
        .map_err(Into::into)
}

/// Test-page probe: write a throwaway harmless `.cs` into the current project's
/// `Assets`, drive a real recompile, then delete it (and its `.meta`) and
/// converge the deletion. Verifies a recompile actually converges — and whether
/// the background hook let it happen without bringing Unity to the foreground.
/// Returns a line-oriented report for the test page to render.
#[tauri::command]
pub async fn unity_recompile_probe_run(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_recompile_probe_run",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::run_recompile_probe(&cwd)
        .await
        .map_err(Into::into)
}

/// Run a one-off C# snippet in the connected Unity Editor and return its
/// string result. Thin host-driver wrapper over `unity_execute_code` (the
/// same path the `unity_execute` tool and the hot-reload self-test use) so a
/// CDP driver can poke editor state — e.g. toggle `EditorSettings` for the
/// no-domain-reload verification matrix.
#[tauri::command]
pub async fn unity_execute_snippet_run(
    code: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "unity_execute_snippet_run",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::unity_execute_code(&cwd, &code)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn send_unity_log(
    message: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready =
        resolve_unity_ready_ipc_scope(workspace_registry.inner(), &workspace_ref, "send_unity_log")
            .await?;
    let cwd = ready.root_text();
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
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "select_unity_asset",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::select_asset(&cwd, &asset_path, focus_project_window.unwrap_or(true))
        .await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_asset_inspector(
    asset_path: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "open_unity_asset_inspector",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::open_asset_inspector(&cwd, &asset_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn select_unity_scene_object(
    scene_path: String,
    object_path: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "select_unity_scene_object",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::select_scene_object(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn validate_unity_scene_object(
    scene_path: String,
    object_path: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "validate_unity_scene_object",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::validate_scene_object(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn open_unity_scene_object_inspector(
    scene_path: String,
    object_path: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<String, AppError> {
    let ready = resolve_unity_ready_ipc_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "open_unity_scene_object_inspector",
    )
    .await?;
    let cwd = ready.root_text();
    crate::unity_bridge::open_scene_object_inspector(&cwd, &scene_path, &object_path).await?;
    Ok("ok".to_string())
}

#[tauri::command]
pub async fn reset_all_config(
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    window_contexts: State<'_, Arc<crate::workspace_service::WindowContextRegistry>>,
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

    // Clear keychain secrets: custom endpoint/provider API keys
    let ep_path = custom_endpoints_path(&app_handle)
        .unwrap_or_else(|_| data_dir.join("custom_endpoints.json"));
    if let Ok(content) = std::fs::read_to_string(&ep_path) {
        if let Ok(endpoints) = serde_json::from_str::<Vec<CustomEndpoint>>(&content) {
            for ep in &endpoints {
                let _ = keychain::delete_secret(&keychain::endpoint_key_name(&ep.id));
            }
        }
    }
    if let Ok(providers) = load_custom_providers() {
        for provider in &providers {
            let _ = keychain::delete_secret(&keychain::endpoint_key_name(&provider.id));
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
            "codex_fast_mode.txt",
            "model_defaults.json",
            "custom_endpoints.json",
            "custom_providers.json",
            "model_catalog.json",
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

    let window_ids = window_contexts
        .snapshots()
        .map_err(|error| AppError::new("workspace.focus_context_unavailable", error.to_string()))?
        .into_iter()
        .map(|context| context.window_id)
        .collect::<HashSet<_>>();
    for window_id in window_ids {
        let intent_epoch = window_contexts
            .next_window_intent_epoch(&window_id)
            .map_err(|error| {
                AppError::new("workspace.focus_context_unavailable", error.to_string())
            })?;
        window_contexts
            .remove_window(&window_id, intent_epoch)
            .map_err(|error| {
                AppError::new("workspace.focus_context_unavailable", error.to_string())
            })?;
    }
    workspace_registry.shutdown_all().await;
    super::reset_unity_embed_control_window(&app_handle);
    super::refresh_unity_embed_control_server(app_handle.clone());
    *mode.0.write().await = "auto".to_string();
    *perms.0.write().await = std::collections::HashMap::new();
    *api_key_state.write().await = String::new();
    *provider_keys.write().await = std::collections::HashMap::new();
    auth.lock().await.logout();
    codex.lock().await.logout();

    eprintln!(
        "[Locus] All config reset (keychain + config files + runtime state + WebView browsing data)"
    );
    Ok(())
}

// ── Config registry ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_config_registry(
    category: Option<String>,
    app_handle: AppHandle,
) -> Result<Vec<crate::config_registry::ConfigEntry>, AppError> {
    match category.as_deref() {
        Some(cat) => crate::config_registry::collect_by_category(&app_handle, cat, None),
        None => crate::config_registry::collect_all(&app_handle, None),
    }
}

#[tauri::command]
pub async fn get_workspace_config_registry(
    category: Option<String>,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    app_handle: AppHandle,
) -> Result<Vec<crate::config_registry::ConfigEntry>, AppError> {
    let workspace_scope = super::session::resolve_workspace_scope(
        workspace_registry.inner(),
        &workspace_ref,
        "get_workspace_config_registry",
    )?;
    match category.as_deref() {
        Some(cat) => {
            crate::config_registry::collect_by_category(&app_handle, cat, Some(&workspace_scope))
        }
        None => crate::config_registry::collect_all(&app_handle, Some(&workspace_scope)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        collect_dir_entries, collect_dir_entries_with_hidden,
        default_provider_prefix_cache_ttl_seconds, is_stale_custom_model_ref,
        migrate_custom_providers_file, migrate_endpoint_to_provider,
        normalize_custom_endpoint_config, normalize_custom_provider_config,
        normalize_tool_permission_mode_request, normalize_workspace_sub_path,
        resolve_workspace_dir_target, rewrite_legacy_custom_model_ref,
        search_workspace_entries_in_dir, search_workspace_entries_in_dir_with_hidden,
        unity_checkout_connection_status, unity_connection_flags, valid_custom_model_refs,
        workspace_entry_stat_for_path, workspace_search_score, ApiFormat, CodexModelConfig,
        CodexTransportMode, CustomEndpoint, CustomProvider, CustomProviderModel,
        CustomProvidersFile, ModelDefaults, RemoteCompactionMode, CUSTOM_PROVIDERS_FILE_VERSION,
        DEFAULT_CODEX_CONTEXT_WINDOW, DEFAULT_CODEX_PREFIX_CACHE_TTL_SECONDS,
    };
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;

    use crate::config::AppConfig;
    use crate::resource_policy::ResourcePolicyStore;
    use crate::workspace_service::service::{ServiceReadinessPhase, ServiceReadinessSnapshot};
    use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

    #[cfg(unix)]
    fn create_dir_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(source, link)
    }

    fn create_dir_symlink_or_skip(source: &Path, link: &Path) -> bool {
        match create_dir_symlink(source, link) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipping symlink test; failed to create directory symlink: {error}");
                false
            }
        }
    }

    fn test_project_registry(config_dir: &Path) -> Arc<ProjectRegistry> {
        let config = Arc::new(AppConfig::load_from_path(&config_dir.join("config.json")));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("resource policy"));
        ProjectRegistry::new(policy, Vec::new())
    }

    fn write_shared_project_identity(root: &Path) {
        let locus = root.join("Locus");
        std::fs::create_dir_all(&locus).expect("Locus directory");
        std::fs::write(
            locus.join("config.json"),
            r#"{"workspace_id":"unity-ipc-project"}"#,
        )
        .expect("workspace identity");
    }

    #[test]
    fn connected_readiness_is_visible_before_ready_without_opening_command_barrier() {
        let connected = ServiceReadinessSnapshot {
            phase: ServiceReadinessPhase::Connected,
            revision: 2,
            detail: Some("managed domain initializing".to_string()),
        };
        let ready = ServiceReadinessSnapshot {
            phase: ServiceReadinessPhase::Ready,
            revision: 3,
            detail: None,
        };
        assert_eq!(unity_connection_flags(Some(&connected)), (true, false));
        assert_eq!(unity_connection_flags(Some(&ready)), (true, true));
    }

    #[tokio::test]
    async fn checkout_connection_status_is_bound_to_each_explicit_scope() {
        let root = tempdir().expect("root");
        let checkout_a = root.path().join("checkout-a");
        let checkout_b = root.path().join("checkout-b");
        std::fs::create_dir_all(&checkout_a).expect("checkout A");
        std::fs::create_dir_all(&checkout_b).expect("checkout B");
        write_shared_project_identity(&checkout_a);
        write_shared_project_identity(&checkout_b);

        let registry = test_project_registry(&root.path().join("config"));
        let runtime_a = registry.open_workspace(&checkout_a).expect("runtime A");
        let runtime_b = registry.open_workspace(&checkout_b).expect("runtime B");
        assert_eq!(runtime_a.project_id(), runtime_b.project_id());
        assert_ne!(runtime_a.checkout_id(), runtime_b.checkout_id());

        let workspace_ref_a = WorkspaceRef::for_runtime(&runtime_a);
        let workspace_ref_b = WorkspaceRef::for_runtime(&runtime_b);
        let status_a =
            unity_checkout_connection_status(&registry, &workspace_ref_a, "test_status_a");
        let status_b =
            unity_checkout_connection_status(&registry, &workspace_ref_b, "test_status_b");
        let (status_a, status_b) = tokio::join!(status_a, status_b);
        let status_a = status_a.expect("status A");
        let status_b = status_b.expect("status B");

        assert_eq!(status_a.checkout_id, runtime_a.checkout_id().as_str());
        assert_eq!(status_b.checkout_id, runtime_b.checkout_id().as_str());
        assert_eq!(status_a.workspace_generation, runtime_a.generation());
        assert_eq!(status_b.workspace_generation, runtime_b.generation());
        assert!(!status_a.connected);
        assert!(!status_b.connected);
    }

    #[test]
    fn legacy_codex_model_config_keeps_opt_in_features_disabled() {
        let config: CodexModelConfig =
            serde_json::from_str(r#"{"transport":"websocket"}"#).expect("codex config");

        assert_eq!(config.transport, CodexTransportMode::Websocket);
        assert_eq!(config.context_window, None);
        assert_eq!(
            config.resolved_context_window(),
            DEFAULT_CODEX_CONTEXT_WINDOW
        );
        assert!(!config.extended_context);
        assert!(!config.generate_session_titles);
        assert!(!config.auto_review);
        assert_eq!(
            config.prefix_cache_ttl_seconds,
            DEFAULT_CODEX_PREFIX_CACHE_TTL_SECONDS
        );
    }

    #[test]
    fn legacy_model_defaults_add_empty_subagent_runtime_overrides() {
        let defaults: ModelDefaults = serde_json::from_str(
            r#"{"mainModel":"openai/gpt-5.6-sol","subagentModels":{"explorer":"openai/gpt-5.6-luna"}}"#,
        )
        .expect("legacy model defaults");

        assert_eq!(
            defaults.subagent_models.get("explorer").map(String::as_str),
            Some("openai/gpt-5.6-luna")
        );
        assert!(defaults.subagent_efforts.is_empty());
        assert!(defaults.subagent_fast_modes.is_empty());
    }

    #[test]
    fn legacy_codex_extended_context_migrates_to_372k() {
        let config: CodexModelConfig =
            serde_json::from_str(r#"{"transport":"websocket","extendedContext":true}"#)
                .expect("codex config");

        assert_eq!(
            config.resolved_context_window(),
            super::LEGACY_CODEX_EXTENDED_CONTEXT_WINDOW
        );
        let normalized = config.normalized_for_save();
        assert_eq!(normalized.context_window, Some(372_000));
        assert!(!normalized.extended_context);
    }

    #[test]
    fn codex_context_window_is_clamped_to_supported_range() {
        let below_minimum: CodexModelConfig =
            serde_json::from_str(r#"{"contextWindow":1}"#).expect("codex config");
        let above_maximum: CodexModelConfig =
            serde_json::from_str(r#"{"contextWindow":2000000}"#).expect("codex config");

        assert_eq!(
            below_minimum.resolved_context_window(),
            super::MIN_CODEX_CONTEXT_WINDOW
        );
        assert_eq!(
            above_maximum.resolved_context_window(),
            super::MAX_CODEX_CONTEXT_WINDOW
        );
    }

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
    fn custom_endpoint_defaults_to_256k_context_length() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "model",
            "endpoint": "https://example.com/v1",
            "apiFormat": "openai_chat"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].context_length, 256_000);
        assert_eq!(endpoints[0].replay_reasoning_content, Some(true));
        assert!(!endpoints[0].server_tools.web_search);
        assert!(!endpoints[0].supports_tool_lazy_loading);
        assert!(endpoints[0].supports_vision);
    }

    #[test]
    fn custom_endpoint_disables_tool_lazy_loading_for_all_formats() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "model",
            "endpoint": "https://example.com/v1",
            "apiFormat": "openai_responses",
            "supportsToolLazyLoading": true
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(!endpoints[0].supports_tool_lazy_loading);
    }

    #[test]
    fn custom_endpoint_preserves_server_tool_settings() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Custom",
            "apiModel": "claude-sonnet-4-20250514",
            "endpoint": "https://api.anthropic.com/v1",
            "apiFormat": "anthropic_messages",
            "serverTools": {
                "webSearch": true
            }
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(endpoints[0].server_tools.web_search);
    }

    #[test]
    fn custom_endpoint_preserves_disabled_vision_setting() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Text Only",
            "apiModel": "local-text",
            "endpoint": "http://localhost:8080/v1",
            "apiFormat": "openai_chat",
            "supportsVision": false
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert!(!endpoints[0].supports_vision);
    }

    #[test]
    fn custom_endpoint_disables_reasoning_content_replay_for_non_chat_formats() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Responses",
            "apiModel": "gpt-5.1",
            "endpoint": "https://api.openai.com/v1",
            "apiFormat": "openai_responses"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].replay_reasoning_content, Some(false));
    }

    #[test]
    fn custom_endpoint_defaults_anthropic_messages_reasoning_replay_to_disabled() {
        let raw = r#"[{
            "id": "custom-1",
            "name": "Anthropic",
            "apiModel": "claude-sonnet-4-20250514",
            "endpoint": "https://api.anthropic.com/v1",
            "apiFormat": "anthropic_messages"
        }]"#;

        let mut endpoints: Vec<CustomEndpoint> =
            serde_json::from_str(raw).expect("deserialize custom endpoint");
        normalize_custom_endpoint_config(&mut endpoints[0]);

        assert_eq!(endpoints[0].replay_reasoning_content, Some(false));
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
    fn normalize_workspace_sub_path_rejects_workspace_escapes() {
        assert_eq!(
            normalize_workspace_sub_path("Assets\\Linked\\Hero.cs").unwrap(),
            "Assets/Linked/Hero.cs"
        );
        assert!(normalize_workspace_sub_path("../Assets").is_err());
        assert!(normalize_workspace_sub_path("Assets/../ProjectSettings").is_err());
        assert!(normalize_workspace_sub_path("C:/outside").is_err());
        assert!(normalize_workspace_sub_path("C:outside").is_err());
        assert!(normalize_workspace_sub_path("/tmp/outside").is_err());
        assert!(normalize_workspace_sub_path("//server/share").is_err());
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

    #[test]
    fn configured_hidden_directories_control_workspace_listing() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("Library")).expect("create Library");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");

        let defaults = collect_dir_entries(temp.path(), "", false).expect("default listing");
        assert!(!defaults.iter().any(|entry| entry.rel_path == "Library"));

        let visible = HashSet::new();
        let configured = collect_dir_entries_with_hidden(temp.path(), "", false, Some(&visible))
            .expect("configured listing");
        assert!(configured.iter().any(|entry| entry.rel_path == "Library"));

        let hidden = HashSet::from(["src".to_string()]);
        let configured = collect_dir_entries_with_hidden(temp.path(), "", false, Some(&hidden))
            .expect("custom hidden listing");
        assert!(!configured.iter().any(|entry| entry.rel_path == "src"));
    }

    #[test]
    fn configured_workspace_search_includes_files_under_unity_roots() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("Assets/Scripts")).expect("create Assets");
        std::fs::write(temp.path().join("Assets/Scripts/Hud.cs"), "class Hud {}")
            .expect("write asset source");

        let configured = HashSet::new();
        let results =
            search_workspace_entries_in_dir_with_hidden(temp.path(), "Hud", 100, Some(&configured))
                .expect("configured search");
        assert!(results
            .iter()
            .any(|entry| entry.rel_path == "Assets/Scripts/Hud.cs" && !entry.is_dir));
    }

    #[test]
    fn workspace_entry_stat_classifies_existing_roots_files_and_missing_paths() {
        let temp = tempdir().expect("create temp dir");
        std::fs::create_dir_all(temp.path().join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(temp.path().join("Packages")).expect("create packages dir");
        std::fs::write(temp.path().join("Assets/LICENSE"), "license")
            .expect("write extensionless file");

        let cwd = temp.path().to_string_lossy();
        let assets = workspace_entry_stat_for_path(&cwd, "Assets");
        let packages = workspace_entry_stat_for_path(&cwd, "Packages/");
        let license = workspace_entry_stat_for_path(&cwd, "Assets/LICENSE");
        let missing = workspace_entry_stat_for_path(&cwd, "Assets/Missing.prefab");

        assert!(assets.exists);
        assert_eq!(assets.entry_kind, "folder");
        assert_eq!(assets.path, "Assets");
        assert!(packages.exists);
        assert_eq!(packages.entry_kind, "folder");
        assert_eq!(packages.path, "Packages");
        assert!(license.exists);
        assert_eq!(license.entry_kind, "file");
        assert_eq!(license.path, "Assets/LICENSE");
        assert!(!missing.exists);
        assert_eq!(missing.entry_kind, "missing");
    }

    #[test]
    fn directory_listing_treats_symlinked_folders_as_directories() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(external.join("Nested")).expect("create linked target");
        std::fs::write(external.join("Nested/Hero.prefab"), b"prefab").expect("write asset");

        let link = workspace.join("Assets/Linked");
        if !create_dir_symlink_or_skip(&external, &link) {
            return;
        }

        let assets_entries =
            collect_dir_entries(&workspace.join("Assets"), "Assets", true).expect("list Assets");
        assert!(assets_entries
            .iter()
            .any(|entry| entry.rel_path == "Assets/Linked" && entry.is_dir));

        let workspace_str = workspace.to_string_lossy();
        let (target, normalized) = resolve_workspace_dir_target(&workspace_str, "Assets/Linked")
            .expect("resolve symlinked folder");
        assert_eq!(normalized, "Assets/Linked");

        let linked_entries =
            collect_dir_entries(&target, &normalized, true).expect("list symlinked folder");
        assert!(linked_entries
            .iter()
            .any(|entry| entry.rel_path == "Assets/Linked/Nested" && entry.is_dir));

        let (nested_target, nested_normalized) =
            resolve_workspace_dir_target(&workspace_str, "Assets/Linked/Nested")
                .expect("resolve nested symlinked folder path");
        assert_eq!(nested_normalized, "Assets/Linked/Nested");
        assert!(nested_target.is_dir());
    }

    #[test]
    fn directory_listing_rejects_non_asset_symlinked_folders() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-docs");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Secret.txt"), b"secret").expect("write external file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Docs")) {
            return;
        }

        let workspace_str = workspace.to_string_lossy();
        assert!(resolve_workspace_dir_target(&workspace_str, "Docs").is_err());

        let root_entries = collect_dir_entries(&workspace, "", true).expect("list workspace root");
        assert!(!root_entries.iter().any(|entry| entry.rel_path == "Docs"));
    }

    #[test]
    fn workspace_search_skips_non_asset_symlinked_folders() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        let external = temp.path().join("external-docs");
        std::fs::create_dir_all(workspace.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(external.join("Secret.txt"), b"secret").expect("write external file");

        if !create_dir_symlink_or_skip(&external, &workspace.join("Docs")) {
            return;
        }

        let results =
            search_workspace_entries_in_dir(&workspace, "Secret", 100).expect("search workspace");
        assert!(!results
            .iter()
            .any(|entry| entry.rel_path == "Docs/Secret.txt"));
    }

    #[test]
    fn workspace_search_does_not_recurse_forever_through_symlink_cycle() {
        let temp = tempdir().expect("create temp dir");
        let workspace = temp.path().join("project");
        std::fs::create_dir_all(workspace.join("Assets/Real")).expect("create assets dir");
        std::fs::write(workspace.join("Assets/Real/Hero.prefab"), "prefab")
            .expect("write asset file");

        let loop_link = workspace.join("Assets/Loop");
        if !create_dir_symlink_or_skip(&workspace, &loop_link) {
            return;
        }

        let results =
            search_workspace_entries_in_dir(&workspace, "Loop", 100).expect("search workspace");
        assert!(results
            .iter()
            .any(|entry| entry.rel_path == "Assets/Loop" && entry.is_dir));
        assert!(results.len() < 20);
    }

    fn legacy_endpoint(id: &str, api_model: &str) -> CustomEndpoint {
        serde_json::from_str(&format!(
            r#"{{
                "id": "{id}",
                "name": "My DeepSeek",
                "apiModel": "{api_model}",
                "endpoint": "https://api.deepseek.com",
                "apiFormat": "openai_chat",
                "contextLength": 131072,
                "supportedReasoningEfforts": ["low", "high"],
                "replayReasoningContent": true,
                "supportsVision": false
            }}"#
        ))
        .expect("legacy endpoint json")
    }

    #[test]
    fn migrating_endpoint_keeps_id_and_model_settings() {
        let provider = migrate_endpoint_to_provider(legacy_endpoint("ep-1", "deepseek-chat"));

        assert_eq!(provider.id, "ep-1");
        assert_eq!(provider.name, "My DeepSeek");
        assert_eq!(provider.endpoint, "https://api.deepseek.com");
        assert_eq!(provider.api_format, ApiFormat::OpenaiChat);
        assert_eq!(provider.models.len(), 1);

        let model = &provider.models[0];
        assert_eq!(model.id, "deepseek-chat");
        assert_eq!(model.api_model, "deepseek-chat");
        assert_eq!(model.context_length, 131_072);
        assert_eq!(model.remote_compaction_mode, RemoteCompactionMode::Disabled);
        assert!(!model.supports_tool_lazy_loading);
        assert_eq!(
            model.supported_reasoning_efforts,
            vec!["low".to_string(), "high".to_string()]
        );
        assert_eq!(model.replay_reasoning_content, Some(true));
        assert!(model.reasoning_replay_field.is_none());
        assert!(!model.supports_vision);
    }

    #[test]
    fn migrating_endpoint_slugifies_api_model_with_slash() {
        let provider = migrate_endpoint_to_provider(legacy_endpoint("ep-2", "zai-org/GLM-5.2"));
        assert_eq!(provider.models[0].id, "zai-org-GLM-5.2");
        assert_eq!(provider.models[0].api_model, "zai-org/GLM-5.2");
    }

    #[test]
    fn v2_custom_provider_file_migrates_remote_compaction_to_explicit_disabled() {
        let mut file: CustomProvidersFile = serde_json::from_value(serde_json::json!({
            "version": 2,
            "providers": [{
                "id": "cpa",
                "name": "CPA",
                "endpoint": "http://192.168.0.2:8317/v1",
                "apiFormat": "openai_responses",
                "models": [{
                    "id": "gpt-5.6-sol",
                    "apiModel": "gpt-5.6-sol",
                    "name": "GPT-5.6 Sol",
                    "contextLength": 272000
                }]
            }]
        }))
        .expect("parse v2 custom providers");

        assert!(migrate_custom_providers_file(&mut file));
        assert_eq!(file.version, CUSTOM_PROVIDERS_FILE_VERSION);
        assert_eq!(
            file.providers[0].models[0].remote_compaction_mode,
            RemoteCompactionMode::Disabled
        );
        let persisted = serde_json::to_value(&file).expect("serialize migrated providers");
        assert_eq!(
            persisted["providers"][0]["models"][0]["remoteCompactionMode"],
            serde_json::json!("disabled")
        );
        assert!(!migrate_custom_providers_file(&mut file));
        assert_eq!(
            serde_json::to_value(RemoteCompactionMode::CodexV2).expect("serialize Codex V2 mode"),
            serde_json::json!("codex_v2")
        );
    }

    #[test]
    fn deepseek_v4_provider_forces_reasoning_replay_and_infers_its_field() {
        let mut provider: CustomProvider = serde_json::from_value(serde_json::json!({
            "id": "deepseek",
            "name": "DeepSeek",
            "endpoint": "https://api.deepseek.com/anthropic/v1",
            "apiFormat": "anthropic_messages",
            "models": [{
                "id": "deepseek-v4-pro",
                "apiModel": "deepseek-v4-pro",
                "name": "DeepSeek V4 Pro",
                "replayReasoningContent": false
            }]
        }))
        .expect("parse DeepSeek provider");

        normalize_custom_provider_config(&mut provider);

        let model = &provider.models[0];
        assert_eq!(model.replay_reasoning_content, Some(true));
        assert_eq!(
            model.reasoning_replay_field,
            Some(super::ReasoningReplayField::ReasoningContent)
        );
    }

    fn provider_with_models(id: &str, model_ids: &[&str]) -> CustomProvider {
        CustomProvider {
            id: id.to_string(),
            name: id.to_string(),
            endpoint: "https://example.com/v1".to_string(),
            api_format: ApiFormat::OpenaiChat,
            api_key: String::new(),
            catalog_id: None,
            prefix_cache_ttl_seconds: default_provider_prefix_cache_ttl_seconds(),
            models: model_ids
                .iter()
                .map(|mid| CustomProviderModel {
                    id: mid.to_string(),
                    api_model: mid.to_string(),
                    name: mid.to_string(),
                    context_length: 128_000,
                    remote_compaction_mode: RemoteCompactionMode::Disabled,
                    supports_tool_lazy_loading: false,
                    supported_reasoning_efforts: vec!["high".to_string()],
                    reasoning_param_format: None,
                    replay_reasoning_content: None,
                    reasoning_replay_field: None,
                    server_tools: Default::default(),
                    supports_vision: true,
                    catalog_model_id: None,
                })
                .collect(),
        }
    }

    #[test]
    fn normalize_provider_dedupes_row_ids_and_fills_defaults() {
        let mut provider = provider_with_models("prov", &["m", "m", ""]);
        provider.models[2].api_model = "  ".to_string();
        normalize_custom_provider_config(&mut provider);

        assert_eq!(provider.models[0].id, "m");
        assert_eq!(provider.models[1].id, "m-2");
        assert_eq!(provider.models[2].id, "model");
        assert_eq!(
            provider.models[0].reasoning_param_format,
            Some(super::CustomReasoningParamFormat::OpenaiChatReasoningEffort)
        );
        assert_eq!(provider.models[0].replay_reasoning_content, Some(true));
    }

    #[test]
    fn legacy_custom_refs_rewrite_to_first_model() {
        let providers = vec![provider_with_models("ep-1", &["chat", "reasoner"])];

        assert_eq!(
            rewrite_legacy_custom_model_ref("custom/ep-1", &providers),
            Some("custom/ep-1/chat".to_string())
        );
        // Already two-segment or unknown ids stay untouched.
        assert_eq!(
            rewrite_legacy_custom_model_ref("custom/ep-1/reasoner", &providers),
            None
        );
        assert_eq!(
            rewrite_legacy_custom_model_ref("custom/ghost", &providers),
            None
        );
        assert_eq!(
            rewrite_legacy_custom_model_ref("openrouter/claude-fable-5", &providers),
            None
        );
    }

    #[test]
    fn stale_pruning_accepts_both_legacy_and_two_segment_refs() {
        let providers = vec![provider_with_models("ep-1", &["chat"])];
        let valid = valid_custom_model_refs(&providers);

        assert!(!is_stale_custom_model_ref("custom/ep-1", &valid));
        assert!(!is_stale_custom_model_ref("custom/ep-1/chat", &valid));
        assert!(is_stale_custom_model_ref("custom/ep-1/ghost", &valid));
        assert!(is_stale_custom_model_ref("custom/ghost", &valid));
        assert!(!is_stale_custom_model_ref(
            "openrouter/claude-fable-5",
            &valid
        ));
    }
}
