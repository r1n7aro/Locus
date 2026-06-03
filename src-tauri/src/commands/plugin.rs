use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use walkdir::WalkDir;

use crate::agent::definition::AgentDefRegistry;
use crate::error::AppError;
use crate::plugin::{
    install_plugin_from_path_sync, list_installed_plugin_summaries, normalize_plugin_id,
    uninstall_plugin_sync, InstalledPluginSummary, LocusPluginProjectDependency,
    PluginInstallScope, PLUGIN_MANIFEST_FILE_NAME,
};
use crate::workspace::Workspace;
use crate::{AgentDefRegistryState, AppAgentDir};

pub const PLUGINS_CHANGED_EVENT: &str = "plugins-changed";

fn project_agent_dir(working_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(working_dir)
        .join("Locus")
        .join("agent")
}

pub(crate) async fn reload_agent_registry(
    registry: &AgentDefRegistryState,
    app_agent_dir: &AppAgentDir,
    working_dir: &str,
) {
    let project_agent_dir = project_agent_dir(working_dir);
    let project_agent_opt = project_agent_dir
        .is_dir()
        .then_some(project_agent_dir.as_path());
    let next = AgentDefRegistry::load_with_plugins(
        app_agent_dir.0.as_deref(),
        project_agent_opt,
        &crate::plugin::installed_agent_sources(working_dir),
    );
    *registry.0.write().await = next;
}

fn emit_plugins_changed(app_handle: &AppHandle, working_dir: &str, source: &str) {
    if let Err(error) = app_handle.emit(PLUGINS_CHANGED_EVENT, ()) {
        eprintln!("[Locus] failed to emit plugins changed event: {}", error);
    }
    crate::view::emit_view_tree_changed(app_handle);
    super::knowledge::emit_knowledge_changed(app_handle, working_dir, source);
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportRequest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub file_path: String,
    #[serde(default)]
    pub skill_package_ids: Vec<String>,
    #[serde(default)]
    pub view_ids: Vec<String>,
    #[serde(default)]
    pub project_dependencies: Vec<LocusPluginProjectDependency>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportResult {
    pub id: String,
    pub path: String,
    pub skill_count: usize,
    pub view_count: usize,
    pub file_count: usize,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginExportComponent {
    id: String,
    path: String,
}

fn normalize_export_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Plugin export path is required".to_string());
    }
    let mut path = PathBuf::from(trimmed);
    let has_zip_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if !has_zip_extension {
        path.set_extension("zip");
    }
    Ok(path)
}

fn normalize_project_dependencies(
    dependencies: Vec<LocusPluginProjectDependency>,
) -> Vec<LocusPluginProjectDependency> {
    dependencies
        .into_iter()
        .filter_map(|mut dependency| {
            dependency.kind = dependency.kind.trim().to_string();
            if dependency.kind.is_empty() {
                dependency.kind = "custom".to_string();
            }
            dependency.name = dependency.name.trim().to_string();
            if dependency.name.is_empty() {
                return None;
            }
            dependency.version = dependency
                .version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            dependency.notes = dependency
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            Some(dependency)
        })
        .collect()
}

fn unique_ids(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn component_dir_name(value: &str) -> Result<String, String> {
    normalize_plugin_id(value)
}

fn zip_plugin_root(root: &Path, output_path: &Path) -> Result<usize, String> {
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    let output = fs::File::create(output_path)
        .map_err(|e| format!("Failed to create {}: {}", output_path.display(), e))?;
    let mut archive = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut paths = Vec::new();
    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry = entry.map_err(|e| format!("Failed to scan plugin export files: {}", e))?;
        paths.push(entry.path().to_path_buf());
    }
    paths.sort();

    let mut file_count = 0usize;
    for path in paths {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to export symlinked plugin entry: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "Unsupported plugin export entry: {}",
                path.display()
            ));
        }
        let rel_path = path
            .strip_prefix(root)
            .map_err(|e| format!("Failed to resolve plugin export path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        archive
            .start_file(rel_path.clone(), options)
            .map_err(|e| format!("Failed to write plugin archive entry '{}': {}", rel_path, e))?;
        let mut input = fs::File::open(&path)
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        std::io::copy(&mut input, &mut archive)
            .map_err(|e| format!("Failed to write archive data for {}: {}", path.display(), e))?;
        file_count += 1;
    }
    archive
        .finish()
        .map_err(|e| format!("Failed to finish plugin archive: {}", e))?;
    Ok(file_count)
}

