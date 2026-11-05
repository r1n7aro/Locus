//! Re-prepare a job against the current destination after its snapshot went
//! stale. Source deltas are pinned to the previous job's exact commits and
//! bases, so change IDs stay the same even if the source branch moved. A
//! decision is carried over only when the destination bytes of every file it
//! touches are unchanged; the others are reported for review, never guessed.
use super::*;
use std::path::PathBuf;

/// Longer report lists are cut here; their totals stay exact.
const REPORT_LIMIT: usize = 200;

pub fn replan(project_root: &Path, id: &str) -> Result<(MergeJob, Value), String> {
    let old = load(project_root, id)?;
    if !matches!(old.state.as_str(), "prepared" | "aborted") {
        return Err(format!(
            "Cannot replan: job is {}; replan re-prepares a prepared or aborted job (abort an applied job first)",
            old.state
        ));
    }
    let root = PathBuf::from(&old.root);
    let epoch = crate::workspace_service::worktrees::record_for_root(Path::new(&old.project_root))?
        .map(|record| record.materialization_epoch)
        .unwrap_or(0);
    if epoch != old.snapshot.materialization_epoch {
        return Err("Cannot replan: the destination checkout was reassigned since this job was prepared; prepare a new job".into());
    }
    if git_text(&root, &["symbolic-ref", "--quiet", "HEAD"]).ok() != old.snapshot.branch {
        return Err("Cannot replan: the destination branch changed since this job was prepared; prepare a new job for the current branch".into());
    }
    let mut commits = BTreeSet::new();
    let sources: Vec<_> = old
        .deltas
        .iter()
        .filter(|delta| commits.insert(delta.commit.clone()))
        .map(|delta| SourceSelection {
            branch_ref: delta.source_branch.clone(),
            pinned: Some((delta.commit.clone(), delta.parent.clone())),
            ..Default::default()
        })
        .collect();
    if sources.is_empty() {
        return Err("Cannot replan: the job has no source changes; prepare a new job".into());
    }
    let mut job = prepare(
        project_root,
        &PrepareRequest {
            sources,
            project_id: old.project_id.clone(),
            paths: old.paths.clone(),
            mode: old.mode,
            eager_catalog: old.catalog_ready,
            keep_job: Some(old.id.clone()),
        },
    )?;
    let dir = job_dir(&root, &job.id)?;
    let old_dir = job_dir(&root, &old.id)?;
    let (carried, dropped, verdicts) = carry_selection(&root, &dir, &old_dir, &old, &mut job)?;
    if carried.decisions.len() + carried.files.len() + carried.objects.len() + carried.fields.len() > 0 {
        job.revision = 1;
    }
    let counts = json!({"decisions":carried.decisions.len(),"files":carried.files.len(),
        "objects":carried.objects.len(),"fields":carried.fields.len()});
    job.selection = carried;
    save(&dir, &job)?;

    let mut changed: BTreeSet<String> = verdicts
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(path, _)| path)
        .collect();
    changed.extend(
        old.deltas
            .iter()
            .map(|delta| &delta.path)
            .filter(|path| old.snapshot.files.get(*path) != job.snapshot.files.get(*path))
            .cloned(),
    );
    let mut report = json!({
        "replaced_job_id": old.id,
        "previous_state": old.state,
        "head_changed": old.snapshot.head != job.snapshot.head,
        "carried": counts,
        "dropped": dropped.iter().take(REPORT_LIMIT).collect::<Vec<_>>(),
        "dropped_total": dropped.len(),
        "changed_paths": changed.iter().take(REPORT_LIMIT).collect::<Vec<_>>(),
        "changed_paths_total": changed.len(),
    });
    // The new job is durable before the old one is retired, so a failure here
    // never loses the plan. A prepared job has no writes to restore.
    if let Err(error) = execute(project_root, &old.id, "abort", json!({})) {
        report["warnings"] = json!([format!(
            "The previous job {} could not be removed: {error}",
            old.id
        )]);
    }
    Ok((job, report))
}

