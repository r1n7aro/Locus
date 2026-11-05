//! One-time import of the pre-settings files into the sparse settings
//! document: `config.json` (version 1) and the per-feature files (version 2).
//! The old files are only read; Locus never writes their settings again, so
//! an older build still finds the values as they were at migration time.

use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use super::definitions::DEFINITIONS;
use super::store::{index_of, insert_path, lookup};

/// Where the legacy files live: `config_dir` is the persistent config
/// directory, `data_dir` the (movable) storage directory.
pub struct LegacySources<'a> {
    pub config_dir: &'a Path,
    pub data_dir: &'a Path,
}

pub const LEGACY_CONFIG_FILE_NAME: &str = "config.json";
const LEGACY_MODEL_DEFAULTS_FILE_NAME: &str = "model_defaults.json";

/// `config.json` field → setting key. Retired fields (`model`, `base_url`,
/// `api_key`, `async_tasks_enabled` now that background tasks are always on,
/// the `*_migrated` markers) are intentionally absent.
const TOP_LEVEL_FIELDS: &[(&str, &str)] = &[
    ("debug", "system.debug"),
    ("tool_failure_log_enabled", "system.toolFailureLog"),
    ("close_behavior", "system.closeBehavior"),
    ("session_undo_enabled", "agent.sessionUndo"),
    ("dynamic_tool_loading_mode", "agent.tools.loadingMode"),
    ("llm_retry_max_attempts", "agent.llm.retryMaxAttempts"),
    ("llm_strip_inline_think_tags", "agent.llm.stripInlineThinkTags"),
    ("subagent_max_depth", "agent.subagent.maxDepth"),
    ("subagent_max_concurrent", "agent.subagent.maxConcurrent"),
    ("file_tool_workspace_boundary", "permissions.fileWorkspaceBoundary"),
    ("anthropic_native_lazy_enabled", "providers.anthropic.nativeLazyLoading"),
    ("claude_code_cli_enabled", "providers.claudeCode.enabled"),
    ("worktree_root", "workspace.worktree.root"),
    ("default_skill_package_namespace", "knowledge.defaultSkillPackageNamespace"),
    ("unity_enabled", "unity.enabled"),
    ("unity_embed_enabled", "unity.embed.enabled"),
    ("unity_native_bridge_enabled", "unity.connection.nativeBridge"),
    ("unity_background_hook_enabled", "unity.connection.backgroundHook"),
    ("unity_state_probe_enabled", "unity.connection.stateProbe"),
    ("unity_external_editor_default_enabled", "unity.externalEditor.autoSelect"),
    ("csharp_lsp_enabled", "unity.codeAnalysis.csharpLsp"),
    ("unity_sidecar_compiler", "unity.compile.sidecar"),
    ("unity_in_process_compile_fallback", "unity.compile.inProcessFallback"),
    ("unity_non_public_access", "unity.compile.nonPublicAccess"),
    ("unity_hot_reload", "unity.hotReload.enabled"),
    ("unity_inline_force_evaluate_enabled", "unity.hotReload.inlineForceEvaluate"),
    ("unity_multi_agent_editor_enabled", "unity.multiAgentEditor"),
    ("blender_enabled", "blender.enabled"),
];

const CODE_ANALYSIS_TOOL_FIELDS: &[(&str, &str)] = &[
    ("codeSymbolSearch", "unity.codeAnalysis.tools.codeSymbolSearch"),
    ("codeGotoDefinition", "unity.codeAnalysis.tools.codeGotoDefinition"),
    ("codeFindReferences", "unity.codeAnalysis.tools.codeFindReferences"),
    ("codeDiagnostics", "unity.codeAnalysis.tools.codeDiagnostics"),
    ("editWriteDiagnostics", "unity.codeAnalysis.tools.editWriteDiagnostics"),
    ("codeHover", "unity.codeAnalysis.tools.codeHover"),
    ("unityCodeUsages", "unity.codeAnalysis.tools.unityCodeUsages"),
    ("unityAnalyzers", "unity.codeAnalysis.tools.unityAnalyzers"),
];

