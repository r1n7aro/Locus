//! Remembers the user's last explicit Agent/model choice per project.
//!
//! The memory is personal, so it lives in the app config directory keyed by
//! project id instead of the shared `<workspace>/Locus/config.json`. Managed
//! worktrees share their project's entry.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::State;

use super::workspace::{persistent_config_dir, ModelDefaults};
use crate::error::AppError;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};

const MEMORY_FILE: &str = "workspace_selections.json";
const MAIN_MODEL_MIGRATION_MARKER: &str = ".main_model_selection_migrated";
const MAX_REMEMBERED_WORKSPACES: usize = 256;
const EFFORT_LEVELS: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSelectionMemory {
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub effort: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSelectionMemory {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub model_id: String,
    #[serde(default)]
    pub effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_mode: Option<bool>,
    /// The model each Agent last used in this project.
    #[serde(default)]
    pub agents: BTreeMap<String, AgentSelectionMemory>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SelectionMemoryFile {
    /// Most recent explicit Agent choice in any project.
    #[serde(default)]
    last_agent_id: String,
    #[serde(default)]
    workspaces: BTreeMap<String, WorkspaceSelectionMemory>,
}

/// Fields omitted from a patch keep their remembered value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSelectionPatch {
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub fast_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSelectionSnapshot {
    pub project_id: String,
    pub memory: WorkspaceSelectionMemory,
    pub last_agent_id: String,
    /// Explicit default from `<workspace>/Locus/config.json`, if any.
    pub workspace_default_agent_id: Option<String>,
}

fn memory_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_memory_file(dir: &Path) -> SelectionMemoryFile {
    std::fs::read_to_string(dir.join(MEMORY_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_memory_file(dir: &Path, file: &SelectionMemoryFile) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(file)
        .map_err(|error| format!("Failed to serialize workspace selections: {error}"))?;
    crate::config::atomic_write_config(&dir.join(MEMORY_FILE), &json)
}

fn normalize_agent_id(agent_id: &str) -> Option<String> {
    let canonical = crate::agent::definition::canonical_agent_id(agent_id.trim());
    (!canonical.is_empty() && !crate::agent::definition::is_removed_agent_id(canonical))
        .then(|| canonical.to_string())
}

fn normalize_effort(effort: &str) -> Option<String> {
    let effort = effort.trim();
    EFFORT_LEVELS.contains(&effort).then(|| effort.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn apply_patch(
    file: &mut SelectionMemoryFile,
    project_id: &str,
    patch: &WorkspaceSelectionPatch,
    updated_at_ms: u64,
) -> WorkspaceSelectionMemory {
    let entry = file.workspaces.entry(project_id.to_string()).or_default();
    if let Some(agent_id) = patch.agent_id.as_deref().and_then(normalize_agent_id) {
        entry.agent_id = agent_id.clone();
        file.last_agent_id = agent_id;
    }
    let model_id = patch
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty());
    if let Some(model_id) = model_id {
        entry.model_id = model_id.to_string();
    }
    if let Some(effort) = patch.effort.as_deref().and_then(normalize_effort) {
        entry.effort = effort;
    }
    if let Some(fast_mode) = patch.fast_mode {
        entry.fast_mode = Some(fast_mode);
    }
    // Only a patch that names the model describes the Agent's model. An
    // Agent-only patch must not attach the previous Agent's model to it.
    if model_id.is_some() && !entry.agent_id.is_empty() {
        entry.agents.insert(
            entry.agent_id.clone(),
            AgentSelectionMemory {
                model_id: entry.model_id.clone(),
                effort: entry.effort.clone(),
            },
        );
    }
    entry.updated_at_ms = updated_at_ms;
    let memory = entry.clone();
    prune_workspaces(file, project_id);
    memory
}

fn prune_workspaces(file: &mut SelectionMemoryFile, keep: &str) {
    while file.workspaces.len() > MAX_REMEMBERED_WORKSPACES {
        let Some(oldest) = file
            .workspaces
            .iter()
            .filter(|(project_id, _)| project_id.as_str() != keep)
            .min_by_key(|(_, memory)| memory.updated_at_ms)
            .map(|(project_id, _)| project_id.clone())
        else {
            return;
        };
        file.workspaces.remove(&oldest);
    }
}

fn snapshot(
    file: &SelectionMemoryFile,
    project_id: &str,
    workspace_default_agent_id: Option<String>,
) -> WorkspaceSelectionSnapshot {
    WorkspaceSelectionSnapshot {
        project_id: project_id.to_string(),
        memory: file.workspaces.get(project_id).cloned().unwrap_or_default(),
        last_agent_id: file.last_agent_id.clone(),
        workspace_default_agent_id,
    }
}

struct WorkspaceIdentity {
    project_id: String,
    root: String,
}

fn resolve_identity(
    workspace_registry: &ProjectRegistry,
    workspace_ref: &WorkspaceRef,
    operation: &'static str,
) -> Result<WorkspaceIdentity, AppError> {
    let scope =
        super::session::resolve_workspace_scope(workspace_registry, workspace_ref, operation)?;
    let runtime = scope.runtime();
    Ok(WorkspaceIdentity {
        project_id: runtime.project_id().as_str().to_string(),
        root: runtime.root().to_string_lossy().into_owned(),
    })
}

fn save_selection(
    project_id: &str,
    workspace_root: &str,
    patch: &WorkspaceSelectionPatch,
) -> Result<WorkspaceSelectionSnapshot, String> {
    let dir = persistent_config_dir()?;
    let file = {
        let _guard = memory_lock();
        let mut file = read_memory_file(&dir);
        apply_patch(&mut file, project_id, patch, now_ms());
        write_memory_file(&dir, &file)?;
        file
    };
    let memory = file.workspaces.get(project_id).cloned().unwrap_or_default();
    // Keep the global fallbacks in step for projects without their own memory.
    if patch
        .model_id
        .as_deref()
        .is_some_and(|model| !model.trim().is_empty())
    {
        super::workspace::store_last_model(&memory.model_id)?;
        if !memory.agent_id.is_empty() && normalize_effort(&memory.effort).is_some() {
            super::workspace::store_agent_model_preference(
                &memory.agent_id,
                &memory.model_id,
                &memory.effort,
            )?;
        }
    }
    Ok(snapshot(
        &file,
        project_id,
        crate::workspace::default_agent_id(workspace_root),
    ))
}

/// Making an Agent the workspace default is also the user's current choice.
pub(crate) fn remember_workspace_agent(
    project_id: &str,
    workspace_root: &str,
    agent_id: &str,
) -> Result<(), String> {
    save_selection(
        project_id,
        workspace_root,
        &WorkspaceSelectionPatch {
            agent_id: Some(agent_id.to_string()),
            ..WorkspaceSelectionPatch::default()
        },
    )
    .map(|_| ())
}

pub(crate) fn remembered_model_for_project(project_id: &str) -> Option<String> {
    let dir = persistent_config_dir().ok()?;
    let file = read_memory_file(&dir);
    let model = file.workspaces.get(project_id)?.model_id.trim().to_string();
    (!model.is_empty()).then_some(model)
}

/// Model for a request that names none, in the order a new chat uses: a
/// pinned main model, the project's remembered choice, then the most recent
/// choice anywhere.
pub(crate) async fn default_model_for_project(
    app: &tauri::AppHandle,
    project_id: Option<&str>,
) -> Option<String> {
    let nonempty = |model: String| {
        let model = model.trim().to_string();
        (!model.is_empty()).then_some(model)
    };
    if let Ok(defaults) = super::workspace::get_model_defaults(app.clone()).await {
        if let Some(model) = nonempty(defaults.main_model) {
            return Some(model);
        }
    }
    if let Some(model) = project_id.and_then(remembered_model_for_project) {
        return Some(model);
    }
    super::workspace::get_last_model(app.clone())
        .await
        .ok()
        .and_then(nonempty)
}

fn migrate_legacy_main_model_in(
    dir: &Path,
    defaults: &mut ModelDefaults,
    save: impl FnOnce(&ModelDefaults) -> Result<(), String>,
) -> Result<bool, String> {
    let marker = dir.join(MAIN_MODEL_MIGRATION_MARKER);
    if marker.exists() {
        return Ok(false);
    }
    let main_model = defaults.main_model.trim().to_string();
    let migrated = !main_model.is_empty();
    if migrated {
        std::fs::write(dir.join("last_model.txt"), &main_model)
            .map_err(|error| format!("Failed to seed last model: {error}"))?;
        defaults.main_model.clear();
        save(defaults)?;
    }
    std::fs::write(&marker, b"1")
        .map_err(|error| format!("Failed to record main model migration: {error}"))?;
    Ok(migrated)
}

/// Converts a legacy pinned main model into the remembered selection once.
pub(crate) fn migrate_legacy_main_model(defaults: &mut ModelDefaults) -> Result<bool, String> {
    let dir = persistent_config_dir()?;
    let _guard = memory_lock();
    migrate_legacy_main_model_in(&dir, defaults, super::workspace::write_model_defaults)
}

#[tauri::command]
pub async fn get_workspace_selection(
    workspace_ref: WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<WorkspaceSelectionSnapshot, AppError> {
    let identity = resolve_identity(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "getWorkspaceSelection",
    )?;
    let dir = persistent_config_dir().map_err(AppError::from)?;
    let file = read_memory_file(&dir);
    Ok(snapshot(
        &file,
        &identity.project_id,
        crate::workspace::default_agent_id(&identity.root),
    ))
}

#[tauri::command]
pub async fn save_workspace_selection(
    workspace_ref: WorkspaceRef,
    selection: WorkspaceSelectionPatch,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<WorkspaceSelectionSnapshot, AppError> {
    let identity = resolve_identity(
        workspace_registry.inner().as_ref(),
        &workspace_ref,
        "saveWorkspaceSelection",
    )?;
    save_selection(&identity.project_id, &identity.root, &selection).map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(
        agent: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> WorkspaceSelectionPatch {
        WorkspaceSelectionPatch {
            agent_id: agent.map(str::to_string),
            model_id: model.map(str::to_string),
            effort: effort.map(str::to_string),
            fast_mode: None,
        }
    }

    #[test]
    fn full_selection_records_workspace_agent_model_and_global_agent() {
        let mut file = SelectionMemoryFile::default();
        let memory = apply_patch(
            &mut file,
            "p1",
            &patch(Some("unity"), Some("m1"), Some("high")),
            5,
        );
        assert_eq!(memory.agent_id, "unity");
        assert_eq!(memory.model_id, "m1");
        assert_eq!(memory.effort, "high");
        assert_eq!(
            memory.agents["unity"],
            AgentSelectionMemory {
                model_id: "m1".into(),
                effort: "high".into()
            }
        );
        assert_eq!(file.last_agent_id, "unity");
        assert_eq!(memory.updated_at_ms, 5);
    }

    #[test]
    fn agent_only_patch_keeps_per_agent_models_untouched() {
        let mut file = SelectionMemoryFile::default();
        apply_patch(
            &mut file,
            "p1",
            &patch(Some("unity"), Some("m1"), Some("high")),
            1,
        );
        let memory = apply_patch(&mut file, "p1", &patch(Some("simple"), None, None), 2);
        assert_eq!(memory.agent_id, "simple");
        assert_eq!(memory.model_id, "m1");
        assert!(!memory.agents.contains_key("simple"));
        assert_eq!(file.last_agent_id, "simple");
    }

    #[test]
    fn projects_are_independent_and_invalid_values_are_ignored() {
        let mut file = SelectionMemoryFile::default();
        apply_patch(
            &mut file,
            "p1",
            &patch(Some("unity"), Some("m1"), Some("high")),
            1,
        );
        let other = apply_patch(
            &mut file,
            "p2",
            &patch(Some("dev"), Some("  "), Some("turbo")),
            2,
        );
        assert_eq!(other.agent_id, "");
        assert_eq!(other.model_id, "");
        assert_eq!(other.effort, "");
        assert_eq!(file.workspaces["p1"].model_id, "m1");
        assert_eq!(file.last_agent_id, "unity");
    }

    #[test]
    fn legacy_agent_ids_are_canonicalized() {
        let mut file = SelectionMemoryFile::default();
        let memory = apply_patch(
            &mut file,
            "p1",
            &patch(Some("wiki"), Some("m1"), Some("low")),
            1,
        );
        assert_eq!(
            memory.agent_id,
            crate::agent::definition::canonical_agent_id("wiki")
        );
    }

    #[test]
    fn pruning_drops_the_least_recent_other_project() {
        let mut file = SelectionMemoryFile::default();
        for index in 0..MAX_REMEMBERED_WORKSPACES {
            apply_patch(
                &mut file,
                &format!("p{index}"),
                &patch(None, Some("m"), None),
                index as u64 + 10,
            );
        }
        apply_patch(&mut file, "fresh", &patch(None, Some("m"), None), 0);
        assert_eq!(file.workspaces.len(), MAX_REMEMBERED_WORKSPACES);
        assert!(file.workspaces.contains_key("fresh"));
        assert!(!file.workspaces.contains_key("p0"));
    }

    #[test]
    fn legacy_main_model_seeds_last_model_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut defaults = ModelDefaults {
            main_model: "openai/gpt-5.5".into(),
            ..ModelDefaults::default()
        };
        let mut saved = None;
        assert!(migrate_legacy_main_model_in(dir.path(), &mut defaults, |value| {
            saved = Some(value.clone());
            Ok(())
        })
        .unwrap());
        assert_eq!(defaults.main_model, "");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("last_model.txt")).unwrap(),
            "openai/gpt-5.5"
        );
        assert_eq!(saved.unwrap().main_model, "");

        // A model pinned after the migration stays pinned.
        let mut pinned = ModelDefaults {
            main_model: "custom/a/b".into(),
            ..ModelDefaults::default()
        };
        assert!(!migrate_legacy_main_model_in(dir.path(), &mut pinned, |_| {
            panic!("a completed migration saves nothing")
        })
        .unwrap());
        assert_eq!(pinned.main_model, "custom/a/b");
    }

    #[test]
    fn fresh_install_only_records_the_migration() {
        let dir = tempfile::tempdir().unwrap();
        let mut defaults = ModelDefaults::default();
        assert!(!migrate_legacy_main_model_in(dir.path(), &mut defaults, |_| {
            panic!("an empty main model saves nothing")
        })
        .unwrap());
        assert!(dir.path().join(MAIN_MODEL_MIGRATION_MARKER).exists());
        assert!(!dir.path().join("last_model.txt").exists());
    }
}
