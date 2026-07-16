//! MCP (Model Context Protocol) client integration.
//!
//! Locus acts as an MCP *client*: it connects to external servers (a Blender
//! bridge, a Figma desktop endpoint, ...) over stdio or Streamable HTTP and
//! surfaces their tools to agents. This module owns configuration storage,
//! the transports, the persistent connection registry (manager), config
//! importers for other clients' formats, and the one-shot connection test
//! used by the settings page (see plans/mcp-client-plan.md).

pub mod client;
pub mod config;
pub mod http;
pub mod import;
pub mod manager;
pub mod stdio;
pub mod types;

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;

use client::McpClient;
use config::McpServerConfig;
use types::{InitializeResult, McpToolInfo, ToolsListResult};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const LIST_TOOLS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TOOL_PAGES: usize = 10;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolSummary {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerTestResult {
    pub ok: bool,
    pub protocol_version: Option<String>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub tools: Vec<McpToolSummary>,
    pub error: Option<String>,
    pub elapsed_ms: u64,
}

impl McpServerTestResult {
    fn failure(error: String, started: Instant) -> Self {
        Self {
            ok: false,
            protocol_version: None,
            server_name: None,
            server_version: None,
            tools: Vec::new(),
            error: Some(error),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }
}

/// One-shot connection test: connect → initialize → initialized →
/// tools/list → shutdown. Used by the settings page "test" button; it never
/// touches the persistent connection registry, so testing an unsaved form
/// draft is fine.
pub async fn test_server(config: &McpServerConfig) -> McpServerTestResult {
    let started = Instant::now();
    let client = match McpClient::connect(config).await {
        Ok(client) => client,
        Err(e) => return McpServerTestResult::failure(e, started),
    };
    let result = run_handshake_and_list(&client).await;
    client.shutdown().await;
    match result {
        Ok((init, tools)) => McpServerTestResult {
            ok: true,
            protocol_version: init.protocol_version,
            server_name: init.server_info.as_ref().and_then(|s| s.name.clone()),
            server_version: init.server_info.as_ref().and_then(|s| s.version.clone()),
            tools: tools
                .into_iter()
                .map(|t| McpToolSummary {
                    name: t.name,
                    description: t.description.unwrap_or_default(),
                })
                .collect(),
            error: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
        },
        Err(e) => McpServerTestResult::failure(e, started),
    }
}

pub(crate) async fn run_handshake_and_list(
    client: &McpClient,
) -> Result<(InitializeResult, Vec<McpToolInfo>), String> {
    let init_value = client
        .request(
            "initialize",
            Some(types::initialize_params()),
            HANDSHAKE_TIMEOUT,
        )
        .await
        .map_err(|e| format!("initialize failed: {e}"))?;
    let init: InitializeResult = serde_json::from_value(init_value)
        .map_err(|e| format!("initialize returned an unexpected shape: {e}"))?;

    client
        .notify("notifications/initialized", None)
        .await
        .map_err(|e| format!("initialized notification failed: {e}"))?;

    let tools = list_all_tools(client).await?;
    Ok((init, tools))
}

/// Pages through tools/list; also used by the tools/list_changed reconcile.
pub(crate) async fn list_all_tools(client: &McpClient) -> Result<Vec<McpToolInfo>, String> {
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_TOOL_PAGES {
        let params = cursor
            .as_ref()
            .map(|c| json!({ "cursor": c }))
            .unwrap_or_else(|| json!({}));
        let page_value = client
            .request("tools/list", Some(params), LIST_TOOLS_TIMEOUT)
            .await
            .map_err(|e| format!("tools/list failed: {e}"))?;
        let page: ToolsListResult = serde_json::from_value(page_value)
            .map_err(|e| format!("tools/list returned an unexpected shape: {e}"))?;
        tools.extend(page.tools);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(tools)
}