const RESOURCE_LIMIT_FIELDS: &[(&str, &str)] = &[
    ("workspaceIdleTimeoutSecs", "workspace.resources.workspaceIdleTimeoutSecs"),
    ("serviceIdleTimeoutSecs", "workspace.resources.serviceIdleTimeoutSecs"),
    ("lspIdleTimeoutSecs", "workspace.resources.lspIdleTimeoutSecs"),
    ("blenderMaxBackgroundInstances", "blender.resources.maxBackgroundInstances"),
    ("blenderBackgroundIdleTimeoutSecs", "blender.resources.backgroundIdleTimeoutSecs"),
];

/// Builds the initial settings document when `settings.json` does not exist
/// yet: imports the legacy `config.json` when present (otherwise applies the
/// first-run defaults the old loader used), then the per-feature files.
pub fn seed_document(sources: &LegacySources) -> Map<String, Value> {
    let legacy_path = sources.config_dir.join(LEGACY_CONFIG_FILE_NAME);
    let mut document = match read_legacy(&legacy_path) {
        Some(legacy) => {
            println!(
                "[Locus] importing settings from legacy '{}'",
                legacy_path.display()
            );
            import(legacy, Some(sources.config_dir))
        }
        None => first_run_document(sources.config_dir),
    };
    import_feature_files(&mut document, sources);
    document
}

/// Brings a document written by an older settings version up to date.
pub fn upgrade_document(
    from_version: u64,
    document: &mut Map<String, Value>,
    sources: &LegacySources,
) {
    if from_version < 2 {
        import_feature_files(document, sources);
    }
}

fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    let content = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(object)) => Some(object),
        _ => {
            eprintln!("[Locus] legacy settings file '{}' is unreadable; skipped", path.display());
            None
        }
    }
}

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().trim_matches('"').trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Stores `value` unless the document already has a value for `key`.
fn put_missing(document: &mut Map<String, Value>, key: &str, value: Value) {
    if lookup(document, key).is_none() {
        put(document, key, value);
    }
}

/// Copies `fields` (dotted paths inside `object`) to `prefix.<field>`.
fn import_fields(
    document: &mut Map<String, Value>,
    object: &Map<String, Value>,
    prefix: &str,
    fields: &[&str],
) {
    for field in fields {
        if let Some(value) = lookup(object, field).filter(|value| !value.is_null()) {
            put_missing(document, &format!("{prefix}.{field}"), value.clone());
        }
    }
}

fn clamp_number(value: &Value, min: u64, max: u64) -> Value {
    value
        .as_u64()
        .map(|number| Value::from(number.clamp(min, max)))
        .unwrap_or_else(|| value.clone())
}

