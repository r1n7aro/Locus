//! A merge journal keeps only what an unfinished job can still use.
//!
//! - A committed or aborted job is removed when it ends, with its private
//!   refs (`refs/locus/merge-jobs/<id>/`).
//! - An applied or staged job stays usable for validation, stage and commit
//!   until its destination moves on (HEAD, branch or an applied file changed,
//!   or the checkout is gone), or after a week without use.
//! - A prepared job expires after three days without use, or when its
//!   destination checkout or branch is gone (replan needs both).
//! - A job mid-transaction (`applying`, `staging`, `committing`) or one that
//!   needs recovery is never removed automatically.
//!
//! Every job end and every prepare runs this prune under the journal lock,
//! then removes shared blobs no remaining job refers to, ages out derived
//! caches and deletes obsolete stores. Directories leave through a trash
//! folder (one rename each) and are deleted in the background.
use super::io::{fingerprint, git, git_text, journal_root, repository, BLOB_STORE};
use super::journal;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

const PREPARED_TTL: Duration = Duration::from_secs(3 * 24 * 3600);
const APPLIED_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const CACHE_BYTES: u64 = 256 * 1024 * 1024;
const TRASH: &str = "trash-";
const LOCK: &str = "operation.lock";

pub(super) fn catalog_cache() -> String {
    format!("catalog-cache-{}-v3", crate::unity_asset_core::PARSER_VERSION)
}
pub(super) fn reference_cache() -> String {
    format!("reference-cache-{}-objects-v4", crate::unity_asset_core::PARSER_VERSION)
}
pub(super) const GUID_INDEX: &str = "guid-index-v2";
pub(super) const SCRIPT_CACHE: &str = "script-cache-v2";
pub(super) const SELECTED_SCHEMAS: &str = "schema-cache-selected-v2";
pub(super) const TREE_SCHEMAS: &str = "schema-cache-v1";

fn caches() -> [String; 6] {
    [catalog_cache(), reference_cache(), GUID_INDEX.into(), SCRIPT_CACHE.into(), SELECTED_SCHEMAS.into(), TREE_SCHEMAS.into()]
}

const IN_FLIGHT: &[&str] = &["applying", "staging", "committing", "recovery_required"];

fn is_job(name: &str) -> bool {
    uuid::Uuid::parse_str(name).is_ok()
}

fn idle_for(path: &Path) -> Duration {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .unwrap_or_default()
}

/// Whether a checkout still exists, with its branch and HEAD, read once per
/// prune however many jobs it has.
type Destinations = BTreeMap<PathBuf, (bool, Option<String>, Option<String>)>;

/// Why a version-3 job is finished, or None while it can still be used.
fn finished(dir: &Path, destinations: &mut Destinations) -> Option<&'static str> {
    let record: Value = serde_json::from_slice(&std::fs::read(dir.join("job.json")).ok()?).ok()?;
    let state = record["state"].as_str().unwrap_or_default();
    if matches!(state, "committed" | "aborted") {
        return Some("ended");
    }
    if IN_FLIGHT.contains(&state) {
        return None;
    }
    let root = Path::new(record["root"].as_str().unwrap_or_default());
    let (exists, branch, head) = destinations.entry(root.to_path_buf()).or_insert_with(|| {
        if !root.join(".git").exists() {
            return (false, None, None);
        }
        (true, git_text(root, &["symbolic-ref", "--quiet", "HEAD"]).ok(), git_text(root, &["rev-parse", "HEAD"]).ok())
    }).clone();
    if !exists {
        return Some("destination checkout removed");
    }
    if branch.as_deref() != record["snapshot"]["branch"].as_str() {
        return Some("destination branch changed");
    }
    let idle = idle_for(&dir.join("job.json"));
    match state {
        "prepared" => (idle > PREPARED_TTL).then_some("unused for three days"),
        "applied" | "staged" => {
            if idle > APPLIED_TTL {
                return Some("unused for a week");
            }
            if head.as_deref() != record["snapshot"]["head"].as_str() {
                return Some("destination HEAD moved");
            }
            let applied: BTreeMap<String, Option<super::FileState>> =
                serde_json::from_value(record["applied_files"].clone()).unwrap_or_default();
            let moved = applied.iter().any(|(path, state)| {
                fingerprint(root, path, state.as_ref()).map_or(true, |current| &current != state)
            });
            moved.then_some("applied files changed")
        }
        _ => None,
    }
}

