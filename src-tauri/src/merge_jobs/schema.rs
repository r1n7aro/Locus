//! Schema evidence comes from immutable source bytes for each tree, never from
//! the active Editor's previous assembly. Only directly declared, unambiguous
//! FormerlySerializedAs aliases with identical declared types are inferred.
use super::io::*;
use super::types::FileState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use rayon::prelude::*;
use std::collections::BTreeSet;

fn persisted_script(dir: &Path, path: &str, bytes: &[u8]) -> Result<Option<ScriptSchema>, String> {
    let folder = dir.parent().ok_or("Missing schema cache parent")?.join("script-cache-v2");
    std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let key = blake3::hash(&serde_json::to_vec(&(path, blake3::hash(bytes).to_hex().as_str())).map_err(|e|e.to_string())?);
    let cache = folder.join(format!("{key}.json"));
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(parsed) = serde_json::from_slice(&bytes) { return Ok(parsed); }
    }
    let parsed = script(path, bytes);
    cache_json(&cache, &parsed)?;
    Ok(parsed)
}

pub fn working_for_guids(dir: &Path, files: &BTreeMap<String, Option<FileState>>, guids: &BTreeSet<String>) -> Result<SchemaMap, String> {
    if guids.is_empty() { return Ok(BTreeMap::new()); }
    prefetch_git(dir, files.iter().filter(|(p,_)|p.ends_with(".cs.meta")).filter_map(|(_,s)|s.as_ref()))?;
    let candidates: Result<Vec<_>, String> = super::parallel::pool().install(|| files.par_iter()
        .filter(|(p,_)|p.ends_with(".cs.meta"))
        .map(|(path, state)| {
            let Some(state) = state else { return Ok(None); };
            let guid = meta_guid(&read_blob(dir, state)?).filter(|g|guids.contains(g));
            Ok(guid.and_then(|guid| files.get(path.trim_end_matches(".meta")).and_then(Option::as_ref)
                .map(|state|(guid,path.trim_end_matches(".meta").to_string(),state.clone()))))
        }).collect());
    let candidates: Vec<_> = candidates?.into_iter().flatten().collect();
    prefetch_git(dir, candidates.iter().map(|(_,_,s)|s))?;
    let parsed: Result<Vec<_>, String> = super::parallel::pool().install(|| candidates.par_iter().map(|(guid,path,state)| {
        Ok(persisted_script(dir,path,&read_blob(dir,state)?)?.map(|s|(guid.clone(),s)))
    }).collect());
    Ok(parsed?.into_iter().flatten().collect())
}