/// Settings that lived in their own files before version 2. Secrets and
/// state in the same files (MCP token, remote desktop id, Claude SSH
/// authorization) stay where they are.
fn import_feature_files(document: &mut Map<String, Value>, sources: &LegacySources) {
    let config_dir = sources.config_dir;

    if let Some(codex) = read_json_object(&config_dir.join("codex_model_config.json")) {
        import_fields(
            document,
            &codex,
            "providers.codex",
            &[
                "autoUpdateClientVersion",
                "transport",
                "generateSessionTitles",
                "autoReview",
                "useApplyPatch",
                "prefixCacheTtlSeconds",
            ],
        );
        let window = match codex.get("contextWindow").filter(|value| !value.is_null()) {
            Some(window) => Some(clamp_number(
                window,
                crate::commands::MIN_CODEX_CONTEXT_WINDOW as u64,
                crate::commands::MAX_CODEX_CONTEXT_WINDOW as u64,
            )),
            None => (codex.get("extendedContext").and_then(Value::as_bool) == Some(true))
                .then(|| Value::from(crate::commands::LEGACY_CODEX_EXTENDED_CONTEXT_WINDOW)),
        };
        if let Some(window) = window {
            put_missing(document, "providers.codex.contextWindow", window);
        }
    }

    if let Some(defaults) = read_json_object(&config_dir.join(LEGACY_MODEL_DEFAULTS_FILE_NAME)) {
        import_fields(
            document,
            &defaults,
            "models.defaults",
            &["mainModel", "planModel", "subagentEfforts", "subagentFastModes"],
        );
        if let Some(models) = defaults.get("subagentModels").and_then(Value::as_object) {
            // Single-model strings predate multi-select.
            let models = models
                .iter()
                .map(|(agent, value)| {
                    let list = match value {
                        Value::String(model) => vec![Value::from(model.as_str())],
                        Value::Array(items) => items.clone(),
                        _ => Vec::new(),
                    };
                    (agent.clone(), Value::Array(list))
                })
                .collect::<Map<_, _>>();
            put_missing(document, "models.defaults.subagentModels", Value::Object(models));
        }
    }

    if let Some(proxy) = read_json_object(&config_dir.join("proxy_config.json")) {
        import_fields(
            document,
            &proxy,
            "network.proxy",
            &["mode", "manual.httpProxy", "manual.httpsProxy", "manual.allProxy"],
        );
    }

    if let Some(server) = read_json_object(&config_dir.join("mcp_server.json")) {
        import_fields(document, &server, "mcpServer", &["enabled", "disabledTools"]);
        if let Some(port) = server.get("port").filter(|port| port.as_u64() != Some(0)) {
            put_missing(document, "mcpServer.port", port.clone());
        }
        if let Some(timeout) = server.get("callTimeoutMs") {
            put_missing(
                document,
                "mcpServer.callTimeoutMs",
                clamp_number(
                    timeout,
                    crate::mcp::server::config::MIN_CALL_TIMEOUT_MS,
                    crate::mcp::server::config::MAX_CALL_TIMEOUT_MS,
                ),
            );
        }
    }

    if let Some(remote) = read_json_object(&config_dir.join("remote_access.json")) {
        import_fields(
            document,
            &remote,
            "remoteAccess",
            &[
                "enabled",
                "port",
                "loopback",
                "interfaces",
                "hostnames",
                "requireConfirmation",
                "appDownload",
            ],
        );
        if let Some(days) = remote.get("tokenIdleDays") {
            put_missing(document, "remoteAccess.tokenIdleDays", clamp_number(days, 1, 365));
        }
    }

    if let Some(claude) = read_json_object(&config_dir.join("claude_connection.json")) {
        import_fields(
            document,
            &claude,
            "providers.claudeCode",
            &[
                "permissionMode",
                "idleCompaction",
                "telemetry",
                "autoCompactWindow",
                "prewarm",
                "poolCapacity",
                "idleMinutes",
                "remote.target",
                "remote.port",
                "remote.identityFile",
                "remote.knownHostsFile",
                "remote.cliPath",
            ],
        );
    }

    if let Some(python) =
        read_json_object(&config_dir.join(crate::python_runtime::config_file_name()))
    {
        import_fields(
            document,
            &python,
            "runtimes.python",
            &["selectedId", "selectedPath"],
        );
    }

    let data_dir = sources.data_dir;
    if let Some(git) = read_text(&data_dir.join("git_path_override.txt")) {
        put_missing(document, "runtimes.git.path", Value::from(git));
    }
    if read_text(&data_dir.join("tool_permission_mode.txt")).as_deref() == Some("ask") {
        put_missing(document, "permissions.mode", Value::from("ask"));
    }
    if let Some(permissions) = read_json_object(&data_dir.join("tool_permissions.json")) {
        let mut tools = permissions
            .iter()
            .map(|(tool, mode)| {
                let ask = mode
                    .as_str()
                    .is_some_and(|mode| mode.trim().eq_ignore_ascii_case("ask"));
                (tool.clone(), Value::from(if ask { "ask" } else { "auto" }))
            })
            .collect::<Map<_, _>>();
        // `task` was renamed to `subagent`.
        if let Some(mode) = tools.remove("task") {
            tools.entry("subagent".to_string()).or_insert(mode);
        }
        put_missing(document, "permissions.tools", Value::Object(tools));
    }
}

fn read_legacy(path: &Path) -> Option<Map<String, Value>> {
    let content = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(object)) => Some(object),
        _ => {
            eprintln!(
                "[Locus] legacy config '{}' is unreadable; starting from defaults",
                path.display()
            );
            None
        }
    }
}

