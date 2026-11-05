//! Agent access to settings: `config_query` output and `config_set`
//! parsing and confirmation previews. Every setting is readable and
//! writable; sensitive settings need user confirmation by default
//! (`behavior.sensitive_settings_change`), normal ones follow
//! `behavior.settings_change` (default: no confirmation).

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::catalog;
use super::definitions::DEFINITIONS;
use super::schema::{Scope, Sensitivity};
use super::store::index_of;
use crate::config::AppConfig;

pub const BEHAVIOR_SETTINGS_CHANGE: &str = "behavior.settings_change";
pub const BEHAVIOR_SENSITIVE_SETTINGS_CHANGE: &str = "behavior.sensitive_settings_change";

#[derive(Debug, Clone)]
pub struct ConfigSetRequest {
    pub changes: Vec<(usize, Value)>,
    pub reason: String,
}

impl ConfigSetRequest {
    pub fn is_sensitive(&self) -> bool {
        self.changes
            .iter()
            .any(|(index, _)| DEFINITIONS[*index].sensitivity == Sensitivity::Sensitive)
    }

}

/// Parses `{ "changes": [{ "key", "value" }], "reason" }`. Values are
/// type-checked here so invalid calls fail before any confirmation prompt;
/// write-time validators (existing directories, package ids) run on apply.
pub fn parse_config_set(args: &Value) -> Result<ConfigSetRequest, String> {
    // Models that call the deferred tool before its schema is loaded tend to
    // pass one change at the top level; accept that shape too.
    let single;
    let entries = match args.get("changes") {
        None if args.get("key").is_some() => {
            single = [args.clone()];
            &single[..]
        }
        changes => changes
            .and_then(Value::as_array)
            .filter(|entries| !entries.is_empty())
            .map(Vec::as_slice)
            .ok_or_else(|| {
                "config_set needs `changes`: [{ \"key\": ..., \"value\": ... }]".to_string()
            })?,
    };
    let mut changes = Vec::with_capacity(entries.len());
    for entry in entries {
        let key = entry
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| "each change needs a `key`".to_string())?;
        let value = entry
            .get("value")
            .ok_or_else(|| format!("change for '{key}' needs a `value`"))?;
        let index = index_of(key).ok_or_else(|| {
            format!("Unknown setting '{key}'. Use config_query to list setting keys.")
        })?;
        let value = DEFINITIONS[index]
            .kind
            .normalize(value)
            .map_err(|reason| format!("Invalid value for '{key}': {reason}."))?;
        if changes.iter().any(|(existing, _)| *existing == index) {
            return Err(format!("'{key}' appears more than once"));
        }
        changes.push((index, value));
    }
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    Ok(ConfigSetRequest { changes, reason })
}

fn behavior_requires_confirm(
    permissions: &HashMap<String, String>,
    behavior: &str,
    default: bool,
) -> bool {
    match permissions.get(behavior).map(|mode| mode.trim()) {
        Some(mode) if mode.eq_ignore_ascii_case("auto") => false,
        Some(mode) if mode.eq_ignore_ascii_case("ask") => true,
        _ => default,
    }
}

/// Whether a change needs the user's confirmation under the current tool
/// permissions. Callers without a confirmation UI (the SDK) reject such
/// changes instead of applying them unconfirmed.
pub fn requires_confirmation(
    request: &ConfigSetRequest,
    permissions: &HashMap<String, String>,
) -> bool {
    behavior_requires_confirm(permissions, BEHAVIOR_SETTINGS_CHANGE, false)
        || (request.is_sensitive()
            && behavior_requires_confirm(permissions, BEHAVIOR_SENSITIVE_SETTINGS_CHANGE, true))
}

fn current_value(config: &AppConfig, workspace_root: Option<&str>, index: usize) -> Value {
    let def = &DEFINITIONS[index];
    match (def.scope, workspace_root) {
        (Scope::App, _) => config.settings().value_at(index),
        (Scope::Project, Some(root)) => super::project::value_at(root, index),
        (Scope::Project, None) => def.default_value(),
    }
}

