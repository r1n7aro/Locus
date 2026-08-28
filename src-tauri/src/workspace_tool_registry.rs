use std::path::Path;
use std::sync::Arc;

use crate::agent::definition::AgentDefRegistry;
use crate::tool::ToolRegistry;
use crate::workspace_service::WorkspaceRuntime;

type ToolOverlayLoader =
    dyn Fn(&mut ToolRegistry, &Path) -> Result<(), String> + Send + Sync + 'static;

/// Builds the effective tool schema for one checkout execution.
///
/// The process registry is an immutable application base. Project Skill and
/// plugin overlays are discovered from the resolved checkout root for every
/// snapshot, so a different focused window can never replace another run's
/// tool definitions and runtime edits become visible on the next run.
pub struct WorkspaceToolRegistry {
    app_base: Arc<ToolRegistry>,
    overlay_loader: Arc<ToolOverlayLoader>,
}

impl WorkspaceToolRegistry {
    pub fn new(app_base: Arc<ToolRegistry>) -> Self {
        Self::with_loader(app_base, Arc::new(load_checkout_tool_overlay))
    }

    fn with_loader(app_base: Arc<ToolRegistry>, overlay_loader: Arc<ToolOverlayLoader>) -> Self {
        Self {
            app_base,
            overlay_loader,
        }
    }

    pub async fn snapshot(
        &self,
        runtime: &WorkspaceRuntime,
        agent_definitions: &AgentDefRegistry,
    ) -> Result<Arc<ToolRegistry>, String> {
        let expected_generation = runtime.generation();
        let checkout_id = runtime.checkout_id().clone();
        let root = runtime.root().to_path_buf();
        let mut registry = self.app_base.as_ref().clone();
        let loader = Arc::clone(&self.overlay_loader);
        let subagents = agent_definitions.list_subagent_descriptions();

        let registry = tokio::task::spawn_blocking(move || {
            loader(&mut registry, &root)?;
            if !subagents.is_empty() {
                registry.register_subagent_tool(&subagents);
            }
            Ok::<_, String>(registry)
        })
        .await
        .map_err(|error| {
            format!("failed to build tool definitions for checkout {checkout_id}: {error}")
        })??;

        if runtime.generation() != expected_generation {
            return Err(format!(
                "workspace runtime generation changed while building tool definitions for checkout {} (expected {}, found {})",
                runtime.checkout_id(),
                expected_generation,
                runtime.generation()
            ));
        }
        Ok(Arc::new(registry))
    }
}

fn load_checkout_tool_overlay(registry: &mut ToolRegistry, root: &Path) -> Result<(), String> {
    let working_dir = root.to_string_lossy();
    crate::commands::register_skill_package_tools_for_working_dir(registry, &working_dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::future;

    use crate::tool::{ToolDef, ToolResult};
    use crate::workspace_service::identity::ProjectIdResolver;

    use super::*;

    fn runtime(root: &Path) -> Arc<WorkspaceRuntime> {
        WorkspaceRuntime::new(
            ProjectIdResolver::resolve(root).expect("workspace identity"),
            Vec::new(),
            1,
        )
    }

    fn test_tool(name: &str, output: &str) -> ToolDef {
        let output = output.to_string();
        ToolDef {
            name: name.to_string(),
            description: output.clone(),
            parameters: serde_json::json!({ "type": "object" }),
            mutates_workspace: false,
            execute: Arc::new(move |_, _| {
                let output = output.clone();
                Box::pin(future::ready(ToolResult {
                    output,
                    is_error: false,
                }))
            }),
        }
    }

    #[tokio::test]
    async fn checkout_tool_snapshots_do_not_overwrite_each_other() {
        let temp = tempfile::tempdir().expect("temp");
        let root_a = temp.path().join("checkout-a");
        let root_b = temp.path().join("checkout-b");
        std::fs::create_dir_all(&root_a).expect("checkout A");
        std::fs::create_dir_all(&root_b).expect("checkout B");
        let runtime_a = runtime(&root_a);
        let runtime_b = runtime(&root_b);
        let loader: Arc<ToolOverlayLoader> = Arc::new(|registry, root| {
            let marker = root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            registry.register(test_tool("project_tool", marker));
            Ok(())
        });
        let registry =
            WorkspaceToolRegistry::with_loader(Arc::new(ToolRegistry::with_builtins()), loader);
        let definitions = AgentDefRegistry::load(None, None);

        let (snapshot_a, snapshot_b) = tokio::join!(
            registry.snapshot(runtime_a.as_ref(), &definitions),
            registry.snapshot(runtime_b.as_ref(), &definitions),
        );
        let snapshot_a = snapshot_a.expect("snapshot A");
        let snapshot_b = snapshot_b.expect("snapshot B");

        assert_eq!(
            snapshot_a
                .get("project_tool")
                .map(|tool| tool.description.as_str()),
            Some("checkout-a")
        );
        assert_eq!(
            snapshot_b
                .get("project_tool")
                .map(|tool| tool.description.as_str()),
            Some("checkout-b")
        );
    }
}
