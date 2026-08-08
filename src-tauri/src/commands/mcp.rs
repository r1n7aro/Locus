use crate::error::AppError;
use crate::mcp::config::{self, McpServerConfig};
use crate::mcp::import::McpImportCandidate;
use crate::mcp::manager::McpServerRuntimeStatus;
use crate::mcp::McpServerTestResult;

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
) -> Result<Vec<mcp_server::install::IntegrationStatus>, AppError> {
    let settings = mcp_server::config::load_settings();
    Ok(mcp_server::install::integration_statuses(
        &settings.endpoint_url(),
        &settings.token,
    ))
}

#[tauri::command]
pub async fn mcp_server_integration_apply(
    id: String,
) -> Result<mcp_server::install::IntegrationStatus, AppError> {
    let settings = mcp_server::config::load_settings();
    mcp_server::install::apply_integration(&id, &settings.endpoint_url(), &settings.token)
        .map_err(|e| AppError::new("mcp_server.integration_failed", e))
}

#[tauri::command]
pub async fn mcp_server_integration_remove(
    id: String,
) -> Result<mcp_server::install::IntegrationStatus, AppError> {
    mcp_server::install::remove_integration(&id)
        .map_err(|e| AppError::new("mcp_server.integration_failed", e))
}
