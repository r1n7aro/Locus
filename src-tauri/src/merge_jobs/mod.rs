//! Agent-controlled, immutable-input integration jobs. Git discovers parent deltas;
//! Unity YAML is read and written exclusively by the in-house asset core.
//! Journals live outside the checkout, and applying a plan never writes its index.
mod commit_validation;
mod asset_preflight;
mod asset_apply;
mod catalog_query;
mod chain;
pub(crate) mod coordination;
mod dependencies;
mod eol_proof;
mod inspection;
mod journal;
mod parallel;
mod replan;
pub use replan::replan;
mod retention;
mod shared;
mod snapshot;
pub(crate) mod io;
mod schema;
mod scratch_validation;
mod text_merge;
mod transforms;
pub use commit_validation::validate_commit_unity;
mod types;
mod validation_scope;
use io::*;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use rayon::prelude::*;
use std::time::Instant;
pub use types::*;

fn error_value(code: &str, path: &str, detail: impl Into<String>) -> Value {
    json!({"code":code,"path":path,"detail":detail.into()})
}
fn state_bytes(dir: &Path, state: &Option<FileState>) -> Result<Vec<u8>, String> {
    state
        .as_ref()
        .map(|s| read_blob(dir, s))
        .transpose()
        .map(|v| v.unwrap_or_default())
}
fn unity_yaml(path: &str, bytes: &[u8]) -> bool {
    if opaque(path) {
        return false;
    }
    bytes.starts_with(b"%YAML") || bytes.starts_with(b"--- !u!") || path.ends_with(".meta")
}
fn opaque(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "fbx"
            | "glb"
            | "blend"
            | "obj"
            | "png"
            | "jpg"
            | "jpeg"
            | "psd"
            | "tga"
            | "exr"
            | "hdr"
            | "dds"
            | "wav"
            | "mp3"
            | "ogg"
            | "mp4"
            | "dll"
            | "bytes"
            | "ttf"
            | "otf"
    )
}

fn path_in_project(repo: &Path, project: &Path, path: &str) -> bool {
    crate::workspace_service::worktrees::path_relative(project, &repo.join(path))
        .ok()
        .flatten()
        .is_some_and(|relative| !relative.as_os_str().is_empty())
}
fn scope_allows(job: &MergeJob, path: &str) -> bool {
    path_in_project(Path::new(&job.root), Path::new(&job.project_root), path)
        && job.paths.as_ref().map_or(true, |paths| paths.iter().any(|p| p == path))
}
fn save(dir: &Path, job: &MergeJob) -> Result<(), String> {
    journal::save(dir, job)
}
pub fn load(root: &Path, id: &str) -> Result<MergeJob, String> {
    let requested_project = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let root = repository(root)?;
    let job = journal::load(&job_dir(&root, id)?)?;
    if Path::new(&job.root) != root {
        return Err("Merge job belongs to a different destination checkout; prepare a new job to change direction".into());
    }
    if Path::new(&job.project_root) != requested_project {
        return Err("Merge job belongs to another Unity project root within this repository; use its exact destination WorkspaceRef".into());
    }
    Ok(job)
}
/// Recent three-way sessions. Building the change catalog and the first
/// preview of the same snapshots otherwise each parse all three versions and
/// rebuild the catalog. Bounded by entry count and input size, since a
/// session keeps the parsed assets alive.
fn merge_session(
    base: &[u8],
    target: &[u8],
    source: &[u8],
    aliases: &[crate::unity_asset_core::FieldAlias],
) -> Result<std::sync::Arc<crate::unity_asset_core::MergeSession>, crate::unity_asset_core::CoreError> {
    use std::sync::{Arc, LazyLock, Mutex};
    type Sessions = Vec<(blake3::Hash, Arc<crate::unity_asset_core::MergeSession>)>;
    static SESSIONS: LazyLock<Mutex<Sessions>> = LazyLock::new(|| Mutex::new(Vec::new()));
    const ENTRIES: usize = 2;
    const INPUT_BYTES: usize = 32 * 1024 * 1024;
    let mut key = blake3::Hasher::new();
    for bytes in [base, target, source] {
        key.update(&(bytes.len() as u64).to_le_bytes());
        key.update(bytes);
    }
    key.update(&serde_json::to_vec(aliases).unwrap_or_default());
    let key = key.finalize();
    if let Some(session) = SESSIONS
        .lock()
        .ok()
        .and_then(|sessions| sessions.iter().find(|(k, _)| *k == key).map(|(_, s)| s.clone()))
    {
        return Ok(session);
    }
    let session = Arc::new(crate::unity_asset_core::prepare_merge_with_aliases(base, target, source, aliases)?);
    if base.len() + target.len() + source.len() <= INPUT_BYTES {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.retain(|(k, _)| *k != key);
            sessions.insert(0, (key, session.clone()));
            sessions.truncate(ENTRIES);
        }
    }
    Ok(session)
}

fn namespaced(commit: &str, path: &str, id: &str) -> String {
    format!("{commit}:{path}:{id}")
}

/// Catalog construction is paid only by structural queries/selections. A file
/// replacement keeps a coarse catalog and never parses the C# project schema.
fn ensure_catalog(dir: &Path, job: &mut MergeJob) -> Result<(), String> {
    if job.catalog_ready { return Ok(()); }
    if job.mode == PrepareMode::Structural {
        let mut guids = BTreeSet::new();
        for delta in &job.deltas {
            if delta.kind != "unity_yaml" { continue; }
            let bytes = state_bytes(dir, job.snapshot.files.get(&delta.path).unwrap_or(&None))?;
            if let Ok(asset) = crate::unity_asset_core::parse_shared(&bytes) {
                for doc in &asset.documents {
                    if let Some(guid) = doc.root.get("MonoBehaviour").and_then(|n|n.get("m_Script"))
                        .and_then(|n|n.get("guid")).and_then(|n|n.scalar(&asset)) {
                        guids.insert(guid.trim().to_ascii_lowercase());
                    }
                }
            }
        }
        if !guids.is_empty() {
            let files = snapshot::all_files(job);
            job.schemas.insert("target".into(), schema::working_for_guids(dir, &files, &guids)?);
            let trees: BTreeSet<_> = job.deltas.iter().flat_map(|d| [&d.parent, &d.commit]).cloned().collect();
            let schemas: Result<Vec<_>, String> = parallel::pool().install(|| trees.par_iter().map(|tree| {
                Ok((tree.clone(), schema::tree_for_guids(Path::new(&job.root), dir, tree, Path::new(&job.project_root), &guids)?))
            }).collect());
            job.schemas.extend(schemas?);
        }
    }
    let folder = dir.parent().ok_or("Missing catalog cache parent")?
        .join(retention::catalog_cache());
    std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let structural = job.mode == PrepareMode::Structural;
    // The commits of one file are cataloged in preview order, each against the
    // file as the earlier selected commits leave it (see `chain`).
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, delta) in job.deltas.iter().enumerate() {
        groups.entry(delta.path.as_str()).or_default().push(index);
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    for indices in &mut groups {
        indices.sort_by_key(|&i| job.deltas[i].kind != "text");
    }
    let shared = &*job;
    let computed: Result<Vec<Vec<(usize, Vec<Value>)>>, String> = parallel::pool().install(|| groups.par_iter().map(|indices| {
        let mut target = shared.snapshot.files.get(&shared.deltas[indices[0]].path).cloned().unwrap_or(None);
        let mut out = Vec::with_capacity(indices.len());
        for (position, &index) in indices.iter().enumerate() {
            let delta = &shared.deltas[index];
            let key = blake3::hash(&serde_json::to_vec(&(&delta.commit, &delta.parent, &delta.path,
                &delta.base, &delta.source, &target, structural, &shared.schemas)).map_err(|e|e.to_string())?).to_hex();
            let cache = folder.join(format!("{key}.json"));
            let cached = std::fs::read(&cache).ok().and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok());
            let mut changes = match cached {
                Some(value) => value,
                None => {
                    let value = catalog(dir, delta, &target, &shared.schemas, structural)?;
                    cache_json(&cache, &value)?;
                    value
                }
            };
            for change in &mut changes {
                change["outside_destination_scope"] = json!(!scope_allows(shared, &delta.path));
            }
            if position + 1 < indices.len() {
                target = chain::included_result(dir, delta, &target, &changes, &shared.schemas)?;
            }
            out.push((index, changes));
        }
        Ok(out)
    }).collect());
    let mut cataloged: Vec<Option<Vec<Value>>> = vec![None; job.deltas.len()];
    for (index, changes) in computed?.into_iter().flatten() {
        cataloged[index] = Some(changes);
    }
    for (delta, changes) in job.deltas.iter_mut().zip(cataloged) {
        delta.changes = changes.ok_or("A source delta was not cataloged")?;
    }
    job.catalog_ready = true;
    Ok(())
}

pub fn summary(job: &MergeJob) -> Value {
    json!({"id":job.id,"version":job.version,"project_root":job.project_root,"state":job.state,
        "revision":job.revision,"mode":job.mode,"paths":job.paths,"catalog_ready":job.catalog_ready,
        "source_files":job.deltas.len(),"prepare_metrics":job.prepare_metrics,
        "snapshot":{"head":job.snapshot.head,"branch":job.snapshot.branch,
            "materialization_epoch":job.snapshot.materialization_epoch,"file_count":job.snapshot.files.len(),
            "dependency_file_count":job.dependency_files.len()}})
}
/// Byte equality, or text equality that ignores CRLF/LF differences.
fn eol_equal(left: &[u8], right: &[u8]) -> bool {
    left == right
        || match (std::str::from_utf8(left), std::str::from_utf8(right)) {
            (Ok(left), Ok(right)) => left.replace("\r\n", "\n") == right.replace("\r\n", "\n"),
            _ => false,
        }
}

/// Status of a whole-file change. `clean` means `include` applies without a
/// decision (for text this may be a three-way line merge, see `merge`);
/// `conflict` always carries a `reason`, and text conflicts list `hunks`.
fn file_status(
    delta: &CommitDelta,
    target_state: &Option<FileState>,
    base: &[u8],
    target: &[u8],
    source: &[u8],
    structural_error: Option<String>,
) -> (&'static str, Value) {
    let present = (delta.base.is_some(), target_state.is_some(), delta.source.is_some());
    let text = delta.kind == "text";
    let same = |a: &[u8], b: &[u8]| if text { eol_equal(a, b) } else { a == b };
    if present.1 == present.0 && same(target, base) {
        if target != base {
            return ("clean", json!({"merge":"source_lines","reason":"the destination differs from the base only by line endings; include applies the source lines in the destination's line endings"}));
        }
        return ("clean", Value::Null);
    }
    if present.1 == present.2 && same(target, source) {
        if target != source {
            return ("already_applied", json!({"reason":"the destination already contains these source lines (line endings ignored)"}));
        }
        return ("already_applied", Value::Null);
    }
    let conflict = |reason: String| ("conflict", json!({ "reason": reason }));
    match present {
        (true, true, true) if text => text_merge::classify(base, target, source),
        (true, true, false) => conflict("the source deletes this file while the destination modified it; resolve side='source' to delete it or 'target' to keep the destination version".into()),
        (true, false, true) => conflict("the destination deleted this file while the source modified it; resolve side='source' to restore the source version or 'target' to keep it deleted".into()),
        (false, true, true) => conflict("both sides added this path with different content; resolve side='source' or 'target', or supply merged text with files.set_text".into()),
        _ if delta.kind == "binary" => conflict("the destination and the source both changed this binary/opaque file; choose a complete version with files.take".into()),
        _ => conflict(match structural_error {
            Some(error) => format!("structural Unity YAML merge is unavailable for this file ({error}); choose a complete version with files.take"),
            None => "the destination and the source both changed this file; choose a complete version with files.take".into(),
        }),
    }
}

fn catalog(
    dir: &Path,
    delta: &CommitDelta,
    target: &Option<FileState>,
    schemas: &BTreeMap<String, schema::SchemaMap>,
    structural: bool,
) -> Result<Vec<Value>, String> {
    let mode_change = match (&delta.base, &delta.source) {
        (Some(base), Some(source)) if base.mode != source.mode => Some(json!({
            "id":namespaced(&delta.commit,&delta.path,"file_mode"),"core_id":null,
            "commit":delta.commit,"parent":delta.parent,"path":delta.path,"kind":"file_mode",
            "file_kind":delta.kind,"status":"explicit_choice","base":base.mode,"source":source.mode,
            "target":target.as_ref().map(|s|&s.mode),"requires_whole_file_choice":true
        })),
        _ => None,
    };
    let target_state = target;
    let base = state_bytes(dir, &delta.base)?;
    let source = state_bytes(dir, &delta.source)?;
    let target = state_bytes(dir, target)?;
    let mut structural_error = None;
    if structural && delta.kind == "unity_yaml"
        && delta.base.is_some()
        && delta.source.is_some()
        && !target.is_empty()
    {
        let empty = schema::SchemaMap::new();
        let aliases = schema::aliases(
            &target,
            schemas.get(&delta.parent).unwrap_or(&empty),
            schemas.get(&delta.commit).unwrap_or(&empty),
            schemas.get("target").unwrap_or(&empty),
        );
        match merge_session(&base, &target, &source, &aliases)
        {
            Ok(session) => {
                let value = serde_json::to_value(session.catalog()).map_err(|e| e.to_string())?;
                let mut changes = value["changes"].as_array().cloned().unwrap_or_default();
                for change in &mut changes {
                    let id = change["id"]
                        .as_str()
                        .ok_or("Core change has no ID")?
                        .to_string();
                    change["core_id"] = json!(id);
                    change["id"] = json!(namespaced(&delta.commit, &delta.path, &id));
                    change["commit"] = json!(delta.commit);
                    change["parent"] = json!(delta.parent);
                    change["path"] = json!(delta.path);
                    change["file_kind"] = json!(delta.kind);
                }
                if !changes.is_empty() || base == source {
                    if let Some(mode_change) = &mode_change {
                        changes.push(mode_change.clone());
                    }
                    return Ok(changes);
                }
            }
            // Inspection remains available as an explicit whole-file choice.
            Err(error) => structural_error = Some(error.to_string()),
        }
    }
    let mut changes = vec![];
    if base != source || delta.base.is_some() != delta.source.is_some() {
        let (status, details) = file_status(delta, target_state, &base, &target, &source, structural_error);
        let mut change = json!({"id":namespaced(&delta.commit,&delta.path,"file"),"core_id":null,
        "commit":delta.commit,"parent":delta.parent,"path":delta.path,"kind":"file",
        "file_kind":delta.kind,"status":status,"reason":null,
        "base":delta.base,"source":delta.source,"requires_whole_file_choice":delta.kind=="binary" || delta.kind=="unity_yaml"});
        if let Some(details) = details.as_object() {
            for (key, value) in details {
                change[key] = value.clone();
            }
        }
        changes.push(change);
    }
    if let Some(mode_change) = mode_change {
        changes.push(mode_change);
    }
    Ok(changes)
}

