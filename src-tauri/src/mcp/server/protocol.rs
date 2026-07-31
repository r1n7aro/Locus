//! JSON-RPC / MCP protocol layer for the Locus MCP *server*: pure functions
//! from parsed request JSON to response JSON. No IO, no tauri — fully unit
//! testable. Wire subset mirrors what Locus's own client speaks:
//! initialize / ping / tools/list / tools/call, everything else is a
//! polite JSON-RPC error.

use serde_json::{json, Value};

/// Protocol revisions this server can answer with. A client asking for a
/// known revision gets it echoed; anything else falls back to the newest.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18"];
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

pub const ERR_METHOD_NOT_FOUND: i64 = -32601;
pub const ERR_INVALID_PARAMS: i64 = -32602;
pub const ERR_INVALID_REQUEST: i64 = -32600;
pub const ERR_PARSE: i64 = -32700;

pub struct ToolListing {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One parsed inbound message.
pub enum RpcAction {
    Initialize {
        id: Value,
        requested_version: Option<String>,
    },
    /// Any `notifications/*` (no id, no response body — HTTP layer answers 202).
    Notification,
    Ping {
        id: Value,
    },
    ToolsList {
        id: Value,
    },
    ToolsCall {
        id: Value,
        name: String,
        arguments: Value,
    },
    UnknownMethod {
        id: Value,
        method: String,
    },
    /// Not a JSON-RPC request object at all (or a batch, which 2025-06-18 removed).
    Invalid,
}

pub fn parse_message(message: &Value) -> RpcAction {
    let Some(obj) = message.as_object() else {
        return RpcAction::Invalid;
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return RpcAction::Invalid;
    };
    let id = obj.get("id").cloned().unwrap_or(Value::Null);
    if method.starts_with("notifications/") {
        return RpcAction::Notification;
    }
    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "initialize" => RpcAction::Initialize {
            id,
            requested_version: params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        "ping" => RpcAction::Ping { id },
        "tools/list" => RpcAction::ToolsList { id },
        "tools/call" => RpcAction::ToolsCall {
            id,
            name: params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
        },
        other => RpcAction::UnknownMethod {
            id,
            method: other.to_string(),
        },
    }
}

pub fn negotiate_version(requested: Option<&str>) -> &'static str {
    match requested {
        Some(v) => SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .find(|known| **known == v)
            .copied()
            .unwrap_or(LATEST_PROTOCOL_VERSION),
        None => LATEST_PROTOCOL_VERSION,
    }
}

pub fn initialize_response(id: &Value, requested: Option<&str>, instructions: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "protocolVersion": negotiate_version(requested),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": "locus",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": instructions,
        }
    })
}

pub fn pong_response(id: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": {} })
}

pub fn tools_list_response(id: &Value, tools: &[ToolListing]) -> Value {
    let items: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.input_schema,
            })
        })
        .collect();
    json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": items } })
}

/// `images`: (base64 data, mime type) pairs, mapped to MCP image content.
pub fn tool_result_response(
    id: &Value,
    output: &str,
    is_error: bool,
    images: &[(String, String)],
) -> Value {
    let mut content = vec![json!({ "type": "text", "text": output })];
    for (data, mime_type) in images {
        if data.trim().is_empty() {
            continue;
        }
        content.push(json!({ "type": "image", "data": data, "mimeType": mime_type }));
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": content, "isError": is_error }
    })
}

pub fn error_response(id: &Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_core_methods() {
        assert!(matches!(
            parse_message(&json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":"2025-03-26"}})),
            RpcAction::Initialize { requested_version: Some(v), .. } if v == "2025-03-26"
        ));
        assert!(matches!(
            parse_message(&json!({"jsonrpc":"2.0","method":"notifications/initialized"})),
            RpcAction::Notification
        ));
        assert!(matches!(
            parse_message(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"unity_project_info","arguments":{}}})),
            RpcAction::ToolsCall { name, .. } if name == "unity_project_info"
        ));
        assert!(matches!(parse_message(&json!([1, 2])), RpcAction::Invalid));
        assert!(matches!(
            parse_message(&json!({"jsonrpc":"2.0","id":3})),
            RpcAction::Invalid
        ));
        assert!(matches!(
            parse_message(&json!({"jsonrpc":"2.0","id":3,"method":"resources/list"})),
            RpcAction::UnknownMethod { .. }
        ));
    }

    #[test]
    fn negotiates_known_versions_and_falls_back() {
        assert_eq!(negotiate_version(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate_version(Some("2025-06-18")), "2025-06-18");
        assert_eq!(negotiate_version(Some("2099-01-01")), LATEST_PROTOCOL_VERSION);
        assert_eq!(negotiate_version(None), LATEST_PROTOCOL_VERSION);
    }

    #[test]
    fn initialize_response_carries_instructions_and_server_info() {
        let value = initialize_response(&json!(1), Some("2024-11-05"), "Active project: X");
        assert_eq!(value["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(value["result"]["serverInfo"]["name"], "locus");
        assert_eq!(value["result"]["instructions"], "Active project: X");
        assert_eq!(value["result"]["capabilities"]["tools"]["listChanged"], false);
    }

    #[test]
    fn tool_result_maps_text_and_images() {
        let value = tool_result_response(
            &json!(7),
            "done",
            false,
            &[
                ("QUJD".to_string(), "image/png".to_string()),
                ("  ".to_string(), "image/png".to_string()),
            ],
        );
        let content = value["result"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2, "blank image data is dropped");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "done");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["mimeType"], "image/png");
        assert_eq!(value["result"]["isError"], false);
    }

    #[test]
    fn error_response_echoes_id_and_code() {
        let value = error_response(&json!("abc"), ERR_METHOD_NOT_FOUND, "nope");
        assert_eq!(value["id"], "abc");
        assert_eq!(value["error"]["code"], -32601);
        assert_eq!(value["error"]["message"], "nope");
    }
}
