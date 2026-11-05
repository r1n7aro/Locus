//! Paired desktops, persisted as `desktops.json` in the app data directory.
//! Device tokens are stored only in sealed form (see `secure`).

use std::path::{Path, PathBuf};

use locus_remote_protocol::CandidateAddress;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopRecord {
    pub id: String,
    pub name: String,
    pub addresses: Vec<CandidateAddress>,
    /// Pinned certificate of the direct addresses; empty when the desktop
    /// was paired through a CA-trusted hostname only.
    #[serde(default)]
    pub fingerprint: String,
    pub device_id: String,
    pub device_name: String,
    pub paired_at_ms: i64,
    #[serde(default)]
    pub last_address: Option<String>,
    pub sealed_token: String,
}

impl DesktopRecord {
    /// Last working address first, then the pairing order.
    pub fn ordered_addresses(&self) -> Vec<CandidateAddress> {
        let mut addresses = self.addresses.clone();
        if let Some(last) = &self.last_address {
            if let Some(index) = addresses.iter().position(|address| &address.url == last) {
                let preferred = addresses.remove(index);
                addresses.insert(0, preferred);
            }
        }
        addresses
    }

    /// Key alias of the sealed token in the platform keystore.
    pub fn key_alias(&self) -> String {
        format!("locus-remote-{}", self.id)
    }
}

/// What the launcher page sees: no token, no fingerprint internals.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopView {
    pub id: String,
    pub name: String,
    pub device_name: String,
    pub paired_at_ms: i64,
    pub last_address: Option<String>,
    pub addresses: Vec<String>,
}

impl From<&DesktopRecord> for DesktopView {
    fn from(record: &DesktopRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            device_name: record.device_name.clone(),
            paired_at_ms: record.paired_at_ms,
            last_address: record.last_address.clone(),
            addresses: record.addresses.iter().map(|address| address.url.clone()).collect(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    desktops: Vec<DesktopRecord>,
}

pub struct DesktopStore {
    path: PathBuf,
    records: Vec<DesktopRecord>,
}

impl DesktopStore {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("desktops.json");
        let records = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StoreFile>(&bytes).ok())
            .map(|file| file.desktops)
            .unwrap_or_default();
        Self { path, records }
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let data = serde_json::to_vec_pretty(&StoreFile { version: 1, desktops: self.records.clone() })
            .map_err(|error| error.to_string())?;
        let temp = self.path.with_extension("json.tmp");
        std::fs::write(&temp, data).map_err(|error| error.to_string())?;
        if cfg!(windows) {
            let _ = std::fs::remove_file(&self.path);
        }
        std::fs::rename(&temp, &self.path).map_err(|error| error.to_string())
    }

    pub fn list(&self) -> Vec<DesktopView> {
        let mut views: Vec<DesktopView> = self.records.iter().map(DesktopView::from).collect();
        views.sort_by(|a, b| b.paired_at_ms.cmp(&a.paired_at_ms));
        views
    }

    pub fn get(&self, id: &str) -> Option<DesktopRecord> {
        self.records.iter().find(|record| record.id == id).cloned()
    }

    /// Re-pairing a desktop replaces its record.
    pub fn upsert(&mut self, record: DesktopRecord) -> Result<(), String> {
        self.records.retain(|existing| existing.id != record.id);
        self.records.push(record);
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> Result<Option<DesktopRecord>, String> {
        let Some(index) = self.records.iter().position(|record| record.id == id) else {
            return Ok(None);
        };
        let removed = self.records.remove(index);
        self.save()?;
        Ok(Some(removed))
    }

    pub fn set_last_address(&mut self, id: &str, url: &str) {
        let Some(record) = self.records.iter_mut().find(|record| record.id == id) else {
            return;
        };
        if record.last_address.as_deref() == Some(url) {
            return;
        }
        record.last_address = Some(url.to_string());
        if let Err(error) = self.save() {
            eprintln!("[LocusRemote] failed to remember address: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> DesktopRecord {
        DesktopRecord {
            id: id.into(),
            name: "PC".into(),
            addresses: vec![
                CandidateAddress { url: "https://10.0.0.2:47831".into(), pinned: true },
                CandidateAddress { url: "https://pc.ts.net".into(), pinned: false },
            ],
            fingerprint: "a".repeat(64),
            device_id: "dev".into(),
            device_name: "Pixel".into(),
            paired_at_ms: 1,
            last_address: None,
            sealed_token: "sealed".into(),
        }
    }

    #[test]
    fn last_working_address_is_tried_first() {
        let mut desktop = record("d");
        assert_eq!(desktop.ordered_addresses()[0].url, "https://10.0.0.2:47831");
        desktop.last_address = Some("https://pc.ts.net".into());
        assert_eq!(desktop.ordered_addresses()[0].url, "https://pc.ts.net");
    }

    #[test]
    fn persists_and_replaces_records() {
        let dir = std::env::temp_dir().join(format!("locus-remote-store-{}", uuid::Uuid::new_v4()));
        let mut store = DesktopStore::load(&dir);
        store.upsert(record("d")).unwrap();
        store.upsert(DesktopRecord { name: "Renamed".into(), ..record("d") }).unwrap();
        store.set_last_address("d", "https://pc.ts.net");
        let reloaded = DesktopStore::load(&dir);
        assert_eq!(reloaded.list().len(), 1);
        assert_eq!(reloaded.get("d").unwrap().name, "Renamed");
        assert_eq!(reloaded.get("d").unwrap().last_address.as_deref(), Some("https://pc.ts.net"));
        let text = std::fs::read_to_string(dir.join("desktops.json")).unwrap();
        assert!(!text.contains("lrt_"), "no clear tokens on disk");
        let _ = std::fs::remove_dir_all(dir);
    }
}
