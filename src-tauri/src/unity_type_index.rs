use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tree_sitter::{Node, Parser};

const CACHE_VERSION: u32 = 2;
const CACHE_REL_PATH: &[&str] = &["Library", "Locus", "TypeIndex", "unity-type-index-v2.json"];
const LEGACY_CACHE_REL_PATH: &[&str] =
    &["Library", "Locus", "TypeIndex", "unity-type-index-v1.json"];
const LEGACY_CACHE_VERSION: u32 = 1;
const SKILL_PACKAGE_ASSEMBLY_PREFIX: &str = "__LocusSkillPackage_";

const DEFAULT_NAMESPACE_USINGS: &[&str] = &[
    "System",
    "System.IO",
    "System.Text",
    "System.Linq",
    "System.Reflection",
    "System.Threading",
    "System.Threading.Tasks",
    "System.Collections",
    "System.Collections.Generic",
    "UnityEngine",
    "UnityEngine.SceneManagement",
    "UnityEditor",
    "UnityEditor.SceneManagement",
    "UnityEditor.Animations",
];

const RUN_STATES_EXTRA_DEFAULT_NAMESPACE_USINGS: &[&str] = &[
    "Unity.Profiling",
    "UnityEditor.Profiling",
    "UnityEditorInternal",
];

const UNITY_MATH_LOWERCASE_TYPES: &[&str] = &[
    "bool2",
    "bool3",
    "bool4",
    "double2",
    "double3",
    "double4",
    "float2",
    "float3",
    "float4",
    "half",
    "half2",
    "half3",
    "half4",
    "int2",
    "int3",
    "int4",
    "quaternion",
    "uint2",
    "uint3",
    "uint4",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityTypeIndexEntry {
    pub simple_name: String,
    #[serde(default, rename = "ns")]
    pub namespace: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub assembly: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityTypeIndexExport {
    #[serde(default)]
    fingerprint: String,
    #[serde(default)]
    types: Vec<UnityTypeIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityTypeIndexFingerprintExport {
    #[serde(default)]
    fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnityTypeIndexCacheFile {
    version: u32,
    #[serde(default)]
    fingerprint: String,
    exported_at_unix_ms: u64,
    #[serde(default)]
    assemblies: BTreeMap<String, UnityTypeIndexAssemblyInfo>,
    #[serde(default)]
    types: Vec<UnityTypeIndexEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityTypeIndexAssemblyInfo {
    #[serde(default)]
    pub package_id: String,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone)]
pub struct UnityTypeIndex {
    pub fingerprint: String,
    entries: Vec<UnityTypeIndexEntry>,
    assemblies: BTreeMap<String, UnityTypeIndexAssemblyInfo>,
    by_simple_name: HashMap<String, Vec<UnityTypeIndexEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousTypeResolution {
    pub simple_name: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsingConflict {
    pub simple_name: String,
    pub namespaces: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PreparedUnityCode {
    pub code: String,
    pub injected_namespaces: Vec<String>,
    pub ambiguous_types: Vec<AmbiguousTypeResolution>,
    pub blocked_conflicts: Vec<UsingConflict>,
}

#[derive(Debug, Clone)]
pub struct PreparedUnityRunStatesRequest {
    pub request: serde_json::Value,
    pub prepared_code: PreparedUnityCode,
}

fn type_index_cache() -> &'static Mutex<HashMap<String, Arc<UnityTypeIndex>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<UnityTypeIndex>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_project_key(project_path: &str) -> String {
    strip_extended_path_prefix(project_path)
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn strip_extended_path_prefix(path: &str) -> &str {
    path.strip_prefix(r"\\?\").unwrap_or(path)
}

pub fn type_index_cache_path(project_path: &str) -> PathBuf {
    type_index_cache_path_for(project_path, CACHE_REL_PATH)
}

fn legacy_type_index_cache_path(project_path: &str) -> PathBuf {
    type_index_cache_path_for(project_path, LEGACY_CACHE_REL_PATH)
}

fn type_index_cache_path_for(project_path: &str, rel_path: &[&str]) -> PathBuf {
    let mut path = PathBuf::from(strip_extended_path_prefix(project_path));
    for segment in rel_path {
        path.push(segment);
    }
    path
}

pub async fn cached_type_index(project_path: &str) -> Option<Arc<UnityTypeIndex>> {
    let key = normalize_project_key(project_path);
    type_index_cache().lock().await.get(&key).cloned()
}

pub async fn set_cached_type_index(project_path: &str, index: Arc<UnityTypeIndex>) {
    let key = normalize_project_key(project_path);
    type_index_cache().lock().await.insert(key, index);
}

pub async fn invalidate_cached_type_index(project_path: &str) {
    let key = normalize_project_key(project_path);
    type_index_cache().lock().await.remove(&key);

    // The sidecar compile params and serialized schema go stale at the same
    // moments as the type index: recompiles and domain reloads.
    crate::csharp_compile::params::invalidate(project_path).await;
    crate::unity_serialized_schema::invalidate(project_path).await;

    for path in [
        type_index_cache_path(project_path),
        legacy_type_index_cache_path(project_path),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "[Locus] Failed to remove Unity type index cache '{}': {}",
                path.display(),
                error
            ),
        }
    }
}

pub async fn load_cached_type_index(
    project_path: &str,
) -> Result<Option<Arc<UnityTypeIndex>>, String> {
    if let Some(index) = cached_type_index(project_path).await {
        return Ok(Some(index));
    }

    let Some(cache) = read_type_index_cache_file(project_path)? else {
        return Ok(None);
    };

    let index = Arc::new(UnityTypeIndex::from_cache(cache));
    set_cached_type_index(project_path, index.clone()).await;
    Ok(Some(index))
}

pub async fn persist_exported_type_index(
    project_path: &str,
    export_json: &str,
) -> Result<Arc<UnityTypeIndex>, String> {
    let export: UnityTypeIndexExport = serde_json::from_str(export_json)
        .map_err(|e| format!("Failed to parse Unity type index export: {}", e))?;
    let fingerprint = export.fingerprint.trim().to_string();
    if fingerprint.is_empty() {
        return Err("Unity type index export is missing fingerprint".to_string());
    }
    let cache = UnityTypeIndexCacheFile {
        version: CACHE_VERSION,
        fingerprint,
        exported_at_unix_ms: now_unix_ms(),
        assemblies: BTreeMap::new(),
        types: export.types,
    };
    let index = write_type_index_cache_file(project_path, cache).await?;

    set_cached_type_index(project_path, index.clone()).await;
    Ok(index)
}

pub async fn persist_skill_package_type_index_delta(
    project_path: &str,
    base_fingerprint: &str,
    fingerprint: &str,
    package_id: &str,
    source_hash: &str,
    assembly_id: &str,
    previous_assembly_id: &str,
    types: Vec<UnityTypeIndexEntry>,
) -> Result<Option<Arc<UnityTypeIndex>>, String> {
    let base_fingerprint = base_fingerprint.trim();
    if base_fingerprint.is_empty() {
        return Err("Unity type index delta is missing base fingerprint".to_string());
    }
    let fingerprint = fingerprint.trim();
    if fingerprint.is_empty() {
        return Err("Unity type index delta is missing fingerprint".to_string());
    }
    let package_id = package_id.trim();
    if package_id.is_empty() {
        return Err("Unity type index delta is missing packageId".to_string());
    }
    let assembly_id = assembly_id.trim();
    if assembly_id.is_empty() {
        return Err("Unity type index delta is missing assemblyId".to_string());
    }

    let Some(mut cache) = read_type_index_cache_file(project_path)? else {
        return Ok(None);
    };
    if cache.fingerprint != base_fingerprint {
        return Ok(None);
    }

    let previous_assembly_id = previous_assembly_id.trim();
    let mut assemblies_to_remove = BTreeSet::new();
    assemblies_to_remove.insert(assembly_id.to_string());
    if !previous_assembly_id.is_empty() {
        assemblies_to_remove.insert(previous_assembly_id.to_string());
    }
    for (assembly, info) in &cache.assemblies {
        if info.package_id == package_id {
            assemblies_to_remove.insert(assembly.clone());
        }
    }

    let package_assembly_prefix = skill_package_assembly_prefix_for_package(package_id);
    let has_untracked_package_assembly = cache.types.iter().any(|entry| {
        let assembly = entry.assembly.trim();
        assembly.starts_with(&package_assembly_prefix)
            && !assemblies_to_remove.contains(assembly)
            && cache
                .assemblies
                .get(assembly)
                .map(|info| info.package_id != package_id)
                .unwrap_or(true)
    });
    if has_untracked_package_assembly {
        return Ok(None);
    }

    cache.types.retain(|entry| {
        let assembly = entry.assembly.trim();
        !assemblies_to_remove.contains(assembly)
    });

    cache.assemblies.retain(|assembly, info| {
        !assemblies_to_remove.contains(assembly.as_str()) && info.package_id != package_id
    });

    for mut entry in types {
        if entry.assembly.trim().is_empty() {
            entry.assembly = assembly_id.to_string();
        }
        cache.types.push(entry);
    }

    cache.assemblies.insert(
        assembly_id.to_string(),
        UnityTypeIndexAssemblyInfo {
            package_id: package_id.to_string(),
            source_hash: source_hash.to_string(),
            active: true,
        },
    );
    cache.version = CACHE_VERSION;
    cache.fingerprint = fingerprint.to_string();
    cache.exported_at_unix_ms = now_unix_ms();

    let index = write_type_index_cache_file(project_path, cache).await?;
    set_cached_type_index(project_path, index.clone()).await;
    Ok(Some(index))
}

/// TI-B: persist a sidecar-built type index. The fingerprint is the
/// Unity-side `export_type_index_fingerprint` value, so currency checks and
/// the skill-package delta channel work identically for both sources.
pub async fn persist_sidecar_type_index(
    project_path: &str,
    fingerprint: String,
    types: Vec<UnityTypeIndexEntry>,
) -> Result<Arc<UnityTypeIndex>, String> {
    let fingerprint = fingerprint.trim().to_string();
    if fingerprint.is_empty() {
        return Err("sidecar type index is missing the Unity fingerprint".to_string());
    }
    let cache = UnityTypeIndexCacheFile {
        version: CACHE_VERSION,
        fingerprint,
        exported_at_unix_ms: now_unix_ms(),
        assemblies: BTreeMap::new(),
        types,
    };
    let index = write_type_index_cache_file(project_path, cache).await?;
    set_cached_type_index(project_path, index.clone()).await;
    Ok(index)
}

/// TI-C: layer hot-patch new types into the cached index so auto-usings
/// resolve them immediately. The cache fingerprint is untouched on purpose:
/// Unity's own export fingerprint skips `__LocusHotPatch_` assemblies, so
/// the cache stays "current" with these extra rows; any domain reload
/// changes the Unity fingerprint and the next full refresh drops them.
pub async fn append_hot_patch_types(
    project_path: &str,
    assembly_id: &str,
    types: Vec<UnityTypeIndexEntry>,
) -> Result<(), String> {
    if types.is_empty() {
        return Ok(());
    }
    let Some(mut cache) = read_type_index_cache_file(project_path)? else {
        return Err("base Unity type index cache is missing".to_string());
    };

    // Re-patching the same new type supersedes the previous patch's row.
    cache
        .types
        .retain(|entry| !types.iter().any(|t| t.full_name == entry.full_name));
    for mut entry in types {
        if entry.assembly.trim().is_empty() {
            entry.assembly = assembly_id.to_string();
        }
        cache.types.push(entry);
    }
    cache.exported_at_unix_ms = now_unix_ms();

    let index = write_type_index_cache_file(project_path, cache).await?;
    set_cached_type_index(project_path, index).await;
    Ok(())
}

/// Drop TI-C rows that came from transient hot-patch assemblies. Detours and
/// hot-patch assemblies disappear on domain reload; the base Unity fingerprint
/// may stay unchanged, so the cache needs an explicit cleanup path.
pub async fn drop_hot_patch_types(project_path: &str) -> Result<usize, String> {
    let Some(mut cache) = read_type_index_cache_file(project_path)? else {
        return Ok(0);
    };
    let before = cache.types.len();
    cache
        .types
        .retain(|entry| !entry.assembly.starts_with("__LocusHotPatch_"));
    let removed = before.saturating_sub(cache.types.len());
    if removed == 0 {
        return Ok(0);
    }
    cache
        .assemblies
        .retain(|assembly, _| !assembly.starts_with("__LocusHotPatch_"));
    cache.exported_at_unix_ms = now_unix_ms();
    let index = write_type_index_cache_file(project_path, cache).await?;
    set_cached_type_index(project_path, index).await;
    Ok(removed)
}

fn read_type_index_cache_file(
    project_path: &str,
) -> Result<Option<UnityTypeIndexCacheFile>, String> {
    let path = type_index_cache_path(project_path);
    if path.is_file() {
        return read_type_index_cache_file_from_path(&path);
    }

    let legacy_path = legacy_type_index_cache_path(project_path);
    if legacy_path.is_file() {
        return read_type_index_cache_file_from_path(&legacy_path);
    }

    Ok(None)
}

fn read_type_index_cache_file_from_path(
    path: &Path,
) -> Result<Option<UnityTypeIndexCacheFile>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "Failed to read Unity type index cache '{}': {}",
            path.display(),
            e
        )
    })?;
    let cache: UnityTypeIndexCacheFile = serde_json::from_str(&content).map_err(|e| {
        format!(
            "Failed to parse Unity type index cache '{}': {}",
            path.display(),
            e
        )
    })?;

    if cache.version != CACHE_VERSION && cache.version != LEGACY_CACHE_VERSION {
        return Ok(None);
    }

    Ok(Some(cache))
}

async fn write_type_index_cache_file(
    project_path: &str,
    cache: UnityTypeIndexCacheFile,
) -> Result<Arc<UnityTypeIndex>, String> {
    let index = Arc::new(UnityTypeIndex::from_cache(cache));
    let cache = UnityTypeIndexCacheFile {
        version: CACHE_VERSION,
        fingerprint: index.fingerprint.clone(),
        exported_at_unix_ms: now_unix_ms(),
        assemblies: index.assemblies.clone(),
        types: index.entries.clone(),
    };

    let path = type_index_cache_path(project_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create Unity type index cache directory '{}': {}",
                parent.display(),
                e
            )
        })?;
    }
    let json = serde_json::to_string_pretty(&cache)
        .map_err(|e| format!("Failed to serialize Unity type index cache: {}", e))?;
    std::fs::write(&path, json).map_err(|e| {
        format!(
            "Failed to write Unity type index cache '{}': {}",
            path.display(),
            e
        )
    })?;

    Ok(index)
}

