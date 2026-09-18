//! Checkout-scoped CSV view operations for the Python SDK.
use super::*;
use crate::agent::workspace_execution_lock::{
    WorkspaceExecutionLockOwner, WorkspaceExecutionLockRequest,
};
use crate::csv_document::{self, CsvViewPatch};
use crate::workspace_service::WorkspaceRef;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    workspace_ref: WorkspaceRef,
    file_path: String,
    expected_revision: Option<String>,
    patch: Option<CsvViewPatch>,
    content: Option<String>,
    execution_delegation: Option<String>,
    session_id: Option<String>,
}

pub(super) async fn dispatch(app: &AppHandle, action: &str, value: Value) -> Result<Value, String> {
    let params: Params = parse_params(value)?;
    let mutates = match action {
        "read_view" | "read_workbook" if params.patch.is_none() && params.expected_revision.is_none() && params.content.is_none() => false,
        "patch_view" if params.content.is_none() && params.patch.is_some() && params.expected_revision.as_ref().is_some_and(|value| !value.is_empty()) => true,
        "save_workbook" if params.patch.is_some() && params.expected_revision.as_ref().is_some_and(|value| !value.is_empty()) => true,
        _ => return Err("Use csv.read_view(file_path) or csv.patch_view(file_path, patch, expected_revision=...).".into()),
    };
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "csv")?;
    let root = scope.runtime().root().to_path_buf();
    let (csv, relative) = crate::commands::asset::resolve_workspace_path(&root, &params.file_path)
        .map_err(|error| error.to_string())?;
    // Check the CSV extension even for read calls before touching companion paths.
    csv_document::view_path(&csv).map_err(|error| error.to_string())?;
    if mutates {
        if let Some(session_id) = params.session_id.as_deref() {
            if app
                .state::<Arc<SessionStore>>()
                .get_plan_mode_state(session_id)?
                .active
            {
                return Err("CSV view changes are unavailable in Plan mode.".into());
            }
        }
    }
    let guard = if mutates {
        Some(
            tokio::time::timeout(
                Duration::from_secs(30),
                crate::merge_jobs::coordination::acquire_workspace(
                    app,
                    scope.runtime(),
                    WorkspaceExecutionLockRequest::Exclusive,
                    WorkspaceExecutionLockOwner {
                        session_id: "python-sdk".into(),
                        run_id: format!("sdk-csv-{}", uuid::Uuid::new_v4()),
                        iteration: 0,
                        workspace: root.to_string_lossy().into_owned(),
                        tools: vec![format!("csv.{action}")],
                    },
                    params.execution_delegation.as_deref(),
                ),
            )
            .await
            .map_err(|_| "Workspace is busy; retry the CSV operation")??,
        )
    } else {
        None
    };
    let app_knowledge_dir = app.state::<crate::commands::AppKnowledgeDir>().0.clone();
    let reads_workbook = action == "read_workbook";
    let app = app.clone();
    crate::merge_jobs::coordination::run_blocking(guard, move || {
        // The blocking worker owns the runtime lease and write gate through
        // persistence and notification, even if its HTTP waiter is cancelled.
        let mut result = if mutates {
            csv_document::ensure_write_allowed(
                &root,
                &csv,
                app_knowledge_dir.as_ref().as_ref(),
                true,
            )?;
            csv_document::save_workbook(
                &csv,
                params.expected_revision.as_deref().unwrap(),
                params.patch.unwrap(),
                params.content.clone(),
            )?
        } else if reads_workbook {
            csv_document::read_workbook(&csv)?
        } else {
            csv_document::read_view(&csv)?
        };
        if mutates && Some(result.revision.as_str()) != params.expected_revision.as_deref() {
            use crate::workspace_changes::{WorkspaceChangeKind, WorkspaceChangeSource};
            let hub = crate::workspace_changes::hub_for_workspace(&root);
            let mut paths = vec![format!("{relative}.view")];
            if params.content.is_some() {
                paths.push(relative.clone());
            }
            for path in paths {
                if let Some(change) = hub.observe(
                    &path,
                    WorkspaceChangeKind::Upsert,
                    WorkspaceChangeSource::LocusWrite,
                ) {
                    crate::workspace_service::event::emit_for_workspace_scope(
                        &app,
                        &crate::workspace_service::event::WorkspaceEventScope::for_runtime(
                            scope.runtime(),
                        ),
                        crate::workspace_changes::WORKSPACE_FILE_CHANGED_EVENT,
                        change,
                    );
                }
            }
        }
        result.file_path = relative;
        serde_json::to_value(result)
            .map_err(|error| crate::error::AppError::new("csv.serialize_failed", error.to_string()))
    })
    .await?
    .map_err(|error| error.to_string())
}
