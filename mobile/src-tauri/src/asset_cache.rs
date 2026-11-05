//! Disk cache of the desktop page's content-hashed assets (`/assets/*`,
//! served `immutable`). Requests answered through the `locus-remote` scheme
//! bypass the WebView's HTTP cache, so without this every launch would
//! download the whole bundle again over the tunnel.
//!
//! One directory per desktop; an entry is `<content-type>\n<body>`, written
//! to a temporary file and renamed into place. Entries are evicted by age and
//! total size (least recently used first).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_UNUSED: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const MAX_ENTRY_BYTES: usize = 16 * 1024 * 1024;

pub struct CachedAsset {
    pub content_type: String,
    pub body: Vec<u8>,
}

#[derive(Clone)]
pub struct AssetCache {
    root: PathBuf,
}

fn safe_segment(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && !value.starts_with('.')
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Whether a response to `path_and_query` may be stored at all.
pub fn cacheable_request(path_and_query: &str) -> bool {
    path_and_query
        .strip_prefix("/assets/")
        .is_some_and(|name| safe_segment(name, 200))
}

/// `immutable` is what the desktop sends for content-hashed files.
pub fn cacheable_response(status: u16, cache_control: Option<&str>) -> bool {
    status == 200
        && cache_control.is_some_and(|value| {
            value.split(',').any(|directive| directive.trim().eq_ignore_ascii_case("immutable"))
        })
}

impl AssetCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn desktop_dir(&self, desktop_id: &str) -> Option<PathBuf> {
        safe_segment(desktop_id, 64).then(|| self.root.join(desktop_id))
    }

    fn entry_path(&self, desktop_id: &str, path_and_query: &str) -> Option<PathBuf> {
        let name = path_and_query.strip_prefix("/assets/").filter(|name| safe_segment(name, 200))?;
        Some(self.desktop_dir(desktop_id)?.join(name))
    }

    pub async fn get(&self, desktop_id: &str, path_and_query: &str) -> Option<CachedAsset> {
        let path = self.entry_path(desktop_id, path_and_query)?;
        tokio::task::spawn_blocking(move || read_entry(&path)).await.ok().flatten()
    }

    /// Stores in the background; a failed write only costs a later download.
    pub fn put(&self, desktop_id: &str, path_and_query: &str, content_type: &str, body: Vec<u8>) {
        let Some(path) = self.entry_path(desktop_id, path_and_query) else {
            return;
        };
        if content_type.contains('\n') || body.len() > MAX_ENTRY_BYTES {
            return;
        }
        let content_type = content_type.to_string();
        tokio::task::spawn_blocking(move || {
            if let Err(error) = write_entry(&path, &content_type, &body) {
                eprintln!("[LocusRemote] asset cache write failed: {error}");
            }
        });
    }

    pub fn forget_desktop(&self, desktop_id: &str) {
        if let Some(dir) = self.desktop_dir(desktop_id) {
            tokio::task::spawn_blocking(move || {
                let _ = fs::remove_dir_all(dir);
            });
        }
    }

    /// Drops entries unused for a month, then the least recently used ones
    /// beyond the size budget.
    pub fn prune(&self) {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || prune(&root, MAX_TOTAL_BYTES, MAX_UNUSED, SystemTime::now()));
    }
}

fn read_entry(path: &Path) -> Option<CachedAsset> {
    let bytes = fs::read(path).ok()?;
    let split = bytes.iter().position(|byte| *byte == b'\n')?;
    let content_type = std::str::from_utf8(&bytes[..split]).ok()?.to_string();
    // Recency for eviction; best effort.
    if let Ok(file) = fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
    let mut body = bytes;
    body.drain(..=split);
    Some(CachedAsset { content_type, body })
}

fn write_entry(path: &Path, content_type: &str, body: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
    fs::create_dir_all(dir)?;
    let temp = dir.join(format!(".tmp-{}-{}", std::process::id(), rand_suffix()));
    let result = (|| {
        let mut file = fs::File::create(&temp)?;
        file.write_all(content_type.as_bytes())?;
        file.write_all(b"\n")?;
        file.write_all(body)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn rand_suffix() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos());
    hasher.finish()
}

fn prune(root: &Path, max_total: u64, max_unused: Duration, now: SystemTime) {
    let Ok(desktops) = fs::read_dir(root) else {
        return;
    };
    let mut entries = Vec::new();
    for desktop in desktops.flatten() {
        let Ok(files) = fs::read_dir(desktop.path()) else {
            continue;
        };
        for file in files.flatten() {
            let Ok(meta) = file.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let unused = now.duration_since(modified).unwrap_or_default();
            // Leftover temporaries are only kept while a write may still be running.
            let stale_temp = file.file_name().to_string_lossy().starts_with(".tmp-")
                && unused > Duration::from_secs(60 * 60);
            if unused > max_unused || stale_temp {
                let _ = fs::remove_file(file.path());
            } else {
                entries.push((modified, meta.len(), file.path()));
            }
        }
    }
    let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
    if total <= max_total {
        return;
    }
    entries.sort_by_key(|(modified, _, _)| *modified);
    for (_, len, path) in entries {
        if total <= max_total {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("locus-remote-cache-{name}-{}", rand_suffix()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn request_and_response_rules() {
        assert!(cacheable_request("/assets/main-BTWibRZL.js"));
        assert!(!cacheable_request("/assets/main.js?v=1"));
        assert!(!cacheable_request("/assets/../secret"));
        assert!(!cacheable_request("/assets/sub/dir.js"));
        assert!(!cacheable_request("/assets/.hidden"));
        assert!(!cacheable_request("/src/main.ts"));
        assert!(!cacheable_request("/m/"));
        assert!(cacheable_response(200, Some("private, max-age=31536000, immutable")));
        assert!(!cacheable_response(200, Some("no-store")));
        assert!(!cacheable_response(200, None));
        assert!(!cacheable_response(404, Some("immutable")));
    }

    #[test]
    fn entries_round_trip_and_reject_unsafe_ids() {
        let root = temp_root("roundtrip");
        let cache = AssetCache::new(root.clone());
        let path = cache.entry_path("abc123", "/assets/app-1.js").unwrap();
        write_entry(&path, "text/javascript", b"line1\nline2").unwrap();
        let entry = read_entry(&path).unwrap();
        assert_eq!(entry.content_type, "text/javascript");
        assert_eq!(entry.body, b"line1\nline2");
        assert!(cache.entry_path("../x", "/assets/app.js").is_none());
        assert!(cache.entry_path("abc", "/assets/../app.js").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_evicts_old_then_least_recent() {
        let root = temp_root("prune");
        let dir = root.join("desk");
        fs::create_dir_all(&dir).unwrap();
        let now = SystemTime::now();
        let write = |name: &str, len: usize, age_secs: u64| {
            let path = dir.join(name);
            fs::write(&path, vec![0u8; len]).unwrap();
            let file = fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(now - Duration::from_secs(age_secs)).unwrap();
        };
        write("ancient.js", 10, 90 * 24 * 3600);
        write("old.js", 100, 300);
        write("new.js", 100, 10);
        prune(&root, 150, Duration::from_secs(30 * 24 * 3600), now);
        assert!(!dir.join("ancient.js").exists(), "unused for too long");
        assert!(!dir.join("old.js").exists(), "least recently used beyond the budget");
        assert!(dir.join("new.js").exists());
        let _ = fs::remove_dir_all(root);
    }
}
