//! Exposed-tool inventory and execution for the MCP server.
//!
//! Only Unity-domain tools are exposed (external harnesses have their own
//! file/shell tools). Descriptions and schemas come from the same ToolDef
//! prompts the in-app agent sees; availability follows the same feature
//! gates as resolve_effective_tool_names. Editor-status mismatches are
//! resolved by switching the editor automatically (user decision: external
//! calls never block on Locus UI prompts).

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::config::McpServerSettings;
use super::http::ToolCallOutcome;
use super::protocol::ToolListing;
use crate::agent::instance::AgentInstance;
use crate::agent::workspace_execution_lock::{
    process_workspace_execution_lock, WorkspaceExecutionLockMode, WorkspaceExecutionLockOwner,
};
use crate::tool::{ToolExecutionContext, ToolRegistry, ToolResult, ToolRuntimeState};

/// Every tool the MCP server can expose, in tools/list order.
pub const EXPOSED_TOOLS: &[&str] = &[
    "unity_project_info",
    "unity_set_play_mode",
    "unity_execute",
    "unity_recompile",
    "unity_hot_reload",
    "unity_run_states",
    "unity_capture_viewport",
    "unity_get_console_log",
    "unity_test_list",
    "unity_test_run",
    "unity_asset_search",
    "unity_ref_search",
    "unity_code_usages",
    "unity_yaml_list",
    "unity_yaml_search",
    "unity_yaml_read",
    "code_symbol_search",
    "code_goto_definition",
    "code_find_references",
    "code_diagnostics",
    "code_hover",
];

const EMPTY_OBJECT_SCHEMA: &str = r#"{"type":"object","properties":{},"required":[]}"#;

const PROJECT_INFO_DESCRIPTION: &str = "Report which local Unity project the Locus MCP tools currently target: project path and name, workspace id, Unity Editor connection state and editor status, and the Locus app version. Call this first to orient, and again whenever a tool result mentions a workspace change.";

const RECOMPILE_MCP_NOTE: &str = "\n\n(MCP note: editor_status / project_path parameters are not required over MCP; the call targets the active Locus workspace and exits play mode automatically if needed.)";

/// Feature gate per tool, mirroring resolve_effective_tool_names
/// (agent/instance/mod.rs) so the external surface matches what the in-app
/// agent would get.
fn tool_available(name: &str, working_dir: Option<&str>) -> (bool, Option<String>) {
    match name {
        "code_find_references"
        | "code_goto_definition"
        | "code_symbol_search"
        | "code_diagnostics"
        | "code_hover" => {
            if !crate::csharp_lsp::is_enabled() {
                return (
                    false,
                    Some("C# language server is disabled in Locus settings".to_string()),
                );
            }
            if !crate::code_tools::tool_enabled(name) {
                return (
                    false,
                    Some("This tool is disabled in Locus code-tool settings".to_string()),
                );
            }
            (true, None)
        }
        "unity_code_usages" => {
            if crate::code_tools::tool_enabled(name) {
                (true, None)
            } else {
                (
                    false,
                    Some("This tool is disabled in Locus code-tool settings".to_string()),
                )
            }
        }
        "unity_hot_reload" => {
            if crate::unity_hotreload::is_enabled() && crate::csharp_compile::is_enabled() {
                (true, None)
            } else {
                (
                    false,
                    Some(
                        "Hot reload (or the compile server) is disabled in Locus settings"
                            .to_string(),
                    ),
                )
            }
        }
        "unity_test_list" | "unity_test_run" => {
            let Some(working_dir) = working_dir.filter(|value| !value.trim().is_empty()) else {
                return (false, Some("No Unity workspace is active".to_string()));
            };
            let status = crate::workspace::unity_test_tools_workspace_status(working_dir);
            if !status.enabled {
                return (
                    false,
                    Some("Unity Test tools are disabled for this workspace".to_string()),
                );
            }
            if !status.package_installed {
                return (
                    false,
                    Some("Unity Test Framework is not installed in this project".to_string()),
                );
            }
            (true, None)
        }
        _ => (true, None),
    }
}

