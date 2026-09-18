//! Fixed-path Unity project pool. Only clean, closed managed checkouts are
//! reassigned. Library remains private to the physical slot and is reused only
//! across an exact Editor version match. No hard links and no live cache copy.
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::worktrees::{self, ManagedWorktree, WorktreeStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquirePoolRequest {
    pub source_root: String,
    pub pool_root: String,
    pub commit: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// Upper bound on physical slots, supplied from the user's concurrency /
    /// storage settings. Existing assignments survive lowering this limit.
    pub max_slots: usize,
    #[serde(default)]
    pub assignment_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolAcquisition {
    pub worktree: ManagedWorktree,
    pub reused: bool,
    pub preserved_library: bool,
}

fn version_at(repo: &Path, oid: &str, relative: &str) -> Result<String, String> {
    let path = if relative.is_empty() {
        "ProjectSettings/ProjectVersion.txt".into()
    } else {
        format!("{relative}/ProjectSettings/ProjectVersion.txt")
    };
    let bytes = worktrees::git(repo, &["show", &format!("{oid}:{path}")])?;
    String::from_utf8(bytes)
        .map_err(|e| e.to_string())?
        .lines()
        .find_map(|line| {
            line.strip_prefix("m_EditorVersion:")
                .map(|v| v.trim().to_string())
        })
        .filter(|v| !v.is_empty())
        .ok_or("Target commit has no Unity Editor version".into())
}

fn invalidate_locus_projections(root: &Path) -> Result<(), String> {
    let root = worktrees::canonical(root)?;
    let library = root.join("Library/Locus");
    for name in [
        "locus.db",
        "locus.db-wal",
        "locus.db-shm",
        "knowledge_index.db",
        "knowledge_index.db-wal",
        "knowledge_index.db-shm",
        "knowledge_tantivy_index",
    ] {
        let path = library.join(name);
        if !path.exists() {
            continue;
        }
        let canonical = worktrees::canonical(&path)?;
        if !worktrees::existing_path_contains(&root, &canonical)?
            || std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
        {
            return Err("Derived Locus cache escapes its managed project".into());
        }
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        }
        .map_err(|e| {
            format!(
                "Could not retire derived Locus cache {}: {e}",
                path.display()
            )
        })?;
    }
    crate::workspace_changes::hub_for_workspace(&root)
        .mark_rescan_required("pool_materialization_changed");
    Ok(())
}

/// Git status omits ignored files. An old ignored script or setting must never
/// survive into another assignment, even when neither commit tracks its path.
/// Library and other derived directories deliberately remain outside this set.
fn ensure_source_files_isolated(record: &ManagedWorktree) -> Result<(), String> {
    use std::collections::BTreeSet;
    let repo = std::fs::canonicalize(&record.repo_root).map_err(|e| e.to_string())?;
    let relative = if record.project_relative_path.is_empty() {
        std::path::PathBuf::new()
    } else {
        worktrees::safe_relative(&record.project_relative_path)?
    };
    let project = std::fs::canonicalize(&record.root).map_err(|e| e.to_string())?;
    if !worktrees::path_components_equal(&repo.join(relative), &project)? {
        return Err("Pool source project resolves outside its recorded directory".into());
    }
    let roots = ["Assets", "Packages", "ProjectSettings"].map(|name| project.join(name));
    let in_source = |path: &Path| -> Result<bool, String> {
        for root in &roots {
            if worktrees::path_relative(root, path)?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let mut tracked = BTreeSet::new();
    let index = worktrees::git(&repo, &["ls-files", "--stage", "--full-name", "-z"])?;
    for entry in index.split(|byte| *byte == 0).filter(|entry| !entry.is_empty()) {
        let entry = std::str::from_utf8(entry).map_err(|e| e.to_string())?;
        let (header, name) = entry.split_once('\t').ok_or("Invalid pool index entry")?;
        let path = repo.join(worktrees::safe_relative(name)?);
        if !in_source(&path)? {
            continue;
        }
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") || fields[2] != "0" {
            return Err(format!("Pool source must be a tracked ordinary file: {name}"));
        }
        let physical = std::fs::canonicalize(&path)
            .map_err(|e| format!("Pool tracked source is missing or inaccessible: {name}: {e}"))?;
        if !worktrees::path_components_equal(&path, &physical)? {
            return Err(format!("Pool source resolves through an alias: {name}"));
        }
        tracked.insert(physical);
    }
    let mut observed = BTreeSet::new();
    let mut pending = Vec::from(roots);
    while let Some(path) = pending.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Cannot inspect pool source {}: {error}", path.display())),
        };
        let mut alias = metadata.file_type().is_symlink();
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            alias |= metadata.file_attributes() & 0x400 != 0;
        }
        if alias {
            return Err(format!("Pool source cannot contain a symbolic link or reparse point: {}", path.display()));
        }
        let physical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !worktrees::path_components_equal(&path, &physical)?
            || worktrees::path_relative(&project, &physical)?.is_none()
        {
            return Err(format!("Pool source escapes its recorded directory: {}", path.display()));
        }
        if metadata.is_dir() {
            for child in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(child.map_err(|e| e.to_string())?.path());
            }
        } else if metadata.is_file() {
            if !tracked.contains(&physical) {
                return Err(format!("Pool source contains an untracked or ignored file; preserve or remove it explicitly before reuse: {}", path.display()));
            }
            observed.insert(physical);
        } else {
            return Err(format!("Pool source is not an ordinary file: {}", path.display()));
        }
    }
    if let Some(missing) = tracked.difference(&observed).next() {
        return Err(format!("Pool tracked source is missing: {}", missing.display()));
    }
    Ok(())
}

