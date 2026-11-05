//! `AppConfig` is the typed facade over the settings store
//! (`crate::settings`). Values live in `settings.json`; the definitions,
//! defaults and validation live in `settings/definitions.rs`. Getters and
//! setters keep their historical names so call sites stay unchanged.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::settings::{self, Key, SettingValue, SettingsStore};

const LEGACY_CONFIG_FILE_NAME: &str = settings::legacy::LEGACY_CONFIG_FILE_NAME;

/// Resource lifetime settings. Legacy count/concurrency fields are deliberately
/// ignored on load and disappear on the next save.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceServiceResourceLimits {
    /// Idle TTL for checkout-owned watchers and lazily opened indexes. The
    /// checkout runtime identity remains registered for the process lifetime.
    pub workspace_idle_timeout_secs: u64,
    /// Idle TTL for workspace services and their Locus-owned headless Editor.
    /// Interactive Editors remain under user control.
    pub service_idle_timeout_secs: u64,
    pub lsp_idle_timeout_secs: u64,
    /// Locus-launched background Blender instances across all checkouts.
    pub blender_max_background_instances: u32,
    /// Idle TTL for a Locus-launched background Blender instance.
    pub blender_background_idle_timeout_secs: u64,
}

impl Default for WorkspaceServiceResourceLimits {
    fn default() -> Self {
        Self {
            workspace_idle_timeout_secs: 600,
            service_idle_timeout_secs: 3600,
            lsp_idle_timeout_secs: 600,
            blender_max_background_instances: 2,
            blender_background_idle_timeout_secs: 600,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServiceResourceLimitFieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceServiceResourceLimitsValidationErrors {
    pub fields: Vec<WorkspaceServiceResourceLimitFieldError>,
}

impl std::fmt::Display for WorkspaceServiceResourceLimitsValidationErrors {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = self
            .fields
            .iter()
            .map(|error| format!("{}: {}", error.field, error.message))
            .collect::<Vec<_>>()
            .join("; ");
        write!(
            formatter,
            "invalid workspace service resource limits: {message}"
        )
    }
}

impl std::error::Error for WorkspaceServiceResourceLimitsValidationErrors {}

impl WorkspaceServiceResourceLimits {
    pub fn validate(&self) -> Result<(), WorkspaceServiceResourceLimitsValidationErrors> {
        let mut fields = Vec::new();

        fn require_checked_timeout(
            fields: &mut Vec<WorkspaceServiceResourceLimitFieldError>,
            field: &'static str,
            value: u64,
        ) {
            if value == 0 {
                fields.push(WorkspaceServiceResourceLimitFieldError {
                    field: field.to_string(),
                    message: "must be greater than zero".to_string(),
                });
                return;
            }

            let duration = std::time::Duration::from_secs(value);
            if value > crate::settings::MAX_IDLE_TIMEOUT_SECS as u64
                || std::time::Instant::now().checked_add(duration).is_none()
            {
                fields.push(WorkspaceServiceResourceLimitFieldError {
                    field: field.to_string(),
                    message: "exceeds the platform-supported timeout range".to_string(),
                });
            }
        }

        require_checked_timeout(
            &mut fields,
            "workspaceIdleTimeoutSecs",
            self.workspace_idle_timeout_secs,
        );
        require_checked_timeout(
            &mut fields,
            "serviceIdleTimeoutSecs",
            self.service_idle_timeout_secs,
        );
        require_checked_timeout(
            &mut fields,
            "lspIdleTimeoutSecs",
            self.lsp_idle_timeout_secs,
        );
        require_checked_timeout(
            &mut fields,
            "blenderBackgroundIdleTimeoutSecs",
            self.blender_background_idle_timeout_secs,
        );
        if !(1..=8).contains(&self.blender_max_background_instances) {
            fields.push(WorkspaceServiceResourceLimitFieldError {
                field: "blenderMaxBackgroundInstances".to_string(),
                message: "must be between 1 and 8".to_string(),
            });
        }

        if fields.is_empty() {
            Ok(())
        } else {
            Err(WorkspaceServiceResourceLimitsValidationErrors { fields })
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceServiceResourceLimitsUpdateError {
    Validation {
        fields: Vec<WorkspaceServiceResourceLimitFieldError>,
    },
    Persistence {
        message: String,
    },
}

impl WorkspaceServiceResourceLimitsUpdateError {
    pub fn validation_fields(&self) -> &[WorkspaceServiceResourceLimitFieldError] {
        match self {
            Self::Validation { fields } => fields,
            Self::Persistence { .. } => &[],
        }
    }
}

impl std::fmt::Display for WorkspaceServiceResourceLimitsUpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation { fields } => {
                let errors = WorkspaceServiceResourceLimitsValidationErrors {
                    fields: fields.clone(),
                };
                std::fmt::Display::fmt(&errors, formatter)
            }
            Self::Persistence { message } => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for WorkspaceServiceResourceLimitsUpdateError {}

impl From<WorkspaceServiceResourceLimitsValidationErrors>
    for WorkspaceServiceResourceLimitsUpdateError
{
    fn from(value: WorkspaceServiceResourceLimitsValidationErrors) -> Self {
        Self::Validation {
            fields: value.fields,
        }
    }
}

/// Default `subagent` nesting depth: the top-level agent may spawn
/// subagents, but those subagents may not spawn further subagents.
pub const DEFAULT_SUBAGENT_MAX_DEPTH: u32 = 1;
pub const SUBAGENT_MAX_DEPTH_LIMIT: u32 = 8;
/// Default cap on `subagent` calls running at the same time within one
/// top-level agent tree.
pub const DEFAULT_SUBAGENT_MAX_CONCURRENT: u32 = 3;
pub const SUBAGENT_MAX_CONCURRENT_LIMIT: u32 = 16;

fn clamp_subagent_max_depth(value: u32) -> u32 {
    value.clamp(1, SUBAGENT_MAX_DEPTH_LIMIT)
}

fn clamp_subagent_max_concurrent(value: u32) -> u32 {
    value.clamp(1, SUBAGENT_MAX_CONCURRENT_LIMIT)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AppCloseBehavior {
    #[default]
    Exit,
    MinimizeToTray,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DynamicToolLoadingMode {
    #[serde(alias = "meta-tool", alias = "meta_tool")]
    MetaTool,
    Direct,
    /// Protocol-native lazy loading: Anthropic `defer_loading` +
    /// `tool_reference`, Codex `tool_search`. Backends without a native
    /// renderer fall back to the MetaTool mechanism per request.
    #[default]
    Native,
}

/// Per-tool switches for the code-analysis tool family. Each flag controls
/// whether the tool is offered to agents at all (disabled tools are filtered
/// out of the request tool list, see
/// `AgentInstance::resolve_effective_tool_names`). `unity_analyzers` is not a
/// tool: it injects Microsoft.Unity.Analyzers into the Roslyn language server
/// so `code_diagnostics` reports Unity-specific rules (UNT*).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeAnalysisToolsConfig {
    pub code_symbol_search: bool,
    pub code_goto_definition: bool,
    pub code_find_references: bool,
    pub code_diagnostics: bool,
    pub edit_write_diagnostics: bool,
    pub code_hover: bool,
    pub unity_code_usages: bool,
    pub unity_analyzers: bool,
}

impl CodeAnalysisToolsConfig {
    pub fn from_settings(store: &SettingsStore) -> Self {
        Self {
            code_symbol_search: store.get(settings::UNITY_CODE_ANALYSIS_SYMBOL_SEARCH),
            code_goto_definition: store.get(settings::UNITY_CODE_ANALYSIS_GOTO_DEFINITION),
            code_find_references: store.get(settings::UNITY_CODE_ANALYSIS_FIND_REFERENCES),
            code_diagnostics: store.get(settings::UNITY_CODE_ANALYSIS_DIAGNOSTICS),
            edit_write_diagnostics: store.get(settings::UNITY_CODE_ANALYSIS_EDIT_WRITE_DIAGNOSTICS),
            code_hover: store.get(settings::UNITY_CODE_ANALYSIS_HOVER),
            unity_code_usages: store.get(settings::UNITY_CODE_ANALYSIS_UNITY_CODE_USAGES),
            unity_analyzers: store.get(settings::UNITY_CODE_ANALYSIS_UNITY_ANALYZERS),
        }
    }
}

impl Default for CodeAnalysisToolsConfig {
    fn default() -> Self {
        Self {
            code_symbol_search: true,
            code_goto_definition: true,
            code_find_references: true,
            code_diagnostics: false,
            edit_write_diagnostics: true,
            code_hover: false,
            unity_code_usages: true,
            unity_analyzers: true,
        }
    }
}

/// Serde form of an enum setting value (`"minimizeToTray"`).
fn enum_setting<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .expect("setting enums serialize to strings")
}

/// Reads a JSON object file that holds state or secrets next to (former)
/// settings; missing or unreadable files read as empty.
pub(crate) fn read_state_file(path: &Path) -> serde_json::Map<String, Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}')).ok())
        .and_then(|value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_default()
}

/// Updates fields of a JSON object file, keeping the other fields (an older
/// Locus still reads its settings from them).
pub(crate) fn update_state_file(
    path: &Path,
    update: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> Result<(), String> {
    let mut object = read_state_file(path);
    update(&mut object);
    let json = serde_json::to_vec_pretty(&Value::Object(object))
        .map_err(|error| format!("failed to serialize '{}': {error}", path.display()))?;
    atomic_write_config(path, &json)
}

fn parse_enum_setting<T: for<'de> Deserialize<'de> + Default>(value: String) -> T {
    serde_json::from_value(Value::String(value)).unwrap_or_default()
}

#[derive(Clone)]
pub struct AppConfig {
    store: Arc<SettingsStore>,
    /// Shared with the tracing layer so `system.debug` reaches log filtering
    /// without a settings lookup per event.
    debug_mirror: Arc<AtomicBool>,
}

impl AppConfig {
    /// Opens the process-wide settings in the persistent config directory,
    /// importing legacy files from it and from `data_dir` on first use.
    pub fn load(data_dir: &Path) -> Self {
        match crate::commands::persistent_config_dir() {
            Ok(config_dir) => Self::from_store(settings::global::open(&config_dir, data_dir)),
            Err(error) => {
                eprintln!(
                    "[Locus] failed to resolve persistent config dir, falling back to runtime storage: {error}"
                );
                Self::load_from_path(&data_dir.join(LEGACY_CONFIG_FILE_NAME))
            }
        }
    }

    /// A separate store in the directory of `legacy_config_path` (the old
    /// `config.json`), with legacy files imported from that directory.
    pub(crate) fn load_from_path(legacy_config_path: &Path) -> Self {
        let dir = legacy_config_path.parent().unwrap_or_else(|| Path::new("."));
        Self::from_store(Arc::new(settings::global::open_store(dir, dir)))
    }

    pub fn from_store(store: Arc<SettingsStore>) -> Self {
        let debug_mirror = Arc::new(AtomicBool::new(store.get(settings::SYSTEM_DEBUG)));
        Self {
            store,
            debug_mirror,
        }
    }

    pub fn settings(&self) -> &Arc<SettingsStore> {
        &self.store
    }

    /// Replaces the debug mirror with a flag owned by another component
    /// (the tracing layer) and seeds it with the persisted value.
    pub fn share_debug_flag(&mut self, flag: Arc<AtomicBool>) {
        flag.store(self.store.get(settings::SYSTEM_DEBUG), Ordering::Relaxed);
        self.debug_mirror = flag;
    }

    fn get<T: SettingValue>(&self, key: Key<T>) -> T {
        self.store.get(key)
    }

    fn set<T: SettingValue>(&self, key: Key<T>, value: T) -> Result<(), String> {
        self.store.set(key, value).map(|_| ()).map_err(String::from)
    }

    pub fn debug_enabled(&self) -> bool {
        self.debug_mirror.load(Ordering::Relaxed)
    }

    pub fn set_debug_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::SYSTEM_DEBUG, value)?;
        // The app's mirror listener does this too; stores without one
        // (tests, tools) still need the flag to follow.
        self.debug_mirror.store(value, Ordering::Relaxed);
        Ok(())
    }

    pub fn debug_flag(&self) -> Arc<AtomicBool> {
        self.debug_mirror.clone()
    }

    pub fn tool_failure_log_enabled(&self) -> bool {
        self.get(settings::SYSTEM_TOOL_FAILURE_LOG)
    }

    pub fn set_tool_failure_log_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::SYSTEM_TOOL_FAILURE_LOG, value)
    }

    pub fn session_undo_enabled(&self) -> bool {
        self.get(settings::AGENT_SESSION_UNDO)
    }

    pub fn set_session_undo_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::AGENT_SESSION_UNDO, value)
    }

    pub fn file_tool_workspace_boundary_enabled(&self) -> bool {
        self.get(settings::PERMISSIONS_FILE_WORKSPACE_BOUNDARY)
    }

    pub fn set_file_tool_workspace_boundary_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::PERMISSIONS_FILE_WORKSPACE_BOUNDARY, value)
    }

    pub fn unity_multi_agent_editor_enabled(&self) -> bool {
        self.get(settings::UNITY_MULTI_AGENT_EDITOR)
    }

    pub fn set_unity_multi_agent_editor_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_MULTI_AGENT_EDITOR, value)
    }

    pub fn unity_enabled(&self) -> bool {
        self.get(settings::UNITY_ENABLED)
    }

    pub fn set_unity_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_ENABLED, value)
    }

    pub fn blender_enabled(&self) -> bool {
        self.get(settings::BLENDER_ENABLED)
    }

    pub fn set_blender_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::BLENDER_ENABLED, value)
    }

    pub fn claude_code_cli_enabled(&self) -> bool {
        self.get(settings::PROVIDERS_CLAUDE_CODE_ENABLED)
    }

    pub fn set_claude_code_cli_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::PROVIDERS_CLAUDE_CODE_ENABLED, value)
    }

    pub fn close_behavior(&self) -> AppCloseBehavior {
        parse_enum_setting(self.get(settings::SYSTEM_CLOSE_BEHAVIOR))
    }

    pub fn set_close_behavior(&self, value: AppCloseBehavior) -> Result<(), String> {
        self.set(settings::SYSTEM_CLOSE_BEHAVIOR, enum_setting(value))
    }

    pub fn dynamic_tool_loading_mode(&self) -> DynamicToolLoadingMode {
        parse_enum_setting(self.get(settings::AGENT_TOOL_LOADING_MODE))
    }

    pub fn set_dynamic_tool_loading_mode(
        &self,
        value: DynamicToolLoadingMode,
    ) -> Result<(), String> {
        self.set(settings::AGENT_TOOL_LOADING_MODE, enum_setting(value))
    }

    pub fn anthropic_native_lazy_enabled(&self) -> bool {
        self.get(settings::PROVIDERS_ANTHROPIC_NATIVE_LAZY_LOADING)
    }

    pub fn set_anthropic_native_lazy_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::PROVIDERS_ANTHROPIC_NATIVE_LAZY_LOADING, value)
    }

    pub fn default_skill_package_namespace(&self) -> String {
        self.get(settings::KNOWLEDGE_DEFAULT_SKILL_PACKAGE_NAMESPACE)
    }

    pub fn set_default_skill_package_namespace(&self, value: String) -> Result<(), String> {
        self.set(settings::KNOWLEDGE_DEFAULT_SKILL_PACKAGE_NAMESPACE, value)
    }

    /// Persisted worktree base directory; `None` when unset.
    pub fn worktree_root(&self) -> Option<PathBuf> {
        Some(self.get(settings::WORKSPACE_WORKTREE_ROOT))
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
    }

    pub fn set_worktree_root(&self, value: Option<&Path>) -> Result<(), String> {
        self.set(
            settings::WORKSPACE_WORKTREE_ROOT,
            value
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    }

    pub fn unity_background_hook_enabled(&self) -> bool {
        self.get(settings::UNITY_CONNECTION_BACKGROUND_HOOK)
    }

    pub fn set_unity_background_hook_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_CONNECTION_BACKGROUND_HOOK, value)
    }

    pub fn unity_embed_enabled(&self) -> bool {
        self.get(settings::UNITY_EMBED_ENABLED)
    }

    pub fn set_unity_embed_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_EMBED_ENABLED, value)
    }

    /// Forces Unity embed windows off for this process only (CLI drivers).
    pub fn disable_unity_embed_for_process(&self) {
        self.store
            .set_runtime_override(settings::UNITY_EMBED_ENABLED, false);
    }

    pub fn unity_state_probe_enabled(&self) -> bool {
        self.get(settings::UNITY_CONNECTION_STATE_PROBE)
    }

    pub fn set_unity_state_probe_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_CONNECTION_STATE_PROBE, value)
    }

    pub fn csharp_lsp_enabled(&self) -> bool {
        self.get(settings::UNITY_CODE_ANALYSIS_CSHARP_LSP)
    }

    pub fn set_csharp_lsp_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_CODE_ANALYSIS_CSHARP_LSP, value)
    }

    pub fn unity_sidecar_compiler_enabled(&self) -> bool {
        self.get(settings::UNITY_COMPILE_SIDECAR)
    }

    pub fn set_unity_sidecar_compiler_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_COMPILE_SIDECAR, value)
    }

    pub fn unity_non_public_access_enabled(&self) -> bool {
        self.get(settings::UNITY_COMPILE_NON_PUBLIC_ACCESS)
    }

    pub fn set_unity_non_public_access_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_COMPILE_NON_PUBLIC_ACCESS, value)
    }

    pub fn unity_in_process_compile_fallback_enabled(&self) -> bool {
        self.get(settings::UNITY_COMPILE_IN_PROCESS_FALLBACK)
    }

    pub fn set_unity_in_process_compile_fallback_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_COMPILE_IN_PROCESS_FALLBACK, value)
    }

    pub fn unity_hot_reload_enabled(&self) -> bool {
        self.get(settings::UNITY_HOT_RELOAD_ENABLED)
    }

    pub fn set_unity_hot_reload_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_HOT_RELOAD_ENABLED, value)
    }

    pub fn unity_external_editor_default_enabled(&self) -> bool {
        self.get(settings::UNITY_EXTERNAL_EDITOR_AUTO_SELECT)
    }

    pub fn set_unity_external_editor_default_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_EXTERNAL_EDITOR_AUTO_SELECT, value)
    }

    pub fn unity_native_bridge_enabled(&self) -> bool {
        self.get(settings::UNITY_CONNECTION_NATIVE_BRIDGE)
    }

    pub fn set_unity_native_bridge_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_CONNECTION_NATIVE_BRIDGE, value)
    }

    pub fn unity_inline_force_evaluate_enabled(&self) -> bool {
        self.get(settings::UNITY_HOT_RELOAD_INLINE_FORCE_EVALUATE)
    }

    pub fn set_unity_inline_force_evaluate_enabled(&self, value: bool) -> Result<(), String> {
        self.set(settings::UNITY_HOT_RELOAD_INLINE_FORCE_EVALUATE, value)
    }

    pub fn code_analysis_tools(&self) -> CodeAnalysisToolsConfig {
        CodeAnalysisToolsConfig::from_settings(&self.store)
    }

    pub fn set_code_analysis_tools(&self, value: CodeAnalysisToolsConfig) -> Result<(), String> {
        let changes = [
            (settings::UNITY_CODE_ANALYSIS_SYMBOL_SEARCH, value.code_symbol_search),
            (settings::UNITY_CODE_ANALYSIS_GOTO_DEFINITION, value.code_goto_definition),
            (settings::UNITY_CODE_ANALYSIS_FIND_REFERENCES, value.code_find_references),
            (settings::UNITY_CODE_ANALYSIS_DIAGNOSTICS, value.code_diagnostics),
            (settings::UNITY_CODE_ANALYSIS_EDIT_WRITE_DIAGNOSTICS, value.edit_write_diagnostics),
            (settings::UNITY_CODE_ANALYSIS_HOVER, value.code_hover),
            (settings::UNITY_CODE_ANALYSIS_UNITY_CODE_USAGES, value.unity_code_usages),
            (settings::UNITY_CODE_ANALYSIS_UNITY_ANALYZERS, value.unity_analyzers),
        ]
        .map(|(key, enabled)| (key.index(), Value::Bool(enabled)));
        self.store
            .set_values(&changes)
            .map(|_| ())
            .map_err(String::from)
    }

    pub fn llm_retry_max_attempts(&self) -> u32 {
        self.get(settings::AGENT_LLM_RETRY_MAX_ATTEMPTS)
    }

    pub fn set_llm_retry_max_attempts(&self, value: u32) -> Result<(), String> {
        self.set(
            settings::AGENT_LLM_RETRY_MAX_ATTEMPTS,
            crate::llm::retry::clamp_max_retries(value),
        )
    }

    pub fn llm_strip_inline_think_tags(&self) -> bool {
        self.get(settings::AGENT_LLM_STRIP_INLINE_THINK_TAGS)
    }

    pub fn subagent_max_depth(&self) -> u32 {
        self.get(settings::AGENT_SUBAGENT_MAX_DEPTH)
    }

    pub fn set_subagent_max_depth(&self, value: u32) -> Result<(), String> {
        self.set(
            settings::AGENT_SUBAGENT_MAX_DEPTH,
            clamp_subagent_max_depth(value),
        )
    }

    pub fn subagent_max_concurrent(&self) -> u32 {
        self.get(settings::AGENT_SUBAGENT_MAX_CONCURRENT)
    }

    pub fn set_subagent_max_concurrent(&self, value: u32) -> Result<(), String> {
        self.set(
            settings::AGENT_SUBAGENT_MAX_CONCURRENT,
            clamp_subagent_max_concurrent(value),
        )
    }

    pub fn workspace_service_resource_limits(&self) -> WorkspaceServiceResourceLimits {
        WorkspaceServiceResourceLimits {
            workspace_idle_timeout_secs: self
                .get(settings::WORKSPACE_RESOURCES_WORKSPACE_IDLE_TIMEOUT_SECS),
            service_idle_timeout_secs: self
                .get(settings::WORKSPACE_RESOURCES_SERVICE_IDLE_TIMEOUT_SECS),
            lsp_idle_timeout_secs: self.get(settings::WORKSPACE_RESOURCES_LSP_IDLE_TIMEOUT_SECS),
            blender_max_background_instances: self
                .get(settings::BLENDER_RESOURCES_MAX_BACKGROUND_INSTANCES),
            blender_background_idle_timeout_secs: self
                .get(settings::BLENDER_RESOURCES_BACKGROUND_IDLE_TIMEOUT_SECS),
        }
    }

    pub fn try_workspace_service_resource_limits(
        &self,
    ) -> Result<WorkspaceServiceResourceLimits, String> {
        Ok(self.workspace_service_resource_limits())
    }

    /// Persists a complete, validated resource-policy replacement. A failed
    /// write leaves the prior values untouched.
    pub fn set_workspace_service_resource_limits(
        &self,
        value: WorkspaceServiceResourceLimits,
    ) -> Result<(), WorkspaceServiceResourceLimitsUpdateError> {
        value.validate()?;
        let changes = [
            (
                settings::WORKSPACE_RESOURCES_WORKSPACE_IDLE_TIMEOUT_SECS.index(),
                Value::from(value.workspace_idle_timeout_secs),
            ),
            (
                settings::WORKSPACE_RESOURCES_SERVICE_IDLE_TIMEOUT_SECS.index(),
                Value::from(value.service_idle_timeout_secs),
            ),
            (
                settings::WORKSPACE_RESOURCES_LSP_IDLE_TIMEOUT_SECS.index(),
                Value::from(value.lsp_idle_timeout_secs),
            ),
            (
                settings::BLENDER_RESOURCES_MAX_BACKGROUND_INSTANCES.index(),
                Value::from(value.blender_max_background_instances),
            ),
            (
                settings::BLENDER_RESOURCES_BACKGROUND_IDLE_TIMEOUT_SECS.index(),
                Value::from(value.blender_background_idle_timeout_secs),
            ),
        ];
        self.store
            .set_values(&changes)
            .map(|_| ())
            .map_err(|error| match error.code {
                "settings.persist_failed" => {
                    WorkspaceServiceResourceLimitsUpdateError::Persistence {
                        message: error.message,
                    }
                }
                _ => WorkspaceServiceResourceLimitsUpdateError::Validation {
                    fields: vec![WorkspaceServiceResourceLimitFieldError {
                        field: error
                            .key
                            .as_deref()
                            .and_then(|key| key.rsplit('.').next())
                            .unwrap_or_default()
                            .to_string(),
                        message: error.message,
                    }],
                },
            })
    }
}

