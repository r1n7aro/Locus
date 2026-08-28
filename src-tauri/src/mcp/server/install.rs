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
use std::sync::Mutex;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationTarget {
    pub checkout_id: String,
    pub workspace_generation: u64,
    pub endpoint_url: String,
    pub entry_name: String,
}

impl IntegrationTarget {
    pub fn new(
        checkout_id: impl Into<String>,
        workspace_generation: u64,
        endpoint_url: impl Into<String>,
    ) -> Self {
        let checkout_id = checkout_id.into();
        let entry_name = scoped_entry_name(&checkout_id);
        Self {
            checkout_id,
            workspace_generation,
            endpoint_url: endpoint_url.into(),
            entry_name,
        }
    }
}

fn scoped_entry_name(checkout_id: &str) -> String {
    let normalized = checkout_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('-');
    let checkout = if normalized.is_empty() {
        "checkout"
    } else {
        normalized
    };
    format!("locus-{checkout}")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    /// Scoped row identity. External integrations remain independently
    /// addressable for each checkout generation.
    pub id: String,
    pub integration_id: String,
    pub name: String,
    pub entry_name: String,
    pub checkout_id: String,
    pub workspace_generation: u64,
    pub endpoint_url: String,
    pub config_path: String,
    /// The harness appears to be installed on this machine.
    pub detected: bool,
    /// "absent" | "current" | "stale"
    pub state: String,
}

fn integration_config_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
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

fn load_codex_document(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    if !path.exists() {
        return Ok(toml_edit::DocumentMut::new());
    }
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    data.parse::<toml_edit::DocumentMut>().map_err(|e| {
        format!(
            "{} is not valid TOML ({e}); refusing to modify it",
            path.display()
        )
    })
}

fn codex_entry_value(document: &toml_edit::DocumentMut, entry_name: &str) -> Option<Value> {
    let entry = document
        .get("mcp_servers")
        .and_then(|servers| servers.get(entry_name))?;
    let url = entry.get("url").and_then(|item| item.as_str())?;
    let auth = entry
        .get("http_headers")
        .and_then(|headers| headers.get("Authorization"))
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    Some(json!({ "url": url, "headers": { "Authorization": auth } }))
}

fn current_entry(
    integration: &Integration,
    config_path: &Path,
    entry_name: &str,
) -> Result<Option<Value>, String> {
    match integration.format {
        ConfigFormat::Json => {
            let mut document = load_json_document(config_path)?;
            Ok(
                json_server_map(&mut document, integration.json_map_path, false)
                    .and_then(|map| map.get(entry_name).cloned()),
            )
        }
        ConfigFormat::CodexToml => Ok(codex_entry_value(
            &load_codex_document(config_path)?,
            entry_name,
        )),
    }
}

fn status_of(
    integration: &Integration,
    home: &Path,
    target: &IntegrationTarget,
    token: &str,
) -> IntegrationStatus {
    let config_path = join_rel(home, integration.config_rel);
    let detected = if integration.detect_rel.is_empty() {
        config_path.exists()
    } else {
        join_rel(home, integration.detect_rel).exists()
    };
    let state = match current_entry(integration, &config_path, &target.entry_name) {
        Ok(entry) => classify_entry(entry.as_ref(), &target.endpoint_url, token).to_string(),
        // Unparsable config: surface as stale so the UI offers an update,
        // which will then fail with the parse error message.
        Err(_) => "stale".to_string(),
    };
    IntegrationStatus {
        id: format!(
            "{}:{}:g{}",
            integration.id, target.entry_name, target.workspace_generation
        ),
        integration_id: integration.id.to_string(),
        name: integration.name.to_string(),
        entry_name: target.entry_name.clone(),
        checkout_id: target.checkout_id.clone(),
        workspace_generation: target.workspace_generation,
        endpoint_url: target.endpoint_url.clone(),
        config_path: config_path.display().to_string(),
        detected,
        state,
    }
}

