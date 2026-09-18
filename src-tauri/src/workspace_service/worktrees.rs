//! Managed linked worktrees. Git owns the checkout/index; this journal owns
//! Locus project assignment, lifecycle and content epochs. No stash or reset
//! is ever performed against a source checkout.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};

use fs4::FileExt;
use serde::{Deserialize, Serialize};

use super::identity::{
    normalize_existing_workspace_root, resolve_git_common_dir, CheckoutId, ProjectIdResolver,
};

#[path = "path_boundary.rs"]
mod path_boundary;
pub(crate) use path_boundary::{
    existing_path_contains, existing_path_relative, path_components_equal, path_relative,
    prospective_path_relative,
};

const SCHEMA_VERSION: u32 = 1;

pub mod selector;
pub mod usage;
pub mod estimate;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorktree {
    pub checkout_id: String,
    pub project_id: String,
    pub root: String,
    pub repo_root: String,
    pub project_relative_path: String,
    pub branch: Option<String>,
    pub head_oid: String,
    pub materialization_epoch: u64,
    pub managed: bool,
    pub lifecycle: String,
    pub dirty: bool,
    pub pool_slot: bool,
    pub assignment_id: Option<String>,
    pub editor_version: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorktreeRequest {
    pub source_root: String,
    pub destination: String,
    pub branch: String,
    #[serde(default)]
    pub start_ref: Option<String>,
    #[serde(default)]
    pub include_dirty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeOperation {
    pub id: String,
    pub kind: String,
    pub checkout_id: String,
    pub destination: String,
    pub expected_head: String,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorktreeState {
    version: u32,
    pub records: BTreeMap<String, ManagedWorktree>,
    pub operations: Vec<WorktreeOperation>,
}

impl Default for WorktreeState {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            records: BTreeMap::new(),
            operations: Vec::new(),
        }
    }
}

pub(crate) struct WorktreeStore {
    pub common_dir: PathBuf,
    _lock: File,
    pub state: WorktreeState,
}

fn io_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Git for Windows does not accept verbatim prefixes in several argument/env
/// positions even with core.longpaths. This representation is for the child
/// process only, never for persisted identities or filesystem boundary checks.
pub(crate) fn git_cli_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(drive) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(drive);
        }
    }
    path.to_path_buf()
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = crate::process_util::command("git");
    #[cfg(windows)]
    command.args(["-c", "core.longpaths=true"]);
    let output = command
        .arg("-C")
        .arg(git_cli_path(root))
        .args(args.iter().map(|arg| git_cli_path(Path::new(arg))))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(io_error)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut tail = stderr.chars().rev().take(4096).collect::<Vec<_>>();
        tail.reverse();
        let detail: String = tail.into_iter().collect();
        return Err(format!(
            "git {} failed for project root {}: {}",
            args.first().unwrap_or(&""),
            root.display(),
            detail.trim()
        ));
    }
    Ok(output.stdout)
}

pub(crate) fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git(root, args)?)
        .map(|s| s.trim().to_string())
        .map_err(io_error)
}

pub(crate) fn canonical(path: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(path)
        .map(|p| dunce::simplified(&p).to_path_buf())
        .map_err(io_error)
}

pub(crate) fn path_key(path: &Path) -> String {
    let key = dunce::simplified(path).to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

impl WorktreeStore {
    pub(crate) fn open(root: &Path) -> Result<Self, String> {
        let common_dir = resolve_git_common_dir(root).ok_or("Workspace is not a Git checkout")?;
        let dir = common_dir.join("locus-worktrees");
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("registry.lock"))
            .map_err(io_error)?;
        FileExt::try_lock(&lock)
            .map_err(|e| format!("Another Locus worktree operation holds this repository: {e}"))?;
        let state = read_state(&common_dir)?;
        Ok(Self {
            common_dir,
            _lock: lock,
            state,
        })
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(io_error)?;
        crate::config::atomic_write_config(
            &self.common_dir.join("locus-worktrees/state.json"),
            &bytes,
        )
    }

    pub(crate) fn begin(&mut self, record: &ManagedWorktree, kind: &str) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.state.operations.push(WorktreeOperation {
            id: id.clone(),
            kind: kind.into(),
            checkout_id: record.checkout_id.clone(),
            destination: record.repo_root.clone(),
            expected_head: record.head_oid.clone(),
            state: "pending".into(),
            error: None,
        });
        self.save()?;
        Ok(id)
    }

    pub(crate) fn finish(&mut self, id: &str, error: Option<String>) -> Result<(), String> {
        if let Some(op) = self.state.operations.iter_mut().find(|op| op.id == id) {
            op.state = if error.is_some() {
                "failed"
            } else {
                "complete"
            }
            .into();
            op.error = error;
        }
        self.save()
    }
}

fn read_state(common_dir: &Path) -> Result<WorktreeState, String> {
    let path = common_dir.join("locus-worktrees/state.json");
    if !path.exists() {
        return Ok(WorktreeState::default());
    }
    let state: WorktreeState =
        serde_json::from_slice(&std::fs::read(path).map_err(io_error)?).map_err(io_error)?;
    // Explicit version gate: never reinterpret a newer on-disk lifecycle.
    if state.version != SCHEMA_VERSION {
        return Err(format!(
            "Unsupported worktree journal version {}",
            state.version
        ));
    }
    Ok(state)
}