/// Read-only availability probe for the settings view and allocation preview.
/// Acquisition repeats these checks while retaining the mutation lease.
pub fn can_reuse(record: &ManagedWorktree) -> bool {
    if !record.pool_slot || !record.managed || record.lifecycle != "available"
        || record.assignment_id.is_some() || record.dirty { return false; }
    let Some(common) = super::identity::resolve_git_common_dir(Path::new(&record.root)) else { return false; };
    let Ok(_lease) = worktrees::mutation_lease(&common, &record.checkout_id) else { return false; };
    worktrees::ensure_recyclable(record).is_ok() && ensure_source_files_isolated(record).is_ok()
}

pub fn acquire(request: &AcquirePoolRequest) -> Result<PoolAcquisition, String> {
    acquire_with_creation_policy(request, true)
}

pub const NEW_PROJECT_REQUIRED: &str = "WORKTREE_NEW_PROJECT_REQUIRED: No reusable project is available; creating a local project requires confirmation";

pub fn acquire_with_creation_policy(request: &AcquirePoolRequest, allow_new_project: bool) -> Result<PoolAcquisition, String> {
    if request.max_slots == 0 {
        return Err("The project pool is disabled (maxSlots is zero)".into());
    }
    if request.commit.starts_with('-') {
        return Err("Invalid commit revision".into());
    }
    let source = worktrees::canonical(Path::new(&request.source_root))?;
    let source_identity =
        super::identity::ProjectIdResolver::resolve(&source).map_err(|e| e.to_string())?;
    let repo = worktrees::canonical(Path::new(&worktrees::git_text(
        &source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let relative = worktrees::existing_path_relative(&repo, &source)?
        .ok_or("Source Unity project is outside its repository")?
        .to_string_lossy()
        .replace('\\', "/");
    let pool_root = worktrees::canonical(Path::new(&request.pool_root))?;
    if worktrees::existing_path_contains(&repo, &pool_root)?
        || worktrees::existing_path_contains(&pool_root, &repo)?
    {
        return Err("Pool root must be separate from the source checkout".into());
    }
    let oid = worktrees::git_text(
        &repo,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", request.commit),
        ],
    )?;
    worktrees::ensure_tree_supported(&repo, &oid, &relative)?;
    let version = version_at(&repo, &oid, &relative)?;
    if !version.starts_with("6000.5.") {
        return Err(format!(
            "The first pool release supports Unity 6.5 (6000.5.x), target declares {version}"
        ));
    }
    if let Some(branch) = &request.branch {
        worktrees::git(&repo, &["check-ref-format", "--branch", branch])?;
    }
    let assignment_id = request
        .assignment_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if assignment_id.is_empty()
        || !assignment_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Assignment id may contain only letters, digits, hyphens and underscores".into(),
        );
    }
    let mut store = WorktreeStore::open(&source)?;
    if let Some(existing) = store
        .state
        .records
        .values()
        .find(|r| r.assignment_id.as_deref() == Some(&assignment_id))
        .cloned()
    {
        if existing.head_oid != oid || existing.project_id != source_identity.project_id.as_str() {
            return Err("Assignment id was already used with a different commit or project".into());
        }
        if existing.project_relative_path != relative
            || Path::new(&existing.repo_root)
                .parent()
                .is_none_or(|parent| worktrees::path_key(parent) != worktrees::path_key(&pool_root))
        {
            return Err(
                "Assignment id was already used with a different pool or project subdirectory"
                    .into(),
            );
        }
        if request.branch.as_ref().is_some_and(|branch| {
            existing.branch.as_deref() != Some(&format!("refs/heads/{branch}"))
        }) {
            return Err("Assignment id was already used with a different branch".into());
        }
        if existing.checkout_id.starts_with("reservation-") {
            let reservation = existing.checkout_id.clone();
            drop(store);
            return complete_reservation(&source, &reservation, &assignment_id);
        }
        if existing.lifecycle != "active" {
            return Err(format!(
                "Pool assignment is {}; inspect its operation journal before retrying",
                existing.lifecycle
            ));
        }
        return Ok(PoolAcquisition {
            worktree: existing.clone(),
            reused: true,
            preserved_library: Path::new(&existing.root).join("Library").is_dir(),
        });
    }
    if let Some(branch) = &request.branch {
        if branch.starts_with(['-', '@'])
            || worktrees::git(
                &repo,
                &["show-ref", "--verify", &format!("refs/heads/{branch}")],
            )
            .is_ok()
        {
            return Err(
                "Pool assignments require a literal new branch name or detached HEAD".into(),
            );
        }
    }
    if worktrees::git(
        &repo,
        &[
            "show-ref",
            "--verify",
            &format!("refs/locus/pool-history/{assignment_id}"),
        ],
    )
    .is_ok()
    {
        return Err("This pool assignment was already released; use a new assignment id".into());
    }
    let candidates: Vec<_> = store
        .state
        .records
        .values()
        .filter(|record| {
            record.pool_slot
                && record.managed
                && record.lifecycle == "available"
                && record.assignment_id.is_none()
                && record.project_id == source_identity.project_id.as_str()
                && record.project_relative_path == relative
                && record.editor_version.as_deref() == Some(&version)
                && Path::new(&record.repo_root).parent().is_some_and(|parent| {
                    worktrees::path_key(parent) == worktrees::path_key(&pool_root)
                })
        })
        .cloned()
        .collect();
    let mut rejected_sources = Vec::new();
    for mut record in candidates {
        let Ok(_lease) = worktrees::mutation_lease(&store.common_dir, &record.checkout_id) else {
            continue;
        };
        if worktrees::ensure_recyclable(&record).is_err() {
            continue;
        }
        if let Err(error) = ensure_source_files_isolated(&record) {
            record.lifecycle = "quarantined".into();
            record.last_error = Some(error.clone());
            store.state.records.insert(record.checkout_id.clone(), record);
            store.save()?;
            rejected_sources.push(error);
            continue;
        }
        let slot_path = std::path::PathBuf::from(&record.repo_root);
        let slot = slot_path.as_path();
        if super::identity::resolve_git_common_dir(slot).as_ref() != Some(&store.common_dir) {
            continue;
        }
        let preserved_library = Path::new(&record.root).join("Library").is_dir();
        // Never let checkout overwrite an ignored cache that happens to become
        // tracked in the new revision.
        let op = store.begin(&record, "pool_acquire")?;
        record.lifecycle = "preparing".into();
        store
            .state
            .records
            .insert(record.checkout_id.clone(), record.clone());
        store.save()?;
        let result: Result<PoolAcquisition, String> = (|| {
            worktrees::git(
                slot,
                &["checkout", "--detach", "--no-overwrite-ignore", &oid, "--"],
            )?;
            if let Some(branch) = &request.branch {
                worktrees::git(slot, &["checkout", "-b", branch])?;
            }
            invalidate_locus_projections(Path::new(&record.root))?;
            worktrees::ensure_lfs_materialized(slot)?;
            ensure_source_files_isolated(&record)?;
            record.materialization_epoch = record
                .materialization_epoch
                .checked_add(1)
                .ok_or("Materialization epoch exhausted")?;
            record.head_oid = oid.clone();
            record.branch = request
                .branch
                .as_ref()
                .map(|branch| format!("refs/heads/{branch}"));
            record.assignment_id = Some(assignment_id.clone());
            record.lifecycle = "active".into();
            record.dirty = false;
            record.last_error = None;
            store
                .state
                .records
                .insert(record.checkout_id.clone(), record.clone());
            store.finish(&op, None)?;
            Ok(PoolAcquisition {
                worktree: record.clone(),
                reused: true,
                preserved_library,
            })
        })();
        if let Err(error) = &result {
            record.lifecycle = "quarantined".into();
            record.last_error = Some(error.clone());
            store
                .state
                .records
                .insert(record.checkout_id.clone(), record);
            store.finish(&op, Some(error.clone()))?;
        }
        return result;
    }
    if !allow_new_project {
        return Err(NEW_PROJECT_REQUIRED.into());
    }
    let occupied = store
        .state
        .records
        .values()
        .filter(|r| {
            r.pool_slot
                && r.lifecycle != "removed"
                && Path::new(&r.repo_root)
                    .parent()
                    .is_some_and(|p| worktrees::path_key(p) == worktrees::path_key(&pool_root))
        })
        .count();
    if occupied >= request.max_slots {
        let detail = if rejected_sources.is_empty() { String::new() } else { format!("; {}", rejected_sources.join("; ")) };
        return Err(format!("Project pool is at capacity ({occupied}/{}); wait for a compatible clean slot or raise maxSlots{detail}", request.max_slots));
    }
    // Reserve capacity durably before releasing the registry lock to enter
    // create(); pending reservations count for every concurrent acquire.
    let slot_name = format!("unity-{}", uuid::Uuid::new_v4().simple());
    let destination = pool_root.join(&slot_name);
    let reservation_id = format!("reservation-{slot_name}");
    let branch = request
        .branch
        .clone()
        .unwrap_or_else(|| format!("locus/pool/{slot_name}"));
    let placeholder = ManagedWorktree {
        checkout_id: reservation_id.clone(),
        project_id: source_identity.project_id.to_string(),
        root: destination.join(&relative).to_string_lossy().into(),
        repo_root: destination.to_string_lossy().into(),
        project_relative_path: relative,
        branch: Some(format!("refs/heads/{branch}")),
        head_oid: oid.clone(),
        materialization_epoch: 0,
        managed: true,
        lifecycle: "preparing".into(),
        dirty: false,
        pool_slot: true,
        assignment_id: Some(assignment_id.clone()),
        editor_version: Some(version.clone()),
        last_error: None,
    };
    store
        .state
        .records
        .insert(reservation_id.clone(), placeholder);
    store.save()?;
    drop(store);
    complete_reservation(&source, &reservation_id, &assignment_id)
}

