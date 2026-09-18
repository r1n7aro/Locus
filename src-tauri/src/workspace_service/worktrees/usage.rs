//! Read-only pool usage and creation estimates. Previewing never creates a slot.
use super::*;
use super::estimate::{Progress, is_alias, library_bytes};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDiskBudget {
    pub checkout_bytes: u64,
    pub reference_cache_bytes: u64,
    pub cache_known: bool,
    pub estimated_bytes: u64,
    pub free_bytes: Option<u64>,
    pub directory: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeCreationPlan {
    pub start_oid: String,
    pub requires_new_project: bool,
    pub at_capacity: bool,
    pub budget: Option<WorktreeDiskBudget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeUsageItem {
    pub worktree: ManagedWorktree,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePoolUsage {
    pub items: Vec<WorktreeUsageItem>,
    pub total_bytes: u64,
    pub available_projects: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeProjectUsage {
    pub project_id: String,
    pub source_root: String,
    pub usage: Option<WorktreePoolUsage>,
    pub error: Option<String>,
}

/// Saved projects are inspected without activating their workspace services.
pub fn project_pool_usages(checkouts: Vec<(String, String)>) -> Vec<WorktreeProjectUsage> {
    let mut projects = BTreeMap::<(String, String, String), Vec<String>>::new();
    for (project_id, root) in checkouts {
        // Linked checkouts share one journal. Separate clones with the same Unity
        // GUID must still be counted, as must distinct projects in one repository.
        let common = resolve_git_common_dir(Path::new(&root));
        let relative = common
            .as_ref()
            .and_then(|_| source_ownership(Path::new(&root)).ok())
            .map(|(_, relative)| relative)
            .unwrap_or_default();
        let repository = common
            .as_deref()
            .map(path_key)
            .unwrap_or_else(|| path_key(Path::new(&root)));
        projects
            .entry((project_id, repository, relative))
            .or_default()
            .push(root);
    }
    projects
        .into_iter()
        .map(|((project_id, _, _), mut roots)| {
            roots.sort_by_key(|root| (!Path::new(root).join(".git").is_dir(), root.clone()));
            let source_root = roots
                .iter()
                .find(|root| Path::new(root).is_dir())
                .unwrap_or(&roots[0])
                .clone();
            let source = Path::new(&source_root);
            let result = if !source.is_dir() {
                Err("Project directory is unavailable".into())
            } else if resolve_git_common_dir(source).is_none() && !project_id.starts_with("git-") {
                Ok(WorktreePoolUsage {
                    items: Vec::new(),
                    total_bytes: 0,
                    available_projects: 0,
                })
            } else {
                pool_usage(source)
            };
            match result {
                Ok(usage) => WorktreeProjectUsage {
                    project_id,
                    source_root,
                    usage: Some(usage),
                    error: None,
                },
                Err(error) => WorktreeProjectUsage {
                    project_id,
                    source_root,
                    usage: None,
                    error: Some(error),
                },
            }
        })
        .collect()
}

pub fn pool_usage(source: &Path) -> Result<WorktreePoolUsage, String> {
    let common = resolve_git_common_dir(source).ok_or("Workspace is not a Git checkout")?;
    let (project_id, relative) = source_ownership(source)?;
    let state = read_state(&common)?;
    let mut items = Vec::new();
    let mut total_bytes = 0u64;
    let mut seen = BTreeSet::new();
    for mut worktree in state.records.into_values() {
        if worktree.project_id != project_id || worktree.project_relative_path != relative {
            continue;
        }
        if !worktree.managed
            || worktree.lifecycle == "removed"
            || worktree.checkout_id.starts_with("reservation-")
        {
            continue;
        }
        let root = Path::new(&worktree.repo_root);
        if !root.is_dir() {
            worktree.lifecycle = "missing".into();
        } else if matches!(worktree.lifecycle.as_str(), "active" | "available") {
            worktree.dirty = !git(
                root,
                &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
            )?
            .is_empty();
            worktree.branch = git_text(root, &["symbolic-ref", "-q", "HEAD"]).ok();
        }
        let size_bytes = estimate::directory_bytes(root, &mut Progress::new(&|_| {}))?.0;
        if seen.insert(path_key(Path::new(&worktree.repo_root))) {
            total_bytes = total_bytes.saturating_add(size_bytes);
        }
        items.push(WorktreeUsageItem {
            worktree,
            size_bytes,
        });
    }
    let available_projects = items
        .iter()
        .filter(|item| crate::workspace_service::pool::can_reuse(&item.worktree))
        .count();
    Ok(WorktreePoolUsage {
        items,
        total_bytes,
        available_projects,
    })
}

pub fn disk_budget(
    source: &Path,
    oid: &str,
    include_dirty: bool,
    directory: &Path,
) -> Result<WorktreeDiskBudget, String> {
    disk_budget_with_progress(source, oid, include_dirty, directory, &mut Progress::new(&|_| {}))
}

fn disk_budget_with_progress(source: &Path, oid: &str, include_dirty: bool, directory: &Path, progress: &mut Progress<'_>) -> Result<WorktreeDiskBudget, String> {
    progress.send("checkout", 0, None, 0);
    let repo = canonical(Path::new(&git_text(
        source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let tree = git(&repo, &["ls-tree", "-r", "-l", "-z", oid])?;
    let mut checkout_bytes = 0u64;
    let total_files = tree.iter().filter(|byte| **byte == 0).count() as u64;
    for (index, entry) in tree
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .enumerate()
    {
        let entry = std::str::from_utf8(entry).map_err(io_error)?;
        let (header, name) = entry.split_once('\t').ok_or("Invalid Git tree entry")?;
        let blob_size = header
            .split_whitespace()
            .nth(3)
            .and_then(|size| size.parse::<u64>().ok())
            .unwrap_or(0);
        // Materialized LFS objects can be much larger than their Git pointers.
        let local_size = std::fs::symlink_metadata(repo.join(safe_relative(name)?))
            .ok()
            .filter(|metadata| metadata.is_file() && !is_alias(metadata))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        checkout_bytes = checkout_bytes.saturating_add(blob_size.max(local_size));
        progress.send("checkout", index as u64 + 1, Some(total_files), checkout_bytes);
    }
    if include_dirty {
        progress.send("untracked", 0, None, checkout_bytes);
        for (index, path) in git(&repo, &["ls-files", "--others", "--exclude-standard", "-z"])?
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .enumerate()
        {
            let name = std::str::from_utf8(path).map_err(io_error)?;
            let metadata =
                std::fs::symlink_metadata(repo.join(safe_relative(name)?)).map_err(io_error)?;
            if metadata.is_file() && !is_alias(&metadata) {
                checkout_bytes = checkout_bytes.saturating_add(metadata.len());
            }
            progress.send("untracked", index as u64 + 1, None, checkout_bytes);
        }
    }
    let library = source.join("Library");
    let reference_cache_bytes = library_bytes(&library, progress)?;
    let cache_known = library.join("ArtifactDB").is_file()
        || library.join("SourceAssetDB").is_file()
        || !source.join("ProjectSettings/ProjectVersion.txt").exists();
    let mut volume_path = directory;
    progress.send("space", 0, None, checkout_bytes.saturating_add(reference_cache_bytes));
    while !volume_path.exists() {
        volume_path = volume_path
            .parent()
            .ok_or("Destination has no existing parent")?;
    }
    Ok(WorktreeDiskBudget {
        checkout_bytes,
        reference_cache_bytes,
        cache_known,
        estimated_bytes: checkout_bytes.saturating_add(reference_cache_bytes),
        free_bytes: fs4::available_space(volume_path).ok(),
        directory: directory.to_string_lossy().into(),
    })
}

pub fn creation_plan(
    source: &Path,
    start_ref: &str,
    include_dirty: bool,
    pool_mode: bool,
    directory: Option<&Path>,
    max_slots: usize,
) -> Result<WorktreeCreationPlan, String> {
    creation_plan_with_progress(source, start_ref, include_dirty, pool_mode, directory, max_slots, &mut Progress::new(&|_| {}))
}

pub fn creation_plan_with_progress(source: &Path, start_ref: &str, include_dirty: bool, pool_mode: bool, directory: Option<&Path>, max_slots: usize, progress: &mut Progress<'_>) -> Result<WorktreeCreationPlan, String> {
    progress.send("checking", 0, None, 0);
    if start_ref.starts_with('-') {
        return Err("Invalid start revision".into());
    }
    let oid = git_text(
        source,
        &["rev-parse", "--verify", &format!("{start_ref}^{{commit}}")],
    )?;
    let repo = canonical(Path::new(&git_text(
        source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let relative = existing_path_relative(&repo, source)?.ok_or("Source is outside repository")?;
    let version_path = relative
        .join("ProjectSettings/ProjectVersion.txt")
        .to_string_lossy()
        .replace('\\', "/");
    let version = git_text(&repo, &["show", &format!("{oid}:{version_path}")]).unwrap_or_default();
    let editor = version
        .lines()
        .find_map(|line| line.strip_prefix("m_EditorVersion:").map(str::trim))
        .unwrap_or("");
    let compatible = pool_mode
        && editor.starts_with("6000.5.")
        && (!include_dirty || editor_version(source).as_deref() == Some(editor));
    let records = list(source)?;
    let mut pools: Vec<_> = records
        .iter()
        .filter(|item| {
            item.pool_slot
                && item.managed
                && item.editor_version.as_deref() == Some(editor)
                && matches!(item.lifecycle.as_str(), "available" | "active")
        })
        .collect();
    pools.sort_by_cached_key(|item| {
        (
            !crate::workspace_service::pool::can_reuse(item),
            item.checkout_id.clone(),
        )
    });
    let container = match directory {
        Some(path) => path.to_path_buf(),
        None => match pools
            .iter()
            .find_map(|item| Path::new(&item.repo_root).parent())
        {
            Some(path) if compatible => path.to_path_buf(),
            _ => selector::pool_root_path(source)?,
        },
    };
    let in_container = |item: &&ManagedWorktree| {
        Path::new(&item.repo_root)
            .parent()
            .is_some_and(|parent| path_key(parent) == path_key(&container))
    };
    let reusable = compatible
        && pools
            .iter()
            .copied()
            .filter(in_container)
            .any(|item| crate::workspace_service::pool::can_reuse(item));
    let occupied = records
        .iter()
        .filter(|item| item.pool_slot && item.lifecycle != "removed")
        .filter(in_container)
        .count();
    let at_capacity = compatible && !reusable && occupied >= max_slots;
    Ok(WorktreeCreationPlan {
        start_oid: oid.clone(),
        requires_new_project: !reusable,
        at_capacity,
        budget: if reusable || at_capacity {
            None
        } else {
            Some(disk_budget_with_progress(source, &oid, include_dirty, &container, progress)?)
        },
    })
}

pub fn selection_plan(
    source: &Path,
    branch: &str,
    create_branch: bool,
    include_dirty: bool,
    max_slots: usize,
) -> Result<WorktreeCreationPlan, String> {
    selection_plan_with_progress(source, branch, create_branch, include_dirty, max_slots, None, &mut Progress::new(&|_| {}))
}

pub fn selection_plan_with_progress(source: &Path, branch: &str, create_branch: bool, include_dirty: bool, max_slots: usize, start_ref: Option<&str>, progress: &mut Progress<'_>) -> Result<WorktreeCreationPlan, String> {
    progress.send("checking", 0, None, 0);
    let start = selector::selection_start(source, branch, create_branch, include_dirty, start_ref)?;
    if branch.is_empty() || branch.starts_with(['-', '@']) {
        return Err("A literal branch name is required".into());
    }
    git(source, &["check-ref-format", "--branch", branch])?;
    if create_branch
        && git(
            source,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")],
        )
        .is_ok()
    {
        return Err("Branch already exists".into());
    }
    if !create_branch {
        if let Some(item) = selector::branches(source)?
            .into_iter()
            .find(|item| !item.remote && item.branch.as_deref() == Some(branch) && item.root.is_some())
        {
            if item.unavailable {
                return Err("Worktree is not active; inspect it in settings".into());
            }
            return Ok(WorktreeCreationPlan {
                start_oid: item.head_oid,
                requires_new_project: false,
                at_capacity: false,
                budget: None,
            });
        }
    }
    creation_plan_with_progress(
        source,
        &start,
        include_dirty,
        true,
        None,
        max_slots,
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;
    use crate::workspace_service::pool;

    #[test]
    fn global_usage_deduplicates_linked_checkouts_but_counts_separate_clones() {
        let (_temp, source) = fixture();
        let (_other_temp, other) = fixture();
        let first =
            selector::select_with_creation_policy(&source, "codex/first", true, false, 2, true)
                .unwrap();
        let second =
            selector::select_with_creation_policy(&other, "codex/second", true, false, 2, true)
                .unwrap();
        let rows = project_pool_usages(vec![
            ("same-logical-project".into(), first.root.clone()),
            (
                "same-logical-project".into(),
                source.to_string_lossy().into(),
            ),
            ("same-logical-project".into(), second.root.clone()),
            (
                "same-logical-project".into(),
                other.to_string_lossy().into(),
            ),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter()
                .map(|row| row.usage.as_ref().unwrap().items.len())
                .sum::<usize>(),
            2
        );
        assert!(rows
            .iter()
            .all(|row| Path::new(&row.source_root).join(".git").is_dir()));
    }

    #[test]
    fn inspecting_all_projects_never_initializes_empty_pools_and_keeps_partial_results() {
        let (temp, source) = fixture();
        let plain = temp.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        let common = resolve_git_common_dir(&source).unwrap();
        let rows = project_pool_usages(vec![
            (
                source_ownership(&source).unwrap().0,
                source.to_string_lossy().into(),
            ),
            ("plain-project".into(), plain.to_string_lossy().into()),
            (
                "missing-project".into(),
                temp.path().join("missing").to_string_lossy().into(),
            ),
        ]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|row| row.usage.is_some()).count(), 2);
        assert_eq!(rows.iter().filter(|row| row.error.is_some()).count(), 1);
        assert!(!common.join("locus-worktrees").exists());
        assert!(!selector::pool_root_path(&source).unwrap().exists());
        assert_eq!(std::fs::read_dir(plain).unwrap().count(), 0);
    }

    #[test]
    fn existing_checkouts_bypass_pool_capacity_and_new_project_confirmation() {
        let (temp, source) = fixture();
        let branch = git_text(&source, &["branch", "--show-current"]).unwrap();
        let current = selection_plan(&source, &branch, false, false, 0).unwrap();
        assert!(!current.requires_new_project && !current.at_capacity && current.budget.is_none());
        let external = temp.path().join("external");
        git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "feature/external",
                external.to_str().unwrap(),
            ],
        )
        .unwrap();
        let plan = selection_plan(&source, "feature/external", false, false, 0).unwrap();
        assert!(!plan.requires_new_project && !plan.at_capacity && plan.budget.is_none());
        let selected = selector::select(&source, "feature/external", false, false, 0).unwrap();
        assert_eq!(
            canonical(Path::new(&selected.root)).unwrap(),
            canonical(&external).unwrap()
        );
        assert!(!selected.pool_slot);
        assert!(!selector::pool_root_path(&source).unwrap().exists());
        assert!(pool_usage(&source).unwrap().items.is_empty());
    }

    #[test]
    fn full_pool_does_not_scan_project_files_or_library() {
        let (_temp, source) = fixture();
        let events = std::sync::Mutex::new(Vec::new());
        let report = |event: estimate::WorktreePlanProgress| events.lock().unwrap().push(event.phase);
        let plan = selection_plan_with_progress(&source, "codex/full", true, false, 0, None, &mut Progress::new(&report)).unwrap();
        assert!(plan.at_capacity);
        assert!(plan.budget.is_none());
        let events = events.lock().unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().all(|phase| *phase == "checking"));
    }

    #[test]
    fn empty_pool_is_only_estimated_until_new_project_is_confirmed() {
        let (_temp, source) = fixture();
        let container = selector::pool_root_path(&source).unwrap();
        let plan = selection_plan(&source, "codex/planned", true, true, 2).unwrap();
        assert!(plan.requires_new_project);
        assert!(plan.budget.as_ref().unwrap().checkout_bytes > 0);
        assert!(!plan.budget.as_ref().unwrap().cache_known);
        assert!(!container.exists());
        assert!(pool_usage(&source).unwrap().items.is_empty());
        assert!(selector::select(&source, "codex/planned", true, true, 2)
            .unwrap_err()
            .contains("WORKTREE_NEW_PROJECT_REQUIRED"));
        assert!(!container.exists());
        assert!(git(
            &source,
            &["show-ref", "--verify", "refs/heads/codex/planned"]
        )
        .is_err());
        let created =
            selector::select_with_creation_policy(&source, "codex/planned", true, true, 2, true)
                .unwrap();
        assert!(Path::new(&created.root).is_dir());
        assert!(created.assignment_id.is_some());
    }

    #[test]
    fn usage_counts_fixed_projects_and_only_manual_release_allows_reuse() {
        let (_temp, source) = fixture();
        let first =
            selector::select_with_creation_policy(&source, "codex/fixed", true, false, 1, true)
                .unwrap();
        let root = Path::new(&first.root);
        std::fs::create_dir_all(root.join("Library")).unwrap();
        std::fs::write(root.join("Library/ArtifactDB"), vec![0u8; 4096]).unwrap();
        let usage = pool_usage(&source).unwrap();
        assert_eq!(usage.items.len(), 1);
        assert_eq!(usage.available_projects, 0);
        assert!(usage.total_bytes >= 4096);
        assert!(
            selection_plan(&source, "codex/next", true, false, 1)
                .unwrap()
                .at_capacity
        );
        pool::release(
            &source,
            &first.checkout_id,
            first.assignment_id.as_deref().unwrap(),
            first.materialization_epoch,
        )
        .unwrap();
        assert_eq!(pool_usage(&source).unwrap().available_projects, 1);
        assert!(
            !selection_plan(&source, "codex/next", true, false, 1)
                .unwrap()
                .requires_new_project
        );
        let reused = selector::select(&source, "codex/next", true, false, 1).unwrap();
        assert_eq!(reused.checkout_id, first.checkout_id);
        assert_eq!(
            std::fs::read(root.join("Library/ArtifactDB"))
                .unwrap()
                .len(),
            4096
        );
    }

    #[test]
    fn a_reuse_race_never_silently_allocates_another_project() {
        let (_temp, source) = fixture();
        let first =
            selector::select_with_creation_policy(&source, "codex/first", true, false, 3, true)
                .unwrap();
        pool::release(
            &source,
            &first.checkout_id,
            first.assignment_id.as_deref().unwrap(),
            first.materialization_epoch,
        )
        .unwrap();
        assert!(
            !selection_plan(&source, "codex/waiting", true, false, 3)
                .unwrap()
                .requires_new_project
        );
        selector::select(&source, "codex/other", true, false, 3).unwrap();
        assert!(selector::select(&source, "codex/waiting", true, false, 3)
            .unwrap_err()
            .contains("WORKTREE_NEW_PROJECT_REQUIRED"));
        assert_eq!(pool_usage(&source).unwrap().items.len(), 1);
    }

    #[test]
    fn disk_estimate_includes_reference_library_and_untracked_source() {
        let (_temp, source) = fixture();
        std::fs::create_dir_all(source.join("Library")).unwrap();
        std::fs::write(source.join("Library/ArtifactDB"), vec![0u8; 4096]).unwrap();
        std::fs::write(source.join("Assets/new.asset"), vec![1u8; 1024]).unwrap();
        let clean = selection_plan(&source, "codex/estimate", true, false, 2)
            .unwrap()
            .budget
            .unwrap();
        let dirty = selection_plan(&source, "codex/estimate", true, true, 2)
            .unwrap()
            .budget
            .unwrap();
        assert_eq!(dirty.checkout_bytes, clean.checkout_bytes + 1024);
        assert_eq!(dirty.reference_cache_bytes, 4096);
        assert!(dirty.cache_known);
        assert_eq!(dirty.estimated_bytes, dirty.checkout_bytes + 4096);
        assert!(dirty.free_bytes.is_some());
    }
}
