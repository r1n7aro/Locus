//! One-click registration of the Locus MCP endpoint into external harness
//! configs (Claude Code, Codex, OpenCode, Cursor, Gemini CLI).
//!
//! Mirrors the formats mcp/import.rs already reads, plus OpenCode/Gemini.
//! Every writer refuses to touch a file it cannot parse (never clobber a
//! user's config), and JSON rewrites go through serde_json pretty-printing
//! (key order may change; semantically lossless). Codex's TOML is edited
//! with toml_edit so comments and formatting survive.
//!
//! Path derivation takes an explicit `home` so tests can run against a
//! temp directory.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    /// `mcpServers.<name>` maps (Claude Code, Cursor, Gemini) or the
    /// OpenCode `mcp.<name>` map — all JSON.
    Json,
    /// Codex `config.toml` `[mcp_servers.<name>]` tables.
    CodexToml,
}

struct Integration {
    id: &'static str,
    name: &'static str,
    /// Config file, relative to home.
    config_rel: &'static [&'static str],
    /// Directory whose presence marks the harness as installed, relative to
    /// home. Empty = use the config file itself.
    detect_rel: &'static [&'static str],
    format: ConfigFormat,
    /// JSON pointer segments to the server map inside the document.
    json_map_path: &'static [&'static str],
}

const INTEGRATIONS: &[Integration] = &[
    Integration {
        id: "claude_code",
        name: "Claude Code",
        config_rel: &[".claude.json"],
        detect_rel: &[],
        format: ConfigFormat::Json,
        json_map_path: &["mcpServers"],
    },
    Integration {
        id: "codex",
        name: "Codex CLI",
        config_rel: &[".codex", "config.toml"],
        detect_rel: &[".codex"],
        format: ConfigFormat::CodexToml,
        json_map_path: &[],
    },
    Integration {
        id: "opencode",
        name: "OpenCode",
        config_rel: &[".config", "opencode", "opencode.json"],
        detect_rel: &[".config", "opencode"],
        format: ConfigFormat::Json,
        json_map_path: &["mcp"],
    },
    Integration {
        id: "cursor",
        name: "Cursor",
        config_rel: &[".cursor", "mcp.json"],
        detect_rel: &[".cursor"],
        format: ConfigFormat::Json,
        json_map_path: &["mcpServers"],
    },
    Integration {
        id: "gemini",
        name: "Gemini CLI",
        config_rel: &[".gemini", "settings.json"],
        detect_rel: &[".gemini"],
        format: ConfigFormat::Json,
        json_map_path: &["mcpServers"],
    },
];

pub const SERVER_ENTRY_NAME: &str = "locus";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    pub id: String,
    pub name: String,
    pub config_path: String,
    /// The harness appears to be installed on this machine.
    pub detected: bool,
    /// "absent" | "current" | "stale"
    pub state: String,
}

fn join_rel(home: &Path, rel: &[&str]) -> PathBuf {
    let mut path = home.to_path_buf();
    for part in rel {
        path.push(part);
    }
    path
}

fn integration(id: &str) -> Result<&'static Integration, String> {
    INTEGRATIONS
        .iter()
        .find(|integration| integration.id == id)
        .ok_or_else(|| format!("Unknown integration '{id}'"))
}

fn bearer_value(token: &str) -> String {
    format!("Bearer {token}")
}

/// The JSON entry each harness gets. Claude Code wants an explicit
/// `type: "http"`; OpenCode uses `type: "remote"`; Gemini keys the URL as
/// `httpUrl`; Cursor infers the transport from `url`.
fn json_entry(integration_id: &str, url: &str, token: &str) -> Value {
    let headers = json!({ "Authorization": bearer_value(token) });
    match integration_id {
        "claude_code" => json!({ "type": "http", "url": url, "headers": headers }),
        "opencode" => {
            json!({ "type": "remote", "url": url, "headers": headers, "enabled": true })
        }
        "gemini" => json!({ "httpUrl": url, "headers": headers }),
        _ => json!({ "url": url, "headers": headers }),
    }
}

/// Reads the current entry (if any) and classifies it against the wanted
/// url + auth header.
fn classify_entry(entry: Option<&Value>, url: &str, token: &str) -> &'static str {
    let Some(entry) = entry else { return "absent" };
    let entry_url = entry
        .get("url")
        .or_else(|| entry.get("httpUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let entry_auth = entry
        .get("headers")
        .and_then(|headers| headers.get("Authorization"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if entry_url == url && entry_auth == bearer_value(token) {
        "current"
    } else {
        "stale"
    }
}

fn load_json_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    if data.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&data).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); refusing to modify it",
            path.display()
        )
    })
}

fn json_server_map<'a>(
    document: &'a mut Value,
    map_path: &[&str],
    create: bool,
) -> Option<&'a mut serde_json::Map<String, Value>> {
    let mut current = document;
    for segment in map_path {
        if create && current.get(*segment).is_none() {
            current
                .as_object_mut()?
                .insert(segment.to_string(), json!({}));
        }
        current = current.get_mut(*segment)?;
    }
    current.as_object_mut()
}