pub(crate) fn atomic_write_config(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create config dir '{}': {}", parent.display(), e))?;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| format!("invalid config path '{}'", path.display()))?;
    let temp_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        file_name.to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut temp_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            format!(
                "failed to create temporary config '{}': {}",
                temp_path.display(),
                error
            )
        })?;
    if let Err(error) = temp_file
        .write_all(contents)
        .and_then(|_| temp_file.sync_all())
    {
        drop(temp_file);
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "failed to write temporary config '{}': {}",
            temp_path.display(),
            error
        ));
    }
    drop(temp_file);

    #[cfg(target_os = "windows")]
    let replace_result = {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        use windows_core::PCWSTR;

        let source = temp_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| error.to_string())
    };

    #[cfg(not(target_os = "windows"))]
    let replace_result = fs::rename(&temp_path, path).map_err(|error| error.to_string());

    replace_result.map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!("failed to replace config '{}': {}", path.display(), error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_file(dir: &Path) -> Value {
        serde_json::from_str(
            &fs::read_to_string(dir.join(settings::store::SETTINGS_FILE_NAME)).expect("settings file"),
        )
        .expect("settings json")
    }

    fn load_legacy(dir: &Path, legacy: &str) -> AppConfig {
        let config_path = dir.join(LEGACY_CONFIG_FILE_NAME);
        fs::write(&config_path, legacy).expect("legacy config");
        AppConfig::load_from_path(&config_path)
    }

    #[test]
    fn legacy_config_is_imported_once_and_never_rewritten() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = r#"{
  "api_key": "or-legacy-secret",
  "model": "legacy-model",
  "base_url": "https://example.com",
  "debug": true,
  "dynamic_tool_loading_native_migrated": true,
  "async_tasks_default_enabled_migrated": true,
  "workspace_service_ttl_hour_migrated": true
}"#;
        let config = load_legacy(temp.path(), legacy);
        assert!(config.debug_enabled());

        config.set_debug_enabled(false).expect("persist debug");
        assert_eq!(
            fs::read_to_string(temp.path().join(LEGACY_CONFIG_FILE_NAME)).unwrap(),
            legacy
        );
        let written = settings_file(temp.path()).to_string();
        assert!(!written.contains("or-legacy-secret"));
        assert!(!written.contains("legacy-model"));
        assert!(!written.contains("example.com"));

        // settings.json now wins over the legacy file.
        let reloaded = AppConfig::load_from_path(&temp.path().join(LEGACY_CONFIG_FILE_NAME));
        assert!(!reloaded.debug_enabled());
    }

    #[test]
    fn fresh_install_uses_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config = AppConfig::load_from_path(&temp.path().join(LEGACY_CONFIG_FILE_NAME));

        assert!(!config.file_tool_workspace_boundary_enabled());
        assert_eq!(config.close_behavior(), AppCloseBehavior::Exit);
        assert_eq!(config.dynamic_tool_loading_mode(), DynamicToolLoadingMode::Native);
        assert!(config.anthropic_native_lazy_enabled());
        assert!(!config.unity_multi_agent_editor_enabled());
        assert!(config.default_skill_package_namespace().is_empty());
        assert!(config.unity_sidecar_compiler_enabled());
        assert!(config.unity_non_public_access_enabled());
        assert!(!config.unity_hot_reload_enabled());
        assert!(!config.tool_failure_log_enabled());
        assert!(config.session_undo_enabled());
        assert!(!config.unity_external_editor_default_enabled());
        assert!(config.unity_native_bridge_enabled());
        assert!(config.unity_inline_force_evaluate_enabled());
        assert!(config.unity_embed_enabled());
        assert!(config.unity_background_hook_enabled());
        assert_eq!(config.code_analysis_tools(), CodeAnalysisToolsConfig::default());
        assert_eq!(config.worktree_root(), None);
        assert_eq!(
            config.llm_retry_max_attempts(),
            crate::llm::retry::DEFAULT_MAX_RETRIES
        );
        assert_eq!(config.subagent_max_depth(), DEFAULT_SUBAGENT_MAX_DEPTH);
        assert_eq!(config.subagent_max_concurrent(), DEFAULT_SUBAGENT_MAX_CONCURRENT);
        assert_eq!(
            config.workspace_service_resource_limits(),
            WorkspaceServiceResourceLimits::default()
        );
        assert_eq!(settings_file(temp.path()), serde_json::json!({ "version": settings::store::SETTINGS_VERSION }));
    }

    #[test]
    fn enum_settings_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        config
            .set_close_behavior(AppCloseBehavior::MinimizeToTray)
            .unwrap();
        config
            .set_dynamic_tool_loading_mode(DynamicToolLoadingMode::MetaTool)
            .unwrap();

        let reloaded = AppConfig::load_from_path(&path);
        assert_eq!(reloaded.close_behavior(), AppCloseBehavior::MinimizeToTray);
        assert_eq!(
            reloaded.dynamic_tool_loading_mode(),
            DynamicToolLoadingMode::MetaTool
        );
        assert_eq!(
            settings_file(temp.path())["system"]["closeBehavior"],
            "minimizeToTray"
        );
    }

    #[test]
    fn opt_outs_and_opt_ins_persist() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        config.set_anthropic_native_lazy_enabled(false).unwrap();
        config.set_unity_embed_enabled(false).unwrap();
        config.set_unity_native_bridge_enabled(false).unwrap();
        config.set_unity_background_hook_enabled(false).unwrap();
        config.set_tool_failure_log_enabled(true).unwrap();
        config.set_unity_external_editor_default_enabled(true).unwrap();
        config
            .set_default_skill_package_namespace("acme".to_string())
            .unwrap();
        let worktrees = tempfile::tempdir().expect("worktree root");
        config.set_worktree_root(Some(worktrees.path())).unwrap();
        let worktree_root = dunce::canonicalize(worktrees.path()).unwrap();

        let reloaded = AppConfig::load_from_path(&path);
        assert!(!reloaded.anthropic_native_lazy_enabled());
        assert!(!reloaded.unity_embed_enabled());
        assert!(!reloaded.unity_native_bridge_enabled());
        assert!(!reloaded.unity_background_hook_enabled());
        assert!(reloaded.tool_failure_log_enabled());
        assert!(reloaded.unity_external_editor_default_enabled());
        assert_eq!(reloaded.default_skill_package_namespace(), "acme");
        assert_eq!(reloaded.worktree_root(), Some(worktree_root));
        assert!(reloaded
            .set_worktree_root(Some(Path::new("Z:/does/not/exist")))
            .is_err());

        reloaded.set_worktree_root(None).unwrap();
        assert_eq!(AppConfig::load_from_path(&path).worktree_root(), None);
    }

    #[test]
    fn code_analysis_tools_round_trip_as_individual_settings() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        let value = CodeAnalysisToolsConfig {
            code_diagnostics: true,
            unity_analyzers: false,
            ..Default::default()
        };
        config.set_code_analysis_tools(value).unwrap();

        assert_eq!(AppConfig::load_from_path(&path).code_analysis_tools(), value);
        assert_eq!(
            settings_file(temp.path())["unity"]["codeAnalysis"]["tools"],
            serde_json::json!({ "codeDiagnostics": true, "unityAnalyzers": false })
        );
    }

    #[test]
    fn counts_persist_and_clamp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);

        config.set_llm_retry_max_attempts(0).unwrap();
        config.set_subagent_max_depth(3).unwrap();
        config.set_subagent_max_concurrent(5).unwrap();
        let reloaded = AppConfig::load_from_path(&path);
        assert_eq!(reloaded.llm_retry_max_attempts(), 0);
        assert_eq!(reloaded.subagent_max_depth(), 3);
        assert_eq!(reloaded.subagent_max_concurrent(), 5);

        config.set_llm_retry_max_attempts(999).unwrap();
        config.set_subagent_max_depth(0).unwrap();
        config.set_subagent_max_concurrent(999).unwrap();
        let reloaded = AppConfig::load_from_path(&path);
        assert_eq!(
            reloaded.llm_retry_max_attempts(),
            crate::llm::retry::MAX_RETRIES_LIMIT
        );
        assert_eq!(reloaded.subagent_max_depth(), 1);
        assert_eq!(reloaded.subagent_max_concurrent(), SUBAGENT_MAX_CONCURRENT_LIMIT);
    }

    #[test]
    fn process_override_disables_unity_embed_without_persisting() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        config.disable_unity_embed_for_process();
        assert!(!config.unity_embed_enabled());
        assert!(AppConfig::load_from_path(&path).unity_embed_enabled());
    }

    #[test]
    fn shared_debug_flag_follows_the_setting() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut config = load_legacy(
            temp.path(),
            r#"{ "debug": true, "dynamic_tool_loading_native_migrated": true }"#,
        );
        let flag = Arc::new(AtomicBool::new(false));
        config.share_debug_flag(flag.clone());
        assert!(flag.load(Ordering::Relaxed));
        config.set_debug_enabled(false).unwrap();
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[test]
    fn workspace_service_resource_limits_validation_reports_every_invalid_field() {
        let invalid = WorkspaceServiceResourceLimits {
            workspace_idle_timeout_secs: 0,
            service_idle_timeout_secs: 0,
            lsp_idle_timeout_secs: 0,
            ..Default::default()
        };

        let errors = invalid.validate().expect_err("all zero values are invalid");
        assert_eq!(
            errors
                .fields
                .iter()
                .map(|error| error.field.as_str())
                .collect::<Vec<_>>(),
            vec![
                "workspaceIdleTimeoutSecs",
                "serviceIdleTimeoutSecs",
                "lspIdleTimeoutSecs",
            ]
        );
    }

    #[test]
    fn workspace_service_resource_limits_reject_timeout_outside_platform_range() {
        let mut invalid = WorkspaceServiceResourceLimits::default();
        invalid.workspace_idle_timeout_secs = u64::MAX;

        let errors = invalid
            .validate()
            .expect_err("unrepresentable scheduler deadline must be rejected");
        assert_eq!(errors.fields.len(), 1);
        assert_eq!(errors.fields[0].field, "workspaceIdleTimeoutSecs");
        assert!(errors.fields[0].message.contains("platform-supported"));
    }

    #[test]
    fn workspace_service_resource_limits_set_is_transactional_on_persistence_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        let settings_path = temp.path().join(settings::store::SETTINGS_FILE_NAME);
        fs::remove_file(&settings_path).unwrap();
        fs::create_dir(&settings_path).expect("make the settings target unwritable");
        let before = config.workspace_service_resource_limits();
        let mut candidate = before.clone();
        candidate.lsp_idle_timeout_secs = 180;

        let error = config
            .set_workspace_service_resource_limits(candidate)
            .expect_err("directory target must reject persistence");

        assert!(matches!(
            error,
            WorkspaceServiceResourceLimitsUpdateError::Persistence { .. }
        ));
        assert_eq!(config.workspace_service_resource_limits(), before);
    }

    #[test]
    fn workspace_service_resource_limits_set_persists_complete_replacement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(LEGACY_CONFIG_FILE_NAME);
        let config = AppConfig::load_from_path(&path);
        let candidate = WorkspaceServiceResourceLimits {
            lsp_idle_timeout_secs: 180,
            service_idle_timeout_secs: 900,
            blender_max_background_instances: 4,
            ..Default::default()
        };

        config
            .set_workspace_service_resource_limits(candidate.clone())
            .expect("persist resource limits");

        assert_eq!(config.workspace_service_resource_limits(), candidate);
        assert_eq!(
            AppConfig::load_from_path(&path).workspace_service_resource_limits(),
            candidate
        );
    }
}
