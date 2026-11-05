//! Chat's branch picker delegates materialization to the existing project pool.
use super::*;
use super::location::{editor_version_at, reusable_pool_container, WorktreePolicy};
use crate::workspace_service::pool::{self, AcquirePoolRequest};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeBranchOption {
    pub branch: Option<String>,
    pub remote: bool,
    pub root: Option<String>,
    pub current: bool,
    pub dirty: bool,
    pub head_oid: String,
    pub unavailable: bool,
}

/// Empty only outside a Git repository: a repository always lists its own
/// checkout, so the picker reads an empty list as "no worktrees here".
pub fn branches(source: &Path) -> Result<Vec<WorktreeBranchOption>, String> {
    let source = canonical(source)?;
    if !inside_git_repository(&source) {
        return Ok(Vec::new());
    }
    let records = list(&source)?;
    let refs = git_text(
        &source,
        &[
            "for-each-ref",
            "--format=%(refname)\t%(objectname)\t%(symref)",
            "refs/heads/",
            "refs/remotes/",
        ],
    )?;
    let mut options: Vec<_> = refs
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let reference = fields.next()?;
            let oid = fields.next()?;
            if !fields.next().unwrap_or_default().is_empty() { return None; }
            let remote = reference.starts_with("refs/remotes/");
            let branch = reference.strip_prefix(if remote { "refs/remotes/" } else { "refs/heads/" })?;
            Some(WorktreeBranchOption {
                branch: Some(branch.into()),
                remote,
                root: None,
                current: false,
                dirty: false,
                head_oid: oid.into(),
                unavailable: false,
            })
        })
        .collect();
    for path in discover(&source)? {
        let discovered = Path::new(&path);
        // Prunable worktrees and projects absent on a branch are not selectable.
        if !discovered.is_dir() {
            continue;
        }
        let physical_root = canonical(discovered)?;
        let root = physical_root.as_path();
        let branch = git_text(root, &["symbolic-ref", "-q", "HEAD"])
            .ok()
            .and_then(|value| value.strip_prefix("refs/heads/").map(str::to_string));
        let record = records
            .iter()
            .find(|item| path_key(Path::new(&item.root)) == path_key(root));
        let option = WorktreeBranchOption {
            branch: branch.clone(),
            remote: false,
            root: Some(path.clone()),
            current: path_key(root) == path_key(&source),
            dirty: !git(
                root,
                &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
            )?
            .is_empty(),
            head_oid: git_text(root, &["rev-parse", "HEAD"])?,
            unavailable: record.is_some_and(|item| item.lifecycle != "active"),
        };
        if let Some(index) = options
            .iter()
            .position(|item| !item.remote && branch.is_some() && item.branch == branch)
        {
            options[index] = option;
        } else {
            options.push(option);
        }
    }
    options.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then_with(|| a.remote.cmp(&b.remote))
            .then_with(|| a.branch.cmp(&b.branch))
    });
    Ok(options)
}

/// Mirrors Git's upward discovery with file-system checks only, so a
/// non-repository workspace costs no git process and no localized error text.
fn inside_git_repository(path: &Path) -> bool {
    path.ancestors().any(|dir| dir.join(".git").exists())
}

pub fn select(
    source: &Path,
    branch: &str,
    create_branch: bool,
    include_dirty: bool,
    max_slots: usize,
) -> Result<ManagedWorktree, String> {
    select_with_creation_policy(
        source,
        branch,
        create_branch,
        include_dirty,
        max_slots,
        false,
    )
}

pub fn select_with_creation_policy(
    source: &Path,
    branch: &str,
    create_branch: bool,
    include_dirty: bool,
    max_slots: usize,
    allow_new_project: bool,
) -> Result<ManagedWorktree, String> {
    select_from_ref(
        source,
        branch,
        create_branch,
        include_dirty,
        &WorktreePolicy::beside_repository(max_slots),
        allow_new_project,
        None,
    )
}

