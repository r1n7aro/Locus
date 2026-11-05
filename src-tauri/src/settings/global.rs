//! The process-wide app settings store. `AppConfig` and modules without an
//! app handle (proxy, Codex options, MCP server, ...) share this instance so
//! every reader sees the same values.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use super::legacy::{self, LegacySources};
use super::store::{SettingsStore, SETTINGS_FILE_NAME};

static GLOBAL: OnceLock<Arc<SettingsStore>> = OnceLock::new();

/// Opens `config_dir/settings.json`, importing legacy files from
/// `config_dir` and `data_dir` on first use or after an upgrade.
pub fn open_store(config_dir: &Path, data_dir: &Path) -> SettingsStore {
    let sources = LegacySources {
        config_dir,
        data_dir,
    };
    let path = config_dir.join(SETTINGS_FILE_NAME);
    let store = SettingsStore::open_with_upgrade(
        &path,
        || legacy::seed_document(&sources),
        |from_version, document| legacy::upgrade_document(from_version, document, &sources),
    );
    println!(
        "[Locus] settings loaded from {:?}",
        dunce::canonicalize(&path).unwrap_or(path)
    );
    store
}

/// The shared store at the given locations; the first caller decides them.
pub fn open(config_dir: &Path, data_dir: &Path) -> Arc<SettingsStore> {
    GLOBAL
        .get_or_init(|| Arc::new(open_store(config_dir, data_dir)))
        .clone()
}

/// The shared store. Startup opens it through `AppConfig::load`; an earlier
/// caller opens the persistent config directory itself.
pub fn global() -> &'static Arc<SettingsStore> {
    GLOBAL.get_or_init(|| Arc::new(default_store()))
}

#[cfg(test)]
fn default_store() -> SettingsStore {
    // Unit tests never touch the user's settings.
    SettingsStore::in_memory(serde_json::Map::new())
}

#[cfg(not(test))]
fn default_store() -> SettingsStore {
    match crate::commands::persistent_config_dir() {
        Ok(config_dir) => {
            let data_dir = crate::runtime_paths::active_data_dir()
                .map(Path::to_path_buf)
                .unwrap_or_else(|_| config_dir.clone());
            open_store(&config_dir, &data_dir)
        }
        Err(error) => {
            eprintln!("[Locus] settings unavailable ({error}); using defaults for this run");
            SettingsStore::in_memory(serde_json::Map::new())
        }
    }
}