fn active_workspace_path(app: &AppHandle) -> Option<String> {
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    workspace
        .path
        .try_read()
        .ok()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty())
}

fn empty_object_schema() -> Value {
    serde_json::from_str(EMPTY_OBJECT_SCHEMA).expect("static schema parses")
}

fn listing_for(registry: &ToolRegistry, name: &str) -> Option<ToolListing> {
    match name {
        "unity_project_info" => Some(ToolListing {
            name: name.to_string(),
            description: PROJECT_INFO_DESCRIPTION.to_string(),
            input_schema: empty_object_schema(),
        }),
        "unity_recompile" => {
            let (description, _schema) = registry.tool_description(name)?;
            Some(ToolListing {
                name: name.to_string(),
                description: format!("{description}{RECOMPILE_MCP_NOTE}"),
                input_schema: empty_object_schema(),
            })
        }
        _ => {
            let (description, input_schema) = registry.tool_description(name)?;
            Some(ToolListing {
                name: name.to_string(),
                description,
                input_schema,
            })
        }
    }
}

/// Tools currently visible to external harnesses (enabled + feature-gated).
pub fn listed_tools(app: &AppHandle, settings: &McpServerSettings) -> Vec<ToolListing> {
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
    let working_dir = active_workspace_path(app);
    EXPOSED_TOOLS
        .iter()
        .filter(|name| settings.tool_enabled(name))
        .filter(|name| tool_available(name, working_dir.as_deref()).0)
        .filter_map(|name| listing_for(&registry, name))
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposedToolInfo {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub available: bool,
    pub unavailable_reason: Option<String>,
}

/// Full inventory for the settings page (includes disabled/unavailable rows).
pub fn exposed_tool_inventory(app: &AppHandle) -> Vec<ExposedToolInfo> {
    let settings = super::config::load_settings();
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
    let working_dir = active_workspace_path(app);
    EXPOSED_TOOLS
        .iter()
        .map(|name| {
            let (available, unavailable_reason) = tool_available(name, working_dir.as_deref());
            let description = match *name {
                "unity_project_info" => PROJECT_INFO_DESCRIPTION.to_string(),
                _ => registry
                    .tool_description(name)
                    .map(|(description, _)| description)
                    .unwrap_or_default(),
            };
            let description = first_sentence(&description, 160);
            ExposedToolInfo {
                name: name.to_string(),
                description,
                enabled: settings.tool_enabled(name),
                available,
                unavailable_reason,
            }
        })
        .collect()
}

fn first_sentence(text: &str, max_chars: usize) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    if first_line.chars().count() <= max_chars {
        return first_line.to_string();
    }
    let truncated: String = first_line.chars().take(max_chars).collect();
    format!("{truncated}…")
}

/// Instructions surfaced in the MCP initialize response so the external
/// harness knows which project it is driving.
pub async fn build_instructions(app: &AppHandle) -> String {
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    let path = workspace.path.read().await.trim().to_string();
    if path.is_empty() {
        return "Locus is running but no Unity project workspace is currently open in the Locus app. Ask the user to open one in Locus, then call unity_project_info to confirm.".to_string();
    }
    let (connected, status, _scene) = crate::unity_bridge::query_unity_status(&path).await;
    let connected_desc = if connected {
        "connected"
    } else {
        "not connected"
    };
    format!(
        "Locus exposes Unity-editor tools for the Unity project currently active in the Locus app.\nActive project: {path}\nUnity Editor: {connected_desc} (status: {status})\nThe active project can only be switched inside the Locus app. If a tool result mentions a workspace change, call unity_project_info before continuing."
    )
}

fn err(message: &str) -> ToolResult {
    ToolResult {
        output: message.to_string(),
        is_error: true,
    }
}

fn ok(message: &str) -> ToolResult {
    ToolResult {
        output: message.to_string(),
        is_error: false,
    }
}

