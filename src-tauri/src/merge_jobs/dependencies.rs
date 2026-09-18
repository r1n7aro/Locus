use super::io::*;
use super::types::FileState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use rayon::prelude::*;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct References {
    pub own_guid: Option<String>,
    pub guids: BTreeSet<String>,
    pub parsed: bool,
    /// Only tagged, writable Unity document streams prove their local object IDs.
    pub document_ids: Option<BTreeSet<String>>,
    pub external_objects: Vec<ExternalObjectReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalObjectReference {
    pub guid: String,
    pub file_id: String,
    pub object_id: String,
    pub property_path: String,
}
fn project_guid(guid: &str) -> bool {
    guid.len() == 32
        && guid.bytes().all(|b| b.is_ascii_hexdigit())
        && !guid.starts_with("0000000000000000")
}

pub fn reference_candidate(path: &str) -> bool {
    !super::opaque(path) && !path.ends_with(".cs") && !path.ends_with(".asmdef")
        && !path.ends_with(".asmref") && !path.ends_with(".json")
        && !path.ends_with("ProjectVersion.txt")
}
pub fn references(dir: &Path, state: &FileState) -> Result<References, String> {
    let folder = dir
        .parent()
        .ok_or("Missing reference cache parent")?
        .join(format!(
            "reference-cache-{}-objects-v2",
            crate::unity_asset_core::PARSER_VERSION
        ));
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!("{}.json", state.blob));
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(cached) = serde_json::from_slice(&bytes) {
            return Ok(cached);
        }
    }
    let bytes = read_blob(dir, state)?;
    let summary = super::parallel::with_bytes((bytes.len() as u64).saturating_mul(16), || match crate::unity_asset_core::parse_shared(&bytes) {
        Ok(asset) => {
            let references = asset.references();
            let document_ids = (asset.is_writable()
                && !asset.documents.is_empty()
                && asset.documents.iter().all(|doc| doc.class_id.is_some()))
            .then(|| {
                asset
                    .documents
                    .iter()
                    .map(|doc| doc.object_id.parse::<i64>().ok().map(|id| id.to_string()))
                    .collect::<Option<BTreeSet<_>>>()
            })
            .flatten();
            let external_objects = references
                .iter()
                .filter_map(|reference| {
                    if reference.kind != crate::unity_asset_core::ReferenceKind::ExternalObject {
                        return None;
                    }
                    let guid = reference.guid.as_ref()?.to_ascii_lowercase();
                    let file_id = reference.file_id.as_ref()?.parse::<i64>().ok()?;
                    if !project_guid(&guid) || file_id == 0 {
                        return None;
                    }
                    Some(ExternalObjectReference {
                        guid,
                        file_id: file_id.to_string(),
                        object_id: reference.host_object_id.clone(),
                        property_path: reference.property_path.clone(),
                    })
                })
                .collect();
            References {
                own_guid: asset
                    .documents
                    .first()
                    .and_then(|d| d.root.get("guid"))
                    .and_then(|n| n.scalar(&asset))
                    .map(|s| s.trim().to_ascii_lowercase()),
                guids: references
                    .iter()
                    .filter_map(|r| r.guid.as_ref().map(|g| g.to_ascii_lowercase()))
                    .collect(),
                parsed: true,
                document_ids,
                external_objects,
            }
        }
        Err(_) => References::default(),
    });
    cache_json(&path, &summary)?;
    Ok(summary)
}
pub fn owners(
    dir: &Path,
    files: &BTreeMap<String, Option<FileState>>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let metas: BTreeMap<_, _> = files.iter().filter(|(p,_)|p.ends_with(".meta")).collect();
    let key = blake3::hash(&serde_json::to_vec(&metas).map_err(|e|e.to_string())?);
    let folder = dir.parent().ok_or("Missing GUID cache parent")?.join("guid-index-v2");
    std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let cache = folder.join(format!("{key}.json"));
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(owners) = serde_json::from_slice(&bytes) { return Ok(owners); }
    }
    prefetch_git(dir, metas.values().filter_map(|s|s.as_ref()))?;
    let entries: Result<Vec<_>, String> = super::parallel::pool().install(|| metas.par_iter().map(|(path, state)| {
        let Some(state) = state else { return Ok(None); };
        let bytes = read_blob(dir, state)?;
        let guid = std::str::from_utf8(&bytes).ok().and_then(|text|text.lines()
            .find_map(|line|line.strip_prefix("guid:").map(str::trim)))
            .filter(|guid|guid.len()==32 && guid.bytes().all(|b|b.is_ascii_hexdigit()))
            .map(|guid|guid.to_ascii_lowercase());
        let guid = match guid { Some(guid)=>Some(guid), None=>references(dir,state)?.own_guid };
        Ok(guid.map(|guid|(guid,(*path).clone())))
    }).collect());
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (guid,path) in entries?.into_iter().flatten() {
        owners.entry(guid).or_default().push(path);
    }
    cache_json(&cache,&owners)?;
    Ok(owners)
}

