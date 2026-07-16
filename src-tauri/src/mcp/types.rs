//! Minimal MCP (Model Context Protocol) wire types.
//!
//! Covers only the subset Locus speaks as a client: JSON-RPC 2.0 framing,
//! the `initialize` handshake, and `tools/list` / `tools/call`. Everything
//! else (resources, prompts, sampling, ...) is intentionally out of scope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol revision Locus requests during `initialize`. Servers may answer
/// with an older revision; the tools-only subset is compatible across all
/// published revisions, so any non-empty answer is accepted.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

pub const JSONRPC_VERSION: &str = "2.0";

/// Sentinel error returned by cancellable requests when the caller aborted;
/// the agent layer maps it to the interrupted tool result instead of an
/// error the model would see.
pub const MCP_CALL_CANCELLED: &str = "__locus_mcp_call_cancelled__";

/// Unwraps a JSON-RPC response envelope into its `result`, mapping the
/// `error` member to a readable message.
pub fn extract_response_payload(envelope: &Value) -> Result<Value, String> {
    if let Some(error) = envelope.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        return Err(format!("MCP error {code}: {message}"));
    }
    Ok(envelope.get("result").cloned().unwrap_or(Value::Null))
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: i64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: i64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            method: method.into(),
            params,
        }
    }
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            method: method.into(),
            params,
        }
    }
}

/// One tool as reported by `tools/list`. `input_schema` is passed through
/// verbatim; Locus does not validate it beyond requiring a JSON object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsListResult {
    #[serde(default)]
    pub tools: Vec<McpToolInfo>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    #[serde(default)]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub server_info: Option<ServerInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub fn initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {
            "name": "locus",
            "version": env!("CARGO_PKG_VERSION"),
        },
    })
}