fn skill_package_assembly_prefix_for_package(package_id: &str) -> String {
    format!(
        "{}{}_",
        SKILL_PACKAGE_ASSEMBLY_PREFIX,
        sanitize_assembly_name_part(package_id)
    )
}

fn sanitize_assembly_name_part(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "Script".to_string()
    } else {
        out
    }
}

pub fn parse_exported_type_index_fingerprint(export_json: &str) -> Result<String, String> {
    let export: UnityTypeIndexFingerprintExport = serde_json::from_str(export_json)
        .map_err(|e| format!("Failed to parse Unity type index fingerprint export: {}", e))?;
    let fingerprint = export.fingerprint.trim().to_string();
    if fingerprint.is_empty() {
        return Err("Unity type index fingerprint export is empty".to_string());
    }
    Ok(fingerprint)
}

pub fn prepare_unity_execute_code(code: &str, index: Option<&UnityTypeIndex>) -> PreparedUnityCode {
    prepare_code_with_extra_default_namespaces(code, index, &[])
}

pub fn prepare_unity_run_states_request(
    request: &serde_json::Value,
    index: Option<&UnityTypeIndex>,
) -> PreparedUnityRunStatesRequest {
    let combined_code = collect_run_states_code_fragments(request);
    let prepared_code = prepare_code_with_extra_default_namespaces(
        &combined_code,
        index,
        RUN_STATES_EXTRA_DEFAULT_NAMESPACE_USINGS,
    );

    let mut prepared_request = request.clone();
    if let Some(obj) = prepared_request.as_object_mut() {
        obj.remove("auto_usings");
        if !prepared_code.injected_namespaces.is_empty() {
            obj.insert(
                "auto_usings".to_string(),
                serde_json::Value::Array(
                    prepared_code
                        .injected_namespaces
                        .iter()
                        .map(|namespace| serde_json::Value::String(namespace.clone()))
                        .collect(),
                ),
            );
        }
    }

    PreparedUnityRunStatesRequest {
        request: prepared_request,
        prepared_code,
    }
}

