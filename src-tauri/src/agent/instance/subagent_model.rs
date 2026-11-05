use super::*;

/// Appended to the subagent tool description only while some agent type
/// offers a model choice, so sessions without choices see the old contract.
pub(super) const SUBAGENT_MODEL_CHOICE_GUIDANCE: &str = "\n\nWhen an agent lists several models, you may set model to one of the listed names to fit the task, such as a smaller model for directed searches and a larger one for difficult reasoning or implementation. Omit model to use the default.";

impl AgentInstance {
    /// Models the delegating agent may choose for `subagent_type`, default
    /// first. An unconfigured type runs on this session's model.
    pub(super) fn subagent_model_choices(&self, subagent_type: &str) -> Vec<String> {
        match self.subagent_model_overrides.allowed(subagent_type) {
            [] => vec![self.effective_model.trim().to_string()],
            allowed => allowed.to_vec(),
        }
    }

    /// Whether any delegatable agent offers more than one model, which is
    /// when the subagent tool exposes its `model` parameter.
    pub(super) fn subagent_model_choice_available(&self) -> bool {
        self.registry
            .list_subagent_descriptions_for_project_type(self.workspace_project_type())
            .iter()
            .any(|(id, _)| self.subagent_model_overrides.allowed(id).len() > 1)
    }

    /// Removes the optional `model` parameter while no agent type offers a
    /// choice, keeping the schema unchanged for the default configuration.
    pub(super) fn apply_subagent_model_parameter(&self, parameters: &mut serde_json::Value) {
        if self.subagent_model_choice_available() {
            return;
        }
        if let Some(properties) = parameters
            .get_mut("properties")
            .and_then(serde_json::Value::as_object_mut)
        {
            properties.remove("model");
        }
    }

    /// Names shown to the delegating agent. Short display names are used
    /// unless they would match another allowed model, so every shown name
    /// selects exactly one model.
    pub(super) fn subagent_model_labels(models: &[String]) -> Vec<String> {
        models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let short = Self::subagent_model_display_name(model);
                let ambiguous = models.iter().enumerate().any(|(other_index, other)| {
                    other_index != index
                        && (Self::subagent_model_display_name(other) == short
                            || other.trim() == short)
                });
                if ambiguous { model.trim() } else { short }.to_string()
            })
            .collect()
    }

    /// Model for a subagent run. A resumed task keeps the model it started
    /// with; otherwise the requested model must be one of the type's allowed
    /// models, and an omitted request selects the type's default.
    pub(super) fn subagent_launch_model(
        &self,
        subagent_type: &str,
        requested: Option<&str>,
    ) -> Result<String, String> {
        if let Some(info) = self.resumed_subagent.as_ref() {
            return Ok(info.model_id.trim().to_string());
        }
        let choices = self.subagent_model_choices(subagent_type);
        let Some(requested) = requested.map(str::trim).filter(|model| !model.is_empty()) else {
            return Ok(choices[0].clone());
        };
        let labels = Self::subagent_model_labels(&choices);
        choices
            .iter()
            .zip(&labels)
            .find(|(model, label)| model.as_str() == requested || label.as_str() == requested)
            .map(|(model, _)| model.clone())
            .ok_or_else(|| {
                format!(
                    "Model '{}' is not available for subagent type '{}' under the current Agent settings. Choose one of: {}. Omit model to use the default `{}`.",
                    requested,
                    subagent_type,
                    labels
                        .iter()
                        .map(|label| format!("`{label}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    labels[0]
                )
            })
    }

    pub(super) async fn new_subagent_instance(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        agent_def: AgentDef,
        session_id: &str,
        requested_model: Option<&str>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Self, String> {
        let mut child = self
            .new_subagent_instance_with(
                store,
                agent_def,
                session_id,
                requested_model,
                cancel_rx,
                |model| async move {
                    crate::commands::resolve_model_backend_for_app(app_handle, &model)
                        .await
                        .map_err(|error| error.to_string())
                },
            )
            .await?;
        let max_depth = app_handle
            .state::<Arc<crate::config::AppConfig>>()
            .subagent_max_depth();
        child.subagent_tool_suppressed = child.subagent_depth >= max_depth;
        Ok(child)
    }

    /// Keep construction and persistence on the same resolved route. The resolver
    /// is injected so tests can inspect real HTTP requests without account state.
    async fn new_subagent_instance_with<F, Fut>(
        &self,
        store: &SessionStore,
        agent_def: AgentDef,
        session_id: &str,
        requested_model: Option<&str>,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        resolve_backend: F,
    ) -> Result<Self, String>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<LlmBackend, String>>,
    {
        let subagent_type = agent_def.id.as_str();
        let model = self.subagent_launch_model(subagent_type, requested_model)?;
        if model.is_empty() {
            return Err("No model selected for subagent.".to_string());
        }
        // A backend contains credentials, protocol and (for Custom) api_model
        // and capabilities. Cloning the parent's backend would keep its route
        // even when effective_model names a different child model.
        let backend = resolve_backend(model.clone())
            .await
            .map_err(|error| format!("Failed to resolve subagent model '{}': {}", model, error))?;
        let effort = self
            .resumed_subagent
            .as_ref()
            .map(|info| info.effort.clone())
            .unwrap_or_else(|| self.resolve_subagent_effort(subagent_type));
        let fast_mode = self
            .resumed_subagent
            .as_ref()
            .map(|info| info.fast_mode)
            .unwrap_or_else(|| self.resolve_subagent_fast_mode(subagent_type));
        let mut child = Self::new(
            Arc::new(agent_def),
            session_id,
            backend,
            self.debug,
            self.registry.clone(),
            self.tool_registry.clone(),
            self.working_dir.clone(),
            self.raw_store.clone(),
            self.workspace_id.clone(),
            model,
            effort,
            self.app_knowledge_dir.clone(),
            self.app_agent_dir.clone(),
            self.knowledge_access_mode,
            self.undo_manager.clone(),
            self.subagent_model_overrides.clone(),
            cancel_rx,
        );
        child.execution_context = self.execution_context.clone();
        child.subagent_depth = self.subagent_depth + 1;
        child.multi_agent_enabled = self.multi_agent_enabled;
        child.subagent_active = self.subagent_active.clone();
        child.session_undo_enabled = self.session_undo_enabled;
        child.codex_fast_mode = fast_mode;
        child.subagent_effort_overrides = self.subagent_effort_overrides.clone();
        child.subagent_fast_mode_overrides = self.subagent_fast_mode_overrides.clone();
        if self.plan_runtime_snapshot().is_some()
            || self
                .resumed_subagent
                .as_ref()
                .is_some_and(|info| info.readonly)
        {
            child.mark_plan_readonly_subagent();
        }
        store
            .set_session_execution_state(
                session_id,
                &child.effective_model,
                child.effort.as_deref(),
                child.codex_fast_mode,
                Some(child.multi_agent_enabled),
            )
            .map_err(|error| format!("Failed to save subagent execution state: {}", error))?;
        Ok(child)
    }
}

#[cfg(test)]
#[path = "subagent_model_tests.rs"]
mod tests;
