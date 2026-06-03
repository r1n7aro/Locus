use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::KnowledgeType;
use crate::session::store::SessionStore;
use crate::vcs::undo::{ChangedFile, UndoConflict, UndoEntry, UndoPerformError};
use crate::workspace::Workspace;
use crate::UndoManagerHandle;

const MAX_INCREMENTAL_KNOWLEDGE_UNDO_DOCS: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoConflictInfo {
    pub session_id: String,
    pub session_title: String,
    pub assistant_message_id: String,
    pub checkpoint: crate::vcs::Checkpoint,
    pub changed_files: Vec<ChangedFile>,
}

fn enrich_conflicts(store: &SessionStore, conflicts: Vec<UndoConflict>) -> Vec<UndoConflictInfo> {
    conflicts
        .into_iter()
        .map(|conflict| UndoConflictInfo {
            session_title: store
                .get_session_title(&conflict.session_id)
                .ok()
                .flatten()
                .unwrap_or_else(|| conflict.session_id.clone()),
            session_id: conflict.session_id,
            assistant_message_id: conflict.assistant_message_id,
            checkpoint: conflict.checkpoint,
            changed_files: conflict.changed_files,
        })
        .collect()
}

fn format_conflict_detail(conflicts: &[UndoConflictInfo]) -> String {
    conflicts
        .iter()
        .map(|conflict| {
            let files = conflict
                .changed_files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "- {} [{}]: {}",
                conflict.session_title, conflict.session_id, files
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KnowledgeUndoSyncPlan {
    None,
    Incremental(Vec<(KnowledgeType, String)>),
    Reconcile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KnowledgePathKind {
    Document(KnowledgeType, String),
    Reconcile,
}

fn strip_workspace_knowledge_prefix(path: &str) -> Option<String> {
    let normalized = path.trim().replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if lower == "locus/knowledge" {
        return Some(String::new());
    }
    if lower.starts_with("locus/knowledge/") {
        return Some(
            trimmed["locus/knowledge/".len()..]
                .trim_matches('/')
                .to_string(),
        );
    }
    None
}

fn knowledge_type_from_segment(segment: &str) -> Option<KnowledgeType> {
    match segment.to_ascii_lowercase().as_str() {
        "design" => Some(KnowledgeType::Design),
        "memory" => Some(KnowledgeType::Memory),
        "skill" => Some(KnowledgeType::Skill),
        "reference" => Some(KnowledgeType::Reference),
        _ => None,
    }
}

fn classify_workspace_knowledge_path(path: &str) -> Option<KnowledgePathKind> {
    let stripped = strip_workspace_knowledge_prefix(path)?;
    if stripped.is_empty() {
        return Some(KnowledgePathKind::Reconcile);
    }

    let Some((doc_type_segment, relative_path)) = stripped.split_once('/') else {
        return Some(KnowledgePathKind::Reconcile);
    };
    let Some(doc_type) = knowledge_type_from_segment(doc_type_segment) else {
        return Some(KnowledgePathKind::Reconcile);
    };
    let relative_path = relative_path.trim_matches('/');
    if !relative_path.to_ascii_lowercase().ends_with(".md") {
        return Some(KnowledgePathKind::Reconcile);
    }

    match crate::knowledge_store::ensure_document_path(relative_path) {
        Ok(path) => Some(KnowledgePathKind::Document(doc_type, path)),
        Err(_) => Some(KnowledgePathKind::Reconcile),
    }
}

fn knowledge_target_key(doc_type: KnowledgeType, path: &str) -> String {
    let key = format!("{}/{}", doc_type.as_str(), path.replace('\\', "/"));
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

fn add_incremental_knowledge_target(
    targets: &mut Vec<(KnowledgeType, String)>,
    seen: &mut HashSet<String>,
    doc_type: KnowledgeType,
    path: String,
) {
    if seen.insert(knowledge_target_key(doc_type, &path)) {
        targets.push((doc_type, path));
    }
}

fn knowledge_undo_sync_plan(files: &[ChangedFile]) -> KnowledgeUndoSyncPlan {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut touched_knowledge = false;

    for file in files {
        for path in std::iter::once(file.path.as_str()).chain(file.old_path.as_deref()) {
            let Some(kind) = classify_workspace_knowledge_path(path) else {
                continue;
            };
            touched_knowledge = true;
            match kind {
                KnowledgePathKind::Document(doc_type, path) => {
                    add_incremental_knowledge_target(&mut targets, &mut seen, doc_type, path);
                    if targets.len() > MAX_INCREMENTAL_KNOWLEDGE_UNDO_DOCS {
                        return KnowledgeUndoSyncPlan::Reconcile;
                    }
                }
                KnowledgePathKind::Reconcile => return KnowledgeUndoSyncPlan::Reconcile,
            }
        }
    }

    if !touched_knowledge {
        KnowledgeUndoSyncPlan::None
    } else if targets.is_empty() {
        KnowledgeUndoSyncPlan::Reconcile
    } else {
        KnowledgeUndoSyncPlan::Incremental(targets)
    }
}

fn normalized_workspace_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_ascii_lowercase()
}

fn is_workspace_view_path(path: &str) -> bool {
    let normalized = normalized_workspace_path(path);
    let view_root = crate::view::VIEW_ROOT_RELATIVE.to_ascii_lowercase();
    normalized == view_root || normalized.starts_with(&format!("{view_root}/"))
}

fn undo_touches_view_tree(files: &[ChangedFile]) -> bool {
    files.iter().any(|file| {
        std::iter::once(file.path.as_str())
            .chain(file.old_path.as_deref())
            .any(is_workspace_view_path)
    })
}

async fn sync_knowledge_after_undo(
    app_handle: &AppHandle,
    working_dir: &str,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    files: &[ChangedFile],
) {
    match knowledge_undo_sync_plan(files) {
        KnowledgeUndoSyncPlan::None => {}
        KnowledgeUndoSyncPlan::Incremental(targets) => {
            if let Err(error) =
                crate::commands::knowledge::sync_visible_documents_for_paths_and_emit(
                    app_handle,
                    working_dir,
                    knowledge_index_state.clone(),
                    "undo_perform",
                    &targets,
                )
                .await
            {
                eprintln!(
                    "[undo_perform] failed to sync knowledge documents after undo: {}",
                    error
                );
                if let Err(error) =
                    crate::commands::knowledge::reconcile_and_emit_knowledge_changed(
                        app_handle,
                        working_dir,
                        knowledge_index_state,
                        "undo_perform",
                    )
                    .await
                {
                    eprintln!(
                        "[undo_perform] failed to reconcile knowledge index after incremental sync failure: {}",
                        error
                    );
                }
            }
        }
        KnowledgeUndoSyncPlan::Reconcile => {
            if let Err(error) = crate::commands::knowledge::reconcile_and_emit_knowledge_changed(
                app_handle,
                working_dir,
                knowledge_index_state,
                "undo_perform",
            )
            .await
            {
                eprintln!(
                    "[undo_perform] failed to reconcile knowledge index after undo: {}",
                    error
                );
            }
        }
    }
}

#[tauri::command]
pub async fn undo_perform(
    session_id: String,
    assistant_message_id: String,
    force: Option<bool>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    undo_manager: State<'_, UndoManagerHandle>,
    store: State<'_, Arc<SessionStore>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<UndoEntry, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let result = match undo_manager
        .perform_undo_checked(
            &session_id,
            &assistant_message_id,
            &working_dir,
            force.unwrap_or(false),
        )
        .await
    {
        Ok(entry) => entry,
        Err(UndoPerformError::Conflict(conflicts)) => {
            let conflicts = enrich_conflicts(store.inner(), conflicts);
            return Err(AppError::new(
                "undo.conflict",
                "Undo blocked because newer changes from other sessions would be overwritten.",
            )
            .detail(format_conflict_detail(&conflicts))
            .operation("undo"));
        }
        Err(UndoPerformError::Other(msg)) => return Err(msg.into()),
    };
    if let Err(e) = store.truncate_from_message(&session_id, &result.entry.assistant_message_id) {
        eprintln!("[undo_perform] failed to truncate messages: {}", e);
    } else {
        if let Err(e) = store.set_latest_completed_run_id(&session_id, None) {
            eprintln!(
                "[undo_perform] failed to clear latest completed run id for session {}: {}",
                session_id, e
            );
        }
        crate::llm::codex::reset_cached_session_window(&session_id).await;
    }

    sync_knowledge_after_undo(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        &result.restored_files,
    )
    .await;

    if undo_touches_view_tree(&result.restored_files) {
        crate::view::emit_view_tree_changed(&app_handle);
    }

    super::emit_session_content_changed(&app_handle, &working_dir, &session_id, "undo_perform");

    Ok(result.entry)
}

#[tauri::command]
pub async fn undo_perform_to_message(
    session_id: String,
    assistant_message_id: String,
    truncate_message_id: String,
    force: Option<bool>,
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    undo_manager: State<'_, UndoManagerHandle>,
    store: State<'_, Arc<SessionStore>>,
    knowledge_index_state: State<'_, Arc<KnowledgeIndexState>>,
) -> Result<UndoEntry, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let result = match undo_manager
        .perform_undo_checked(
            &session_id,
            &assistant_message_id,
            &working_dir,
            force.unwrap_or(false),
        )
        .await
    {
        Ok(entry) => entry,
        Err(UndoPerformError::Conflict(conflicts)) => {
            let conflicts = enrich_conflicts(store.inner(), conflicts);
            return Err(AppError::new(
                "undo.conflict",
                "Undo blocked because newer changes from other sessions would be overwritten.",
            )
            .detail(format_conflict_detail(&conflicts))
            .operation("undo"));
        }
        Err(UndoPerformError::Other(msg)) => return Err(msg.into()),
    };

    if let Err(e) = store.truncate_after_message(&session_id, &truncate_message_id) {
        eprintln!(
            "[undo_perform_to_message] failed to truncate messages after {}: {}",
            truncate_message_id, e
        );
    } else {
        crate::llm::codex::reset_cached_session_window(&session_id).await;
    }

    sync_knowledge_after_undo(
        &app_handle,
        &working_dir,
        knowledge_index_state.inner().clone(),
        &result.restored_files,
    )
    .await;

    if undo_touches_view_tree(&result.restored_files) {
        crate::view::emit_view_tree_changed(&app_handle);
    }

    super::emit_session_content_changed(
        &app_handle,
        &working_dir,
        &session_id,
        "undo_perform_to_message",
    );

    Ok(result.entry)
}

#[tauri::command]
pub async fn undo_preview(
    session_id: String,
    assistant_message_id: String,
    undo_manager: State<'_, UndoManagerHandle>,
) -> Result<Vec<ChangedFile>, AppError> {
    undo_manager
        .preview(&session_id, &assistant_message_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn undo_list(
    session_id: String,
    undo_manager: State<'_, UndoManagerHandle>,
) -> Result<Vec<UndoEntry>, AppError> {
    Ok(undo_manager.list_entries(&session_id).await)
}

#[tauri::command]
pub async fn undo_check_conflicts(
    session_id: String,
    assistant_message_id: String,
    undo_manager: State<'_, UndoManagerHandle>,
    store: State<'_, Arc<SessionStore>>,
) -> Result<Vec<UndoConflictInfo>, AppError> {
    Ok(enrich_conflicts(
        store.inner(),
        undo_manager
            .check_conflicts(&session_id, &assistant_message_id)
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{knowledge_undo_sync_plan, undo_touches_view_tree, KnowledgeUndoSyncPlan};
    use crate::knowledge_store::KnowledgeType;
    use crate::vcs::undo::ChangedFile;

    fn changed(path: &str) -> ChangedFile {
        ChangedFile {
            status: "M".to_string(),
            path: path.to_string(),
            old_path: None,
        }
    }

    fn renamed(old_path: &str, path: &str) -> ChangedFile {
        ChangedFile {
            status: "R".to_string(),
            path: path.to_string(),
            old_path: Some(old_path.to_string()),
        }
    }

    #[test]
    fn knowledge_undo_sync_plan_uses_incremental_for_single_document() {
        assert_eq!(
            knowledge_undo_sync_plan(&[changed("Locus/knowledge/design/core-loop.md")]),
            KnowledgeUndoSyncPlan::Incremental(vec![(
                KnowledgeType::Design,
                "core-loop.md".to_string()
            )])
        );
    }

    #[test]
    fn knowledge_undo_sync_plan_syncs_both_paths_for_document_rename() {
        assert_eq!(
            knowledge_undo_sync_plan(&[renamed(
                "Locus/knowledge/design/old-loop.md",
                "Locus/knowledge/design/new-loop.md"
            )]),
            KnowledgeUndoSyncPlan::Incremental(vec![
                (KnowledgeType::Design, "new-loop.md".to_string()),
                (KnowledgeType::Design, "old-loop.md".to_string()),
            ])
        );
    }

    #[test]
    fn knowledge_undo_sync_plan_reconciles_for_directory_or_metadata_changes() {
        assert_eq!(
            knowledge_undo_sync_plan(&[changed("Locus/knowledge/design")]),
            KnowledgeUndoSyncPlan::Reconcile
        );
        assert_eq!(
            knowledge_undo_sync_plan(&[changed("Locus/knowledge/design/core.locus-meta")]),
            KnowledgeUndoSyncPlan::Reconcile
        );
    }

    #[test]
    fn knowledge_undo_sync_plan_reconciles_for_large_batches() {
        let files = (0..=super::MAX_INCREMENTAL_KNOWLEDGE_UNDO_DOCS)
            .map(|index| changed(&format!("Locus/knowledge/memory/doc-{index}.md")))
            .collect::<Vec<_>>();

        assert_eq!(
            knowledge_undo_sync_plan(&files),
            KnowledgeUndoSyncPlan::Reconcile
        );
    }

    #[test]
    fn knowledge_undo_sync_plan_ignores_non_knowledge_paths() {
        assert_eq!(
            knowledge_undo_sync_plan(&[changed("src/main.rs")]),
            KnowledgeUndoSyncPlan::None
        );
    }

    #[test]
    fn undo_touches_view_tree_detects_view_paths() {
        assert!(undo_touches_view_tree(&[changed(
            "Locus/View/ProjectName/player-tool/view.json"
        )]));
        assert!(undo_touches_view_tree(&[changed(
            "locus\\view\\ProjectName\\player-tool\\src\\App.vue"
        )]));
        assert!(undo_touches_view_tree(&[changed("Locus/View")]));
    }

    #[test]
    fn undo_touches_view_tree_detects_renamed_old_view_path() {
        assert!(undo_touches_view_tree(&[renamed(
            "Locus/View/ProjectName/player-tool/view.json",
            "Assets/player-tool-view.json"
        )]));
    }

    #[test]
    fn undo_touches_view_tree_ignores_adjacent_paths() {
        assert!(!undo_touches_view_tree(&[changed(
            "Locus/Viewer/tool.json"
        )]));
        assert!(!undo_touches_view_tree(&[changed(
            "Assets/Locus/View/tool.json"
        )]));
        assert!(!undo_touches_view_tree(&[changed("src/main.rs")]));
    }
}
