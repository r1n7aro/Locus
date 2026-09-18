//! Scoped targets plus immutable dependency evidence. Git-backed inputs are
//! dependency-only: they must never stand in for the raw bytes of a write target.
use super::*;
use rayon::prelude::*;

pub fn evidence(path: &str) -> bool {
    path.ends_with(".meta")
        || path.ends_with(".cs")
        || path.ends_with(".asmdef")
        || matches!(
            Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "unity"
                | "prefab"
                | "asset"
                | "mat"
                | "anim"
                | "controller"
                | "overridecontroller"
                | "playable"
                | "mask"
                | "rendertexture"
                | "terrainlayer"
        )
        || matches!(
            path,
            "Packages/manifest.json"
                | "Packages/packages-lock.json"
                | "ProjectSettings/ProjectVersion.txt"
        )
}

pub fn paths(
    root: &Path,
    project: &Path,
    request: &PrepareRequest,
) -> Result<Option<Vec<String>>, String> {
    let Some(paths) = &request.paths else {
        if request.mode == PrepareMode::Files {
            return Err("File replacement requires explicit paths".into());
        }
        return Ok(None);
    };
    if paths.is_empty() {
        return Err("paths must contain at least one exact repository-relative file".into());
    }
    let mut result = BTreeSet::new();
    for path in paths {
        if path.contains('*') || path.contains('?') {
            return Err("Merge paths must be exact file paths, not globs".into());
        }
        let full = safe_path(root, path)?;
        if !path_in_project(root, project, path) || full.is_dir() {
            return Err(format!(
                "Merge path must be a file inside the destination project: {path}"
            ));
        }
        result.insert(path.clone());
        // Pairing expands the available scope, never the selected operations.
        if let Some(asset) = path.strip_suffix(".meta") {
            result.insert(asset.to_string());
        } else {
            result.insert(format!("{path}.meta"));
        }
    }
    Ok(Some(result.into_iter().collect()))
}

pub fn names(root: &Path, args: &[&str]) -> Result<BTreeSet<String>, String> {
    git(root, args)?
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(|v| String::from_utf8(v.to_vec()).map_err(|e| e.to_string()))
        .collect()
}

pub fn tree(root: &Path, revision: &str) -> Result<BTreeMap<String, FileState>, String> {
    let bytes = git(root, &["ls-tree", "-r", "-z", revision])?;
    let mut files = BTreeMap::new();
    for raw in bytes.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let record = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
        let (header, path) = record.split_once('\t').ok_or("Invalid tree entry")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 {
            return Err("Invalid tree entry".into());
        }
        if fields[1] == "blob" || evidence(path) {
            files.insert(
                path.into(),
                FileState {
                    blob: format!("git-{}", fields[2]),
                    mode: fields[0].into(),
                },
            );
        }
    }
    Ok(files)
}

