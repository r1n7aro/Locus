//! On-disk layout of a merge job (journal version 3). A job directory holds
//! a few files and is removed as a whole when the job ends (see `retention`):
//!
//! - `job.json`: the mutable record (state, plan, results). Every mutation
//!   rewrites only this file; it stays small even for a large merge because a
//!   file whose changes all carry one decision is recorded once.
//! - `<part>-<hash>.json` for each large part (target snapshot, Git index,
//!   dependency inputs, deltas with the catalog, schemas), written when it
//!   changes. The record names each part by content hash.
//! - `preview-latest.json`: the last rendered preview.
//!
//! Parts are immutable once written, so a process keeps parsed parts by
//! content hash: loading a job reads `job.json` and reuses the rest.
use super::io::atomic_bytes;
use super::shared::Shared;
use super::types::{CommitDelta, MergeJob};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Map, Value};
use std::any::Any;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

pub(super) const VERSION: u32 = 3;
/// (record key, file name prefix) of each part.
const PARTS: [(&str, &str); 6] = [
    ("files", "files"),
    ("index_records", "index-records"),
    ("index_flags", "index-flags"),
    ("dependency_files", "dependencies"),
    ("deltas", "deltas"),
    ("schemas", "schemas"),
];
const RECORD: &str = "job.json";
/// Parsed parts kept per process, bounded by their stored size.
const CACHE_BYTES: usize = 192 * 1024 * 1024;

struct Cached {
    dir: PathBuf,
    name: &'static str,
    hash: String,
    bytes: usize,
    value: Arc<dyn Any + Send + Sync>,
}

static CACHE: LazyLock<Mutex<Vec<Cached>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn cache_get<T: Send + Sync + 'static>(dir: &Path, name: &str, hash: &str) -> Option<Arc<T>> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let index = cache.iter().position(|c| c.name == name && c.hash == hash && c.dir == dir)?;
    let entry = cache.remove(index);
    let value = entry.value.clone().downcast::<T>().ok();
    cache.push(entry);
    value
}

fn cache_put<T: Send + Sync + 'static>(dir: &Path, name: &'static str, hash: &str, bytes: usize, value: Arc<T>) {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|c| !(c.name == name && c.dir == dir));
    cache.push(Cached { dir: dir.to_path_buf(), name, hash: hash.to_string(), bytes, value });
    let mut total: usize = cache.iter().map(|c| c.bytes).sum();
    while total > CACHE_BYTES && cache.len() > 1 {
        total -= cache.remove(0).bytes;
    }
}

/// Drops the cached parts of a removed job.
pub(super) fn forget(dir: &Path) {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| c.dir != dir);
}

fn file_name(name: &str, hash: &str) -> String {
    format!("{name}-{}.json", &hash[..16])
}

fn save_part<T: Serialize + Send + Sync + 'static>(dir: &Path, name: &'static str, part: &Shared<T>) -> Result<String, String> {
    if let Some(hash) = part.stored_in(dir) {
        if dir.join(file_name(name, hash)).is_file() {
            return Ok(hash.to_string());
        }
    }
    let bytes = serde_json::to_vec(&**part).map_err(|e| e.to_string())?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let path = dir.join(file_name(name, &hash));
    if !path.is_file() {
        atomic_bytes(&path, &bytes)?;
    }
    part.mark_stored(dir, hash.clone());
    cache_put(dir, name, &hash, bytes.len(), part.arc().clone());
    Ok(hash)
}

fn load_part<T: DeserializeOwned + Send + Sync + 'static>(dir: &Path, name: &'static str, hash: &str) -> Result<Shared<T>, String> {
    if let Some(value) = cache_get::<T>(dir, name, hash) {
        return Ok(Shared::from_arc(value, dir, hash.to_string()));
    }
    let path = dir.join(file_name(name, hash));
    let bytes = std::fs::read(&path).map_err(|e| format!("Merge job part {} is unreadable: {e}", path.display()))?;
    if blake3::hash(&bytes).to_hex().as_str() != hash {
        return Err(format!("Merge job part {} is corrupt", path.display()));
    }
    let value: Arc<T> = Arc::new(serde_json::from_slice(&bytes).map_err(|e| e.to_string())?);
    cache_put(dir, name, hash, bytes.len(), value.clone());
    Ok(Shared::from_arc(value, dir, hash.to_string()))
}

/// Hashes of the parts, written to disk first when they changed.
fn parts(dir: &Path, job: &MergeJob, write: bool) -> Result<Map<String, Value>, String> {
    fn hash<T: Serialize + Send + Sync + 'static>(dir: &Path, name: &'static str, part: &Shared<T>, write: bool) -> Result<String, String> {
        if write {
            return save_part(dir, name, part);
        }
        match part.stored_in(dir) {
            Some(hash) => Ok(hash.to_string()),
            None => Ok(blake3::hash(&serde_json::to_vec(&**part).map_err(|e| e.to_string())?).to_hex().to_string()),
        }
    }
    let mut out = Map::new();
    out.insert("files".into(), json!(hash(dir, "files", &job.snapshot.files, write)?));
    out.insert("index_records".into(), json!(hash(dir, "index-records", &job.snapshot.index_records, write)?));
    out.insert("index_flags".into(), json!(hash(dir, "index-flags", &job.snapshot.index_flags, write)?));
    out.insert("dependency_files".into(), json!(hash(dir, "dependencies", &job.dependency_files, write)?));
    out.insert("deltas".into(), json!(hash(dir, "deltas", &job.deltas, write)?));
    out.insert("schemas".into(), json!(hash(dir, "schemas", &job.schemas, write)?));
    Ok(out)
}

