//! MCP server (expose-to-other-agents) configuration.
//!
//! Stored in `{persistent_config_dir}/mcp_server.json`, deliberately OUTSIDE
//! config.json/AppConfig: the auth token must never surface through
//! config_registry / the agent-facing config_query tool.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MCP_SERVER_FILE: &str = "mcp_server.json";

pub const DEFAULT_PORT: u16 = 27121;
pub const DEFAULT_CALL_TIMEOUT_MS: u64 = 600_000;
pub const MIN_CALL_TIMEOUT_MS: u64 = 10_000;
pub const MAX_CALL_TIMEOUT_MS: u64 = 3_600_000;

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_call_timeout_ms() -> u64 {
    DEFAULT_CALL_TIMEOUT_MS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Bearer token external harnesses must present. Generated on first
    /// load; regenerable from the settings page.
    #[serde(default)]
    pub token: String,
    /// Exposed tools removed by the user (default: everything exposed).
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default = "default_call_timeout_ms")]
    pub call_timeout_ms: u64,
}

impl Default for McpServerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
            disabled_tools: Vec::new(),
            call_timeout_ms: DEFAULT_CALL_TIMEOUT_MS,
        }
    }
}

impl McpServerSettings {
    pub fn endpoint_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn tool_enabled(&self, name: &str) -> bool {
        !self.disabled_tools.iter().any(|t| t.trim() == name)
    }
}

pub fn generate_token() -> String {
    // 32 bytes of entropy as hex; uuid v4 carries 122 random bits each.
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn config_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(MCP_SERVER_FILE))
}

/// Serializes concurrent save calls; reads are lock-free (last write wins).
fn save_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

/// Loads settings; generates and persists a token on first use so the
/// settings page always has one to show.
pub fn load_settings() -> McpServerSettings {
    let mut settings = config_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|data| serde_json::from_str::<McpServerSettings>(&data).ok())
        .unwrap_or_default();
    settings.call_timeout_ms = settings
        .call_timeout_ms
        .clamp(MIN_CALL_TIMEOUT_MS, MAX_CALL_TIMEOUT_MS);
    if settings.token.trim().is_empty() {
        settings.token = generate_token();
        if let Err(e) = save_settings(&settings) {
            eprintln!("[McpServer] failed to persist generated token: {e}");
        }
    }
    settings
}

pub fn save_settings(settings: &McpServerSettings) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config dir: {e}"))?;
    }
    let data = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize mcp_server.json: {e}"))?;
    let _guard = save_lock().lock().unwrap_or_else(|p| p.into_inner());
    std::fs::write(path, data).map_err(|e| format!("Failed to write mcp_server.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_tool_enabled() {
        let settings = McpServerSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.port, DEFAULT_PORT);
        assert_eq!(settings.endpoint_url(), "http://127.0.0.1:27121/mcp");
        assert!(settings.tool_enabled("unity_execute"));

        let settings = McpServerSettings {
            disabled_tools: vec!["unity_execute".to_string()],
            ..Default::default()
        };
        assert!(!settings.tool_enabled("unity_execute"));
        assert!(settings.tool_enabled("unity_project_info"));
    }

    #[test]
    fn generated_tokens_are_long_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
    }

    #[test]
    fn settings_roundtrip_preserves_fields() {
        let settings = McpServerSettings {
            enabled: true,
            port: 30000,
            token: "t".repeat(64),
            disabled_tools: vec!["code_hover".to_string()],
            call_timeout_ms: 120_000,
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("disabledTools"), "camelCase wire shape");
        let back: McpServerSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
    }
}