/// The root can be any participating checkout. The caller resolves its explicit
/// WorkspaceRef before entry; no active pane or global working directory is consulted.
/// Tip and merge base of a squash source: `range` "A..B", or `HEAD..branch_ref`.
fn squash_endpoints(root: &Path, selection: &SourceSelection) -> Result<(String, String), String> {
    if !selection.commits.is_empty() {
        return Err("squash takes a range or branch_ref, not a commit list".into());
    }
    let range = match (&selection.range, &selection.branch_ref) {
        (Some(range), _) => range.clone(),
        (None, Some(branch)) => format!("HEAD..{branch}"),
        (None, None) => return Err("squash requires a range or branch_ref".into()),
    };
    let Some((from, to)) = range.split_once("..").filter(|_| !range.contains("...")) else {
        return Err("squash requires a two-dot A..B range".into());
    };
    let from = if from.is_empty() { "HEAD" } else { from };
    if to.is_empty() || from.starts_with('-') || to.starts_with('-') {
        return Err("Invalid squash range".into());
    }
    let tip = git_text(root, &["rev-parse", "--verify", &format!("{to}^{{commit}}")])?;
    let from = git_text(root, &["rev-parse", "--verify", &format!("{from}^{{commit}}")])?;
    let bases = git_text(root, &["merge-base", "--all", &from, &tip])
        .map_err(|e| format!("squash found no merge base between {from} and {tip}: {e}"))?;
    let bases: Vec<_> = bases.lines().collect();
    let [base] = bases.as_slice() else {
        return Err(format!(
            "squash needs exactly one merge base between {from} and {tip}, found {}; select commits explicitly",
            bases.len()
        ));
    };
    if *base == tip {
        return Err("Source selection contains no commits".into());
    }
    Ok((tip, base.to_string()))
}

