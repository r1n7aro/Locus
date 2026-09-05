use super::*;

impl AgentInstance {
    pub(super) async fn new_subagent_instance(
        &self,
        app_handle: &AppHandle,
        store: &SessionStore,
        agent_def: AgentDef,
        session_id: &str,
        cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Self, String> {
        let mut child = self
            .new_subagent_instance_with(
                store,
                agent_def,
                session_id,
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
        cancel_rx: tokio::sync::watch::Receiver<bool>,
        resolve_backend: F,
    ) -> Result<Self, String>
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<LlmBackend, String>>,
    {
        let subagent_type = agent_def.id.as_str();
        let model = self
            .resumed_subagent
            .as_ref()
            .map(|info| info.model_id.clone())
            .or_else(|| self.resolve_subagent_model_name(subagent_type))
            .unwrap_or_else(|| self.effective_model.clone())
            .trim()
            .to_string();
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
        child.async_tasks_enabled = self.async_tasks_enabled;
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