fn first_run_document(config_dir: &Path) -> Map<String, Value> {
    let mut document = Map::new();
    let debug = std::env::var("LOCUS_DEBUG")
        .map(|value| value == "1" || value == "true")
        .unwrap_or(false);
    put(&mut document, "system.debug", Value::Bool(debug));
    put(
        &mut document,
        "providers.claudeCode.enabled",
        Value::Bool(legacy_claude_code_cli_enabled(config_dir)),
    );
    document
}

/// Converts a legacy `config.json` object. `config_dir` locates sibling
/// legacy files for the Claude Code CLI migration.
pub fn import(mut legacy: Map<String, Value>, config_dir: Option<&Path>) -> Map<String, Value> {
    apply_native_tool_loading_migration(&mut legacy);
    apply_workspace_service_ttl_hour_migration(&mut legacy);
    apply_claude_code_cli_enabled_migration(&mut legacy, config_dir);

    let mut document = Map::new();
    for (field, key) in TOP_LEVEL_FIELDS {
        if let Some(value) = legacy.get(*field) {
            put(&mut document, key, legacy_value(key, value));
        }
    }
    if let Some(tools) = legacy.get("code_analysis_tools").and_then(Value::as_object) {
        for (field, key) in CODE_ANALYSIS_TOOL_FIELDS {
            if let Some(value) = tools.get(*field) {
                put(&mut document, key, value.clone());
            }
        }
    }
    if let Some(limits) = legacy
        .get("workspace_service_resource_limits")
        .and_then(Value::as_object)
    {
        for (field, key) in RESOURCE_LIMIT_FIELDS {
            if let Some(value) = limits.get(*field) {
                put(&mut document, key, value.clone());
            }
        }
    }
    document
}

/// The old setters clamped these counts instead of rejecting them.
fn legacy_value(key: &str, value: &Value) -> Value {
    let clamp = |min: u64, max: u64| {
        value
            .as_u64()
            .map(|number| Value::from(number.clamp(min, max)))
            .unwrap_or_else(|| value.clone())
    };
    match key {
        "agent.llm.retryMaxAttempts" => clamp(0, crate::llm::retry::MAX_RETRIES_LIMIT as u64),
        "agent.subagent.maxDepth" => clamp(1, crate::config::SUBAGENT_MAX_DEPTH_LIMIT as u64),
        "agent.subagent.maxConcurrent" => {
            clamp(1, crate::config::SUBAGENT_MAX_CONCURRENT_LIMIT as u64)
        }
        _ => value.clone(),
    }
}

/// Stores `value` when it is valid and differs from the default.
fn put(document: &mut Map<String, Value>, key: &str, value: Value) {
    let Some(index) = index_of(key) else {
        return;
    };
    let def = &DEFINITIONS[index];
    match def.kind.normalize(&value) {
        Ok(value) if value != def.default_value() => insert_path(document, key, value),
        Ok(_) => {}
        Err(reason) => eprintln!("[Locus] skipping legacy setting '{key}': {reason}"),
    }
}