fn save_json_document(path: &Path, document: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(document)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    std::fs::write(path, data).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

fn load_codex_document(path: &Path) -> Result<toml_edit::Document, String> {
    if !path.exists() {
        return Ok(toml_edit::Document::new());
    }
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    data.parse::<toml_edit::Document>().map_err(|e| {
        format!(
            "{} is not valid TOML ({e}); refusing to modify it",
            path.display()
        )
    })
}

fn codex_entry_value(document: &toml_edit::Document) -> Option<Value> {
    let entry = document
        .get("mcp_servers")
        .and_then(|servers| servers.get(SERVER_ENTRY_NAME))?;
    let url = entry.get("url").and_then(|item| item.as_str())?;
    let auth = entry
        .get("http_headers")
        .and_then(|headers| headers.get("Authorization"))
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    Some(json!({ "url": url, "headers": { "Authorization": auth } }))
}

fn current_entry(integration: &Integration, config_path: &Path) -> Result<Option<Value>, String> {
    match integration.format {
        ConfigFormat::Json => {
            let mut document = load_json_document(config_path)?;
            Ok(
                json_server_map(&mut document, integration.json_map_path, false)
                    .and_then(|map| map.get(SERVER_ENTRY_NAME).cloned()),
            )
        }
        ConfigFormat::CodexToml => Ok(codex_entry_value(&load_codex_document(config_path)?)),
    }
}

fn status_of(
    integration: &Integration,
    home: &Path,
    url: &str,
    token: &str,
) -> IntegrationStatus {
    let config_path = join_rel(home, integration.config_rel);
    let detected = if integration.detect_rel.is_empty() {
        config_path.exists()
    } else {
        join_rel(home, integration.detect_rel).exists()
    };
    let state = match current_entry(integration, &config_path) {
        Ok(entry) => classify_entry(entry.as_ref(), url, token).to_string(),
        // Unparsable config: surface as stale so the UI offers an update,
        // which will then fail with the parse error message.
        Err(_) => "stale".to_string(),
    };
    IntegrationStatus {
        id: integration.id.to_string(),
        name: integration.name.to_string(),
        config_path: config_path.display().to_string(),
        detected,
        state,
    }
}

pub(super) fn statuses_at(home: &Path, url: &str, token: &str) -> Vec<IntegrationStatus> {
    INTEGRATIONS
        .iter()
        .map(|integration| status_of(integration, home, url, token))
        .collect()
}

pub(super) fn apply_at(
    home: &Path,
    id: &str,
    url: &str,
    token: &str,
) -> Result<IntegrationStatus, String> {
    let integration = integration(id)?;
    let config_path = join_rel(home, integration.config_rel);
    match integration.format {
        ConfigFormat::Json => {
            let mut document = load_json_document(&config_path)?;
            if !document.is_object() {
                return Err(format!(
                    "{} does not contain a JSON object; refusing to modify it",
                    config_path.display()
                ));
            }
            let map = json_server_map(&mut document, integration.json_map_path, true)
                .ok_or_else(|| {
                    format!(
                        "{} has an unexpected shape; refusing to modify it",
                        config_path.display()
                    )
                })?;
            map.insert(
                SERVER_ENTRY_NAME.to_string(),
                json_entry(integration.id, url, token),
            );
            save_json_document(&config_path, &document)?;
        }
        ConfigFormat::CodexToml => {
            let mut document = load_codex_document(&config_path)?;
            let mut entry = toml_edit::Table::new();
            entry["url"] = toml_edit::value(url);
            let mut headers = toml_edit::InlineTable::new();
            headers.insert("Authorization", bearer_value(token).into());
            entry["http_headers"] = toml_edit::value(headers);
            if document.get("mcp_servers").is_none() {
                let mut servers = toml_edit::Table::new();
                servers.set_implicit(true);
                document["mcp_servers"] = toml_edit::Item::Table(servers);
            }
            document["mcp_servers"][SERVER_ENTRY_NAME] = toml_edit::Item::Table(entry);
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            }
            std::fs::write(&config_path, document.to_string())
                .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
        }
    }
    Ok(status_of(integration, home, url, token))
}

pub(super) fn remove_at(home: &Path, id: &str) -> Result<IntegrationStatus, String> {
    let integration = integration(id)?;
    let config_path = join_rel(home, integration.config_rel);
    if config_path.exists() {
        match integration.format {
            ConfigFormat::Json => {
                let mut document = load_json_document(&config_path)?;
                if let Some(map) = json_server_map(&mut document, integration.json_map_path, false)
                {
                    if map.remove(SERVER_ENTRY_NAME).is_some() {
                        save_json_document(&config_path, &document)?;
                    }
                }
            }
            ConfigFormat::CodexToml => {
                let mut document = load_codex_document(&config_path)?;
                let removed = document
                    .get_mut("mcp_servers")
                    .and_then(|item| item.as_table_mut())
                    .map(|servers| servers.remove(SERVER_ENTRY_NAME).is_some())
                    .unwrap_or(false);
                if removed {
                    std::fs::write(&config_path, document.to_string()).map_err(|e| {
                        format!("Failed to write {}: {e}", config_path.display())
                    })?;
                }
            }
        }
    }
    // State is computed against empty credentials on purpose: after a
    // removal the entry is gone, so url/token no longer matter.
    Ok(status_of(integration, home, "", ""))
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Could not determine the home directory".to_string())
}

