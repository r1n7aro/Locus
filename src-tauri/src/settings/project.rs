//! Project-scope settings, stored in `<project>/Locus/config.json` under
//! their historical field names so the file stays readable by every Locus
//! version and can be shared with the repository.

use serde_json::{Map, Value};

use super::definitions::DEFINITIONS;
use super::schema::{Key, Scope, SettingValue};
use super::store::{SettingChange, SettingsError};

fn camel_case(field: &str) -> String {
    let mut out = String::with_capacity(field.len());
    let mut upper = false;
    for ch in field.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn read_object(root: &str) -> Result<Map<String, Value>, String> {
    let path = crate::workspace::workspace_config_path(root);
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(format!("Failed to read workspace config: {error}")),
    };
    match serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err("Workspace config must be a JSON object".to_string()),
        Err(error) => Err(format!("Failed to parse workspace config: {error}")),
    }
}

fn project_field(index: usize) -> Option<&'static str> {
    let def = DEFINITIONS.get(index)?;
    (def.scope == Scope::Project).then_some(def.project_field?)
}

/// Effective value of a project setting for the project at `root`.
pub fn value_at(root: &str, index: usize) -> Value {
    let def = &DEFINITIONS[index];
    let Some(field) = project_field(index) else {
        return def.default_value();
    };
    if root.trim().is_empty() {
        return def.default_value();
    }
    read_object(root)
        .ok()
        .and_then(|object| {
            object
                .get(field)
                .or_else(|| object.get(&camel_case(field)))
                .cloned()
        })
        .and_then(|value| def.kind.normalize(&value).ok())
        .unwrap_or_else(|| def.default_value())
}

pub fn get<T: SettingValue>(root: &str, key: Key<T>) -> T {
    T::from_setting(&value_at(root, key.index()))
        .or_else(|| T::from_setting(&DEFINITIONS[key.index()].default_value()))
        .expect("setting default matches its declared Rust type")
}

pub fn write(root: &str, changes: &[(usize, Value)]) -> Result<Vec<SettingChange>, SettingsError> {
    let mut normalized = Vec::with_capacity(changes.len());
    for (index, value) in changes {
        let def = DEFINITIONS
            .get(*index)
            .ok_or_else(|| SettingsError::unknown_key(&index.to_string()))?;
        let field = project_field(*index).ok_or_else(|| SettingsError::unknown_key(def.key))?;
        let value = def
            .normalize(value)
            .map_err(|reason| SettingsError::invalid_value(def.key, &reason))?;
        normalized.push((*index, field, value));
    }

    let mut object = read_object(root).map_err(SettingsError::persistence)?;
    let mut applied = Vec::with_capacity(normalized.len());
    for (index, field, value) in &normalized {
        let old = value_at(root, *index);
        object.remove(&camel_case(field));
        object.insert(field.to_string(), value.clone());
        applied.push(SettingChange {
            key: DEFINITIONS[*index].key,
            old,
            new: value.clone(),
        });
    }
    let json = serde_json::to_string_pretty(&Value::Object(object))
        .map_err(|error| SettingsError::persistence(error.to_string()))?;
    crate::config::atomic_write_config(&crate::workspace::workspace_config_path(root), json.as_bytes())
        .map_err(SettingsError::persistence)?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::definitions::*;
    use serde_json::json;

    #[test]
    fn reads_historical_fields_and_writes_them_back_in_place() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let path = crate::workspace::workspace_config_path(&root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{ "workspace_id": "git-1", "knowledgeEnabled": false, "other": 1 }"#,
        )
        .unwrap();

        assert!(!get(&root, PROJECT_KNOWLEDGE_ENABLED));
        assert!(!get(&root, PROJECT_UNITY_TEST_TOOLS_ENABLED));

        let changes = write(
            &root,
            &[
                (PROJECT_KNOWLEDGE_ENABLED.index(), json!(true)),
                (PROJECT_UNITY_TEST_TOOLS_ENABLED.index(), json!(true)),
            ],
        )
        .unwrap();
        assert_eq!(changes[0].old, json!(false));
        assert_eq!(changes[0].new, json!(true));

        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({
                "workspace_id": "git-1",
                "knowledge_enabled": true,
                "unity_test_tools_enabled": true,
                "other": 1
            })
        );
    }

    #[test]
    fn missing_project_config_uses_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        assert!(get(&root, PROJECT_KNOWLEDGE_ENABLED));
        assert!(!get(&root, PROJECT_UNITY_TEST_TOOLS_ENABLED));
        // No project: the definition default, as for a request without a workspace.
        assert!(get("", PROJECT_KNOWLEDGE_ENABLED));
    }

    #[test]
    fn app_settings_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let error = write(&root, &[(SYSTEM_DEBUG.index(), json!(true))]).unwrap_err();
        assert_eq!(error.code, "settings.unknown_key");
    }
}
