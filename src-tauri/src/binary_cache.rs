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
    ttl: Duration,
    max_entries: usize,
    max_bytes: usize,
}

impl BinaryCache {
    const TTL: Duration = Duration::from_secs(600); // 10 minutes
    const MAX_ENTRIES: usize = 20;
    /// Resident bytes across all blobs. The entry cap alone would allow 20
    /// previews of the largest supported media (50 MB PSD) to stay in memory.
    const MAX_BYTES: usize = 256 * 1024 * 1024;

    pub fn new() -> Self {
        Self::with_limits(Self::TTL, Self::MAX_ENTRIES, Self::MAX_BYTES)
    }

    pub fn with_limits(ttl: Duration, max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
            max_entries,
            max_bytes,
        }
    }

    /// Insert binary data and return a UUID blob ID.
    ///
    /// Expired blobs are dropped first, then the least recently used ones until
    /// both the entry and the byte budget have room. A single blob larger than
    /// the byte budget is still stored, alone.
    pub fn insert(&self, bytes: Vec<u8>, mime: String) -> String {
        let op_start = Instant::now();
        let id = Uuid::new_v4().to_string();
        let byte_len = bytes.len();
        let trace = mime == "application/octet-stream";
        let mut map = self.entries.lock().unwrap();

        // Evict expired entries
        let now = Instant::now();
        map.retain(|_, e| now.duration_since(e.last_access) < self.ttl);

        // Make room: oldest first, by count and by resident bytes
        let mut resident: usize = map.values().map(|e| e.bytes.len()).sum();
        while !map.is_empty() && (map.len() >= self.max_entries || resident + byte_len > self.max_bytes) {
            let Some(oldest_key) = map
                .iter()
                .min_by_key(|(_, e)| e.last_access)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(removed) = map.remove(&oldest_key) {
                resident -= removed.bytes.len();
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

    /// Reads a blob, or only the bytes of `range`, and renews its lease. Only
    /// the requested slice is copied, so seeking inside a large video does not
    /// clone the whole file per request.
    pub fn read(&self, id: &str, range: Option<ByteRange>) -> Option<BlobRead> {
        let mut map = self.entries.lock().unwrap();
        let entry = map.get_mut(id)?;
        entry.last_access = Instant::now();
        let mime = entry.mime.clone();
        let total = entry.bytes.len();
        Some(match range {
            None => BlobRead::Whole { bytes: entry.bytes.clone(), mime },
            Some(range) => match range.resolve(total) {
                Some((first, last)) => BlobRead::Part {
                    bytes: entry.bytes[first..=last].to_vec(),
                    mime,
                    first,
                    total,
                },
                None => BlobRead::Unsatisfiable { total },
            },
        })
    }

    /// Drop a blob its owner no longer needs. Returns whether it was present.
    pub fn release(&self, id: &str) -> bool {
        self.entries.lock().unwrap().remove(id).is_some()
    }

    /// Number of blobs currently held.
    pub fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// Bytes currently held across all blobs.
    pub fn resident_bytes(&self) -> usize {
        self.entries.lock().unwrap().values().map(|e| e.bytes.len()).sum()
    }
}

/// A `Range: bytes=...` request before it is resolved against a blob's length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    start: Option<usize>,
    end: Option<usize>,
}

impl ByteRange {
    /// Understands one `bytes=` range. Several ranges, other units, junk and
    /// inverted bounds are ignored (RFC 9110 allows that), so the whole blob is
    /// served instead.
    pub fn parse(header: &str) -> Option<Self> {
        let spec = header.trim().strip_prefix("bytes=")?;
        if spec.contains(',') {
            return None;
        }
        let (start, end) = spec.split_once('-')?;
        let bound = |text: &str| -> Option<Option<usize>> {
            let text = text.trim();
            if text.is_empty() {
                Some(None)
            } else {
                text.parse().ok().map(Some)
            }
        };
        let (start, end) = (bound(start)?, bound(end)?);
        match (start, end) {
            (None, None) => None,
            (Some(start), Some(end)) if end < start => None,
            _ => Some(Self { start, end }),
        }
    }

    /// Inclusive `(first, last)` offsets inside `total` bytes; `None` when the
    /// range lies outside them.
    fn resolve(self, total: usize) -> Option<(usize, usize)> {
        if total == 0 {
            return None;
        }
        match (self.start, self.end) {
            (Some(start), end) => {
                (start < total).then(|| (start, end.map_or(total - 1, |end| end.min(total - 1))))
            }
            (None, Some(suffix)) => (suffix > 0).then(|| (total.saturating_sub(suffix), total - 1)),
            (None, None) => None,
        }
    }
}

/// Outcome of [`BinaryCache::read`].
pub enum BlobRead {
    Whole { bytes: Vec<u8>, mime: String },
    Part { bytes: Vec<u8>, mime: String, first: usize, total: usize },
    Unsatisfiable { total: usize },
}

/// Response of the `locus-binary` URI scheme for `path` (`/blob/{id}`).
///
/// Range requests are honoured: audio and video elements need them to seek
/// without loading the whole file again.
pub fn serve(cache: &BinaryCache, path: &str, range_header: Option<&str>) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{header, Response, StatusCode};

    let started = Instant::now();
    let id = path.strip_prefix("/blob/").unwrap_or("");
    let builder = |status: StatusCode| {
        Response::builder()
            .status(status)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCEPT_RANGES, "bytes")
    };
    let response = match cache.read(id, range_header.and_then(ByteRange::parse)) {
        Some(BlobRead::Whole { bytes, mime }) => {
            if mime == "application/octet-stream" {
                eprintln!(
                    "[perf:locus-binary] blob={id} status=200 bytes={} total={}ms",
                    bytes.len(),
                    started.elapsed().as_millis()
                );
            }
            builder(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CONTENT_LENGTH, bytes.len())
                .body(bytes)
        }
        Some(BlobRead::Part { bytes, mime, first, total }) => builder(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CONTENT_LENGTH, bytes.len())
            .header(header::CONTENT_RANGE, format!("bytes {first}-{}/{total}", first + bytes.len() - 1))
            .header(header::ACCESS_CONTROL_EXPOSE_HEADERS, "content-range")
            .body(bytes),
        Some(BlobRead::Unsatisfiable { total }) => builder(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{total}"))
            .body(Vec::new()),
        None => {
            eprintln!(
                "[perf:locus-binary] blob={id} status=404 total={}ms",
                started.elapsed().as_millis()
            );
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .body(Vec::new())
        }
    };
    // A stored mime type that is not a valid header value must not panic the handler.
    response.unwrap_or_else(|_| {
        let mut failed = Response::new(Vec::new());
        *failed.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        failed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: Duration = Duration::from_secs(600);

    fn blob(len: usize) -> Vec<u8> {
        vec![1u8; len]
    }

    /// `Instant` ties would make "oldest" ambiguous.
    fn tick() {
        std::thread::sleep(Duration::from_millis(3));
    }

    #[test]
    fn byte_budget_evicts_the_oldest_blobs_first() {
        let cache = BinaryCache::with_limits(LONG, 20, 100);
        let first = cache.insert(blob(40), "a".into());
        tick();
        let second = cache.insert(blob(40), "a".into());
        tick();
        let third = cache.insert(blob(40), "a".into());

        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
        assert!(cache.get(&third).is_some());
        assert_eq!(cache.resident_bytes(), 80);
    }

    #[test]
    fn reading_a_blob_keeps_it_ahead_of_unused_ones() {
        let cache = BinaryCache::with_limits(LONG, 20, 100);
        let used = cache.insert(blob(40), "a".into());
        tick();
        let unused = cache.insert(blob(40), "a".into());
        tick();
        assert!(cache.get(&used).is_some());
        tick();
        cache.insert(blob(40), "a".into());

        assert!(cache.get(&used).is_some());
        assert!(cache.get(&unused).is_none());
    }

    #[test]
    fn entry_cap_still_applies_to_small_blobs() {
        let cache = BinaryCache::with_limits(LONG, 2, usize::MAX);
        let first = cache.insert(blob(1), "a".into());
        tick();
        cache.insert(blob(1), "a".into());
        tick();
        cache.insert(blob(1), "a".into());

        assert_eq!(cache.entry_count(), 2);
        assert!(cache.get(&first).is_none());
    }

    #[test]
    fn a_blob_over_the_whole_budget_is_stored_alone() {
        let cache = BinaryCache::with_limits(LONG, 20, 100);
        for _ in 0..3 {
            cache.insert(blob(10), "a".into());
            tick();
        }
        let big = cache.insert(blob(200), "a".into());

        assert_eq!(cache.entry_count(), 1);
        assert_eq!(cache.get(&big).map(|(bytes, _)| bytes.len()), Some(200));
    }

    #[test]
    fn release_frees_the_bytes_once() {
        let cache = BinaryCache::with_limits(LONG, 20, 100);
        let id = cache.insert(blob(60), "a".into());

        assert!(cache.release(&id));
        assert!(!cache.release(&id));
        assert!(cache.get(&id).is_none());
        assert_eq!(cache.resident_bytes(), 0);
    }

    #[test]
    fn expired_blobs_are_dropped_on_the_next_insert() {
        let cache = BinaryCache::with_limits(Duration::ZERO, 20, 100);
        let stale = cache.insert(blob(10), "a".into());
        let fresh = cache.insert(blob(10), "a".into());

        assert!(cache.get(&stale).is_none());
        assert!(cache.get(&fresh).is_some());
    }

    fn digits() -> Vec<u8> {
        (0u8..10).collect()
    }

    fn served(range: Option<&str>) -> (u16, Vec<u8>, tauri::http::HeaderMap) {
        let cache = BinaryCache::with_limits(LONG, 20, usize::MAX);
        let id = cache.insert(digits(), "video/mp4".into());
        let response = serve(&cache, &format!("/blob/{id}"), range);
        (response.status().as_u16(), response.body().clone(), response.headers().clone())
    }

    fn header<'a>(headers: &'a tauri::http::HeaderMap, name: &str) -> &'a str {
        headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or("")
    }

    #[test]
    fn only_a_single_well_formed_byte_range_is_understood() {
        let range = |start, end| Some(ByteRange { start, end });
        assert_eq!(ByteRange::parse("bytes=0-99"), range(Some(0), Some(99)));
        assert_eq!(ByteRange::parse(" bytes=100- "), range(Some(100), None));
        assert_eq!(ByteRange::parse("bytes=-50"), range(None, Some(50)));
        for ignored in ["bytes=-", "bytes=5-2", "bytes=0-1,5-9", "items=0-1", "bytes=a-b", "0-1", ""] {
            assert_eq!(ByteRange::parse(ignored), None, "{ignored}");
        }
    }

    #[test]
    fn a_request_without_a_range_gets_the_whole_blob_and_advertises_ranges() {
        let (status, body, headers) = served(None);

        assert_eq!(status, 200);
        assert_eq!(body, digits());
        assert_eq!(header(&headers, "accept-ranges"), "bytes");
        assert_eq!(header(&headers, "content-length"), "10");
        assert_eq!(header(&headers, "content-type"), "video/mp4");
        assert_eq!(header(&headers, "access-control-allow-origin"), "*");
    }

    #[test]
    fn ranges_return_only_the_requested_bytes() {
        for (request, expected, content_range) in [
            ("bytes=2-5", vec![2, 3, 4, 5], "bytes 2-5/10"),
            ("bytes=8-", vec![8, 9], "bytes 8-9/10"),
            ("bytes=-3", vec![7, 8, 9], "bytes 7-9/10"),
            ("bytes=5-999", vec![5, 6, 7, 8, 9], "bytes 5-9/10"),
            ("bytes=0-0", vec![0], "bytes 0-0/10"),
        ] {
            let (status, body, headers) = served(Some(request));
            assert_eq!(status, 206, "{request}");
            assert_eq!(body, expected, "{request}");
            assert_eq!(header(&headers, "content-range"), content_range, "{request}");
            assert_eq!(header(&headers, "content-length"), expected.len().to_string(), "{request}");
        }
    }

    #[test]
    fn a_range_past_the_end_is_not_satisfiable() {
        for request in ["bytes=10-", "bytes=50-60", "bytes=-0"] {
            let (status, body, headers) = served(Some(request));
            assert_eq!(status, 416, "{request}");
            assert!(body.is_empty());
            assert_eq!(header(&headers, "content-range"), "bytes */10");
        }
    }

    #[test]
    fn a_malformed_range_is_ignored_and_the_whole_blob_is_served() {
        for request in ["bytes=5-2", "bytes=0-1,4-5", "garbage"] {
            let (status, body, _) = served(Some(request));
            assert_eq!((status, body), (200, digits()), "{request}");
        }
    }

    #[test]
    fn serving_renews_the_lease_and_an_unknown_blob_is_a_404() {
        let cache = BinaryCache::with_limits(LONG, 20, 100);
        let used = cache.insert(blob(40), "a".into());
        tick();
        let unused = cache.insert(blob(40), "a".into());
        tick();
        assert_eq!(serve(&cache, &format!("/blob/{used}"), Some("bytes=0-1")).status().as_u16(), 206);
        tick();
        cache.insert(blob(40), "a".into());

        assert_eq!(serve(&cache, &format!("/blob/{used}"), None).status().as_u16(), 200);
        assert_eq!(serve(&cache, &format!("/blob/{unused}"), None).status().as_u16(), 404);
        assert_eq!(serve(&cache, "/blob/", None).status().as_u16(), 404);
    }
}
