//! Paired devices in `{persistent_config_dir}/remote_devices.json`.
//!
//! Only SHA-256 token hashes are stored, so the file grants nothing when it
//! leaks. Lookups go by hash; the token itself never reaches disk or logs.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const DEVICES_FILE: &str = "remote_devices.json";
const FILE_VERSION: u32 = 1;
const MAX_DEVICES: usize = 32;
/// `last_seen` updates are kept in memory and flushed at most this often.
const TOUCH_PERSIST_INTERVAL_MS: i64 = 5 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub paired_at_ms: i64,
    pub last_seen_at_ms: i64,
    #[serde(default)]
    pub last_address: Option<String>,
    pub token_hash: String,
    /// Approve tool calls and use commands that change things (files, git,
    /// settings). Without it a device can view and chat. Stored as
    /// `canApproveTools` by earlier builds.
    #[serde(default, alias = "canApproveTools")]
    pub full_control: bool,
}

/// What the settings UI sees: everything but the token hash.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub paired_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub last_address: Option<String>,
    pub full_control: bool,
}

impl From<&DeviceRecord> for DeviceView {
    fn from(record: &DeviceRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            platform: record.platform.clone(),
            paired_at_ms: record.paired_at_ms,
            last_seen_at_ms: record.last_seen_at_ms,
            last_address: record.last_address.clone(),
            full_control: record.full_control,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct DeviceFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    devices: Vec<DeviceRecord>,
}

struct Inner {
    devices: Vec<DeviceRecord>,
    last_persist_ms: i64,
}

pub struct DeviceStore {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl DeviceStore {
    pub fn load_default() -> Self {
        let path = crate::commands::persistent_config_dir()
            .ok()
            .map(|dir| dir.join(DEVICES_FILE));
        Self::load(path)
    }

