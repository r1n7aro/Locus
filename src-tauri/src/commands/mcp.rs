use crate::error::AppError;
use crate::mcp::config::{self, McpServerConfig};
use crate::mcp::import::McpImportCandidate;
use crate::mcp::manager::McpServerRuntimeStatus;
use crate::mcp::McpServerTestResult;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn mcp_servers_get() -> Result<Vec<McpServerConfig>, AppError> {
    Ok(config::load_servers())
}

/// Inserts or updates one server (matched by id; an empty id means insert
/// with a generated slug). Returns the full normalized list.
#[tauri::command]
pub async fn mcp_servers_upsert(server: McpServerConfig) -> Result<Vec<McpServerConfig>, AppError> {
    let mut servers = config::load_servers();
    let editing_id = server.id.trim().to_string();
    let position = servers.iter().position(|s| s.id == editing_id);
    let others: Vec<McpServerConfig> = servers
        .iter()
        .filter(|s| s.id != editing_id || editing_id.is_empty())
        .cloned()
        .collect();
    let mut normalized = if position.is_some() && !editing_id.is_empty() {
        // Keep the id stable across edits: normalize against the others,
        // then restore the original id (normalize_server treats a known id
        // as a duplicate otherwise).
        let mut draft = server;
        draft.id = String::new();
        let mut cleaned = config::normalize_server(draft, &others)
            .map_err(|e| AppError::new("mcp.invalid_server", e))?;
        cleaned.id = editing_id;
        cleaned
    } else {
        config::normalize_server(server, &others)
            .map_err(|e| AppError::new("mcp.invalid_server", e))?
    };
    normalized.name = normalized.name.trim().to_string();
    match position {
        Some(index) => servers[index] = normalized,
        None => servers.push(normalized),
    }
    config::save_servers(&servers).map_err(|e| AppError::new("mcp.save_failed", e))?;
    spawn_reconcile();
    Ok(servers)
}

#[tauri::command]
pub async fn mcp_servers_remove(id: String) -> Result<Vec<McpServerConfig>, AppError> {
    let mut servers = config::load_servers();
    servers.retain(|s| s.id != id);
    config::save_servers(&servers).map_err(|e| AppError::new("mcp.save_failed", e))?;
    spawn_reconcile();
    Ok(servers)
}

/// Settings writes apply to live connections in the background; the command
/// returns as soon as the file is saved so the page stays snappy.
fn spawn_reconcile() {
    tauri::async_runtime::spawn(async {
        let _ = crate::mcp::manager::reconcile().await;
    });
}

/// Tests the given configuration without saving it: spawn, handshake,
/// tools/list, shutdown. The settings form can therefore test a draft.
#[tauri::command]
pub async fn mcp_server_test(server: McpServerConfig) -> Result<McpServerTestResult, AppError> {
    Ok(crate::mcp::test_server(&server).await)
}

/// Live per-server status for the chat-bar indicator.
#[tauri::command]
pub async fn mcp_get_status() -> Result<Vec<McpServerRuntimeStatus>, AppError> {
    Ok(crate::mcp::manager::collect_status().await)
}

/// Enable/disable one server without resending its whole config (the
/// indicator popover only knows runtime status, not command/args/env).
#[tauri::command]
pub async fn mcp_server_set_enabled(
    id: String,
    enabled: bool,
) -> Result<Vec<McpServerConfig>, AppError> {
    let mut servers = config::load_servers();
    let Some(server) = servers.iter_mut().find(|s| s.id == id) else {
        return Err(AppError::new(
            "mcp.unknown_server",
            format!("MCP server '{id}' not found"),
        ));
    };
    server.enabled = enabled;
    config::save_servers(&servers).map_err(|e| AppError::new("mcp.save_failed", e))?;
    spawn_reconcile();
    Ok(servers)
}

/// Scans Claude Desktop / Claude Code / Cursor config files for importable
/// servers. Read-only; nothing is written until mcp_import_apply.
#[tauri::command]
pub async fn mcp_import_scan() -> Result<Vec<McpImportCandidate>, AppError> {
    let existing = config::load_servers();
    Ok(crate::mcp::import::scan_import_candidates(&existing))
}