fn prepare_code_with_extra_default_namespaces(
    code: &str,
    index: Option<&UnityTypeIndex>,
    extra_default_namespaces: &[&str],
) -> PreparedUnityCode {
    let Some(index) = index else {
        return PreparedUnityCode {
            code: code.to_string(),
            injected_namespaces: Vec::new(),
            ambiguous_types: Vec::new(),
            blocked_conflicts: Vec::new(),
        };
    };

    let existing_namespaces = parse_leading_using_namespaces(code);
    let used_names = collect_snippet_type_names(code);
    let resolution = resolve_using_namespaces(
        index,
        &used_names,
        &existing_namespaces,
        extra_default_namespaces,
    );

    if resolution.injected_namespaces.is_empty() {
        return PreparedUnityCode {
            code: code.to_string(),
            injected_namespaces: Vec::new(),
            ambiguous_types: resolution.ambiguous_types,
            blocked_conflicts: resolution.blocked_conflicts,
        };
    }

    let mut prefix = String::new();
    for namespace in &resolution.injected_namespaces {
        prefix.push_str("using ");
        prefix.push_str(namespace);
        prefix.push_str(";\n");
    }
    prefix.push('\n');
    prefix.push_str(code);

    PreparedUnityCode {
        code: prefix,
        injected_namespaces: resolution.injected_namespaces,
        ambiguous_types: resolution.ambiguous_types,
        blocked_conflicts: resolution.blocked_conflicts,
    }
}

