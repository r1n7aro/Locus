use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use tauri::AppHandle;

use crate::asset_db::AssetDbState;
use crate::error::AppResult;

use super::content::{is_binary, is_unity_yaml, lang_from_path, SideContentState};
use super::context::DiffBuildContext;
use super::profiler::{DiffPhase, DiffProfiler};
use super::semantic;
use super::text::{
    compute_hunks, count_stats, truncate_for_preview, MAX_TEXT_DIFF_BYTES, MAX_TEXT_DIFF_LINES,
};
use super::types::*;

// ── Semantic session ──

#[derive(Debug, Clone)]
pub(crate) struct SemanticSession {
    pub(crate) layout: SemanticLayout,
    pub(crate) asset_kind: UnityAssetKind,
    pub(crate) summary: SemanticSummary,
    pub(crate) default_target_id: Option<String>,
    pub(crate) script_class_name: Option<String>,
    pub(crate) tree: Vec<SemanticTreeNode>,
    pub(crate) targets: Vec<SemanticTargetSummary>,
    pub(crate) changed_inspectors: HashMap<String, SemanticTargetInspector>,
    pub(crate) full_inspectors: HashMap<String, SemanticTargetInspector>,
}

#[derive(Debug, Default)]
struct SemanticCache {
    order: VecDeque<String>,
    sessions: HashMap<String, Arc<SemanticSession>>,
}

const SEMANTIC_CACHE_CAPACITY: usize = 64;

fn semantic_cache() -> &'static Mutex<SemanticCache> {
    static CACHE: OnceLock<Mutex<SemanticCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SemanticCache::default()))
}

pub(crate) fn cache_semantic_session(key: &str, session: SemanticSession) {
    if let Ok(mut cache) = semantic_cache().lock() {
        if !cache.sessions.contains_key(key) {
            cache.order.push_back(key.to_string());
        }
        cache.sessions.insert(key.to_string(), Arc::new(session));
        while cache.order.len() > SEMANTIC_CACHE_CAPACITY {
            if let Some(oldest) = cache.order.pop_front() {
                cache.sessions.remove(&oldest);
            }
        }
    }
}

pub(crate) fn get_semantic_session(key: &str) -> Option<Arc<SemanticSession>> {
    let mut cache = semantic_cache().lock().ok()?;
    let session = cache.sessions.get(key)?.clone();
    if let Some(index) = cache.order.iter().position(|item| item == key) {
        cache.order.remove(index);
    }
    cache.order.push_back(key.to_string());
    Some(session)
}

pub(crate) fn build_semantic_payload(session: &SemanticSession) -> SemanticDiff {
    let inspector = session
        .default_target_id
        .as_ref()
        .and_then(|target_id| session.changed_inspectors.get(target_id))
        .cloned();

    SemanticDiff {
        engine: "unityYaml".into(),
        asset_kind: session.asset_kind.clone(),
        layout: session.layout.clone(),
        summary: session.summary.clone(),
        default_target_id: session.default_target_id.clone(),
        script_class_name: session.script_class_name.clone(),
        tree: if matches!(session.layout, SemanticLayout::SceneHierarchyInspector) {
            Some(session.tree.clone())
        } else {
            None
        },
        targets: if matches!(session.layout, SemanticLayout::AssetInspector) {
            Some(session.targets.clone())
        } else {
            None
        },
        inspector,
    }
}

pub(crate) fn semantic_target_lookup_detail(
    session: &SemanticSession,
    target_id: &str,
    include_unchanged: bool,
) -> String {
    let available_targets = if include_unchanged {
        &session.full_inspectors
    } else {
        &session.changed_inspectors
    };
    let available_target_ids = available_targets
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let tree_node = session.tree.iter().find(|node| node.id == target_id);
    let tree_context = tree_node
        .map(|node| {
            format!(
                "treeNode={{label:'{}', path:'{}', changeKind:'{}', hasInspector:{}, childCount:{}}}",
                node.label,
                node.path,
                node.change_kind,
                node.has_inspector,
                node.child_ids.len()
            )
        })
        .unwrap_or_else(|| "treeNode=<missing>".to_string());

    format!(
        "layout={:?}, includeUnchanged={}, availableTargetCount={}, availableTargets={:?}, {}",
        session.layout,
        include_unchanged,
        available_target_ids.len(),
        available_target_ids,
        tree_context
    )
}