/// Directories queued for deletion by this process.
static DELETING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Moves a directory out of the journal. A failure (a file held open on
/// Windows) never blocks a merge: the directory stays for the next prune.
fn to_trash(journal: &Path, path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let trash = journal.join(format!("{TRASH}{}", uuid::Uuid::new_v4().simple()));
    match std::fs::rename(path, &trash) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(target: "locus::merge_jobs", path = %path.display(), %error, "Could not remove a merge journal directory; retrying at the next prune");
            false
        }
    }
}

/// Deletes every trash folder, off the caller's thread unless `wait`.
fn empty_trash(journal: &Path, wait: bool) {
    let Ok(entries) = std::fs::read_dir(journal) else { return };
    let mut queued = DELETING.lock().unwrap_or_else(|e| e.into_inner());
    let folders: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(TRASH))
        .map(|e| e.path())
        .filter(|p| wait || queued.insert(p.clone()))
        .collect();
    drop(queued);
    if folders.is_empty() {
        return;
    }
    let delete = move || {
        for folder in folders {
            let _ = std::fs::remove_dir_all(&folder);
            DELETING.lock().unwrap_or_else(|e| e.into_inner()).remove(&folder);
        }
    };
    if wait {
        delete();
    } else {
        // If no thread can start, the folders wait for the next prune.
        let _ = std::thread::Builder::new().name("merge-journal-trash".into()).spawn(delete);
    }
}

/// Removes refs of jobs that no longer have a directory.
fn delete_orphan_refs(repo: &Path, live: &BTreeSet<String>) -> Result<(), String> {
    let refs = git_text(repo, &["for-each-ref", "--format=%(refname)", "refs/locus/merge-jobs/"])?;
    let mut commands = String::new();
    for name in refs.lines() {
        let id = name.trim_start_matches("refs/locus/merge-jobs/").split('/').next().unwrap_or_default();
        if !live.contains(id) {
            commands.push_str(&format!("delete {name}\n"));
        }
    }
    if !commands.is_empty() {
        super::io::git_input(repo, &["update-ref", "--stdin"], commands.as_bytes(), None)?;
    }
    Ok(())
}

/// Blob names every file of a live job refers to (`"blob":"<name>"`).
fn referenced_blobs(dir: &Path, into: &mut HashSet<String>) -> Result<(), String> {
    const KEY: &[u8] = b"\"blob\":\"";
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".json") {
            continue;
        }
        let bytes = std::fs::read(entry.path()).map_err(|e| e.to_string())?;
        let mut rest = &bytes[..];
        while let Some(at) = rest.windows(KEY.len()).position(|w| w == KEY) {
            rest = &rest[at + KEY.len()..];
            let end = rest.iter().position(|b| *b == b'"').unwrap_or(0);
            if let Ok(name) = std::str::from_utf8(&rest[..end]) {
                into.insert(name.to_string());
            }
            rest = &rest[end..];
        }
    }
    Ok(())
}

