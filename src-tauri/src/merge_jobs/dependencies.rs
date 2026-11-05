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
    /// GUIDs a lexically scanned asset names in `guid` keys it could not pair
    /// with a fileID: it may reference these assets in an unknown way.
    #[serde(default)]
    pub unresolved_guids: BTreeSet<String>,
    /// Such a key had no value on its line, so it may name any GUID.
    #[serde(default)]
    pub unresolved_any: bool,
    /// Only tagged, writable Unity document streams prove their local object IDs.
    pub document_ids: Option<BTreeSet<String>>,
    pub external_objects: Vec<ExternalObjectReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalObjectReference {
    pub guid: String,
    pub file_id: String,
    pub object_id: String,
    /// Exact pointer from the parser; empty for a lexically found reference.
    pub property_path: String,
    /// Byte offset of a lexically found reference, located when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}
impl References {
    /// Which `affected` GUIDs this asset may reference in an unresolved form.
    /// `spelled` lists the affected GUIDs its bytes contain, read only when a
    /// value-less key makes every GUID possible.
    pub fn unresolved(
        &self,
        affected: &BTreeSet<String>,
        spelled: impl FnOnce() -> Result<BTreeSet<String>, String>,
    ) -> Result<BTreeSet<String>, String> {
        if affected.is_empty() {
            Ok(BTreeSet::new())
        } else if self.unresolved_any {
            spelled()
        } else {
            Ok(self.unresolved_guids.intersection(affected).cloned().collect())
        }
    }
}

fn project_guid(guid: &str) -> bool {
    guid.len() == 32
        && guid.bytes().all(|b| b.is_ascii_hexdigit())
        && !guid.starts_with("0000000000000000")
}

/// Finds affected GUIDs spelled anywhere in an asset. A Unity reference to
/// another asset always spells that asset's GUID (a hex token is never split
/// across lines), so an asset naming none of them references none of their
/// objects and needs no parse. One case-insensitive multi-literal search per
/// file, compiled once per check.
pub struct GuidMatcher {
    regex: Option<regex::bytes::Regex>,
    guids: Vec<String>,
}
impl GuidMatcher {
    pub fn new<'a>(guids: impl IntoIterator<Item = &'a String>) -> Self {
        let guids: Vec<String> = guids
            .into_iter()
            .map(|g| g.to_ascii_lowercase())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let pattern = format!(
            "(?i-u){}",
            guids.iter().map(|g| regex::escape(g)).collect::<Vec<_>>().join("|")
        );
        let regex = (!guids.is_empty())
            .then(|| regex::bytes::RegexBuilder::new(&pattern).size_limit(256 << 20).build().ok())
            .flatten();
        Self { regex, guids }
    }
    pub fn is_empty(&self) -> bool {
        self.guids.is_empty()
    }
    pub fn any(&self, bytes: &[u8]) -> bool {
        match &self.regex {
            Some(regex) => regex.is_match(bytes),
            None => !self.mentioned(bytes).is_empty(),
        }
    }
    /// The affected GUIDs (lowercase) that `bytes` spells.
    pub fn mentioned(&self, bytes: &[u8]) -> BTreeSet<String> {
        match &self.regex {
            Some(regex) => regex
                .find_iter(bytes)
                .map(|m| String::from_utf8_lossy(m.as_bytes()).to_ascii_lowercase())
                .collect(),
            None if self.guids.is_empty() => BTreeSet::new(),
            // A pattern beyond the regex size budget: exact but slower.
            None => {
                let text = bytes.to_ascii_lowercase();
                self.guids
                    .iter()
                    .filter(|g| text.windows(g.len()).any(|w| w == g.as_bytes()))
                    .cloned()
                    .collect()
            }
        }
    }
}

