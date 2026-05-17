pub mod builtins;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, Default)]
pub struct ToolRuntimeState {
    seen_unity_asset_reads: Mutex<HashSet<String>>,
}

#[derive(Clone, Default)]
pub struct ToolExecutionContext {
    pub working_dir: Option<String>,
    pub unity_connected: Option<bool>,
    pub runtime_state: Option<Arc<ToolRuntimeState>>,
}

impl ToolExecutionContext {
    pub fn is_unity_connected(&self) -> bool {
        self.unity_connected.unwrap_or(false)
    }

    pub fn should_redirect_unity_asset_read(&self, file_path: &str) -> bool {
        if !self.is_unity_connected() || !is_unity_yaml_candidate_path(file_path) {
            return false;
        }

        let key = self.normalize_path_for_session(file_path);
        match self.runtime_state.as_ref() {
            Some(state) => {
                let mut seen = state
                    .seen_unity_asset_reads
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                seen.insert(key)
            }
            None => true,
        }
    }

    fn normalize_path_for_session(&self, file_path: &str) -> String {
        let path = Path::new(file_path);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(working_dir) = self.working_dir.as_deref() {
            Path::new(working_dir).join(path)
        } else {
            path.to_path_buf()
        };

        resolved.to_string_lossy().replace('\\', "/").to_lowercase()
    }
}

pub fn is_unity_yaml_candidate_path(file_path: &str) -> bool {
    let lower = file_path.trim().to_ascii_lowercase();
    [
        ".unity",
        ".prefab",
        ".asset",
        ".mat",
        ".anim",
        ".controller",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

pub type ToolExecuteFn = Arc<
    dyn Fn(
            serde_json::Value,
            ToolExecutionContext,
        ) -> Pin<Box<dyn Future<Output = ToolResult> + Send>>
        + Send
        + Sync,
>;

pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub execute: ToolExecuteFn,
}

pub struct ToolRegistry {
    tools: HashMap<String, ToolDef>,
}

fn normalize_tool_name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: ToolDef) {
        self.tools.insert(normalize_tool_name_key(&tool.name), tool);
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(&normalize_tool_name_key(name))
    }

    pub fn canonical_name(&self, name: &str) -> Option<&str> {
        self.get(name).map(|def| def.name.as_str())
    }

    pub fn resolve_api_tools(&self, tool_names: &[String]) -> Vec<serde_json::Value> {
        tool_names
            .iter()
            .filter_map(|name| {
                self.get(name).map(|def| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": def.name,
                            "description": def.description,
                            "parameters": def.parameters,
                        }
                    })
                })
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, arguments: &serde_json::Value) -> ToolResult {
        self.execute_with_context(name, arguments, ToolExecutionContext::default())
            .await
    }

    pub async fn execute_with_context(
        &self,
        name: &str,
        arguments: &serde_json::Value,
        context: ToolExecutionContext,
    ) -> ToolResult {
        match self.get(name) {
            Some(def) => (def.execute)(arguments.clone(), context).await,
            None => ToolResult {
                output: format!("Tool '{}' not found", name),
                is_error: true,
            },
        }
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        builtins::register_all(&mut registry);
        registry
    }

    pub fn register_task_tool(&mut self, subagents: &[(String, String)]) {
        let agent_list: String = subagents
            .iter()
            .map(|(id, desc)| format!("- {}: {}", id, desc))
            .collect::<Vec<_>>()
            .join("\n");

        let description = crate::prompt::tools::TASK.replace("{agent_list}", &agent_list);

        let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
            Box::pin(async {
                ToolResult {
                    output: "Error: task tool should be intercepted by agent loop, not executed directly".to_string(),
                    is_error: true,
                }
            })
        });

        self.register(ToolDef {
            name: "task".to_string(),
            description,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "A short (3-5 words) description of the task"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The task for the agent to perform"
                    },
                    "subagent_type": {
                        "type": "string",
                        "description": "The type of specialized agent to use for this task"
                    }
                },
                "required": ["description", "prompt", "subagent_type"]
            }),
            execute,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolDef, ToolExecutionContext, ToolRegistry, ToolResult, ToolRuntimeState};
    use std::sync::Arc;

    #[test]
    fn registry_resolves_api_tools_with_canonical_name_case_insensitively() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolDef {
            name: "edit".to_string(),
            description: "Edit a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            execute: Arc::new(|_, _| {
                Box::pin(async {
                    ToolResult {
                        output: String::new(),
                        is_error: false,
                    }
                })
            }),
        });

        let tools = registry.resolve_api_tools(&["Edit".to_string()]);

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"].as_str(), Some("edit"));
        assert_eq!(registry.canonical_name(" EDIT "), Some("edit"));
    }

    #[tokio::test]
    async fn registry_executes_tool_names_case_insensitively() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolDef {
            name: "edit".to_string(),
            description: "Edit a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            execute: Arc::new(|_, _| {
                Box::pin(async {
                    ToolResult {
                        output: "ok".to_string(),
                        is_error: false,
                    }
                })
            }),
        });

        let result = registry.execute("Edit", &serde_json::json!({})).await;

        assert!(!result.is_error);
        assert_eq!(result.output, "ok");
    }

    #[test]
    fn unity_asset_read_redirects_only_once_for_same_file() {
        let context = ToolExecutionContext {
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(true),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
        };

        assert!(context.should_redirect_unity_asset_read("Assets/Test/MyAsset.asset"));
        assert!(!context.should_redirect_unity_asset_read("Assets\\Test\\MyAsset.asset"));
        assert!(!context.should_redirect_unity_asset_read("C:/Project/Assets/Test/MyAsset.asset"));
    }

    #[test]
    fn unity_asset_read_redirect_requires_connection_and_supported_extension() {
        let disconnected = ToolExecutionContext {
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(false),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
        };
        assert!(!disconnected.should_redirect_unity_asset_read("Assets/Test/MyAsset.asset"));

        let connected = ToolExecutionContext {
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(true),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
        };
        assert!(!connected.should_redirect_unity_asset_read("src/main.rs"));
    }
}
