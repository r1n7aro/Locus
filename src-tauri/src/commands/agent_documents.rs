use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use super::knowledge::{ensure_workspace_rule_path, resolve_knowledge_workspace_scope};
use crate::agent::definition::canonical_agent_id;
use crate::error::AppError;
use crate::workspace_definition_registry::WorkspaceDefinitionRegistry;
use crate::workspace_service::{ProjectRegistry, WorkspaceRef};
use crate::workspace_tool_registry::WorkspaceToolRegistry;

static DOCUMENT_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentDocumentKind {
    Soul,
    Env,
    Tool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDocument {
    content: String,
    path: String,
    revision: Option<String>,
}

fn validate_segment(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains("..")
        || value.ends_with(['.', ' '])
        || value
            .chars()
            .any(|ch| ch.is_control() || "\\/:*?\"<>|".contains(ch))
    {
        return Err("Invalid Agent document name".into());
    }
    Ok(())
}

fn document_path(
    root: &Path,
    agent_id: &str,
    kind: AgentDocumentKind,
    name: &str,
) -> Result<PathBuf, String> {
    validate_segment(agent_id)?;
    let dir = root.join("Locus").join("agent").join(agent_id);
    let path = match kind {
        AgentDocumentKind::Soul => dir.join("soul.md"),
        AgentDocumentKind::Env => dir.join("env.md"),
        AgentDocumentKind::Tool => {
            validate_segment(name)?;
            dir.join("tools").join(format!("{name}.json"))
        }
    };
    ensure_workspace_rule_path(&root.to_string_lossy(), &path)?;
    Ok(path)
}

fn read_revision(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn write_document(
    path: &Path,
    kind: AgentDocumentKind,
    content: &str,
    expected_revision: Option<&str>,
) -> Result<String, String> {
    let _guard = DOCUMENT_WRITE_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
    let current = read_revision(path)?;
    if current.as_deref() != expected_revision {
        return Err("Agent document changed on disk. Reload it before saving.".into());
    }
    let raw = if matches!(kind, AgentDocumentKind::Tool) {
        let mut overlay = match current.as_deref() {
            Some(raw) => {
                serde_json::from_str::<serde_json::Value>(raw).map_err(|error| error.to_string())?
            }
            None => serde_json::json!({}),
        };
        let object = overlay
            .as_object_mut()
            .ok_or("Invalid Agent tool description override")?;
        object.insert(
            "description".into(),
            serde_json::Value::String(content.into()),
        );
        // Preserve existing parameter descriptions; executable schemas remain owned by tools.
        serde_json::to_string_pretty(&overlay).map_err(|error| error.to_string())?
    } else {
        content.to_string()
    };
    std::fs::create_dir_all(path.parent().ok_or("Invalid Agent document path")?)
        .map_err(|error| error.to_string())?;
    crate::config::atomic_write_config(path, raw.as_bytes())?;
    Ok(raw)
}

#[tauri::command]
pub async fn read_workspace_agent_document(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    kind: AgentDocumentKind,
    name: String,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    workspace_tools: State<'_, Arc<WorkspaceToolRegistry>>,
    registry: State<'_, Arc<ProjectRegistry>>,
) -> Result<AgentDocument, AppError> {
    let scope = resolve_knowledge_workspace_scope(registry.inner().as_ref(), &workspace_ref)?;
    let agent_id = canonical_agent_id(&agent_id);
    let path = document_path(scope.runtime().root(), agent_id, kind, &name)?;
    let revision = read_revision(&path)?;
    // Rebuild from disk so opening/reloading a document also sees external edits.
    definitions.invalidate_checkout(scope.runtime().checkout_id())?;
    let agents = definitions.snapshot(scope.runtime()).await?;
    let def = agents
        .get(agent_id)
        .ok_or_else(|| format!("Agent '{agent_id}' not found"))?;
    let content = match kind {
        AgentDocumentKind::Soul => def.system_prompt.clone(),
        AgentDocumentKind::Env => def.env_template.clone(),
        AgentDocumentKind::Tool => {
            let tools = workspace_tools.snapshot(scope.runtime(), &agents).await?;
            let mut tool = tools.resolve_api_tool(&name).or_else(|| {
                crate::mcp::manager::resolve_wire_tool(&name).map(|tool| serde_json::json!({
                    "function": { "name": tool.wire_name, "description": tool.description, "parameters": tool.input_schema }
                }))
            }).ok_or_else(|| format!("Tool '{name}' not found"))?;
            def.apply_tool_description_override(&name, &mut tool);
            tool["function"]["description"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        }
    };
    Ok(AgentDocument {
        content,
        path: path.to_string_lossy().into_owned(),
        revision,
    })
}

#[tauri::command]
pub async fn save_workspace_agent_document(
    workspace_ref: WorkspaceRef,
    agent_id: String,
    kind: AgentDocumentKind,
    name: String,
    content: String,
    expected_revision: Option<String>,
    definitions: State<'_, Arc<WorkspaceDefinitionRegistry>>,
    registry: State<'_, Arc<ProjectRegistry>>,
    app: tauri::AppHandle,
) -> Result<AgentDocument, AppError> {
    let scope = resolve_knowledge_workspace_scope(registry.inner().as_ref(), &workspace_ref)?;
    let agent_id = canonical_agent_id(&agent_id);
    let agents = definitions.snapshot(scope.runtime()).await?;
    if agents.get(agent_id).is_none() {
        return Err(format!("Agent '{agent_id}' not found").into());
    }
    let path = document_path(scope.runtime().root(), agent_id, kind, &name)?;
    let revision = write_document(&path, kind, &content, expected_revision.as_deref())?;
    definitions.invalidate_checkout(scope.runtime().checkout_id())?;
    let _ = app.emit("agents-changed", ());
    Ok(AgentDocument {
        content,
        path: path.to_string_lossy().into_owned(),
        revision: Some(revision),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_documents_preserve_base_and_reject_stale_saves() {
        let root = tempfile::tempdir().unwrap();
        let path = document_path(root.path(), "unity", AgentDocumentKind::Soul, "").unwrap();
        assert_eq!(read_revision(&path).unwrap(), None);
        let revision =
            write_document(&path, AgentDocumentKind::Soul, "project soul", None).unwrap();
        assert!(write_document(&path, AgentDocumentKind::Soul, "stale", None).is_err());
        write_document(&path, AgentDocumentKind::Soul, "updated", Some(&revision)).unwrap();
        let registry = crate::agent::definition::AgentDefRegistry::load(
            None,
            Some(&root.path().join("Locus/agent")),
        );
        assert!(registry.list_all().is_empty()); // An overlay alone never creates an Agent.
        assert!(document_path(root.path(), "../escape", AgentDocumentKind::Soul, "").is_err());
        assert!(document_path(root.path(), "unity", AgentDocumentKind::Tool, "../escape").is_err());
    }

    #[test]
    fn description_edit_preserves_parameter_overrides() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tool.json");
        let original = r#"{"description":"old","parameters":{"properties":{"path":{"description":"path help"}}}}"#;
        std::fs::write(&path, original).unwrap();
        let result = write_document(&path, AgentDocumentKind::Tool, "new", Some(original)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["description"], "new");
        assert_eq!(
            value["parameters"]["properties"]["path"]["description"],
            "path help"
        );
    }

    #[test]
    fn project_overrides_are_effective_without_changing_other_projects_or_builtins() {
        use crate::agent::definition::AgentDefRegistry;
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app/agent");
        let builtin = app.join("unity");
        std::fs::create_dir_all(builtin.join("tools")).unwrap();
        std::fs::write(
            builtin.join("config.json"),
            r#"{"name":"Unity","default":true,"tools":["read"]}"#,
        )
        .unwrap();
        std::fs::write(builtin.join("soul.md"), "builtin soul").unwrap();
        let original_tool = r#"{"description":"builtin help","parameters":{"properties":{"path":{"description":"builtin parameter help"}}}}"#;
        std::fs::write(builtin.join("tools/read.json"), original_tool).unwrap();
        let project_a = root.path().join("project-a");
        let project_b = root.path().join("project-b");
        std::fs::create_dir_all(&project_a).unwrap();
        std::fs::create_dir_all(&project_b).unwrap();
        let soul = document_path(&project_a, "unity", AgentDocumentKind::Soul, "").unwrap();
        write_document(&soul, AgentDocumentKind::Soul, "project soul", None).unwrap();
        let tool = document_path(&project_a, "unity", AgentDocumentKind::Tool, "read").unwrap();
        write_document(&tool, AgentDocumentKind::Tool, "project help", None).unwrap();
        let a = AgentDefRegistry::load(Some(&app), Some(&project_a.join("Locus/agent")));
        let b = AgentDefRegistry::load(Some(&app), Some(&project_b.join("Locus/agent")));
        assert_eq!(a.get("unity").unwrap().system_prompt, "project soul");
        assert_eq!(b.get("unity").unwrap().system_prompt, "builtin soul");
        let mut schema = serde_json::json!({"function":{"description":"shared", "parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}});
        a.get("unity")
            .unwrap()
            .apply_tool_description_override("read", &mut schema);
        assert_eq!(schema["function"]["description"], "project help");
        assert_eq!(
            schema["function"]["parameters"]["properties"]["path"]["description"],
            "builtin parameter help"
        );
        assert_eq!(
            schema["function"]["parameters"]["properties"]["path"]["type"],
            "string"
        );
        assert_eq!(
            schema["function"]["parameters"]["required"],
            serde_json::json!(["path"])
        );
        assert_eq!(
            std::fs::read_to_string(builtin.join("soul.md")).unwrap(),
            "builtin soul"
        );
        assert_eq!(
            std::fs::read_to_string(builtin.join("tools/read.json")).unwrap(),
            original_tool
        );
    }
}
