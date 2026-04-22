use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::error::AppError;
use crate::knowledge_index::KnowledgeIndexState;
use crate::session::store::SessionStore;
use crate::vcs::undo::{ChangedFile, UndoConflict, UndoEntry, UndoPerformError};
use crate::workspace::Workspace;
use crate::UndoManagerHandle;

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

fn path_touches_workspace_knowledge(path: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    normalized.eq_ignore_ascii_case("Locus/knowledge")
        || normalized
            .to_ascii_lowercase()
            .starts_with("locus/knowledge/")
}

fn changed_file_touches_workspace_knowledge(file: &ChangedFile) -> bool {
    path_touches_workspace_knowledge(&file.path)
        || file
            .old_path
            .as_deref()
            .is_some_and(path_touches_workspace_knowledge)
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
    let knowledge_touched = result
        .restored_files
        .iter()
        .any(changed_file_touches_workspace_knowledge);

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

    if knowledge_touched {
        if let Err(error) = crate::commands::knowledge::reconcile_and_emit_knowledge_changed(
            &app_handle,
            &working_dir,
            knowledge_index_state.inner().clone(),
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
