use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::asset_db::AssetDbState;
use crate::config::AppConfig;
use crate::error::{AppError, AppResult};
use crate::session::store::SessionStore;
use crate::workspace_service::{ProjectRegistry, ResolvedWorkspaceScope, WorkspaceRef};
use crate::UndoManagerHandle;

pub use crate::diff::compute_hunks;
pub use crate::diff::types::*;

fn resolve_diff_workspace_scope(
    request: &FileDiffRequest,
    workspace_ref: Option<&WorkspaceRef>,
    store: &SessionStore,
    registry: &ProjectRegistry,
    operation: &'static str,
) -> Result<ResolvedWorkspaceScope, AppError> {
    if let Some(session_id) = request
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    {
        return super::session::resolve_session_workspace_scope(
            store,
            registry,
            session_id,
            workspace_ref,
            operation,
        );
    }
    let workspace_ref = workspace_ref.ok_or_else(|| {
        AppError::new(
            "workspace.scope_required",
            "A workspace checkout is required for this diff request.",
        )
        .operation(operation)
    })?;
    super::session::resolve_workspace_scope(registry, workspace_ref, operation)
}

fn diff_cache_namespace(
    request: &FileDiffRequest,
    workspace_ref: Option<&WorkspaceRef>,
    scope: &ResolvedWorkspaceScope,
) -> String {
    if workspace_ref.is_none() {
        if let Some(session_id) = request
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|session_id| !session_id.is_empty())
        {
            return format!("s={session_id}");
        }
    }
    format!(
        "w={}@{}",
        scope.runtime().checkout_id(),
        scope.runtime().generation()
    )
}

#[tauri::command]
pub async fn diff_single_file(
    app_handle: AppHandle,
    config: State<'_, Arc<AppConfig>>,
    undo_mgr: State<'_, UndoManagerHandle>,
    binary_cache: State<'_, Arc<crate::binary_cache::BinaryCache>>,
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<ProjectRegistry>>,
    workspace_ref: Option<WorkspaceRef>,
    request: FileDiffRequest,
) -> AppResult<FileDiffPayload> {
    let scope = resolve_diff_workspace_scope(
        &request,
        workspace_ref.as_ref(),
        store.inner(),
        registry.inner(),
        "diff_single_file",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    let cache_namespace = diff_cache_namespace(&request, workspace_ref.as_ref(), &scope);
    let ref_graph_state = AssetDbState(scope.runtime().core().asset_db());

    crate::diff::service::build_file_diff_payload(
        &app_handle,
        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime()),
        &cwd,
        &cache_namespace,
        &request,
        &undo_mgr,
        &ref_graph_state,
        &binary_cache,
        config.debug_enabled(),
    )
    .await
}

#[tauri::command]
pub async fn diff_semantic_target(
    request: SemanticTargetRequest,
) -> AppResult<SemanticTargetInspector> {
    let session =
        crate::diff::service::get_semantic_session(&request.diff_key).ok_or_else(|| {
            AppError::new(
                "diff.semantic_missing",
                "Semantic diff session was not found. Reload the file diff and try again.",
            )
        })?;

    let inspector = if request.include_unchanged {
        session.full_inspectors.get(&request.target_id)
    } else {
        session.changed_inspectors.get(&request.target_id)
    }
    .cloned()
    .ok_or_else(|| {
        let detail = crate::diff::service::semantic_target_lookup_detail(
            &session,
            &request.target_id,
            request.include_unchanged,
        );
        eprintln!(
            "[diff] semantic target lookup failed: diff_key='{}', target_id='{}', {}",
            request.diff_key, request.target_id, detail
        );
        AppError::new(
            "diff.semantic_target_missing",
            format!("Semantic target '{}' was not found", request.target_id),
        )
        .detail(detail)
    })?;

    Ok(inspector)
}

#[tauri::command]
pub async fn diff_text_for_large(
    undo_mgr: State<'_, UndoManagerHandle>,
    store: State<'_, Arc<SessionStore>>,
    registry: State<'_, Arc<ProjectRegistry>>,
    workspace_ref: Option<WorkspaceRef>,
    request: FileDiffRequest,
) -> AppResult<TextDiffResult> {
    let scope = resolve_diff_workspace_scope(
        &request,
        workspace_ref.as_ref(),
        store.inner(),
        registry.inner(),
        "diff_text_for_large",
    )?;
    let cwd = scope.runtime().root().to_string_lossy().to_string();
    crate::diff::service::compute_text_diff_on_demand(&cwd, &request, &undo_mgr).await
}

#[tauri::command]
pub async fn diff_strings(
    old_text: String,
    new_text: String,
    context_lines: Option<usize>,
) -> AppResult<Vec<DiffHunk>> {
    Ok(compute_hunks(
        &old_text,
        &new_text,
        context_lines.unwrap_or(3),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> FileDiffRequest {
        FileDiffRequest {
            source: DiffSource::GitUnstaged,
            file_path: "Assets/Shared.cs".to_string(),
            old_path: None,
            commit_hash: None,
            session_id: None,
            assistant_message_id: None,
            detail: DiffDetail::Full,
            full_context: false,
        }
    }

    #[test]
    fn explicit_checkout_scope_keeps_same_relative_diff_paths_isolated() {
        let config_dir = tempfile::tempdir().expect("config");
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.path().join("config.json"),
        ));
        let policy = Arc::new(
            crate::resource_policy::ResourcePolicyStore::from_config(config).expect("policy"),
        );
        let registry = ProjectRegistry::new(policy, Vec::new());
        let root_a = tempfile::tempdir().expect("checkout A");
        let root_b = tempfile::tempdir().expect("checkout B");
        let runtime_a = registry.register(root_a.path()).expect("runtime A");
        let runtime_b = registry.register(root_b.path()).expect("runtime B");
        let workspace_ref_a = WorkspaceRef::for_runtime(&runtime_a);
        let workspace_ref_b = WorkspaceRef::for_runtime(&runtime_b);
        let store_dir = tempfile::tempdir().expect("store");
        let store = SessionStore::new(store_dir.path()).expect("store");
        let request = request();

        let scope_a = resolve_diff_workspace_scope(
            &request,
            Some(&workspace_ref_a),
            &store,
            &registry,
            "test_diff",
        )
        .expect("scope A");
        let scope_b = resolve_diff_workspace_scope(
            &request,
            Some(&workspace_ref_b),
            &store,
            &registry,
            "test_diff",
        )
        .expect("scope B");

        assert_ne!(scope_a.runtime().root(), scope_b.runtime().root());
        assert_ne!(
            diff_cache_namespace(&request, Some(&workspace_ref_a), &scope_a),
            diff_cache_namespace(&request, Some(&workspace_ref_b), &scope_b)
        );
    }

    #[test]
    fn non_session_diff_rejects_an_implicit_process_workspace() {
        let config_dir = tempfile::tempdir().expect("config");
        let config = Arc::new(crate::config::AppConfig::load_from_path(
            &config_dir.path().join("config.json"),
        ));
        let policy = Arc::new(
            crate::resource_policy::ResourcePolicyStore::from_config(config).expect("policy"),
        );
        let registry = ProjectRegistry::new(policy, Vec::new());
        let store_dir = tempfile::tempdir().expect("store");
        let store = SessionStore::new(store_dir.path()).expect("store");

        let error =
            match resolve_diff_workspace_scope(&request(), None, &store, &registry, "test_diff") {
                Ok(_) => panic!("scope must be explicit"),
                Err(error) => error,
            };
        assert_eq!(error.code, "workspace.scope_required");
    }
}