pub fn prepare(project_root: &Path, request: &PrepareRequest) -> Result<MergeJob, String> {
    let started = Instant::now();
    let mut timings = serde_json::Map::new();
    let root = repository(project_root)?;
    let scope_root = dunce::canonicalize(project_root).map_err(|e| e.to_string())?;
    let selected_paths = snapshot::paths(&root, &scope_root, request)?;
    let _lock = lock(&root)?;
    if request.sources.is_empty() {
        return Err("At least one source commit selection is required".into());
    }
    let head = git_text(&root, &["rev-parse", "--verify", "HEAD"])?;
    let branch = git_text(&root, &["symbolic-ref", "--quiet", "HEAD"]).ok();
    let (index_records, index_flags) = index_entries(&root)?;
    if index_records
        .split('\0')
        .any(|r| !r.is_empty() && !r.split('\t').next().unwrap_or("").ends_with(" 0"))
    {
        return Err("Resolve the destination's existing unmerged index entries before preparing a new integration".into());
    }
    let markers = ["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD", "rebase-merge", "rebase-apply"];
    let mut args = vec!["rev-parse"];
    for marker in &markers {
        args.extend(["--git-path", marker]);
    }
    let marker_paths = git_text(&root, &args)?;
    let marker_paths: Vec<&str> = marker_paths.lines().collect();
    if marker_paths.len() != markers.len() {
        return Err("Git did not resolve the operation marker paths".into());
    }
    for (marker, path) in markers.iter().zip(marker_paths) {
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if path.exists() {
            return Err(format!(
                "Destination has an existing Git operation ({marker})"
            ));
        }
    }
    // Finished jobs and whatever only they used leave before a new one starts.
    let prune_started = Instant::now();
    retention::prune(&root, request.keep_job.as_deref())?;
    timings.insert("prune_ms".into(), json!(prune_started.elapsed().as_millis()));
    let id = uuid::Uuid::new_v4().to_string();
    let dir = job_dir(&root, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let guard = retention::PrepareGuard::new(&root, &id);
    init_storage(&root, &dir)?;
    // Private refs keep dependency/schema objects reachable even after source
    // branches move and Git garbage collection runs.
    git(&root, &["update-ref", &format!("refs/locus/merge-jobs/{id}/target"), &head])?;
    let mut deltas = vec![];
    let mut paths = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for selection in &request.sources {
        if !selection.commits.is_empty() && selection.range.is_some() {
            return Err("Specify commits or range, not both".into());
        }
        // (requested revision, delta base overriding its parent)
        let commits: Vec<(String, Option<String>)> = if let Some((commit, base)) = &selection.pinned {
            vec![(commit.clone(), Some(base.clone()))]
        } else if selection.squash {
            let (tip, base) = squash_endpoints(&root, selection)?;
            vec![(tip, Some(base))]
        } else if let Some(range) = &selection.range {
            if !range.contains("..") || range.starts_with('-') {
                return Err("Source range must be an explicit A..B or A...B revision range".into());
            }
            git_text(
                &root,
                &["rev-list", "--reverse", "--topo-order", range, "--"],
            )?
            .lines()
            .map(|commit| (commit.to_string(), None))
            .collect()
        } else {
            selection.commits.iter().map(|commit| (commit.clone(), None)).collect()
        };
        if commits.is_empty() {
            return Err("Source selection contains no commits".into());
        }
        for (requested, squash_base) in commits {
            if requested.starts_with('-') {
                return Err("Invalid source revision".into());
            }
            let commit = git_text(
                &root,
                &["rev-parse", "--verify", &format!("{requested}^{{commit}}")],
            )?;
            if !seen.insert(commit.clone()) {
                return Err(format!("Commit selected more than once: {commit}"));
            }
            let parents = git_text(&root, &["rev-list", "--parents", "-n", "1", &commit])?;
            let parents: Vec<_> = parents.split_whitespace().skip(1).collect();
            let parent = if let Some(base) = squash_base {
                base
            } else if parents.is_empty() {
                git_input(
                    &root,
                    &["hash-object", "-t", "tree", "-w", "--stdin"],
                    b"",
                    None,
                )?
            } else if parents.len() == 1 {
                parents[0].to_string()
            } else {
                let n = selection
                    .mainline
                    .get(&requested)
                    .or_else(|| selection.mainline.get(&commit))
                    .ok_or_else(|| {
                        format!(
                            "Merge commit {commit} requires an explicit one-based mainline parent"
                        )
                    })?;
                parents
                    .get(n.checked_sub(1).ok_or("mainline must be at least 1")?)
                    .ok_or("mainline parent out of range")?
                    .to_string()
            };
            let changed = git(
                &root,
                &[
                    "diff-tree",
                    "--no-commit-id",
                    "--name-only",
                    "--no-renames",
                    "-r",
                    "-z",
                    &parent,
                    &commit,
                    "--",
                ],
            )?;
            git(&root, &["update-ref", &format!("refs/locus/merge-jobs/{id}/source-{}", seen.len()), &commit])?;
            let mut changed_paths: Vec<_> = changed
                .split(|b| *b == 0)
                .filter(|v| !v.is_empty())
                .map(|raw| {
                    String::from_utf8(raw.to_vec())
                        .map_err(|_| "Non-UTF8 repository paths are not supported")
                })
                .collect::<Result<_, _>>()?;
            if let Some(selected) = &selected_paths {
                changed_paths.retain(|p| selected.contains(p));
                // Whole-file integration may choose a file unchanged by this
                // commit's parent delta. Pin its exact source version as well.
                changed_paths.extend(selected.clone());
                changed_paths.sort(); changed_paths.dedup();
            }
            let (bases, sources) = parallel::pool().install(|| rayon::join(
                || tree_files(&root, &dir, &parent, &changed_paths),
                || tree_files(&root, &dir, &commit, &changed_paths)));
            let (bases, sources) = (bases?, sources?);
            for path in changed_paths {
                safe_path(&root, &path)?;
                let base = bases[&path].clone();
                let source = sources[&path].clone();
                let bytes = state_bytes(&dir, &source)?;
                let base_bytes = state_bytes(&dir, &base)?;
                let kind = if opaque(&path)
                    || is_lfs_pointer(&bytes)
                    || is_lfs_pointer(&base_bytes)
                    || bytes.contains(&0)
                    || base_bytes.contains(&0)
                    || std::str::from_utf8(&bytes).is_err()
                {
                    "binary"
                } else if unity_yaml(&path, &bytes) || unity_yaml(&path, &base_bytes) {
                    "unity_yaml"
                } else {
                    "text"
                };
                if path_in_project(&root, &scope_root, &path) {
                    paths.insert(path.clone());
                }
                deltas.push(CommitDelta {
                    commit: commit.clone(),
                    parent: parent.clone(),
                    source_branch: selection.branch_ref.clone(),
                    path,
                    base,
                    source,
                    kind: kind.into(),
                    changes: vec![],
                });
            }
        }
    }
    timings.insert("git_sources_ms".into(), json!(started.elapsed().as_millis()));
    let snapshot_started = Instant::now();
    // Writable targets (changed paths, plus the destination's dirty and
    // untracked paths for a full snapshot) keep their raw bytes. Every other
    // Unity input is a read-only dependency pinned to its HEAD blob, so a
    // large project is not copied. Ignored caches never enter the job and
    // unrelated paths are never written on apply.
    let (dirty, untracked) = parallel::pool().install(|| rayon::join(
        || snapshot::names(&root, &["diff", "--name-only", "-z", "HEAD", "--"]),
        || snapshot::names(&root, &["ls-files", "--others", "--exclude-standard", "-z"])));
    let (dirty, untracked) = (dirty?, untracked?);
    if let Some(selected) = &selected_paths {
        paths.extend(selected.iter().cloned());
    } else {
        paths.extend(dirty.iter().chain(&untracked)
            .filter(|p| path_in_project(&root, &scope_root, p)).cloned());
    }
    let index_modes: BTreeMap<_, _> = index_records
        .split('\0')
        .filter_map(|record| {
            let (header, path) = record.split_once('\t')?;
            Some((
                path.to_string(),
                header.split_whitespace().next()?.to_string(),
            ))
        })
        .collect();
    let (files, dependency_files) = parallel::pool().install(|| rayon::join(
        || snapshot::capture_paths(&root, &dir, &paths, &index_modes),
        || snapshot::dependencies(&root, &scope_root, &dir, &head, &paths, &dirty, &untracked, &index_modes, &index_flags)));
    let (files, (dependency_files, dependency_policy)) = (files?, dependency_files?);
    if let Some(requested) = &request.paths {
        if !requested.iter().any(|path| files.get(path).and_then(Option::as_ref).is_some()
            || deltas.iter().any(|d|d.path==*path && (d.base.is_some() || d.source.is_some()))) {
            return Err("All selected files are absent from the target and source versions".into());
        }
    }
    timings.insert("snapshot_ms".into(), json!(snapshot_started.elapsed().as_millis()));
    let materialization_epoch = crate::workspace_service::worktrees::record_for_root(project_root)?
        .map(|r| r.materialization_epoch)
        .unwrap_or(0);
    let mut job = MergeJob {
        id,
        version: journal::VERSION,
        root: root.to_string_lossy().into_owned(),
        project_root: dunce::canonicalize(project_root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned(),
        project_id: request.project_id.clone(),
        snapshot: TargetSnapshot {
            materialization_epoch,
            head,
            branch,
            index_records: index_records.into(),
            index_flags: index_flags.into(),
            files: files.into(),
        },
        deltas: deltas.into(),
        paths: selected_paths,
        mode: request.mode,
        dependency_files: dependency_files.into(),
        dependency_policy,
        catalog_ready: false,
        prepare_metrics: Value::Null,
        schemas: Default::default(),
        selection: Selection::default(),
        revision: 0,
        state: "prepared".into(),
        applied_hash: None,
        applied_files: BTreeMap::new(),
        applied_before: BTreeMap::new(),
        staged_index_records: None,
        staged_index_flags: None,
        unity_validated_hash: None,
        commit_oid: None,
        pending_index_before: None,
        pending_index_after: None,
        commit_validations: BTreeMap::new(),
    };
    let catalog_started = Instant::now();
    if request.eager_catalog || request.mode == PrepareMode::Files {
        ensure_catalog(&dir, &mut job)?;
    }
    timings.insert("catalog_ms".into(), json!(catalog_started.elapsed().as_millis()));
    let verify_started = Instant::now();
    check_target(&root, &dir, &job)?;
    timings.insert("verify_ms".into(), json!(verify_started.elapsed().as_millis()));
    job.prepare_metrics = json!({"phases":timings,"target_files":job.snapshot.files.len(),
        "dependency_files":job.dependency_files.len(), "git_backed_dependencies":job.dependency_files.values().flatten().filter(|s|git_blob_oid(s).is_some()).count(),
        "workers":parallel::pool().current_num_threads(), "total_ms":started.elapsed().as_millis()});
    save(&dir, &job)?;
    guard.disarm();
    tracing::info!(target: "locus::merge_jobs", job_id = %job.id, metrics = %job.prepare_metrics, "Merge prepare completed");
    Ok(job)
}

/// An explicit destination branch can inherit a dirty base without a stash or
/// user-visible snapshot commit. HEAD, index and disk contents remain separate.
/// Without an explicit `path`, a new worktree goes to the policy's container.
pub fn prepare_with_destination(
    project_root: &Path,
    request: &PrepareRequest,
    destination: &Value,
    policy: &crate::workspace_service::worktrees::location::WorktreePolicy,
) -> Result<MergeJob, String> {
    let kind = destination
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("checkout");
    if kind == "checkout" {
        return prepare(project_root, request);
    }
    if kind != "new_branch" {
        return Err("Destination kind must be checkout or new_branch".into());
    }
    let name = param_str(destination, "name")?;
    let root = repository(project_root)?;
    git(&root, &["check-ref-format", "--branch", name])?;
    let location = destination
        .get("location")
        .and_then(Value::as_str)
        .unwrap_or("new_worktree");
    if location == "current_checkout" {
        git(&root, &["switch", "-c", name])?;
        return prepare(project_root, request);
    }
    if location != "new_worktree" {
        return Err("New branch location must be current_checkout or new_worktree".into());
    }
    // A new checkout must inherit every dirty path even when the final merge is scoped.
    let mut base_request = request.clone();
    base_request.paths = None;
    base_request.mode = PrepareMode::Structural;
    base_request.eager_catalog = false;
    let base = prepare(project_root, &base_request)?;
    let base_dir = job_dir(&root, &base.id)?;
    check_target(&root, &base_dir, &base)?;
    let destination_path = match destination.get("path").and_then(Value::as_str) {
        Some(path) => std::path::PathBuf::from(path),
        None => policy.new_checkout(&root, "merge")?,
    };
    let record = crate::workspace_service::worktrees::create(
        &crate::workspace_service::worktrees::CreateWorktreeRequest {
            source_root: project_root.to_string_lossy().into_owned(),
            destination: destination_path.to_string_lossy().into_owned(),
            branch: name.into(),
            start_ref: Some(base.snapshot.head.clone()),
            include_dirty: false,
        },
    )?;
    let target = Path::new(&record.repo_root);
    // Source blobs are in the shared object database. Empty the new checkout's
    // index and reconstruct the exact stage records, including staged deletions.
    git_input(target, &["read-tree", "--empty"], b"", None)?;
    git_input(
        target,
        &["update-index", "-z", "--index-info"],
        base.snapshot.index_records.as_bytes(),
        None,
    )?;
    for flag in base
        .snapshot
        .index_flags
        .split('\0')
        .filter(|s| !s.is_empty())
    {
        if flag.len() < 3 {
            continue;
        }
        let marker = flag.as_bytes()[0];
        let path = &flag[2..];
        if marker.is_ascii_lowercase() {
            git(target, &["update-index", "--assume-unchanged", "--", path])?;
        }
        if marker.to_ascii_uppercase() == b'S' {
            git(target, &["update-index", "--skip-worktree", "--", path])?;
        }
    }
    for (path, state) in &base.snapshot.files {
        write_file(target, &base_dir, path, state)?;
    }
    // Retain the frozen source journal for provenance but mark its no-write plan
    // aborted. The final job is reoriented to the new branch and its own snapshot.
    execute(project_root, &base.id, "abort", json!({}))?;
    prepare(Path::new(&record.root), request)
}

fn check_target(root: &Path, dir: &Path, job: &MergeJob) -> Result<(), String> {
    check_destination(root, job)?;
    let epoch = crate::workspace_service::worktrees::record_for_root(Path::new(&job.project_root))?
        .map(|r| r.materialization_epoch)
        .unwrap_or(0);
    if epoch != job.snapshot.materialization_epoch {
        return Err("stale: destination pool assignment changed".into());
    }
    if git_text(root, &["rev-parse", "HEAD"])? != job.snapshot.head {
        return Err("stale: destination HEAD changed".into());
    }
    if git_text(root, &["symbolic-ref", "--quiet", "HEAD"]).ok() != job.snapshot.branch {
        return Err("stale: destination branch changed".into());
    }
    let (records, flags) = index_entries(root)?;
    if records != *job.snapshot.index_records || flags != *job.snapshot.index_flags {
        return Err("stale: destination index changed".into());
    }
    check_files(root, dir, &job.snapshot.files)?;
    snapshot::check_dependencies(root, dir, job)
}

fn check_destination(root: &Path, job: &MergeJob) -> Result<(), String> {
    let epoch = crate::workspace_service::worktrees::record_for_root(Path::new(&job.project_root))?
        .map(|r| r.materialization_epoch)
        .unwrap_or(0);
    if epoch != job.snapshot.materialization_epoch {
        return Err("stale: destination assignment changed".into());
    }
    if git_text(root, &["symbolic-ref", "--quiet", "HEAD"]).ok() != job.snapshot.branch {
        return Err("stale: destination branch changed; prepare a new job for this branch".into());
    }
    Ok(())
}

fn resolution_name(value: &Value) -> &str {
    value
        .as_str()
        .or_else(|| value.get("kind").and_then(Value::as_str))
        .unwrap_or("include")
}
fn core_decision(id: &str, decision: &Value) -> Result<crate::unity_asset_core::Decision, String> {
    let resolution = if decision.is_string() {
        json!({"kind":decision})
    } else {
        decision.clone()
    };
    serde_json::from_value(json!({"change_id":id,"resolution":resolution}))
        .map_err(|e| format!("Invalid structural resolution: {e}"))
}

pub fn preview(root: &Path, id: &str) -> Result<Preview, String> {
    let job = load(root, id)?;
    preview_job(&job_dir(Path::new(&job.root), id)?, &job)
}
/// A preview is a pure function of the job record and its immutable,
/// content-addressed blobs. An agent previews, then applies, and apply and the
/// Editor preflight each preview again: the latest result is kept per job so an
/// unchanged plan renders once. Any change to the record misses the cache.
fn preview_job(dir: &Path, job: &MergeJob) -> Result<Preview, String> {
    let key = journal::fingerprint(dir, job)?;
    let cache = dir.join("preview-latest.json");
    if let Some(preview) = std::fs::read(&cache)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<(String, Preview)>(&bytes).ok())
        .and_then(|(cached, preview)| (cached == key).then_some(preview))
    {
        return Ok(preview);
    }
    let preview = compute_preview(dir, job)?;
    if depends_on_lfs_cache(&preview) {
        return Ok(preview);
    }
    // Best effort: a failed cache write only costs the next render.
    if let Ok(mut temp) = tempfile::NamedTempFile::new_in(dir) {
        use std::io::Write;
        if serde_json::to_vec(&(&key, &preview))
            .ok()
            .is_some_and(|bytes| temp.write_all(&bytes).is_ok())
        {
            let _ = temp.persist(&cache);
        }
    }
    Ok(preview)
}

/// Remote for `git lfs fetch <remote> <commit>`, which must be named when a
/// commit is given: `origin`, else the only remote, else a placeholder.
fn lfs_fetch_remote(root: &Path) -> String {
    let remotes = git_text(root, &["remote"]).unwrap_or_default();
    let remotes: Vec<_> = remotes.lines().map(str::trim).filter(|r| !r.is_empty()).collect();
    if remotes.contains(&"origin") {
        "origin".into()
    } else if let [only] = remotes.as_slice() {
        (*only).into()
    } else {
        "<remote>".into()
    }
}

/// One command downloads every missing object of `commit` (None: the current
/// checkout) into the LFS cache. Paths that cannot be passed literally as a
/// comma-separated, single-quoted `--include` list fetch the whole commit.
fn lfs_fetch_command(remote: &str, commit: Option<&str>, paths: &BTreeSet<String>) -> String {
    let mut command = match commit {
        Some(commit) => format!("git lfs fetch {remote} {commit}"),
        None => "git lfs fetch".into(),
    };
    if paths
        .iter()
        .all(|path| !path.contains([',', '\'', '*', '?', '[', '\\']))
    {
        let include = paths.iter().map(String::as_str).collect::<Vec<_>>().join(",");
        command.push_str(&format!(" --include '{include}'"));
    }
    command
}

/// Whether a preview depends on the local LFS cache rather than only the job,
/// so it must be recomputed after fetching instead of served from cache.
fn depends_on_lfs_cache(preview: &Preview) -> bool {
    preview
        .issues
        .iter()
        .any(|issue| issue["code"] == "lfs_object_missing")
}

fn compute_preview(dir: &Path, job: &MergeJob) -> Result<Preview, String> {
    let mut result = job.snapshot.files.to_inner();
    let mut issues = vec![];
    let mut excluded = vec![];
    let mut deferred = vec![];
    let mut result_schema_cache: Option<schema::SchemaMap> = None;
    let mut rendered_fields: BTreeSet<(String, String)> = BTreeSet::new();
    for delta in job
        .deltas
        .iter()
        .filter(|d| d.kind == "text")
        .chain(job.deltas.iter().filter(|d| d.kind != "text"))
    {
        let object_choices: Vec<_> = job
            .selection
            .objects
            .values()
            .filter(|o| o.path == delta.path && o.commit == delta.commit)
            .collect();
        let mut selected = vec![];
        for change in &delta.changes {
            let id = change["id"].as_str().ok_or("Invalid catalog ID")?;
            match job.selection.decisions.get(id) {
                None => excluded.push(id.into()),
                Some(d) if resolution_name(d) == "exclude" || resolution_name(d) == "target" => {
                    excluded.push(id.into())
                }
                Some(d) if resolution_name(d) == "defer" => deferred.push(id.into()),
                Some(d) => selected.push((change, d)),
            }
        }
        if selected.is_empty() && object_choices.is_empty() {
            continue;
        }
        if !scope_allows(job, &delta.path) {
            issues.push(error_value("outside_destination_scope",&delta.path,"This change is outside the destination Unity project root; use an explicitly authorized repository-root workspace or another job"));
            continue;
        }
        if let Some(choice) = job.selection.files.get(&delta.path) {
            // Supplied merged text is the explicit resolution of this file's
            // text change; it supersedes include/resolve for the same path.
            if choice.version == "content" && delta.kind == "text" {
                continue;
            }
            let ids: Vec<_> = selected.iter().map(|(change, _)| change["id"].clone()).collect();
            issues.push(json!({"code":"overlapping_selection","path":delta.path,"change_ids":ids,
                "file_operation":choice.version,
                "detail":"A whole-file operation and interior change selections overlap; files.clear the file operation or exclude these change_ids"}));
            continue;
        }
        let target = result.get(&delta.path).cloned().unwrap_or(None);
        if delta.kind == "binary" {
            issues.push(error_value(
                "binary_choice_required",
                &delta.path,
                "Use files.take with a complete target/source/base version and source commit",
            ));
            continue;
        }
        if selected
            .iter()
            .any(|(change, _)| change["kind"] == "file_mode")
        {
            issues.push(error_value("file_mode_choice_required",&delta.path,
                "Mode changes require an explicit complete files.take version; exclude the mode item to retain destination mode while merging fields"));
            selected.retain(|(change, _)| change["kind"] != "file_mode");
            if selected.is_empty() && object_choices.is_empty() {
                continue;
            }
        }
        if selected.iter().any(|(c, _)| c["core_id"].is_null()) && object_choices.is_empty() {
            let (_, decision) = selected[0];
            let name = resolution_name(decision);
            if delta.kind == "unity_yaml" && name == "include" {
                let source_bytes = state_bytes(dir, &delta.source)?;
                let is_added = delta.base.is_none() && target.is_none();
                let is_removed = delta.source.is_none() && target == delta.base;
                let valid_added = is_added
                    && crate::unity_asset_core::parse(&source_bytes)
                        .map(|a| a.is_writable())
                        .unwrap_or(false);
                if !valid_added && !is_removed {
                    issues.push(error_value("asset_choice_required",&delta.path,"Structural parsing could not prove coverage; select the complete file version explicitly"));
                    continue;
                }
            }
            let output = match name {
                "source" => Some(delta.source.clone()),
                "base" => Some(delta.base.clone()),
                "include" if target == delta.base || target == delta.source => {
                    Some(delta.source.clone())
                }
                "include" if delta.kind == "text" => {
                    let (base_bytes, target_bytes, source_bytes) = (
                        state_bytes(dir, &delta.base)?,
                        state_bytes(dir, &target)?,
                        state_bytes(dir, &delta.source)?,
                    );
                    let change_id = selected[0].0["id"].clone();
                    let present = (delta.base.is_some(), target.is_some(), delta.source.is_some());
                    match present {
                        (true, true, true) => match text_merge::merge(&base_bytes, &target_bytes, &source_bytes) {
                            Ok(outcome) => match outcome.bytes(&target_bytes) {
                                Some(bytes) => Some(Some(store_blob(
                                    dir,
                                    &bytes,
                                    target.as_ref().map(|s| s.mode.as_str()).unwrap_or("100644"),
                                )?)),
                                None => {
                                    let hunks = match outcome {
                                        text_merge::Outcome::Conflict { hunks, .. } => hunks,
                                        _ => vec![],
                                    };
                                    issues.push(json!({"code":"text_conflict","path":delta.path,"change_id":change_id,
                                        "detail":format!("{} overlapping line region(s) changed on both sides; inspect_asset(path, version='result') shows conflict markers; resolve with files.set_text or resolve side='source'/'target'", hunks.len()),
                                        "hunks":hunks}));
                                    None
                                }
                            },
                            Err(reason) => {
                                issues.push(json!({"code":"text_conflict","path":delta.path,"change_id":change_id,"detail":reason}));
                                None
                            }
                        },
                        (true, true, false) if eol_equal(&target_bytes, &base_bytes) => Some(None),
                        (false, true, true) if eol_equal(&target_bytes, &source_bytes) => Some(target.clone()),
                        _ => {
                            let reason = file_status(delta, &target, &base_bytes, &target_bytes, &source_bytes, None).1;
                            issues.push(json!({"code":"text_conflict","path":delta.path,"change_id":change_id,
                                "detail":reason["reason"].as_str().unwrap_or("the destination differs from this commit's parent")}));
                            None
                        }
                    }
                }
                _ => {
                    issues.push(json!({"code":"file_conflict","path":delta.path,"change_id":selected[0].0["id"],
                        "detail":"Select source/base/target explicitly; the destination differs from this commit's parent"}));
                    None
                }
            };
            if let Some(mut output) = output {
                // Including content keeps the destination's mode. A chmod has
                // its own explicit whole-file decision, even for clean text.
                if name == "include" {
                    if let (Some(state), Some(target)) = (&mut output, &target) {
                        state.mode = target.mode.clone();
                    }
                }
                result.insert(delta.path.clone(), output);
            }
            continue;
        }
        let base = state_bytes(dir, &delta.base)?;
        let source = state_bytes(dir, &delta.source)?;
        let current = state_bytes(dir, &target)?;
        // The destination never changed this file and every source change is
        // included: the three-way result is the source file, byte for byte.
        // Rendering it from the destination could only reorder or respell it.
        if object_choices.is_empty()
            && current == base
            && selected.len() == delta.changes.len()
            && selected.iter().all(|(_, decision)| resolution_name(decision) == "include")
        {
            let output = delta.source.clone().map(|mut state| {
                if let Some(target) = &target {
                    state.mode = target.mode.clone();
                }
                state
            });
            result.insert(delta.path.clone(), output);
            continue;
        }
        let empty = schema::SchemaMap::new();
        if result_schema_cache.is_none() {
            let mut schema_files = job.dependency_files.to_inner();
            schema_files.extend(result.clone());
            for choice in job
                .selection
                .files
                .values()
                .filter(|c| c.path.ends_with(".cs") || c.path.ends_with(".cs.meta"))
            {
                let value = match choice.version.as_str() {
                    "target" => job
                        .snapshot
                        .files
                        .get(&choice.path)
                        .cloned()
                        .unwrap_or(None),
                    "delete" => None,
                    "source" | "base" => {
                        let candidates: Vec<_> = job
                            .deltas
                            .iter()
                            .filter(|d| {
                                d.path == choice.path
                                    && choice
                                        .commit
                                        .as_ref()
                                        .map(|c| c == &d.commit)
                                        .unwrap_or(true)
                            })
                            .collect();
                        if candidates.len() != 1 {
                            continue;
                        }
                        if choice.version == "source" {
                            candidates[0].source.clone()
                        } else {
                            candidates[0].base.clone()
                        }
                    }
                    "move" => {
                        if let Some(destination) = &choice.destination {
                            schema_files.insert(
                                destination.clone(),
                                job.snapshot
                                    .files
                                    .get(&choice.path)
                                    .cloned()
                                    .unwrap_or(None),
                            );
                        }
                        None
                    }
                    _ => continue,
                };
                schema_files.insert(choice.path.clone(), value);
            }
            let guids = job.schemas.values().flat_map(|schema|schema.keys()).cloned().collect();
            result_schema_cache = Some(schema::working_for_guids(dir, &schema_files, &guids)?);
        }
        let result_schema = result_schema_cache.as_ref().unwrap();
        let aliases = schema::aliases(
            &current,
            job.schemas.get(&delta.parent).unwrap_or(&empty),
            job.schemas.get(&delta.commit).unwrap_or(&empty),
            result_schema,
        );
        match merge_session(&base, &current, &source, &aliases) {
            Ok(session) => {
                // A later commit of this file was cataloged against the file as
                // its earlier selected commits leave it when included in full.
                // Other decisions for those commits can change what it meets.
                let known: BTreeSet<&str> = session.catalog().changes.iter().map(|c| c.id.as_str()).collect();
                let moved: Vec<Value> = selected
                    .iter()
                    .filter(|(change, _)| change["core_id"].as_str().is_some_and(|id| !known.contains(id)))
                    .map(|(change, _)| change["id"].clone())
                    .collect();
                if !moved.is_empty() {
                    let earlier: Vec<&str> = job.deltas.iter()
                        .take_while(|d| !std::ptr::eq(*d, delta))
                        .filter(|d| d.path == delta.path)
                        .map(|d| d.commit.as_str())
                        .collect();
                    issues.push(json!({"code":"earlier_commit_decisions","path":delta.path,"commit":delta.commit,
                        "change_ids":moved,"earlier_commits":earlier,
                        "detail":"This commit's changes were listed against the file as the earlier selected commits leave it when included in full; the current decisions for those commits change the file, so these changes no longer apply as listed. Include the earlier commits' changes for this file, or choose the complete file with files.take"}));
                    continue;
                }
                // Field operations bound to this commit that refine no selected
                // change render with it, so their combined result is validated
                // once (e.g. a restored component and its owner's listing).
                let fields: Vec<&FieldChoice> = job.selection.fields.values()
                    .filter(|f| f.path == delta.path && f.commit.as_deref() == Some(delta.commit.as_str()))
                    .collect();
                let independent = !fields.is_empty() && fields.iter().all(|f| {
                    !job.selection.objects.values().any(|o| o.path == delta.path && o.object_id == f.object_id)
                        && !selected.iter().any(|(c, _)| c["object_id"].as_str() == Some(f.object_id.as_str())
                            && transforms::overlaps(c["property_path"].as_str().unwrap_or(""), &f.property_path))
                });
                let mut decisions = vec![];
                if independent {
                    for field in &fields {
                        let resolution = serde_json::from_value(field.resolution.clone()).map_err(|e| e.to_string())?;
                        decisions.push(crate::unity_asset_core::MergeSession::field_decision(&field.object_id, &field.property_path, resolution));
                    }
                    rendered_fields.insert((delta.path.clone(), delta.commit.clone()));
                }
                for (change, decision) in selected {
                    decisions.push(core_decision(
                        change["core_id"].as_str().ok_or("Missing core ID")?,
                        decision,
                    )?);
                }
                for choice in object_choices {
                    if choice.operation == "add"
                        && (session.target_asset().document(&choice.object_id).is_some()
                            || session.source_asset().document(&choice.object_id).is_none())
                    {
                        issues.push(error_value(
                            "object_add_precondition",
                            &delta.path,
                            "Object add requires an absent target ID and a present source object",
                        ));
                        continue;
                    }
                    if choice.operation == "delete"
                        && session.target_asset().document(&choice.object_id).is_none()
                    {
                        issues.push(error_value(
                            "object_delete_precondition",
                            &delta.path,
                            "Object delete requires an existing target object",
                        ));
                        continue;
                    }
                    decisions.push(core_decision(
                        &format!("$object:{}", choice.object_id),
                        &choice.resolution,
                    )?);
                }
                match session.render(&decisions) {
                    Ok(output) => {
                        if output.ready {
                            result.insert(
                                delta.path.clone(),
                                Some(store_blob(
                                    dir,
                                    &output.bytes,
                                    target.as_ref().map(|s| s.mode.as_str()).unwrap_or("100644"),
                                )?),
                            );
                        } else {
                            let conflicts: Vec<Value> = output
                                .conflicts
                                .iter()
                                .map(|conflict| {
                                    let mut value = json!(conflict);
                                    if !conflict.change_id.is_empty()
                                        && !conflict.change_id.starts_with('$')
                                    {
                                        value["change_id"] = json!(namespaced(
                                            &delta.commit,
                                            &delta.path,
                                            &conflict.change_id
                                        ));
                                    } else if conflict.change_id.is_empty() && !conflict.object_id.is_empty() {
                                        // A validation finding names an object, not a change:
                                        // list the catalog changes of that object or of
                                        // references to it, which are the ones to decide.
                                        let reference = format!("={}", conflict.object_id);
                                        let related: Vec<&Value> = delta.changes.iter()
                                            .filter(|c| c["object_id"].as_str() == Some(conflict.object_id.as_str())
                                                || c["property_path"].as_str().is_some_and(|p| p.ends_with(&reference)))
                                            .map(|c| &c["id"])
                                            .collect();
                                        value["related_change_ids"] = json!(related);
                                    }
                                    value
                                })
                                .collect();
                            let mut change_ids: Vec<String> = vec![];
                            for conflict in &conflicts {
                                let direct = conflict["change_id"].as_str().filter(|id| id.contains(':'));
                                let related = conflict["related_change_ids"].as_array().into_iter().flatten().filter_map(Value::as_str);
                                for id in direct.into_iter().chain(related) {
                                    if !change_ids.iter().any(|known| known == id) {
                                        change_ids.push(id.to_string());
                                    }
                                }
                            }
                            issues.push(json!({"code":"asset_conflict","path":delta.path,"commit":delta.commit,
                                "change_ids":change_ids,"conflicts":conflicts,"diagnostics":output.diagnostics,
                                "detail":"resolve each listed change with plan.resolve(change_id, side=...) or a typed field operation"}));
                        }
                    }
                    Err(e) => issues.push(error_value("asset_render", &delta.path, e.to_string())),
                }
            }
            Err(e) => issues.push(error_value("asset_parse", &delta.path, e.to_string())),
        }
    }
    // (issue index, commit, path) of whole-file versions whose LFS object is not cached.
    let mut missing_lfs: Vec<(usize, Option<String>, String)> = vec![];
    for choice in job.selection.files.values() {
        // Commit whose tree holds a source/base version, for fetching its LFS objects.
        let mut version_commit = None;
        if !scope_allows(job, &choice.path)
            || choice
                .destination
                .as_ref()
                .map(|path| !scope_allows(job, path))
                .unwrap_or(false)
        {
            issues.push(error_value(
                "outside_destination_scope",
                &choice.path,
                "File operation escapes the destination project root",
            ));
            continue;
        }
        let output = match choice.version.as_str() {
            "target" => job
                .snapshot
                .files
                .get(&choice.path)
                .cloned()
                .unwrap_or(None),
            "delete" => None,
            "content" => Some(
                choice
                    .content
                    .clone()
                    .ok_or("Supplied text content is missing from the plan; call files.set_text again")?,
            ),
            "move" => job
                .snapshot
                .files
                .get(&choice.path)
                .cloned()
                .ok_or("File move source was not captured; prepare with it in scope")?,
            "source" | "base" => {
                let candidates: Vec<_> = job
                    .deltas
                    .iter()
                    .filter(|d| {
                        d.path == choice.path
                            && choice
                                .commit
                                .as_ref()
                                .map(|c| c == &d.commit)
                                .unwrap_or(true)
                    })
                    .collect();
                if candidates.len() != 1 {
                    issues.push(error_value(
                        "ambiguous_version",
                        &choice.path,
                        "Specify one full selected commit OID for this file version",
                    ));
                    continue;
                }
                if choice.version == "source" {
                    version_commit = Some(candidates[0].commit.clone());
                    candidates[0].source.clone()
                } else {
                    version_commit = Some(candidates[0].parent.clone());
                    candidates[0].base.clone()
                }
            }
            _ => return Err("Unsupported whole-file version".into()),
        };
        let output = match output
            .as_ref()
            .map(|state| materialize_lfs(Path::new(&job.root), dir, state))
            .transpose()?
        {
            None => None,
            Some(Materialized::Complete(state)) => Some(state),
            Some(Materialized::Missing { oid, size }) => {
                // Report and keep going so one preview lists every missing object;
                // the pointer bytes are never planned.
                let detail = if choice.version == "content" {
                    "Supplied content is a Git LFS pointer whose object is not in the local LFS cache; supply the complete file content instead"
                } else {
                    missing_lfs.push((issues.len(), version_commit.clone(), choice.path.clone()));
                    "The complete Git LFS object for this file version is not in the local LFS cache; run each distinct fetch_command once (it only fills the LFS cache and leaves the working tree untouched), then preview again"
                };
                issues.push(json!({"code":"lfs_object_missing","path":choice.path,
                    "version":choice.version,"commit":version_commit,"oid":oid,"size":size,
                    "fetch_command":null,"detail":detail}));
                continue;
            }
        };
        if choice.version == "move" {
            let destination = choice
                .destination
                .as_ref()
                .ok_or("Move requires destination")?;
            safe_path(Path::new(&job.root), destination)?;
            if !job.snapshot.files.contains_key(destination) {
                return Err(
                    "Move destination was not snapshotted; add it with files.move before preview"
                        .into(),
                );
            }
            if job
                .snapshot
                .files
                .get(destination)
                .and_then(|s| s.as_ref())
                .is_some()
            {
                issues.push(error_value(
                    "move_destination_exists",
                    destination,
                    "Move cannot overwrite an existing destination",
                ));
                continue;
            }
            result.insert(destination.clone(), output);
            result.insert(choice.path.clone(), None);
        } else {
            result.insert(choice.path.clone(), output);
        }
    }
    if !missing_lfs.is_empty() {
        let mut by_commit: BTreeMap<Option<String>, BTreeSet<String>> = BTreeMap::new();
        for (_, commit, path) in &missing_lfs {
            by_commit.entry(commit.clone()).or_default().insert(path.clone());
        }
        let remote = lfs_fetch_remote(Path::new(&job.root));
        for (index, commit, _) in &missing_lfs {
            issues[*index]["fetch_command"] =
                json!(lfs_fetch_command(&remote, commit.as_deref(), &by_commit[commit]));
        }
    }
    transforms::apply(dir, job, &mut result, &mut issues, &rendered_fields)?;
    let changed: BTreeMap<_, _> = result
        .iter()
        .filter(|(p, s)| job.snapshot.files.get(*p) != Some(*s))
        .map(|(p, s)| (p.clone(), s.clone()))
        .collect();
    if !changed.is_empty() { dependency_issues(dir, job, &result, &changed, &mut issues)?; }
    let hash = blake3::hash(
        &serde_json::to_vec(&(
            job.id.as_str(),
            job.revision,
            &job.selection,
            &changed,
            &issues,
        ))
        .map_err(|e| e.to_string())?,
    )
    .to_hex()
    .to_string();
    Ok(Preview {
        job_id: job.id.clone(),
        revision: job.revision,
        plan_hash: hash.clone(),
        files: changed,
        ready_to_apply: issues.is_empty(),
        issues,
        excluded_changes: excluded,
        deferred_changes: deferred,
        unity_validated: job.unity_validated_hash.as_deref() == Some(&hash),
    })
}

fn dependency_issues(
    dir: &Path,
    job: &MergeJob,
    result: &BTreeMap<String, Option<FileState>>,
    changed: &BTreeMap<String, Option<FileState>>,
    issues: &mut Vec<Value>,
) -> Result<(), String> {
    let target = snapshot::all_files(job);
    let mut full_result = job.dependency_files.clone();
    full_result.extend(result.clone());
    let result = &full_result;
    let root = Path::new(&job.root);
    let target_guids = dependencies::owners(dir, &target)?;
    let guids = dependencies::owners(dir, result)?;
    issues.extend(dependencies::removed_object_issues(
        root, dir, &target, result, changed, &target_guids, &guids,
    )?);
    for (guid, owners) in &guids {
        if owners.len() > 1 && target_guids.get(guid) != Some(owners) {
            issues.push(error_value(
                "duplicate_guid",
                &owners[0],
                format!("{guid} has multiple owners: {}", owners.join(", ")),
            ));
        }
    }
    let removed_guids: BTreeSet<_> = target_guids
        .keys()
        .filter(|guid| !guids.contains_key(*guid))
        .cloned()
        .collect();
    // A changed/deleted meta can break an UNCHANGED asset, but only one that
    // spells a removed GUID: those few are found by a budgeted byte search and
    // only they are parsed. Changed assets are always checked for new
    // references. Compact blob summaries are reused across jobs and previews.
    let matcher = dependencies::GuidMatcher::new(&removed_guids);
    let mut checked: Vec<(&String, dependencies::Mention)> = Vec::new();
    for (path, state) in changed {
        let Some(state) = state.as_ref().filter(|_| dependencies::reference_candidate(path)) else {
            continue;
        };
        match dependencies::dependency_bytes(root, dir, state)? {
            Ok((state, bytes)) if unity_yaml(path, &bytes) => {
                checked.push((path, dependencies::Mention::Spells(state)))
            }
            Ok(_) => {}
            Err(reason) => checked.push((path, dependencies::Mention::Unreadable(reason))),
        }
    }
    if !matcher.is_empty() {
        prefetch_git(dir, result.iter().filter(|(p,_)| snapshot::evidence(p)
            && !p.ends_with(".cs") && !p.ends_with(".meta") && !p.ends_with(".asmdef"))
            .filter_map(|(_,s)|s.as_ref()))?;
        checked.extend(dependencies::spelling_assets(
            root, dir, result, &|path| !changed.contains_key(path), &matcher,
        )?);
        checked.sort_by(|a, b| a.0.cmp(b.0));
    }
    let check = |path: &String, found: dependencies::Mention| -> Result<Vec<Value>, String> {
        let state = match found {
            dependencies::Mention::Spells(state) => state,
            dependencies::Mention::Unreadable(reason) if !matcher.is_empty() => {
                return Ok(vec![error_value("dependency_coverage", path,
                    format!("Cannot check this asset's references while a GUID is removed: {reason}"))]);
            }
            dependencies::Mention::Unreadable(_) => return Ok(vec![]),
        };
        let references = dependencies::references(dir, &state)?;
        let mut found = Vec::new();
        // GUIDs named only in `guid` keys the lexical scan could not resolve.
        let unresolved = references.unresolved(&removed_guids, || {
            parallel::with_bytes(blob_len(dir, &state).unwrap_or(0), || read_blob(dir, &state))
                .map(|bytes| matcher.mentioned(&bytes))
        })?;
        if !unresolved.is_empty() {
            found.push(json!({"code":"dependency_coverage","path":path,"guids":unresolved,
                "detail":"This asset names a GUID whose asset/meta is being removed in a `guid` key outside a recognizable `{fileID, guid}` reference, so its references cannot be checked; keep the removed asset/meta, or take a version of this asset in Unity's text format"}));
        }
        let old_guids = match target.get(path).and_then(|s| s.as_ref()) {
            Some(state) => dependencies::references(dir, state)?.guids,
            None => BTreeSet::new(),
        };
        found.extend(
            references
                .guids
                .iter()
                .filter(|guid| {
                    !(guid.starts_with("0000000000000000")
                        || guids.contains_key(*guid)
                        || (old_guids.contains(*guid) && !removed_guids.contains(*guid)))
                })
                .map(|guid| error_value("missing_guid_dependency",path,format!("Reference {guid} has no meta in the frozen result; include its asset/meta or explicitly revise the reference"))),
        );
        Ok(found)
    };
    let found = parallel::pool().install(|| {
        checked
            .into_par_iter()
            .map(|(path, found)| check(path, found))
            .collect::<Result<Vec<_>, String>>()
    })?;
    issues.extend(found.into_iter().flatten());
    for (path, state) in changed.iter().filter(|(p, _)| !p.ends_with(".meta")) {
        if state.is_none()
            && result
                .get(&format!("{path}.meta"))
                .and_then(|s| s.as_ref())
                .is_some()
        {
            issues.push(error_value("asset_meta_dependency",path,"Deleting/moving an asset also requires an explicit matching operation for its meta"));
        }
    }
    for delta in &job.deltas {
        if delta.base.is_none()
            && delta.source.is_some()
            && changed.get(&delta.path).and_then(|s| s.as_ref()).is_some()
        {
            let peer = if let Some(asset) = delta.path.strip_suffix(".meta") {
                asset.to_string()
            } else {
                format!("{}.meta", delta.path)
            };
            if job
                .deltas
                .iter()
                .any(|d| d.path == peer && d.base.is_none() && d.source.is_some())
                && result.get(&peer).and_then(|s| s.as_ref()).is_none()
            {
                issues.push(error_value(
                    "asset_meta_dependency",
                    &delta.path,
                    format!("Select paired added file {peer}"),
                ));
            }
        }
    }
    Ok(())
}

/// Three-way line merge in destination terms (see `text_merge`). Overlapping
/// or touching edits, including insertions at one boundary, are conflicts.
#[cfg(test)]
fn merge_text(base: &[u8], target: &[u8], source: &[u8]) -> Result<Vec<u8>, String> {
    text_merge::merge(base, target, source)?.bytes(target).ok_or_else(|| {
        "Selected source change overlaps or touches destination edits".to_string()
    })
}

/// Record agent-merged code/plain text as this path's resolution inside the
/// plan. The content is stored in the job, bound to the plan hash, applied by
/// `apply` and restored by `abort`, so manual merges never bypass the journal.
fn set_text(root: &Path, dir: &Path, job: &mut MergeJob, path: &str, params: &Value) -> Result<(), String> {
    if !scope_allows(job, path) {
        return Err("files.set_text is outside the prepared destination scope".into());
    }
    safe_path(root, path)?;
    let content = params
        .get("content")
        .and_then(Value::as_str)
        .ok_or("files.set_text requires content as a string")?;
    let eol = params.get("eol").and_then(Value::as_str).unwrap_or("target");
    if !matches!(eol, "target" | "exact") {
        return Err("eol must be 'target' (convert to the destination's line ending) or 'exact'".into());
    }
    let deltas: Vec<_> = job.deltas.iter().filter(|d| d.path == path).collect();
    if deltas.is_empty() {
        return Err("files.set_text resolves a selected source change; this path has no change in the selected commits".into());
    }
    if deltas.iter().any(|d| d.kind != "text") {
        return Err("files.set_text supplies merged code/plain text only; resolve Unity YAML with field/object operations and binary files with files.take".into());
    }
    if content.contains('\0') {
        return Err("files.set_text content must be text without NUL bytes".into());
    }
    if let Some(line) = content.lines().find(|line| {
        line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line.starts_with("||||||| ")
    }) {
        return Err(format!("files.set_text content still contains a conflict marker line: {line}"));
    }
    let target = job.snapshot.files.get(path).cloned().unwrap_or(None);
    let mut text = content.to_string();
    if eol == "target" {
        if let Some(state) = &target {
            if let Ok(existing) = String::from_utf8(read_blob(dir, state)?) {
                let crlf = existing.matches("\r\n").count();
                if crlf > existing.matches('\n').count() - crlf {
                    text = text.replace("\r\n", "\n").replace('\n', "\r\n");
                }
            }
        }
    }
    let mode = target
        .as_ref()
        .map(|state| state.mode.clone())
        .or_else(|| deltas.iter().find_map(|d| d.source.as_ref().map(|s| s.mode.clone())))
        .unwrap_or_else(|| "100644".into());
    let state = store_blob(dir, text.as_bytes(), &mode)?;
    job.selection.files.insert(
        path.into(),
        FileChoice {
            path: path.into(),
            version: "content".into(),
            commit: None,
            destination: None,
            content: Some(state),
        },
    );
    Ok(())
}

fn action_ids(job: &MergeJob, params: &Value) -> Result<Vec<String>, String> {
    if let Some(params) = params.as_object() {
        for key in params.keys() {
            if !matches!(key.as_str(), "workspaceRef" | "job_id" | "execution_delegation"
                | "path" | "object_id" | "property_path" | "commit" | "all" | "change_ids" | "side")
            {
                return Err(format!("Unknown merge selection parameter: {key}"));
            }
        }
    }
    if let Some(ids) = params.get("change_ids").and_then(Value::as_array) {
        if ["path", "object_id", "property_path", "commit", "all"]
            .iter()
            .any(|key| params.get(*key).is_some_and(|value| !value.is_null()))
        {
            return Err("change_ids cannot be combined with path/object/property/commit/all selectors; choose one explicit selection form".into());
        }
        let ids: Vec<String> = ids
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "change_ids must contain strings".to_string())
            })
            .collect::<Result<_, _>>()?;
        if ids.iter().any(|id| {
            !job.deltas
                .iter()
                .flat_map(|d| &d.changes)
                .any(|c| c["id"].as_str() == Some(id.as_str()))
        }) {
            return Err(
                "Unknown change ID; selections must refer to this job's immutable catalog".into(),
            );
        }
        return Ok(ids);
    }
    let path = params.get("path").and_then(Value::as_str);
    let object = params.get("object_id").and_then(Value::as_str);
    let property = params.get("property_path").and_then(Value::as_str);
    let commit = params.get("commit").and_then(Value::as_str);
    if params.get("all").and_then(Value::as_bool) != Some(true)
        && path.is_none()
        && commit.is_none()
    {
        return Err("Provide change_ids, a path/commit selector, or all=true explicitly".into());
    }
    let ids: Vec<_> = job
        .deltas
        .iter()
        .filter(|d| {
            path.map(|p| p == d.path).unwrap_or(true)
                && commit.map(|c| c == d.commit).unwrap_or(true)
        })
        .flat_map(|d| &d.changes)
        .filter(|c| {
            object
                .map(|o| c["object_id"].as_str() == Some(o))
                .unwrap_or(true)
                && property
                    .map(|p| c["property_path"].as_str() == Some(p))
                    .unwrap_or(true)
        })
        .filter_map(|c| c["id"].as_str().map(str::to_string))
        .collect();
    if ids.is_empty() {
        return Err("Selector matched no changes".into());
    }
    Ok(ids)
}
fn param_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} is required"))
}
fn editable(job: &MergeJob) -> Result<(), String> {
    if job.state != "prepared" {
        return Err(format!(
            "Job is {}; abort it or prepare a new job before changing decisions",
            job.state
        ));
    }
    Ok(())
}
fn chosen_paths(job: &MergeJob, params: &Value) -> Result<Vec<String>, String> {
    let paths=params.get("paths").and_then(Value::as_array).ok_or("Explicit paths are required; staging and committing never include the entire workspace implicitly")?;
    let mut paths: Vec<_> = paths
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| "paths must contain strings".to_string())
        })
        .collect::<Result<_, _>>()?;
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err("At least one explicit path is required".into());
    }
    let include_local = params
        .get("include_local_changes")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let dir = job_dir(Path::new(&job.root), &job.id)?;
    for path in &paths {
        if !scope_allows(job, path) {
            return Err(format!(
                "outside_destination_scope: {path} is outside this job's project root"
            ));
        }
        if !job.applied_files.contains_key(path)
            && !(include_local && job.snapshot.files.contains_key(path))
        {
            return Err(format!("{path} is not an applied output of this job"));
        }
        let original = job.snapshot.files.get(path).cloned().unwrap_or(None);
        let at_head = tree_file(Path::new(&job.root), &dir, &job.snapshot.head, path)?;
        let indexed = job.snapshot.index_records.split('\0').find_map(|record| {
            let (header, name) = record.split_once('\t')?;
            if name != path {
                return None;
            }
            let mut fields = header.split_whitespace();
            Some((fields.next()?.to_string(), fields.next()?.to_string()))
        });
        let head_identity = match &at_head {
            Some(state) => Some((
                state.mode.clone(),
                git_input(
                    Path::new(&job.root),
                    &["hash-object", "--stdin"],
                    &read_blob(&dir, state)?,
                    None,
                )?,
            )),
            None => None,
        };
        let work_identity = match &original {
            Some(state) => Some((
                state.mode.clone(),
                git_input(
                    Path::new(&job.root),
                    &["hash-object", "--stdin", "--path", path],
                    &read_blob(&dir, state)?,
                    None,
                )?,
            )),
            None => None,
        };
        if !include_local && (work_identity != head_identity || indexed != head_identity) {
            return Err(format!("{path} contained pre-existing local changes; include_local_changes=true explicitly accepts them in this staging/commit scope"));
        }
    }
    Ok(paths)
}