/// Read-only identity lookup, deliberately independent of ProjectIdResolver.
pub(crate) fn record_for_root(root: &Path) -> Result<Option<ManagedWorktree>, String> {
    let Some(common_dir) = resolve_git_common_dir(root) else {
        return Ok(None);
    };
    let normalized = normalize_existing_workspace_root(root).map_err(io_error)?;
    let id = CheckoutId::from_normalized_root(&normalized).to_string();
    Ok(read_state(&common_dir)?.records.remove(&id))
}

pub(crate) fn runtime_lease(root: &Path) -> Result<Option<File>, String> {
    if let Some(common) = resolve_git_common_dir(root) {
        let current_root = canonical(root)?;
        let state = read_state(&common)?;
        for record in state
            .records
            .values()
            .filter(|record| record.checkout_id.starts_with("reservation-"))
        {
            if prospective_path_relative(Path::new(&record.repo_root), &current_root)?.is_some() {
                return Err("Checkout belongs to an incomplete pool reservation".into());
            }
        }
        for operation in state
            .operations
            .iter()
            .filter(|operation| matches!(operation.state.as_str(), "pending" | "recovery_required"))
        {
            if prospective_path_relative(Path::new(&operation.destination), &current_root)?
                .is_some()
            {
                return Err("Checkout has an incomplete materialization operation; inspect its journal before opening it".into());
            }
        }
    }
    let Some(record) = record_for_root(root)? else {
        return Ok(None);
    };
    if record.lifecycle != "active" {
        return Err(format!(
            "Checkout is {} and cannot execute sessions",
            record.lifecycle
        ));
    }
    let common = resolve_git_common_dir(root).ok_or("Missing Git common directory")?;
    let file = lease_file(&common, &record.checkout_id)?;
    FileExt::try_lock_shared(&file)
        .map_err(|e| format!("Checkout materialization is changing: {e}"))?;
    // Mutation may have completed between the first read and shared lock.
    let current = record_for_root(root)?.ok_or("Checkout assignment was removed")?;
    if current.lifecycle != "active"
        || current.materialization_epoch != record.materialization_epoch
    {
        return Err("Checkout assignment changed; register again".into());
    }
    Ok(Some(file))
}

fn lease_file(common: &Path, checkout_id: &str) -> Result<File, String> {
    if checkout_id.contains(['/', '\\']) {
        return Err("Invalid checkout identity".into());
    }
    let dir = common.join("locus-worktrees/leases");
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(format!("{checkout_id}.lock")))
        .map_err(io_error)
}

pub(crate) fn mutation_lease(common: &Path, checkout_id: &str) -> Result<File, String> {
    let file = lease_file(common, checkout_id)?;
    FileExt::try_lock(&file).map_err(|e| format!("Checkout is registered in a Locus process; close its panes and services before recycling: {e}"))?;
    Ok(file)
}

fn prospective_destination(path: &Path, source_repo: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err("An absolute worktree directory is required".into());
    }
    if path.exists() {
        return Err("Worktree destination already exists".into());
    }
    let parent_path = path.parent().ok_or("Missing destination parent")?;
    let parent = canonical(parent_path).map_err(|error| {
        format!(
            "Worktree destination parent {} is unavailable: {error}",
            parent_path.display()
        )
    })?;
    let result = parent.join(path.file_name().unwrap());
    if prospective_path_relative(source_repo, &result)?.is_some() {
        return Err("Worktrees must be outside the source checkout".into());
    }
    Ok(result)
}

pub(crate) fn safe_relative(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(format!("Unsafe repository-relative path {value}"));
    }
    if value
        .split(['/', '\\'])
        .any(|p| p.eq_ignore_ascii_case(".git"))
    {
        return Err("Git administrative files cannot be transferred".into());
    }
    Ok(path.to_path_buf())
}

pub(crate) fn ensure_tree_supported(repo: &Path, oid: &str, relative: &str) -> Result<(), String> {
    let tree = git(repo, &["ls-tree", "-r", "-z", oid])?;
    for entry in tree.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        if entry.starts_with(b"160000 ") || entry.starts_with(b"120000 ") {
            return Err("Worktree creation currently requires submodules and symbolic links to be materialized independently".into());
        }
    }
    let manifest = if relative.is_empty() {
        "Packages/manifest.json".into()
    } else {
        format!("{relative}/Packages/manifest.json")
    };
    if let Ok(contents) = git(repo, &["show", &format!("{oid}:{manifest}")]) {
        ensure_local_packages_supported(&contents)?;
    }
    Ok(())
}