pub fn integration_statuses(url: &str, token: &str) -> Vec<IntegrationStatus> {
    match home_dir() {
        Ok(home) => statuses_at(&home, url, token),
        Err(_) => Vec::new(),
    }
}

pub fn apply_integration(id: &str, url: &str, token: &str) -> Result<IntegrationStatus, String> {
    apply_at(&home_dir()?, id, url, token)
}

pub fn remove_integration(id: &str) -> Result<IntegrationStatus, String> {
    remove_at(&home_dir()?, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:27121/mcp";
    const TOKEN: &str = "tok-abc";

    fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp home")
    }

    #[test]
    fn apply_then_remove_roundtrips_for_every_integration() {
        let home = temp_home();
        for integration in INTEGRATIONS {
            // Mark the harness as detected.
            if integration.detect_rel.is_empty() {
                std::fs::write(join_rel(home.path(), integration.config_rel), "{}").unwrap();
            } else {
                std::fs::create_dir_all(join_rel(home.path(), integration.detect_rel)).unwrap();
            }

            let status = apply_at(home.path(), integration.id, URL, TOKEN).unwrap();
            assert_eq!(status.state, "current", "{} after apply", integration.id);
            assert!(status.detected, "{} detected", integration.id);

            // A different token classifies as stale.
            let stale = statuses_at(home.path(), URL, "other-token");
            let row = stale.iter().find(|s| s.id == integration.id).unwrap();
            assert_eq!(row.state, "stale", "{} with rotated token", integration.id);

            let removed = remove_at(home.path(), integration.id).unwrap();
            assert_eq!(removed.state, "absent", "{} after remove", integration.id);
        }
    }

    #[test]
    fn codex_toml_preserves_comments_and_other_entries() {
        let home = temp_home();
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let config = codex_dir.join("config.toml");
        std::fs::write(
            &config,
            "# my codex settings\nmodel = \"o4\"\n\n[mcp_servers.blender]\ncommand = \"uvx\"\n",
        )
        .unwrap();

        apply_at(home.path(), "codex", URL, TOKEN).unwrap();
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(written.contains("# my codex settings"), "comment survives");
        assert!(written.contains("model = \"o4\""));
        assert!(written.contains("[mcp_servers.blender]"));
        assert!(written.contains("[mcp_servers.locus]"));
        assert!(written.contains(URL));
        assert!(written.contains("Bearer tok-abc"));

        remove_at(home.path(), "codex").unwrap();
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(!written.contains("[mcp_servers.locus]"));
        assert!(written.contains("[mcp_servers.blender]"), "others survive");
    }

    #[test]
    fn corrupt_json_config_is_never_overwritten() {
        let home = temp_home();
        let path = home.path().join(".claude.json");
        std::fs::write(&path, "{ not json").unwrap();

        let error = apply_at(home.path(), "claude_code", URL, TOKEN).unwrap_err();
        assert!(error.contains("refusing to modify"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

        // Status still reports the row (as stale) instead of panicking.
        let statuses = statuses_at(home.path(), URL, TOKEN);
        let row = statuses.iter().find(|s| s.id == "claude_code").unwrap();
        assert_eq!(row.state, "stale");
    }

    #[test]
    fn claude_entry_shape_matches_import_expectations() {
        let home = temp_home();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        apply_at(home.path(), "claude_code", URL, TOKEN).unwrap();

        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        )
        .unwrap();
        let entry = &document["mcpServers"]["locus"];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], URL);
        assert_eq!(entry["headers"]["Authorization"], "Bearer tok-abc");
    }

    #[test]
    fn existing_json_keys_survive_apply() {
        let home = temp_home();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"numStartups": 4, "mcpServers": {"blender": {"command": "uvx"}}}"#,
        )
        .unwrap();
        apply_at(home.path(), "claude_code", URL, TOKEN).unwrap();

        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["numStartups"], 4);
        assert_eq!(document["mcpServers"]["blender"]["command"], "uvx");
        assert_eq!(document["mcpServers"]["locus"]["url"], URL);
    }

    #[test]
    fn gemini_uses_http_url_key() {
        let home = temp_home();
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        apply_at(home.path(), "gemini", URL, TOKEN).unwrap();
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".gemini").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["mcpServers"]["locus"]["httpUrl"], URL);
        assert!(document["mcpServers"]["locus"].get("url").is_none());
    }
}
