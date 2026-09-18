//! Exact commit-scope validation. A validation result is bound to the actual
//! Git tree, applied plan, parent and explicitly acknowledged local scope.
use super::*;

pub(super) fn build_tree(
    root: &Path,
    dir: &Path,
    head: &str,
    outputs: &BTreeMap<String, Option<FileState>>,
) -> Result<String, String> {
    let index = dir.join(format!("candidate-index-{}", uuid::Uuid::new_v4().simple()));
    git_input(root, &["read-tree", head], b"", Some(&index))?;
    let bytes = prepare_index(
        root,
        dir,
        &std::fs::read(&index).map_err(|e| e.to_string())?,
        outputs,
        &outputs.keys().cloned().collect::<Vec<_>>(),
    )?;
    std::fs::write(&index, bytes).map_err(|e| e.to_string())?;
    git_input(root, &["write-tree"], b"", Some(&index))
}
pub(super) fn key(
    job: &MergeJob,
    tree: &str,
    paths: &[String],
    include_local: bool,
) -> Result<String, String> {
    Ok(blake3::hash(
        &serde_json::to_vec(&(
            job.applied_hash
                .as_ref()
                .ok_or("Plan has not been applied")?,
            &job.snapshot.head,
            tree,
            paths,
            include_local,
        ))
        .map_err(|e| e.to_string())?,
    )
    .to_hex()
    .to_string())
}
pub(super) fn covers(
    job: &MergeJob,
    tree: &str,
    paths: &[String],
    include_local: bool,
) -> Result<bool, String> {
    let key = key(job, tree, paths, include_local)?;
    Ok(job
        .commit_validations
        .get(&key)
        .map(|v| {
            v.validated
                && v.tree == tree
                && Some(&v.plan_hash) == job.applied_hash.as_ref()
                && v.parent == job.snapshot.head
                && v.paths == paths
                && v.include_local_changes == include_local
        })
        .unwrap_or(false))
}

fn prepare(
    project: &Path,
    id: &str,
    params: &Value,
) -> Result<(MergeJob, CommitValidation), String> {
    let mut job = load(project, id)?;
    let root = Path::new(&job.root);
    let _lock = lock(root)?;
    let dir = job_dir(root, id)?;
    if !matches!(job.state.as_str(), "applied" | "staged") {
        return Err("Apply the merge plan before validating its separate commit scope".into());
    }
    check_destination(root, &job)?;
    snapshot::check_dependencies(root, &dir, &job)?;
    if git_text(root, &["rev-parse", "HEAD"])? != job.snapshot.head {
        return Err("stale: HEAD changed before candidate validation".into());
    }
    let paths = chosen_paths(&job, params)?;
    let files = scoped_files(&job);
    check_files(root, &dir, &files)?;
    let outputs: BTreeMap<_, _> = paths
        .iter()
        .map(|p| (p.clone(), files[p].clone()))
        .collect();
    validate_commit_dependencies(root, &dir, &job, &outputs)?;
    let tree = build_tree(root, &dir, &job.snapshot.head, &outputs)?;
    let include_local = params
        .get("include_local_changes")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let key = key(&job, &tree, &paths, include_local)?;
    if let Some(existing) = job.commit_validations.get(&key).filter(|v| v.validated) {
        return Ok((job.clone(), existing.clone()));
    }
    let validation = CommitValidation {
        key: key.clone(),
        tree,
        parent: job.snapshot.head.clone(),
        plan_hash: job
            .applied_hash
            .clone()
            .ok_or("Applied plan hash missing")?,
        paths,
        include_local_changes: include_local,
        validated: false,
        details: json!({"state":"pending"}),
    };
    job.commit_validations.insert(key, validation.clone());
    save(&dir, &job)?;
    Ok((job, validation))
}

pub async fn validate_commit_unity(
    app: &tauri::AppHandle,
    project: &Path,
    id: &str,
    params: Value,
) -> Result<Value, String> {
    let source = project.to_path_buf();
    let job_id = id.to_string();
    let (job, mut validation) =
        tokio::task::spawn_blocking(move || prepare(&source, &job_id, &params))
            .await
            .map_err(|e| e.to_string())??;
    if validation.validated {
        return Ok(
            json!({"job_id":id,"plan_hash":validation.plan_hash,"tree":validation.tree,"paths":validation.paths,"unity_validated":true,"placement":"isolated_commit_candidate","idempotent":true,"details":validation.details}),
        );
    }
    let outcome = super::scratch_validation::validate(
        app,
        Path::new(&job.project_root),
        &validation.tree,
        &validation.parent,
        &validation.paths,
    )
    .await;
    let _lock = lock(Path::new(&job.root))?;
    let dir = job_dir(Path::new(&job.root), id)?;
    let mut current = load(project, id)?;
    if current.applied_hash.as_deref() != Some(&validation.plan_hash)
        || current.revision != job.revision
    {
        return Err("stale: plan changed while validating the commit candidate".into());
    }
    check_destination(Path::new(&job.root), &current)?;
    if git_text(Path::new(&job.root), &["rev-parse", "HEAD"])? != validation.parent {
        return Err("stale: destination HEAD changed while validating the candidate".into());
    }
    check_files(Path::new(&job.root), &dir, &scoped_files(&current))?;
    match outcome {
        Ok(details) => {
            if details["validated_tree"].as_str() != Some(&validation.tree) {
                return Err("Unity validation returned a different candidate tree".into());
            }
            validation.validated = true;
            validation.details = details;
            current
                .commit_validations
                .insert(validation.key.clone(), validation.clone());
            save(&dir, &current)?;
            Ok(
                json!({"job_id":id,"plan_hash":validation.plan_hash,"tree":validation.tree,"paths":validation.paths,"include_local_changes":validation.include_local_changes,"unity_validated":true,"placement":"isolated_commit_candidate","details":validation.details}),
            )
        }
        Err(error) => {
            validation.details = json!({"state":"failed","error":error});
            current
                .commit_validations
                .insert(validation.key.clone(), validation);
            save(&dir, &current)?;
            Err(error)
        }
    }
}
