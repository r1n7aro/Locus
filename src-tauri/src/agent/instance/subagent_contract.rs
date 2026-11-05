//! Session-stable `subagent` tool definition. The description lists the
//! delegatable agents and their models, which follow Agent settings. A
//! conversation keeps the definition it first offered: changing the settings
//! must not rewrite an existing tool prefix (prompt caches, and Claude Code's
//! pinned tool catalog, which rejects any changed definition). Calls are
//! validated against the current settings, and failures list the current
//! choices.
use super::*;
use crate::session::store::SessionToolContract;

#[cfg(test)]
#[path = "subagent_contract_tests.rs"]
mod tests;

const SUBAGENT_TOOL: &str = "subagent";

impl AgentInstance {
    /// Contextual description and parameters for `subagent`: the session's
    /// pinned definition, else the current settings (Agent previews, sessions
    /// that have not offered the tool yet).
    pub(super) fn subagent_tool_contract(
        &self,
        base_parameters: serde_json::Value,
    ) -> (String, serde_json::Value) {
        let pinned = self
            .pinned_subagent_tool
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let contract = pinned.unwrap_or_else(|| self.live_subagent_tool_contract(base_parameters));
        (contract.description, contract.parameters)
    }

    fn live_subagent_tool_contract(
        &self,
        mut parameters: serde_json::Value,
    ) -> SessionToolContract {
        self.apply_subagent_model_parameter(&mut parameters);
        SessionToolContract {
            description: self.render_subagent_tool_description(),
            parameters,
        }
    }

    /// Loads the session's pinned definition, or records one the first time
    /// the session offers `subagent`. Runs before each run assembles tools.
    pub(super) fn pin_session_tool_contracts(&self, store: &SessionStore) {
        if self
            .pinned_subagent_tool
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            return;
        }
        let pinned = match store.session_tool_contract(&self.session_id, SUBAGENT_TOOL) {
            Ok(Some(contract)) => Some(contract),
            Ok(None) if self.multi_agent_enabled && !self.subagent_tool_suppressed => {
                let Some((_, base_parameters)) = self.tool_registry.tool_description(SUBAGENT_TOOL)
                else {
                    return;
                };
                let contract = self
                    .claude_catalog_subagent_contract()
                    .unwrap_or_else(|| self.live_subagent_tool_contract(base_parameters));
                match store.pin_session_tool_contract(&self.session_id, SUBAGENT_TOOL, &contract) {
                    Ok(contract) => Some(contract),
                    Err(error) => {
                        eprintln!(
                            "[Agent {}] subagent definition was not pinned: session={} {error}",
                            self.id, self.session_id
                        );
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                eprintln!(
                    "[Agent {}] pinned subagent definition is unreadable: session={} {error}",
                    self.id, self.session_id
                );
                None
            }
        };
        if let Some(contract) = pinned {
            *self
                .pinned_subagent_tool
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(contract);
        }
    }

    /// A Claude Code conversation that offered `subagent` before session
    /// pinning existed has already fixed the definition in its tool catalog.
    fn claude_catalog_subagent_contract(&self) -> Option<SessionToolContract> {
        if !matches!(self.backend, LlmBackend::ClaudeCodeCli) {
            return None;
        }
        let definition = crate::llm::claude_code_cli::tool_catalog::pinned_definition(
            &self.session_id,
            SUBAGENT_TOOL,
        )?;
        Some(contract_from_published_definition(
            &definition.description,
            definition.input_schema,
        ))
    }

    /// Current delegation choices for call-time errors. The session's tool
    /// description may predate the current Agent settings.
    pub(super) fn current_subagent_choices(&self) -> String {
        let lines = self
            .registry
            .list_subagent_descriptions_for_project_type(self.workspace_project_type())
            .into_iter()
            .map(|(id, _)| {
                let models = Self::subagent_model_labels(&self.subagent_model_choices(&id))
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        if index == 0 {
                            format!("`{label}` (default)")
                        } else {
                            format!("`{label}`")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("- {id}: {models}")
            })
            .collect::<Vec<_>>();
        if lines.is_empty() {
            "No agent type is currently available for delegation in this workspace.".to_string()
        } else {
            format!(
                "Currently available agent types and models:\n{}",
                lines.join("\n")
            )
        }
    }

    /// Delegation target for `subagent_type`, checked against current settings.
    pub(super) fn resolve_delegation_target(
        &self,
        subagent_type: &str,
    ) -> Result<AgentDef, String> {
        match self.registry.get(subagent_type) {
            Some(def)
                if def.is_delegation_target_for_project_type(self.workspace_project_type()) =>
            {
                Ok(def.clone())
            }
            found => Err(format!(
                "Agent type '{}' {}. {}",
                subagent_type,
                if found.is_some() {
                    "is not available for delegation in this workspace"
                } else {
                    "is unknown"
                },
                self.current_subagent_choices()
            )),
        }
    }
}

/// Inverse of the request decoration applied on top of the contract: the
/// Multi-Agent guidance suffix and the async-task parameter.
fn contract_from_published_definition(
    description: &str,
    mut parameters: serde_json::Value,
) -> SessionToolContract {
    let guidance = format!("\n\n{}", multi_agent::PROACTIVE_DELEGATION_GUIDANCE);
    let description = description
        .strip_suffix(&guidance)
        .unwrap_or(description)
        .to_string();
    if let Some(properties) = parameters
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        properties.remove(crate::async_tasks::ASYNC_MODE_PARAMETER);
    }
    SessionToolContract {
        description,
        parameters,
    }
}
