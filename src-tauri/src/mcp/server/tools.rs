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
    process_workspace_execution_lock, WorkspaceExecutionLockOwner, WorkspaceExecutionLockRequest,
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
    "unity_yaml_search",
    "unity_yaml_read",
    "code_symbol_search",
    "code_goto_definition",
    "code_find_references",
    "code_diagnostics",
    "code_hover",
];

const EMPTY_OBJECT_SCHEMA: &str = r#"{"type":"object","properties":{},"required":[]}"#;
const MCP_SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(45);

const PROJECT_INFO_DESCRIPTION: &str = "Report which local Unity project the Locus MCP tools currently target: project path and name, workspace id, Unity Editor connection state and editor status, and the Locus app version. Call this first to orient, and again whenever a tool result mentions a workspace change.";

const RECOMPILE_MCP_NOTE: &str = "\n\n(MCP note: editor_status / project_path parameters are not required over MCP; the call targets this session's bound checkout and exits play mode automatically if needed.)";

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
            if !status.package_supported {
                return (
                    false,
                    Some(format!(
                        "Unity Test tools require com.unity.test-framework {} or newer (found {})",
                        crate::workspace::UNITY_TEST_FRAMEWORK_MIN_VERSION,
                        status
                            .package_version
                            .as_deref()
                            .unwrap_or("unknown version")
                    )),
                );
            }
            (true, None)
        }
        _ => (true, None),
    }
}