pub fn export_plugin_archive_sync(
    working_dir: &str,
    request: PluginExportRequest,
) -> Result<PluginExportResult, String> {
    let plugin_id = normalize_plugin_id(&request.id)?;
    let plugin_name = request.name.trim();
    let plugin_name = if plugin_name.is_empty() {
        plugin_id.clone()
    } else {
        plugin_name.to_string()
    };
    let plugin_version = request.version.trim();
    let plugin_version = if plugin_version.is_empty() {
        "0.1.0".to_string()
    } else {
        plugin_version.to_string()
    };
    let output_path = normalize_export_path(&request.file_path)?;
    let skill_ids = unique_ids(request.skill_package_ids);
    let view_ids = unique_ids(request.view_ids);
    if skill_ids.is_empty() && view_ids.is_empty() {
        return Err("Select at least one Skill package or View to export.".to_string());
    }
    let project_dependencies = normalize_project_dependencies(request.project_dependencies);
    let allow_project_dependencies = !project_dependencies.is_empty();

    let staging_root =
        std::env::temp_dir().join(format!("locus-plugin-export-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create plugin export staging directory: {}", e))?;

    let export_result = (|| -> Result<PluginExportResult, String> {
        let mut skill_components = Vec::new();
        let mut view_components = Vec::new();
        let mut copied_file_count = 0usize;

        for package_id in skill_ids {
            let dir_name = component_dir_name(&package_id)?;
            let rel_path = format!("skills/{}", dir_name);
            let target_root = staging_root.join(&rel_path);
            let copied = super::skill::copy_skill_package_for_plugin_sync(
                working_dir,
                &package_id,
                &target_root,
                allow_project_dependencies,
            )?;
            copied_file_count += copied.file_count;
            skill_components.push(PluginExportComponent {
                id: copied.id,
                path: rel_path,
            });
        }

        for view_id in view_ids {
            let dir_name = component_dir_name(&view_id)?;
            let rel_path = format!("views/{}", dir_name);
            let target_root = staging_root.join(&rel_path);
            let copied = crate::view::copy_view_package_for_plugin_sync(
                working_dir,
                &view_id,
                &target_root,
            )?;
            copied_file_count += copied.file_count;
            view_components.push(PluginExportComponent {
                id: copied.id,
                path: rel_path,
            });
        }

        let project_independent = project_dependencies.is_empty();
        let skill_count = skill_components.len();
        let view_count = view_components.len();
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": &plugin_id,
            "name": &plugin_name,
            "version": &plugin_version,
            "compatibility": {
                "projectIndependent": project_independent
            },
            "dependencies": {
                "project": &project_dependencies
            },
            "components": {
                "agents": [],
                "skills": &skill_components,
                "views": &view_components
            }
        });
        let manifest_path = staging_root.join(PLUGIN_MANIFEST_FILE_NAME);
        let manifest_text = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Failed to serialize plugin manifest: {}", e))?;
        fs::write(&manifest_path, manifest_text)
            .map_err(|e| format!("Failed to write {}: {}", manifest_path.display(), e))?;

        let file_count = zip_plugin_root(&staging_root, &output_path)?;
        let byte_size = fs::metadata(&output_path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        Ok(PluginExportResult {
            id: plugin_id,
            path: output_path.display().to_string().replace('\\', "/"),
            skill_count,
            view_count,
            file_count: file_count.max(copied_file_count.saturating_add(1)),
            byte_size,
        })
    })();

    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    export_result
}

#[tauri::command]
pub async fn plugin_list_installed(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<InstalledPluginSummary>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    Ok(list_installed_plugin_summaries(&working_dir))
}

#[tauri::command]
pub async fn plugin_install_from_path(
    source_path: String,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<InstalledPluginSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let summary =
        install_plugin_from_path_sync(&working_dir, &source_path, scope).map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_install");
    Ok(summary)
}

#[tauri::command]
pub async fn plugin_uninstall(
    plugin_id: String,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let removed = uninstall_plugin_sync(&working_dir, &plugin_id, scope).map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_uninstall");
    Ok(removed)
}

#[tauri::command]
pub async fn plugin_export(
    request: PluginExportRequest,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<PluginExportResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    export_plugin_archive_sync(&working_dir, request).map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::TempDir;

    #[test]
    fn export_plugin_archive_writes_project_dependency_metadata() {
        let workspace = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        crate::view::create_view_sync(
            &workspace.path().to_string_lossy(),
            crate::view::ViewCreateRequest {
                id: "asset-inspector".to_string(),
                package_name: None,
                name: Some("Asset Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
                display_path: None,
            },
        )
        .expect("create view");

        let output_path = output_dir.path().join("asset-tools.zip");
        let result = export_plugin_archive_sync(
            &workspace.path().to_string_lossy(),
            PluginExportRequest {
                id: "com.example.asset-tools".to_string(),
                name: "Asset Tools".to_string(),
                version: "0.1.0".to_string(),
                file_path: output_path.to_string_lossy().to_string(),
                skill_package_ids: Vec::new(),
                view_ids: vec!["asset-inspector".to_string()],
                project_dependencies: vec![LocusPluginProjectDependency {
                    kind: "unityPackage".to_string(),
                    name: "com.example.runtime".to_string(),
                    version: Some("1.2.3".to_string()),
                    notes: Some("Runtime scripts required by the View bindings.".to_string()),
                }],
            },
        )
        .expect("export plugin");

        assert_eq!(result.id, "com.example.asset-tools");
        assert_eq!(result.view_count, 1);
        assert!(output_path.is_file());

        let file = fs::File::open(output_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut manifest_entry = archive.by_name(PLUGIN_MANIFEST_FILE_NAME).unwrap();
        let mut manifest_text = String::new();
        manifest_entry.read_to_string(&mut manifest_text).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["compatibility"]["projectIndependent"], false);
        assert_eq!(
            manifest["dependencies"]["project"][0]["name"],
            "com.example.runtime"
        );
        assert_eq!(
            manifest["components"]["views"][0]["path"],
            "views/asset-inspector"
        );
    }
}