/// Resume a dead preparation from immutable reservation data. The separate
/// OS lease spans the interval where create() owns the repository journal, so
/// a concurrent retry cannot mistake a live create for crash recovery.
fn complete_reservation(
    source: &Path,
    reservation_id: &str,
    assignment_id: &str,
) -> Result<PoolAcquisition, String> {
    let mut store = WorktreeStore::open(source)?;
    let Some(reservation) = store.state.records.get(reservation_id).cloned() else {
        let completed = store
            .state
            .records
            .values()
            .find(|record| {
                record.assignment_id.as_deref() == Some(assignment_id)
                    && record.lifecycle == "active"
            })
            .cloned()
            .ok_or("Pool reservation no longer exists")?;
        return Ok(PoolAcquisition {
            preserved_library: Path::new(&completed.root).join("Library").is_dir(),
            worktree: completed,
            reused: true,
        });
    };
    let _reservation_lease = worktrees::mutation_lease(&store.common_dir, reservation_id)?;
    let branch = reservation
        .branch
        .as_deref()
        .and_then(|value| value.strip_prefix("refs/heads/"))
        .ok_or("Reservation is missing its immutable branch name")?
        .to_string();
    let destination = Path::new(&reservation.repo_root);
    let recovered = destination.exists();
    let created: Result<ManagedWorktree, String> = if recovered {
        (|| {
            if super::identity::resolve_git_common_dir(destination).as_ref()
                != Some(&store.common_dir)
            {
                return Err("Interrupted pool directory belongs to another repository".into());
            }
            worktrees::ensure_recyclable(&reservation)?;
            ensure_source_files_isolated(&reservation)?;
            worktrees::ensure_lfs_materialized(destination)?;
            if worktrees::git_text(destination, &["rev-parse", "HEAD"])? != reservation.head_oid
                || worktrees::git_text(destination, &["symbolic-ref", "-q", "HEAD"])?
                    != format!("refs/heads/{branch}")
                || worktrees::editor_version(Path::new(&reservation.root))
                    != reservation.editor_version
            {
                return Err("Interrupted pool directory no longer matches its reserved commit, branch or Editor version".into());
            }
            let normalized = super::identity::normalize_existing_workspace_root(&reservation.root)
                .map_err(|e| e.to_string())?;
            let id = super::identity::CheckoutId::from_normalized_root(&normalized).to_string();
            let _actual_lease = worktrees::mutation_lease(&store.common_dir, &id)?;
            let mut record = reservation.clone();
            record.checkout_id = id;
            record.materialization_epoch = 1;
            record.lifecycle = "active".into();
            record.last_error = None;
            Ok(record)
        })()
    } else {
        drop(store);
        let result = worktrees::create(&worktrees::CreateWorktreeRequest {
            source_root: source.to_string_lossy().into(),
            destination: reservation.repo_root.clone(),
            branch,
            start_ref: Some(reservation.head_oid.clone()),
            include_dirty: false,
        });
        store = WorktreeStore::open(source)?;
        result
    };
    let created = created.and_then(|mut record| {
        if let Err(error) = ensure_source_files_isolated(&record) {
            record.lifecycle = "quarantined".into();
            record.last_error = Some(error.clone());
            store.state.records.insert(record.checkout_id.clone(), record);
            return Err(error);
        }
        Ok(record)
    });
    match created {
        Ok(mut record) => {
            record.pool_slot = true;
            record.assignment_id = reservation.assignment_id;
            record.editor_version = reservation.editor_version;
            store.state.records.remove(reservation_id);
            store
                .state
                .records
                .insert(record.checkout_id.clone(), record.clone());
            for operation in &mut store.state.operations {
                if operation.kind == "create"
                    && operation.destination == record.repo_root
                    && operation.expected_head == record.head_oid
                {
                    operation.state = "complete".into();
                    operation.error = None;
                    operation.checkout_id = record.checkout_id.clone();
                }
            }
            store.save()?;
            Ok(PoolAcquisition {
                preserved_library: recovered && Path::new(&record.root).join("Library").is_dir(),
                worktree: record,
                reused: recovered,
            })
        }
        Err(error) => {
            if let Some(record) = store.state.records.get_mut(reservation_id) {
                record.lifecycle = "quarantined".into();
                record.last_error = Some(error.clone());
            }
            store.save()?;
            Err(error)
        }
    }
}

