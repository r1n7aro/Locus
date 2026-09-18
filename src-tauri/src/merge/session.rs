use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tauri::AppHandle;

use crate::asset_db::AssetDbState;
use crate::diff::profiler::{DiffPhase, DiffProfiler};
use crate::error::AppResult;

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

fn emit_merge_progress(
    app_handle: Option<&AppHandle>,
    event_scope: Option<&crate::workspace_service::event::WorkspaceEventScope>,
    profiler: &DiffProfiler,
    phase: DiffPhase,
) {
    if let (Some(handle), Some(scope)) = (app_handle, event_scope) {
        crate::workspace_service::event::emit_for_workspace_scope(
            handle,
            scope,
            "merge-progress",
            profiler.progress_event(phase, None),
        );
    }
}

pub(crate) fn build_merge_session(
    cwd: &str,
    path: &str,
    base_oid: &str,
    left_oid: &str,
    right_oid: &str,
    _ref_graph_state: &AssetDbState,
    app_handle: Option<&AppHandle>,
    event_scope: Option<&crate::workspace_service::event::WorkspaceEventScope>,
) -> AppResult<MergeSemanticSession> {
    let mut profiler = DiffProfiler::new(format!("merge:{}", path), true, false);
    emit_merge_progress(app_handle, event_scope, &profiler, DiffPhase::FetchContent);
    let session = super::core_adapter::build(cwd, path, [base_oid, left_oid, right_oid])?;
    profiler.record(DiffPhase::BuildSemantic);
    emit_merge_progress(app_handle, event_scope, &profiler, DiffPhase::BuildSemantic);
    profiler.record(DiffPhase::Done);
    emit_merge_progress(app_handle, event_scope, &profiler, DiffPhase::Done);
    profiler.log_summary(path);
    Ok(session)
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
