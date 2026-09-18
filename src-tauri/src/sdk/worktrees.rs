//! SDK adapters use the same journal, registration and retirement as the UI.
use super::*;
use crate::workspace_service::{worktrees as store, CheckoutId, ProjectRegistry, WorkspaceRef};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    workspace_ref: WorkspaceRef,
    execution_delegation: Option<String>,
    worktree: Option<WorkspaceRef>,
    checkout_id: Option<String>,
    destination: Option<String>,
    branch: Option<String>,
    start_ref: Option<String>,
    #[serde(default)]
    include_dirty: bool,
    target_root: Option<String>,
    pool_root: Option<String>,
    commit: Option<String>,
    max_slots: Option<usize>,
    assignment_id: Option<String>,
}

fn required(value: Option<String>, name: &str) -> Result<String, String> {
    value
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("{name} is required"))
}

fn descriptor(registry: &ProjectRegistry, record: store::ManagedWorktree) -> Result<Value, String> {
    let id = CheckoutId::new(&record.checkout_id).map_err(|e| e.to_string())?;
    let generation = registry
        .runtime(&id)
        .filter(|runtime| runtime.materialization_epoch() == record.materialization_epoch)
        .map(|runtime| runtime.generation());
    let reference = WorkspaceRef::new(id, generation)
        .with_materialization_epoch(Some(record.materialization_epoch));
    let mut value = serde_json::to_value(record).map_err(|e| e.to_string())?;
    value["workspaceRef"] = serde_json::to_value(reference).map_err(|e| e.to_string())?;
    Ok(value)
}

fn target_epoch(reference: &WorkspaceRef) -> Result<u64, String> {
    reference.expected_materialization_epoch.ok_or_else(||
        "Worktree retirement requires its explicit materialization epoch; query worktrees.get first".into())
}

pub(super) async fn dispatch(app: &AppHandle, action: &str, value: Value) -> Result<Value, String> {
    use crate::commands;
    let params: Params = parse_params(value)?;
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "worktrees")?;
    let source = scope.runtime().root().to_string_lossy().into_owned();
    let registry = app.state::<Arc<ProjectRegistry>>();
    let sessions = app.state::<Arc<SessionStore>>();
    match action {
        "list" => {
            let records = commands::list_managed_worktrees(source, registry.clone()).await?;
            records
                .into_iter()
                .map(|row| descriptor(&registry, row))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        "get" => {
            let id = required(params.checkout_id, "checkoutId")?;
            let record = commands::list_managed_worktrees(source, registry.clone())
                .await?
                .into_iter()
                .find(|row| row.checkout_id == id)
                .ok_or("Checkout is outside the source project/repository")?;
            if record.lifecycle == "active" {
                // An explicit get can reopen the runtime; it never accepts a stale handle.
                commands::register_record(&registry, &sessions, &record)?;
            }
            descriptor(&registry, record)
        }
        "discover" => serde_json::to_value(commands::discover_worktrees(source, registry).await?)
            .map_err(|e| e.to_string()),
        "operations" => {
            serde_json::to_value(commands::get_worktree_operations(source, registry).await?)
                .map_err(|e| e.to_string())
        }
        "create" | "import" | "acquire" => {
            // Ordinary materialization reads an immutable commit and is already
            // coordinated by the worktree journal/slot lease. Only an undo-enabled
            // dirty snapshot participates in the caller's existing write gate.
            let _guard = if action == "create"
                && params.include_dirty
                && app
                    .state::<Arc<crate::config::AppConfig>>()
                    .session_undo_enabled()
            {
                Some(
                    crate::merge_jobs::coordination::acquire(
                        app,
                        scope.runtime(),
                        "worktrees.create",
                        params.execution_delegation.as_deref(),
                    )
                    .await?,
                )
            } else {
                None
            };
            if action == "create" {
                let record = commands::create_worktree(
                    store::CreateWorktreeRequest {
                        source_root: source,
                        destination: required(params.destination, "destination")?,
                        branch: required(params.branch, "branch")?,
                        start_ref: params.start_ref,
                        include_dirty: params.include_dirty,
                    },
                    registry.clone(),
                    sessions,
                )
                .await?;
                return descriptor(&registry, record);
            }
            if action == "import" {
                let record = commands::import_worktree(
                    source,
                    required(params.target_root, "targetRoot")?,
                    registry.clone(),
                    sessions,
                )
                .await?;
                return descriptor(&registry, record);
            }
            let acquired = commands::acquire_unity_project_slot(
                crate::workspace_service::pool::AcquirePoolRequest {
                    source_root: source,
                    pool_root: required(params.pool_root, "poolRoot")?,
                    commit: required(params.commit, "commit")?,
                    branch: params.branch,
                    max_slots: params
                        .max_slots
                        .filter(|v| *v > 0)
                        .ok_or("maxSlots must be positive")?,
                    assignment_id: params.assignment_id,
                },
                registry.clone(),
                sessions,
                Some(true),
            )
            .await?;
            Ok(
                json!({"worktree": descriptor(&registry, acquired.worktree)?,
                "reused": acquired.reused, "preservedLibrary": acquired.preserved_library}),
            )
        }
        "remove" | "release" => {
            let target = params.worktree.ok_or("worktree reference is required")?;
            let epoch = target_epoch(&target)?;
            if target.checkout_id == *scope.runtime().checkout_id() {
                return Err(
                    "Cannot retire the SDK's source checkout; use a sibling as workspace_ref"
                        .into(),
                );
            }
            // Do not take a target RunningTask lease here: retirement must see no users.
            // Commands recheck repository ownership, epoch, dirty state and Editor closure.
            if let Some(generation) = target.expected_generation {
                if registry
                    .runtime(&target.checkout_id)
                    .is_some_and(|r| r.generation() != generation)
                {
                    return Err(
                        "Worktree runtime generation is stale; query worktrees.get first".into(),
                    );
                }
            }
            if action == "remove" {
                commands::remove_managed_worktree(
                    source,
                    target.checkout_id.to_string(),
                    epoch,
                    registry,
                )
                .await?;
                return Ok(Value::Null);
            }
            let record = commands::release_unity_project_slot(
                source,
                target.checkout_id.to_string(),
                required(params.assignment_id, "assignmentId")?,
                epoch,
                registry.clone(),
            )
            .await?;
            descriptor(&registry, record)
        }
        _ => Err(format!("Unknown worktrees action '{action}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retirement_requires_explicit_epoch_and_requests_reject_typos() {
        let reference = WorkspaceRef::new(CheckoutId::new("checkout-a").unwrap(), None);
        assert!(target_epoch(&reference).is_err());
        assert_eq!(
            target_epoch(&reference.with_materialization_epoch(Some(3))).unwrap(),
            3
        );
        assert!(serde_json::from_value::<Params>(json!({
            "workspaceRef": {"checkoutId":"source"}, "includeDity": true,
        }))
        .is_err());
    }
}