pub(super) fn statuses_at(
    home: &Path,
    target: &IntegrationTarget,
    token: &str,
) -> Vec<IntegrationStatus> {
    let _guard = integration_config_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    statuses_at_unlocked(home, target, token)
}

fn statuses_at_unlocked(
    home: &Path,
    target: &IntegrationTarget,
    token: &str,
) -> Vec<IntegrationStatus> {
    INTEGRATIONS
        .iter()
        .map(|integration| status_of(integration, home, target, token))
        .collect()
}

pub(super) fn apply_at(
    home: &Path,
    id: &str,
    target: &IntegrationTarget,
    token: &str,
) -> Result<IntegrationStatus, String> {
    let _guard = integration_config_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let map = json_server_map(&mut document, integration.json_map_path, true).ok_or_else(
                || {
                    format!(
                        "{} has an unexpected shape; refusing to modify it",
                        config_path.display()
                    )
                },
            )?;
            map.insert(
                target.entry_name.clone(),
                json_entry(integration.id, &target.endpoint_url, token),
            );
            save_json_document(&config_path, &document)?;
        }
        ConfigFormat::CodexToml => {
            let mut document = load_codex_document(&config_path)?;
            let mut entry = toml_edit::Table::new();
            entry["url"] = toml_edit::value(&target.endpoint_url);
            let mut headers = toml_edit::InlineTable::new();
            headers.insert("Authorization", bearer_value(token).into());
            entry["http_headers"] = toml_edit::value(headers);
            if document.get("mcp_servers").is_none() {
                let mut servers = toml_edit::Table::new();
                servers.set_implicit(true);
                document["mcp_servers"] = toml_edit::Item::Table(servers);
            }
            document["mcp_servers"][&target.entry_name] = toml_edit::Item::Table(entry);
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
            }
            std::fs::write(&config_path, document.to_string())
                .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
        }
    }
    Ok(status_of(integration, home, target, token))
}

pub(super) fn remove_at(
    home: &Path,
    id: &str,
    target: &IntegrationTarget,
) -> Result<IntegrationStatus, String> {
    let _guard = integration_config_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let integration = integration(id)?;
    let config_path = join_rel(home, integration.config_rel);
    if config_path.exists() {
        match integration.format {
            ConfigFormat::Json => {
                let mut document = load_json_document(&config_path)?;
                if let Some(map) = json_server_map(&mut document, integration.json_map_path, false)
                {
                    if map.remove(&target.entry_name).is_some() {
                        save_json_document(&config_path, &document)?;
                    }
                }
            }
            ConfigFormat::CodexToml => {
                let mut document = load_codex_document(&config_path)?;
                let removed = document
                    .get_mut("mcp_servers")
                    .and_then(|item| item.as_table_mut())
                    .map(|servers| servers.remove(&target.entry_name).is_some())
                    .unwrap_or(false);
                if removed {
                    std::fs::write(&config_path, document.to_string())
                        .map_err(|e| format!("Failed to write {}: {e}", config_path.display()))?;
                }
            }
        }
    }
    // State is computed against empty credentials on purpose: after a
    // removal the entry is gone, so url/token no longer matter.
    Ok(status_of(integration, home, target, ""))
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Could not determine the home directory".to_string())
}

pub fn integration_statuses(target: &IntegrationTarget, token: &str) -> Vec<IntegrationStatus> {
    match home_dir() {
        Ok(home) => statuses_at(&home, target, token),
        Err(_) => Vec::new(),
    }
}

pub fn apply_integration(
    id: &str,
    target: &IntegrationTarget,
    token: &str,
) -> Result<IntegrationStatus, String> {
    apply_at(&home_dir()?, id, target, token)
}

