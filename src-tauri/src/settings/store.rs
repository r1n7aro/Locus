//! App-scope settings storage: one `settings.json` that holds only values
//! that differ from their defaults, plus any keys this version does not know
//! (kept so that alternating Locus versions do not erase each other's data).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, RwLock};

use serde::Serialize;
use serde_json::{Map, Value};

use super::definitions::DEFINITIONS;
use super::schema::{Key, Scope, SettingDef, SettingValue};

pub const SETTINGS_FILE_NAME: &str = "settings.json";
/// 1: `config.json` imported. 2: per-feature files (Codex, model defaults,
/// proxy, MCP server, remote access, Claude connection, runtimes, tool
/// permissions) imported as well.
pub const SETTINGS_VERSION: u64 = 2;
const VERSION_FIELD: &str = "version";

/// Index of a setting by key.
pub fn index_of(key: &str) -> Option<usize> {
    static INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    INDEX
        .get_or_init(|| {
            DEFINITIONS
                .iter()
                .enumerate()
                .map(|(index, def)| (def.key, index))
                .collect()
        })
        .get(key)
        .copied()
}

pub fn definition(key: &str) -> Option<&'static SettingDef> {
    index_of(key).map(|index| &DEFINITIONS[index])
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingChange {
    pub key: &'static str,
    pub old: Value,
    pub new: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsError {
    pub code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub message: String,
}

impl SettingsError {
    pub fn unknown_key(key: &str) -> Self {
        Self {
            code: "settings.unknown_key",
            key: Some(key.to_string()),
            message: format!("Unknown setting '{key}'."),
        }
    }

    pub fn invalid_value(key: &str, reason: &str) -> Self {
        Self {
            code: "settings.invalid_value",
            key: Some(key.to_string()),
            message: format!("Invalid value for '{key}': {reason}."),
        }
    }

    pub fn persistence(message: String) -> Self {
        Self {
            code: "settings.persist_failed",
            key: None,
            message,
        }
    }
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SettingsError {}

impl From<SettingsError> for String {
    fn from(error: SettingsError) -> Self {
        error.message
    }
}

impl From<SettingsError> for crate::error::AppError {
    fn from(error: SettingsError) -> Self {
        crate::error::AppError::new(error.code, error.message)
    }
}

struct StoreState {
    /// File contents without the version field.
    document: Map<String, Value>,
    /// Effective value per definition index (user value, override or default).
    resolved: Vec<Value>,
    /// Memory-only values that win over the document; never persisted.
    overrides: HashMap<usize, Value>,
}

/// Called after every committed write with the changes whose effective
/// value moved. Runs synchronously on the writing thread, outside the state
/// lock; listeners must stay cheap and must not write settings themselves.
pub type ChangeListener = Box<dyn Fn(&SettingsStore, &[SettingChange]) + Send + Sync>;

pub struct SettingsStore {
    path: Option<PathBuf>,
    state: RwLock<StoreState>,
    /// Serializes file replacements so concurrent writers never lose updates.
    persist_lock: Mutex<()>,
    listeners: RwLock<Vec<ChangeListener>>,
}

impl SettingsStore {
    /// Opens `path`. When the file is missing, `seed` provides the initial
    /// document (legacy import or first-run values) and it is written once.
    pub fn open(path: &Path, seed: impl FnOnce() -> Map<String, Value>) -> Self {
        Self::open_with_upgrade(path, seed, |_, _| {})
    }

    /// Like `open`; a file written by an older version is first passed to
    /// `upgrade(from_version, document)` and saved with the current version.
    pub fn open_with_upgrade(
        path: &Path,
        seed: impl FnOnce() -> Map<String, Value>,
        upgrade: impl FnOnce(u64, &mut Map<String, Value>),
    ) -> Self {
        let document = match read_document(path) {
            Ok(Some((mut document, version))) => {
                if version < SETTINGS_VERSION {
                    upgrade(version, &mut document);
                    if let Err(error) = write_document(path, &document) {
                        eprintln!(
                            "[Locus] failed to upgrade settings '{}': {error}",
                            path.display()
                        );
                    }
                }
                document
            }
            Ok(None) => {
                let document = seed();
                if let Err(error) = write_document(path, &document) {
                    eprintln!(
                        "[Locus] failed to write settings '{}': {error}",
                        path.display()
                    );
                }
                document
            }
            Err(error) => {
                // Keep the unreadable file for inspection and start from the
                // seed instead of failing startup.
                let backup = path.with_file_name(format!(
                    "{SETTINGS_FILE_NAME}.invalid-{}",
                    chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
                ));
                eprintln!(
                    "[Locus] settings '{}' is unreadable ({error}); moving it to '{}'",
                    path.display(),
                    backup.display()
                );
                let _ = fs::rename(path, &backup);
                let document = seed();
                if let Err(error) = write_document(path, &document) {
                    eprintln!(
                        "[Locus] failed to write settings '{}': {error}",
                        path.display()
                    );
                }
                document
            }
        };
        Self::from_document(Some(path.to_path_buf()), document)
    }

    /// A store that never touches disk (tests and tools).
    pub fn in_memory(document: Map<String, Value>) -> Self {
        Self::from_document(None, document)
    }

    fn from_document(path: Option<PathBuf>, document: Map<String, Value>) -> Self {
        let resolved = DEFINITIONS
            .iter()
            .map(|def| resolve(def, &document, true))
            .collect();
        Self {
            path,
            state: RwLock::new(StoreState {
                document,
                resolved,
                overrides: HashMap::new(),
            }),
            persist_lock: Mutex::new(()),
            listeners: RwLock::new(Vec::new()),
        }
    }

    pub fn add_listener(&self, listener: ChangeListener) {
        self.listeners
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .push(listener);
    }

    fn notify(&self, changes: &[SettingChange]) {
        let moved = changes
            .iter()
            .filter(|change| change.old != change.new)
            .cloned()
            .collect::<Vec<_>>();
        if moved.is_empty() {
            return;
        }
        let listeners = self.listeners.read().unwrap_or_else(|error| error.into_inner());
        for listener in listeners.iter() {
            listener(self, &moved);
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn get<T: SettingValue>(&self, key: Key<T>) -> T {
        let value = self.value_at(key.index());
        T::from_setting(&value)
            .or_else(|| T::from_setting(&DEFINITIONS[key.index()].default_value()))
            .expect("setting default matches its declared Rust type")
    }

    pub fn value_at(&self, index: usize) -> Value {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        state.resolved[index].clone()
    }

    /// The persisted user value, if the user changed this setting.
    pub fn user_value_at(&self, index: usize) -> Option<Value> {
        let def = &DEFINITIONS[index];
        if def.scope != Scope::App {
            return None;
        }
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        lookup(&state.document, def.key)
            .and_then(|value| def.kind.normalize(value).ok())
            .filter(|value| *value != def.default_value())
    }

    pub fn set<T: SettingValue>(&self, key: Key<T>, value: T) -> Result<SettingChange, SettingsError> {
        self.set_values(&[(key.index(), value.to_setting())])
            .map(|mut changes| changes.remove(0))
    }

    /// Validates the whole batch, persists it atomically and only then
    /// exposes the new values. Returns one change per requested setting.
    pub fn set_values(&self, changes: &[(usize, Value)]) -> Result<Vec<SettingChange>, SettingsError> {
        let mut normalized = Vec::with_capacity(changes.len());
        for (index, value) in changes {
            let def = DEFINITIONS
                .get(*index)
                .ok_or_else(|| SettingsError::unknown_key(&index.to_string()))?;
            if def.scope != Scope::App {
                return Err(SettingsError {
                    code: "settings.project_scope",
                    key: Some(def.key.to_string()),
                    message: format!(
                        "'{}' is a project setting and needs a project scope.",
                        def.key
                    ),
                });
            }
            let value = def
                .normalize(value)
                .map_err(|reason| SettingsError::invalid_value(def.key, &reason))?;
            normalized.push((*index, value));
        }

        let _persist = self
            .persist_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut document = {
            let state = self.state.read().unwrap_or_else(|error| error.into_inner());
            state.document.clone()
        };
        for (index, value) in &normalized {
            let def = &DEFINITIONS[*index];
            if *value == def.default_value() {
                remove_path(&mut document, def.key);
            } else {
                insert_path(&mut document, def.key, value.clone());
            }
        }
        if let Some(path) = &self.path {
            write_document(path, &document).map_err(SettingsError::persistence)?;
        }

        let mut state = self.state.write().unwrap_or_else(|error| error.into_inner());
        let mut applied = Vec::with_capacity(normalized.len());
        for (index, _) in &normalized {
            let old = state.resolved[*index].clone();
            // An explicit write supersedes a process-only override.
            state.overrides.remove(index);
            let new = resolve(&DEFINITIONS[*index], &document, false);
            state.resolved[*index] = new.clone();
            applied.push(SettingChange {
                key: DEFINITIONS[*index].key,
                old,
                new,
            });
        }
        state.document = document;
        drop(state);
        drop(_persist);
        self.notify(&applied);
        Ok(applied)
    }

    /// One-time import from older storage: stores each value only when the
    /// user has not set that setting yet. Invalid values are skipped with a
    /// warning instead of failing the import. Returns the imported changes.
    pub fn import_missing(&self, values: &[(usize, Value)]) -> Result<Vec<SettingChange>, SettingsError> {
        let _persist = self
            .persist_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut document = {
            let state = self.state.read().unwrap_or_else(|error| error.into_inner());
            state.document.clone()
        };
        let mut imported = Vec::new();
        for (index, value) in values {
            let Some(def) = DEFINITIONS.get(*index) else {
                continue;
            };
            if def.scope != Scope::App || lookup(&document, def.key).is_some() {
                continue;
            }
            match def.normalize(value) {
                Ok(value) if value != def.default_value() => {
                    insert_path(&mut document, def.key, value);
                    imported.push(*index);
                }
                Ok(_) => {}
                Err(reason) => eprintln!("[Locus] skipping imported setting '{}': {reason}", def.key),
            }
        }
        if imported.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(path) = &self.path {
            write_document(path, &document).map_err(SettingsError::persistence)?;
        }

        let mut state = self.state.write().unwrap_or_else(|error| error.into_inner());
        let mut applied = Vec::with_capacity(imported.len());
        for index in imported {
            let old = state.resolved[index].clone();
            state.overrides.remove(&index);
            let new = resolve(&DEFINITIONS[index], &document, false);
            state.resolved[index] = new.clone();
            applied.push(SettingChange {
                key: DEFINITIONS[index].key,
                old,
                new,
            });
        }
        state.document = document;
        drop(state);
        drop(_persist);
        self.notify(&applied);
        Ok(applied)
    }

    /// Returns every app setting to its default and drops unknown keys
    /// (Settings → reset all). Listeners see each moved value.
    pub fn reset_to_defaults(&self) -> Result<Vec<SettingChange>, SettingsError> {
        let _persist = self
            .persist_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let document = Map::new();
        if let Some(path) = &self.path {
            write_document(path, &document).map_err(SettingsError::persistence)?;
        }
        let mut state = self.state.write().unwrap_or_else(|error| error.into_inner());
        let mut applied = Vec::new();
        for (index, def) in DEFINITIONS.iter().enumerate() {
            if def.scope != Scope::App {
                continue;
            }
            let old = state.resolved[index].clone();
            state.overrides.remove(&index);
            let new = def.default_value();
            state.resolved[index] = new.clone();
            applied.push(SettingChange {
                key: def.key,
                old,
                new,
            });
        }
        state.document = document;
        drop(state);
        drop(_persist);
        self.notify(&applied);
        Ok(applied)
    }

    /// Forces an effective value for this process without persisting it,
    /// until the setting is written explicitly.
    pub fn set_runtime_override<T: SettingValue>(&self, key: Key<T>, value: T) {
        let value = value.to_setting();
        let mut state = self.state.write().unwrap_or_else(|error| error.into_inner());
        state.resolved[key.index()] = value.clone();
        state.overrides.insert(key.index(), value);
    }
}

/// The document without its version field, and that version (0 if absent).
fn read_document(path: &Path) -> Result<Option<(Map<String, Value>, u64)>, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let content = content.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Value>(content).map_err(|error| error.to_string())? {
        Value::Object(mut document) => {
            let version = document
                .remove(VERSION_FIELD)
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if version > SETTINGS_VERSION {
                eprintln!(
                    "[Locus] settings '{}' has newer version {version}; unknown keys are preserved",
                    path.display()
                );
            }
            Ok(Some((document, version)))
        }
        _ => Err("settings file is not a JSON object".to_string()),
    }
}

fn write_document(path: &Path, document: &Map<String, Value>) -> Result<(), String> {
    let mut file = Map::with_capacity(document.len() + 1);
    // Never downgrade a file written by a newer Locus.
    let version = read_document(path)
        .ok()
        .flatten()
        .map(|(_, version)| version)
        .unwrap_or(0)
        .max(SETTINGS_VERSION);
    file.insert(VERSION_FIELD.to_string(), Value::from(version));
    for (key, value) in document {
        file.insert(key.clone(), value.clone());
    }
    let json = serde_json::to_string_pretty(&Value::Object(file))
        .map_err(|error| format!("failed to serialize settings: {error}"))?;
    crate::config::atomic_write_config(path, json.as_bytes())
}

fn resolve(def: &SettingDef, document: &Map<String, Value>, warn: bool) -> Value {
    if def.scope != Scope::App {
        return def.default_value();
    }
    // Extra validators (existing directories, package ids) only run on
    // writes: a value that was valid when saved must not vanish because, for
    // example, its drive is offline at startup.
    match lookup(document, def.key) {
        Some(value) => def.kind.normalize(value).unwrap_or_else(|reason| {
            if warn {
                eprintln!(
                    "[Locus] ignoring invalid setting '{}' ({reason}); using the default",
                    def.key
                );
            }
            def.default_value()
        }),
        None => def.default_value(),
    }
}

pub(crate) fn lookup<'a>(document: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    let mut segments = key.split('.');
    let mut current = document.get(segments.next()?)?;
    for segment in segments {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

pub(crate) fn insert_path(document: &mut Map<String, Value>, key: &str, value: Value) {
    let segments = key.split('.').collect::<Vec<_>>();
    let (last, parents) = segments.split_last().expect("setting keys are non-empty");
    let mut current = document;
    for segment in parents {
        let entry = current
            .entry(segment.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("just ensured an object");
    }
    current.insert(last.to_string(), value);
}

/// Removes `key` and prunes parents left empty by the removal.
pub(crate) fn remove_path(document: &mut Map<String, Value>, key: &str) {
    fn remove(map: &mut Map<String, Value>, segments: &[&str]) {
        match segments {
            [] => {}
            [last] => {
                map.remove(*last);
            }
            [first, rest @ ..] => {
                let now_empty = match map.get_mut(*first).and_then(Value::as_object_mut) {
                    Some(child) => {
                        remove(child, rest);
                        child.is_empty()
                    }
                    None => false,
                };
                if now_empty {
                    map.remove(*first);
                }
            }
        }
    }
    let segments = key.split('.').collect::<Vec<_>>();
    remove(document, &segments);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::definitions::*;
    use serde_json::json;

    fn document(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn missing_values_resolve_to_defaults() {
        let store = SettingsStore::in_memory(Map::new());
        assert!(!store.get(SYSTEM_DEBUG));
        assert_eq!(store.get(AGENT_SUBAGENT_MAX_CONCURRENT), 3);
        assert_eq!(store.get(AGENT_TOOL_LOADING_MODE), "native");
    }

    #[test]
    fn invalid_stored_values_fall_back_to_defaults() {
        let store = SettingsStore::in_memory(document(json!({
            "agent": { "subagent": { "maxDepth": 99 } },
            "system": { "debug": "yes" }
        })));
        assert_eq!(store.get(AGENT_SUBAGENT_MAX_DEPTH), 1);
        assert!(!store.get(SYSTEM_DEBUG));
    }

    #[test]
    fn writes_only_values_that_differ_from_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, Map::new);

        store.set(AGENT_SUBAGENT_MAX_DEPTH, 3).unwrap();
        store.set(SYSTEM_DEBUG, false).unwrap();
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({ "version": SETTINGS_VERSION, "agent": { "subagent": { "maxDepth": 3 } } })
        );

        // Returning to the default removes the value and its empty parents.
        store.set(AGENT_SUBAGENT_MAX_DEPTH, 1).unwrap();
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({ "version": SETTINGS_VERSION }));
    }

    #[test]
    fn unknown_keys_survive_writes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        fs::write(
            &path,
            r#"{ "version": 99, "future": { "flag": true }, "agent": { "newer": 1 } }"#,
        )
        .unwrap();
        let store = SettingsStore::open(&path, Map::new);
        store.set(AGENT_SESSION_UNDO, false).unwrap();

        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["version"], json!(99));
        assert_eq!(written["future"], json!({ "flag": true }));
        assert_eq!(written["agent"]["newer"], json!(1));
        assert_eq!(written["agent"]["sessionUndo"], json!(false));
    }

    #[test]
    fn batch_is_rejected_as_a_whole_when_one_value_is_invalid() {
        let store = SettingsStore::in_memory(Map::new());
        let error = store
            .set_values(&[
                (SYSTEM_DEBUG.index(), json!(true)),
                (AGENT_SUBAGENT_MAX_DEPTH.index(), json!(0)),
            ])
            .unwrap_err();
        assert_eq!(error.code, "settings.invalid_value");
        assert_eq!(error.key.as_deref(), Some("agent.subagent.maxDepth"));
        assert!(!store.get(SYSTEM_DEBUG));
    }

    #[test]
    fn failed_persistence_leaves_values_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, Map::new);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let error = store.set(SYSTEM_DEBUG, true).unwrap_err();
        assert_eq!(error.code, "settings.persist_failed");
        assert!(!store.get(SYSTEM_DEBUG));
    }

    #[test]
    fn changes_report_old_and_new_effective_values() {
        let store = SettingsStore::in_memory(Map::new());
        let change = store.set(AGENT_LLM_RETRY_MAX_ATTEMPTS, 5).unwrap();
        assert_eq!(
            change,
            SettingChange {
                key: "agent.llm.retryMaxAttempts",
                old: json!(3),
                new: json!(5)
            }
        );
    }

    #[test]
    fn runtime_overrides_win_and_are_not_persisted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, Map::new);
        store.set_runtime_override(UNITY_EMBED_ENABLED, false);
        assert!(!store.get(UNITY_EMBED_ENABLED));

        store.set(SYSTEM_DEBUG, true).unwrap();
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(written.get("unity").is_none());
        assert!(!store.get(UNITY_EMBED_ENABLED));

        // Writing the overridden setting itself replaces the override.
        let change = store.set(UNITY_EMBED_ENABLED, true).unwrap();
        assert_eq!(change.old, json!(false));
        assert_eq!(change.new, json!(true));
        assert!(store.get(UNITY_EMBED_ENABLED));
    }

    #[test]
    fn listeners_see_only_moved_values_after_commit() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let store = SettingsStore::in_memory(Map::new());
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        {
            let seen = seen.clone();
            let calls = calls.clone();
            store.add_listener(Box::new(move |store, changes| {
                calls.fetch_add(1, Ordering::SeqCst);
                // The committed value is already visible to listeners.
                assert!(store.get(SYSTEM_DEBUG));
                seen.lock().unwrap().extend(changes.iter().map(|change| change.key));
            }));
        }
        store
            .set_values(&[
                (SYSTEM_DEBUG.index(), json!(true)),
                (AGENT_SESSION_UNDO.index(), json!(true)),
            ])
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["system.debug"]);
        store.set(SYSTEM_DEBUG, true).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn project_settings_are_rejected_by_the_app_store() {
        let store = SettingsStore::in_memory(Map::new());
        let error = store.set(PROJECT_KNOWLEDGE_ENABLED, false).unwrap_err();
        assert_eq!(error.code, "settings.project_scope");
    }

    #[test]
    fn unreadable_file_is_set_aside_and_seed_is_used() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        fs::write(&path, "{ not json").unwrap();
        let store = SettingsStore::open(&path, || document(json!({ "system": { "debug": true } })));
        assert!(store.get(SYSTEM_DEBUG));
        let backups = fs::read_dir(temp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".invalid-"))
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn older_files_are_upgraded_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        fs::write(&path, r#"{ "version": 1, "system": { "debug": true } }"#).unwrap();
        let store = SettingsStore::open_with_upgrade(&path, Map::new, |from, document| {
            assert_eq!(from, 1);
            insert_path(document, "agent.sessionUndo", json!(false));
        });
        assert!(store.get(SYSTEM_DEBUG));
        assert!(!store.get(AGENT_SESSION_UNDO));
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["version"], json!(SETTINGS_VERSION));
        drop(store);
        SettingsStore::open_with_upgrade(&path, Map::new, |_, _| {
            panic!("an upgraded file is not upgraded again")
        });
    }

    #[test]
    fn reset_restores_defaults_and_notifies() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, || {
            document(json!({ "system": { "debug": true }, "future": 1 }))
        });
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        {
            let seen = seen.clone();
            store.add_listener(Box::new(move |_, changes| {
                seen.lock().unwrap().extend(changes.iter().map(|change| change.key));
            }));
        }
        store.reset_to_defaults().unwrap();
        assert!(!store.get(SYSTEM_DEBUG));
        assert_eq!(*seen.lock().unwrap(), vec!["system.debug"]);
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({ "version": SETTINGS_VERSION }));
    }

    #[test]
    fn import_fills_only_unset_settings_and_skips_invalid_values() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, || {
            document(json!({ "appearance": { "theme": { "main": "light" } } }))
        });
        let changes = store
            .import_missing(&[
                (APPEARANCE_THEME_MAIN.index(), json!("system")),
                (APPEARANCE_LANGUAGE.index(), json!("zh")),
                (CHAT_HISTORY_PAGE_SIZE.index(), json!(99)),
                (CHAT_MESSAGES_COMPACT_TOOL_CALLS.index(), json!(true)),
            ])
            .unwrap();
        assert_eq!(
            changes,
            vec![SettingChange {
                key: "appearance.language",
                old: json!("system"),
                new: json!("zh")
            }]
        );
        assert_eq!(store.get(APPEARANCE_THEME_MAIN), "light");
        assert_eq!(store.get(CHAT_HISTORY_PAGE_SIZE), 120);
        let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written["appearance"],
            json!({ "theme": { "main": "light" }, "language": "zh" })
        );
        // Values equal to the default are not written and stay importable.
        assert!(written.get("chat").is_none());
        assert!(store
            .import_missing(&[(APPEARANCE_LANGUAGE.index(), json!("en"))])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn seed_is_used_only_when_the_file_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(SETTINGS_FILE_NAME);
        let store = SettingsStore::open(&path, || document(json!({ "system": { "debug": true } })));
        assert!(store.get(SYSTEM_DEBUG));
        drop(store);
        let reopened = SettingsStore::open(&path, || panic!("seed must not run when the file exists"));
        assert!(reopened.get(SYSTEM_DEBUG));
    }
}
