//! Generic settings commands. New UI code reads and writes every setting
//! through these instead of per-setting commands.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, State};

use super::apply::{resolve_keys, write, WriteContext};
use super::definitions::DEFINITIONS;
use super::schema::{Scope, SettingDef};
use super::store::{index_of, SettingChange, SettingsError};
use crate::config::AppConfig;
use crate::error::AppError;
use crate::workspace_service::{ProjectRegistry, ResolvedWorkspaceScope, WorkspaceRef};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingSchemaEntry {
    pub key: &'static str,
    pub scope: Scope,
    pub sensitivity: super::schema::Sensitivity,
    pub experimental: bool,
    pub restart: bool,
    pub boot: bool,
    pub kind: Value,
    pub default: Value,
}

impl From<&SettingDef> for SettingSchemaEntry {
    fn from(def: &SettingDef) -> Self {
        Self {
            key: def.key,
            scope: def.scope,
            sensitivity: def.sensitivity,
            experimental: def.experimental,
            restart: def.restart,
            boot: def.boot,
            kind: def.kind.describe(),
            default: def.default_value(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingWrite {
    pub key: String,
    pub value: Value,
}

fn resolve_workspace(
    registry: &ProjectRegistry,
    workspace_ref: Option<&WorkspaceRef>,
) -> Result<Option<ResolvedWorkspaceScope>, AppError> {
    workspace_ref
        .map(|workspace_ref| {
            crate::commands::resolve_workspace_scope(registry, workspace_ref, "settings")
        })
        .transpose()
}

/// Effective values keyed by setting key. Project settings need
/// `workspace_root`; without it they report their defaults.
pub fn effective_values(
    config: &AppConfig,
    workspace_root: Option<&str>,
    keys: Option<&[String]>,
) -> Result<Map<String, Value>, SettingsError> {
    let indices = match keys {
        Some(keys) => keys
            .iter()
            .map(|key| index_of(key).ok_or_else(|| SettingsError::unknown_key(key)))
            .collect::<Result<Vec<_>, _>>()?,
        None => (0..DEFINITIONS.len()).collect(),
    };
    Ok(indices
        .into_iter()
        .map(|index| {
            let def = &DEFINITIONS[index];
            let value = match (def.scope, workspace_root) {
                (Scope::App, _) => config.settings().value_at(index),
                (Scope::Project, Some(root)) => super::project::value_at(root, index),
                (Scope::Project, None) => def.default_value(),
            };
            (def.key.to_string(), value)
        })
        .collect())
}

#[tauri::command]
pub fn settings_schema() -> Vec<SettingSchemaEntry> {
    DEFINITIONS.iter().map(SettingSchemaEntry::from).collect()
}

#[tauri::command]
pub fn settings_get(
    keys: Option<Vec<String>>,
    workspace_ref: Option<WorkspaceRef>,
    config: State<'_, Arc<AppConfig>>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Map<String, Value>, AppError> {
    let workspace = resolve_workspace(registry.inner(), workspace_ref.as_ref())?;
    let root = workspace
        .as_ref()
        .map(|scope| scope.runtime().root().to_string_lossy().to_string());
    effective_values(config.inner(), root.as_deref(), keys.as_deref()).map_err(AppError::from)
}

/// One-time import of the preferences older frontends kept in localStorage.
/// Only frontend (`boot`) settings the user has not set yet are filled;
/// returns the imported keys.
#[tauri::command]
pub fn settings_import_frontend_legacy(
    values: Map<String, Value>,
    config: State<'_, Arc<AppConfig>>,
) -> Result<Vec<String>, AppError> {
    let entries = values
        .into_iter()
        .filter_map(|(key, value)| {
            let index = index_of(&key)?;
            DEFINITIONS[index].boot.then_some((index, value))
        })
        .collect::<Vec<_>>();
    let changes = config.settings().import_missing(&entries)?;
    Ok(changes.into_iter().map(|change| change.key.to_string()).collect())
}

#[tauri::command]
pub async fn settings_set(
    changes: Vec<SettingWrite>,
    workspace_ref: Option<WorkspaceRef>,
    app: AppHandle,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<Vec<SettingChange>, AppError> {
    let workspace = resolve_workspace(registry.inner(), workspace_ref.as_ref())?;
    let requested = resolve_keys(
        &changes
            .into_iter()
            .map(|change| (change.key, change.value))
            .collect::<Vec<_>>(),
    )?;
    let context = WriteContext::new(app, workspace.as_ref());
    write(&context, &requested).await
}
