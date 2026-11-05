//! The write path shared by the settings UI, the agent (`config_set`) and
//! the per-setting legacy commands: validate and persist the batch through
//! the store (its listener updates in-process mirrors and broadcasts
//! `settings-changed`), then run the async effects that need the app and
//! live workspaces. A failed effect reverts the batch.

use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::definitions::DEFINITIONS;
use super::schema::{Key, Scope, SettingValue};
use super::store::{index_of, SettingChange, SettingsError};
use crate::error::AppError;
use crate::workspace_service::{CheckoutId, ResolvedWorkspaceScope};

/// Where a write comes from and which workspace it concerns.
pub struct WriteContext {
    pub app: AppHandle,
    /// Root of the workspace the change was requested from (focused settings
    /// window, agent run). Effects use it as the warm-up target.
    pub workspace_root: Option<String>,
    pub workspace_checkout: Option<CheckoutId>,
    /// A checkout whose per-project application the caller performs and
    /// reports itself; effects skip it.
    pub reporting_root: Option<String>,
}

impl WriteContext {
    pub fn new(app: AppHandle, workspace: Option<&ResolvedWorkspaceScope>) -> Self {
        Self {
            app,
            workspace_root: workspace
                .map(|scope| scope.runtime().root().to_string_lossy().to_string()),
            workspace_checkout: workspace.map(|scope| scope.runtime().checkout_id().clone()),
            reporting_root: None,
        }
    }

    pub fn reporting_root(mut self, root: String) -> Self {
        self.reporting_root = Some(root);
        self
    }
}

/// Serializes writes so persistence and effects of one batch never
/// interleave with another batch.
fn write_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Resolves `(key, value)` pairs to definition indices.
pub fn resolve_keys(changes: &[(String, Value)]) -> Result<Vec<(usize, Value)>, SettingsError> {
    changes
        .iter()
        .map(|(key, value)| {
            index_of(key)
                .map(|index| (index, value.clone()))
                .ok_or_else(|| SettingsError::unknown_key(key))
        })
        .collect()
}

pub async fn write_one<T: SettingValue>(
    context: &WriteContext,
    key: Key<T>,
    value: T,
) -> Result<SettingChange, AppError> {
    write(context, &[(key.index(), value.to_setting())])
        .await
        .map(|mut changes| changes.remove(0))
}

pub async fn write(
    context: &WriteContext,
    requested: &[(usize, Value)],
) -> Result<Vec<SettingChange>, AppError> {
    let _write = write_lock().lock().await;
    let mut app_changes = Vec::new();
    let mut project_changes = Vec::new();
    for (index, value) in requested {
        let def = DEFINITIONS
            .get(*index)
            .ok_or_else(|| SettingsError::unknown_key(&index.to_string()))?;
        match def.scope {
            Scope::App => app_changes.push((*index, value.clone())),
            Scope::Project => project_changes.push((*index, value.clone())),
        }
    }

    let mut applied = Vec::new();
    if !project_changes.is_empty() {
        let root = context.workspace_root.as_deref().ok_or_else(|| {
            AppError::new(
                "settings.project_scope_required",
                "Project settings need a workspace.",
            )
        })?;
        let project_applied = super::project::write(root, &project_changes)?;
        super::effects::apply_project(context, &project_applied);
        applied.extend(project_applied);
    }

    let config = context.app.state::<std::sync::Arc<crate::config::AppConfig>>();
    let store = config.settings().clone();
    let app_applied = store.set_values(&app_changes)?;
    for (position, change) in app_applied.iter().enumerate() {
        if change.old == change.new {
            continue;
        }
        if let Err(message) = super::effects::apply(context, change).await {
            revert(context, &store, &app_applied[..=position]).await;
            return Err(AppError::new("settings.apply_failed", message)
                .detail(change.key.to_string()));
        }
    }
    super::effects::apply_groups(&context.app, &app_applied).await;
    applied.extend(app_applied);
    Ok(applied)
}

/// Group effects for changes made outside `write` (reset to defaults).
pub async fn apply_group_effects(app: &AppHandle, changes: &[SettingChange]) {
    super::effects::apply_groups(app, changes).await;
}

/// Restores the old values of `changes`; the last entry is the one whose
/// effect failed (and rolled itself back), so only earlier effects are undone.
async fn revert(
    context: &WriteContext,
    store: &super::store::SettingsStore,
    changes: &[SettingChange],
) {
    let restore = changes
        .iter()
        .filter_map(|change| index_of(change.key).map(|index| (index, change.old.clone())))
        .collect::<Vec<_>>();
    if let Err(error) = store.set_values(&restore) {
        eprintln!("[Locus] failed to revert settings after a failed change: {error}");
    }
    let Some((_, earlier)) = changes.split_last() else {
        return;
    };
    for change in earlier.iter().rev() {
        if change.old == change.new {
            continue;
        }
        let undo = SettingChange {
            key: change.key,
            old: change.new.clone(),
            new: change.old.clone(),
        };
        if let Err(error) = super::effects::apply(context, &undo).await {
            eprintln!(
                "[Locus] failed to undo the effect of '{}' after a failed change: {error}",
                change.key
            );
        }
    }
}