fn scoped_files(job: &MergeJob) -> BTreeMap<String, Option<FileState>> {
    let mut files = job.snapshot.files.to_inner();
    files.extend(job.applied_files.clone());
    files
}

/// Synchronous job operations are usable by the CLI driver without a UI process.
/// Async Unity validation is exposed separately below and by commands/SDK.
pub fn execute(root: &Path, id: &str, action: &str, params: Value) -> Result<Value, String> {
    // A malformed narrow selector must never silently become a broader one.
    // IDs stay strings to preserve Unity's complete signed 64-bit identity space.
    for key in ["path", "object_id", "property_path", "commit", "side", "destination", "parent_id", "kind"] {
        if params.get(key).is_some_and(|v| !v.is_null() && !v.is_string()) {
            return Err(format!("{key} must be a string (Unity object IDs must be exact decimal strings)"));
        }
    }
    if params.get("change_ids").is_some_and(|v| !v.is_null() && !v.is_array()) {
        return Err("change_ids must be an array of strings".into());
    }
    if params.get("all").is_some_and(|v| !v.is_null() && !v.is_boolean()) {
        return Err("all must be a boolean".into());
    }
    let allowed: Option<&[&str]> = match action {
        "snapshot" => Some(&["kind", "offset", "limit"]),
        "assets.read" => Some(&["path"]),
        "assets.preview" | "assets.apply" => Some(&["path", "operations", "expected_revision", "persist"]),
        "files.take" => Some(&["path", "version", "commit"]),
        "files.delete" | "files.clear" => Some(&["path"]),
        "files.set_text" => Some(&["path", "content", "eol"]),
        "files.move" => Some(&["path", "destination"]),
        "objects.take" | "objects.add" => Some(&["path", "object_id", "commit", "side"]),
        "objects.delete" | "objects.restore" => Some(&["path", "object_id", "commit"]),
        "objects.clear" => Some(&["path", "object_id"]),
        "objects.move" => Some(&["path", "object_id", "parent_id", "position"]),
        "fields.set" => Some(&["path", "object_id", "property_path", "commit", "value"]),
        "fields.take" => Some(&["path", "object_id", "property_path", "commit", "side"]),
        "fields.delete" => Some(&["path", "object_id", "property_path", "commit"]),
        "inspect_asset" => Some(&["path", "version", "commit", "object_id", "property_path", "offset", "reference_offset", "limit", "scalar_limit"]),
        _ => None,
    };
    if let (Some(allowed), Some(params)) = (allowed, params.as_object()) {
        for key in params.keys() {
            if !matches!(key.as_str(), "workspaceRef" | "job_id" | "execution_delegation")
                && !allowed.contains(&key.as_str())
            {
                return Err(format!("Unknown {action} parameter: {key}"));
            }
        }
    }
    for key in ["position", "offset", "reference_offset", "limit", "scalar_limit"] {
        if params.get(key).is_some_and(|v| !v.is_null() && v.as_u64().is_none()) {
            return Err(format!("{key} must be a nonnegative integer"));
        }
    }
    if action == "files.take" {
        if let Some(version) = params.get("version").and_then(Value::as_object) {
            if version.keys().any(|key| !matches!(key.as_str(), "side" | "commit")) {
                return Err("Whole-file version only accepts side and commit".into());
            }
            for key in ["side", "commit"] {
                if version.get(key).is_some_and(|v| !v.is_null() && !v.is_string()) {
                    return Err(format!("version.{key} must be a string"));
                }
            }
            if let (Some(commit), Some(nested_commit)) = (
                params.get("commit").and_then(Value::as_str),
                version.get("commit").and_then(Value::as_str),
            ) {
                if commit != nested_commit {
                    return Err("commit and version.commit must name the same exact source commit".into());
                }
            }
        }
    } else if action == "inspect_asset" && params.get("version").is_some_and(|v| !v.is_null() && !v.is_string()) {
        return Err("version must be a string".into());
    }
    let project_root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let root = repository(root)?;
    let _lock = lock(&root)?;
    let dir = job_dir(&root, id)?;
    if action == "abort" && !dir.join("job.json").is_file() {
        return Ok(json!({"job_id":id,"state":"removed","idempotent":true}));
    }
    let mut job = load(&project_root, id)?;
    if job.mode == PrepareMode::Files && (action.starts_with("objects.") || action.starts_with("fields.") || action.starts_with("assets.")) {
        return Err("File replacement plans support file operations; prepare mode='structural' for object/field edits".into());
    }
    if matches!(action, "changes" | "change_summary" | "include" | "exclude" | "defer" | "resolve"
        | "files.include" | "files.exclude" | "objects.include" | "objects.exclude"
        | "objects.take" | "objects.add" | "objects.delete" | "objects.restore" | "fields.include" | "fields.exclude"
        | "fields.take" | "fields.set" | "fields.delete") && !job.catalog_ready {
        ensure_catalog(&dir, &mut job)?;
        save(&dir, &job)?;
    }
    // A full snapshot freezes clean Unity files as read-only inputs; an edit
    // of one makes it a writable target with its unchanged raw bytes.
    if job.paths.is_none() && matches!(action, "assets.preview" | "assets.apply" | "files.set_text"
        | "objects.take" | "objects.add" | "objects.delete" | "objects.move" | "objects.restore"
        | "fields.take" | "fields.set" | "fields.delete")
    {
        if let Some(path) = params.get("path").and_then(Value::as_str) {
            if !job.snapshot.files.contains_key(path) && scope_allows(&job, path) {
                safe_path(&root, path)?;
                let state = snapshot::capture_target(&root, &dir, &job, path)?;
                job.snapshot.files.insert(path.to_string(), state);
            }
        }
    }
    if action == "assets.read" { return transforms::read_asset(&dir,&job,&params); }
    if matches!(action,"assets.preview"|"assets.apply") {
        editable(&job)?;
        let apply=action=="assets.apply";
        let mut output=transforms::edit_asset(&dir,&mut job,&params,apply)?;
        if apply { job.revision+=1; job.unity_validated_hash=None; save(&dir,&job)?; }
        output["job_id"]=json!(id); output["plan_revision"]=json!(job.revision);
        return Ok(output);
    }
    if action == "get" {
        return Ok(summary(&job));
    }
    if action == "snapshot" {
        let kind = params.get("kind").and_then(Value::as_str).unwrap_or("target");
        let files = match kind { "target" => &job.snapshot.files, "dependencies" => &job.dependency_files,
            _ => return Err("snapshot kind must be target or dependencies".into()) };
        let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1,1000) as usize;
        return Ok(json!({"job_id":job.id,"kind":kind,"files":files.iter().skip(offset).take(limit).collect::<BTreeMap<_,_>>(),
            "total":files.len(),"next_offset":offset.checked_add(limit).filter(|n|*n<files.len())}));
    }
    if action == "inspect_asset" {
        return inspection::inspect(&dir, &job, &params);
    }
    if action == "changes" {
        return catalog_query::changes(&job, &params);
    }
    if action == "change_summary" {
        return catalog_query::summary(&dir, &job, &params);
    }
    if matches!(action, "preview" | "validate" | "check_dependencies") {
        let preview = preview_job(&dir, &job)?;
        if action == "validate" {
            let level = params
                .get("level")
                .and_then(Value::as_str)
                .unwrap_or("static");
            if level != "static" {
                return Err("Unity validation is asynchronous; use merges.validate through the SDK or validate_unity from the CLI driver".into());
            }
        }
        return serde_json::to_value(preview).map_err(|e| e.to_string());
    }
    let mut restored = Value::Null;
    match action {
        "new_plan" => {
            editable(&job)?;
            if params
                .get("default")
                .and_then(Value::as_str)
                .unwrap_or("keep_target")
                != "keep_target"
            {
                return Err("Only the explicit keep_target default is supported; include(all=true) selects source changes".into());
            }
            job.selection = Selection::default();
        }
        "include" | "exclude" | "defer" | "resolve" | "files.include" | "files.exclude"
        | "objects.include" | "objects.exclude" | "fields.include" | "fields.exclude" => {
            editable(&job)?;
            let ids = action_ids(&job, &params)?;
            let resolution = if action == "resolve" || action.ends_with(".take") {
                json!({"kind":params.get("side").and_then(Value::as_str).unwrap_or("source")})
            } else if action == "fields.set" {
                json!({"kind":"set","value":params.get("value").ok_or("set requires a typed value")?})
            } else if action.ends_with("exclude") {
                json!({"kind":"exclude"})
            } else if action == "defer" {
                json!({"kind":"defer"})
            } else {
                json!({"kind":"include"})
            };
            for id in ids {
                job.selection.decisions.insert(id, resolution.clone());
            }
        }
        "objects.take" | "objects.add" | "objects.delete" => {
            editable(&job)?;
            let path = param_str(&params, "path")?;
            let object_id = param_str(&params, "object_id")?;
            let commit = params.get("commit").and_then(Value::as_str);
            let candidates: Vec<_> = job
                .deltas
                .iter()
                .filter(|d| d.path == path && commit.map(|c| c == d.commit).unwrap_or(true))
                .collect();
            if candidates.len() != 1 {
                return Err("An object operation requires one explicit selected source commit when the file has multiple versions".into());
            }
            let delta = candidates[0];
            if delta.kind != "unity_yaml" {
                return Err("Object operations require a Unity YAML asset".into());
            }
            let resolution = if action == "objects.delete" {
                json!({"kind":"delete"})
            } else {
                json!({"kind":params.get("side").and_then(Value::as_str).unwrap_or("source")})
            };
            let choice = ObjectChoice {
                path: path.into(),
                object_id: object_id.into(),
                commit: delta.commit.clone(),
                operation: action.strip_prefix("objects.").unwrap().into(),
                resolution,
            };
            job.selection
                .objects
                .insert(format!("{}:{}:{}", choice.commit, path, object_id), choice);
        }
        "objects.restore" => {
            editable(&job)?;
            restored = objects_restore(&dir, &mut job, &params)?;
        }
        "objects.clear" => {
            editable(&job)?;
            let path = param_str(&params, "path")?;
            let object = param_str(&params, "object_id")?;
            job.selection
                .objects
                .retain(|_, choice| choice.path != path || choice.object_id != object);
        }
        "fields.set" | "fields.take" | "fields.delete" => {
            editable(&job)?;
            transforms::set(&mut job, action, &params)?;
        }
        "objects.move" => {
            editable(&job)?;
            transforms::move_object(&dir, &mut job, &params)?;
        }
        "files.take" | "files.delete" | "files.move" => {
            editable(&job)?;
            let path = param_str(&params, "path")?.to_string();
            if !scope_allows(&job, &path) {
                return Err("File operation is outside the prepared destination scope".into());
            }
            safe_path(&root, &path)?;
            if !job.snapshot.files.contains_key(&path) {
                let state = snapshot::capture_target(&root, &dir, &job, &path)?;
                job.snapshot.files.insert(path.clone(), state);
            }
            let version = if action == "files.delete" {
                "delete"
            } else if action == "files.move" {
                "move"
            } else {
                params
                    .get("version")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        params
                            .get("version")
                            .and_then(|v| v.get("side"))
                            .and_then(Value::as_str)
                    })
                    .ok_or("Whole-file take requires an explicit version")?
            };
            let commit = params
                .get("commit")
                .and_then(Value::as_str)
                .or_else(|| {
                    params
                        .get("version")
                        .and_then(|v| v.get("commit"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string);
            if matches!(version, "source" | "base") && !job.deltas.iter().any(|delta|
                delta.path == path && commit.as_ref().map_or(true, |commit|commit == &delta.commit)
                    && (delta.base.is_some() || delta.source.is_some())) {
                return Err("No source file version exists at this exact path; check its spelling or use files.delete for an explicit deletion".into());
            }
            let destination = params
                .get("destination")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(dest) = &destination {
                if !scope_allows(&job, dest) {
                    return Err("Move destination is outside the prepared scope; include it in prepare paths".into());
                }
                safe_path(&root, dest)?;
                if !job.snapshot.files.contains_key(dest) {
                    let state = snapshot::capture_target(&root, &dir, &job, dest)?;
                    job.snapshot.files.insert(dest.clone(), state);
                }
            }
            job.selection.files.insert(
                path.clone(),
                FileChoice {
                    path,
                    version: version.into(),
                    commit,
                    destination,
                    content: None,
                },
            );
        }
        "files.set_text" => {
            editable(&job)?;
            let path = param_str(&params, "path")?.to_string();
            set_text(&root, &dir, &mut job, &path, &params)?;
        }
        "files.clear" => {
            editable(&job)?;
            job.selection.files.remove(param_str(&params, "path")?);
        }
        "apply" => {
            let expected = param_str(&params, "expected_plan_hash")?;
            if job.applied_hash.as_deref() == Some(expected)
                && matches!(job.state.as_str(), "applied" | "staged" | "committed")
            {
                return Ok(
                    json!({"job_id":id,"state":job.state,"plan_hash":expected,"idempotent":true,"files":job.applied_files}),
                );
            }
            editable(&job)?;
            if params
                .get("index_policy")
                .and_then(Value::as_str)
                .unwrap_or("preserve")
                != "preserve"
            {
                return Err(
                    "apply preserves HEAD/index; use explicit stage/commit operations".into(),
                );
            }
            let preview = preview_job(&dir, &job)?;
            if preview.plan_hash != expected {
                return Err("stale: preview hash does not match the current plan".into());
            }
            if !preview.ready_to_apply {
                return Err(format!(
                    "Unresolved merge dependencies/conflicts: {}",
                    serde_json::to_string(&preview.issues).unwrap_or_default()
                ));
            }
            check_target(&root, &dir, &job)?;
            job.applied_files = preview.files;
            job.applied_before = job
                .applied_files
                .keys()
                .map(|path| {
                    (
                        path.clone(),
                        job.snapshot.files.get(path).cloned().unwrap_or(None),
                    )
                })
                .collect();
            job.applied_hash = Some(expected.into());
            job.state = "applying".into();
            save(&dir, &job)?;
            let editor_applied = match asset_apply::try_apply(&root, &dir, &job) {
                Ok(applied) => applied,
                Err(error) => {
                    let unknown = error.contains("outcome_unknown") || error.contains("invalid_response");
                    let confirmed_before = !unknown && asset_apply::files_match(&root, &dir, &job.applied_before).is_ok();
                    job.state = if confirmed_before { "aborted" } else { "recovery_required" }.into();
                    save(&dir, &job)?;
                    return Err(format!("Editor merge apply failed: {error}; {}; merge journal={id}",
                        if confirmed_before { "all original files verified; rolled_back" } else { "outcome requires inspection; no Rust rollback performed" }));
                }
            };
            if !editor_applied { for (path, state) in &job.applied_files {
                if capture_like(&root, &dir, path, job.applied_before[path].as_ref())?
                    != job.applied_before[path]
                {
                    return Err(format!(
                        "stale during apply: {path}; job journal retained for guarded abort"
                    ));
                }
                if let Err(error) = write_file(&root, &dir, path, state) {
                    let rollback = restore(&root, &dir, &mut job);
                    save(&dir, &job)?;
                    return Err(format!("Apply failed: {error}; rollback: {rollback:?}"));
                }
            } }
            if editor_applied {
                if let Err(error) = asset_apply::files_match(&root, &dir, &job.applied_files) {
                    job.state = "recovery_required".into();
                    save(&dir, &job)?;
                    return Err(format!("Editor import changed merge output: {error}; merge journal={id}; no Rust rollback performed"));
                }
            }
            // An external index/ref writer is never reverted by this operation.
            if git_text(&root, &["rev-parse", "HEAD"])? != job.snapshot.head
                || index_entries(&root)?
                    != (
                        job.snapshot.index_records.to_inner(),
                        job.snapshot.index_flags.to_inner(),
                    )
            {
                job.state = "recovery_required".into();
                save(&dir, &job)?;
                return Err("Destination Git state changed during apply; preserved journal and external state for recovery".into());
            }
            job.state = "applied".into();
            save(&dir, &job)?;
            return Ok(
                json!({"job_id":id,"state":job.state,"plan_hash":expected,"files":job.applied_files,"index_preserved":true,"unity_validated":false,"editor_applied":editor_applied}),
            );
        }
        "abort" => {
            if job.state == "aborted" {
                retention::retire(&root, id)?;
                return Ok(json!({"job_id":id,"state":"aborted","idempotent":true,"removed":true}));
            }
            if job.commit_oid.is_some() || matches!(job.state.as_str(), "staged" | "staging") {
                return Err("A staged/committed job requires a new explicit reverse plan; abort never resets index or rewrites commits".into());
            }
            restore(&root, &dir, &mut job)?;
            save(&dir, &job)?;
            retention::retire(&root, id)?;
            return Ok(json!({"job_id":id,"state":job.state,"removed":true}));
        }
        "stage" => {
            check_destination(&root, &job)?;
            snapshot::check_dependencies(&root, &dir, &job)?;
            if git_text(&root, &["rev-parse", "HEAD"])? != job.snapshot.head {
                return Err("stale: destination HEAD changed before stage".into());
            }
            if job.state == "staging" {
                recover_index(&root, &dir, &job)?;
                let (records, flags) = index_entries(&root)?;
                job.staged_index_records = Some(records);
                job.staged_index_flags = Some(flags);
                job.state = "staged".into();
                save(&dir, &job)?;
            }
            if !matches!(job.state.as_str(), "applied" | "staged") {
                return Err("Apply the plan before staging its explicit paths".into());
            }
            let paths = chosen_paths(&job, &params)?;
            let all_files = scoped_files(&job);
            check_files(
                &root,
                &dir,
                &paths
                    .iter()
                    .map(|path| (path.clone(), all_files[path].clone()))
                    .collect(),
            )?;
            let transaction = IndexTransaction::begin(&root)?;
            let expected_index = job
                .staged_index_records
                .as_ref()
                .unwrap_or(&job.snapshot.index_records);
            if index_entries(&root)?
                != (
                    expected_index.clone(),
                    job.staged_index_flags
                        .as_ref()
                        .unwrap_or(&job.snapshot.index_flags)
                        .clone(),
                )
            {
                return Err("stale: destination index changed before stage".into());
            }
            let after = prepare_index(&root, &dir, &transaction.original, &all_files, &paths)?;
            job.pending_index_before =
                Some(blake3::hash(&transaction.original).to_hex().to_string());
            job.pending_index_after = Some(store_blob(&dir, &after, "index")?);
            job.state = "staging".into();
            save(&dir, &job)?;
            transaction.install(&after)?;
            let (records, flags) = index_entries(&root)?;
            job.staged_index_records = Some(records);
            job.staged_index_flags = Some(flags);
            job.state = "staged".into();
            save(&dir, &job)?;
            return Ok(json!({"job_id":id,"state":job.state,"paths":paths}));
        }
        "commit" => {
            let mut result = commit_job(&root, &dir, &mut job, &params)?;
            if job.state == "committed" {
                // The commit is the job's record from here on.
                retention::retire(&root, id)?;
                result["removed"] = json!(true);
            }
            return Ok(result);
        }
        _ => return Err(format!("Unknown merge job operation: {action}")),
    }
    job.revision += 1;
    job.unity_validated_hash = None;
    save(&dir, &job)?;
    let mut result = json!({"job_id":id,"revision":job.revision,"state":job.state});
    if !restored.is_null() {
        result["restored"] = restored;
    }
    Ok(result)
}