/// Appends the selected import candidates to the config. Every import is
/// forced to disabled (the user reviews and enables explicitly) and gets a
/// fresh unique id.
#[tauri::command]
pub async fn mcp_import_apply(
    servers: Vec<McpServerConfig>,
) -> Result<Vec<McpServerConfig>, AppError> {
    let mut all = config::load_servers();
    for mut incoming in servers {
        incoming.id = String::new();
        incoming.enabled = false;
        let normalized = config::normalize_server(incoming, &all)
            .map_err(|e| AppError::new("mcp.invalid_server", e))?;
        all.push(normalized);
    }
    config::save_servers(&all).map_err(|e| AppError::new("mcp.save_failed", e))?;
    spawn_reconcile();
    Ok(all)
}

/// Wire tool names currently exposed for one server; feeds the settings
/// page's per-server approval bulk action.
#[tauri::command]
pub async fn mcp_server_wire_tools(id: String) -> Result<Vec<String>, AppError> {
    Ok(crate::mcp::manager::wire_tool_names_for_server(&id))
}

/// Full tool list one server reports, before allow/deny filtering; feeds the
/// settings form's per-tool toggles.
#[tauri::command]
pub async fn mcp_server_tools_inventory(
    id: String,
) -> Result<Vec<crate::mcp::McpToolSummary>, AppError> {
    Ok(crate::mcp::manager::server_tool_inventory(&id).await)
}

// ─── Locus-as-MCP-server (expose unity tools to external harnesses) ─────────

use crate::mcp::server as mcp_server;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStateView {
    pub settings: mcp_server::config::McpServerSettings,
    pub status: mcp_server::McpServerStatus,
    pub endpoint_url: String,
}

fn mcp_server_state(app: &tauri::AppHandle) -> McpServerStateView {
    use tauri::Manager;
    let settings = mcp_server::config::load_settings();
    let handle = app.state::<std::sync::Arc<mcp_server::McpServerHandle>>();
    McpServerStateView {
        endpoint_url: settings.endpoint_url(),
        status: mcp_server::status(&handle),
        settings,
    }
}

#[tauri::command]
pub async fn mcp_server_get_state(app: tauri::AppHandle) -> Result<McpServerStateView, AppError> {
    Ok(mcp_server_state(&app))
}

/// Writes settings (token is preserved; use mcp_server_regenerate_token to
/// rotate it) and restarts the listener to apply.
#[tauri::command]
pub async fn mcp_server_update_settings(
    app: tauri::AppHandle,
    enabled: bool,
    port: u16,
    disabled_tools: Vec<String>,
    call_timeout_ms: u64,
) -> Result<McpServerStateView, AppError> {
    let mut settings = mcp_server::config::load_settings();
    settings.enabled = enabled;
    settings.port = if port == 0 {
        mcp_server::config::DEFAULT_PORT
    } else {
        port
    };
    settings.disabled_tools = disabled_tools;
    settings.call_timeout_ms = call_timeout_ms.clamp(
        mcp_server::config::MIN_CALL_TIMEOUT_MS,
        mcp_server::config::MAX_CALL_TIMEOUT_MS,
    );
    mcp_server::config::save_settings(&settings)
        .map_err(|e| AppError::new("mcp_server.save_failed", e))?;
    mcp_server::reconcile(app.clone()).await;
    Ok(mcp_server_state(&app))
}

#[tauri::command]
pub async fn mcp_server_regenerate_token(
    app: tauri::AppHandle,
) -> Result<McpServerStateView, AppError> {
    let mut settings = mcp_server::config::load_settings();
    settings.token = mcp_server::config::generate_token();
    mcp_server::config::save_settings(&settings)
        .map_err(|e| AppError::new("mcp_server.save_failed", e))?;
    mcp_server::reconcile(app.clone()).await;
    Ok(mcp_server_state(&app))
}

#[tauri::command]
pub async fn mcp_server_tool_inventory(
    app: tauri::AppHandle,
) -> Result<Vec<mcp_server::tools::ExposedToolInfo>, AppError> {
    Ok(mcp_server::tools::exposed_tool_inventory(&app))
}