/// A selected object deletion can break an unchanged asset while its owner GUID
/// remains valid. Check the final frozen graph, never implicitly select fixes.
pub fn removed_object_issues(
    dir: &Path,
    target: &BTreeMap<String, Option<FileState>>,
    result: &BTreeMap<String, Option<FileState>>,
    changed: &BTreeMap<String, Option<FileState>>,
    target_owners: &BTreeMap<String, Vec<String>>,
    result_owners: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut removed: BTreeMap<(String, String), String> = BTreeMap::new();
    for (path, after) in changed
        .iter()
        .filter(|(path, _)| !path.ends_with(".meta") && !super::opaque(path))
    {
        let (Some(before), Some(after)) =
            (target.get(path).and_then(Option::as_ref), after.as_ref())
        else {
            // Whole asset removals already use the GUID/meta dependency checks.
            continue;
        };
        if before.blob == after.blob {
            continue;
        }
        let meta_path = format!("{path}.meta");
        let (Some(before_meta), Some(after_meta)) = (
            target.get(&meta_path).and_then(Option::as_ref),
            result.get(&meta_path).and_then(Option::as_ref),
        ) else {
            continue;
        };
        let Some(guid) = references(dir, before_meta)?
            .own_guid
            .filter(|guid| project_guid(guid))
        else {
            continue;
        };
        if references(dir, after_meta)?.own_guid.as_ref() != Some(&guid)
            || !target_owners
                .get(&guid)
                .is_some_and(|owners| owners.as_slice() == [meta_path.as_str()])
            || !result_owners
                .get(&guid)
                .is_some_and(|owners| owners.as_slice() == [meta_path.as_str()])
        {
            // Changed/ambiguous GUID ownership does not prove which object's
            // identity survived; the GUID-level checker handles that case.
            continue;
        }
        let (Some(before_ids), Some(after_ids)) = (
            references(dir, before)?.document_ids,
            references(dir, after)?.document_ids,
        ) else {
            continue;
        };
        for id in before_ids.difference(&after_ids) {
            removed.insert((guid.clone(), id.clone()), path.clone());
        }
    }
    if removed.is_empty() {
        return Ok(vec![]);
    }
    prefetch_git(dir, result.iter().filter(|(path,_)|super::snapshot::evidence(path)
        && !path.ends_with(".meta") && !path.ends_with(".cs") && !path.ends_with(".asmdef"))
        .filter_map(|(_,s)|s.as_ref()))?;
    let mut issues = vec![];
    for (path, state) in result {
        let Some(state) = state else {
            continue;
        };
        if !reference_candidate(path) { continue; }
        let bytes = read_blob(dir, state)?;
        if !super::unity_yaml(path, &bytes) {
            if is_lfs_pointer(&bytes) {
                issues.push(super::error_value("external_object_dependency_coverage",path,
                    "A dependency is an unmaterialized LFS pointer while object IDs are removed"));
            }
            continue;
        }
        let summary = references(dir, state)?;
        if !summary.parsed {
            issues.push(super::error_value("external_object_dependency_coverage", path,
                "Cannot prove this asset's references while Unity document IDs are being removed; inspect this asset explicitly"));
            continue;
        }
        for reference in &summary.external_objects {
            if let Some(deleted_from) =
                removed.get(&(reference.guid.clone(), reference.file_id.clone()))
            {
                issues.push(serde_json::json!({
                    "code":"removed_object_dependency", "path":path,
                    "object_id":reference.object_id, "property_path":reference.property_path,
                    "guid":reference.guid, "file_id":reference.file_id, "deleted_from":deleted_from,
                    "detail":format!("Reference {}/{} still targets a document removed from {}; explicitly revise or exclude the deletion", reference.guid, reference.file_id, deleted_from)
                }));
            }
        }
    }
    Ok(issues)
}
