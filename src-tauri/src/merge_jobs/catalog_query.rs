//! Server-side views of the immutable change catalog. A large scene can hold
//! tens of thousands of clean changes; an agent reads the per-file summary and
//! the changes that need a decision instead of transferring the whole catalog.
use super::types::{FileState, MergeJob};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const STATUSES: &[&str] = &["clean", "conflict", "already_applied", "explicit_choice"];

fn strings(params: &Value, key: &str) -> Result<Option<BTreeSet<String>>, String> {
    let invalid = || format!("{key} must be a string or a list of strings");
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(BTreeSet::from([value.clone()]))),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or_else(invalid))
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(invalid()),
    }
}

fn page(params: &Value, default: u64) -> (usize, usize) {
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(default).clamp(1, 1000) as usize;
    (offset, limit)
}

fn next_offset(offset: usize, limit: usize, total: usize) -> Option<usize> {
    offset.checked_add(limit).filter(|next| *next < total)
}

/// One catalog page. `status` and `paths` filter on the server; `total`
/// counts the matching changes and `catalog_total` the whole catalog, so a
/// filtered read never hides how much exists.
pub(super) fn changes(job: &MergeJob, params: &Value) -> Result<Value, String> {
    let statuses = strings(params, "status")?;
    if let Some(unknown) = statuses.iter().flatten().find(|s| !STATUSES.contains(&s.as_str())) {
        return Err(format!("Unknown change status `{unknown}`; use one of {}", STATUSES.join(", ")));
    }
    let paths = strings(params, "paths")?;
    let (offset, limit) = page(params, 100);
    let all = job.deltas.iter().flat_map(|d| &d.changes);
    let catalog_total = all.clone().count();
    let matching: Vec<&Value> = all
        .filter(|change| {
            statuses.as_ref().map_or(true, |s| change["status"].as_str().is_some_and(|v| s.contains(v)))
                && paths.as_ref().map_or(true, |p| change["path"].as_str().is_some_and(|v| p.contains(v)))
        })
        .collect();
    Ok(json!({
        "changes": matching.iter().skip(offset).take(limit).collect::<Vec<_>>(),
        "total": matching.len(),
        "catalog_total": catalog_total,
        "next_offset": next_offset(offset, limit, matching.len()),
    }))
}

/// Byte identity across the two blob namespaces: Git object IDs from trees
/// and BLAKE3 digests of frozen working files.
fn same_bytes(dir: &Path, a: &Option<FileState>, b: &Option<FileState>) -> Result<bool, String> {
    match (a, b) {
        (None, None) => Ok(true),
        (Some(a), Some(b)) if a.blob == b.blob => Ok(true),
        (Some(a), Some(b)) if a.blob.starts_with("git-") == b.blob.starts_with("git-") => Ok(false),
        (Some(a), Some(b)) => Ok(super::read_blob(dir, a)? == super::read_blob(dir, b)?),
        _ => Ok(false),
    }
}

fn bump(counts: &mut Map<String, Value>, key: &str) {
    let count = counts.get(key).and_then(Value::as_u64).unwrap_or(0) + 1;
    counts.insert(key.to_string(), json!(count));
}

/// Per-file rollup of the catalog. `destination_changed=false` means the
/// destination still holds the base of the first selected commit touching
/// that path; for a single delta, including all of its changes reproduces the
/// source file exactly.
pub(super) fn summary(dir: &Path, job: &MergeJob, params: &Value) -> Result<Value, String> {
    let (offset, limit) = page(params, 200);
    let mut files: BTreeMap<&str, Value> = BTreeMap::new();
    let mut statuses = Map::new();
    let mut total = 0usize;
    for delta in &job.deltas {
        let entry = files.entry(delta.path.as_str()).or_insert_with(|| {
            json!({"path":delta.path,"file_kind":delta.kind,"commits":0,"changes":0,
                "statuses":{},"kinds":{},"destination_changed":false,"outside_destination_scope":false})
        });
        entry["commits"] = json!(entry["commits"].as_u64().unwrap_or(0) + 1);
        entry["changes"] = json!(entry["changes"].as_u64().unwrap_or(0) + delta.changes.len() as u64);
        for change in &delta.changes {
            let status = change["status"].as_str().unwrap_or("unknown");
            bump(entry["statuses"].as_object_mut().ok_or("summary statuses")?, status);
            bump(entry["kinds"].as_object_mut().ok_or("summary kinds")?, change["kind"].as_str().unwrap_or("unknown"));
            bump(&mut statuses, status);
            if change["outside_destination_scope"] == true {
                entry["outside_destination_scope"] = json!(true);
            }
        }
        total += delta.changes.len();
    }
    // Only the returned page pays for the destination comparison.
    let page: Vec<(&str, Value)> = files.into_iter().skip(offset).take(limit).collect();
    let mut out = Vec::with_capacity(page.len());
    for (path, mut entry) in page {
        let target = job.snapshot.files.get(path).cloned().unwrap_or(None);
        let first = job.deltas.iter().find(|d| d.path == path).ok_or("summary lost a delta")?;
        entry["destination_changed"] = json!(!same_bytes(dir, &first.base, &target)?);
        out.push(entry);
    }
    let file_total = job.deltas.iter().map(|d| d.path.as_str()).collect::<BTreeSet<_>>().len();
    Ok(json!({
        "files": out,
        "file_total": file_total,
        "changes": total,
        "statuses": statuses,
        "next_offset": next_offset(offset, limit, file_total),
    }))
}