#[tauri::command]
pub async fn mcp_server_integrations(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<mcp_server::install::IntegrationStatus>, AppError> {
    let settings = mcp_server::config::load_settings();
    let (target, _scope) = mcp_server_integration_target(
        workspace_registry.inner(),
        &workspace_ref,
        &settings,
        "mcp_server_integrations",
    )?;
    Ok(mcp_server::install::integration_statuses(
        &target,
        &settings.token,
    ))
}

#[tauri::command]
pub async fn mcp_server_integration_apply(
    integration_id: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<mcp_server::install::IntegrationStatus, AppError> {
    let settings = mcp_server::config::load_settings();
    let (target, _scope) = mcp_server_integration_target(
        workspace_registry.inner(),
        &workspace_ref,
        &settings,
        "mcp_server_integration_apply",
    )?;
    mcp_server::install::apply_integration(&integration_id, &target, &settings.token)
        .map_err(|e| AppError::new("mcp_server.integration_failed", e))
}

#[tauri::command]
pub async fn mcp_server_integration_remove(
    integration_id: String,
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<mcp_server::install::IntegrationStatus, AppError> {
    let settings = mcp_server::config::load_settings();
    let (target, _scope) = mcp_server_integration_target(
        workspace_registry.inner(),
        &workspace_ref,
        &settings,
        "mcp_server_integration_remove",
    )?;
    mcp_server::install::remove_integration(&integration_id, &target)
        .map_err(|e| AppError::new("mcp_server.integration_failed", e))
}

fn mcp_server_integration_target(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    settings: &mcp_server::config::McpServerSettings,
    operation: &'static str,
) -> Result<
    (
        mcp_server::install::IntegrationTarget,
        crate::workspace_service::ResolvedWorkspaceScope,
    ),
    AppError,
> {
    if workspace_ref.expected_generation.is_none() {
        return Err(AppError::new(
            "workspace.generation_required",
            "MCP integration operations require checkoutId and expectedGeneration",
        )
        .operation(operation));
    }
    let scope = workspace_registry
        .resolve_workspace_ref(workspace_ref)
        .map_err(|error| {
            AppError::new("workspace.scope_invalid", error.to_string()).operation(operation)
        })?;
    let runtime = scope.runtime();
    let target = mcp_server::install::IntegrationTarget::new(
        runtime.checkout_id().as_str(),
        runtime.generation(),
        settings.scoped_endpoint_url(runtime.checkout_id().as_str(), Some(runtime.generation())),
    );
    Ok((target, scope))
}

#[cfg(test)]
mod scoped_integration_tests {
    use super::*;

    fn registry() -> Arc<ProjectRegistry> {
        let config_dir = tempfile::tempdir().expect("config");
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.path().join("config.json"),
        ));
        let policy = Arc::new(
            crate::resource_policy::ResourcePolicyStore::from_config(config).expect("policy"),
        );
        ProjectRegistry::new(policy, Vec::new())
    }

    #[test]
    fn integration_target_rejects_a_checkout_without_generation() {
        let workspace_ref = WorkspaceRef::new(
            crate::workspace_service::CheckoutId::new("checkout-a").expect("checkout id"),
            None,
        );
        let error = mcp_server_integration_target(
            &registry(),
            &workspace_ref,
            &mcp_server::config::McpServerSettings::default(),
            "test_integration",
        )
        .err()
        .expect("generation must be mandatory");
        assert_eq!(error.code, "workspace.generation_required");
        assert_eq!(error.operation.as_deref(), Some("test_integration"));
    }

    #[test]
    fn integration_target_uses_the_resolved_runtime_identity() {
        let registry = registry();
        let root = tempfile::tempdir().expect("checkout");
        let runtime = registry.register(root.path()).expect("runtime");
        let workspace_ref = WorkspaceRef::for_runtime(&runtime);
        let settings = mcp_server::config::McpServerSettings {
            port: 28991,
            ..Default::default()
        };
        let (target, _scope) =
            mcp_server_integration_target(&registry, &workspace_ref, &settings, "test_integration")
                .expect("target");
        assert_eq!(target.checkout_id, runtime.checkout_id().as_str());
        assert_eq!(target.workspace_generation, runtime.generation());
        assert!(target
            .endpoint_url
            .contains(&format!("checkoutId={}", runtime.checkout_id())));
        assert!(target
            .endpoint_url
            .contains(&format!("workspaceGeneration={}", runtime.generation())));
        assert!(target.entry_name.contains(runtime.checkout_id().as_str()));
    }
}