pub fn capture_paths(
    root: &Path,
    dir: &Path,
    paths: &BTreeSet<String>,
    modes: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Option<FileState>>, String> {
    parallel::pool().install(|| {
        paths
            .par_iter()
            .map(|path| {
                let state = capture(root, dir, path)?;
                #[cfg(windows)]
                let state = state.map(|mut state| {
                    if let Some(mode) = modes.get(path) {
                        state.mode = mode.clone();
                    }
                    state
                });
                #[cfg(not(windows))]
                let _ = modes;
                Ok((path.clone(), state))
            })
            .collect()
    })
}

pub fn dependencies(
    root: &Path,
    project: &Path,
    dir: &Path,
    head: &str,
    selected: &BTreeSet<String>,
    dirty: &BTreeSet<String>,
    untracked: &BTreeSet<String>,
    modes: &BTreeMap<String, String>,
    flags: &str,
) -> Result<(BTreeMap<String, Option<FileState>>, Option<String>), String> {
    let tree = tree(root, head)?;
    // Git can omit assume-unchanged/skip-worktree content from diff. Freeze those
    // bytes conservatively instead of treating an advisory flag as proof.
    let hidden: BTreeSet<_> = flags
        .split('\0')
        .filter_map(|record| {
            let (flag, path) = record.split_once(' ')?;
            (flag == "S" || flag.chars().any(char::is_lowercase)).then(|| path.to_string())
        })
        .collect();
    let mut files: BTreeMap<_, _> = tree
        .into_iter()
        .filter(|(p, _)| evidence(p) && path_in_project(root, project, p) && !selected.contains(p))
        .map(|(p, s)| (p, Some(s)))
        .collect();
    for (path, state) in &files {
        if state
            .as_ref()
            .is_some_and(|state| !matches!(state.mode.as_str(), "100644" | "100755"))
        {
            return Err(format!("Unsupported dependency mode: {path}"));
        }
    }
    let mut overlay: BTreeSet<_> = dirty
        .iter()
        .chain(untracked)
        .chain(&hidden)
        .filter(|p| evidence(p) && path_in_project(root, project, p) && !selected.contains(*p))
        .cloned()
        .collect();
    // Include dirty/untracked evidence in the policy manifest as well.
    for path in &overlay {
        files.entry(path.clone()).or_insert(None);
    }
    let attributes = attributes(root, &files)?;
    let policy = policy(root, &attributes)?;
    if !files.is_empty() {
        let fields: Vec<_> = attributes.split(|b| *b == 0).collect();
        for record in fields.chunks_exact(3) {
            let path = std::str::from_utf8(record[0]).map_err(|e| e.to_string())?;
            let value = std::str::from_utf8(record[2]).map_err(|e| e.to_string())?;
            if !matches!(value, "unspecified" | "unset") {
                overlay.insert(path.to_string());
            }
        }
    }
    files.extend(capture_paths(root, dir, &overlay, modes)?);
    Ok((files, Some(policy)))
}

fn attributes(root: &Path, files: &BTreeMap<String, Option<FileState>>) -> Result<Vec<u8>, String> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    let input = files
        .keys()
        .flat_map(|p| p.as_bytes().iter().copied().chain([0]))
        .collect();
    git_output_input(
        root,
        &[
            "check-attr",
            "-z",
            "--stdin",
            "filter",
            "working-tree-encoding",
        ],
        input,
    )
}
fn policy(root: &Path, attributes: &[u8]) -> Result<String, String> {
    let config = git(root, &["config", "--null", "--list"])?;
    let mut hash = blake3::Hasher::new();
    hash.update(&config);
    hash.update(&[0]);
    hash.update(attributes);
    Ok(hash.finalize().to_hex().to_string())
}

pub fn all_files(job: &MergeJob) -> BTreeMap<String, Option<FileState>> {
    let mut files = job.dependency_files.clone();
    files.extend(job.snapshot.files.clone());
    files
}

pub fn check_dependencies(root: &Path, dir: &Path, job: &MergeJob) -> Result<(), String> {
    if job.paths.is_none() {
        return Ok(());
    }
    if let Some(expected) = &job.dependency_policy {
        if &policy(root, &attributes(root, &job.dependency_files)?)? != expected {
            return Err(
                "stale: Git dependency attributes/configuration changed after the snapshot".into(),
            );
        }
    }
    let raw: BTreeMap<_, _> = job
        .dependency_files
        .iter()
        .filter(|(_, state)| !state.as_ref().is_some_and(|s| git_blob_oid(s).is_some()))
        .map(|(p, s)| (p.clone(), s.clone()))
        .collect();
    check_files(root, dir, &raw)?;
    let dirty = names(root, &["diff", "--name-only", "-z", "HEAD", "--"])?;
    let untracked = names(root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in dirty.iter().chain(&untracked) {
        if !evidence(path)
            || !path_in_project(root, Path::new(&job.project_root), path)
            || job.snapshot.files.contains_key(path)
        {
            continue;
        }
        if !job.dependency_files.contains_key(path)
            || job.dependency_files[path]
                .as_ref()
                .is_some_and(|s| git_blob_oid(s).is_some())
        {
            return Err(format!(
                "stale: dependency {path} changed after the snapshot"
            ));
        }
    }
    Ok(())
}