type Carried = (Selection, Vec<Value>, BTreeMap<String, bool>);

fn carry_selection(
    root: &Path,
    dir: &Path,
    old_dir: &Path,
    old: &MergeJob,
    job: &mut MergeJob,
) -> Result<Carried, String> {
    let owners: BTreeMap<&str, &str> = old
        .deltas
        .iter()
        .flat_map(|delta| {
            delta
                .changes
                .iter()
                .filter_map(move |change| Some((change["id"].as_str()?, delta.path.as_str())))
        })
        .collect();
    let catalog: BTreeSet<String> = job
        .deltas
        .iter()
        .flat_map(|delta| &delta.changes)
        .filter_map(|change| change["id"].as_str().map(str::to_string))
        .collect();
    let mut verdicts = BTreeMap::new();
    let mut unchanged = |job: &mut MergeJob, path: &str| -> Result<bool, String> {
        if let Some(same) = verdicts.get(path) {
            return Ok(*same);
        }
        let same = same_destination(root, dir, old, job, path)?;
        verdicts.insert(path.to_string(), same);
        Ok(same)
    };
    let mut carried = Selection::default();
    let mut dropped = vec![];

    for (id, decision) in &old.selection.decisions {
        let path = owners.get(id.as_str()).copied();
        let reason = match path {
            None => Some("change_not_in_previous_catalog"),
            Some(path) => {
                if !unchanged(job, path)? {
                    Some("destination_changed")
                } else if !catalog.contains(id) {
                    Some("change_not_in_new_catalog")
                } else {
                    None
                }
            }
        };
        match reason {
            None => {
                carried.decisions.insert(id.clone(), decision.clone());
            }
            Some(reason) => dropped.push(json!({"selection":"decision","change_id":id,
                "path":path,"decision":decision,"reason":reason})),
        }
    }
    for (key, choice) in &old.selection.files {
        let mut same = unchanged(job, &choice.path)?;
        if let Some(destination) = &choice.destination {
            same &= unchanged(job, destination)?;
        }
        if !same {
            dropped.push(json!({"selection":"file","path":choice.path,"version":choice.version,
                "destination":choice.destination,"reason":"destination_changed"}));
            continue;
        }
        // Jobs share a blob store; this only copies text a v1 job kept locally.
        if let Some(content) = &choice.content {
            if read_blob(dir, content).is_err() {
                store_blob(dir, &read_blob(old_dir, content)?, &content.mode)?;
            }
        }
        carried.files.insert(key.clone(), choice.clone());
    }
    for (key, choice) in &old.selection.objects {
        if unchanged(job, &choice.path)? {
            carried.objects.insert(key.clone(), choice.clone());
        } else {
            dropped.push(json!({"selection":"object","path":choice.path,"object_id":choice.object_id,
                "operation":choice.operation,"reason":"destination_changed"}));
        }
    }
    for (key, choice) in &old.selection.fields {
        if unchanged(job, &choice.path)? {
            carried.fields.insert(key.clone(), choice.clone());
        } else {
            dropped.push(json!({"selection":"field","path":choice.path,"object_id":choice.object_id,
                "property_path":choice.property_path,"reason":"destination_changed"}));
        }
    }
    Ok((carried, dropped, verdicts))
}

/// Whether `path` has identical destination bytes and mode in both snapshots.
/// A path the old job captured for an explicit file operation is captured the
/// same way, so the comparison never reads anything but the current file.
fn same_destination(
    root: &Path,
    dir: &Path,
    old: &MergeJob,
    job: &mut MergeJob,
    path: &str,
) -> Result<bool, String> {
    let Some(before) = old.snapshot.files.get(path) else {
        return Ok(false);
    };
    if !job.snapshot.files.contains_key(path) {
        if !scope_allows(job, path) {
            return Ok(false);
        }
        job.snapshot.files.insert(path.to_string(), capture(root, dir, path)?);
    }
    Ok(job.snapshot.files.get(path) == Some(before))
}
