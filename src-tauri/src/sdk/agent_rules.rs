//! Workspace rule management for installed Agents, shared with the Agent page.
use super::*;
use crate::agent::workspace_execution_lock::{
    WorkspaceExecutionLockOwner, WorkspaceExecutionLockRequest,
};
use crate::workspace_service::WorkspaceRef;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    workspace_ref: WorkspaceRef,
    agent_id: String,
    file_name: Option<String>,
    content: Option<String>,
    enabled: Option<bool>,
    execution_delegation: Option<String>,
}

enum Operation {
    List,
    Read(String),
    Save(String, String),
    SetEnabled(String, bool),
}

impl Operation {
    fn parse(action: &str, params: &Params) -> Result<Self, String> {
        let file_name = || {
            params
                .file_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| "fileName is required".to_string())
        };
        match action {
            "list"
                if params.file_name.is_none()
                    && params.content.is_none()
                    && params.enabled.is_none() =>
            {
                Ok(Self::List)
            }
            "read" if params.content.is_none() && params.enabled.is_none() => {
                Ok(Self::Read(file_name()?))
            }
            "save" if params.enabled.is_none() => Ok(Self::Save(
                file_name()?,
                params.content.clone().ok_or("content is required")?,
            )),
            "set_enabled" if params.content.is_none() => Ok(Self::SetEnabled(
                file_name()?,
                params.enabled.ok_or("enabled is required")?,
            )),
            _ => Err(format!(
                "Invalid Agent rules operation or arguments: {action}"
            )),
        }
    }

    fn mutates(&self) -> bool {
        matches!(self, Self::Save(..) | Self::SetEnabled(..))
    }

    fn execute(
        &self,
        app_agent_dir: &Option<std::path::PathBuf>,
        working_dir: &str,
        agent_id: &str,
    ) -> Result<Value, String> {
        match self {
            Self::List => {
                let rules: Vec<_> = crate::commands::collect_agent_rule_files(
                    app_agent_dir,
                    working_dir,
                    agent_id,
                    false,
                )?
                .into_iter()
                .map(|entry| entry.into_item())
                .collect();
                serde_json::to_value(rules).map_err(|e| e.to_string())
            }
            Self::Read(key) => {
                let entry = crate::commands::collect_agent_rule_files(
                    app_agent_dir,
                    working_dir,
                    agent_id,
                    false,
                )?
                .into_iter()
                .find(|entry| entry.key == *key)
                .ok_or_else(|| format!("Rule file not found: {key}"))?;
                std::fs::read_to_string(&entry.path)
                    .map(Value::String)
                    .map_err(|error| format!("Failed to read rule: {error}"))
            }
            Self::Save(file_name, content) => {
                serde_json::to_value(crate::commands::save_workspace_agent_rule(
                    app_agent_dir,
                    working_dir,
                    agent_id,
                    file_name,
                    content,
                )?)
                .map_err(|e| e.to_string())
            }
            Self::SetEnabled(key, enabled) => {
                serde_json::to_value(crate::commands::set_workspace_agent_rule_enabled(
                    app_agent_dir,
                    working_dir,
                    agent_id,
                    key,
                    *enabled,
                )?)
                .map_err(|e| e.to_string())
            }
        }
    }
}

pub(super) async fn dispatch(app: &AppHandle, action: &str, value: Value) -> Result<Value, String> {
    let params: Params = parse_params(value)?;
    let operation = Operation::parse(action, &params)?;
    let scope = resolve_sdk_workspace_scope(app, &params.workspace_ref, "agents.rules")?;
    let agent_id = canonical_agent_id(&params.agent_id);
    let definitions = app
        .state::<Arc<crate::workspace_definition_registry::WorkspaceDefinitionRegistry>>()
        .snapshot(scope.runtime().as_ref())
        .await?;
    if definitions.get(agent_id).is_none() {
        return Err(format!("Agent '{agent_id}' not found"));
    }
    let working_dir = scope.runtime().root().to_string_lossy().into_owned();
    // Reuse the Python call's write gate when invoked inside an Agent round.
    // Acquiring a second exclusive gate would deadlock with that outer call.
    let _guard = if operation.mutates() {
        Some(
            tokio::time::timeout(
                Duration::from_secs(30),
                crate::merge_jobs::coordination::acquire_workspace(
                    app,
                    scope.runtime(),
                    WorkspaceExecutionLockRequest::Exclusive,
                    WorkspaceExecutionLockOwner {
                        session_id: "python-sdk".into(),
                        run_id: format!("sdk-rules-{}", uuid::Uuid::new_v4()),
                        iteration: 0,
                        workspace: working_dir.clone(),
                        tools: vec![format!("agents.rules.{action}")],
                    },
                    params.execution_delegation.as_deref(),
                ),
            )
            .await
            .map_err(|_| "Workspace is busy; retry the rule operation")??,
        )
    } else {
        None
    };
    let app_agent_dir = app.state::<AppAgentDir>();
    let result = operation.execute(app_agent_dir.0.as_ref(), &working_dir, agent_id)?;
    if operation.mutates() {
        crate::commands::emit_agents_changed(app);
        crate::commands::emit_agents_changed_for_workspace(app, scope.runtime().as_ref());
    }
    Ok(result)
}

#[cfg(test)]
#[path = "agent_rules_tests.rs"]
mod tests;
