//! MCP server configuration storage.
//!
//! Servers are machine-level assets (a Blender bridge lives on this machine,
//! not in a project), so the list is stored globally in
//! `{persistent_config_dir}/mcp_servers.json` — the same home as
//! `custom_endpoints.json` — rather than per-workspace under `Library/Locus`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MCP_SERVERS_FILE: &str = "mcp_servers.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    Stdio,
    Http,
}

impl Default for McpTransport {
    fn default() -> Self {
        McpTransport::Stdio
    }
}

fn default_true() -> bool {
    true
}

/// Tool-call wall clock default. Two-hop bridges (blender-mcp keeps a 180s
/// socket timeout towards Blender) need the outer timeout to stay above the
/// inner one, so anything below ~120s misreports slow operations as failures.
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 120_000;
pub const MIN_CALL_TIMEOUT_MS: u64 = 1_000;
pub const MAX_CALL_TIMEOUT_MS: u64 = 3_600_000;

fn default_call_timeout_ms() -> u64 {
    DEFAULT_CALL_TIMEOUT_MS
}

/// How a server's tools enter the agent context. Lazy (the default) rides
/// the provider-native deferred loading path; direct puts every tool schema
/// in the request up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpLoadMode {
    #[default]
    Lazy,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub transport: McpTransport,
    /// stdio: executable or PATH-resolvable program name.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment for the child process. Values may reference the
    /// parent environment as `${VAR}`; unknown variables stay literal.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: String,
    /// http: server endpoint URL (reserved; transport lands in a later batch).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_call_timeout_ms")]
    pub call_timeout_ms: u64,
    /// stdio only: restart the process automatically (with backoff and
    /// crash-loop protection) when it exits. Off by default — local process
    /// crashes are usually environment problems, and the lazy per-call
    /// restart already covers the common case.
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default)]
    pub load_mode: McpLoadMode,
    /// When non-empty, only these tool names are exposed to agents.
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    /// Tools hidden from agents; wins over the allowlist.
    #[serde(default)]
    pub tool_denylist: Vec<String>,
}

impl McpServerConfig {
    /// Applies the allow/deny lists to one tool name (raw MCP name, not the
    /// wire name).
    pub fn tool_exposed(&self, tool_name: &str) -> bool {
        if self
            .tool_denylist
            .iter()
            .any(|t| t.trim() == tool_name)
        {
            return false;
        }
        let allow: Vec<&str> = self
            .tool_allowlist
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        allow.is_empty() || allow.contains(&tool_name)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServersFile {
    #[serde(default)]
    servers: Vec<serde_json::Value>,
}

fn config_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(MCP_SERVERS_FILE))
}

/// mtime of mcp_servers.json; `None` when the file does not exist. Drives
/// the manager's staleness check for edits that bypass the settings
/// commands (agents writing the file via bash).
pub fn config_file_mtime() -> Option<std::time::SystemTime> {
    let path = config_path().ok()?;
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Serializes concurrent save calls; reads are lock-free (last write wins).
fn save_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

/// Loads all configured servers. Entries that fail to parse individually
/// (e.g. an unknown transport written by a future version) are skipped with
/// a log line instead of discarding the whole file.
pub fn load_servers() -> Vec<McpServerConfig> {
    let Ok(path) = config_path() else {
        return Vec::new();
    };
    let Ok(data) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(file) = serde_json::from_str::<McpServersFile>(&data) else {
        eprintln!("[Mcp] mcp_servers.json is not valid JSON; ignoring");
        return Vec::new();
    };
    file.servers
        .into_iter()
        .filter_map(|value| match serde_json::from_value::<McpServerConfig>(value) {
            Ok(server) => Some(server),
            Err(e) => {
                eprintln!("[Mcp] skipping unparsable server entry: {e}");
                None
            }
        })
        .collect()
}

pub fn save_servers(servers: &[McpServerConfig]) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir: {e}"))?;
    }
    let file = McpServersFile {
        servers: servers
            .iter()
            .map(|s| serde_json::to_value(s).expect("McpServerConfig serializes"))
            .collect(),
    };
    let data = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("Failed to serialize MCP servers: {e}"))?;
    let _guard = save_lock().lock().unwrap_or_else(|p| p.into_inner());
    std::fs::write(path, data).map_err(|e| format!("Failed to write mcp_servers.json: {e}"))
}

/// Lowercases and strips a display name into a stable id slug. The slug ends
/// up inside wire tool names (`mcp__<slug>__<tool>`), so it must stay within
/// `[a-z0-9_-]` and non-empty.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.trim().chars() {
        let mapped = match ch {
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            'a'..='z' | '0'..='9' | '_' => Some(ch),
            _ => None,
        };
        match mapped {
            Some(c) => {
                slug.push(c);
                last_dash = false;
            }
            None if !last_dash && !slug.is_empty() => {
                slug.push('-');
                last_dash = true;
            }
            None => {}
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "server".to_string()
    } else {
        slug
    }
}