/// Brings back an object the destination deleted, with the deleted part of
/// the hierarchy it belongs to and the owner list elements that attach it.
/// Objects with a catalog change are resolved to the source; the others are
/// added from the source; attachments are field takes. All of them render
/// with this commit's changes and are validated as one result.
fn objects_restore(dir: &Path, job: &mut MergeJob, params: &Value) -> Result<Value, String> {
    let path = param_str(params, "path")?.to_string();
    let object_id = param_str(params, "object_id")?.to_string();
    let commit = params.get("commit").and_then(Value::as_str);
    let candidates: Vec<&CommitDelta> = job
        .deltas
        .iter()
        .filter(|d| d.path == path && commit.map_or(true, |c| c == d.commit))
        .collect();
    let [delta] = candidates.as_slice() else {
        return Err("objects.restore requires one explicit selected source commit when the file has multiple versions".into());
    };
    if delta.kind != "unity_yaml" {
        return Err("objects.restore requires a Unity YAML asset".into());
    }
    let delta = (*delta).clone();
    let target = state_bytes(dir, job.snapshot.files.get(&path).unwrap_or(&None))?;
    let source = state_bytes(dir, &delta.source)?;
    let parse = |bytes: &[u8], side: &str| {
        crate::unity_asset_core::parse(bytes).map_err(|e| format!("Cannot read the {side} version of {path}: {e}"))
    };
    let restore = crate::unity_asset_core::restore_closure(&parse(&target, "destination")?, &parse(&source, "source")?, &object_id)?;
    let mut resolved = vec![];
    let mut added = vec![];
    for object in &restore.objects {
        let key = format!("{}:{}:{}", delta.commit, path, object);
        let ids: Vec<String> = delta
            .changes
            .iter()
            .filter(|c| c["object_id"].as_str() == Some(object.as_str()) && c["property_path"].as_str() == Some(""))
            .filter_map(|c| c["id"].as_str().map(str::to_string))
            .collect();
        if ids.is_empty() {
            job.selection.objects.insert(key, ObjectChoice {
                path: path.clone(),
                object_id: object.clone(),
                commit: delta.commit.clone(),
                operation: "add".into(),
                resolution: json!({"kind":"source"}),
            });
            added.push(object.clone());
        } else {
            job.selection.objects.remove(&key);
            for id in ids {
                job.selection.decisions.insert(id.clone(), json!({"kind":"source"}));
                resolved.push(id);
            }
        }
    }
    let mut attached = vec![];
    for (owner, property) in &restore.attach {
        transforms::set(job, "fields.take", &json!({"path":path,"object_id":owner,"property_path":property,"side":"source","commit":delta.commit}))?;
        attached.push(json!({"object_id":owner,"property_path":property}));
    }
    Ok(json!({"objects":restore.objects,"resolved_change_ids":resolved,"added":added,"attached":attached}))
}