fn ensure_local_packages_supported(bytes: &[u8]) -> Result<(), String> {
    let manifest: serde_json::Value = serde_json::from_slice(bytes).map_err(io_error)?;
    if manifest
        .get("dependencies")
        .and_then(|d| d.as_object())
        .is_some_and(|deps| {
            deps.values()
                .any(|v| v.as_str().is_some_and(|v| v.starts_with("file:")))
        })
    {
        return Err(
            "Local file: package dependencies require an explicit isolated dependency mapping"
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn editor_version(root: &Path) -> Option<String> {
    std::fs::read_to_string(root.join("ProjectSettings/ProjectVersion.txt"))
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("m_EditorVersion:")
                .map(|v| v.trim().into())
        })
}

#[derive(Debug, PartialEq, Eq)]
struct DirtySnapshot {
    status: Vec<u8>,
    files: BTreeMap<String, Option<Vec<u8>>>,
}

fn dirty_snapshot(repo: &Path) -> Result<DirtySnapshot, String> {
    let status = git(
        repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut paths = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", "--no-renames", "-z", "HEAD", "--"],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        for name in git(repo, &args)?
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
        {
            paths.insert(String::from_utf8(name.to_vec()).map_err(io_error)?);
        }
    }
    let mut files = BTreeMap::new();
    let mut bytes_total = 0u64;
    for name in paths {
        let relative = safe_relative(&name)?;
        let path = repo.join(relative);
        let bytes = match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
            Ok(meta) => {
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err(format!("Cannot snapshot non-regular file {name}"));
                }
                if !existing_path_contains(repo, &path)? {
                    return Err(format!("Snapshot path escapes repository: {name}"));
                }
                bytes_total += meta.len();
                if bytes_total > 1024 * 1024 * 1024 {
                    return Err("Dirty snapshot exceeds the 1 GiB preparation budget".into());
                }
                Some(std::fs::read(path).map_err(io_error)?)
            }
        };
        if name.ends_with("Packages/manifest.json") {
            if let Some(bytes) = bytes.as_ref() {
                ensure_local_packages_supported(bytes)?;
            }
        }
        files.insert(name, bytes);
    }
    Ok(DirtySnapshot { status, files })
}

fn transfer_dirty(destination: &Path, snapshot: &DirtySnapshot) -> Result<(), String> {
    for (name, contents) in &snapshot.files {
        let path = destination.join(safe_relative(name)?);
        if let Some(contents) = contents {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(io_error)?;
            std::fs::write(path, contents).map_err(io_error)?;
        } else if path.is_file() {
            std::fs::remove_file(path).map_err(io_error)?;
        }
    }
    Ok(())
}

/// Git can leave a pointer as a regular file when LFS filters are absent or
/// disabled. A successful checkout alone therefore does not prove that Unity
/// received usable source assets.
pub(crate) fn ensure_lfs_materialized(repo: &Path) -> Result<(), String> {
    let paths = git(repo, &["ls-files", "-z"])?;
    let mut missing = Vec::new();
    for path in paths.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = String::from_utf8(path.to_vec()).map_err(io_error)?;
        let full = repo.join(safe_relative(&path)?);
        let Ok(meta) = std::fs::metadata(&full) else {
            continue;
        };
        if !meta.is_file() || meta.len() > 1024 {
            continue;
        }
        let bytes = std::fs::read(&full).map_err(io_error)?;
        if bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n")
            || bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\r\n")
        {
            missing.push(path);
            if missing.len() == 8 {
                break;
            }
        }
    }
    if !missing.is_empty() {
        return Err(format!("Checkout contains unresolved Git LFS pointers; fetch the source objects before opening Unity: {}", missing.join(", ")));
    }
    Ok(())
}

pub fn create(request: &CreateWorktreeRequest) -> Result<ManagedWorktree, String> {
    create_for_branch(request, false)
}

