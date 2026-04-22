use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tauri::{AppHandle, Emitter};

use crate::asset_db::AssetDbState;
use crate::diff::content::{is_unity_yaml, unity_asset_kind};
use crate::diff::profiler::{DiffPhase, DiffProfiler};
use crate::diff::types::{SemanticLayout, UnityAssetKind};
use crate::error::{AppError, AppResult};
use crate::unity_yaml::parse_yaml_docs;
use crate::vcs::git_merge::read_three_way;

use super::types::*;

// ── Merge session cache ──

const MERGE_CACHE_CAPACITY: usize = 32;

#[derive(Debug, Default)]
struct MergeCache {
    order: VecDeque<String>,
    sessions: HashMap<String, Arc<RwLock<MergeSemanticSession>>>,
}

fn merge_cache() -> &'static Mutex<MergeCache> {
    static CACHE: OnceLock<Mutex<MergeCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(MergeCache::default()))
}

pub(crate) fn cache_merge_session(key: &str, session: MergeSemanticSession) {
    if let Ok(mut cache) = merge_cache().lock() {
        if !cache.sessions.contains_key(key) {
            cache.order.push_back(key.to_string());
        }
        cache
            .sessions
            .insert(key.to_string(), Arc::new(RwLock::new(session)));
        while cache.order.len() > MERGE_CACHE_CAPACITY {
            if let Some(oldest) = cache.order.pop_front() {
                cache.sessions.remove(&oldest);
            }
        }
    }
}

pub(crate) fn get_merge_session(key: &str) -> Option<Arc<RwLock<MergeSemanticSession>>> {
    let mut cache = merge_cache().lock().ok()?;
    let session = cache.sessions.get(key)?.clone();
    // LRU bump
    if let Some(index) = cache.order.iter().position(|item| item == key) {
        cache.order.remove(index);
    }
    cache.order.push_back(key.to_string());
    Some(session)
}

// ── Session key ──

pub(crate) fn merge_session_key(
    path: &str,
    base_oid: &str,
    left_oid: &str,
    right_oid: &str,
) -> String {
    format!("merge:{}:{}:{}:{}", path, base_oid, left_oid, right_oid)
}

pub(crate) fn hash_workspace_bytes(workspace_bytes: Option<&[u8]>) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    workspace_bytes.hash(&mut hasher);
    hasher.finish()
}

// ── Build merge session ──

fn emit_merge_progress(app_handle: Option<&AppHandle>, profiler: &DiffProfiler, phase: DiffPhase) {
    if let Some(handle) = app_handle {
        let _ = handle.emit("merge-progress", profiler.progress_event(phase, None));
    }
}

pub(crate) fn build_merge_session(
    cwd: &str,
    path: &str,
    _base_oid: &str,
    _left_oid: &str,
    _right_oid: &str,
    ref_graph_state: &AssetDbState,
    app_handle: Option<&AppHandle>,
) -> AppResult<MergeSemanticSession> {
    let mut profiler = DiffProfiler::new(format!("merge:{}", path), true, false);

    // 1. Read three-way content from git index stages.
    let three_way = read_three_way(cwd, path);
    let base_content = three_way.base.unwrap_or_default();
    let ours_content = three_way.left.unwrap_or_default();
    let theirs_content = three_way.right.unwrap_or_default();

    profiler.record(DiffPhase::FetchContent);
    emit_merge_progress(app_handle, &profiler, DiffPhase::FetchContent);

    // 2. Validate: if any side is empty and content was expected, this is unusual.
    //    If all three are empty, nothing to merge.
    if base_content.is_empty() && ours_content.is_empty() && theirs_content.is_empty() {
        return Err(AppError::new(
            "merge.empty_content",
            "All three sides are empty",
        ));
    }

    // 3. Determine if this is Unity YAML and which kind.
    let is_yaml = is_unity_yaml(path);
    if !is_yaml {
        return Err(AppError::new(
            "merge.not_unity_yaml",
            "File is not a Unity YAML asset",
        ));
    }
    let asset_kind = unity_asset_kind(path);

    // 4. Parse YAML docs for all three sides (in parallel).
    let (base_docs, (ours_docs, theirs_docs)) = rayon::join(
        || parse_yaml_docs(base_content.as_bytes()),
        || {
            rayon::join(
                || parse_yaml_docs(ours_content.as_bytes()),
                || parse_yaml_docs(theirs_content.as_bytes()),
            )
        },
    );

    profiler.record(DiffPhase::ParseYaml);
    profiler.set_doc_counts(base_docs.len(), ours_docs.len());
    emit_merge_progress(app_handle, &profiler, DiffPhase::ParseYaml);

    // 5. Validate parse results: if content was non-empty but docs are empty, parse failed.
    let base_parse_ok = base_content.is_empty() || !base_docs.is_empty();
    let ours_parse_ok = ours_content.is_empty() || !ours_docs.is_empty();
    let theirs_parse_ok = theirs_content.is_empty() || !theirs_docs.is_empty();
    if !base_parse_ok || !ours_parse_ok || !theirs_parse_ok {
        return Err(AppError::new(
            "merge.parse_failed",
            "YAML parsing failed for one or more sides",
        ));
    }

    let (base_lines, (ours_lines, theirs_lines)) = rayon::join(
        || {
            base_content
                .lines()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        },
        || {
            rayon::join(
                || {
                    ours_content
                        .lines()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                },
                || {
                    theirs_content
                        .lines()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                },
            )
        },
    );

    // 6. Determine layout based on asset kind.
    let layout = match asset_kind {
        UnityAssetKind::Scene | UnityAssetKind::Prefab => SemanticLayout::SceneHierarchyInspector,
        _ => SemanticLayout::AssetInspector,
    };

    // 7. Build target summaries, locator metadata, and exact conflict ids.
    let built = super::inspector::build_merge_session_targets(
        cwd,
        &asset_kind,
        &base_docs,
        &ours_docs,
        &theirs_docs,
        &base_lines,
        &ours_lines,
        &theirs_lines,
        ref_graph_state,
        &mut profiler,
    )?;

    profiler.record(DiffPhase::BuildSemantic);
    emit_merge_progress(app_handle, &profiler, DiffPhase::BuildSemantic);
    profiler.record(DiffPhase::Done);
    emit_merge_progress(app_handle, &profiler, DiffPhase::Done);
    profiler.log_summary(path);

    // Hash workspace file content to detect external modifications at apply time.
    let workspace_hash = {
        let full_path = std::path::Path::new(cwd).join(path);
        std::fs::read(&full_path)
            .map(|bytes| hash_workspace_bytes(Some(bytes.as_slice())))
            .unwrap_or_else(|_| hash_workspace_bytes(None))
    };

    Ok(MergeSemanticSession {
        layout,
        asset_kind,
        summary: built.summary,
        tree: built.tree,
        targets: built.targets,
        inspectors: built.inspectors,
        target_locators: built.target_locators,
        conflict_field_ids: built.conflict_field_ids,
        base_docs,
        ours_docs,
        theirs_docs,
        base_lines,
        ours_lines,
        theirs_lines,
        workspace_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::hash_workspace_bytes;

    #[test]
    fn workspace_hash_treats_optional_bytes_consistently() {
        let bytes = b"%YAML 1.1\n";
        assert_eq!(
            hash_workspace_bytes(Some(bytes.as_slice())),
            hash_workspace_bytes(Some(bytes.as_slice()))
        );
        assert_ne!(
            hash_workspace_bytes(Some(bytes.as_slice())),
            hash_workspace_bytes(None)
        );
    }
}