/// A file the Editor serializes (an asset, scene, Prefab or `.meta`) and can
/// therefore hold with unsaved changes; code and plain text it never writes.
fn editor_serialized(path: &str) -> bool {
    snapshot::evidence(path)
        && !path.ends_with(".cs")
        && !path.ends_with(".asmdef")
        && !path.ends_with(".json")
        && !path.ends_with("ProjectVersion.txt")
}

fn restore(root: &Path, dir: &Path, job: &mut MergeJob) -> Result<(), String> {
    if job.applied_files.keys().any(|path| editor_serialized(path)) && crate::unity_bridge::is_unity_project(&job.project_root) {
        let editor = crate::unity_bridge::query_current_project_editor_process_uncached(job.project_root.clone());
        match editor.state {
            crate::unity_bridge::UnityEditorProcessState::NotRunning => {}
            crate::unity_bridge::UnityEditorProcessState::Running => {
                return Err("Close the destination Unity Editor before restoring an applied merge; disk recovery must not overwrite unsaved Editor assets".into());
            }
            _ => {
                return Err(format!("Cannot prove that the destination Unity Editor is closed; close it before merge recovery: {:?}", editor.last_error));
            }
        }
    }
    let mut blocked = vec![];
    for (path, before) in &job.applied_before {
        let current = capture_like(
            root,
            dir,
            path,
            job.applied_files.get(path).and_then(|s| s.as_ref()),
        )?;
        if &current == before {
            continue;
        }
        if job.applied_files.get(path) != Some(&current) {
            blocked.push(path.clone());
            continue;
        }
        write_file(root, dir, path, before)?;
    }
    if !blocked.is_empty() {
        job.state = "recovery_required".into();
        save(dir, job)?;
        return Err(format!(
            "Later external edits preserved; recovery required for {}",
            blocked.join(", ")
        ));
    }
    job.state = "aborted".into();
    Ok(())
}