/// Remote refs are explicit creation sources; they never replace a local branch.
pub(super) fn selection_start(source: &Path, branch: &str, create_branch: bool, include_dirty: bool, start_ref: Option<&str>) -> Result<String, String> {
    if let Some(reference) = start_ref {
        if !create_branch || include_dirty || !reference.starts_with("refs/remotes/") {
            return Err("A remote start ref requires a new branch without local changes".into());
        }
        git(source, &["check-ref-format", reference])?;
        git(source, &["show-ref", "--verify", reference])?;
        if git(source, &["symbolic-ref", "-q", reference]).is_ok() {
            return Err("Select a remote branch, not a symbolic remote HEAD".into());
        }
        Ok(reference.into())
    } else {
        Ok(if create_branch { "HEAD".into() } else { format!("refs/heads/{branch}") })
    }
}

pub fn select_from_ref(
    source: &Path,
    branch: &str,
    create_branch: bool,
    include_dirty: bool,
    policy: &WorktreePolicy,
    allow_new_project: bool,
    start_ref: Option<&str>,
) -> Result<ManagedWorktree, String> {
    let start = selection_start(source, branch, create_branch, include_dirty, start_ref)?;
    if branch.is_empty() || branch.starts_with(['-', '@']) {
        return Err("A literal branch name is required".into());
    }
    git(source, &["check-ref-format", "--branch", branch])?;
    if !create_branch {
        if include_dirty {
            return Err("Local changes can only be copied to a new branch".into());
        }
        if let Some(option) = branches(source)?
            .into_iter()
            .find(|item| !item.remote && item.branch.as_deref() == Some(branch) && item.root.is_some())
        {
            if option.unavailable {
                return Err("Worktree is not active; inspect it in the worktree manager".into());
            }
            return import(source, Path::new(option.root.as_deref().unwrap()));
        }
    } else if git(
        source,
        &["show-ref", "--verify", &format!("refs/heads/{branch}")],
    )
    .is_ok()
    {
        return Err("Branch already exists".into());
    }
    let oid = git_text(
        source,
        &["rev-parse", "--verify", &format!("{start}^{{commit}}")],
    )?;
    let repo = canonical(Path::new(&git_text(
        source,
        &["rev-parse", "--show-toplevel"],
    )?))?;
    let editor = editor_version_at(source, &oid)?;
    let editor = editor.as_str();
    if !editor.starts_with("6000.5.")
        || (include_dirty && editor_version(source).as_deref() != Some(editor))
    {
        // The current pool supports Unity 6.5 only; ordinary worktrees keep other
        // Unity versions and non-Unity repositories usable without a new pool.
        if !allow_new_project {
            return Err(pool::NEW_PROJECT_REQUIRED.into());
        }
        return create_for_branch(
            &CreateWorktreeRequest {
                source_root: source.to_string_lossy().into(),
                destination: policy.new_checkout(source, "chat")?.to_string_lossy().into(),
                branch: branch.into(),
                start_ref: Some(oid),
                include_dirty,
            },
            !create_branch,
        );
    }
    // A released compatible slot is reused wherever it lives, including a
    // custom pool created by WorktreeManager / SDK callers. A new slot always
    // goes to the configured container.
    let container = match reusable_pool_container(&list(source)?, editor) {
        Some(path) => canonical(&path)?,
        None => {
            if !allow_new_project {
                return Err(pool::NEW_PROJECT_REQUIRED.into());
            }
            policy.ensure_container(source)?
        }
    };
    ensure_git_idle(&repo)?;
    let snapshot = if include_dirty {
        Some(dirty_snapshot(&repo)?)
    } else {
        None
    };
    // A chat may already run from a pool slot. Acquire through a sibling
    // outside the pool so the pool's source-isolation guard remains intact;
    // the immutable commit and dirty snapshot still come from this chat.
    let mut acquisition_source = source.to_path_buf();
    if existing_path_contains(&container, &repo)? {
        acquisition_source = discover(source)?
            .into_iter()
            .map(PathBuf::from)
            .find(|candidate| {
                candidate.is_dir()
                    && matches!(existing_path_contains(&container, candidate), Ok(false))
            })
            .ok_or("No source checkout outside the project pool is available")?;
    }
    let acquisition = pool::acquire_with_creation_policy(
        &AcquirePoolRequest {
            source_root: acquisition_source.to_string_lossy().into(),
            pool_root: container.to_string_lossy().into(),
            commit: oid.clone(),
            branch: create_branch.then(|| branch.into()),
            max_slots: policy.max_slots,
            assignment_id: None,
        },
        allow_new_project,
    )?;
    let mut record = acquisition.worktree;
    // Finalize under the same journal/lease used by pool materialization. A
    // failure retains an inspectable quarantined assignment, never a rollback
    // of the user's source checkout or another live slot.
    let mut store = WorktreeStore::open(source)?;
    let _lease = mutation_lease(&store.common_dir, &record.checkout_id)?;
    let op = store.begin(&record, "chat_worktree_prepare")?;
    let result: Result<(), String> = (|| {
        let target = Path::new(&record.repo_root);
        if !create_branch {
            git(target, &["checkout", "--no-overwrite-ignore", branch, "--"])?;
            if git_text(target, &["rev-parse", "HEAD"])? != oid {
                return Err("Branch changed during preparation".into());
            }
        }
        if let Some(snapshot) = &snapshot {
            if git_text(&repo, &["rev-parse", "HEAD"])? != oid
                || &dirty_snapshot(&repo)? != snapshot
            {
                return Err("Source changed during worktree preparation".into());
            }
            transfer_dirty(target, snapshot)?;
        }
        record.branch = Some(format!("refs/heads/{branch}"));
        record.dirty = !git(
            target,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?
        .is_empty();
        Ok(())
    })();
    if let Err(error) = &result {
        record.lifecycle = "quarantined".into();
        record.last_error = Some(error.clone());
    }
    store
        .state
        .records
        .insert(record.checkout_id.clone(), record.clone());
    store.finish(&op, result.as_ref().err().cloned())?;
    result?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;
    #[test]
    fn workspace_outside_a_repository_lists_no_branches() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain/nested");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(branches(&plain).unwrap().is_empty());

        let (_temp, source) = fixture();
        let nested = source.join("Assets");
        assert!(!branches(&source).unwrap().is_empty());
        assert!(inside_git_repository(&canonical(&nested).unwrap()));
    }

    #[test]
    fn remote_branches_are_distinct_from_local_names_and_exclude_symbolic_head() {
        let (_temp, source) = fixture();
        git(&source, &["update-ref", "refs/remotes/origin/白盒-4", "HEAD"]).unwrap();
        git(&source, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/白盒-4"]).unwrap();
        git(&source, &["branch", "origin/白盒-4"]).unwrap();
        let options = branches(&source).unwrap();
        assert_eq!(options.iter().filter(|item| item.branch.as_deref() == Some("origin/白盒-4")).count(), 2);
        assert!(options.iter().any(|item| item.remote && item.branch.as_deref() == Some("origin/白盒-4")));
        assert!(!options.iter().any(|item| item.branch.as_deref() == Some("origin/HEAD")));
    }

    #[test]
    fn remote_creation_uses_remote_commit_for_pool_and_ordinary_worktrees() {
        for editor in ["6000.5.0f1", "2022.3.47f1"] {
            let (_temp, source) = fixture();
            std::fs::write(source.join("ProjectSettings/ProjectVersion.txt"), format!("m_EditorVersion: {editor}\n")).unwrap();
            git(&source, &["add", "."]).unwrap();
            git(&source, &["commit", "--allow-empty", "-m", "remote version"]).unwrap();
            let remote_oid = git_text(&source, &["rev-parse", "HEAD"]).unwrap();
            git(&source, &["update-ref", "refs/remotes/origin/白盒-4", &remote_oid]).unwrap();
            std::fs::write(source.join("Assets/local-only.asset"), "local committed change").unwrap();
            git(&source, &["add", "."]).unwrap();
            git(&source, &["commit", "-m", "local ahead"]).unwrap();
            let local_oid = git_text(&source, &["rev-parse", "HEAD"]).unwrap();
            std::fs::write(source.join("Assets/draft.asset"), "local draft").unwrap();
            let policy = WorktreePolicy::beside_repository(2);
            let plan = usage::selection_plan_with_progress(&source, "白盒-4", true, false, &policy, Some("refs/remotes/origin/白盒-4"), &mut estimate::Progress::new(&|_| {})).unwrap();
            assert_eq!(plan.start_oid, remote_oid);
            assert!(git(&source, &["show-ref", "--verify", "refs/heads/白盒-4"]).is_err());
            let created = select_from_ref(&source, "白盒-4", true, false, &policy, true, Some("refs/remotes/origin/白盒-4")).unwrap();
            assert_eq!(created.head_oid, remote_oid);
            assert_eq!(created.pool_slot, editor.starts_with("6000.5."));
            assert_eq!(created.branch.as_deref(), Some("refs/heads/白盒-4"));
            assert!(!Path::new(&created.root).join("Assets/local-only.asset").exists());
            assert!(!Path::new(&created.root).join("Assets/draft.asset").exists());
            assert_eq!(git_text(&source, &["rev-parse", "HEAD"]).unwrap(), local_oid);
            assert!(source.join("Assets/draft.asset").is_file());
            assert!(select_from_ref(&source, "白盒-4", true, false, &policy, true, Some("refs/remotes/origin/白盒-4")).unwrap_err().contains("already exists"));
        }
    }

    #[test]
    fn remote_start_rejects_dirty_copy_missing_refs_and_revision_expressions() {
        let (_temp, source) = fixture();
        for (reference, create, dirty) in [
            ("refs/remotes/origin/missing", true, false),
            ("HEAD", true, false),
            ("refs/remotes/origin/main~1", true, false),
            ("refs/remotes/origin/main", false, false),
            ("refs/remotes/origin/main", true, true),
        ] {
            assert!(selection_start(&source, "codex/test", create, dirty, Some(reference)).is_err());
        }
    }
    fn select(
        source: &Path,
        branch: &str,
        create_branch: bool,
        include_dirty: bool,
        max_slots: usize,
    ) -> Result<ManagedWorktree, String> {
        super::select_with_creation_policy(
            source,
            branch,
            create_branch,
            include_dirty,
            max_slots,
            true,
        )
    }

    #[test]
    fn picker_creates_from_the_current_pool_branch_and_keeps_its_dirty_changes() {
        let (_temp, source) = fixture();
        let first = select(&source, "codex/pool-source", true, false, 2).unwrap();
        let current = Path::new(&first.root);
        std::fs::write(current.join("Assets/test.asset"), "current branch commit\n").unwrap();
        git(current, &["add", "."]).unwrap();
        git(current, &["commit", "-m", "current branch"]).unwrap();
        let head = git_text(current, &["rev-parse", "HEAD"]).unwrap();
        std::fs::write(current.join("Assets/test.asset"), "current branch draft\n").unwrap();
        let second = select(current, "codex/from-current-pool", true, true, 2).unwrap();
        assert_ne!(first.checkout_id, second.checkout_id);
        assert!(second.pool_slot);
        assert_eq!(second.head_oid, head);
        assert_eq!(
            std::fs::read_to_string(Path::new(&second.root).join("Assets/test.asset")).unwrap(),
            "current branch draft\n"
        );
        assert_eq!(
            std::fs::read_to_string(current.join("Assets/test.asset")).unwrap(),
            "current branch draft\n"
        );
    }

    #[test]
    fn picker_reuses_existing_checkout_without_changing_source() {
        let (_temp, source) = fixture();
        let first = select(&source, "codex/picker-one", true, false, 1).unwrap();
        git(&source, &["tag", "codex/picker-one"]).unwrap();
        let original = git_text(&source, &["rev-parse", "HEAD"]).unwrap();
        let again = select(&source, "codex/picker-one", false, false, 1).unwrap();
        assert_eq!(first.checkout_id, again.checkout_id);
        assert!(first.pool_slot);
        assert_eq!(git_text(&source, &["rev-parse", "HEAD"]).unwrap(), original);
        let items = branches(&source).unwrap();
        assert!(items[0].current);
        assert!(items
            .iter()
            .any(|item| item.branch.as_deref() == Some("codex/picker-one") && item.root.is_some()));
    }

    #[test]
    fn picking_the_source_branch_keeps_the_source_checkout_epoch() {
        let (_temp, source) = fixture();
        let identity = crate::workspace_service::identity::ProjectIdResolver::resolve(&source).unwrap();
        assert!(record_for_root(&source).unwrap().is_none());
        let current = git_text(&source, &["branch", "--show-current"]).unwrap();
        let picked = select(&source, &current, false, false, 1).unwrap();
        assert_eq!(picked.checkout_id, identity.checkout_id.to_string());
        assert!(!picked.managed && !picked.pool_slot);
        // Sessions and the open runtime were bound at the unjournaled epoch 0.
        assert_eq!(picked.materialization_epoch, 0);
        assert_eq!(record_for_root(&source).unwrap().unwrap().materialization_epoch, 0);
        assert!(!crate::workspace_service::identity::ProjectIdResolver::resolve(&source).unwrap().managed_worktree);
        assert_eq!(select(&source, &current, false, false, 1).unwrap().materialization_epoch, 0);
    }

    #[test]
    fn picker_reuses_released_pool_library_and_copies_dirty_source() {
        let (_temp, source) = fixture();
        let first = select(&source, "codex/picker-first", true, false, 1).unwrap();
        let sentinel = Path::new(&first.root).join("Library/picker-sentinel");
        std::fs::create_dir_all(sentinel.parent().unwrap()).unwrap();
        std::fs::write(&sentinel, "private library").unwrap();
        pool::release(
            &source,
            &first.checkout_id,
            first.assignment_id.as_deref().unwrap(),
            first.materialization_epoch,
        )
        .unwrap();
        let index = std::fs::read(source.join(".git/index")).unwrap();
        std::fs::write(source.join("Assets/test.asset"), "local work\n").unwrap();
        std::fs::write(source.join("Assets/untracked.asset"), "new local asset\n").unwrap();
        let second = select(&source, "codex/picker-second", true, true, 1).unwrap();
        assert_eq!(first.checkout_id, second.checkout_id);
        assert_eq!(
            second.materialization_epoch,
            first.materialization_epoch + 1
        );
        assert_eq!(
            std::fs::read_to_string(&sentinel).unwrap(),
            "private library"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&second.root).join("Assets/test.asset")).unwrap(),
            "local work\n"
        );
        assert_eq!(
            std::fs::read_to_string(Path::new(&second.root).join("Assets/untracked.asset"))
                .unwrap(),
            "new local asset\n"
        );
        assert_eq!(std::fs::read(source.join(".git/index")).unwrap(), index);
        assert!(second.dirty);
    }

    #[test]
    fn picker_materializes_existing_branch_in_pool_and_respects_capacity() {
        let (_temp, source) = fixture();
        git(&source, &["branch", "feature/existing"]).unwrap();
        let oid = git_text(&source, &["rev-parse", "refs/heads/feature/existing"]).unwrap();
        let target = select(&source, "feature/existing", false, false, 1).unwrap();
        assert!(target.pool_slot);
        assert_eq!(
            target.branch.as_deref(),
            Some("refs/heads/feature/existing")
        );
        assert_eq!(
            git_text(Path::new(&target.root), &["symbolic-ref", "HEAD"]).unwrap(),
            "refs/heads/feature/existing"
        );
        assert_eq!(
            git_text(&source, &["rev-parse", "refs/heads/feature/existing"]).unwrap(),
            oid
        );
        assert!(select(&source, "codex/at-capacity", true, false, 1)
            .unwrap_err()
            .contains("capacity"));
        assert!(select(&source, "feature/existing", true, false, 1)
            .unwrap_err()
            .contains("already exists"));
    }

    #[test]
    fn picker_supports_non_pool_projects_and_stable_container_from_linked_checkout() {
        let (_temp, source) = fixture();
        std::fs::write(
            source.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 2022.3.47f1\n",
        )
        .unwrap();
        git(&source, &["add", "."]).unwrap();
        git(&source, &["commit", "-m", "editor version"]).unwrap();
        git(&source, &["branch", "feature/legacy"]).unwrap();
        let target = select(&source, "feature/legacy", false, false, 1).unwrap();
        assert!(!target.pool_slot);
        let policy = WorktreePolicy::beside_repository(1);
        assert_eq!(
            policy.container(&source).unwrap(),
            policy.container(Path::new(&target.root)).unwrap()
        );
        assert_eq!(
            Path::new(&target.repo_root).parent().unwrap(),
            policy.container(&source).unwrap()
        );
        assert_eq!(
            git_text(Path::new(&target.root), &["symbolic-ref", "HEAD"]).unwrap(),
            "refs/heads/feature/legacy"
        );
        assert!(select(&source, "-invalid", true, false, 1).is_err());
        assert!(select(&source, "missing", false, false, 1).is_err());
    }

    #[test]
    fn picker_reuses_a_pool_previously_created_at_a_custom_location() {
        let (temp, source) = fixture();
        let custom = temp.path().join("custom-pool");
        std::fs::create_dir(&custom).unwrap();
        let first = pool::acquire(&AcquirePoolRequest {
            source_root: source.to_string_lossy().into(),
            pool_root: custom.to_string_lossy().into(),
            commit: "HEAD".into(),
            branch: Some("codex/sdk-created".into()),
            max_slots: Some(1),
            assignment_id: None,
        })
        .unwrap()
        .worktree;
        pool::release(
            &source,
            &first.checkout_id,
            first.assignment_id.as_deref().unwrap(),
            first.materialization_epoch,
        )
        .unwrap();
        let selected = select(&source, "codex/from-picker", true, false, 1).unwrap();
        assert_eq!(first.checkout_id, selected.checkout_id);
        assert_eq!(Path::new(&selected.repo_root).parent().unwrap(), custom);
    }

    #[test]
    fn picker_creates_new_slots_in_the_configured_root_and_reuses_released_slots_elsewhere() {
        let (temp, source) = fixture();
        let old = select(&source, "codex/old-root", true, false, 2).unwrap();
        let configured = temp.path().join("configured-root");
        std::fs::create_dir(&configured).unwrap();
        let policy = location::WorktreePolicy::with_root(
            configured.clone(),
            location::WorktreeRootSource::Settings,
            2,
        );
        let container = policy.container(&source).unwrap();
        // The old slot is still assigned, so a new slot follows the new root
        // instead of joining the old pool directory.
        let created = select_from_ref(&source, "codex/new-root", true, false, &policy, true, None).unwrap();
        assert_eq!(Path::new(&created.repo_root).parent().unwrap(), container);
        assert!(created.pool_slot);
        pool::release(
            &source,
            &old.checkout_id,
            old.assignment_id.as_deref().unwrap(),
            old.materialization_epoch,
        )
        .unwrap();
        let reused = select_from_ref(&source, "codex/reuse-old", true, false, &policy, false, None).unwrap();
        assert_eq!(reused.checkout_id, old.checkout_id);
        assert_ne!(Path::new(&reused.repo_root).parent().unwrap(), container);
    }
}