/// Confirmation card content: what changes, from what, and why it is
/// considered sensitive.
pub fn confirmation_preview(
    config: &AppConfig,
    workspace_root: Option<&str>,
    request: &ConfigSetRequest,
) -> String {
    let changes = request
        .changes
        .iter()
        .map(|(index, value)| {
            let def = &DEFINITIONS[*index];
            let mut entry = Map::new();
            entry.insert("key".into(), Value::from(def.key));
            if let Some(label) = catalog::label(def.key) {
                entry.insert("setting".into(), Value::from(label));
            }
            entry.insert(
                "from".into(),
                current_value(config, workspace_root, *index),
            );
            entry.insert("to".into(), value.clone());
            if def.sensitivity == Sensitivity::Sensitive {
                entry.insert("sensitive".into(), Value::Bool(true));
            }
            if def.scope == Scope::Project {
                entry.insert("scope".into(), Value::from("project"));
            }
            if def.restart {
                entry.insert("requiresRestart".into(), Value::Bool(true));
            }
            Value::Object(entry)
        })
        .collect::<Vec<_>>();
    let mut preview = json!({ "changes": changes });
    if !request.reason.is_empty() {
        preview["reason"] = Value::from(request.reason.as_str());
    }
    serde_json::to_string_pretty(&preview).unwrap_or_default()
}

