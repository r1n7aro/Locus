use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use walkdir::WalkDir;

use crate::asset_db::types::Guid;
use crate::unity_csharp::{parse_cs_script, ScriptMetadata};
use crate::unity_yaml::YamlDoc;

use super::parse::prettify_field_label;
use super::{extract_script_guid, SemanticBuildEnv};
use crate::diff::content::{
    git_show_file_sync, parse_lfs_pointer, resolve_lfs_sync, BatchBlobReader,
};
use crate::diff::context::{SideContext, SideFileSource};
use crate::diff::profiler::DiffProfiler;

#[derive(Debug, Clone)]
pub(crate) struct ScriptFieldEnhancement {
    pub(crate) canonical_name: String,
    pub(crate) display_label: String,
    pub(crate) hidden: bool,
    pub(crate) field_type: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ScriptSemanticInfo {
    pub(crate) class_name: String,
    pub(crate) base_type: Option<String>,
    pub(crate) field_aliases: HashMap<String, ScriptFieldEnhancement>,
}

#[derive(Debug, Default)]
pub(crate) struct ScriptInfoCache {
    pub(crate) infos: HashMap<String, Option<ScriptSemanticInfo>>,
    pub(crate) class_paths: HashMap<String, Option<String>>,
    /// Lazily built on first WalkDir miss: lowercase .cs filename → relative path.
    /// Populated once per request to avoid repeated full-tree scans.
    pub(crate) cs_file_index: Option<HashMap<String, String>>,
    /// Wall-clock ms spent building cs_file_index (for profiler).
    pub(crate) walkdir_ms: u64,
}

pub(crate) fn build_script_semantic_info(metadata: ScriptMetadata) -> ScriptSemanticInfo {
    let mut field_aliases = HashMap::new();

    for field in metadata.serialized_fields {
        let enhancement = ScriptFieldEnhancement {
            canonical_name: field.name.clone(),
            display_label: prettify_field_label(&field.name),
            hidden: field.hidden,
            field_type: if field.field_type.is_empty() {
                None
            } else {
                Some(field.field_type.clone())
            },
        };
        field_aliases.insert(field.name.clone(), enhancement.clone());
        for former_name in field.former_names {
            field_aliases.insert(former_name, enhancement.clone());
        }
    }

    ScriptSemanticInfo {
        class_name: metadata.class_name,
        base_type: metadata.base_type,
        field_aliases,
    }
}

pub(crate) fn doc_script_guid(doc: &YamlDoc, lines: &[String]) -> Option<Guid> {
    doc.m_script_guid
        .or_else(|| extract_script_guid(doc, lines))
}

/// Readonly lookup: returns cached script info without loading from disk on miss.
/// Used in the parallel phase where ScriptInfoCache is immutable.
pub(crate) fn lookup_script_semantic_info(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    cache: &ScriptInfoCache,
) -> Option<ScriptSemanticInfo> {
    if doc.class_id != 114 {
        return None;
    }
    let guid = doc_script_guid(doc, lines)?;
    let script_path = side_ctx.resolve_script_guid_path(&guid)?;
    let cache_key = script_cache_key(side_ctx, &script_path);
    cache.infos.get(&cache_key)?.clone()
}

/// Readonly lookup by class name: returns cached script info without loading on miss.
pub(crate) fn lookup_script_info_by_class_name(
    class_name: &str,
    cache: &ScriptInfoCache,
) -> Option<ScriptSemanticInfo> {
    let cache_key = format!("class:{}", class_name);
    cache.infos.get(&cache_key)?.clone()
}

/// Readonly variant of `resolve_all_field_types` that uses immutable script cache.
/// Falls back gracefully on cache miss (returns empty for unresolvable paths).
pub(crate) fn resolve_all_field_types_readonly(
    paths: impl Iterator<Item = impl AsRef<str>>,
    script_info: Option<&ScriptSemanticInfo>,
    _hint_script_path: Option<&str>,
    cache: &ScriptInfoCache,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let Some(info) = script_info else {
        return result;
    };

    for path in paths {
        let path = path.as_ref();
        let segments = super::parse::split_property_path(path);
        if segments.is_empty() {
            continue;
        }

        // Top-level: direct lookup (fast path)
        if segments.len() == 1 {
            if let Some(enh) = info.field_aliases.get(&segments[0]) {
                if let Some(ft) = &enh.field_type {
                    result.insert(path.to_string(), ft.clone());
                }
            }
            continue;
        }

        // Nested: walk the type chain using readonly cache
        if let Some(ft) = resolve_nested_field_type_readonly(&segments, info, cache) {
            result.insert(path.to_string(), ft);
        }
    }

    result
}

/// Readonly variant of `resolve_nested_field_type`.
fn resolve_nested_field_type_readonly(
    segments: &[String],
    root_info: &ScriptSemanticInfo,
    cache: &ScriptInfoCache,
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    let mut current_info = root_info.clone();

    for (i, segment) in segments.iter().enumerate() {
        if segment.starts_with('[') {
            continue;
        }

        let enhancement = current_info.field_aliases.get(segment)?;
        let field_type = enhancement.field_type.as_ref()?;

        if i == segments.len() - 1 {
            return Some(field_type.clone());
        }

        let inner_type = unwrap_collection_type(field_type);
        if is_primitive_field_type(inner_type) {
            return None;
        }

        current_info = lookup_script_info_by_class_name(inner_type, cache)?;
    }

    None
}

pub(crate) fn script_cache_key(side_ctx: &SideContext, script_path: &str) -> String {
    match &side_ctx.file_source {
        SideFileSource::Workspace => format!("workspace:{}", script_path),
        SideFileSource::GitRef(reference) => format!("git:{}:{}", reference, script_path),
        SideFileSource::GitIndex => format!("index:{}", script_path),
        SideFileSource::GitStage(n) => format!("stage{}:{}", n, script_path),
    }
}

pub(crate) fn normalize_rel_path(path: &Path) -> Option<String> {
    path.to_str().map(|value| value.replace('\\', "/"))
}

pub(crate) fn is_unity_terminal_base_type(base_type: &str) -> bool {
    matches!(
        base_type,
        "ScriptableObject" | "MonoBehaviour" | "Behaviour" | "Component" | "Object"
    )
}

pub(crate) fn should_skip_script_search_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| {
            matches!(
                value,
                ".git" | "node_modules" | "dist" | "target" | "Library" | "Temp" | "Logs" | "obj"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn find_script_path_by_class_name(
    cwd: &str,
    current_script_path: Option<&str>,
    class_name: &str,
    cache: &mut ScriptInfoCache,
) -> Option<String> {
    if let Some(cached) = cache.class_paths.get(class_name) {
        return cached.clone();
    }

    let file_name = format!("{}.cs", class_name);

    if let Some(current_script_path) = current_script_path {
        if let Some(parent) = Path::new(current_script_path).parent() {
            let sibling = parent.join(&file_name);
            if let Some(relative) = normalize_rel_path(&sibling) {
                let full_path = Path::new(cwd).join(&relative);
                if full_path.is_file() {
                    cache
                        .class_paths
                        .insert(class_name.to_string(), Some(relative.clone()));
                    return Some(relative);
                }
            }
        }
    }

    // Build the full .cs file index on first miss (one WalkDir per request)
    if cache.cs_file_index.is_none() {
        let walkdir_start = Instant::now();
        let mut index = HashMap::new();
        for entry in WalkDir::new(cwd)
            .into_iter()
            .filter_entry(|entry| !should_skip_script_search_dir(entry.path()))
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let entry_path = entry.path();
            let Some(fname) = entry_path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if !fname.ends_with(".cs") {
                continue;
            }
            if let Ok(relative) = entry_path.strip_prefix(cwd) {
                if let Some(norm) = normalize_rel_path(relative) {
                    index.insert(fname.to_ascii_lowercase(), norm);
                }
            }
        }
        cache.walkdir_ms = walkdir_start.elapsed().as_millis() as u64;
        cache.cs_file_index = Some(index);
    }

    let found = cache
        .cs_file_index
        .as_ref()
        .and_then(|idx| idx.get(&file_name.to_ascii_lowercase()).cloned());

    cache
        .class_paths
        .insert(class_name.to_string(), found.clone());
    found
}

pub(crate) fn resolve_effective_base_type(
    side_ctx: &SideContext,
    current_script_path: &str,
    direct_base_type: Option<String>,
    env: &mut SemanticBuildEnv,
    visited: &mut HashSet<String>,
) -> Option<String> {
    let base_type = direct_base_type?;
    if is_unity_terminal_base_type(&base_type) {
        return Some(base_type);
    }
    if !visited.insert(base_type.clone()) {
        return Some(base_type);
    }

    let base_script_path = find_script_path_by_class_name(
        env.cwd,
        Some(current_script_path),
        &base_type,
        &mut env.script_cache,
    )?;
    let content = load_side_text_file(
        env.cwd,
        &base_script_path,
        side_ctx,
        env.batch_reader.as_mut(),
        env.profiler,
    )?;
    let metadata = parse_cs_script(&content, file_stem(&base_script_path))?;
    resolve_effective_base_type(
        side_ctx,
        &base_script_path,
        metadata.base_type.clone(),
        env,
        visited,
    )
    .or(metadata.base_type)
}

/// Helper: extract the file stem (without extension) from a relative path
/// for passing to `parse_cs_script` as the expected primary type name.
fn file_stem(rel_path: &str) -> Option<&str> {
    Path::new(rel_path).file_stem().and_then(|s| s.to_str())
}

pub(crate) fn load_side_text_file(
    cwd: &str,
    relative_path: &str,
    side_ctx: &SideContext,
    batch_reader: Option<&mut BatchBlobReader>,
    profiler: &mut DiffProfiler,
) -> Option<String> {
    match &side_ctx.file_source {
        SideFileSource::Workspace => {
            let full_path = Path::new(cwd).join(relative_path);
            std::fs::read_to_string(full_path).ok()
        }
        SideFileSource::GitRef(reference) => {
            let ref_spec = format!("{}:{}", reference, relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    // LFS pointer — try sync smudge, do NOT fall back to workspace
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => {
                    let full_path = Path::new(cwd).join(relative_path);
                    std::fs::read_to_string(full_path).ok()
                }
            }
        }
        SideFileSource::GitIndex => {
            let ref_spec = format!(":{}", relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    // LFS pointer — try sync smudge, do NOT fall back to workspace
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => {
                    let full_path = Path::new(cwd).join(relative_path);
                    std::fs::read_to_string(full_path).ok()
                }
            }
        }
        SideFileSource::GitStage(n) => {
            let ref_spec = format!(":{}:{}", n, relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => {
                    let full_path = Path::new(cwd).join(relative_path);
                    std::fs::read_to_string(full_path).ok()
                }
            }
        }
    }
}

/// Like `load_side_text_file` but without workspace fallback for snapshot sides.
/// Returns None when the file does not exist in the target snapshot/index, rather
/// than silently returning workspace content.
pub(crate) fn load_side_text_file_strict(
    cwd: &str,
    relative_path: &str,
    side_ctx: &SideContext,
    batch_reader: Option<&mut BatchBlobReader>,
    profiler: &mut DiffProfiler,
) -> Option<String> {
    match &side_ctx.file_source {
        SideFileSource::Workspace => {
            let full_path = Path::new(cwd).join(relative_path);
            std::fs::read_to_string(full_path).ok()
        }
        SideFileSource::GitRef(reference) => {
            let ref_spec = format!("{}:{}", reference, relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => None, // strict: no workspace fallback
            }
        }
        SideFileSource::GitIndex => {
            let ref_spec = format!(":{}", relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => None, // strict: no workspace fallback
            }
        }
        SideFileSource::GitStage(n) => {
            let ref_spec = format!(":{}:{}", n, relative_path);
            let raw = batch_reader
                .and_then(|br| br.read_blob(&ref_spec))
                .or_else(|| {
                    let t = Instant::now();
                    let result = git_show_file_sync(cwd, &ref_spec);
                    profiler.record_git_call(t.elapsed().as_millis() as u64);
                    result
                });
            match raw {
                Some(content) if parse_lfs_pointer(&content).is_some() => {
                    resolve_lfs_sync(cwd, Some(content), &ref_spec)
                }
                Some(content) => Some(content),
                None => None, // strict: no workspace fallback
            }
        }
    }
}

/// Well-known Unity primitive / built-in types that should NOT be resolved further.
fn is_primitive_field_type(t: &str) -> bool {
    matches!(
        t,
        "int"
            | "float"
            | "double"
            | "long"
            | "short"
            | "byte"
            | "bool"
            | "string"
            | "String"
            | "Vector2"
            | "Vector3"
            | "Vector4"
            | "Quaternion"
            | "Color"
            | "Color32"
            | "Rect"
            | "Bounds"
            | "Matrix4x4"
            | "AnimationCurve"
            | "Gradient"
            | "LayerMask"
    )
}

/// Strip generic wrapper from field types: `List<Foo>` → `Foo`, `Foo[]` → `Foo`.
fn unwrap_collection_type(t: &str) -> &str {
    if let Some(inner) = t.strip_prefix("List<").and_then(|s| s.strip_suffix('>')) {
        return inner;
    }
    if let Some(inner) = t.strip_suffix("[]") {
        return inner;
    }
    t
}

/// Resolve the C# declared type for a nested property path by walking the type chain.
///
/// For a path like `hitData.hitEffect.magicDefenseCost`, it:
///   1. Looks up `hitData` in the root script → type `EntityHitData`
///   2. Finds and parses `EntityHitData.cs` → looks up `hitEffect` → type `HitEffectConfig`
///   3. Finds and parses `HitEffectConfig.cs` → looks up `magicDefenseCost` → type `float`
///
/// Returns the field type of the final segment, or None if any step fails.
pub(crate) fn resolve_nested_field_type(
    segments: &[String],
    root_info: &ScriptSemanticInfo,
    hint_script_path: Option<&str>,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    let mut current_info = root_info.clone();

    for (i, segment) in segments.iter().enumerate() {
        // Skip array indices — we can't resolve element types without generics tracking
        if segment.starts_with('[') {
            continue;
        }

        let enhancement = current_info.field_aliases.get(segment)?;
        let field_type = enhancement.field_type.as_ref()?;

        // Last segment — return its type
        if i == segments.len() - 1 {
            return Some(field_type.clone());
        }

        // Intermediate segment — resolve the type's script for the next level
        let inner_type = unwrap_collection_type(field_type);
        if is_primitive_field_type(inner_type) {
            // Primitive type has no sub-fields to resolve
            return None;
        }

        current_info = load_script_info_by_class_name(inner_type, hint_script_path, side_ctx, env)?;
    }

    None
}

/// Pre-resolve field types for all property paths using the script type chain.
/// Returns a map from property path to its C# declared type.
pub(crate) fn resolve_all_field_types(
    paths: impl Iterator<Item = impl AsRef<str>>,
    script_info: Option<&ScriptSemanticInfo>,
    hint_script_path: Option<&str>,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let Some(info) = script_info else {
        return result;
    };

    for path in paths {
        let path = path.as_ref();
        let segments = super::parse::split_property_path(path);
        if segments.is_empty() {
            continue;
        }

        // Top-level: direct lookup (fast path)
        if segments.len() == 1 {
            if let Some(enh) = info.field_aliases.get(&segments[0]) {
                if let Some(ft) = &enh.field_type {
                    result.insert(path.to_string(), ft.clone());
                }
            }
            continue;
        }

        // Nested: walk the type chain
        if let Some(ft) =
            resolve_nested_field_type(&segments, info, hint_script_path, side_ctx, env)
        {
            result.insert(path.to_string(), ft);
        }
    }

    result
}

/// Load script semantic info by C# class name (for resolving nested field types).
/// Uses `find_script_path_by_class_name` to locate the .cs file, then parses it.
pub(crate) fn load_script_info_by_class_name(
    class_name: &str,
    hint_script_path: Option<&str>,
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> Option<ScriptSemanticInfo> {
    let cache_key = format!("class:{}", class_name);
    if let Some(cached) = env.script_cache.infos.get(&cache_key) {
        return cached.clone();
    }

    let script_path = find_script_path_by_class_name(
        env.cwd,
        hint_script_path,
        class_name,
        &mut env.script_cache,
    )?;

    let info = load_side_text_file(
        env.cwd,
        &script_path,
        side_ctx,
        env.batch_reader.as_mut(),
        env.profiler,
    )
    .and_then(|content| parse_cs_script(&content, file_stem(&script_path)))
    .map(build_script_semantic_info);

    env.script_cache.infos.insert(cache_key, info.clone());
    info
}

pub(crate) fn load_script_semantic_info(
    doc: &YamlDoc,
    lines: &[String],
    side_ctx: &SideContext,
    env: &mut SemanticBuildEnv,
) -> Option<ScriptSemanticInfo> {
    if doc.class_id != 114 {
        return None;
    }

    let guid = doc_script_guid(doc, lines)?;
    let script_path = side_ctx.resolve_script_guid_path(&guid)?;
    let cache_key = script_cache_key(side_ctx, &script_path);
    if let Some(cached) = env.script_cache.infos.get(&cache_key) {
        return cached.clone();
    }

    let info = load_side_text_file(
        env.cwd,
        &script_path,
        side_ctx,
        env.batch_reader.as_mut(),
        env.profiler,
    )
    .and_then(|content| parse_cs_script(&content, file_stem(&script_path)))
    .map(|metadata| {
        let mut info = build_script_semantic_info(metadata.clone());
        let mut visited = HashSet::new();
        info.base_type = resolve_effective_base_type(
            side_ctx,
            &script_path,
            metadata.base_type,
            env,
            &mut visited,
        );
        info
    });

    env.script_cache.infos.insert(cache_key, info.clone());
    info
}