fn collect_run_states_code_fragments(request: &serde_json::Value) -> String {
    let mut fragments = Vec::new();

    let Some(states) = request.get("states").and_then(serde_json::Value::as_array) else {
        return String::new();
    };

    for state in states {
        for key in ["variables", "start", "update", "end"] {
            if let Some(code) = state
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|code| !code.is_empty())
            {
                fragments.push(code.to_string());
            }
        }
    }

    fragments.join("\n\n")
}

pub fn append_auto_using_notes(error: String, prepared: &PreparedUnityCode) -> String {
    let has_notes = !prepared.injected_namespaces.is_empty()
        || !prepared.ambiguous_types.is_empty()
        || !prepared.blocked_conflicts.is_empty();
    if !has_notes {
        return error;
    }

    let mut out = error;
    out.push_str("\n\nauto using resolution:\n");

    if !prepared.injected_namespaces.is_empty() {
        out.push_str("  injected namespaces: ");
        out.push_str(&prepared.injected_namespaces.join(", "));
        out.push('\n');
    }

    for ambiguity in &prepared.ambiguous_types {
        out.push_str("  ambiguous type ");
        out.push_str(&ambiguity.simple_name);
        out.push_str(": ");
        out.push_str(&ambiguity.candidates.join(", "));
        out.push('\n');
    }

    for conflict in &prepared.blocked_conflicts {
        out.push_str("  skipped inferred using to avoid ambiguous ");
        out.push_str(&conflict.simple_name);
        out.push_str(": ");
        out.push_str(&conflict.namespaces.join(", "));
        out.push('\n');
    }

    out
}