pub fn release(
    source_root: &Path,
    checkout_id: &str,
    assignment_id: &str,
    expected_epoch: u64,
) -> Result<ManagedWorktree, String> {
    let (project_id, relative) = worktrees::source_ownership(source_root)?;
    let mut store = WorktreeStore::open(source_root)?;
    let mut record = store
        .state
        .records
        .get(checkout_id)
        .cloned()
        .ok_or("Unknown pool slot")?;
    if record.project_id != project_id
        || record.project_relative_path != relative
        || super::identity::resolve_git_common_dir(Path::new(&record.root)).as_ref()
            != Some(&store.common_dir)
    {
        return Err("Pool slot belongs to another repository or logical project".into());
    }
    if !record.pool_slot || !record.managed {
        return Err("Checkout is not a managed pool slot".into());
    }
    if record.assignment_id.as_deref() != Some(assignment_id)
        || record.materialization_epoch != expected_epoch
    {
        return Err("Pool assignment or materialization epoch is stale".into());
    }
    let _lease = worktrees::mutation_lease(&store.common_dir, checkout_id)?;
    worktrees::ensure_recyclable(&record)?;
    ensure_source_files_isolated(&record)?;
    let op = store.begin(&record, "pool_release")?;
    let head = worktrees::git_text(Path::new(&record.repo_root), &["rev-parse", "HEAD"])?;
    let history_ref = format!("refs/locus/pool-history/{assignment_id}");
    worktrees::git(
        Path::new(&record.repo_root),
        &["update-ref", &history_ref, &head],
    )?;
    record.head_oid = head;
    record.editor_version = worktrees::editor_version(Path::new(&record.root));
    record.assignment_id = None;
    record.lifecycle = "available".into();
    record.dirty = false;
    store
        .state
        .records
        .insert(checkout_id.to_string(), record.clone());
    store.finish(&op, None)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignored_scripts_and_settings_cannot_cross_pool_assignments() {
        for relative in ["Assets/Old.cs", "ProjectSettings/Old.json"] {
            let (temp, repo) = worktrees::tests::fixture();
            let ignore = repo.join(".gitignore");
            let mut text = std::fs::read_to_string(&ignore).unwrap();
            text.push_str(&format!("\n/{relative}\n"));
            std::fs::write(&ignore, text).unwrap();
            worktrees::git(&repo, &["add", ".gitignore"]).unwrap();
            worktrees::git(&repo, &["commit", "-m", "ignore legacy source"]).unwrap();
            let pool_root = temp.path().join("pool");
            std::fs::create_dir(&pool_root).unwrap();
            let mut request = AcquirePoolRequest {
                source_root: repo.to_string_lossy().into(),
                pool_root: pool_root.to_string_lossy().into(),
                commit: "HEAD".into(), branch: None, max_slots: 1,
                assignment_id: Some("old-source-a".into()),
            };
            let first = acquire(&request).unwrap();
            let stale = Path::new(&first.worktree.root).join(relative);
            std::fs::write(&stale, b"private legacy source").unwrap();
            assert!(worktrees::git(Path::new(&first.worktree.repo_root), &["status", "--porcelain", "-z"]).unwrap().is_empty());
            let error = release(&repo, &first.worktree.checkout_id, "old-source-a", 1).unwrap_err();
            assert!(error.contains("untracked or ignored file"), "{error}");
            assert_eq!(std::fs::read(&stale).unwrap(), b"private legacy source");
            assert_eq!(WorktreeStore::open(&repo).unwrap().state.records[&first.worktree.checkout_id].assignment_id.as_deref(), Some("old-source-a"));
            std::fs::remove_file(&stale).unwrap();
            release(&repo, &first.worktree.checkout_id, "old-source-a", 1).unwrap();
            // An external writer may contaminate an already available slot.
            std::fs::write(&stale, b"private legacy source").unwrap();
            std::fs::write(repo.join("Assets/test.asset"), b"next commit\n").unwrap();
            worktrees::git(&repo, &["add", "Assets/test.asset"]).unwrap();
            worktrees::git(&repo, &["commit", "-m", "new assignment source"]).unwrap();
            request.assignment_id = Some("old-source-b".into());
            let error = acquire(&request).unwrap_err();
            assert!(error.contains("untracked or ignored file"), "{error}");
            let stored = WorktreeStore::open(&repo).unwrap().state.records[&first.worktree.checkout_id].clone();
            assert_eq!(stored.lifecycle, "quarantined");
            assert_eq!(stored.materialization_epoch, 1);
            assert!(stored.assignment_id.is_none());
            assert_eq!(worktrees::git_text(Path::new(&stored.repo_root), &["rev-parse", "HEAD"]).unwrap(), first.worktree.head_oid);
            assert_eq!(std::fs::read(&stale).unwrap(), b"private legacy source");
        }
    }

    #[test]
    fn long_pool_root_inside_source_is_rejected_before_materialization() {
        let (_temp, repo) = worktrees::tests::fixture();
        let mut pool_root = repo.join("nested-pool");
        while pool_root.to_string_lossy().len() < 300 {
            pool_root.push("long-pool-component-123456789");
        }
        std::fs::create_dir_all(&pool_root).unwrap();
        let result = acquire(&AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: None,
            max_slots: 1,
            assignment_id: Some("inside-source".into()),
        });
        assert!(result
            .unwrap_err()
            .contains("separate from the source checkout"));
        assert!(std::fs::read_dir(&pool_root).unwrap().next().is_none());
    }

    #[test]
    fn long_derived_cache_is_retired_within_its_existing_project_boundary() {
        let (temp, _repo) = worktrees::tests::fixture();
        let mut project = temp.path().join("slot");
        while project.to_string_lossy().len() < 230 {
            project.push("project-component-123456");
        }
        let cache = project.join("Library/Locus/knowledge_tantivy_index");
        std::fs::create_dir_all(&cache).unwrap();
        assert!(cache.to_string_lossy().len() > 260);
        std::fs::write(cache.join("owned-index.bin"), b"old projection").unwrap();
        let unity_cache = project.join("Library/ArtifactDB");
        std::fs::write(&unity_cache, b"Unity cache must remain").unwrap();
        invalidate_locus_projections(&project).unwrap();
        assert!(!cache.exists());
        assert_eq!(
            std::fs::read(unity_cache).unwrap(),
            b"Unity cache must remain"
        );
    }

    #[test]
    fn incomplete_reservation_is_not_a_live_slot_and_retry_recovers_once() {
        let (temp, repo) = worktrees::tests::fixture();
        let pool_root = temp.path().join("pool");
        std::fs::create_dir(&pool_root).unwrap();
        let destination = pool_root.join("reserved");
        let reservation = ManagedWorktree {
            checkout_id: "reservation-crash".into(),
            project_id: super::super::identity::ProjectIdResolver::resolve(&repo)
                .unwrap()
                .project_id
                .to_string(),
            root: destination.to_string_lossy().into(),
            repo_root: destination.to_string_lossy().into(),
            project_relative_path: String::new(),
            branch: Some("refs/heads/locus/pool/crash".into()),
            head_oid: worktrees::git_text(&repo, &["rev-parse", "HEAD"]).unwrap(),
            materialization_epoch: 0,
            managed: true,
            lifecycle: "preparing".into(),
            dirty: false,
            pool_slot: true,
            assignment_id: Some("crash-job".into()),
            editor_version: Some("6000.5.8f1".into()),
            last_error: None,
        };
        let mut store = WorktreeStore::open(&repo).unwrap();
        store
            .state
            .records
            .insert(reservation.checkout_id.clone(), reservation.clone());
        store.save().unwrap();
        let live = worktrees::mutation_lease(&store.common_dir, &reservation.checkout_id).unwrap();
        drop(store);
        let request = AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: None,
            max_slots: 1,
            assignment_id: Some("crash-job".into()),
        };
        assert!(acquire(&request).is_err());
        assert!(!destination.exists());
        let listed = worktrees::list(&repo).unwrap();
        assert_eq!(listed[0].lifecycle, "preparing");
        assert_eq!(listed[0].head_oid, reservation.head_oid);
        drop(live);
        let completed = acquire(&request).unwrap();
        assert_eq!(completed.worktree.lifecycle, "active");
        assert_eq!(completed.worktree.materialization_epoch, 1);
        assert!(!completed.worktree.checkout_id.starts_with("reservation-"));
        let repeated = acquire(&request).unwrap();
        assert_eq!(
            completed.worktree.checkout_id,
            repeated.worktree.checkout_id
        );
        assert_eq!(WorktreeStore::open(&repo).unwrap().state.records.len(), 1);
    }

    #[test]
    fn completed_git_checkout_recovers_after_missing_assignment_publish() {
        let (temp, repo) = worktrees::tests::fixture();
        let pool_root = temp.path().join("pool");
        std::fs::create_dir(&pool_root).unwrap();
        let target = pool_root.join("interrupted");
        let record = worktrees::create(&worktrees::CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: target.to_string_lossy().into(),
            branch: "locus/pool/interrupted".into(),
            start_ref: None,
            include_dirty: false,
        })
        .unwrap();
        let mut reservation = record.clone();
        reservation.checkout_id = "reservation-interrupted".into();
        reservation.pool_slot = true;
        reservation.materialization_epoch = 0;
        reservation.lifecycle = "preparing".into();
        reservation.assignment_id = Some("interrupted-job".into());
        let mut store = WorktreeStore::open(&repo).unwrap();
        store.state.records.remove(&record.checkout_id);
        store
            .state
            .records
            .insert(reservation.checkout_id.clone(), reservation);
        store.save().unwrap();
        drop(store);
        assert!(worktrees::runtime_lease(&target).is_err());
        let request = AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: None,
            max_slots: 1,
            assignment_id: Some("interrupted-job".into()),
        };
        // A clean Git status alone must not authorize crash recovery.
        std::fs::write(repo.join(".git/info/exclude"), "Assets/Old.cs\n").unwrap();
        let ignored = target.join("Assets/Old.cs");
        std::fs::write(&ignored, b"preserve interrupted source").unwrap();
        let error = acquire(&request).unwrap_err();
        assert!(error.contains("untracked or ignored file"), "{error}");
        assert_eq!(std::fs::read(&ignored).unwrap(), b"preserve interrupted source");
        assert!(worktrees::runtime_lease(&target).is_err());
        std::fs::remove_file(&ignored).unwrap();
        let recovered = acquire(&request).unwrap();
        assert!(recovered.reused);
        assert_eq!(recovered.worktree.checkout_id, record.checkout_id);
        assert_eq!(recovered.worktree.lifecycle, "active");
        assert!(worktrees::runtime_lease(&target).is_ok());
    }

    #[test]
    fn fixed_slot_preserves_library_and_rejects_stale_assignment() {
        let (temp, repo) = worktrees::tests::fixture();
        let pool_root = temp.path().join("pool");
        std::fs::create_dir(&pool_root).unwrap();
        let mut request = AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: None,
            max_slots: 1,
            assignment_id: Some("job-a".into()),
        };
        let first = acquire(&request).unwrap();
        assert!(!first.reused);
        let library = Path::new(&first.worktree.root).join("Library");
        std::fs::create_dir(&library).unwrap();
        let artifact = library.join("immutable-import.bin");
        std::fs::write(&artifact, [0, 1, 2, 3, 4]).unwrap();
        let created = std::fs::metadata(&artifact).unwrap().created().ok();
        assert!(release(&repo, &first.worktree.checkout_id, "wrong", 1).is_err());
        std::fs::write(
            Path::new(&first.worktree.root).join("Assets/dirty.asset"),
            "preserve",
        )
        .unwrap();
        assert!(release(&repo, &first.worktree.checkout_id, "job-a", 1)
            .unwrap_err()
            .contains("changes"));
        std::fs::remove_file(Path::new(&first.worktree.root).join("Assets/dirty.asset")).unwrap();
        release(&repo, &first.worktree.checkout_id, "job-a", 1).unwrap();
        std::fs::write(repo.join("Assets/test.asset"), "next commit\n").unwrap();
        worktrees::git(&repo, &["add", "."]).unwrap();
        worktrees::git(&repo, &["commit", "-m", "next"]).unwrap();
        request.assignment_id = Some("job-b".into());
        let second = acquire(&request).unwrap();
        assert!(second.reused && second.preserved_library);
        assert_eq!(first.worktree.root, second.worktree.root);
        assert_eq!(second.worktree.materialization_epoch, 2);
        assert_eq!(std::fs::read(&artifact).unwrap(), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            std::fs::metadata(&artifact).unwrap().created().ok(),
            created
        );
        assert!(release(&repo, &second.worktree.checkout_id, "job-a", 1).is_err());
        assert_eq!(
            std::fs::read_to_string(Path::new(&second.worktree.root).join("Assets/test.asset"))
                .unwrap(),
            "next commit\n"
        );
    }

    #[test]
    fn listing_does_not_adopt_changes_in_an_interrupted_reservation() {
        let (temp, repo) = worktrees::tests::fixture();
        let pool_root = temp.path().join("pool");
        std::fs::create_dir(&pool_root).unwrap();
        let target = pool_root.join("interrupted");
        let record = worktrees::create(&worktrees::CreateWorktreeRequest {
            source_root: repo.to_string_lossy().into(),
            destination: target.to_string_lossy().into(),
            branch: "codex/pool/interrupted-list".into(),
            start_ref: None,
            include_dirty: false,
        })
        .unwrap();
        let mut reservation = record.clone();
        reservation.checkout_id = "reservation-list".into();
        reservation.pool_slot = true;
        reservation.materialization_epoch = 0;
        reservation.lifecycle = "preparing".into();
        reservation.assignment_id = Some("interrupted-list-job".into());
        let mut store = WorktreeStore::open(&repo).unwrap();
        store.state.records.remove(&record.checkout_id);
        store
            .state
            .records
            .insert(reservation.checkout_id.clone(), reservation.clone());
        store.save().unwrap();
        drop(store);
        std::fs::write(
            target.join("Assets/test.asset"),
            "preserve unexpected commit\n",
        )
        .unwrap();
        worktrees::git(&target, &["add", "."]).unwrap();
        worktrees::git(&target, &["commit", "-m", "unexpected user work"]).unwrap();
        let changed_head = worktrees::git_text(&target, &["rev-parse", "HEAD"]).unwrap();
        let listed = worktrees::list(&repo).unwrap();
        assert_eq!(listed[0].head_oid, reservation.head_oid);
        assert_ne!(listed[0].head_oid, changed_head);
        assert_eq!(listed[0].branch, reservation.branch);
        let request = AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: reservation.head_oid,
            branch: None,
            max_slots: 1,
            assignment_id: Some("interrupted-list-job".into()),
        };
        assert!(acquire(&request).unwrap_err().contains("no longer matches"));
        assert_eq!(
            worktrees::git_text(&target, &["rev-parse", "HEAD"]).unwrap(),
            changed_head
        );
        assert!(worktrees::runtime_lease(&target).is_err());
    }

    #[test]
    fn capacity_and_exact_editor_version_are_hard_boundaries() {
        let (temp, repo) = worktrees::tests::fixture();
        let pool_root = temp.path().join("pool");
        std::fs::create_dir(&pool_root).unwrap();
        let mut request = AcquirePoolRequest {
            source_root: repo.to_string_lossy().into(),
            pool_root: pool_root.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: None,
            max_slots: 1,
            assignment_id: Some("a".into()),
        };
        let first = acquire(&request).unwrap();
        request.assignment_id = Some("b".into());
        assert!(acquire(&request).unwrap_err().contains("capacity"));
        release(&repo, &first.worktree.checkout_id, "a", 1).unwrap();
        std::fs::write(
            repo.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 6000.5.9f1\n",
        )
        .unwrap();
        worktrees::git(&repo, &["add", "."]).unwrap();
        worktrees::git(&repo, &["commit", "-m", "editor version"]).unwrap();
        assert!(acquire(&request).unwrap_err().contains("capacity"));
    }
}
