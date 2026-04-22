use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct BinaryCacheEntry {
    bytes: Vec<u8>,
    mime: String,
    last_access: Instant,
}

pub struct BinaryCache {
    entries: Mutex<HashMap<String, BinaryCacheEntry>>,
}

impl BinaryCache {
    const TTL: Duration = Duration::from_secs(600); // 10 minutes
    const MAX_ENTRIES: usize = 20;

    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Insert binary data and return a UUID blob ID.
    pub fn insert(&self, bytes: Vec<u8>, mime: String) -> String {
        let op_start = Instant::now();
        let id = Uuid::new_v4().to_string();
        let byte_len = bytes.len();
        let trace = mime == "application/octet-stream";
        let mut map = self.entries.lock().unwrap();

        // Evict expired entries
        let now = Instant::now();
        map.retain(|_, e| now.duration_since(e.last_access) < Self::TTL);

        // If still at capacity, remove oldest
        while map.len() >= Self::MAX_ENTRIES {
            if let Some(oldest_key) = map
                .iter()
                .min_by_key(|(_, e)| e.last_access)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest_key);
            } else {
                break;
            }
        }

        map.insert(
            id.clone(),
            BinaryCacheEntry {
                bytes,
                mime,
                last_access: now,
            },
        );

        if trace {
            eprintln!(
                "[perf:binary-cache:insert] id={} bytes={} total={}ms entries={}",
                id,
                byte_len,
                op_start.elapsed().as_millis(),
                map.len()
            );
        }

        id
    }

    /// Get binary data by blob ID. Refreshes TTL on access (lease renewal).
    pub fn get(&self, id: &str) -> Option<(Vec<u8>, String)> {
        let op_start = Instant::now();
        let mut map = self.entries.lock().unwrap();
        let entry = map.get_mut(id)?;
        entry.last_access = Instant::now(); // lease renewal
        let clone_start = Instant::now();
        let bytes = entry.bytes.clone();
        let mime = entry.mime.clone();

        if mime == "application/octet-stream" {
            eprintln!(
                "[perf:binary-cache:get] id={} bytes={} clone={}ms total={}ms",
                id,
                bytes.len(),
                clone_start.elapsed().as_millis(),
                op_start.elapsed().as_millis()
            );
        }

        Some((bytes, mime))
    }
}