/// `{files: [{commit, path, decision}], changes: {id: decision}}`: a delta
/// whose every change has the same decision is one entry.
fn compact(decisions: &BTreeMap<String, Value>, deltas: &[CommitDelta]) -> Value {
    let mut covered: HashSet<&str> = HashSet::new();
    let mut files = Vec::new();
    for delta in deltas {
        let ids: Vec<&str> = delta.changes.iter().filter_map(|c| c["id"].as_str()).collect();
        let Some(first) = ids.first().and_then(|id| decisions.get(*id)) else { continue };
        if ids.len() > 1 && ids.iter().all(|id| decisions.get(*id) == Some(first)) {
            files.push(json!({"commit":delta.commit,"path":delta.path,"decision":first}));
            covered.extend(ids);
        }
    }
    let changes: Map<String, Value> = decisions
        .iter()
        .filter(|(id, _)| !covered.contains(id.as_str()))
        .map(|(id, decision)| (id.clone(), decision.clone()))
        .collect();
    json!({"files": files, "changes": changes})
}

fn expand(value: &Value, deltas: &[CommitDelta]) -> Result<BTreeMap<String, Value>, String> {
    let mut decisions: BTreeMap<String, Value> = value["changes"]
        .as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let by_file: HashMap<(&str, &str), &CommitDelta> =
        deltas.iter().map(|d| ((d.commit.as_str(), d.path.as_str()), d)).collect();
    for entry in value["files"].as_array().into_iter().flatten() {
        let key = (entry["commit"].as_str().unwrap_or_default(), entry["path"].as_str().unwrap_or_default());
        let delta = by_file.get(&key).ok_or("Merge job record names a file outside its catalog")?;
        for id in delta.changes.iter().filter_map(|c| c["id"].as_str()) {
            decisions.insert(id.to_string(), entry["decision"].clone());
        }
    }
    Ok(decisions)
}

fn record(dir: &Path, job: &MergeJob, write: bool) -> Result<Value, String> {
    let mut record = serde_json::to_value(job).map_err(|e| e.to_string())?;
    let object = record.as_object_mut().ok_or("Merge job record is not an object")?;
    object.insert("parts".into(), Value::Object(parts(dir, job, write)?));
    let selection = object.get_mut("selection").and_then(Value::as_object_mut).ok_or("Merge job record has no selection")?;
    selection.insert("decisions".into(), compact(&job.selection.decisions, &job.deltas));
    Ok(record)
}

pub(super) fn save(dir: &Path, job: &MergeJob) -> Result<(), String> {
    let record = record(dir, job, true)?;
    atomic_bytes(&dir.join(RECORD), &serde_json::to_vec(&record).map_err(|e| e.to_string())?)?;
    // Superseded part versions are never read again.
    let current: HashSet<String> = PARTS
        .iter()
        .filter_map(|(key, name)| record["parts"][*key].as_str().map(|hash| file_name(name, hash)))
        .collect();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let part = PARTS.iter().any(|(_, prefix)| {
            name.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('-') && rest.len() == 22)
        });
        if part && name.ends_with(".json") && !current.contains(&name) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

/// The record's identity: a preview is cached under it.
pub(super) fn fingerprint(dir: &Path, job: &MergeJob) -> Result<String, String> {
    let record = record(dir, job, false)?;
    Ok(blake3::hash(&serde_json::to_vec(&record).map_err(|e| e.to_string())?).to_hex().to_string())
}

/// `version` and `state` of a record, for retention decisions on any version.
pub(super) fn header(dir: &Path) -> Option<(u64, String)> {
    #[derive(serde::Deserialize)]
    struct Header {
        version: u64,
        state: String,
    }
    let bytes = std::fs::read(dir.join(RECORD)).ok()?;
    let header: Header = serde_json::from_slice(&bytes).ok()?;
    Some((header.version, header.state))
}

pub(super) fn load(dir: &Path) -> Result<MergeJob, String> {
    let bytes = match std::fs::read(dir.join(RECORD)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "Merge job {} no longer exists; a job is removed once it is committed or aborted, or after its destination moved on. Prepare a new job",
                dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
        }
        Err(e) => return Err(e.to_string()),
    };
    let mut record: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if record["version"].as_u64() != Some(VERSION as u64) {
        return Err("This merge job was created by an earlier Locus version; prepare a new job".into());
    }
    let parts = record.as_object_mut().and_then(|o| o.remove("parts")).ok_or("Merge job record has no parts")?;
    let decisions = record["selection"].as_object_mut().and_then(|o| o.remove("decisions")).unwrap_or(Value::Null);
    let mut job: MergeJob = serde_json::from_value(record).map_err(|e| e.to_string())?;
    let hash = |name: &str| parts[name].as_str().map(str::to_string).ok_or(format!("Merge job record has no {name} part"));
    job.snapshot.files = load_part(dir, "files", &hash("files")?)?;
    job.snapshot.index_records = load_part(dir, "index-records", &hash("index_records")?)?;
    job.snapshot.index_flags = load_part(dir, "index-flags", &hash("index_flags")?)?;
    job.dependency_files = load_part(dir, "dependencies", &hash("dependency_files")?)?;
    job.deltas = load_part(dir, "deltas", &hash("deltas")?)?;
    job.schemas = load_part(dir, "schemas", &hash("schemas")?)?;
    job.selection.decisions = expand(&decisions, &job.deltas)?;
    Ok(job)
}