fn commit_job(
    root: &Path,
    dir: &Path,
    job: &mut MergeJob,
    params: &Value,
) -> Result<Value, String> {
    if let Some(oid) = job.commit_oid.clone() {
        if job.state == "committed" {
            return Ok(json!({"commit":oid,"idempotent":true}));
        }
        check_destination(root, job)?;
        let current_head = git_text(root, &["rev-parse", "HEAD"])?;
        if current_head == oid {
            if job.state != "committed" {
                recover_index(root, dir, job)?;
            }
            job.state = "committed".into();
            save(dir, job)?;
            return Ok(json!({"commit":oid,"idempotent":true}));
        }
        if current_head == job.snapshot.head && job.state == "committing" {
            let transaction = IndexTransaction::begin(root)?;
            let before = blake3::hash(&transaction.original).to_hex().to_string();
            if job.pending_index_before.as_deref() != Some(before.as_str()) {
                return Err("Index changed before recovering the pending commit; preserved external staged state".into());
            }
            let after = read_blob(
                dir,
                job.pending_index_after
                    .as_ref()
                    .ok_or("Pending commit index is missing")?,
            )?;
            git(root, &["update-ref", "HEAD", &oid, &job.snapshot.head])?;
            transaction.install(&after)?;
            job.state = "committed".into();
            save(dir, job)?;
            return Ok(json!({"commit":oid,"idempotent":true,"recovered":true}));
        }
        return Err("Commit journal has a pending/existing commit whose ref no longer matches; inspect it before proceeding".into());
    }
    if !matches!(job.state.as_str(), "applied" | "staged") {
        return Err("Apply the plan before committing".into());
    }
    let paths = chosen_paths(job, params)?;
    check_destination(root, job)?;
    snapshot::check_dependencies(root, dir, job)?;
    let all_files = scoped_files(job);
    let message = param_str(params, "message")?;
    if message.trim().is_empty() {
        return Err("Commit message cannot be empty".into());
    }
    check_files(root, dir, &all_files)?;
    if git_text(root, &["rev-parse", "HEAD"])? != job.snapshot.head {
        return Err("stale: HEAD changed before commit".into());
    }
    let touches_unity = paths.iter().any(|p| {
        job.deltas
            .iter()
            .any(|d| &d.path == p && (d.kind == "unity_yaml" || d.kind == "binary"))
            || unity_evidence(p)
    });
    let output_files: BTreeMap<_, _> = paths
        .iter()
        .map(|path| (path.clone(), all_files[path].clone()))
        .collect();
    let tree = commit_validation::build_tree(root, dir, &job.snapshot.head, &output_files)?;
    let include_local = params
        .get("include_local_changes")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let exact_validated = commit_validation::covers(job, &tree, &paths, include_local)?;
    if touches_unity && !exact_validated {
        if job.unity_validated_hash != job.applied_hash {
            return Err("Unity asset/code commits require plan.validate(level='unity', paths=the_commit_paths, include_local_changes=...) or validation of the equivalent applied result".into());
        }
        validate_commit_scope(root, dir, job, &output_files, true)?;
    }
    let expected_index = job
        .staged_index_records
        .as_ref()
        .unwrap_or(&job.snapshot.index_records);
    let transaction = IndexTransaction::begin(root)?;
    if index_entries(root)?
        != (
            expected_index.clone(),
            job.staged_index_flags
                .as_ref()
                .unwrap_or(&job.snapshot.index_flags)
                .clone(),
        )
    {
        return Err("stale: index changed before commit".into());
    }
    let oid = git_input(
        root,
        &["commit-tree", &tree, "-p", &job.snapshot.head],
        message.as_bytes(),
        None,
    )?;
    let final_index = prepare_index(root, dir, &transaction.original, &all_files, &paths)?;
    job.pending_index_before = Some(blake3::hash(&transaction.original).to_hex().to_string());
    job.pending_index_after = Some(store_blob(dir, &final_index, "index")?);
    job.commit_oid = Some(oid.clone());
    job.state = "committing".into();
    save(dir, job)?;
    git(root, &["update-ref", "HEAD", &oid, &job.snapshot.head])?;
    // Update only explicitly committed paths in the real index. Existing staged
    // changes on every other path remain byte-for-byte logical entries.
    transaction.install(&final_index)?;
    job.state = "committed".into();
    save(dir, job)?;
    Ok(
        json!({"job_id":job.id,"state":job.state,"commit":oid,"parents":[job.snapshot.head],"paths":paths}),
    )
}