impl UnityTypeIndex {
    fn from_cache(cache: UnityTypeIndexCacheFile) -> Self {
        Self::from_entries_with_assemblies(cache.fingerprint, cache.types, cache.assemblies)
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn from_entries(fingerprint: String, entries: Vec<UnityTypeIndexEntry>) -> Self {
        Self::from_entries_with_assemblies(fingerprint, entries, BTreeMap::new())
    }

    fn from_entries_with_assemblies(
        fingerprint: String,
        entries: Vec<UnityTypeIndexEntry>,
        assemblies: BTreeMap<String, UnityTypeIndexAssemblyInfo>,
    ) -> Self {
        let mut by_key: BTreeMap<(String, String), UnityTypeIndexEntry> = BTreeMap::new();
        for mut entry in entries {
            entry.simple_name = entry.simple_name.trim().to_string();
            entry.namespace = entry.namespace.trim().to_string();
            entry.full_name = entry.full_name.trim().to_string();
            entry.assembly = entry.assembly.trim().to_string();

            if entry.simple_name.is_empty() {
                continue;
            }
            if entry.full_name.is_empty() {
                entry.full_name = if entry.namespace.is_empty() {
                    entry.simple_name.clone()
                } else {
                    format!("{}.{}", entry.namespace, entry.simple_name)
                };
            }
            by_key
                .entry((entry.simple_name.clone(), entry.namespace.clone()))
                .or_insert(entry);
        }

        let normalized_entries = by_key.values().cloned().collect::<Vec<_>>();
        let mut by_simple_name: HashMap<String, Vec<UnityTypeIndexEntry>> = HashMap::new();
        for ((simple_name, _), entry) in by_key {
            by_simple_name.entry(simple_name).or_default().push(entry);
        }
        for entries in by_simple_name.values_mut() {
            entries.sort_by(|a, b| {
                a.namespace
                    .cmp(&b.namespace)
                    .then_with(|| a.full_name.cmp(&b.full_name))
                    .then_with(|| a.assembly.cmp(&b.assembly))
            });
        }

        Self {
            fingerprint,
            entries: normalized_entries,
            assemblies,
            by_simple_name,
        }
    }

    fn candidate_namespaces(&self, simple_name: &str) -> Vec<String> {
        let Some(entries) = self.by_simple_name.get(simple_name) else {
            return Vec::new();
        };

        let mut namespaces = BTreeSet::new();
        for entry in entries {
            namespaces.insert(entry.namespace.clone());
        }
        namespaces.into_iter().collect()
    }

    fn candidate_full_names(&self, simple_name: &str) -> Vec<String> {
        let Some(entries) = self.by_simple_name.get(simple_name) else {
            return Vec::new();
        };

        let mut full_names = BTreeSet::new();
        for entry in entries {
            full_names.insert(entry.full_name.clone());
        }
        full_names.into_iter().collect()
    }
}

#[derive(Debug)]
struct UsingResolution {
    injected_namespaces: Vec<String>,
    ambiguous_types: Vec<AmbiguousTypeResolution>,
    blocked_conflicts: Vec<UsingConflict>,
}

fn resolve_using_namespaces(
    index: &UnityTypeIndex,
    used_names: &BTreeSet<String>,
    explicit_namespaces: &BTreeSet<String>,
    extra_default_namespaces: &[&str],
) -> UsingResolution {
    let mut effective_namespaces = BTreeSet::new();
    for namespace in DEFAULT_NAMESPACE_USINGS {
        effective_namespaces.insert((*namespace).to_string());
    }
    for namespace in extra_default_namespaces {
        effective_namespaces.insert((*namespace).to_string());
    }
    effective_namespaces.extend(explicit_namespaces.iter().cloned());

    let mut proposed = BTreeSet::new();
    let mut ambiguous_types = Vec::new();

    for simple_name in used_names {
        let candidates = index.candidate_namespaces(simple_name);
        if candidates.is_empty() || candidates.iter().any(|ns| ns.is_empty()) {
            continue;
        }

        if candidates
            .iter()
            .any(|namespace| effective_namespaces.contains(namespace))
        {
            continue;
        }

        if candidates.len() == 1 {
            proposed.insert(candidates[0].clone());
        } else {
            ambiguous_types.push(AmbiguousTypeResolution {
                simple_name: simple_name.clone(),
                candidates: index.candidate_full_names(simple_name),
            });
        }
    }

    let mut blocked_conflicts = Vec::new();
    loop {
        let combined = effective_namespaces
            .iter()
            .chain(proposed.iter())
            .cloned()
            .collect::<BTreeSet<_>>();

        let mut blocked_this_round = BTreeSet::new();
        for simple_name in used_names {
            let visible = index
                .candidate_namespaces(simple_name)
                .into_iter()
                .filter(|namespace| !namespace.is_empty() && combined.contains(namespace))
                .collect::<Vec<_>>();

            let unique_visible = visible.into_iter().collect::<BTreeSet<_>>();
            if unique_visible.len() <= 1 {
                continue;
            }

            let proposed_visible = unique_visible
                .iter()
                .filter(|namespace| proposed.contains(*namespace))
                .cloned()
                .collect::<Vec<_>>();
            if proposed_visible.is_empty() {
                continue;
            }

            blocked_conflicts.push(UsingConflict {
                simple_name: simple_name.clone(),
                namespaces: unique_visible.iter().cloned().collect(),
            });
            blocked_this_round.extend(proposed_visible);
        }

        if blocked_this_round.is_empty() {
            break;
        }

        for namespace in blocked_this_round {
            proposed.remove(&namespace);
        }
    }

    UsingResolution {
        injected_namespaces: proposed.into_iter().collect(),
        ambiguous_types,
        blocked_conflicts: dedupe_conflicts(blocked_conflicts),
    }
}

fn dedupe_conflicts(conflicts: Vec<UsingConflict>) -> Vec<UsingConflict> {
    let mut by_key = BTreeMap::new();
    for conflict in conflicts {
        by_key
            .entry((conflict.simple_name.clone(), conflict.namespaces.join("|")))
            .or_insert(conflict);
    }
    by_key.into_values().collect()
}

fn parse_leading_using_namespaces(code: &str) -> BTreeSet<String> {
    let mut namespaces = BTreeSet::new();
    let normalized = code.replace("\r\n", "\n");
    let mut saw_using = false;

    for line in normalized.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if saw_using {
                continue;
            }
            continue;
        }

        if trimmed.starts_with("using ") && trimmed.ends_with(';') {
            saw_using = true;
            let namespace = trimmed
                .trim_start_matches("using ")
                .trim_end_matches(';')
                .trim();
            if !namespace.starts_with("static ") && !namespace.contains('=') {
                namespaces.insert(namespace.to_string());
            }
            continue;
        }

        break;
    }

    namespaces
}

fn collect_snippet_type_names(code: &str) -> BTreeSet<String> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .is_err()
    {
        return BTreeSet::new();
    }

    let parse_source = wrap_snippet_for_type_parse(code);
    let Some(tree) = parser.parse(&parse_source, None) else {
        return BTreeSet::new();
    };

    let mut names = BTreeSet::new();
    collect_type_names_from_node(parse_source.as_bytes(), tree.root_node(), &mut names);
    names
}

fn wrap_snippet_for_type_parse(code: &str) -> String {
    let (leading_usings, body) = split_leading_using_block(code);
    format!(
        "{}\npublic static class __LocusTypeProbe {{ public static void __M() {{\n{}\n;\n}} }}",
        leading_usings, body
    )
}

fn split_leading_using_block(code: &str) -> (String, String) {
    if code.is_empty() {
        return (String::new(), String::new());
    }

    let normalized = code.replace("\r\n", "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut using_lines = Vec::new();
    let mut body_start = 0usize;
    let mut still_in_using_block = true;

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if still_in_using_block {
            if trimmed.is_empty() {
                if using_lines.is_empty() {
                    body_start = index + 1;
                    continue;
                }
                using_lines.push(*line);
                body_start = index + 1;
                continue;
            }

            if trimmed.starts_with("using ") && trimmed.ends_with(';') {
                using_lines.push(*line);
                body_start = index + 1;
                continue;
            }

            still_in_using_block = false;
            body_start = index;
        }
    }

    let body = if body_start < lines.len() {
        lines[body_start..].join("\n")
    } else {
        String::new()
    };

    (using_lines.join("\n"), body)
}

