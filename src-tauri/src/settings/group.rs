//! Settings that a module reads and writes as one object, e.g. every key
//! under `providers.codex` as `{ "transport": ..., "contextWindow": ... }`.
//! Field names follow the keys, so a module's camelCase serde struct maps
//! directly onto its group.

use serde_json::{Map, Value};

use super::definitions::DEFINITIONS;
use super::schema::Scope;
use super::store::{insert_path, lookup, SettingsStore};

fn relative_key<'a>(key: &'a str, prefix: &str) -> Option<&'a str> {
    key.strip_prefix(prefix)?.strip_prefix('.')
}

/// Effective values of every app setting under `prefix`, nested by the
/// remaining key segments.
pub fn read(store: &SettingsStore, prefix: &str) -> Value {
    let mut object = Map::new();
    for (index, def) in DEFINITIONS.iter().enumerate() {
        if def.scope != Scope::App {
            continue;
        }
        if let Some(relative) = relative_key(def.key, prefix) {
            insert_path(&mut object, relative, store.value_at(index));
        }
    }
    Value::Object(object)
}

/// Changes for the settings under `prefix` that `value` provides; fields
/// that are not settings are ignored.
pub fn changes(prefix: &str, value: &Value) -> Vec<(usize, Value)> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    DEFINITIONS
        .iter()
        .enumerate()
        .filter(|(_, def)| def.scope == Scope::App)
        .filter_map(|(index, def)| {
            let relative = relative_key(def.key, prefix)?;
            lookup(object, relative).map(|value| (index, value.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_a_group_as_one_object() {
        let store = SettingsStore::in_memory(Map::new());
        let group = read(&store, "unity.hotReload");
        assert_eq!(group, json!({ "enabled": false, "inlineForceEvaluate": true }));

        let requested = changes(
            "unity.hotReload",
            &json!({ "enabled": true, "notASetting": 1 }),
        );
        store.set_values(&requested).unwrap();
        assert_eq!(read(&store, "unity.hotReload")["enabled"], json!(true));
    }

    #[test]
    fn nested_fields_map_to_nested_keys() {
        let requested = changes(
            "unity.codeAnalysis",
            &json!({ "tools": { "codeHover": true } }),
        );
        assert_eq!(requested.len(), 1);
        assert_eq!(
            DEFINITIONS[requested[0].0].key,
            "unity.codeAnalysis.tools.codeHover"
        );
    }
}
