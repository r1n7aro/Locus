//! Several selected commits of one file apply in order, like a sequence of
//! cherry-picks: each later commit meets the file as the earlier ones leave
//! it. The catalog of a later commit is therefore computed against the file
//! with the earlier commits included in full (clean changes applied, conflicts
//! left at the destination), which is what `include` of the whole selection
//! produces.
use super::*;
use crate::unity_asset_core::{Decision, Resolution};

fn with_mode(state: &Option<FileState>, target: &Option<FileState>) -> Option<FileState> {
    state.clone().map(|mut state| {
        if let Some(target) = target {
            state.mode = target.mode.clone();
        }
        state
    })
}

/// The file after `delta` is included in full over `target`.
pub(super) fn included_result(
    dir: &Path,
    delta: &CommitDelta,
    target: &Option<FileState>,
    changes: &[Value],
    schemas: &BTreeMap<String, schema::SchemaMap>,
) -> Result<Option<FileState>, String> {
    let status = |c: &Value| c["status"].as_str().unwrap_or_default().to_string();
    let content: Vec<&Value> = changes.iter().filter(|c| c["kind"] != "file_mode").collect();
    if content.iter().all(|c| status(c) == "already_applied") {
        return Ok(target.clone());
    }
    let base = state_bytes(dir, &delta.base)?;
    let current = state_bytes(dir, target)?;
    let untouched = target.is_some() == delta.base.is_some() && current == base;
    if untouched && content.iter().all(|c| status(c) != "conflict") {
        return Ok(with_mode(&delta.source, target));
    }
    if let Some(file) = content.iter().find(|c| c["core_id"].is_null()) {
        if status(file) != "clean" {
            return Ok(target.clone());
        }
        if delta.kind == "text" && delta.base.is_some() && target.is_some() && delta.source.is_some() {
            let source = state_bytes(dir, &delta.source)?;
            let mode = target.as_ref().map(|s| s.mode.as_str()).unwrap_or("100644");
            return match text_merge::merge(&base, &current, &source)?.bytes(&current) {
                Some(bytes) => Ok(Some(store_blob(dir, &bytes, mode)?)),
                None => Ok(target.clone()),
            };
        }
        return Ok(with_mode(&delta.source, target));
    }
    let source = state_bytes(dir, &delta.source)?;
    let empty = schema::SchemaMap::new();
    let aliases = schema::aliases(
        &current,
        schemas.get(&delta.parent).unwrap_or(&empty),
        schemas.get(&delta.commit).unwrap_or(&empty),
        schemas.get("target").unwrap_or(&empty),
    );
    let session = merge_session(&base, &current, &source, &aliases).map_err(|e| e.to_string())?;
    let decisions: Vec<Decision> = content
        .iter()
        .filter(|c| status(c) == "clean")
        .filter_map(|c| c["core_id"].as_str())
        .map(|id| Decision { change_id: id.to_string(), resolution: Resolution::Include })
        .collect();
    let output = session.render(&decisions).map_err(|e| e.to_string())?;
    if !output.ready {
        return Ok(target.clone());
    }
    let mode = target.as_ref().map(|s| s.mode.as_str()).unwrap_or("100644");
    Ok(Some(store_blob(dir, &output.bytes, mode)?))
}