fn collect_type_names_from_node(source: &[u8], node: Node, names: &mut BTreeSet<String>) {
    if node.kind() == "using_directive" {
        return;
    }

    if let Some(type_node) = node.child_by_field_name("type") {
        collect_names_from_type_node(source, type_node, names);
    }

    if node.kind() == "generic_name" {
        collect_names_from_type_node(source, node, names);
    }

    if node.kind() == "member_access_expression" {
        collect_member_access_root(source, node, names);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_names_from_node(source, child, names);
    }
}

fn collect_names_from_type_node(source: &[u8], node: Node, names: &mut BTreeSet<String>) {
    match node.kind() {
        "identifier" => {
            if let Ok(text) = node.utf8_text(source) {
                push_candidate_type_name(text, names);
            }
            return;
        }
        "generic_name" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(text) = name_node.utf8_text(source) {
                    push_candidate_type_name(text, names);
                }
            }
        }
        "qualified_name" | "alias_qualified_name" => {
            collect_type_arguments(source, node, names);
            return;
        }
        "predefined_type" => return,
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_names_from_type_node(source, child, names);
    }
}

fn collect_type_arguments(source: &[u8], node: Node, names: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "type_argument_list" {
            collect_names_from_type_node(source, child, names);
        } else {
            collect_type_arguments(source, child, names);
        }
    }
}

fn collect_member_access_root(source: &[u8], node: Node, names: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    let Some(first_named) = node.named_children(&mut cursor).next() else {
        return;
    };

    if first_named.kind() != "identifier" {
        return;
    }

    if let Ok(text) = first_named.utf8_text(source) {
        push_candidate_type_name(text, names);
    }
}

fn push_candidate_type_name(raw: &str, names: &mut BTreeSet<String>) {
    let name = raw.trim();
    if looks_like_type_name(name) {
        names.insert(name.to_string());
    }
}

fn looks_like_type_name(name: &str) -> bool {
    if name.is_empty() || is_csharp_keyword_or_builtin(name) {
        return false;
    }

    if UNITY_MATH_LOWERCASE_TYPES.contains(&name) {
        return true;
    }

    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_uppercase() || first == '_' => true,
        _ => name.contains('_') && name.chars().any(|c| c.is_ascii_uppercase()),
    }
}