fn scoped_workspace_path(
    app: &AppHandle,
    workspace_ref: &crate::workspace_service::WorkspaceRef,
) -> Option<String> {
    app.state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .resolve_workspace_ref(workspace_ref)
        .ok()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string())
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
pub fn listed_tools(
    app: &AppHandle,
    settings: &McpServerSettings,
    workspace_ref: &crate::workspace_service::WorkspaceRef,
) -> Vec<ToolListing> {
    let registry = app.state::<Arc<ToolRegistry>>().inner().clone();
    let working_dir = scoped_workspace_path(app, workspace_ref);
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
    EXPOSED_TOOLS
        .iter()
        .map(|name| {
            // Inventory is app-level. Checkout-specific gates are evaluated by
            // tools/list after a session has an immutable checkout binding.
            let (available, unavailable_reason) = match *name {
                "unity_test_list" | "unity_test_run" => (true, None),
                _ => tool_available(name, None),
            };
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
pub async fn build_instructions(
    app: &AppHandle,
    workspace_ref: &crate::workspace_service::WorkspaceRef,
) -> String {
    let Some(path) = scoped_workspace_path(app, workspace_ref) else {
        return format!(
            "This Locus MCP connection is bound to checkout '{}' generation {:?}, but that runtime is no longer available. Reconnect to obtain a current checkout binding.",
            workspace_ref.checkout_id, workspace_ref.expected_generation
        );
    };
    let (connected, status, _scene) = crate::unity_bridge::query_unity_status(&path).await;
    let connected_desc = if connected {
        "connected"
    } else {
        "not connected"
    };
    format!(
        "Locus exposes Unity-editor tools for the checkout bound when this MCP server started.\nBound project: {path}\nUnity Editor: {connected_desc} (status: {status})\nThis connection keeps that checkout identity for its lifetime. Call unity_project_info before continuing after a reconnect."
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
    match crate::code_tools::recompile_with_semantic_warnings(working_dir).await {
        Ok(msg) => ok(&msg),
        Err(e) => err(&e),
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

async fn project_info(
    path: Option<&str>,
    project_id: Option<&str>,
    checkout_id: Option<&str>,
    workspace_generation: Option<u64>,
) -> ToolResult {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return ToolResult {
            output: serde_json::to_string_pretty(&json!({
                "workspace_open": false,
                "message": "No Unity project workspace is open in Locus. Ask the user to open one in the Locus app.",
                "locus_version": env!("CARGO_PKG_VERSION"),
            }))
            .unwrap_or_default(),
            is_error: false,
        };
    };
    let (connected, status, scene) = crate::unity_bridge::query_unity_status(path).await;
    let project_name = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    ToolResult {
        output: serde_json::to_string_pretty(&json!({
            "workspace_open": true,
            "project_path": path,
            "project_name": project_name,
            "project_id": project_id,
            "checkout_id": checkout_id,
            "workspace_generation": workspace_generation,
            "unity_editor": {
                "connected": connected,
                "editor_status": status,
                "scene": scene,
            },
            "locus_version": env!("CARGO_PKG_VERSION"),
            "note": "This tool call is bound to the checkout identity shown above.",
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

/// Executes one exposed tool against the MCP session's immutable checkout
/// binding. `timeout_ms` and `runtime_state` come from the process-level
/// listener lifecycle (settings changes apply on restart).
pub async fn execute_tool(
    app: AppHandle,
    name: String,
    arguments: Value,
    timeout_ms: u64,
    runtime_state: Arc<ToolRuntimeState>,
    workspace_ref: crate::workspace_service::WorkspaceRef,
) -> ToolCallOutcome {
    let started = Instant::now();
    let workspace_registry = app
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let workspace_scope = match workspace_registry.resolve_workspace_ref(&workspace_ref) {
        Ok(scope) => scope,
        Err(error) => {
            return outcome_from_tool_result(
                err(&format!("Workspace scope resolution failed: {error}")),
                None,
            )
        }
    };
    let working_dir = workspace_scope
        .runtime()
        .root()
        .to_string_lossy()
        .to_string();
    let project_id = workspace_scope.runtime().project_id().to_string();
    let checkout_id = workspace_scope.runtime().checkout_id().to_string();
    let workspace_generation = workspace_scope.runtime().generation();
    let workspace_path = Some(working_dir.clone());

    if name == "unity_project_info" {
        let result = project_info(
            workspace_path.as_deref(),
            Some(&project_id),
            Some(&checkout_id),
            Some(workspace_generation),
        )
        .await;
        return outcome_from_tool_result(result, workspace_path);
    }
    let requested_services = crate::workspace_service::service::owner_service_for_tool(&name)
        .into_iter()
        .collect::<Vec<_>>();
    let execution = match workspace_registry
        .execution_context(workspace_scope.runtime().checkout_id(), &requested_services)
        .await
    {
        Ok(execution) => execution,
        Err(error) => {
            return outcome_from_tool_result(
                err(&format!("Workspace service binding failed: {error}")),
                workspace_path,
            )
        }
    };
    if execution.workspace.generation() != workspace_generation {
        return outcome_from_tool_result(
            err(&format!(
                "Workspace scope resolution failed: checkout {checkout_id} moved from generation {workspace_generation} to {}",
                execution.workspace.generation()
            )),
            workspace_path,
        );
    }
    let definitions = match app
        .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
        .snapshot(execution.workspace.as_ref())
        .await
    {
        Ok(definitions) => definitions,
        Err(error) => {
            return outcome_from_tool_result(
                err(&format!(
                    "Failed to resolve checkout Agent definitions: {error}"
                )),
                workspace_path,
            )
        }
    };
    let tool_registry = match app
        .state::<Arc<crate::workspace_tool_registry::WorkspaceToolRegistry>>()
        .snapshot(execution.workspace.as_ref(), definitions.as_ref())
        .await
    {
        Ok(registry) => registry,
        Err(error) => {
            return outcome_from_tool_result(
                err(&format!(
                    "Failed to resolve checkout tool registry: {error}"
                )),
                workspace_path,
            )
        }
    };

    let request_run_id = format!("mcp-{}", uuid::Uuid::new_v4());
    let fut = async {
        let lock_request = if name == "unity_execute" {
            (!AgentInstance::unity_execute_is_readonly(&arguments))
                .then_some(WorkspaceExecutionLockRequest::Exclusive)
        } else if tool_registry.mutates_workspace(&name)
            || AgentInstance::is_unity_execution_barrier_tool(&name)
        {
            Some(WorkspaceExecutionLockRequest::Exclusive)
        } else {
            None
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
        let workspace_guard = if let Some(request) = lock_request {
            let workspace_event_scope =
                crate::workspace_service::event::WorkspaceEventScope::for_runtime(
                    execution.workspace.as_ref(),
                );
            match process_workspace_execution_lock(&working_dir)
                .acquire_with_diagnostics(
                    request,
                    owner,
                    lock_cancel_rx,
                    workspace_event_scope,
                    &app,
                )
                .await
            {
                Ok(guard) => Some(guard),
                Err(_) => {
                    return outcome_from_tool_result(
                        err(&format!(
                            "Tool '{name}' was cancelled while waiting for workspace mutation coordination."
                        )),
                        None,
                    );
                }
            }
        } else {
            None
        };
        let outcome = execute_workspace_tool(
            &app,
            &name,
            &arguments,
            &working_dir,
            execution,
            tool_registry,
            runtime_state,
        )
        .await;
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
    execution: Arc<crate::workspace_service::AgentExecutionContext>,
    tool_registry: Arc<ToolRegistry>,
    runtime_state: Arc<ToolRuntimeState>,
) -> ToolCallOutcome {
    let requires_ready = crate::workspace_service::service::service_ready_required_for_tool(name);
    let unity_owned = crate::workspace_service::service::owner_service_for_tool(name)
        == Some(crate::workspace_service::ServiceKind::Unity);
    if requires_ready && unity_owned {
        if let Some(error) =
            crate::unity_bridge::dialog::blocked_error(working_dir, "not_sent", None)
        {
            return outcome_from_tool_result(err(&error), None);
        }
    }
    let _service_lease = match resolve_owned_service_for_mcp_tool(
        execution.as_ref(),
        name,
        MCP_SERVICE_READY_TIMEOUT,
    )
    .await
    {
        Ok(binding) => binding,
        Err(error) => {
            if requires_ready && unity_owned {
                if let Some(dialog_error) =
                    crate::unity_bridge::dialog::blocked_error(working_dir, "not_sent", None)
                {
                    return outcome_from_tool_result(err(&dialog_error), None);
                }
            }
            let output = format!("Tool '{name}' service binding error: {error}");
            let output = if unity_owned {
                crate::unity_bridge::enrich_unity_tool_error(working_dir, &output).await
            } else {
                output
            };
            return outcome_from_tool_result(err(&output), None);
        }
    };
    let mut outcome = match name {
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
        "unity_ref_search" => {
            execution
                .workspace
                .core()
                .refresh_asset_db_if_missing(execution.root());
            outcome_from_tool_result(
                AgentInstance::execute_unity_ref_search(
                    arguments,
                    execution.workspace.core().asset_db(),
                ),
                None,
            )
        }
        "unity_asset_search" => {
            execution
                .workspace
                .core()
                .refresh_asset_db_if_missing(execution.root());
            outcome_from_tool_result(
                AgentInstance::execute_unity_asset_search(
                    arguments,
                    execution.workspace.core().asset_db(),
                ),
                None,
            )
        }
        "unity_yaml_search" => {
            execution
                .workspace
                .core()
                .refresh_asset_db_if_missing(execution.root());
            outcome_from_tool_result(
                AgentInstance::execute_unity_yaml_search(
                    app,
                    working_dir,
                    execution.workspace.core().asset_db(),
                    arguments,
                )
                .await,
                None,
            )
        }
        "unity_yaml_read" => {
            execution
                .workspace
                .core()
                .refresh_asset_db_if_missing(execution.root());
            outcome_from_tool_result(
                AgentInstance::execute_unity_yaml_read(
                    app,
                    working_dir,
                    execution.workspace.core().asset_db(),
                    arguments,
                )
                .await,
                None,
            )
        }
        // unity_hot_reload / unity_code_usages / code_* have real registry
        // closures; run them through the shared registry path.
        _ => {
            let context = ToolExecutionContext {
                app_handle: Some(app.clone()),
                execution: Some(execution),
                working_dir: Some(working_dir.to_string()),
                process_owner: Some(crate::process_util::ProcessOwner {
                    working_dir: Some(working_dir.to_string()),
                    ..Default::default()
                }),
                // Registry tools that need Unity perform their own authoritative
                // request. Eagerly probing here duplicated status traffic and
                // could race the real request; this field is only consumed by
                // the built-in file reader's Unity-YAML redirect.
                unity_connected: None,
                runtime_state: Some(runtime_state),
                cancel_rx: None,
                progress: None,
                output: None,
                output_path: None,
                background: false,
            };
            let result = tool_registry
                .execute_with_context(name, arguments, context)
                .await;
            outcome_from_tool_result(result, None)
        }
    };
    if unity_owned && outcome.is_error {
        outcome.output =
            crate::unity_bridge::enrich_unity_tool_error(working_dir, &outcome.output).await;
    }
    outcome
}

async fn resolve_owned_service_for_mcp_tool(
    execution: &crate::workspace_service::AgentExecutionContext,
    name: &str,
    ready_timeout: Duration,
) -> Result<
    Option<crate::workspace_service::service::ResolvedServiceBinding>,
    crate::workspace_service::service::ServiceBindingError,
> {
    let Some(owner) = crate::workspace_service::service::owner_service_for_tool(name) else {
        return Ok(None);
    };
    let binding = if crate::workspace_service::service::service_ready_required_for_tool(name) {
        execution
            .resolve_service_ready(owner, ready_timeout)
            .await?
    } else {
        execution.resolve_service(owner)?
    };
    Ok(Some(binding))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::resource_policy::ResourcePolicyStore;
    use crate::workspace_service::service::{
        DetectionResult, PromptFragment, ServiceActivationPolicy, ServiceBindingError,
        ServiceCapabilities, ServiceContextProvider, ServiceFuture, ServiceLeaseTracker,
        ServiceReadinessError, ServiceReadinessGate, ServiceReadinessPhase,
        ServiceReadinessSnapshot, ServiceRuntimeIdentity, ServiceStatus, ServiceToolDefinition,
        ServiceToolProvider, WorkspaceService, WorkspaceServiceFactory,
    };
    use crate::workspace_service::{
        AgentExecutionContext, ProjectRegistry, ServiceInstanceId, ServiceKind, WorkspaceRuntime,
    };

    struct ReadyBarrierFactory {
        gate: Arc<ServiceReadinessGate>,
    }

    impl WorkspaceServiceFactory for ReadyBarrierFactory {
        fn kind(&self) -> ServiceKind {
            ServiceKind::Unity
        }

        fn detect(&self, _workspace: &WorkspaceRuntime) -> DetectionResult {
            DetectionResult::detected(ServiceActivationPolicy::Lazy)
        }

        fn create<'a>(
            &'a self,
            workspace: Arc<WorkspaceRuntime>,
            generation: u64,
        ) -> ServiceFuture<'a, Result<Arc<dyn WorkspaceService>, String>> {
            let service: Arc<dyn WorkspaceService> = Arc::new(ReadyBarrierService {
                identity: ServiceRuntimeIdentity {
                    project_id: workspace.project_id().clone(),
                    checkout_id: workspace.checkout_id().clone(),
                    service_instance_id: ServiceInstanceId::for_service(
                        workspace.checkout_id(),
                        ServiceKind::Unity.as_str(),
                    ),
                    runtime_generation: generation,
                },
                gate: Arc::clone(&self.gate),
                leases: Arc::new(ServiceLeaseTracker::default()),
            });
            Box::pin(async move { Ok(service) })
        }
    }

    struct ReadyBarrierService {
        identity: ServiceRuntimeIdentity,
        gate: Arc<ServiceReadinessGate>,
        leases: Arc<ServiceLeaseTracker>,
    }

    impl WorkspaceService for ReadyBarrierService {
        fn identity(&self) -> ServiceRuntimeIdentity {
            self.identity.clone()
        }

        fn status(&self) -> ServiceStatus {
            ServiceStatus::Running
        }

        fn capabilities(&self) -> ServiceCapabilities {
            ServiceCapabilities::default()
        }

        fn lease_tracker(&self) -> Arc<ServiceLeaseTracker> {
            Arc::clone(&self.leases)
        }

        fn readiness(&self) -> ServiceReadinessSnapshot {
            self.gate.snapshot()
        }

        fn await_ready(
            &self,
            timeout: Duration,
        ) -> ServiceFuture<
            '_,
            Result<crate::workspace_service::ServiceReadyPermit, ServiceReadinessError>,
        > {
            Box::pin(self.gate.await_ready(&self.identity, timeout))
        }

        fn start(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }

        fn suspend(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }

        fn stop(&self) -> ServiceFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }

        fn tool_provider(&self) -> Arc<dyn ServiceToolProvider> {
            Arc::new(EmptyServiceProvider)
        }

        fn context_provider(&self) -> Arc<dyn ServiceContextProvider> {
            Arc::new(EmptyServiceProvider)
        }
    }

    struct EmptyServiceProvider;

    impl ServiceToolProvider for EmptyServiceProvider {
        fn tool_definitions(&self) -> Vec<ServiceToolDefinition> {
            Vec::new()
        }
    }

    impl ServiceContextProvider for EmptyServiceProvider {
        fn prompt_fragments(&self, _execution: &AgentExecutionContext) -> Vec<PromptFragment> {
            Vec::new()
        }
    }

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

    #[test]
    fn mcp_editor_command_tools_use_the_checkout_ready_barrier() {
        assert_eq!(MCP_SERVICE_READY_TIMEOUT, Duration::from_secs(45));
        for name in [
            "unity_set_play_mode",
            "unity_execute",
            "unity_run_states",
            "unity_capture_viewport",
            "unity_get_console_log",
            "unity_test_list",
            "unity_test_run",
            "unity_recompile",
            "unity_hot_reload",
        ] {
            assert!(
                crate::workspace_service::service::service_ready_required_for_tool(name),
                "{name} must wait for checkout command readiness"
            );
        }
        for name in [
            "unity_asset_search",
            "unity_ref_search",
            "unity_yaml_search",
            "unity_yaml_read",
        ] {
            assert!(
                !crate::workspace_service::service::service_ready_required_for_tool(name),
                "{name} is a checkout-local data-plane tool"
            );
        }
    }

    #[tokio::test]
    async fn mcp_editor_command_resolution_blocks_connected_and_reloading_until_ready() {
        let temp = tempfile::tempdir().expect("workspace");
        let config = Arc::new(AppConfig::load_from_path(&temp.path().join("config.json")));
        let policy = Arc::new(ResourcePolicyStore::from_config(config).expect("policy"));
        let gate = Arc::new(ServiceReadinessGate::new(ServiceReadinessPhase::Connected));
        let factory: Arc<dyn WorkspaceServiceFactory> = Arc::new(ReadyBarrierFactory {
            gate: Arc::clone(&gate),
        });
        let registry = ProjectRegistry::new(policy, vec![factory]);
        let runtime = registry.register(temp.path()).expect("runtime");
        let execution = registry
            .execution_context(runtime.checkout_id(), &[ServiceKind::Unity])
            .await
            .expect("execution context");

        let connected = match resolve_owned_service_for_mcp_tool(
            execution.as_ref(),
            "unity_execute",
            Duration::from_millis(10),
        )
        .await
        {
            Ok(_) => panic!("connected command channel must stay closed"),
            Err(error) => error,
        };
        assert!(matches!(
            connected,
            ServiceBindingError::Readiness {
                source: ServiceReadinessError::Timeout {
                    phase: ServiceReadinessPhase::Connected,
                    ..
                }
            }
        ));

        gate.transition(
            ServiceReadinessPhase::Reloading,
            Some("domain reload".to_string()),
        );
        let reloading = match resolve_owned_service_for_mcp_tool(
            execution.as_ref(),
            "unity_execute",
            Duration::from_millis(10),
        )
        .await
        {
            Ok(_) => panic!("reloading command channel must stay closed"),
            Err(error) => error,
        };
        assert!(matches!(
            reloading,
            ServiceBindingError::Readiness {
                source: ServiceReadinessError::Timeout {
                    phase: ServiceReadinessPhase::Reloading,
                    ..
                }
            }
        ));

        gate.transition(ServiceReadinessPhase::Ready, Some("ready".to_string()));
        let ready = resolve_owned_service_for_mcp_tool(
            execution.as_ref(),
            "unity_execute",
            Duration::from_millis(10),
        )
        .await
        .expect("ready command channel")
        .expect("owned Unity service");
        assert!(ready.ready_permit().is_some());
    }
}