fn create_for_branch(request: &CreateWorktreeRequest, existing_branch: bool) -> Result<ManagedWorktree, String> {
    let source = canonical(Path::new(&request.source_root))?;
    let identity = ProjectIdResolver::resolve(&source).map_err(io_error)?;
    let repo = canonical(Path::new(&git_text(
        &source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let relative = existing_path_relative(&repo, &source)?
        .ok_or("Unity project is outside its source repository")?
        .to_string_lossy()
        .replace('\\', "/");
    let destination = prospective_destination(Path::new(&request.destination), &repo)?;
    if request.branch.trim().is_empty() || request.branch.starts_with(['-', '@']) {
        return Err("A literal new branch name is required".into());
    }
    git(&repo, &["check-ref-format", "--branch", &request.branch])?;
    let head = git_text(&repo, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let start = request.start_ref.as_deref().unwrap_or("HEAD");
    if start.starts_with('-') {
        return Err("Invalid start revision".into());
    }
    let oid = git_text(
        &repo,
        &["rev-parse", "--verify", &format!("{start}^{{commit}}")],
    )?;
    if request.include_dirty && oid != head {
        return Err("Including local changes requires the source HEAD as the start commit".into());
    }
    ensure_tree_supported(&repo, &oid, &relative)?;
    ensure_git_idle(&repo)?;
    let dirty = if request.include_dirty {
        Some(dirty_snapshot(&repo)?)
    } else {
        None
    };
    let mut store = WorktreeStore::open(&source)?;
    let checkout_root = destination.join(&relative);
    // CheckoutId is path-derived; materialize directory before normalizing it.
    let operation_id = uuid::Uuid::new_v4().to_string();
    store.state.operations.push(WorktreeOperation {
        id: operation_id.clone(),
        kind: "create".into(),
        checkout_id: String::new(),
        destination: destination.to_string_lossy().into(),
        expected_head: oid.clone(),
        state: "pending".into(),
        error: None,
    });
    store.save()?;
    let result: Result<ManagedWorktree, String> = (|| {
        if existing_branch {
            git(&repo, &["worktree", "add", "--", &destination.to_string_lossy(), &request.branch])?;
            if git_text(&destination, &["rev-parse", "HEAD"])? != oid {
                return Err("Branch changed during preparation; worktree retained for inspection".into());
            }
        } else {
            git(&repo, &["worktree", "add", "-b", &request.branch, "--", &destination.to_string_lossy(), &oid])?;
        }
        if let Some(snapshot) = &dirty {
            if git_text(&repo, &["rev-parse", "HEAD"])? != head
                || &dirty_snapshot(&repo)? != snapshot
            {
                return Err("Source changed during preparation; the new worktree is retained for inspection".into());
            }
            transfer_dirty(&destination, snapshot)?;
        }
        ensure_lfs_materialized(&destination)?;
        let normalized = normalize_existing_workspace_root(&checkout_root).map_err(io_error)?;
        let record = ManagedWorktree {
            checkout_id: CheckoutId::from_normalized_root(&normalized).to_string(),
            project_id: identity.project_id.to_string(),
            root: normalized.path().to_string_lossy().into(),
            repo_root: destination.to_string_lossy().into(),
            project_relative_path: relative,
            branch: Some(format!("refs/heads/{}", request.branch)),
            head_oid: oid,
            materialization_epoch: 1,
            managed: true,
            lifecycle: "active".into(),
            dirty: !git(
                &destination,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )?
            .is_empty(),
            pool_slot: false,
            assignment_id: None,
            editor_version: editor_version(&checkout_root),
            last_error: None,
        };
        store
            .state
            .records
            .insert(record.checkout_id.clone(), record.clone());
        if let Some(operation) = store
            .state
            .operations
            .iter_mut()
            .find(|o| o.id == operation_id)
        {
            operation.checkout_id = record.checkout_id.clone();
        }
        store.finish(&operation_id, None)?;
        Ok(record)
    })();
    if let Err(error) = &result {
        store.finish(&operation_id, Some(error.clone()))?;
    }
    result
}

pub(crate) fn ensure_git_idle(repo: &Path) -> Result<(), String> {
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
        "index.lock",
    ] {
        let path = git_text(repo, &["rev-parse", "--git-path", marker])?;
        let path = Path::new(&path);
        if (if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo.join(path)
        })
        .exists()
        {
            return Err(format!(
                "Checkout has an unfinished Git operation ({marker})"
            ));
        }
    }
    Ok(())
}

pub(crate) fn ensure_recyclable(record: &ManagedWorktree) -> Result<(), String> {
    let repo = canonical(Path::new(&record.repo_root))?;
    let root = canonical(Path::new(&record.root))?;
    if !existing_path_contains(&repo, &root)? || !repo.join(".git").is_file() {
        return Err("Only verified linked worktrees may be recycled".into());
    }
    ensure_git_idle(&repo)?;
    if !git(
        &repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err("Checkout has staged, unstaged or untracked source changes".into());
    }
    // Fail closed even for a stale marker. Callers can close/verify Unity and
    // explicitly remove stale process artifacts before requesting retirement.
    for path in ["Temp/UnityLockfile", "Library/EditorInstance.json"] {
        if root.join(path).exists() {
            return Err(format!("Unity may still own this project ({path})"));
        }
    }
    #[cfg(windows)]
    {
        let process =
            crate::unity_bridge::query_current_project_editor_process_uncached(record.root.clone());
        if process.state != crate::unity_bridge::UnityEditorProcessState::NotRunning {
            return Err(format!("Unity process must be confirmed stopped before recycling (state={:?}, detail={:?})", process.state, process.last_error));
        }
    }
    Ok(())
}

pub fn import(source_root: &Path, target_root: &Path) -> Result<ManagedWorktree, String> {
    let source_id = ProjectIdResolver::resolve(source_root).map_err(io_error)?;
    let root = canonical(target_root)?;
    let mut store = WorktreeStore::open(source_root)?;
    if resolve_git_common_dir(&root).as_ref() != Some(&store.common_dir) {
        return Err("Worktree belongs to another repository".into());
    }
    let repo = canonical(Path::new(&git_text(
        &root,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let relative = existing_path_relative(&repo, &root)?
        .ok_or("Imported Unity project is outside its repository")?
        .to_string_lossy()
        .replace('\\', "/");
    let source_repo = canonical(Path::new(&git_text(
        source_root,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let source_relative = existing_path_relative(&source_repo, source_root)?
        .ok_or("Source Unity project is outside its repository")?;
    if Path::new(&relative) != source_relative {
        return Err("Worktree Unity project subdirectory differs from source".into());
    }
    let normalized = normalize_existing_workspace_root(&root).map_err(io_error)?;
    let checkout_id = CheckoutId::from_normalized_root(&normalized).to_string();
    if let Some(record) = store.state.records.get(&checkout_id) {
        if record.project_id != source_id.project_id.as_str() {
            return Err("Worktree is already assigned to a different logical project".into());
        }
        return Ok(record.clone());
    }
    let record = ManagedWorktree {
        checkout_id,
        project_id: source_id.project_id.to_string(),
        root: root.to_string_lossy().into(),
        repo_root: repo.to_string_lossy().into(),
        project_relative_path: relative,
        branch: git_text(&repo, &["symbolic-ref", "-q", "HEAD"]).ok(),
        head_oid: git_text(&repo, &["rev-parse", "HEAD"])?,
        materialization_epoch: 1,
        managed: false,
        lifecycle: "active".into(),
        dirty: !git(
            &repo,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?
        .is_empty(),
        pool_slot: false,
        assignment_id: None,
        editor_version: editor_version(&root),
        last_error: None,
    };
    store
        .state
        .records
        .insert(record.checkout_id.clone(), record.clone());
    store.save()?;
    Ok(record)
}

pub fn list(source_root: &Path) -> Result<Vec<ManagedWorktree>, String> {
    let (project_id, relative) = source_ownership(source_root)?;
    let mut store = WorktreeStore::open(source_root)?;
    let mut records = Vec::new();
    for record in store.state.records.values_mut() {
        if record.project_id != project_id || record.project_relative_path != relative {
            continue;
        }
        // A reservation is an immutable recovery request. Listing must not
        // replace its intended commit/branch with a partially created directory
        // or reinterpret an absent destination as a lost live checkout.
        if record.checkout_id.starts_with("reservation-") {
            records.push(record.clone());
            continue;
        }
        let root = Path::new(&record.repo_root);
        if !root.exists() {
            if record.lifecycle != "removed" {
                record.lifecycle = "missing".into();
            }
        } else if let Ok(head) = git_text(root, &["rev-parse", "HEAD"]) {
            record.head_oid = head;
            record.branch = git_text(root, &["symbolic-ref", "-q", "HEAD"]).ok();
            record.dirty = !git(
                root,
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )?
            .is_empty();
        }
        records.push(record.clone());
    }
    // A crash never silently adopts or deletes partially materialized data.
    // Pending operations are exposed with a recovery error for explicit retry.
    for op in &mut store.state.operations {
        if op.state == "pending" {
            op.state = "recovery_required".into();
            op.error = Some(
                "Interrupted worktree operation; retained filesystem state requires inspection"
                    .into(),
            );
        }
    }
    store.save()?;
    Ok(records)
}

pub(crate) fn source_ownership(source_root: &Path) -> Result<(String, String), String> {
    let source = canonical(source_root)?;
    let identity = ProjectIdResolver::resolve(&source).map_err(io_error)?;
    let repo = canonical(Path::new(&git_text(
        &source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let relative = existing_path_relative(&repo, &source)?
        .ok_or("Source Unity project is outside its repository")?
        .to_string_lossy()
        .replace('\\', "/");
    Ok((identity.project_id.to_string(), relative))
}

pub fn operations(source_root: &Path) -> Result<Vec<WorktreeOperation>, String> {
    Ok(WorktreeStore::open(source_root)?.state.operations)
}

pub fn discover(source_root: &Path) -> Result<Vec<String>, String> {
    let bytes = git(source_root, &["worktree", "list", "--porcelain", "-z"])?;
    let relative = {
        let repo = canonical(Path::new(&git_text(
            source_root,
            &["rev-parse", "--show-toplevel"],
        )?))?;
        existing_path_relative(&repo, source_root)?
            .ok_or("Source Unity project is outside its repository")?
    };
    bytes
        .split(|b| *b == 0)
        .filter_map(|part| part.strip_prefix(b"worktree "))
        .map(|part| {
            String::from_utf8(part.to_vec())
                .map(|p| Path::new(&p).join(&relative).to_string_lossy().into())
                .map_err(io_error)
        })
        .collect()
}

pub fn remove(source_root: &Path, checkout_id: &str, expected_epoch: u64) -> Result<(), String> {
    let (project_id, relative) = source_ownership(source_root)?;
    let mut store = WorktreeStore::open(source_root)?;
    let record = store
        .state
        .records
        .get(checkout_id)
        .cloned()
        .ok_or("Unknown managed worktree")?;
    if record.project_id != project_id || record.project_relative_path != relative {
        return Err("Worktree belongs to another logical project within this repository".into());
    }
    if !record.managed {
        return Err(
            "External worktrees retain user ownership and cannot be deleted by Locus".into(),
        );
    }
    if record.materialization_epoch != expected_epoch {
        return Err("Stale checkout materialization epoch".into());
    }
    if record.assignment_id.is_some() {
        return Err("Release the pool assignment before deleting its directory".into());
    }
    let _lease = mutation_lease(&store.common_dir, checkout_id)?;
    ensure_recyclable(&record)?;
    if resolve_git_common_dir(Path::new(&record.root)).as_ref() != Some(&store.common_dir) {
        return Err("Worktree repository identity changed".into());
    }
    let op = store.begin(&record, "remove")?;
    // No --force: Git supplies a second independent clean/lock check.
    let result = git(
        source_root,
        &["worktree", "remove", "--", &record.repo_root],
    );
    match result {
        Ok(_) => {
            store.state.records.get_mut(checkout_id).unwrap().lifecycle = "removed".into();
            store.finish(&op, None)
        }
        Err(error) => {
            store.finish(&op, Some(error.clone()))?;
            Err(error)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn fixture() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("source");
        std::fs::create_dir_all(repo.join("ProjectSettings")).unwrap();
        std::fs::create_dir_all(repo.join("Assets")).unwrap();
        std::fs::write(
            repo.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 6000.5.8f1\n",
        )
        .unwrap();
        std::fs::write(repo.join("Assets/test.asset"), "base\n").unwrap();
        std::fs::write(repo.join(".gitignore"), "Library/\nTemp/\nLogs/\n").unwrap();
        git(&repo, &["init"]).unwrap();
        git(&repo, &["config", "user.email", "worktree-test@localhost"]).unwrap();
        git(&repo, &["config", "user.name", "Worktree tests"]).unwrap();
        git(&repo, &["config", "core.autocrlf", "false"]).unwrap();
        git(&repo, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-m", "initial"]).unwrap();
        (temp, repo)
    }

    #[test]
    #[cfg(windows)]
    fn git_long_root_failure_retains_the_requested_root_and_operation() {
        let (_temp, repo) = fixture();
        let mut project = repo.join("Projects");
        while project.to_string_lossy().len() < 310 {
            project.push("long-project-component-123456789");
        }
        std::fs::create_dir_all(&project).unwrap();
        let verbatim = std::fs::canonicalize(&project).unwrap();
        let config_before = std::fs::read(repo.join(".git/config")).unwrap();
        let discovered = git_text(&repo, &["rev-parse", "--show-toplevel"]).unwrap();
        assert!(path_components_equal(
            &std::fs::canonicalize(discovered).unwrap(),
            &std::fs::canonicalize(&repo).unwrap()
        )
        .unwrap());
        assert!(!git(&repo, &["worktree", "list", "--porcelain", "-z"])
            .unwrap()
            .is_empty());
        match git_text(&verbatim, &["rev-parse", "--show-prefix"]) {
            Ok(prefix) => {
                let relative = existing_path_relative(&repo, &project).unwrap().unwrap();
                assert_eq!(prefix, format!("{}/", relative.to_string_lossy().replace('\\', "/")));
            }
            Err(error) => {
                assert!(error.contains("git rev-parse failed for project root"), "{error}");
                assert!(error.contains(&verbatim.to_string_lossy().to_string()), "{error}");
                assert!(error.contains("Filename too long"), "{error}");
            }
        }
        assert_eq!(
            std::fs::read(repo.join(".git/config")).unwrap(),
            config_before
        );
    }

    #[test]
    fn dirty_snapshot_accepts_long_tracked_and_untracked_children() {
        let (_temp, repo) = fixture();
        let mut directory = repo.join("Assets");
        while directory.to_string_lossy().len() < 300 {
            directory.push("long-dirty-component-123456789");
        }
        std::fs::create_dir_all(&directory).unwrap();
        let tracked = directory.join("tracked.asset");
        let tracked_relative = path_relative(&repo, &tracked)
            .unwrap()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        std::fs::write(&tracked, b"base\n").unwrap();
        git(&repo, &["add", "--", &tracked_relative]).unwrap();
        git(&repo, &["commit", "-m", "Long dirty source"]).unwrap();
        let index = std::fs::read(repo.join(".git/index")).unwrap();
        std::fs::write(&tracked, b"working edit\n").unwrap();
        let untracked = directory.join("untracked.asset");
        std::fs::write(&untracked, b"new source\n").unwrap();
        let snapshot = dirty_snapshot(&canonical(&repo).unwrap()).unwrap();
        assert_eq!(
            snapshot.files[&tracked_relative],
            Some(b"working edit\n".to_vec())
        );
        let untracked_relative = path_relative(&repo, &untracked)
            .unwrap()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(
            snapshot.files[&untracked_relative],
            Some(b"new source\n".to_vec())
        );
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), index);
    }

    #[test]
    fn long_monorepo_project_respects_pending_and_reservation_runtime_gates() {
        let (_temp, repo) = fixture();
        let mut project = repo.join("Projects");
        while project.to_string_lossy().len() < 300 {
            project.push("long-unity-project-123456789");
        }
        std::fs::create_dir_all(project.join("ProjectSettings")).unwrap();
        std::fs::write(
            project.join("ProjectSettings/ProjectVersion.txt"),
            b"m_EditorVersion: 6000.5.8f1\n",
        )
        .unwrap();
        let expected_relative = path_relative(&repo, &project)
            .unwrap()
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // Git's -C boundary is independent from runtime filesystem gating.
        // Obtain repository identity at its supported root, then exercise the
        // real long project root against both journal guards below.
        let (project_id, _) = source_ownership(&repo).unwrap();
        let mut store = WorktreeStore::open(&repo).unwrap();
        let reservation = ManagedWorktree {
            checkout_id: "reservation-long-project".into(),
            project_id,
            root: project.to_string_lossy().into(),
            repo_root: repo.to_string_lossy().into(),
            project_relative_path: expected_relative,
            branch: None,
            head_oid: git_text(&repo, &["rev-parse", "HEAD"]).unwrap(),
            materialization_epoch: 0,
            managed: true,
            lifecycle: "preparing".into(),
            dirty: false,
            pool_slot: true,
            assignment_id: Some("boundary-test".into()),
            editor_version: Some("6000.5.8f1".into()),
            last_error: None,
        };
        store
            .state
            .records
            .insert(reservation.checkout_id.clone(), reservation);
        store.save().unwrap();
        drop(store);
        assert!(runtime_lease(&project)
            .unwrap_err()
            .contains("incomplete pool reservation"));
        let mut store = WorktreeStore::open(&repo).unwrap();
        store.state.records.clear();
        store.state.operations.push(WorktreeOperation {
            id: "pending-long-project".into(),
            kind: "create".into(),
            checkout_id: String::new(),
            destination: repo.to_string_lossy().into(),
            expected_head: git_text(&repo, &["rev-parse", "HEAD"]).unwrap(),
            state: "pending".into(),
            error: None,
        });
        store.save().unwrap();
        drop(store);
        assert!(runtime_lease(&project)
            .unwrap_err()
            .contains("incomplete materialization"));
        let mut store = WorktreeStore::open(&repo).unwrap();
        store.state.operations[0].state = "recovery_required".into();
        store.save().unwrap();
        drop(store);
        assert!(runtime_lease(&project)
            .unwrap_err()
            .contains("incomplete materialization"));
        // A similarly named sibling reservation is not this project, including
        // when that future sibling checkout does not exist yet.
        let mut store = WorktreeStore::open(&repo).unwrap();
        store.state.operations[0].destination = repo
            .with_file_name("source-sibling")
            .to_string_lossy()
            .into();
        store.save().unwrap();
        drop(store);
        assert!(runtime_lease(&project).unwrap().is_none());
    }

    #[test]
    fn prospective_long_destination_inside_source_is_rejected_but_sibling_is_allowed() {
        let (temp, repo) = fixture();
        let mut parent = repo.join("nested");
        while parent.to_string_lossy().len() < 300 {
            parent.push("long-parent-component-123456789");
        }
        std::fs::create_dir_all(&parent).unwrap();
        assert!(
            prospective_destination(&parent.join("future-checkout"), &repo)
                .unwrap_err()
                .contains("outside")
        );
        let sibling = temp.path().join("source-sibling");
        std::fs::create_dir(&sibling).unwrap();
        assert!(prospective_destination(&sibling.join("future-checkout"), &repo).is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn managed_worktree_handles_long_paths_without_changing_git_configuration() {
        let (temp, repo) = fixture();
        let relative = format!("Assets/{}/{}/long.asset", "a".repeat(70), "b".repeat(70));
        let file = repo.join(&relative);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"long tracked asset\n").unwrap();
        git(&repo, &["add", "--", &relative]).unwrap();
        git(&repo, &["commit", "-m", "long asset path"]).unwrap();
        let config_before = std::fs::read(repo.join(".git/config")).unwrap();
        let parent = temp.path().join("destination-container".repeat(4));
        std::fs::create_dir(&parent).unwrap();
        let destination = parent.join("checkout");
        assert!(destination.join(&relative).to_string_lossy().len() > 260);
        let record = create(&CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: destination.to_string_lossy().into(),
            branch: "codex/long-destination".into(),
            start_ref: None,
            include_dirty: false,
        })
        .unwrap();
        assert_eq!(
            std::fs::read(Path::new(&record.root).join(relative)).unwrap(),
            b"long tracked asset\n"
        );
        assert_eq!(
            std::fs::read(repo.join(".git/config")).unwrap(),
            config_before
        );
        assert!(!record.dirty);
    }

    #[test]
    fn explicit_missing_destination_parent_reports_the_path() {
        let (temp, repo) = fixture();
        let parent = temp.path().join("missing").join("parent");
        let error = prospective_destination(&parent.join("checkout"), &repo).unwrap_err();
        assert!(
            error.contains(&parent.to_string_lossy().to_string()),
            "{error}"
        );
        assert!(!parent.exists());
    }

    #[test]
    fn dirty_worktree_preserves_source_index_and_groups_project() {
        let (temp, repo) = fixture();
        std::fs::write(repo.join("Assets/test.asset"), "staged\n").unwrap();
        git(&repo, &["add", "Assets/test.asset"]).unwrap();
        std::fs::write(repo.join("Assets/test.asset"), "working\n").unwrap();
        std::fs::write(repo.join("Assets/new.asset"), "untracked\n").unwrap();
        std::fs::create_dir_all(repo.join("Library")).unwrap();
        std::fs::write(repo.join("Library/private.bin"), [1, 2, 3]).unwrap();
        let index_before = std::fs::read(repo.join(".git/index")).unwrap();
        let record = create(&CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: temp.path().join("target").to_string_lossy().into(),
            branch: "feature/parallel".into(),
            start_ref: None,
            include_dirty: true,
        })
        .unwrap();
        assert_eq!(
            std::fs::read(repo.join(".git/index")).unwrap(),
            index_before
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&record.root).join("Assets/test.asset")).unwrap(),
            "working\n"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&record.root).join("Assets/new.asset")).unwrap(),
            "untracked\n"
        );
        assert!(!Path::new(&record.root).join("Library").exists());
        assert!(git(
            Path::new(&record.root),
            &["diff", "--cached", "--name-only"]
        )
        .unwrap()
        .is_empty());
        assert_eq!(
            ProjectIdResolver::resolve(&record.root).unwrap().project_id,
            ProjectIdResolver::resolve(&repo).unwrap().project_id
        );
        assert!(record.dirty);
        assert!(remove(&repo, &record.checkout_id, 1)
            .unwrap_err()
            .contains("changes"));
    }

    #[test]
    fn lifecycle_protects_leases_dirty_and_external_worktrees() {
        let (temp, repo) = fixture();
        let record = create(&CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: temp.path().join("target").to_string_lossy().into(),
            branch: "feature/clean".into(),
            start_ref: None,
            include_dirty: false,
        })
        .unwrap();
        let lease = runtime_lease(Path::new(&record.root)).unwrap();
        assert!(remove(&repo, &record.checkout_id, 1)
            .unwrap_err()
            .contains("registered"));
        drop(lease);
        assert!(remove(&repo, &record.checkout_id, 2)
            .unwrap_err()
            .contains("Stale"));
        remove(&repo, &record.checkout_id, 1).unwrap();
        assert!(!Path::new(&record.root).exists());
        assert!(git(&repo, &["show-ref", "--verify", "refs/heads/feature/clean"]).is_ok());
        let external = temp.path().join("external");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feature/external",
                &external.to_string_lossy(),
            ],
        )
        .unwrap();
        let imported = import(&repo, &external).unwrap();
        assert!(!imported.managed);
        assert!(remove(&repo, &imported.checkout_id, 1)
            .unwrap_err()
            .contains("ownership"));
    }

    #[test]
    fn nested_project_keeps_relative_path_and_rejects_escape() {
        let (temp, repo) = fixture();
        let nested = repo.join("Game");
        std::fs::create_dir_all(nested.join("ProjectSettings")).unwrap();
        std::fs::write(
            nested.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 6000.5.8f1\n",
        )
        .unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-m", "nested project"]).unwrap();
        let record = create(&CreateWorktreeRequest {
            source_root: nested.to_string_lossy().into(),
            destination: temp.path().join("nested-target").to_string_lossy().into(),
            branch: "feature/nested".into(),
            start_ref: None,
            include_dirty: false,
        })
        .unwrap();
        assert_eq!(record.project_relative_path, "Game");
        assert!(Path::new(&record.root).ends_with("nested-target/Game"));
        for path in ["../escape", ".git/config", "Assets/../../outside"] {
            assert!(safe_relative(path).is_err());
        }
        assert!(!list(&repo)
            .unwrap()
            .iter()
            .any(|r| r.checkout_id == record.checkout_id));
        assert!(remove(&repo, &record.checkout_id, 1)
            .unwrap_err()
            .contains("another logical project"));
    }

    #[test]
    fn duplicate_project_guid_does_not_authorize_foreign_repository_import() {
        let (_left_temp, left) = fixture();
        let (_right_temp, right) = fixture();
        for repo in [&left, &right] {
            std::fs::create_dir_all(repo.join("Locus")).unwrap();
            std::fs::write(
                repo.join("Locus/config.json"),
                r#"{"workspace_id":"copied-project-id"}"#,
            )
            .unwrap();
            git(repo, &["add", "."]).unwrap();
            git(repo, &["commit", "-m", "same copied identity"]).unwrap();
        }
        assert_eq!(
            ProjectIdResolver::resolve(&left).unwrap().project_id,
            ProjectIdResolver::resolve(&right).unwrap().project_id
        );
        assert!(import(&left, &right)
            .unwrap_err()
            .contains("another repository"));
    }

    #[test]
    fn checkout_success_does_not_accept_an_unhydrated_lfs_pointer() {
        let (temp, repo) = fixture();
        std::fs::write(
            repo.join("Assets/model.fbx"),
            format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 1234\n",
                "a".repeat(64)
            ),
        )
        .unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(
            &repo,
            &["commit", "-m", "missing LFS object without filter"],
        )
        .unwrap();
        let result = create(&CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: temp.path().join("missing-lfs").to_string_lossy().into(),
            branch: "feature/missing-lfs".into(),
            start_ref: None,
            include_dirty: false,
        });
        assert!(result.unwrap_err().contains("unresolved Git LFS pointers"));
        assert!(list(&repo).unwrap().is_empty());
    }
}