/// Before the experimental setting existed, the Claude Code CLI opt-in lived
/// in `model_defaults.json` as `claudeCodeEnabled`.
fn legacy_claude_code_cli_enabled(config_dir: &Path) -> bool {
    fs::read_to_string(config_dir.join(LEGACY_MODEL_DEFAULTS_FILE_NAME))
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|value| value.get("claudeCodeEnabled").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn marker_set(legacy: &Map<String, Value>, marker: &str) -> bool {
    legacy.get(marker).and_then(Value::as_bool) == Some(true)
}

/// The Native default reached existing installs exactly once, whatever the
/// previous value was.
fn apply_native_tool_loading_migration(legacy: &mut Map<String, Value>) {
    if !marker_set(legacy, "dynamic_tool_loading_native_migrated") {
        legacy.insert(
            "dynamic_tool_loading_mode".to_string(),
            Value::from("native"),
        );
    }
}

/// Moved the Unity service idle TTL to one hour once.
fn apply_workspace_service_ttl_hour_migration(legacy: &mut Map<String, Value>) {
    if marker_set(legacy, "workspace_service_ttl_hour_migrated") {
        return;
    }
    let limits = legacy
        .entry("workspace_service_resource_limits".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !limits.is_object() {
        *limits = Value::Object(Map::new());
    }
    limits
        .as_object_mut()
        .expect("resource limits object")
        .insert("serviceIdleTimeoutSecs".to_string(), Value::from(3600));
}

fn apply_claude_code_cli_enabled_migration(
    legacy: &mut Map<String, Value>,
    config_dir: Option<&Path>,
) {
    if !legacy.contains_key("claude_code_cli_enabled") {
        let enabled = config_dir.is_some_and(legacy_claude_code_cli_enabled);
        legacy.insert("claude_code_cli_enabled".to_string(), Value::Bool(enabled));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn import_json(value: Value) -> Value {
        Value::Object(import(value.as_object().cloned().unwrap(), None))
    }

    /// Markers that make the one-time migrations no-ops.
    fn migrated(mut value: Value) -> Value {
        let object = value.as_object_mut().unwrap();
        object.insert("dynamic_tool_loading_native_migrated".into(), json!(true));
        object.insert("workspace_service_ttl_hour_migrated".into(), json!(true));
        value
    }

    #[test]
    fn current_local_config_maps_to_sparse_settings() {
        let document = import_json(json!({
            "model": "openrouter/claude-sonnet-4.6",
            "base_url": null,
            "debug": true,
            "tool_failure_log_enabled": true,
            "session_undo_enabled": false,
            "file_tool_workspace_boundary": false,
            "async_tasks_enabled": true,
            "async_tasks_default_enabled_migrated": true,
            "unity_multi_agent_editor_enabled": false,
            "claude_code_cli_enabled": true,
            "close_behavior": "exit",
            "dynamic_tool_loading_mode": "native",
            "dynamic_tool_loading_native_migrated": true,
            "workspace_service_ttl_hour_migrated": true,
            "anthropic_native_lazy_enabled": true,
            "default_skill_package_namespace": "",
            "unity_background_hook_enabled": true,
            "unity_embed_enabled": true,
            "unity_state_probe_enabled": true,
            "csharp_lsp_enabled": false,
            "unity_sidecar_compiler": true,
            "unity_non_public_access": true,
            "unity_in_process_compile_fallback": true,
            "unity_hot_reload": false,
            "unity_external_editor_default_enabled": true,
            "unity_native_bridge_enabled": true,
            "unity_inline_force_evaluate_enabled": true,
            "code_analysis_tools": {
                "codeSymbolSearch": true, "codeGotoDefinition": true,
                "codeFindReferences": true, "codeDiagnostics": false,
                "editWriteDiagnostics": true, "codeHover": false,
                "unityCodeUsages": true, "unityAnalyzers": true
            },
            "llm_retry_max_attempts": 3,
            "llm_strip_inline_think_tags": true,
            "subagent_max_depth": 1,
            "subagent_max_concurrent": 3,
            "workspace_service_resource_limits": {
                "workspaceIdleTimeoutSecs": 600,
                "serviceIdleTimeoutSecs": 3600,
                "lspIdleTimeoutSecs": 600
            },
            "worktree_root": "J:\\locus project"
        }));
        assert_eq!(
            document,
            json!({
                "system": { "debug": true, "toolFailureLog": true },
                "agent": { "sessionUndo": false },
                "providers": { "claudeCode": { "enabled": true } },
                "unity": { "externalEditor": { "autoSelect": true } },
                "workspace": { "worktree": { "root": "J:\\locus project" } }
            })
        );
    }

    #[test]
    fn retired_fields_are_dropped() {
        let document = import_json(migrated(json!({
            "model": "legacy-model",
            "base_url": "https://example.com",
            "api_key": "secret",
            "apiKey": "secret",
            "claude_code_cli_enabled": false
        })));
        assert_eq!(document, json!({}));
    }

    #[test]
    fn tool_loading_mode_migrates_to_native_once() {
        let unmigrated = import_json(json!({
            "dynamic_tool_loading_mode": "metaTool",
            "async_tasks_default_enabled_migrated": true,
            "workspace_service_ttl_hour_migrated": true
        }));
        assert!(unmigrated.pointer("/agent/tools/loadingMode").is_none());

        let migrated_choice = import_json(migrated(json!({
            "dynamic_tool_loading_mode": "meta-tool"
        })));
        assert_eq!(
            migrated_choice.pointer("/agent/tools/loadingMode"),
            Some(&json!("metaTool"))
        );
    }

    #[test]
    fn retired_async_tasks_opt_out_is_not_imported() {
        let document = import_json(migrated(json!({
            "async_tasks_enabled": false,
            "async_tasks_default_enabled_migrated": true
        })));
        assert_eq!(document, json!({}));
    }

    #[test]
    fn service_idle_ttl_moves_to_one_hour_once() {
        let unmigrated = import_json(json!({
            "dynamic_tool_loading_native_migrated": true,
            "async_tasks_default_enabled_migrated": true,
            "workspace_service_resource_limits": { "serviceIdleTimeoutSecs": 600 }
        }));
        assert!(unmigrated
            .pointer("/workspace/resources/serviceIdleTimeoutSecs")
            .is_none());

        let kept = import_json(migrated(json!({
            "workspace_service_resource_limits": { "serviceIdleTimeoutSecs": 7200 }
        })));
        assert_eq!(
            kept.pointer("/workspace/resources/serviceIdleTimeoutSecs"),
            Some(&json!(7200))
        );
    }

    #[test]
    fn retired_resource_caps_are_ignored_and_idle_preferences_kept() {
        let document = import_json(migrated(json!({
            "workspace_service_resource_limits": {
                "maxRunningSessions": 1, "maxUnityEditors": 1,
                "workspaceIdleTimeoutSecs": 321,
                "serviceIdleTimeoutSecs": 900,
                "lspIdleTimeoutSecs": 123,
                "blenderMaxBackgroundInstances": 4
            }
        })));
        assert_eq!(
            document,
            json!({
                "workspace": { "resources": {
                    "workspaceIdleTimeoutSecs": 321,
                    "serviceIdleTimeoutSecs": 900,
                    "lspIdleTimeoutSecs": 123
                } },
                "blender": { "resources": { "maxBackgroundInstances": 4 } }
            })
        );
    }

    #[test]
    fn counts_are_clamped_like_the_old_setters() {
        let document = import_json(migrated(json!({
            "llm_retry_max_attempts": 99,
            "subagent_max_depth": 0,
            "subagent_max_concurrent": 999
        })));
        assert_eq!(document.pointer("/agent/llm/retryMaxAttempts"), Some(&json!(10)));
        assert!(document.pointer("/agent/subagent/maxDepth").is_none());
        assert_eq!(document.pointer("/agent/subagent/maxConcurrent"), Some(&json!(16)));
    }

    fn sources(dir: &Path) -> LegacySources<'_> {
        LegacySources {
            config_dir: dir,
            data_dir: dir,
        }
    }

    #[test]
    fn claude_code_cli_opt_in_is_carried_from_model_defaults() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(LEGACY_MODEL_DEFAULTS_FILE_NAME),
            r#"{ "claudeCodeEnabled": true }"#,
        )
        .unwrap();
        fs::write(temp.path().join(LEGACY_CONFIG_FILE_NAME), r#"{ "debug": false }"#).unwrap();
        let imported = Value::Object(seed_document(&sources(temp.path())));
        assert_eq!(
            imported.pointer("/providers/claudeCode/enabled"),
            Some(&json!(true))
        );

        fs::remove_file(temp.path().join(LEGACY_CONFIG_FILE_NAME)).unwrap();
        let first_run = Value::Object(seed_document(&sources(temp.path())));
        assert_eq!(
            first_run.pointer("/providers/claudeCode/enabled"),
            Some(&json!(true))
        );
    }

    #[test]
    fn feature_files_are_imported_without_secrets_or_state() {
        let config = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let write = |dir: &Path, name: &str, content: &str| fs::write(dir.join(name), content).unwrap();
        write(config.path(), "codex_model_config.json",
            r#"{ "transport": "http", "extendedContext": true, "autoReview": true, "prefixCacheTtlSeconds": 1800 }"#);
        write(config.path(), "model_defaults.json",
            r#"{ "planModel": "openai/gpt-5.5", "subagentModels": { "explore": "a/b", "general": ["c/d"] } }"#);
        write(config.path(), "proxy_config.json",
            r#"{ "mode": "manual", "manual": { "httpProxy": "http://127.0.0.1:7890", "noProxy": "x" } }"#);
        write(config.path(), "mcp_server.json",
            r#"{ "enabled": true, "port": 0, "token": "secret", "callTimeoutMs": 1 }"#);
        write(config.path(), "remote_access.json",
            r#"{ "enabled": true, "hostnames": ["pc.example.ts.net"], "desktopId": "id", "tokenIdleDays": 999 }"#);
        write(config.path(), "claude_connection.json",
            r#"{ "version": 1, "permissionMode": "bypassPermissions", "autoCompactWindow": null,
                 "remote": { "target": "me@mac", "port": 2222, "identityFile": null },
                 "authorizedTarget": "fp", "info": { "model": "m" } }"#);
        write(config.path(), "python_runtime_config.json", r#"{ "selectedId": "managed-windows-x64" }"#);
        write(data.path(), "git_path_override.txt", "\"C:/Git/bin/git.exe\"\n");
        write(data.path(), "tool_permission_mode.txt", "ask");
        write(data.path(), "tool_permissions.json", r#"{ "bash": "ASK", "task": "auto", "read": "x" }"#);

        let document = Value::Object(seed_document(&LegacySources {
            config_dir: config.path(),
            data_dir: data.path(),
        }));
        assert_eq!(document["providers"]["codex"], json!({
            "transport": "http", "contextWindow": 372000, "autoReview": true
        }));
        assert_eq!(document["models"]["defaults"], json!({
            "planModel": "openai/gpt-5.5",
            "subagentModels": { "explore": ["a/b"], "general": ["c/d"] }
        }));
        assert_eq!(document["network"]["proxy"], json!({
            "mode": "manual", "manual": { "httpProxy": "http://127.0.0.1:7890" }
        }));
        assert_eq!(document["mcpServer"], json!({ "enabled": true, "callTimeoutMs": 10000 }));
        assert_eq!(document["remoteAccess"], json!({
            "enabled": true, "hostnames": ["pc.example.ts.net"], "tokenIdleDays": 365
        }));
        assert_eq!(document["providers"]["claudeCode"]["permissionMode"], json!("bypassPermissions"));
        assert_eq!(
            document["providers"]["claudeCode"]["remote"],
            json!({ "target": "me@mac", "port": 2222 })
        );
        assert_eq!(document["runtimes"], json!({
            "python": { "selectedId": "managed-windows-x64" },
            "git": { "path": "C:/Git/bin/git.exe" }
        }));
        assert_eq!(document["permissions"], json!({
            "mode": "ask",
            "tools": { "bash": "ask", "subagent": "auto", "read": "auto" }
        }));
        let text = document.to_string();
        for secret_or_state in ["secret", "desktopId", "authorizedTarget", "noProxy"] {
            assert!(!text.contains(secret_or_state), "{secret_or_state} leaked into settings");
        }
    }

    #[test]
    fn upgrade_keeps_values_already_in_settings() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("proxy_config.json"), r#"{ "mode": "disabled" }"#).unwrap();
        let mut document = json!({ "network": { "proxy": { "mode": "manual" } } })
            .as_object()
            .cloned()
            .unwrap();
        upgrade_document(1, &mut document, &sources(temp.path()));
        assert_eq!(Value::Object(document.clone())["network"]["proxy"]["mode"], json!("manual"));

        let mut fresh = Map::new();
        upgrade_document(2, &mut fresh, &sources(temp.path()));
        assert!(fresh.is_empty(), "current documents are not re-imported");
    }

    #[test]
    fn invalid_legacy_values_are_skipped() {
        let document = import_json(migrated(json!({
            "close_behavior": "explode",
            "debug": "yes",
            "unity_hot_reload": true
        })));
        assert_eq!(document, json!({ "unity": { "hotReload": { "enabled": true } } }));
    }
}