/// Current bytes of a dependency, materializing a Git LFS pointer from the
/// local LFS cache. `Err` explains why the content cannot be checked.
pub fn dependency_bytes(root: &Path, dir: &Path, state: &FileState) -> Result<Result<(FileState, Vec<u8>), String>, String> {
    let bytes = read_blob(dir, state)?;
    if !is_lfs_pointer(&bytes) {
        return Ok(Ok((state.clone(), bytes)));
    }
    Ok(match materialize_lfs(root, dir, state) {
        Ok(Materialized::Complete(complete)) => read_blob(dir, &complete).map(|bytes| (complete, bytes)),
        Ok(Materialized::Missing { oid, .. }) => Err(format!(
            "its Git LFS object {oid} is not in the local LFS cache; fetch it (`git lfs fetch`) and preview again"
        )),
        Err(error) => Err(error),
    })
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
        .join(super::retention::reference_cache());
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
                        offset: None,
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
                unresolved_guids: BTreeSet::new(),
                unresolved_any: false,
                document_ids,
                external_objects,
            }
        }
        // Beyond the parser's budgets or dialect: references only, no tree.
        Err(_) => lexical_references(&bytes),
    });
    cache_json(&path, &summary)?;
    Ok(summary)
}

fn lexical_references(bytes: &[u8]) -> References {
    let scan = crate::unity_asset_core::scan_references(bytes);
    References {
        own_guid: scan.own_guid.map(|guid| guid.trim().to_ascii_lowercase()),
        guids: scan.references.iter().map(|r| r.guid.to_ascii_lowercase()).collect(),
        unresolved_guids: scan.unrecognized.iter().filter_map(|u| u.guid.clone()).collect(),
        unresolved_any: scan.unrecognized.iter().any(|u| u.guid.is_none()),
        document_ids: None,
        external_objects: scan
            .references
            .iter()
            .filter_map(|reference| {
                let guid = reference.guid.to_ascii_lowercase();
                let file_id = reference.file_id.parse::<i64>().ok()?;
                (project_guid(&guid) && file_id != 0).then(|| ExternalObjectReference {
                    guid,
                    file_id: file_id.to_string(),
                    object_id: reference.host_object_id.clone(),
                    property_path: String::new(),
                    offset: Some(reference.offset),
                })
            })
            .collect(),
    }
}
pub fn owners(
    dir: &Path,
    files: &BTreeMap<String, Option<FileState>>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let metas: BTreeMap<_, _> = files.iter().filter(|(p,_)|p.ends_with(".meta")).collect();
    let key = blake3::hash(&serde_json::to_vec(&metas).map_err(|e|e.to_string())?);
    let folder = dir.parent().ok_or("Missing GUID cache parent")?.join(super::retention::GUID_INDEX);
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
    root: &Path,
    dir: &Path,
    target: &BTreeMap<String, Option<FileState>>,
    result: &BTreeMap<String, Option<FileState>>,
    changed: &BTreeMap<String, Option<FileState>>,
    target_owners: &BTreeMap<String, Vec<String>>,
    result_owners: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut removed: BTreeMap<(String, String), String> = BTreeMap::new();
    // Unity cannot serialize a reference from another asset to an object inside
    // a scene (cross-scene/asset-to-scene references are unsupported), so
    // removing scene objects never breaks another asset.
    for (path, after) in changed.iter().filter(|(path, _)| {
        !path.ends_with(".meta") && !path.ends_with(".unity") && !super::opaque(path)
    }) {
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
    let affected: BTreeSet<String> = removed.keys().map(|(guid, _)| guid.clone()).collect();
    let matcher = GuidMatcher::new(&affected);
    prefetch_git(dir, result.iter().filter(|(path,_)|super::snapshot::evidence(path)
        && !path.ends_with(".meta") && !path.ends_with(".cs") && !path.ends_with(".asmdef"))
        .filter_map(|(_,s)|s.as_ref()))?;
    let check = |path: &String, found: Mention| -> Result<Vec<serde_json::Value>, String> {
        let state = match found {
            Mention::Unreadable(reason) => {
                return Ok(vec![super::error_value("external_object_dependency_coverage", path,
                    format!("Cannot check this asset's references to removed objects: {reason}"))]);
            }
            Mention::Spells(state) => state,
        };
        let summary = references(dir, &state)?;
        // Bytes are re-read only to describe an issue, under their own budget.
        let read = || super::parallel::with_bytes(blob_len(dir, &state).unwrap_or(0), || read_blob(dir, &state));
        let mut issues = vec![];
        let unresolved = summary.unresolved(&affected, || Ok(matcher.mentioned(&read()?)))?;
        if !unresolved.is_empty() {
            let deleted_from: BTreeSet<&String> = removed
                .iter()
                .filter(|((guid, _), _)| unresolved.contains(guid))
                .map(|(_, from)| from)
                .collect();
            issues.push(serde_json::json!({
                "code":"external_object_dependency_coverage", "path":path,
                "guids":unresolved, "deleted_from":deleted_from,
                "detail":"This asset names an asset whose Unity objects are being removed in a `guid` key outside a recognizable `{fileID, guid}` reference, so its references to them cannot be checked; exclude those object removals in deleted_from, or take a version of this asset in Unity's text format"
            }));
        }
        let hits: Vec<_> = summary
            .external_objects
            .iter()
            .filter_map(|r| removed.get(&(r.guid.clone(), r.file_id.clone())).map(|from| (r, from)))
            .collect();
        let bytes = if hits.iter().any(|(r, _)| r.offset.is_some()) { read()? } else { Vec::new() };
        for (reference, deleted_from) in hits {
            let mut issue = serde_json::json!({
                "code":"removed_object_dependency", "path":path,
                "object_id":reference.object_id, "property_path":reference.property_path,
                "guid":reference.guid, "file_id":reference.file_id, "deleted_from":deleted_from,
                "detail":format!("Reference {}/{} still targets a document removed from {}; explicitly revise or exclude the deletion", reference.guid, reference.file_id, deleted_from)
            });
            if let Some(offset) = reference.offset {
                // Found by the lexical scan of an asset beyond the parser.
                let (line, property_path) = crate::unity_asset_core::reference_location(&bytes, offset);
                issue["line"] = serde_json::json!(line);
                issue["property_path"] = serde_json::json!(property_path);
                issue["property_path_approximate"] = serde_json::json!(true);
            }
            issues.push(issue);
        }
        Ok(issues)
    };
    let mentions = spelling_assets(root, dir, result, &|_| true, &matcher)?;
    let issues = super::parallel::pool().install(|| {
        mentions
            .into_par_iter()
            .map(|(path, found)| check(path, found))
            .collect::<Result<Vec<_>, String>>()
    })?;
    Ok(issues.into_iter().flatten().collect())
}

/// An asset whose current bytes spell an affected GUID (its state, with any
/// Git LFS pointer materialized), or one whose bytes cannot be checked.
pub enum Mention {
    Spells(FileState),
    Unreadable(String),
}

/// Reference candidates in `files` that spell a GUID of `matcher`, in path
/// order. One budgeted read and multi-literal search per file; nothing is
/// parsed here, so huge or unparsable assets cost a linear scan at most.
pub fn spelling_assets<'a>(
    root: &Path,
    dir: &Path,
    files: &'a BTreeMap<String, Option<FileState>>,
    include: &(dyn Fn(&str) -> bool + Sync),
    matcher: &GuidMatcher,
) -> Result<Vec<(&'a String, Mention)>, String> {
    let found = super::parallel::pool().install(|| {
        files
            .par_iter()
            .map(|(path, state)| -> Result<Option<(&String, Mention)>, String> {
                let Some(state) = state.as_ref().filter(|_| reference_candidate(path) && include(path)) else {
                    return Ok(None);
                };
                let size = blob_len(dir, state).unwrap_or(0);
                super::parallel::with_bytes(size, || {
                    Ok(match dependency_bytes(root, dir, state)? {
                        Err(reason) => Some((path, Mention::Unreadable(reason))),
                        Ok((state, bytes)) => (super::unity_yaml(path, &bytes) && matcher.any(&bytes))
                            .then_some((path, Mention::Spells(state))),
                    })
                })
            })
            .collect::<Result<Vec<_>, String>>()
    })?;
    Ok(found.into_iter().flatten().collect())
}