    pub fn load(path: Option<PathBuf>) -> Self {
        let devices = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<DeviceFile>(&bytes).ok())
            .map(|file| file.devices)
            .unwrap_or_default();
        Self {
            path,
            inner: Mutex::new(Inner { devices, last_persist_ms: 0 }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn persist(&self, inner: &mut Inner, now_ms: i64) -> Result<(), String> {
        inner.last_persist_ms = now_ms;
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        write_file(path, &inner.devices)
    }

    pub fn list(&self) -> Vec<DeviceView> {
        let inner = self.lock();
        let mut devices: Vec<DeviceView> = inner.devices.iter().map(DeviceView::from).collect();
        devices.sort_by(|a, b| b.paired_at_ms.cmp(&a.paired_at_ms));
        devices
    }

    pub fn get(&self, id: &str) -> Option<DeviceRecord> {
        self.lock().devices.iter().find(|device| device.id == id).cloned()
    }

    /// Resolves a bearer token to its device. Tokens idle for more than
    /// `idle_days` are removed and rejected.
    pub fn authenticate(&self, token: &str, idle_days: u32, now_ms: i64) -> Option<DeviceRecord> {
        if !locus_remote_protocol::is_token(token) {
            return None;
        }
        let hash = locus_remote_protocol::token_hash(token);
        let mut inner = self.lock();
        let index = inner.devices.iter().position(|device| device.token_hash == hash)?;
        let idle_limit = i64::from(idle_days.max(1)) * DAY_MS;
        if now_ms.saturating_sub(inner.devices[index].last_seen_at_ms) > idle_limit {
            inner.devices.remove(index);
            if let Err(error) = self.persist(&mut inner, now_ms) {
                eprintln!("[RemoteAccess] failed to drop idle device: {error}");
            }
            return None;
        }
        Some(inner.devices[index].clone())
    }

    /// Records activity; persisted lazily to avoid a disk write per request.
    pub fn touch(&self, id: &str, address: Option<String>, now_ms: i64) {
        let mut inner = self.lock();
        let Some(device) = inner.devices.iter_mut().find(|device| device.id == id) else {
            return;
        };
        device.last_seen_at_ms = now_ms;
        if address.is_some() {
            device.last_address = address;
        }
        if now_ms.saturating_sub(inner.last_persist_ms) >= TOUCH_PERSIST_INTERVAL_MS {
            if let Err(error) = self.persist(&mut inner, now_ms) {
                eprintln!("[RemoteAccess] failed to persist device activity: {error}");
            }
        }
    }

    pub fn add(&self, record: DeviceRecord, now_ms: i64) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.devices.len() >= MAX_DEVICES {
            return Err("已配对设备过多，请先撤销不用的设备".into());
        }
        inner.devices.push(record);
        self.persist(&mut inner, now_ms)
    }

    pub fn revoke(&self, id: &str, now_ms: i64) -> Result<bool, String> {
        let mut inner = self.lock();
        let before = inner.devices.len();
        inner.devices.retain(|device| device.id != id);
        if inner.devices.len() == before {
            return Ok(false);
        }
        self.persist(&mut inner, now_ms)?;
        Ok(true)
    }

    pub fn revoke_all(&self, now_ms: i64) -> Result<Vec<String>, String> {
        let mut inner = self.lock();
        let ids = inner.devices.iter().map(|device| device.id.clone()).collect();
        inner.devices.clear();
        self.persist(&mut inner, now_ms)?;
        Ok(ids)
    }

    pub fn set_full_control(&self, id: &str, allowed: bool, now_ms: i64) -> Result<bool, String> {
        let mut inner = self.lock();
        let Some(device) = inner.devices.iter_mut().find(|device| device.id == id) else {
            return Ok(false);
        };
        device.full_control = allowed;
        self.persist(&mut inner, now_ms)?;
        Ok(true)
    }

    pub fn rename(&self, id: &str, name: &str, now_ms: i64) -> Result<bool, String> {
        let name = sanitize_device_name(name);
        let mut inner = self.lock();
        let Some(device) = inner.devices.iter_mut().find(|device| device.id == id) else {
            return Ok(false);
        };
        device.name = name;
        self.persist(&mut inner, now_ms)?;
        Ok(true)
    }

    /// Flushes pending `touch` updates (shutdown, disable).
    pub fn flush(&self, now_ms: i64) {
        let mut inner = self.lock();
        if let Err(error) = self.persist(&mut inner, now_ms) {
            eprintln!("[RemoteAccess] failed to flush devices: {error}");
        }
    }
}

fn write_file(path: &Path, devices: &[DeviceRecord]) -> Result<(), String> {
    let file = DeviceFile { version: FILE_VERSION, devices: devices.to_vec() };
    let data = serde_json::to_vec_pretty(&file).map_err(|error| error.to_string())?;
    crate::config::atomic_write_config(path, &data)
}

/// Trims, strips control characters and caps the length of names supplied
/// by a phone.
pub fn sanitize_device_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_string();
    if name.is_empty() {
        "手机".to_string()
    } else {
        name
    }
}

pub fn sanitize_platform(raw: &str) -> String {
    let platform: String = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .take(16)
        .collect();
    if platform.is_empty() {
        "unknown".into()
    } else {
        platform.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use locus_remote_protocol::{encode_token, token_hash, TOKEN_BYTES};

    fn record(id: &str, token: &str, now: i64) -> DeviceRecord {
        DeviceRecord {
            id: id.into(),
            name: "Pixel".into(),
            platform: "android".into(),
            paired_at_ms: now,
            last_seen_at_ms: now,
            last_address: None,
            token_hash: token_hash(token),
            full_control: false,
        }
    }

    #[test]
    fn authenticates_by_hash_and_expires_idle_tokens() {
        let store = DeviceStore::load(None);
        let token = encode_token(&[3u8; TOKEN_BYTES]);
        store.add(record("d1", &token, 1_000), 1_000).unwrap();
        assert_eq!(store.authenticate(&token, 30, 2_000).unwrap().id, "d1");
        assert!(store.authenticate(&encode_token(&[4u8; TOKEN_BYTES]), 30, 2_000).is_none());
        assert!(store.authenticate("not-a-token", 30, 2_000).is_none());
        let much_later = 1_000 + 31 * DAY_MS;
        assert!(store.authenticate(&token, 30, much_later).is_none());
        assert!(store.list().is_empty(), "idle device is removed");
    }

    #[test]
    fn touch_keeps_tokens_alive_and_revoke_removes() {
        let store = DeviceStore::load(None);
        let token = encode_token(&[5u8; TOKEN_BYTES]);
        store.add(record("d1", &token, 0), 0).unwrap();
        store.touch("d1", Some("192.168.1.9".into()), 29 * DAY_MS);
        assert!(store.authenticate(&token, 30, 58 * DAY_MS).is_some());
        assert_eq!(store.list()[0].last_address.as_deref(), Some("192.168.1.9"));
        assert!(store.set_full_control("d1", true, 0).unwrap());
        assert!(store.get("d1").unwrap().full_control);
        assert!(store.revoke("d1", 0).unwrap());
        assert!(!store.revoke("d1", 0).unwrap());
        assert!(store.authenticate(&token, 30, 58 * DAY_MS).is_none());
    }

    #[test]
    fn persisted_file_contains_no_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DEVICES_FILE);
        let store = DeviceStore::load(Some(path.clone()));
        let token = encode_token(&[9u8; TOKEN_BYTES]);
        store.add(record("d1", &token, 0), 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains(&token));
        assert!(text.contains(&token_hash(&token)));
        let reloaded = DeviceStore::load(Some(path));
        assert_eq!(reloaded.authenticate(&token, 30, 1).unwrap().id, "d1");
    }

    #[test]
    fn records_from_earlier_builds_keep_their_permission() {
        let legacy = r#"{"id":"d","name":"n","platform":"android","pairedAtMs":1,"lastSeenAtMs":2,
            "tokenHash":"h","canApproveTools":true}"#;
        let record: DeviceRecord = serde_json::from_str(legacy).unwrap();
        assert!(record.full_control);
        let saved = serde_json::to_string(&record).unwrap();
        assert!(saved.contains("\"fullControl\":true") && !saved.contains("canApproveTools"));
    }

    #[test]
    fn names_and_platforms_are_sanitized() {
        assert_eq!(sanitize_device_name("  Pixel\u{0007} 8  "), "Pixel 8");
        assert_eq!(sanitize_device_name("\n"), "手机");
        assert_eq!(sanitize_platform("Android 14!"), "android14");
        assert_eq!(sanitize_platform(""), "unknown");
    }
}