/// Queries the editor and switches it to `requested` when needed. External
/// calls are pre-authorized to change editor status (user decision), so no
/// confirmation prompt is involved.
async fn ensure_editor_status(working_dir: &str, requested: &str) -> Result<(), String> {
    let (connected, actual, _scene) = crate::unity_bridge::query_unity_status(working_dir).await;
    if !connected {
        return Err("Unity Editor not connected".to_string());
    }
    if actual == requested {
        return Ok(());
    }
    crate::unity_bridge::set_editor_status(working_dir, requested)
        .await
        .map_err(|e| format!("Failed to change Unity Editor status: {e}"))
}

fn validated_requested_status(args: &Value) -> Result<String, ToolResult> {
    let requested = args
        .get("request_editor_status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| err("Missing required parameter: request_editor_status"))?;
    if requested == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
        || !crate::unity_bridge::is_known_editor_status(requested)
    {
        return Err(err(&format!(
            "Invalid request_editor_status: '{requested}'. Allowed values: editing, playing, playing_paused."
        )));
    }
    Ok(requested.to_string())
}

/// unity_execute over MCP: same validation as the registry closure, but a
/// status mismatch switches the editor instead of erroring.
async fn run_unity_execute(working_dir: &str, args: &Value) -> ToolResult {
    let Some(code) = args.get("code").and_then(Value::as_str) else {
        return err("Missing required parameter: code");
    };
    let requested = match validated_requested_status(args) {
        Ok(status) => status,
        Err(result) => return result,
    };
    let enable_non_public_access = match crate::csharp_compile::resolve_tool_non_public_access(args)
    {
        Ok(value) => value,
        Err(error) => return err(&error),
    };
    if let Err(e) = ensure_editor_status(working_dir, &requested).await {
        return err(&e);
    }
    match crate::unity_bridge::unity_execute_code_with_non_public_access(
        working_dir,
        code,
        enable_non_public_access,
    )
    .await
    {
        Ok(output) => {
            let trimmed = output.trim();
            ok(if trimmed.is_empty() {
                "Code executed successfully (no output)."
            } else {
                trimmed
            })
        }
        Err(e) => err(&e),
    }
}

/// unity_recompile over MCP mirrors the agent-loop semantics
/// (execute_unity_recompile): no parameters, auto-exit play mode.
async fn run_unity_recompile(working_dir: &str) -> ToolResult {
    let (connected, status, _scene) = crate::unity_bridge::query_unity_status(working_dir).await;
    if !connected {
        return err("Unity Editor not connected");
    }
    if crate::unity_bridge::is_play_mode_status(status) {
        if let Err(e) = crate::unity_bridge::exit_play_mode(working_dir).await {
            return err(&format!("Failed to exit play mode: {e}"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    match crate::unity_bridge::recompile_and_wait(working_dir).await {
        Ok(msg) => ok(&msg),
        Err(e) => err(&format!("Compilation failed:\n{e}")),
    }
}

/// unity_run_states over MCP mirrors execute_unity_run_states minus the
/// progress events and the interactive status-change confirmation.
async fn run_unity_run_states(working_dir: &str, args: &Value) -> ToolResult {
    let requested = match validated_requested_status(args) {
        Ok(status) => status,
        Err(result) => return result,
    };
    let enable_non_public_access = match crate::csharp_compile::resolve_tool_non_public_access(args)
    {
        Ok(value) => value,
        Err(error) => return err(&error),
    };
    let (connected, _status, _scene) = crate::unity_bridge::query_unity_status(working_dir).await;
    if !connected {
        return err("Unity Editor not connected");
    }
    if let Err(e) = crate::unity_bridge::compile_run_states_with_non_public_access(
        working_dir,
        args,
        enable_non_public_access,
    )
    .await
    {
        return err(&e);
    }
    if let Err(e) = ensure_editor_status(working_dir, &requested).await {
        return err(&e);
    }
    match crate::unity_bridge::unity_run_states_with_non_public_access(
        working_dir,
        args,
        enable_non_public_access,
    )
    .await
    {
        Ok(output) => {
            if output.trim().is_empty() {
                ok("unity_run_states completed with no output.")
            } else {
                ok(output.trim())
            }
        }
        Err(e) => err(&e),
    }
}

async fn project_info(app: &AppHandle) -> ToolResult {
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    let path = workspace.path.read().await.trim().to_string();
    if path.is_empty() {
        return ToolResult {
            output: serde_json::to_string_pretty(&json!({
                "workspace_open": false,
                "message": "No Unity project workspace is open in Locus. Ask the user to open one in the Locus app.",
                "locus_version": env!("CARGO_PKG_VERSION"),
            }))
            .unwrap_or_default(),
            is_error: false,
        };
    }
    let workspace_id = workspace.workspace_id.read().await.clone();
    let (connected, status, scene) = crate::unity_bridge::query_unity_status(&path).await;
    let project_name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    ToolResult {
        output: serde_json::to_string_pretty(&json!({
            "workspace_open": true,
            "project_path": path,
            "project_name": project_name,
            "workspace_id": workspace_id,
            "unity_editor": {
                "connected": connected,
                "editor_status": status,
                "scene": scene,
            },
            "locus_version": env!("CARGO_PKG_VERSION"),
            "note": "All Locus MCP tools operate on this project. The active project can only be switched inside the Locus app.",
        }))
        .unwrap_or_default(),
        is_error: false,
    }
}

fn outcome_from_tool_result(result: ToolResult, workspace_path: Option<String>) -> ToolCallOutcome {
    ToolCallOutcome {
        output: result.output,
        is_error: result.is_error,
        images: Vec::new(),
        workspace_path,
    }
}

/// Executes one exposed tool against the active workspace. `timeout_ms` and
/// `runtime_state` come from the server lifecycle (snapshotted at listener
/// start; settings changes apply on restart).
pub async fn execute_tool(
    app: AppHandle,
    name: String,
    arguments: Value,
    timeout_ms: u64,
    runtime_state: Arc<ToolRuntimeState>,
) -> ToolCallOutcome {
    let started = Instant::now();
    let workspace = app.state::<Arc<crate::workspace::Workspace>>();
    let working_dir = workspace.path.read().await.trim().to_string();
    let workspace_path = if working_dir.is_empty() {
        None
    } else {
        Some(working_dir.clone())
    };

    if name == "unity_project_info" {
        let result = project_info(&app).await;
        return outcome_from_tool_result(result, workspace_path);
    }
    if working_dir.is_empty() {
        return outcome_from_tool_result(
            err("No Unity project workspace is open in Locus. Open one in the Locus app first."),
            None,
        );
    }

    let request_run_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let fut = async {
        let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
        let lock_mode = if registry.mutates_workspace(&name)
            || AgentInstance::is_unity_execution_barrier_tool(&name)
        {
            WorkspaceExecutionLockMode::Write
        } else {
            WorkspaceExecutionLockMode::Read
        };
        let owner = WorkspaceExecutionLockOwner {
            session_id: "mcp-server".to_string(),
            run_id: request_run_id,
            iteration: 0,
            workspace: working_dir.clone(),
            tools: vec![name.clone()],
        };
        // The sender stays alive for the acquisition lifetime. If the outer
        // timeout drops this future, waiter registration and any acquired
        // guard are both released by Drop and leave an abandoned/released log.
        let (_lock_cancel_tx, lock_cancel_rx) = tokio::sync::watch::channel(false);
        let workspace_guard = match process_workspace_execution_lock()
            .acquire(lock_mode, owner, lock_cancel_rx)
            .await
        {
            Ok(guard) => guard,
            Err(_) => {
                return outcome_from_tool_result(
                    err(&format!(
                        "Tool '{name}' was cancelled while waiting for the workspace lock."
                    )),
                    None,
                );
            }
        };
        let outcome =
            execute_workspace_tool(&app, &name, &arguments, &working_dir, runtime_state).await;
        drop(workspace_guard);
        outcome
    };
    let outcome = match tokio::time::timeout(Duration::from_millis(timeout_ms), fut).await {
        Ok(outcome) => outcome,
        Err(_) => outcome_from_tool_result(
            err(&format!(
                "Tool '{name}' timed out after {}s in Locus.",
                timeout_ms / 1000
            )),
            workspace_path.clone(),
        ),
    };
    eprintln!(
        "[McpServer] tools/call {name} -> {} ({}ms)",
        if outcome.is_error { "error" } else { "ok" },
        started.elapsed().as_millis()
    );
    ToolCallOutcome {
        workspace_path,
        ..outcome
    }
}

async fn execute_workspace_tool(
    app: &AppHandle,
    name: &str,
    arguments: &Value,
    working_dir: &str,
    runtime_state: Arc<ToolRuntimeState>,
) -> ToolCallOutcome {
    match name {
        "unity_execute" => {
            outcome_from_tool_result(run_unity_execute(working_dir, arguments).await, None)
        }
        "unity_recompile" => outcome_from_tool_result(run_unity_recompile(working_dir).await, None),
        "unity_run_states" => {
            outcome_from_tool_result(run_unity_run_states(working_dir, arguments).await, None)
        }
        "unity_capture_viewport" => {
            // Auto-switch first when the caller requested a specific editor
            // status; the shared implementation then re-checks and matches.
            if let Ok(requested) = validated_requested_status(arguments) {
                if let Err(e) = ensure_editor_status(working_dir, &requested).await {
                    return outcome_from_tool_result(err(&e), None);
                }
            }
            let (output, is_error, images) =
                AgentInstance::execute_unity_capture_viewport(working_dir, arguments)
                    .await
                    .into_output_parts();
            ToolCallOutcome {
                output,
                is_error,
                images: images
                    .unwrap_or_default()
                    .into_iter()
                    .map(|image| (image.data, image.mime_type))
                    .collect(),
                workspace_path: None,
            }
        }
        "unity_ref_search" => outcome_from_tool_result(
            AgentInstance::execute_unity_ref_search(app, arguments),
            None,
        ),
        "unity_asset_search" => outcome_from_tool_result(
            AgentInstance::execute_unity_asset_search(app, arguments),
            None,
        ),
        "unity_yaml_list" => outcome_from_tool_result(
            AgentInstance::execute_unity_yaml_list(app, working_dir, arguments).await,
            None,
        ),
        "unity_yaml_search" => outcome_from_tool_result(
            AgentInstance::execute_unity_yaml_search(app, working_dir, arguments).await,
            None,
        ),
        "unity_yaml_read" => outcome_from_tool_result(
            AgentInstance::execute_unity_yaml_read(app, working_dir, arguments).await,
            None,
        ),
        // unity_hot_reload / unity_code_usages / code_* have real registry
        // closures; run them through the shared registry path.
        _ => {
            let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
            let context = ToolExecutionContext {
                app_handle: Some(app.clone()),
                working_dir: Some(working_dir.to_string()),
                // Registry tools that need Unity perform their own authoritative
                // request. Eagerly probing here duplicated status traffic and
                // could race the real request; this field is only consumed by
                // the built-in file reader's Unity-YAML redirect.
                unity_connected: None,
                runtime_state: Some(runtime_state),
                cancel_rx: None,
                progress: None,
            };
            let result = registry
                .execute_with_context(name, arguments, context)
                .await;
            outcome_from_tool_result(result, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposed_tools_are_unique_and_lead_with_project_info() {
        let mut seen = std::collections::HashSet::new();
        for name in EXPOSED_TOOLS {
            assert!(seen.insert(*name), "duplicate exposed tool {name}");
        }
        assert_eq!(EXPOSED_TOOLS[0], "unity_project_info");
    }

    #[test]
    fn first_sentence_truncates_long_lines() {
        assert_eq!(first_sentence("short line\nrest", 160), "short line");
        let long = "x".repeat(200);
        let truncated = first_sentence(&long, 160);
        assert!(truncated.chars().count() <= 161);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn empty_schema_is_valid_object() {
        let schema = empty_object_schema();
        assert_eq!(schema["type"], "object");
    }
}