fn clean_tool_list(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in list {
        let trimmed = item.trim().to_string();
        if !trimmed.is_empty() && !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    out
}

fn unique_id(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if !taken.iter().any(|t| *t == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Validates and normalizes one server entry against the existing list.
/// Returns the cleaned entry; `existing` must not already contain it (upsert
/// callers filter the edited entry out first).
pub fn normalize_server(
    mut server: McpServerConfig,
    existing: &[McpServerConfig],
) -> Result<McpServerConfig, String> {
    server.name = server.name.trim().to_string();
    if server.name.is_empty() {
        return Err("Server name cannot be empty".to_string());
    }
    server.command = server.command.trim().to_string();
    server.cwd = server.cwd.trim().to_string();
    server.url = server.url.trim().to_string();
    server.args = server
        .args
        .into_iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    server.env = server
        .env
        .into_iter()
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    server.tool_allowlist = clean_tool_list(server.tool_allowlist);
    server.tool_denylist = clean_tool_list(server.tool_denylist);
    match server.transport {
        McpTransport::Stdio => {
            if server.command.is_empty() {
                return Err("Command cannot be empty for a stdio server".to_string());
            }
        }
        McpTransport::Http => {
            if server.url.is_empty() {
                return Err("URL cannot be empty for an HTTP server".to_string());
            }
            if !server.url.starts_with("http://") && !server.url.starts_with("https://") {
                return Err("Server URL must start with http:// or https://".to_string());
            }
            server.auto_restart = false;
        }
    }
    server.call_timeout_ms = server
        .call_timeout_ms
        .clamp(MIN_CALL_TIMEOUT_MS, MAX_CALL_TIMEOUT_MS);

    let taken: Vec<&str> = existing.iter().map(|s| s.id.as_str()).collect();
    let id = server.id.trim().to_string();
    server.id = if id.is_empty() {
        unique_id(&slugify(&server.name), &taken)
    } else {
        let slug = slugify(&id);
        if taken.contains(&slug.as_str()) {
            return Err(format!("Server id '{slug}' already exists"));
        }
        slug
    };
    Ok(server)
}

/// Expands `${VAR}` references against the parent process environment.
/// Unknown variables are kept literal (matching Claude Code's behavior) so a
/// typo is visible instead of silently becoming an empty string.
pub fn expand_env_refs(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let var = &after[..end];
                let valid = !var.is_empty()
                    && var
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_');
                match std::env::var(var) {
                    Ok(v) if valid => out.push_str(&v),
                    _ => {
                        out.push_str("${");
                        out.push_str(var);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_server(name: &str, command: &str) -> McpServerConfig {
        McpServerConfig {
            id: String::new(),
            name: name.to_string(),
            transport: McpTransport::Stdio,
            command: command.to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: String::new(),
            url: String::new(),
            headers: BTreeMap::new(),
            enabled: true,
            call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
            auto_restart: false,
            load_mode: McpLoadMode::default(),
            tool_allowlist: Vec::new(),
            tool_denylist: Vec::new(),
        }
    }

    #[test]
    fn slugify_maps_display_names_to_wire_safe_ids() {
        assert_eq!(slugify("Blender MCP"), "blender-mcp");
        assert_eq!(slugify("  中文名  "), "server");
        assert_eq!(slugify("a__B--9 !!"), "a__b-9");
    }

    #[test]
    fn normalize_generates_unique_ids_and_validates() {
        let existing = vec![McpServerConfig {
            id: "blender".into(),
            ..stdio_server("Blender", "uvx")
        }];
        let normalized = normalize_server(stdio_server("Blender", "uvx"), &existing).unwrap();
        assert_eq!(normalized.id, "blender-2");

        assert!(normalize_server(stdio_server("  ", "uvx"), &[]).is_err());
        assert!(normalize_server(stdio_server("x", "  "), &[]).is_err());

        let mut clamped = stdio_server("x", "uvx");
        clamped.call_timeout_ms = 1;
        assert_eq!(
            normalize_server(clamped, &[]).unwrap().call_timeout_ms,
            MIN_CALL_TIMEOUT_MS
        );
    }

    #[test]
    fn normalize_rejects_duplicate_explicit_id() {
        let existing = vec![McpServerConfig {
            id: "blender".into(),
            ..stdio_server("Blender", "uvx")
        }];
        let mut dup = stdio_server("Other", "uvx");
        dup.id = "Blender".into();
        assert!(normalize_server(dup, &existing).is_err());
    }

    #[test]
    fn normalize_accepts_http_servers_and_validates_url() {
        let mut http = stdio_server("Figma", "");
        http.transport = McpTransport::Http;
        http.url = "http://127.0.0.1:3845/mcp".into();
        let normalized = normalize_server(http.clone(), &[]).unwrap();
        assert_eq!(normalized.url, "http://127.0.0.1:3845/mcp");

        http.url = "ws://bad".into();
        assert!(normalize_server(http.clone(), &[]).is_err());
        http.url = String::new();
        assert!(normalize_server(http, &[]).is_err());
    }

    #[test]
    fn tool_exposed_applies_allow_then_deny() {
        let mut server = stdio_server("x", "uvx");
        assert!(server.tool_exposed("anything"));
        server.tool_allowlist = vec!["a".into(), "b".into()];
        assert!(server.tool_exposed("a"));
        assert!(!server.tool_exposed("c"));
        server.tool_denylist = vec!["a".into()];
        assert!(!server.tool_exposed("a"));
        assert!(server.tool_exposed("b"));
    }

    #[test]
    fn expand_env_refs_expands_known_and_keeps_unknown() {
        std::env::set_var("LOCUS_MCP_TEST_VAR", "hello");
        assert_eq!(expand_env_refs("v=${LOCUS_MCP_TEST_VAR}!"), "v=hello!");
        assert_eq!(expand_env_refs("${LOCUS_MCP_MISSING_VAR}"), "${LOCUS_MCP_MISSING_VAR}");
        assert_eq!(expand_env_refs("${not closed"), "${not closed");
        assert_eq!(expand_env_refs("plain"), "plain");
    }
}
