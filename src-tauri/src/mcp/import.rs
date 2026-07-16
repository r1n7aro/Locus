//! Importers for other MCP clients' config files.
//!
//! Scans the de-facto standard locations (Claude Desktop, Claude Code,
//! Cursor), parses their shared `mcpServers` shape and offers the entries
//! as import candidates. Imports only pre-fill: candidates arrive disabled
//! and the user enables them explicitly (same trust gate as the external
//! skill discovery).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use super::config::{McpLoadMode, McpServerConfig, McpTransport, DEFAULT_CALL_TIMEOUT_MS};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpImportCandidate {
    /// Machine source id: `claude_desktop` | `claude_code` | `cursor`.
    pub source: String,
    /// The file the entry came from (absolute, for display).
    pub source_path: String,
    pub server: McpServerConfig,
    /// Id of an already-configured server with the same command/url, if any.
    pub duplicate_of: Option<String>,
}

/// Scans all known external config locations and returns everything that
/// parses, deduplicated against `existing` and between sources.
pub fn scan_import_candidates(existing: &[McpServerConfig]) -> Vec<McpImportCandidate> {
    let mut candidates: Vec<McpImportCandidate> = Vec::new();
    for (source, path) in known_locations() {
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&data) else {
            eprintln!("[Mcp] import: {} is not valid JSON; skipping", path.display());
            continue;
        };
        for server in parse_external_config(&value) {
            // Same server appearing in several files (Claude Desktop +
            // Cursor both carrying blender) collapses to the first hit.
            let already_candidate = candidates
                .iter()
                .any(|c| same_connection(&c.server, &server));
            if already_candidate {
                continue;
            }
            let duplicate_of = existing
                .iter()
                .find(|s| same_connection(s, &server))
                .map(|s| s.id.clone());
            candidates.push(McpImportCandidate {
                source: source.to_string(),
                source_path: path.display().to_string(),
                server,
                duplicate_of,
            });
        }
    }
    candidates
}

fn known_locations() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    if let Some(config) = dirs::config_dir() {
        // Windows: %APPDATA%\Claude; macOS: ~/Library/Application Support/Claude.
        out.push((
            "claude_desktop",
            config.join("Claude").join("claude_desktop_config.json"),
        ));
    }
    if let Some(home) = dirs::home_dir() {
        out.push(("claude_code", home.join(".claude.json")));
        out.push(("cursor", home.join(".cursor").join("mcp.json")));
    }
    out
}

/// Pulls every `mcpServers` map out of one external config document. Claude
/// Code nests additional maps under `projects.<path>.mcpServers`; the
/// standard shape is a top-level map.
fn parse_external_config(value: &Value) -> Vec<McpServerConfig> {
    let mut servers = Vec::new();
    if let Some(map) = value.get("mcpServers").and_then(Value::as_object) {
        for (name, spec) in map {
            if let Some(server) = parse_server_spec(name, spec) {
                servers.push(server);
            }
        }
    }
    if let Some(projects) = value.get("projects").and_then(Value::as_object) {
        for project in projects.values() {
            let Some(map) = project.get("mcpServers").and_then(Value::as_object) else {
                continue;
            };
            for (name, spec) in map {
                if let Some(server) = parse_server_spec(name, spec) {
                    if !servers.iter().any(|s| same_connection(s, &server)) {
                        servers.push(server);
                    }
                }
            }
        }
    }
    servers
}

/// One `mcpServers` entry → a disabled Locus config. `None` for shapes we
/// cannot connect to (legacy SSE, websocket, malformed entries).
fn parse_server_spec(name: &str, spec: &Value) -> Option<McpServerConfig> {
    let spec = spec.as_object()?;
    let explicit_type = spec.get("type").and_then(Value::as_str).unwrap_or("");
    let command = spec.get("command").and_then(Value::as_str).unwrap_or("");
    let url = spec.get("url").and_then(Value::as_str).unwrap_or("");

    let transport = if !command.trim().is_empty() {
        McpTransport::Stdio
    } else if !url.trim().is_empty() {
        // Legacy HTTP+SSE servers speak a different handshake; importing
        // them as Streamable HTTP would only produce a confusing failure.
        if explicit_type.eq_ignore_ascii_case("sse") {
            return None;
        }
        McpTransport::Http
    } else {
        return None;
    };

    let args = spec
        .get("args")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    let env = string_map(spec.get("env"));
    let headers = string_map(spec.get("headers"));
    let cwd = spec
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Some(McpServerConfig {
        id: String::new(),
        name: name.trim().to_string(),
        transport,
        command: command.trim().to_string(),
        args,
        env,
        cwd,
        url: url.trim().to_string(),
        headers,
        // Imports never auto-enable: adding a server means running a local
        // process / trusting an endpoint, so the user flips the switch.
        enabled: false,
        call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
        auto_restart: false,
        load_mode: McpLoadMode::default(),
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
    })
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(map) = value.and_then(Value::as_object) {
        for (key, value) in map {
            if let Some(text) = value.as_str() {
                out.insert(key.clone(), text.to_string());
            }
        }
    }
    out
}

/// Two configs point at the same server when their connection identity
/// matches (command line for stdio, endpoint for HTTP).
fn same_connection(a: &McpServerConfig, b: &McpServerConfig) -> bool {
    if a.transport != b.transport {
        return false;
    }
    match a.transport {
        McpTransport::Stdio => a.command == b.command && a.args == b.args,
        McpTransport::Http => a.url == b.url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_the_shared_mcp_servers_shape() {
        let doc = json!({
            "mcpServers": {
                "blender": { "command": "uvx", "args": ["blender-mcp"], "env": {"K": "v"} },
                "figma": { "type": "http", "url": "http://127.0.0.1:3845/mcp" },
                "legacy": { "type": "sse", "url": "http://x/sse" },
                "broken": { "note": "no command or url" }
            }
        });
        let servers = parse_external_config(&doc);
        assert_eq!(servers.len(), 2);
        let blender = servers.iter().find(|s| s.name == "blender").unwrap();
        assert_eq!(blender.transport, McpTransport::Stdio);
        assert_eq!(blender.args, vec!["blender-mcp"]);
        assert_eq!(blender.env.get("K").map(String::as_str), Some("v"));
        assert!(!blender.enabled, "imports must arrive disabled");
        let figma = servers.iter().find(|s| s.name == "figma").unwrap();
        assert_eq!(figma.transport, McpTransport::Http);
        assert_eq!(figma.url, "http://127.0.0.1:3845/mcp");
    }

    #[test]
    fn merges_claude_code_project_scopes_without_duplicates() {
        let doc = json!({
            "mcpServers": { "a": { "command": "uvx", "args": ["a"] } },
            "projects": {
                "C:/proj1": { "mcpServers": { "a-copy": { "command": "uvx", "args": ["a"] } } },
                "C:/proj2": { "mcpServers": { "b": { "command": "npx", "args": ["-y", "b"] } } }
            }
        });
        let servers = parse_external_config(&doc);
        assert_eq!(servers.len(), 2, "same command+args collapses");
        assert!(servers.iter().any(|s| s.name == "b"));
    }

    #[test]
    fn same_connection_distinguishes_transports_and_identities() {
        let doc = json!({
            "mcpServers": {
                "x": { "command": "uvx", "args": ["x"] },
                "y": { "url": "http://localhost/mcp" }
            }
        });
        let servers = parse_external_config(&doc);
        assert!(!same_connection(&servers[0], &servers[1]));
        assert!(same_connection(&servers[0], &servers[0].clone()));
    }
}
