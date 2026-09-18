//! Checkout-scoped, read-only history discovery.
use super::*;
use crate::workspace_service::WorkspaceRef;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadParams {
    workspace_ref: WorkspaceRef,
    session_id: String,
    before_row_id: Option<i64>,
    #[serde(default = "default_read_limit")]
    limit: u32,
}

fn default_read_limit() -> u32 {
    50
}
fn default_search_limit() -> u32 {
    20
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SearchParams {
    workspace_ref: WorkspaceRef,
    query: String,
    #[serde(default)]
    archived: bool,
    session_id: Option<String>,
    #[serde(default = "default_search_limit")]
    limit: u32,
    cursor: Option<String>,
}

pub(super) fn read(app: &AppHandle, value: Value) -> Result<Value, String> {
    let params: ReadParams = parse_params(value)?;
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "sessions.read")?;
    let page = app.state::<Arc<SessionStore>>().read_session_history(
        scope.runtime().checkout_id().as_str(),
        params.session_id.trim(),
        params.before_row_id,
        params.limit,
    )?;
    serde_json::to_value(page).map_err(|e| e.to_string())
}

pub(super) async fn search(app: &AppHandle, value: Value) -> Result<Value, String> {
    static SEARCH_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
    let params: SearchParams = parse_params(value)?;
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "sessions.search")?;
    let store = app.state::<Arc<SessionStore>>().inner().clone();
    let permit = SEARCH_WORKERS.acquire().await.map_err(|e| e.to_string())?;
    let page = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        store.search_session_history(
            scope.runtime().checkout_id().as_str(),
            &params.query,
            params.archived,
            params.session_id.as_deref().map(str::trim),
            params.limit,
            params.cursor.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("History search worker failed: {e}"))??;
    serde_json::to_value(page).map_err(|e| e.to_string())
}
