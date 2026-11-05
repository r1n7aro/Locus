//! MCP server (expose-to-other-agents) configuration.
//!
//! The options are settings (`mcpServer.*`, sensitive). The auth token is a
//! secret and stays in `{persistent_config_dir}/mcp_server.json`; it never
//! surfaces through config_query.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const MCP_SERVER_FILE: &str = "mcp_server.json";
/// Settings group behind the non-secret fields.
const SETTINGS_GROUP: &str = "mcpServer";
const TOKEN_FIELD: &str = "token";

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
    /// Process-level listener URL. Workspace tools require a scoped URL from
    /// `scoped_endpoint_url`; the base URL is retained for settings and
    /// integration discovery.
    pub fn endpoint_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn scoped_endpoint_url(
        &self,
        checkout_id: &str,
        workspace_generation: Option<u64>,
    ) -> String {
        let checkout_id: String =
            url::form_urlencoded::byte_serialize(checkout_id.as_bytes()).collect();
        let mut url = format!("{}?checkoutId={checkout_id}", self.endpoint_url());
        if let Some(generation) = workspace_generation {
            url.push_str(&format!("&workspaceGeneration={generation}"));
        }
        url
    }

    pub fn tool_enabled(&self, name: &str) -> bool {
        !self.disabled_tools.iter().any(|t| t.trim() == name)
    }

    pub fn scoped_endpoint_url_with_epoch(
        &self,
        checkout_id: &str,
        generation: Option<u64>,
        epoch: u64,
    ) -> String {
        format!(
            "{}&materializationEpoch={epoch}",
            self.scoped_endpoint_url(checkout_id, generation)
        )
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

fn load_token() -> String {
    config_path()
        .map(|path| crate::config::read_state_file(&path))
        .ok()
        .and_then(|file| file.get(TOKEN_FIELD).and_then(|token| token.as_str()).map(str::to_string))
        .unwrap_or_default()
}

fn save_token(token: &str) -> Result<(), String> {
    let path = config_path()?;
    let _guard = save_lock().lock().unwrap_or_else(|p| p.into_inner());
    crate::config::update_state_file(&path, |file| {
        file.insert(TOKEN_FIELD.to_string(), serde_json::Value::from(token));
    })
}

/// Loads settings; generates and persists a token on first use so the
/// settings page always has one to show.
pub fn load_settings() -> McpServerSettings {
    let group =
        crate::settings::group::read(crate::settings::global::global(), SETTINGS_GROUP);
    let mut settings = serde_json::from_value::<McpServerSettings>(group).unwrap_or_default();
    settings.call_timeout_ms = settings
        .call_timeout_ms
        .clamp(MIN_CALL_TIMEOUT_MS, MAX_CALL_TIMEOUT_MS);
    settings.token = load_token();
    if settings.token.trim().is_empty() {
        settings.token = generate_token();
        if let Err(e) = save_token(&settings.token) {
            eprintln!("[McpServer] failed to persist generated token: {e}");
        }
    }
    settings
}

pub fn save_settings(settings: &McpServerSettings) -> Result<(), String> {
    let value = serde_json::to_value(settings)
        .map_err(|e| format!("Failed to serialize MCP server settings: {e}"))?;
    crate::settings::global::global()
        .set_values(&crate::settings::group::changes(SETTINGS_GROUP, &value))
        .map_err(|error| format!("Failed to save MCP server settings: {error}"))?;
    if !settings.token.trim().is_empty() && settings.token != load_token() {
        save_token(&settings.token)?;
    }
    Ok(())
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
        assert_eq!(
            settings.scoped_endpoint_url("checkout A", Some(7)),
            "http://127.0.0.1:27121/mcp?checkoutId=checkout+A&workspaceGeneration=7"
        );
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