fn recover_index(root: &Path, dir: &Path, job: &MergeJob) -> Result<(), String> {
    let Some(after) = &job.pending_index_after else {
        return Err("Pending index recovery data is missing".into());
    };
    let transaction = IndexTransaction::begin(root)?;
    let current_hash = blake3::hash(&transaction.original).to_hex().to_string();
    if current_hash == after.blob {
        return Ok(());
    }
    if job.pending_index_before.as_deref() != Some(current_hash.as_str()) {
        return Err("The index changed after the interrupted operation; external staged changes were preserved for manual reconciliation".into());
    }
    transaction.install(&read_blob(dir, after)?)
}

fn unity_evidence(path: &str) -> bool {
    path.ends_with(".cs")
        || path.ends_with(".asmdef")
        || path.ends_with(".asmref")
        || path.ends_with(".meta")
        || path.ends_with(".unity")
        || path.ends_with(".prefab")
        || path.ends_with(".asset")
        || path.ends_with(".mat")
        || path.ends_with(".anim")
        || path.ends_with(".controller")
        || path.ends_with(".overrideController")
        || path.ends_with(".playable")
        || path.ends_with(".mask")
        || path.ends_with(".renderTexture")
        || path.ends_with(".terrainlayer")
        || path.contains("ProjectSettings/")
        || path.ends_with("Packages/manifest.json")
        || path.ends_with("Packages/packages-lock.json")
}
fn equivalent_bytes(
    dir: &Path,
    left: &Option<FileState>,
    right: &Option<FileState>,
) -> Result<bool, String> {
    if left == right {
        return Ok(true);
    }
    let (Some(left), Some(right)) = (left, right) else {
        return Ok(false);
    };
    if left.mode != right.mode {
        return Ok(false);
    }
    let a = read_blob(dir, left)?;
    let b = read_blob(dir, right)?;
    Ok(match (std::str::from_utf8(&a), std::str::from_utf8(&b)) {
        (Ok(a), Ok(b)) => a.replace("\r\n", "\n") == b.replace("\r\n", "\n"),
        _ => false,
    })
}
fn validate_commit_scope(
    root: &Path,
    dir: &Path,
    job: &MergeJob,
    outputs: &BTreeMap<String, Option<FileState>>,
    requires_unity: bool,
) -> Result<(), String> {
    if !requires_unity {
        return Ok(());
    }
    let mut all_files = job.dependency_files.clone();
    all_files.extend(scoped_files(job));
    let candidate = validate_commit_dependencies(root, dir, job, outputs)?;
    if requires_unity {
        let mut unvalidated = vec![];
        for (path, expected) in &all_files {
            if unity_evidence(path)
                && !equivalent_bytes(dir, expected, candidate.get(path).unwrap_or(&None))?
            {
                unvalidated.push(path.clone());
            }
        }
        if !unvalidated.is_empty() {
            return Err(format!("needs_commit_validation: this commit tree differs from the Unity-validated snapshot in {}; explicitly include the required source/local paths or validate that narrower result in another checkout",unvalidated.join(", ")));
        }
    }
    Ok(())
}

fn validate_commit_dependencies(
    root: &Path,
    dir: &Path,
    job: &MergeJob,
    outputs: &BTreeMap<String, Option<FileState>>,
) -> Result<BTreeMap<String, Option<FileState>>, String> {
    let mut all_files = job.dependency_files.to_inner();
    all_files.extend(scoped_files(job));
    let paths: Vec<_> = all_files.keys().cloned().collect();
    let head_files = tree_files(root, dir, &job.snapshot.head, &paths)?;
    let mut candidate = head_files.clone();
    candidate.extend(outputs.clone());
    let mut baseline = job.clone();
    baseline.dependency_files.clear();
    baseline.snapshot.files = head_files.into();
    let mut issues = vec![];
    dependency_issues(dir, &baseline, &candidate, outputs, &mut issues)?;
    issues.extend(schema::commit_field_issues(
        dir, &all_files, &candidate, outputs,
    )?);
    if !issues.is_empty() {
        return Err(format!(
            "Commit subset has unresolved dependencies: {}",
            serde_json::to_string(&issues).unwrap_or_default()
        ));
    }
    Ok(candidate)
}

/// Check the actual destination Editor before a coordinated disk apply.
pub async fn preflight_apply(root: &Path, id: &str) -> Result<(), String> {
    let job = load(root, id)?;
    if job.applied_hash.is_some()
        && matches!(job.state.as_str(), "applied" | "staged" | "committed")
    {
        return Ok(());
    }
    let project = &job.project_root;
    if !crate::unity_bridge::is_unity_project(project) {
        return Ok(());
    }
    if !crate::unity_bridge::is_unity_connected(project).await {
        crate::unity_assets::require_closed_editor(Path::new(project)).await.map_err(|error|
            format!("Merge apply requires the destination Editor to be connected or closed: {error}"))?;
        return Ok(());
    }
    let dir = job_dir(Path::new(&job.root), id)?;
    let preview = preview_job(&dir, &job)?;
    let paths = asset_preflight::project_asset_paths(
        Path::new(&job.root), Path::new(project), preview.files.keys().map(String::as_str),
    )?;
    // This is a targeted readiness check for the legacy merge writer. Unlike
    // assets.apply's disk_apply transaction it does not hold Unity's main thread
    // while the subsequent Rust merge filesystem loop executes.
    let output = crate::unity_bridge::asset_api(project, &json!({"action":"disk_preflight","paths":paths})).await?;
    if output["ready"].as_bool() != Some(true) {
        return Err(format!("Unity did not confirm the targeted merge apply barrier: {output}"));
    }
    Ok(())
}

/// Paths an Editor must reimport after a direct disk apply. Scripts and
/// assemblies are left to the normal compile refresh.
fn editor_sync_paths(job: &MergeJob) -> Result<(Vec<String>, Vec<String>), String> {
    let changed: Vec<&str> = job
        .applied_files
        .iter()
        .filter(|(path, state)| job.applied_before.get(*path) != Some(*state))
        .map(|(path, _)| path.as_str())
        .collect();
    let assets = asset_preflight::project_asset_paths(
        Path::new(&job.root),
        Path::new(&job.project_root),
        changed.iter().copied(),
    )?;
    let script = |path: &str| {
        let lower = path.to_ascii_lowercase();
        [".cs", ".asmdef", ".asmref", ".dll"].iter().any(|ext| lower.ends_with(ext))
    };
    let (scripts, assets): (Vec<_>, Vec<_>) = assets.into_iter().partition(|p| script(p));
    Ok((assets, scripts))
}

/// A plan with code, text or new files is written directly by Rust instead of
/// the Editor's disk transaction. Afterwards a connected Editor reimports the
/// written assets and reloads affected open scenes/Prefab Stage, so it never
/// keeps (and later saves) the previous in-memory version. The outcome is
/// reported as `editor_sync`; a failed sync is a warning, the apply stands.
pub async fn sync_editor_after_apply(root: &Path, id: &str, mut result: Value) -> Value {
    if result["editor_applied"].as_bool() != Some(false) {
        return result;
    }
    let sync = async {
        let job = load(root, id)?;
        let project = job.project_root.clone();
        if !crate::unity_bridge::is_unity_project(&project) {
            return Ok::<Value, String>(json!({"status":"not_unity_project"}));
        }
        let (assets, scripts) = editor_sync_paths(&job)?;
        let script_note = (!scripts.is_empty()).then(|| {
            json!({"scripts_changed":scripts,"hint":"the Editor recompiles changed scripts on its next refresh; run unity_recompile before relying on new types"})
        });
        if !crate::unity_bridge::is_unity_connected(&project).await {
            let probe = project.clone();
            let closed = tokio::task::spawn_blocking(move || {
                matches!(
                    crate::unity_bridge::query_current_project_editor_process_uncached(probe).state,
                    crate::unity_bridge::UnityEditorProcessState::NotRunning
                )
            })
            .await
            .unwrap_or(false);
            let status = if closed { "editor_closed" } else { "editor_not_connected" };
            return Ok(json!({"status":status,"assets":assets,"scripts":script_note}));
        }
        if assets.is_empty() {
            return Ok(json!({"status":"no_assets","scripts":script_note}));
        }
        let response = crate::unity_bridge::asset_api(&project, &json!({"action":"disk_reload","paths":assets})).await?;
        Ok(json!({"status":"synced","reloaded":response,"scripts":script_note}))
    };
    match sync.await {
        Ok(status) => {
            let stale = status["reloaded"]["stale"].as_array().filter(|s| !s.is_empty()).cloned();
            if let Some(stale) = stale {
                result["warnings"] = json!([format!(
                    "Unity kept unsaved in-memory edits for {}; these were not reloaded. Revert them in the Editor (or reload the scene) before saving, otherwise the merged files will be overwritten",
                    stale.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                )]);
            }
            if status["status"] == "editor_not_connected" {
                result["warnings"] = json!(["A Unity Editor may be running without a Locus connection; if it has these assets or scenes open, reload them before saving"]);
            }
            result["editor_sync"] = status;
        }
        Err(error) => {
            result["editor_sync"] = json!({"status":"failed","error":error});
            result["warnings"] = json!([format!(
                "The merged files are on disk, but the connected Unity Editor did not confirm reloading them ({error}). Reload affected open scenes/assets before saving in the Editor; an older Locus Unity package may need updating"
            )]);
        }
    }
    result
}

/// Validate the applied, journaled destination in its exact Unity 6.5 Editor.
/// This does not claim that an unapplied preview was imported.
pub async fn validate_unity(root: &Path, id: &str) -> Result<Value, String> {
    let job = load(root, id)?;
    if !matches!(job.state.as_str(), "applied" | "staged") {
        return Err("Unity validation checks the applied destination: preview/static validate, apply, then validate(level='unity'); an unapplied preview has not been imported".into());
    }
    let dir = job_dir(Path::new(&job.root), id)?;
    check_destination(Path::new(&job.root), &job)?;
    let expected_files = scoped_files(&job);
    check_files(Path::new(&job.root), &dir, &expected_files)?;
    snapshot::check_dependencies(Path::new(&job.root), &dir, &job)?;
    let project = job.project_root.clone();
    let version = crate::unity_bridge::read_project_unity_version(&project)?
        .ok_or("Unity project version not found")?;
    if !version.starts_with("6000.5.") {
        return Err(format!("First-release Unity validation requires Unity 6.5 (6000.5.x), project declares {version}"));
    }
    // The existing dirty code is compilation/schema context. Only selected
    // changes may broaden the asset validation scope, otherwise installing the
    // bridge or unrelated local scripts would inspect every historical asset.
    let requested_paths: Vec<_> = job.applied_files.keys().cloned().collect();
    let (paths, validation_scope) = validation_scope::paths(
        Path::new(&job.root),
        Path::new(&project),
        &requested_paths,
    )?;
    // Compilation and domain reload must finish outside unity_execute. Running
    // asset checks in the previous AppDomain would validate the wrong schema.
    let compilation = crate::unity_bridge::recompile_and_wait(&project).await?;
    let code = validation_scope::script(&paths, "message => print(message)");
    let output = crate::unity_bridge::unity_execute_code(&project, &code).await?;
    if !output.contains("LOCUS_MERGE_VALIDATED") {
        return Err(format!(
            "Unity did not return the validation marker: {output}"
        ));
    }
    let _lock = lock(Path::new(&job.root))?;
    let mut current = load(root, id)?;
    if current.applied_hash != job.applied_hash || current.revision != job.revision {
        return Err("stale: merge plan changed during Unity validation".into());
    }
    // Importer or serialization side effects are never silently accepted.
    check_files(Path::new(&job.root), &dir, &expected_files).map_err(|e| {
        format!("Unity produced a changed output; inspect side effects and prepare a new plan: {e}")
    })?;
    snapshot::check_dependencies(Path::new(&job.root), &dir, &job)?;
    current.unity_validated_hash = current.applied_hash.clone();
    save(&dir, &current)?;
    Ok(
        json!({"job_id":id,"plan_hash":current.applied_hash,"unity_validated":true,"placement":"applied_destination","editor_version":version,"compilation":compilation,"requested_paths":requested_paths,"asset_roots":paths,"validation_scope":validation_scope,"output":output}),
    )
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod asset_api_tests;