pub fn tree_for_guids(root: &Path, dir: &Path, revision: &str, scope: &Path, guids: &BTreeSet<String>) -> Result<SchemaMap,String> {
    let key = blake3::hash(&serde_json::to_vec(&(revision,scope_relative_path(root,scope)?,guids)).map_err(|e|e.to_string())?);
    let folder = dir.parent().ok_or("Missing schema cache parent")?.join("schema-cache-selected-v2");
    std::fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let cache = folder.join(format!("{key}.json"));
    if let Ok(bytes) = std::fs::read(&cache) {
        if let Ok(parsed) = serde_json::from_slice(&bytes) { return Ok(parsed); }
    }
    let files = super::snapshot::tree(root,revision)?.into_iter()
        .filter(|(p,s)|super::path_in_project(root,scope,p) && matches!(s.mode.as_str(),"100644"|"100755") && (p.ends_with(".cs")||p.ends_with(".cs.meta")))
        .map(|(p,s)|(p,Some(s))).collect();
    let schema = working_for_guids(dir,&files,guids)?;
    cache_json(&cache,&schema)?;
    Ok(schema)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldSchema {
    pub name: String,
    pub field_type: String,
    pub former_names: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptSchema {
    pub path: String,
    pub class_name: String,
    pub namespace: Option<String>,
    pub fields: Vec<FieldSchema>,
}
pub type SchemaMap = BTreeMap<String, ScriptSchema>;
static SCRIPT_CACHE: LazyLock<Mutex<BTreeMap<String, Option<ScriptSchema>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn script(path: &str, bytes: &[u8]) -> Option<ScriptSchema> {
    let key = format!("{}:{}", blake3::hash(bytes).to_hex(), path);
    if let Some(cached) = SCRIPT_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
    {
        return cached;
    }
    let parsed = script_uncached(path, bytes);
    let mut cache = SCRIPT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= 8192 {
        cache.clear();
    }
    cache.insert(key, parsed.clone());
    parsed
}
fn script_uncached(path: &str, bytes: &[u8]) -> Option<ScriptSchema> {
    let content = std::str::from_utf8(bytes).ok()?;
    // Conditional compilation and partial/inherited declarations require a
    // compiled schema provider; this narrow inference does not pretend to resolve them.
    if content.contains("#if") || content.contains("partial class") {
        return None;
    }
    let metadata = crate::unity_csharp::parse_cs_script(
        content,
        Path::new(path).file_stem().and_then(|s| s.to_str()),
    )?;
    Some(ScriptSchema {
        path: path.into(),
        class_name: metadata.class_name,
        namespace: metadata.namespace,
        fields: metadata
            .serialized_fields
            .into_iter()
            .map(|f| FieldSchema {
                name: f.name,
                field_type: f.field_type,
                former_names: f.former_names,
            })
            .collect(),
    })
}
fn meta_guid(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("guid:").map(str::trim))
        .filter(|g| g.len() == 32 && g.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(|g| g.to_ascii_lowercase())
}
pub fn working(
    dir: &Path,
    files: &BTreeMap<String, Option<FileState>>,
) -> Result<SchemaMap, String> {
    prefetch_git(dir, files.iter().filter(|(p,_)|p.ends_with(".cs")||p.ends_with(".cs.meta")).filter_map(|(_,s)|s.as_ref()))?;
    let mut result = BTreeMap::new();
    for (path, state) in files.iter().filter(|(p, _)| p.ends_with(".cs")) {
        let Some(state) = state else {
            continue;
        };
        let Some(Some(meta)) = files.get(&format!("{path}.meta")) else {
            continue;
        };
        if let (Some(guid), Some(schema)) = (
            meta_guid(&read_blob(dir, meta)?),
            script(path, &read_blob(dir, state)?),
        ) {
            result.insert(guid, schema);
        }
    }
    Ok(result)
}
fn scope_relative_path(root: &Path, scope: &Path) -> Result<String, String> {
    Ok(crate::workspace_service::worktrees::existing_path_relative(root, scope)?
        .ok_or_else(|| format!("Schema project escapes its repository: {}", scope.display()))?
        .to_string_lossy()
        .replace('\\', "/"))
}

pub fn tree(root: &Path, dir: &Path, tree: &str, scope: &Path) -> Result<SchemaMap, String> {
    let cache_dir = dir
        .parent()
        .ok_or("Missing schema cache directory")?
        .join("schema-cache-v1");
    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;
    let relative_scope = scope_relative_path(root, scope)?;
    let scope_hash = blake3::hash(relative_scope.as_bytes()).to_hex().to_string();
    let cache_path = cache_dir.join(format!("{tree}-{scope_hash}.json"));
    if let Ok(bytes) = std::fs::read(&cache_path) {
        if let Ok(schema) = serde_json::from_slice(&bytes) {
            return Ok(schema);
        }
    }
    let names = git(root, &["ls-tree", "-r", "-z", tree])?;
    let mut entries = BTreeMap::new();
    for raw in names.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let text = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
        let Some((header, path)) = text.split_once('\t') else {
            return Err("Invalid ls-tree header".into());
        };
        if !super::path_in_project(root, scope, path) {
            continue;
        }
        if !path.ends_with(".cs") && !path.ends_with(".cs.meta") {
            continue;
        }
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[1] != "blob" || !matches!(fields[0], "100644" | "100755") {
            continue;
        }
        entries.insert(path.to_string(), fields[2].to_string());
    }
    let oids: Vec<_> = entries
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let blobs = git_blobs(root, &oids)?;
    let mut result = BTreeMap::new();
    for (path, meta_oid) in entries
        .iter()
        .filter(|(path, _)| path.ends_with(".cs.meta"))
    {
        let script_path = path.trim_end_matches(".meta");
        let Some(source_oid) = entries.get(script_path) else {
            continue;
        };
        if let (Some(guid), Some(schema)) = (
            meta_guid(&blobs[meta_oid]),
            script(script_path, &blobs[source_oid]),
        ) {
            result.insert(guid, schema);
        }
    }
    atomic_json(&cache_path, &result)?;
    Ok(result)
}

pub fn aliases(
    target: &[u8],
    base: &SchemaMap,
    source: &SchemaMap,
    result: &SchemaMap,
) -> Vec<crate::unity_asset_core::FieldAlias> {
    let Ok(asset) = crate::unity_asset_core::parse_shared(target) else {
        return vec![];
    };
    let mut aliases = vec![];
    for document in &asset.documents {
        let Some(body) = document.root.get("MonoBehaviour") else {
            continue;
        };
        let Some(guid) = body
            .get("m_Script")
            .and_then(|n| n.get("guid"))
            .and_then(|n| n.scalar(&asset))
        else {
            continue;
        };
        let guid = guid.trim().to_ascii_lowercase();
        let (Some(final_schema), Some(base_schema), Some(source_schema)) =
            (result.get(&guid), base.get(&guid), source.get(&guid))
        else {
            continue;
        };
        if final_schema.class_name != base_schema.class_name
            || final_schema.namespace != base_schema.namespace
            || base_schema.class_name != source_schema.class_name
            || base_schema.namespace != source_schema.namespace
        {
            continue;
        }
        for field in &final_schema.fields {
            for old in &field.former_names {
                if final_schema
                    .fields
                    .iter()
                    .filter(|f| f.former_names.contains(old))
                    .count()
                    != 1
                    || final_schema.fields.iter().any(|f| f.name == *old)
                {
                    continue;
                }
                let old_base = base_schema
                    .fields
                    .iter()
                    .find(|f| f.name == *old)
                    .or_else(|| base_schema.fields.iter().find(|f| f.name == field.name));
                let old_source = source_schema
                    .fields
                    .iter()
                    .find(|f| f.name == *old)
                    .or_else(|| source_schema.fields.iter().find(|f| f.name == field.name));
                if old_base.map(|f| f.field_type.as_str()) != Some(field.field_type.as_str())
                    || old_source.map(|f| f.field_type.as_str()) != Some(field.field_type.as_str())
                {
                    continue;
                }
                // Only map to the target's already-materialized renamed field;
                // legacy names still on disk are handled by Unity's own import.
                if body.get(&field.name).is_none() || body.get(old).is_some() {
                    continue;
                }
                aliases.push(crate::unity_asset_core::FieldAlias {
                    object_id: document.object_id.clone(),
                    old_path: format!(
                        "/MonoBehaviour/{}",
                        old.replace('~', "~0").replace('/', "~1")
                    ),
                    new_path: format!(
                        "/MonoBehaviour/{}",
                        field.name.replace('~', "~0").replace('/', "~1")
                    ),
                });
            }
        }
    }
    aliases
}

pub fn commit_field_issues(
    dir: &Path,
    applied: &BTreeMap<String, Option<FileState>>,
    candidate: &BTreeMap<String, Option<FileState>>,
    outputs: &BTreeMap<String, Option<FileState>>,
) -> Result<Vec<serde_json::Value>, String> {
    let applied_schema = working(dir, applied)?;
    let candidate_schema = working(dir, candidate)?;
    let mut issues = vec![];
    for (path, state) in outputs {
        let Some(state) = state else {
            continue;
        };
        let bytes = read_blob(dir, state)?;
        if !super::unity_yaml(path, &bytes) {
            continue;
        }
        let Ok(asset) = crate::unity_asset_core::parse(&bytes) else {
            continue;
        };
        for document in &asset.documents {
            let Some(body) = document.root.get("MonoBehaviour") else {
                continue;
            };
            let Some(guid) = body
                .get("m_Script")
                .and_then(|n| n.get("guid"))
                .and_then(|n| n.scalar(&asset))
            else {
                continue;
            };
            let guid = guid.trim().to_ascii_lowercase();
            let (Some(applied), Some(candidate)) =
                (applied_schema.get(&guid), candidate_schema.get(&guid))
            else {
                continue;
            };
            for entry in body.entries().unwrap_or_default() {
                let known_in_applied = applied.fields.iter().any(|field| {
                    field.name == entry.key || field.former_names.contains(&entry.key)
                });
                let known_in_candidate = candidate.fields.iter().any(|field| {
                    field.name == entry.key || field.former_names.contains(&entry.key)
                });
                if known_in_applied && !known_in_candidate {
                    issues.push(serde_json::json!({"code":"serialized_schema_dependency","path":path,"object_id":document.object_id,"field":entry.key,"script":candidate.path,"detail":"This serialized field is defined by the applied/dirty code but absent from the candidate commit schema; explicitly include its code migration or revise the asset field plan"}));
                }
            }
        }
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn schema_scope_accepts_long_monorepo_paths_with_mixed_native_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let relative = format!("{}/{}/{}", "a".repeat(90), "b".repeat(90), "c".repeat(90));
        let project = temp.path().join(&relative);
        std::fs::create_dir_all(&project).unwrap();
        assert!(project.to_string_lossy().len() > 260);
        for root in [temp.path().to_path_buf(), std::fs::canonicalize(temp.path()).unwrap()] {
            for scope in [project.clone(), std::fs::canonicalize(&project).unwrap()] {
                assert_eq!(scope_relative_path(&root, &scope).unwrap(), relative);
            }
        }
    }

    #[test]
    fn schema_scope_rejects_sibling_and_repeated_name_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        for sibling in [temp.path().join("project-other"), temp.path().join("project-other/project")] {
            std::fs::create_dir_all(&sibling).unwrap();
            assert!(scope_relative_path(&root, &sibling).unwrap_err().contains("escapes its repository"));
        }
        assert_eq!(scope_relative_path(&root, &root).unwrap(), "");
    }
}