fn is_csharp_keyword_or_builtin(name: &str) -> bool {
    matches!(
        name,
        "abstract"
            | "as"
            | "base"
            | "bool"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "checked"
            | "class"
            | "const"
            | "continue"
            | "decimal"
            | "default"
            | "delegate"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "event"
            | "explicit"
            | "extern"
            | "false"
            | "finally"
            | "fixed"
            | "float"
            | "for"
            | "foreach"
            | "goto"
            | "if"
            | "implicit"
            | "in"
            | "int"
            | "interface"
            | "internal"
            | "is"
            | "lock"
            | "long"
            | "namespace"
            | "new"
            | "null"
            | "object"
            | "operator"
            | "out"
            | "override"
            | "params"
            | "private"
            | "protected"
            | "public"
            | "readonly"
            | "ref"
            | "return"
            | "sbyte"
            | "sealed"
            | "short"
            | "sizeof"
            | "stackalloc"
            | "static"
            | "string"
            | "struct"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "uint"
            | "ulong"
            | "unchecked"
            | "unsafe"
            | "ushort"
            | "using"
            | "var"
            | "virtual"
            | "void"
            | "volatile"
            | "while"
    )
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[allow(dead_code)]
fn _cache_path_is_under_project_library(project_path: &str, path: &Path) -> bool {
    path.starts_with(Path::new(strip_extended_path_prefix(project_path)).join("Library"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_index(entries: &[(&str, &str)]) -> UnityTypeIndex {
        UnityTypeIndex::from_entries(
            "test".to_string(),
            entries
                .iter()
                .map(|(simple, namespace)| UnityTypeIndexEntry {
                    simple_name: (*simple).to_string(),
                    namespace: (*namespace).to_string(),
                    full_name: if namespace.is_empty() {
                        (*simple).to_string()
                    } else {
                        format!("{}.{}", namespace, simple)
                    },
                    assembly: "Test".to_string(),
                })
                .collect(),
        )
    }

    fn test_entry(simple: &str, namespace: &str, assembly: &str) -> UnityTypeIndexEntry {
        UnityTypeIndexEntry {
            simple_name: simple.to_string(),
            namespace: namespace.to_string(),
            full_name: if namespace.is_empty() {
                simple.to_string()
            } else {
                format!("{}.{}", namespace, simple)
            },
            assembly: assembly.to_string(),
        }
    }

    #[test]
    fn prepare_injects_unique_namespace_before_compile() {
        let index = test_index(&[
            ("CinemachineCamera", "Unity.Cinemachine"),
            ("TextMeshProUGUI", "TMPro"),
        ]);
        let prepared = prepare_unity_execute_code(
            "var cam = FindObjectOfType<CinemachineCamera>();\nprint(cam.name);",
            Some(&index),
        );

        assert!(prepared.code.contains("using Unity.Cinemachine;\n"));
        assert!(prepared
            .injected_namespaces
            .contains(&"Unity.Cinemachine".to_string()));
    }

    #[test]
    fn prepare_keeps_ambiguous_type_unresolved() {
        let index = test_index(&[("Widget", "Game.UI"), ("Widget", "Tools.UI")]);
        let prepared = prepare_unity_execute_code("Widget widget = null;", Some(&index));

        assert_eq!(prepared.code, "Widget widget = null;");
        assert_eq!(prepared.injected_namespaces, Vec::<String>::new());
        assert_eq!(prepared.ambiguous_types.len(), 1);
        assert_eq!(prepared.ambiguous_types[0].simple_name, "Widget");
    }

    #[test]
    fn prepare_blocks_common_namespace_that_would_break_existing_type() {
        let index = test_index(&[
            ("Object", "UnityEngine"),
            ("Object", "Game.Runtime"),
            ("Widget", "Game.Runtime"),
        ]);
        let prepared = prepare_unity_execute_code(
            "Object existing = null;\nWidget widget = null;",
            Some(&index),
        );

        assert_eq!(prepared.injected_namespaces, Vec::<String>::new());
        assert!(prepared
            .blocked_conflicts
            .iter()
            .any(|conflict| conflict.simple_name == "Object"));
    }

    #[test]
    fn prepare_injects_common_namespace_for_static_class_usage() {
        let index = test_index(&[("JsonConvert", "Newtonsoft.Json")]);
        let prepared = prepare_unity_execute_code(
            "var json = JsonConvert.SerializeObject(new { value = 1 });",
            Some(&index),
        );

        assert!(prepared.code.starts_with("using Newtonsoft.Json;\n\n"));
    }

    #[test]
    fn prepare_does_not_inject_unreferenced_common_namespaces() {
        let index = test_index(&[
            ("JsonConvert", "Newtonsoft.Json"),
            ("CinemachineCamera", "Unity.Cinemachine"),
            ("VisualElement", "UnityEngine.UIElements"),
        ]);
        let code = "var path = AssetStoreTools.Constants.AssetStoreToolsPackagePath;\nprint(path);";
        let prepared = prepare_unity_execute_code(code, Some(&index));

        assert_eq!(prepared.code, code);
        assert!(prepared.injected_namespaces.is_empty());
        assert!(prepared.blocked_conflicts.is_empty());
    }

    #[test]
    fn prepare_run_states_request_injects_namespaces_from_state_snippets() {
        let index = test_index(&[
            ("CinemachineCamera", "Unity.Cinemachine"),
            ("ProfilerRecorder", "Unity.Profiling"),
        ]);
        let request = serde_json::json!({
            "request_editor_status": "playing",
            "initial_state": "inspect",
            "states": [
                {
                    "name": "inspect",
                    "variables": "CinemachineCamera cam;",
                    "update": "cam = FindObjectOfType<CinemachineCamera>(); ctx.Done();"
                }
            ]
        });

        let prepared = prepare_unity_run_states_request(&request, Some(&index));
        let auto_usings = prepared
            .request
            .get("auto_usings")
            .and_then(serde_json::Value::as_array)
            .expect("auto usings");

        assert!(auto_usings
            .iter()
            .any(|value| value.as_str() == Some("Unity.Cinemachine")));
        assert!(!auto_usings
            .iter()
            .any(|value| value.as_str() == Some("Unity.Profiling")));
    }

    #[test]
    fn prepare_run_states_request_removes_stale_auto_usings_when_unused() {
        let index = test_index(&[("JsonConvert", "Newtonsoft.Json")]);
        let request = serde_json::json!({
            "request_editor_status": "editing",
            "initial_state": "done",
            "auto_usings": ["Newtonsoft.Json"],
            "states": [
                {
                    "name": "done",
                    "update": "ctx.Done();"
                }
            ]
        });

        let prepared = prepare_unity_run_states_request(&request, Some(&index));
        assert!(prepared.request.get("auto_usings").is_none());
        assert!(prepared.prepared_code.injected_namespaces.is_empty());
    }

    #[test]
    fn split_leading_using_block_skips_leading_blank_lines() {
        let (leading, body) =
            split_leading_using_block("\n\nusing TMPro;\n\nTextMeshProUGUI label = null;");

        assert!(leading.contains("using TMPro;"));
        assert!(!body.contains("using TMPro;"));
        assert!(body.contains("TextMeshProUGUI label"));
    }

    #[test]
    fn invalidate_cached_type_index_removes_memory_and_disk_cache() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();
            let cache_path = type_index_cache_path(&project_path);
            std::fs::create_dir_all(cache_path.parent().expect("cache parent"))
                .expect("create cache dir");
            std::fs::write(&cache_path, "{}").expect("write cache file");

            let index = Arc::new(test_index(&[("Widget", "Game.UI")]));
            set_cached_type_index(&project_path, index).await;
            assert!(cached_type_index(&project_path).await.is_some());

            invalidate_cached_type_index(&project_path).await;

            assert!(cached_type_index(&project_path).await.is_none());
            assert!(!cache_path.exists());
        });
    }

    #[test]
    fn sidecar_type_index_persists_and_hot_patch_types_layer_on_top() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();

            let index = persist_sidecar_type_index(
                &project_path,
                "unity-fp-1".to_string(),
                vec![
                    test_entry("ProjectType", "Game", "Assembly-CSharp"),
                    test_entry("Widget", "Game.UI", "Assembly-CSharp"),
                ],
            )
            .await
            .expect("persist sidecar index");
            assert_eq!(index.fingerprint, "unity-fp-1");
            assert!(index.by_simple_name.contains_key("Widget"));
            assert!(type_index_cache_path(&project_path).is_file());

            // TI-C layering keeps the fingerprint (Unity's own fingerprint
            // skips hot patch assemblies) and supersedes re-patched types.
            append_hot_patch_types(
                &project_path,
                "__LocusHotPatch_00000000_00000001",
                vec![test_entry("Spawner", "Game", "")],
            )
            .await
            .expect("append hot patch types");

            let layered = cached_type_index(&project_path).await.expect("cached");
            assert_eq!(layered.fingerprint, "unity-fp-1");
            assert!(layered.by_simple_name.contains_key("Spawner"));
            let spawner = &layered.by_simple_name["Spawner"][0];
            assert_eq!(spawner.assembly, "__LocusHotPatch_00000000_00000001");

            append_hot_patch_types(
                &project_path,
                "__LocusHotPatch_00000000_00000002",
                vec![test_entry("Spawner", "Game", "")],
            )
            .await
            .expect("supersede hot patch types");

            let layered = cached_type_index(&project_path).await.expect("cached");
            let spawners = &layered.by_simple_name["Spawner"];
            assert_eq!(spawners.len(), 1);
            assert_eq!(spawners[0].assembly, "__LocusHotPatch_00000000_00000002");
        });
    }

    #[test]
    fn skill_package_delta_updates_cached_type_index() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();
            let base = UnityTypeIndexCacheFile {
                version: CACHE_VERSION,
                fingerprint: "base".to_string(),
                exported_at_unix_ms: 1,
                assemblies: BTreeMap::new(),
                types: vec![
                    test_entry("ProjectType", "Game", "Assembly-CSharp"),
                    test_entry(
                        "OldSkillType",
                        "Locus.SkillPackages.Com.Example.Tool",
                        "__LocusSkillPackage_com_example_tool_aaaaaaaaaaaa",
                    ),
                ],
            };
            write_type_index_cache_file(&project_path, base)
                .await
                .expect("write base cache");

            let updated = persist_skill_package_type_index_delta(
                &project_path,
                "base",
                "next",
                "com.example.tool",
                "bbbbbbbbbbbbbbbb",
                "__LocusSkillPackage_com_example_tool_bbbbbbbbbbbb",
                "__LocusSkillPackage_com_example_tool_aaaaaaaaaaaa",
                vec![test_entry(
                    "NewSkillType",
                    "Locus.SkillPackages.Com.Example.Tool",
                    "__LocusSkillPackage_com_example_tool_bbbbbbbbbbbb",
                )],
            )
            .await
            .expect("persist delta")
            .expect("cache exists");

            assert_eq!(updated.fingerprint, "next");
            assert!(updated.by_simple_name.contains_key("ProjectType"));
            assert!(updated.by_simple_name.contains_key("NewSkillType"));
            assert!(!updated.by_simple_name.contains_key("OldSkillType"));
            assert!(updated
                .assemblies
                .contains_key("__LocusSkillPackage_com_example_tool_bbbbbbbbbbbb"));
        });
    }

    #[test]
    fn skill_package_delta_falls_back_on_untracked_prefix_collision() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();
            let base = UnityTypeIndexCacheFile {
                version: CACHE_VERSION,
                fingerprint: "base".to_string(),
                exported_at_unix_ms: 1,
                assemblies: BTreeMap::new(),
                types: vec![
                    test_entry(
                        "NeighborType",
                        "Locus.SkillPackages.Com.Foo.Bar",
                        "__LocusSkillPackage_com_foo_bar_hash",
                    ),
                    test_entry("ProjectType", "Game", "Assembly-CSharp"),
                ],
            };
            write_type_index_cache_file(&project_path, base)
                .await
                .expect("write base cache");

            let result = persist_skill_package_type_index_delta(
                &project_path,
                "base",
                "next",
                "com.foo",
                "hash",
                "__LocusSkillPackage_com_foo_hash",
                "",
                vec![test_entry(
                    "SkillType",
                    "Locus.SkillPackages.Com.Foo",
                    "__LocusSkillPackage_com_foo_hash",
                )],
            )
            .await
            .expect("prefix collision falls back");

            assert!(result.is_none());
            let cache = read_type_index_cache_file(&project_path)
                .expect("read cache")
                .expect("cache");
            assert!(cache
                .types
                .iter()
                .any(|entry| entry.simple_name == "NeighborType"));
            assert_eq!(cache.fingerprint, "base");
        });
    }

    #[test]
    fn skill_package_delta_requires_existing_cache() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();
            let result = persist_skill_package_type_index_delta(
                &project_path,
                "base",
                "next",
                "com.example.tool",
                "hash",
                "__LocusSkillPackage_com_example_tool_hash",
                "",
                vec![test_entry(
                    "SkillType",
                    "Locus.SkillPackages.Com.Example.Tool",
                    "__LocusSkillPackage_com_example_tool_hash",
                )],
            )
            .await
            .expect("delta without cache");

            assert!(result.is_none());
        });
    }

    #[test]
    fn skill_package_delta_rejects_stale_base_cache() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(async {
            let project = tempfile::tempdir().expect("project");
            let project_path = project.path().to_string_lossy();
            let base = UnityTypeIndexCacheFile {
                version: CACHE_VERSION,
                fingerprint: "stale".to_string(),
                exported_at_unix_ms: 1,
                assemblies: BTreeMap::new(),
                types: vec![test_entry("ProjectType", "Game", "Assembly-CSharp")],
            };
            write_type_index_cache_file(&project_path, base)
                .await
                .expect("write base cache");

            let result = persist_skill_package_type_index_delta(
                &project_path,
                "expected-base",
                "next",
                "com.example.tool",
                "hash",
                "__LocusSkillPackage_com_example_tool_hash",
                "",
                vec![test_entry(
                    "SkillType",
                    "Locus.SkillPackages.Com.Example.Tool",
                    "__LocusSkillPackage_com_example_tool_hash",
                )],
            )
            .await
            .expect("stale delta returns fallback");

            assert!(result.is_none());
        });
    }

    #[test]
    fn cache_path_stays_under_project_library() {
        let project = tempfile::tempdir().expect("project");
        let path = type_index_cache_path(&project.path().to_string_lossy());
        assert!(_cache_path_is_under_project_library(
            &project.path().to_string_lossy(),
            &path
        ));
    }
}