/// `config_set` result text.
pub fn describe_changes(changes: &[super::store::SettingChange]) -> String {
    let lines = changes
        .iter()
        .map(|change| {
            let restart = super::store::definition(change.key)
                .is_some_and(|def| def.restart && change.old != change.new);
            let mut line = if change.old == change.new {
                format!("- {}: unchanged ({})", change.key, change.new)
            } else {
                format!("- {}: {} -> {}", change.key, change.old, change.new)
            };
            if restart {
                line.push_str(" (applies after restarting Locus)");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("Settings updated:\n{lines}")
}

fn matches_filter(key: &str, prefix: Option<&str>, keys: Option<&[String]>) -> bool {
    if let Some(keys) = keys {
        return keys.iter().any(|candidate| candidate == key);
    }
    match prefix {
        Some(prefix) => key == prefix || key.starts_with(&format!("{prefix}.")),
        None => true,
    }
}

struct QueryFilter {
    prefix: Option<String>,
    keys: Option<Vec<String>>,
}

impl QueryFilter {
    fn parse(args: &Value) -> Result<Self, String> {
        let prefix = args
            .get("prefix")
            .or_else(|| args.get("category"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let keys = args.get("keys").and_then(Value::as_array).map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(|key| key.trim().to_string())
                .collect::<Vec<_>>()
        });
        if let Some(unknown) = keys
            .iter()
            .flatten()
            .find(|key| index_of(key).is_none())
        {
            return Err(format!(
                "Unknown setting '{unknown}'. Use config_query without keys to list settings."
            ));
        }
        Ok(Self { prefix, keys })
    }

    fn matches(&self, key: &str) -> bool {
        matches_filter(key, self.prefix.as_deref(), self.keys.as_deref())
    }
}

/// One JSON object per matching setting, in definition order.
fn setting_entries(
    config: &AppConfig,
    workspace_root: Option<&str>,
    filter: &QueryFilter,
) -> Vec<Value> {
    DEFINITIONS
        .iter()
        .enumerate()
        .filter(|(_, def)| filter.matches(def.key))
        .map(|(index, def)| {
            let mut entry = Map::new();
            entry.insert("key".into(), Value::from(def.key));
            entry.insert(
                "value".into(),
                current_value(config, workspace_root, index),
            );
            entry.insert("default".into(), def.default_value());
            entry.insert("type".into(), def.kind.describe());
            if let Some(label) = catalog::label(def.key) {
                entry.insert("label".into(), Value::from(label));
            }
            if let Some(description) = catalog::description(def.key) {
                entry.insert("description".into(), Value::from(description));
            }
            entry.insert(
                "scope".into(),
                Value::from(match def.scope {
                    Scope::App => "app",
                    Scope::Project => "project",
                }),
            );
            entry.insert(
                "sensitive".into(),
                Value::Bool(def.sensitivity == Sensitivity::Sensitive),
            );
            entry.insert("experimental".into(), Value::Bool(def.experimental));
            entry.insert("requiresRestart".into(), Value::Bool(def.restart));
            Value::Object(entry)
        })
        .collect()
}

/// Configuration not in the settings table yet; skipped when exact keys are
/// requested.
fn other_entries(
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
    filter: &QueryFilter,
) -> Result<Vec<crate::config_registry::ConfigEntry>, String> {
    if filter.keys.is_some() {
        return Ok(Vec::new());
    }
    let entries = crate::config_registry::collect_all(workspace_scope)
        .map_err(|error| error.to_string())?;
    Ok(entries
        .into_iter()
        .filter(|entry| {
            filter.matches(&entry.key) || filter.prefix.as_deref() == Some(entry.category.as_str())
        })
        .collect())
}

fn render_setting(entry: &Value, has_workspace: bool) -> String {
    let kind = &entry["type"];
    let mut facts = vec![kind["type"].as_str().unwrap_or_default().to_string()];
    if let Some(values) = kind.get("values") {
        facts.push(format!("one of {values}"));
    } else if let (Some(min), Some(max)) = (kind.get("min"), kind.get("max")) {
        facts.push(format!("{min}..{max}"));
    }
    if kind.get("nullable") == Some(&Value::Bool(true)) {
        facts.push("null = not set".into());
    }
    if let Some(value_type) = kind.get("valueType") {
        let mut value_type_text = value_type["type"].as_str().unwrap_or_default().to_string();
        if let Some(values) = value_type.get("values") {
            value_type_text.push_str(&format!(" {values}"));
        }
        facts.push(format!("values: {value_type_text}"));
    }
    if entry["value"] != entry["default"] {
        facts.push(format!("default {}", entry["default"]));
    }
    if entry["sensitive"] == Value::Bool(true) {
        facts.push("sensitive: changes need user confirmation".into());
    }
    if entry["experimental"] == Value::Bool(true) {
        facts.push("experimental".into());
    }
    if entry["requiresRestart"] == Value::Bool(true) {
        facts.push("applies after restart".into());
    }
    if entry["scope"] == "project" {
        facts.push(if has_workspace {
            "project setting (Locus/config.json)".into()
        } else {
            "project setting; no workspace, showing default".into()
        });
    }
    let mut line = format!(
        "- `{}` = {} ({})",
        entry["key"].as_str().unwrap_or_default(),
        entry["value"],
        facts.join("; ")
    );
    if let Some(label) = entry["label"].as_str() {
        line.push_str(&format!("\n  {label}"));
        if let Some(description) = entry["description"].as_str() {
            line.push_str(&format!(": {description}"));
        }
    }
    line.push('\n');
    line
}

/// `config_query` agent output: the settings table, followed by
/// configuration that is not in the table yet (credential status, model
/// defaults, tool permissions, MCP servers, UI preferences).
pub fn query(
    config: &AppConfig,
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
    args: &Value,
) -> Result<String, String> {
    let filter = QueryFilter::parse(args)?;
    let workspace_root =
        workspace_scope.map(|scope| scope.runtime().root().to_string_lossy().to_string());

    let mut out = String::new();
    let mut current_domain = String::new();
    for entry in setting_entries(config, workspace_root.as_deref(), &filter) {
        let domain = entry["key"]
            .as_str()
            .and_then(|key| key.split('.').next())
            .unwrap_or_default()
            .to_string();
        if domain != current_domain {
            out.push_str(&format!("\n## {domain}\n"));
            current_domain = domain;
        }
        out.push_str(&render_setting(&entry, workspace_root.is_some()));
    }

    let mut other = String::new();
    let mut current_category = String::new();
    for entry in other_entries(workspace_scope, &filter)? {
        if entry.category != current_category {
            other.push_str(&format!("\n### {}\n", entry.category));
            current_category = entry.category.clone();
        }
        other.push_str(&format!(
            "- {} (`{}`) = {}\n  {} Storage: {}\n",
            entry.label, entry.key, entry.current_value, entry.description, entry.storage
        ));
    }
    if !other.is_empty() {
        out.push_str(
            "\n## Related configuration and status (read-only; change values through the setting keys above)\n",
        );
        out.push_str(&other);
    }

    if out.trim().is_empty() {
        return Ok("No settings match this filter.".to_string());
    }
    Ok(format!(
        "Settings (change with config_set using these keys):{out}"
    ))
}

/// `config_query` for the Python SDK: `{ settings: [...], other: [...] }`.
pub fn query_json(
    config: &AppConfig,
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
    args: &Value,
) -> Result<Value, String> {
    let filter = QueryFilter::parse(args)?;
    let workspace_root =
        workspace_scope.map(|scope| scope.runtime().root().to_string_lossy().to_string());
    Ok(json!({
        "settings": setting_entries(config, workspace_root.as_deref(), &filter),
        "other": other_entries(workspace_scope, &filter)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::definitions::*;

    #[test]
    fn parses_and_type_checks_changes() {
        let request = parse_config_set(&json!({
            "changes": [
                { "key": "agent.subagent.maxDepth", "value": 2 },
                { "key": "permissions.fileWorkspaceBoundary", "value": true }
            ],
            "reason": "user asked"
        }))
        .unwrap();
        assert_eq!(request.changes[0], (AGENT_SUBAGENT_MAX_DEPTH.index(), json!(2)));
        assert!(request.is_sensitive());
        assert_eq!(request.reason, "user asked");

        let normal = parse_config_set(&json!({
            "changes": [{ "key": "agent.subagent.maxDepth", "value": 2 }]
        }))
        .unwrap();
        assert!(!normal.is_sensitive());
    }

    #[test]
    fn accepts_a_single_top_level_change() {
        let request = parse_config_set(&json!({
            "key": "agent.subagent.maxDepth",
            "value": 2,
            "reason": "shorthand"
        }))
        .unwrap();
        assert_eq!(request.changes, vec![(AGENT_SUBAGENT_MAX_DEPTH.index(), json!(2))]);
        assert_eq!(request.reason, "shorthand");
    }

    #[test]
    fn rejects_unknown_keys_bad_values_and_duplicates() {
        assert!(parse_config_set(&json!({ "changes": [] })).is_err());
        assert!(parse_config_set(&json!({ "changes": [{ "key": "nope", "value": 1 }] }))
            .unwrap_err()
            .contains("Unknown setting"));
        assert!(parse_config_set(&json!({
            "changes": [{ "key": "agent.subagent.maxDepth", "value": 99 }]
        }))
        .unwrap_err()
        .contains("between 1 and 8"));
        assert!(parse_config_set(&json!({
            "changes": [
                { "key": "system.debug", "value": true },
                { "key": "system.debug", "value": false }
            ]
        }))
        .unwrap_err()
        .contains("more than once"));
    }

    #[test]
    fn preview_shows_current_and_requested_values() {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig::load_from_path(&temp.path().join("config.json"));
        let request = parse_config_set(&json!({
            "changes": [{ "key": "system.debug", "value": true }],
            "reason": "investigate a stall"
        }))
        .unwrap();
        let preview: Value =
            serde_json::from_str(&confirmation_preview(&config, None, &request)).unwrap();
        assert_eq!(preview["changes"][0]["from"], json!(false));
        assert_eq!(preview["changes"][0]["to"], json!(true));
        assert_eq!(preview["changes"][0]["sensitive"], json!(true));
        assert_eq!(preview["changes"][0]["setting"], json!("Debug Mode"));
        assert_eq!(preview["reason"], json!("investigate a stall"));
    }

    #[test]
    fn sensitive_changes_ask_by_default_and_normal_changes_follow_their_behavior() {
        let normal = parse_config_set(&json!({
            "changes": [{ "key": "agent.subagent.maxDepth", "value": 2 }]
        }))
        .unwrap();
        let sensitive = parse_config_set(&json!({
            "changes": [{ "key": "system.debug", "value": true }]
        }))
        .unwrap();
        let mut permissions = HashMap::new();
        assert!(!requires_confirmation(&normal, &permissions));
        assert!(requires_confirmation(&sensitive, &permissions));

        permissions.insert(BEHAVIOR_SENSITIVE_SETTINGS_CHANGE.to_string(), "auto".to_string());
        assert!(!requires_confirmation(&sensitive, &permissions));

        permissions.insert(BEHAVIOR_SETTINGS_CHANGE.to_string(), "ask".to_string());
        assert!(requires_confirmation(&normal, &permissions));
        assert!(requires_confirmation(&sensitive, &permissions));
    }

    #[test]
    fn filters_by_key_prefix_on_segment_boundaries() {
        assert!(matches_filter("agent.subagent.maxDepth", Some("agent"), None));
        assert!(matches_filter("agent.subagent.maxDepth", Some("agent.subagent"), None));
        assert!(!matches_filter("agent.subagent.maxDepth", Some("agent.sub"), None));
        assert!(matches_filter(
            "system.debug",
            None,
            Some(&["system.debug".to_string()])
        ));
    }
}