pub(crate) fn build_preview_summary(
    stats: &DiffStats,
    semantic: Option<&SemanticDiff>,
) -> Vec<String> {
    let mut summary = vec![format!(
        "+{} -{} in {} hunks",
        stats.additions, stats.deletions, stats.changed_hunks
    )];

    if let Some(semantic) = semantic {
        match semantic.layout {
            SemanticLayout::SceneHierarchyInspector => summary.push(format!(
                "{} objects changed / {} components changed",
                semantic.summary.changed_objects, semantic.summary.changed_components
            )),
            SemanticLayout::AssetInspector => {
                summary.push(format!(
                    "{} fields changed",
                    semantic.summary.changed_fields
                ));
            }
        }
    }

    summary
}

/// Core diff pipeline — extracted from diff_single_file command.
pub(crate) async fn build_file_diff_payload(
    app_handle: &AppHandle,
    cwd: &str,
    request: &FileDiffRequest,
    undo_mgr: &crate::vcs::UndoManager,
    ref_graph_state: &AssetDbState,
    binary_cache: &crate::binary_cache::BinaryCache,
    debug: bool,
) -> AppResult<FileDiffPayload> {
    let file_path = request.file_path.clone();
    let old_path = request.old_path.clone();
    let detail = request.detail;
    let is_unity = is_unity_yaml(&file_path);

    let key = format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        match request.source {
            DiffSource::GitCommit => "gitCommit",
            DiffSource::GitStaged => "gitStaged",
            DiffSource::GitUnstaged => "gitUnstaged",
            DiffSource::ChatCheckpoint => "chatCheckpoint",
            DiffSource::GitConflictBaseToLeft => "gitConflictBaseToLeft",
            DiffSource::GitConflictBaseToRight => "gitConflictBaseToRight",
        },
        file_path,
        old_path.as_deref().unwrap_or(""),
        request.commit_hash.as_deref().unwrap_or(""),
        request.session_id.as_deref().unwrap_or(""),
        request.assistant_message_id.as_deref().unwrap_or(""),
        if detail == DiffDetail::Preview {
            "preview"
        } else {
            "full"
        },
        if request.full_context { "fc" } else { "" },
    );

    let mut profiler = DiffProfiler::new(key.clone(), is_unity, debug);

    // Emit FetchContent BEFORE fetch so UI shows progress during the work
    semantic::emit_diff_progress(app_handle, &profiler, DiffPhase::FetchContent, None);
    let mut pair = match super::content::fetch_content(cwd, request, undo_mgr, &mut profiler).await
    {
        Ok(p) => p,
        Err(e) => {
            profiler.record(DiffPhase::Error);
            semantic::emit_diff_progress(
                app_handle,
                &profiler,
                DiffPhase::Error,
                Some(e.message.clone()),
            );
            return Err(e);
        }
    };
    profiler.record(DiffPhase::FetchContent);

    // LFS pointer-only early return: at least one side could not be smudged
    let lfs_not_fetched = match (&pair.old_content_state, &pair.new_content_state) {
        (SideContentState::LfsPointerOnly { oid, size }, _) => Some((oid.clone(), *size)),
        (_, SideContentState::LfsPointerOnly { oid, size }) => Some((oid.clone(), *size)),
        _ => None,
    };
    if let Some((oid, size)) = lfs_not_fetched {
        profiler.record(DiffPhase::Done);
        semantic::emit_diff_progress(app_handle, &profiler, DiffPhase::Done, None);
        profiler.log_summary(&file_path);
        return Ok(FileDiffPayload {
            key,
            file_path,
            old_path,
            status: pair.status,
            language: lang_from_path(&request.file_path),
            is_binary: false,
            is_large: false,
            content_state: DiffContentState::LfsNotFetched { oid, size },
            stats: DiffStats {
                additions: 0,
                deletions: 0,
                changed_hunks: 0,
            },
            preview_summary: vec!["LFS object not fetched".into()],
            text: None,
            semantic: None,
            binary_preview: None,
        });
    }

    // Determine content_state for final payload
    let content_state = match (&pair.old_content_state, &pair.new_content_state) {
        (SideContentState::LfsResolved, _)
        | (_, SideContentState::LfsResolved)
        | (SideContentState::LfsBinaryResolved, _)
        | (_, SideContentState::LfsBinaryResolved) => DiffContentState::LfsResolved,
        _ => DiffContentState::Normal,
    };

    // LFS binary resolved or regular binary: either side is binary
    let is_lfs_binary = matches!(&pair.old_content_state, SideContentState::LfsBinaryResolved)
        || matches!(&pair.new_content_state, SideContentState::LfsBinaryResolved);
    if is_lfs_binary || is_binary(&pair.old_content) || is_binary(&pair.new_content) {
        // Build binary preview if we have bytes for a supported format
        let binary_preview = {
            let old_b = pair.old_bytes.as_deref();
            let new_b = pair.new_bytes.as_deref();
            let detect = new_b.or(old_b).unwrap_or(&[]);
            super::content::detect_binary_kind(&file_path, detect).and_then(|kind| {
                let mut before = None;
                let mut after = None;
                if let Some(ob) = pair.old_bytes.take() {
                    let size = ob.len() as u64;
                    if super::content::within_binary_threshold(kind, size) {
                        let mime = super::content::mime_for_ext(&file_path);
                        let blob_id = binary_cache.insert(ob, mime.clone());
                        before = Some(BinaryAssetRef {
                            url: format!("http://locus-binary.localhost/blob/{}", blob_id),
                            mime_type: Some(mime),
                            byte_size: size,
                        });
                    }
                }
                if let Some(nb) = pair.new_bytes.take() {
                    let size = nb.len() as u64;
                    if super::content::within_binary_threshold(kind, size) {
                        let mime = super::content::mime_for_ext(&file_path);
                        let blob_id = binary_cache.insert(nb, mime.clone());
                        after = Some(BinaryAssetRef {
                            url: format!("http://locus-binary.localhost/blob/{}", blob_id),
                            mime_type: Some(mime),
                            byte_size: size,
                        });
                    }
                }
                if before.is_some() || after.is_some() {
                    Some(BinaryPreview {
                        kind,
                        before,
                        after,
                    })
                } else {
                    None
                }
            })
        };

        profiler.record(DiffPhase::Done);
        semantic::emit_diff_progress(app_handle, &profiler, DiffPhase::Done, None);
        profiler.log_summary(&file_path);
        return Ok(FileDiffPayload {
            key,
            file_path,
            old_path,
            status: pair.status,
            language: lang_from_path(&request.file_path),
            is_binary: true,
            is_large: false,
            content_state,
            stats: DiffStats {
                additions: 0,
                deletions: 0,
                changed_hunks: 0,
            },
            preview_summary: vec!["Binary file".into()],
            text: None,
            semantic: None,
            binary_preview,
        });
    }

    let total_lines = pair
        .old_content
        .lines()
        .count()
        .max(pair.new_content.lines().count());
    let max_bytes = pair.old_content.len().max(pair.new_content.len());
    let is_large = total_lines > MAX_TEXT_DIFF_LINES || max_bytes > MAX_TEXT_DIFF_BYTES;

    let (text_result, stats) = if is_large {
        // Skip expensive text diff for large files; it can be requested on-demand later
        profiler.record(DiffPhase::TextDiff);
        (
            None,
            DiffStats {
                additions: 0,
                deletions: 0,
                changed_hunks: 0,
            },
        )
    } else {
        let context = if request.full_context {
            total_lines
        } else if detail == DiffDetail::Preview {
            1
        } else {
            3
        };
        // Emit TextDiff BEFORE compute so UI shows progress during the work
        semantic::emit_diff_progress(app_handle, &profiler, DiffPhase::TextDiff, None);
        let all_hunks = compute_hunks(&pair.old_content, &pair.new_content, context);
        let stats = count_stats(&all_hunks);
        let hunks = if detail == DiffDetail::Preview {
            truncate_for_preview(all_hunks)
        } else {
            all_hunks
        };
        profiler.record(DiffPhase::TextDiff);
        (Some(TextDiffResult { hunks }), stats)
    };

    let should_build_semantic = is_unity && !(detail == DiffDetail::Preview && is_large);
    let semantic_result = if should_build_semantic {
        eprintln!("[diff] Unity YAML detected: {}", request.file_path);
        let build_ctx = DiffBuildContext::from_sources(
            request.source.clone(),
            ref_graph_state,
            pair.old_file_source.clone(),
            pair.new_file_source.clone(),
        );
        if let Some(session) = semantic::build_semantic_session(
            &request.file_path,
            &pair.old_content,
            &pair.new_content,
            &build_ctx,
            cwd,
            app_handle,
            &mut profiler,
        ) {
            eprintln!(
                "[diff] Semantic session built: {} targets, {} fields",
                session.summary.changed_targets, session.summary.changed_fields
            );
            let payload = build_semantic_payload(&session);
            cache_semantic_session(&key, session);
            Some(payload)
        } else {
            eprintln!("[diff] Semantic session is None for {}", request.file_path);
            None
        }
    } else {
        if is_unity && detail == DiffDetail::Preview && is_large {
            eprintln!(
                "[diff] Unity YAML preview skipped semantic diff for large file: {}",
                request.file_path
            );
        }
        None
    };

    profiler.record(DiffPhase::Done);
    semantic::emit_diff_progress(app_handle, &profiler, DiffPhase::Done, None);
    profiler.log_summary(&file_path);

    let preview_summary = if is_large {
        let mut summary = vec![format!(
            "File too large ({} lines, {} KiB)",
            total_lines,
            max_bytes / 1024
        )];
        if let Some(ref sem) = semantic_result {
            summary.extend(build_preview_summary(&stats, Some(sem)));
        }
        summary
    } else {
        build_preview_summary(&stats, semantic_result.as_ref())
    };

    let result = FileDiffPayload {
        key,
        file_path,
        old_path,
        status: pair.status,
        language: lang_from_path(&request.file_path),
        is_binary: false,
        is_large,
        content_state,
        stats: stats.clone(),
        preview_summary,
        text: text_result,
        semantic: semantic_result,
        binary_preview: None,
    };
    eprintln!(
        "[diff] returning payload, hunks={}, semantic={}",
        result.text.as_ref().map_or(0, |t| t.hunks.len()),
        result.semantic.is_some()
    );
    Ok(result)
}

/// Compute text diff on demand for large files that were initially skipped.
pub(crate) async fn compute_text_diff_on_demand(
    cwd: &str,
    request: &FileDiffRequest,
    undo_mgr: &crate::vcs::UndoManager,
) -> AppResult<TextDiffResult> {
    let mut profiler = DiffProfiler::new(String::new(), false, false);
    let pair = super::content::fetch_content(cwd, request, undo_mgr, &mut profiler).await?;

    let total_lines = pair
        .old_content
        .lines()
        .count()
        .max(pair.new_content.lines().count());
    let context = if request.full_context { total_lines } else { 3 };

    let hunks = compute_hunks(&pair.old_content, &pair.new_content, context);
    Ok(TextDiffResult { hunks })
}