pub fn remove_integration(
    id: &str,
    target: &IntegrationTarget,
) -> Result<IntegrationStatus, String> {
    remove_at(&home_dir()?, id, target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:27121/mcp";
    const TOKEN: &str = "tok-abc";

    fn target(checkout_id: &str, generation: u64) -> IntegrationTarget {
        IntegrationTarget::new(
            checkout_id,
            generation,
            format!("{URL}?checkoutId={checkout_id}&workspaceGeneration={generation}"),
        )
    }

    fn temp_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp home")
    }

    #[test]
    fn apply_then_remove_roundtrips_for_every_integration() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        for integration in INTEGRATIONS {
            // Mark the harness as detected.
            if integration.detect_rel.is_empty() {
                std::fs::write(join_rel(home.path(), integration.config_rel), "{}").unwrap();
            } else {
                std::fs::create_dir_all(join_rel(home.path(), integration.detect_rel)).unwrap();
            }

            let status = apply_at(home.path(), integration.id, &target, TOKEN).unwrap();
            assert_eq!(status.state, "current", "{} after apply", integration.id);
            assert!(status.detected, "{} detected", integration.id);
            assert_eq!(status.checkout_id, "checkout-a");
            assert_eq!(status.workspace_generation, 7);
            assert!(status.endpoint_url.contains("workspaceGeneration=7"));
            assert!(status.id.contains(&target.entry_name));

            // A different token classifies as stale.
            let stale = statuses_at(home.path(), &target, "other-token");
            let row = stale
                .iter()
                .find(|s| s.integration_id == integration.id)
                .unwrap();
            assert_eq!(row.state, "stale", "{} with rotated token", integration.id);

            let removed = remove_at(home.path(), integration.id, &target).unwrap();
            assert_eq!(removed.state, "absent", "{} after remove", integration.id);
        }
    }

    #[test]
    fn codex_toml_preserves_comments_and_other_entries() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        let codex_dir = home.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let config = codex_dir.join("config.toml");
        std::fs::write(
            &config,
            "# my codex settings\nmodel = \"o4\"\n\n[mcp_servers.blender]\ncommand = \"uvx\"\n",
        )
        .unwrap();

        apply_at(home.path(), "codex", &target, TOKEN).unwrap();
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(written.contains("# my codex settings"), "comment survives");
        assert!(written.contains("model = \"o4\""));
        assert!(written.contains("[mcp_servers.blender]"));
        assert!(written.contains(&format!("[mcp_servers.{}]", target.entry_name)));
        assert!(written.contains(&target.endpoint_url));
        assert!(written.contains("Bearer tok-abc"));

        remove_at(home.path(), "codex", &target).unwrap();
        let written = std::fs::read_to_string(&config).unwrap();
        assert!(!written.contains(&format!("[mcp_servers.{}]", target.entry_name)));
        assert!(written.contains("[mcp_servers.blender]"), "others survive");
    }

    #[test]
    fn corrupt_json_config_is_never_overwritten() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        let path = home.path().join(".claude.json");
        std::fs::write(&path, "{ not json").unwrap();

        let error = apply_at(home.path(), "claude_code", &target, TOKEN).unwrap_err();
        assert!(error.contains("refusing to modify"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

        // Status still reports the row (as stale) instead of panicking.
        let statuses = statuses_at(home.path(), &target, TOKEN);
        let row = statuses
            .iter()
            .find(|s| s.integration_id == "claude_code")
            .unwrap();
        assert_eq!(row.state, "stale");
    }

    #[test]
    fn claude_entry_shape_matches_import_expectations() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        apply_at(home.path(), "claude_code", &target, TOKEN).unwrap();

        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        )
        .unwrap();
        let entry = &document["mcpServers"][&target.entry_name];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["url"], target.endpoint_url);
        assert_eq!(entry["headers"]["Authorization"], "Bearer tok-abc");
    }

    #[test]
    fn existing_json_keys_survive_apply() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"numStartups": 4, "mcpServers": {"blender": {"command": "uvx"}}}"#,
        )
        .unwrap();
        apply_at(home.path(), "claude_code", &target, TOKEN).unwrap();

        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".claude.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(document["numStartups"], 4);
        assert_eq!(document["mcpServers"]["blender"]["command"], "uvx");
        assert_eq!(
            document["mcpServers"][&target.entry_name]["url"],
            target.endpoint_url
        );
    }

    #[test]
    fn gemini_uses_http_url_key() {
        let home = temp_home();
        let target = target("checkout-a", 7);
        std::fs::create_dir_all(home.path().join(".gemini")).unwrap();
        apply_at(home.path(), "gemini", &target, TOKEN).unwrap();
        let document: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join(".gemini").join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            document["mcpServers"][&target.entry_name]["httpUrl"],
            target.endpoint_url
        );
        assert!(document["mcpServers"][&target.entry_name]
            .get("url")
            .is_none());
    }

    #[test]
    fn checkout_entry_names_are_stable_and_generation_remains_in_the_endpoint() {
        let checkout_a = target("checkout-a", 7);
        let checkout_b = target("checkout-b", 7);
        let checkout_a_next = target("checkout-a", 8);
        assert_ne!(checkout_a.entry_name, checkout_b.entry_name);
        assert_eq!(checkout_a.entry_name, checkout_a_next.entry_name);
        assert_ne!(checkout_a.endpoint_url, checkout_a_next.endpoint_url);
        assert!(checkout_a.entry_name.contains("checkout-a"));
        assert!(checkout_a.endpoint_url.ends_with("workspaceGeneration=7"));
    }

    #[test]
    fn recreated_checkout_updates_its_stable_entry_without_leaving_a_stale_key() {
        let home = temp_home();
        let config_path = home.path().join(".claude.json");
        std::fs::write(&config_path, "{}").unwrap();
        let generation_7 = target("checkout-a", 7);
        let generation_8 = target("checkout-a", 8);

        apply_at(home.path(), "claude_code", &generation_7, TOKEN).unwrap();
        apply_at(home.path(), "claude_code", &generation_8, TOKEN).unwrap();

        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        let entries = document["mcpServers"].as_object().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[&generation_8.entry_name]["url"],
            generation_8.endpoint_url
        );
        assert_eq!(
            statuses_at(home.path(), &generation_7, TOKEN)
                .into_iter()
                .find(|status| status.integration_id == "claude_code")
                .unwrap()
                .state,
            "stale"
        );
        assert_eq!(
            statuses_at(home.path(), &generation_8, TOKEN)
                .into_iter()
                .find(|status| status.integration_id == "claude_code")
                .unwrap()
                .state,
            "current"
        );
    }

    #[test]
    fn concurrent_checkout_apply_and_targeted_remove_preserve_siblings() {
        let home = temp_home();
        let config_path = home.path().join(".claude.json");
        std::fs::write(&config_path, "{}").unwrap();

        let home_a = home.path().to_path_buf();
        let home_b = home.path().to_path_buf();
        let target_a = target("checkout-a", 11);
        let target_b = target("checkout-b", 23);
        let target_a_thread = target_a.clone();
        let target_b_thread = target_b.clone();

        let apply_a = std::thread::spawn(move || {
            apply_at(&home_a, "claude_code", &target_a_thread, TOKEN).unwrap()
        });
        let apply_b = std::thread::spawn(move || {
            apply_at(&home_b, "claude_code", &target_b_thread, TOKEN).unwrap()
        });
        assert_eq!(apply_a.join().unwrap().state, "current");
        assert_eq!(apply_b.join().unwrap().state, "current");

        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(
            document["mcpServers"][&target_a.entry_name]["url"],
            target_a.endpoint_url
        );
        assert_eq!(
            document["mcpServers"][&target_b.entry_name]["url"],
            target_b.endpoint_url
        );

        let removed = remove_at(home.path(), "claude_code", &target_a).unwrap();
        assert_eq!(removed.state, "absent");
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert!(document["mcpServers"].get(&target_a.entry_name).is_none());
        assert_eq!(
            document["mcpServers"][&target_b.entry_name]["url"],
            target_b.endpoint_url
        );
        let sibling = statuses_at(home.path(), &target_b, TOKEN)
            .into_iter()
            .find(|status| status.integration_id == "claude_code")
            .unwrap();
        assert_eq!(sibling.state, "current");
    }
}