/// Oldest entries leave first once a cache exceeds its age or size bound.
fn age_cache(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut files: Vec<(Duration, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (idle_for(&e.path()), meta.len(), e.path()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    for (idle, size, path) in files {
        if idle <= CACHE_TTL && total <= CACHE_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

/// Removes finished jobs and everything only they used. The caller holds the
/// journal lock. `keep` names a job the caller is still reading.
pub(super) fn prune(root: &Path, keep: Option<&str>) -> Result<Value, String> {
    let repo = repository(root)?;
    let journal = journal_root(&repo)?;
    let mut removed = Vec::new();
    let mut live = BTreeSet::new();
    let mut legacy_live = false;
    let mut obsolete = Vec::new();
    let mut destinations = Destinations::new();
    let current = caches();
    for entry in std::fs::read_dir(&journal).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if name == LOCK || name == BLOB_STORE || name.starts_with(TRASH) {
            continue;
        }
        if current.contains(&name) {
            age_cache(&path);
            continue;
        }
        if !is_job(&name) {
            obsolete.push(path);
            continue;
        }
        if keep == Some(name.as_str()) {
            live.insert(name);
            continue;
        }
        let reason = match journal::header(&path) {
            None => Some("incomplete"),
            Some((version, state)) if version != journal::VERSION as u64 => {
                if IN_FLIGHT.contains(&state.as_str()) {
                    legacy_live = true;
                    tracing::warn!(target: "locus::merge_jobs", job = %name, state = %state,
                        "An earlier Locus left this merge job {state}; it is kept for manual recovery");
                    None
                } else {
                    Some("earlier journal version")
                }
            }
            Some(_) => finished(&path, &mut destinations),
        };
        match reason {
            Some(reason) => {
                journal::forget(&path);
                if to_trash(&journal, &path) {
                    removed.push(json!({"job_id": name, "reason": reason}));
                } else {
                    live.insert(name);
                }
            }
            None => {
                live.insert(name);
            }
        }
    }
    for path in obsolete {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if legacy_live && matches!(name.as_str(), "blobs-v2" | "git-blobs-v2") {
            continue;
        }
        if path.is_dir() {
            to_trash(&journal, &path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    let mut referenced = HashSet::new();
    let known = live.iter().try_for_each(|id| referenced_blobs(&journal.join(id), &mut referenced));
    let mut swept = 0usize;
    if let Err(error) = known {
        // Without every live job's references no blob is provably unused.
        tracing::warn!(target: "locus::merge_jobs", %error, "Skipped the merge blob sweep");
    } else if let Ok(entries) = std::fs::read_dir(journal.join(BLOB_STORE)) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !referenced.contains(&name) && std::fs::remove_file(entry.path()).is_ok() {
                swept += 1;
            }
        }
    }
    if let Err(error) = delete_orphan_refs(&repo, &live) {
        tracing::warn!(target: "locus::merge_jobs", %error, "Could not delete private refs of removed merge jobs");
    }
    empty_trash(&journal, cfg!(test));
    if !removed.is_empty() || swept > 0 {
        tracing::info!(target: "locus::merge_jobs", removed = %json!(removed), swept_blobs = swept, live = live.len(), "Merge journal pruned");
    }
    Ok(json!({"removed": removed, "swept_blobs": swept, "live_jobs": live.len()}))
}

/// Ends a job: its directory, refs and the blobs only it used go away. The
/// caller holds the journal lock.
pub(super) fn retire(root: &Path, id: &str) -> Result<(), String> {
    let repo = repository(root)?;
    let journal = journal_root(&repo)?;
    let dir = journal.join(id);
    journal::forget(&dir);
    to_trash(&journal, &dir);
    prune(root, None).map(|_| ())
}

/// Removes a job directory whose prepare failed; best effort.
pub(super) struct PrepareGuard {
    root: PathBuf,
    id: String,
    armed: bool,
}

impl PrepareGuard {
    pub(super) fn new(root: &Path, id: &str) -> Self {
        Self { root: root.to_path_buf(), id: id.to_string(), armed: true }
    }
    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for PrepareGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(journal) = journal_root(&self.root) {
            let dir = journal.join(&self.id);
            journal::forget(&dir);
            to_trash(&journal, &dir);
            let _ = git(&self.root, &["for-each-ref", "--format=delete %(refname)", &format!("refs/locus/merge-jobs/{}/", self.id)])
                .and_then(|commands| super::io::git_input(&self.root, &["update-ref", "--stdin"], &commands, None).map(|_| Vec::<u8>::new()));
            empty_trash(&journal, cfg!(test));
        }
    }
}
