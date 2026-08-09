pub mod builtins;
pub(crate) mod failure_log;
pub(crate) mod output;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

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
    pub app_handle: Option<AppHandle>,
    pub working_dir: Option<String>,
    pub unity_connected: Option<bool>,
    pub runtime_state: Option<Arc<ToolRuntimeState>>,
    pub cancel_rx: Option<tokio::sync::watch::Receiver<bool>>,
    pub progress: Option<crate::async_tasks::TaskProgressReporter>,
}

impl ToolExecutionContext {
    pub fn is_unity_connected(&self) -> bool {
        self.unity_connected.unwrap_or(false)
    }

    pub fn report_progress(&self, progress: impl Into<String>) {
        if let Some(report) = self.progress.as_ref() {
            report(progress.into());
        }
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

#[derive(Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    /// Declares that executing this tool can change files in the workspace,
    /// so rounds containing it must be checkpointed for undo. Every tool
    /// definition states this explicitly — there is no central whitelist.
    pub mutates_workspace: bool,
    pub execute: ToolExecuteFn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLoadMode {
    Direct,
    Lazy,
    Skill,
}

#[derive(Clone)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolDef>,
    built_in_tools: HashSet<String>,
    load_modes: HashMap<String, ToolLoadMode>,
}

fn normalize_tool_name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

pub fn built_in_tool_name_keys() -> BTreeSet<String> {
    static BUILT_IN_TOOL_NAME_KEYS: OnceLock<BTreeSet<String>> = OnceLock::new();
    BUILT_IN_TOOL_NAME_KEYS
        .get_or_init(|| {
            let registry = ToolRegistry::with_builtins();
            registry.built_in_tools.iter().cloned().collect()
        })
        .clone()
}

pub fn default_load_mode_for_builtin_tool(name: &str) -> ToolLoadMode {
    if matches!(
        normalize_tool_name_key(name).as_str(),
        "create_skill_package" | "skill_list" | "skill_reload" | "mcp_reload"
    ) {
        return ToolLoadMode::Skill;
    }

    if matches!(
        normalize_tool_name_key(name).as_str(),
        "unity_capture_viewport"
            | "unity_run_states"
            | "web_fetch"
            | "get_task_status"
            | "cancel_task"
    ) {
        ToolLoadMode::Lazy
    } else {
        ToolLoadMode::Direct
    }
}

/// Canonical tool ordering for LLM requests, the lazy-tool manifest and the
/// agent inspection UI: high-frequency core tools first, low-frequency
/// utilities last. Tools absent from this table (skill-package tools, future
/// builtins) sort after every listed tool, alphabetically.
const TOOL_PRIORITY_ORDER: &[&str] = &[
    // Meta tools (the agent loop additionally pins these to the front).
    "tool_load",
    "tool_call",
    // Core file & shell operations.
    "read",
    "edit",
    "write",
    "grep",
    "list",
    "bash",
    "get_task_status",
    "cancel_task",
    // Planning, delegation & user interaction.
    "todowrite",
    "subagent",
    "ask_user_question",
    "exit_plan_mode",
    // Unity editor actions.
    "unity_set_play_mode",
    "unity_execute",
    "unity_recompile",
    "unity_hot_reload",
    "unity_get_console_log",
    "unity_test_list",
    "unity_test_run",
    // Unity project search & inspection.
    "unity_asset_search",
    "unity_ref_search",
    "unity_code_usages",
    "unity_yaml_search",
    "unity_yaml_read",
    "unity_yaml_list",
    // C# code intelligence.
    "code_symbol_search",
    "code_goto_definition",
    "code_find_references",
    "code_diagnostics",
    "code_hover",
    // Knowledge base.
    "knowledge_query",
    // Low-frequency utilities (mostly lazy-loaded).
    "web_fetch",
    "unity_run_states",
    "unity_capture_viewport",
    "config_query",
    // Skill & plugin management.
    "create_skill_package",
    "skill_list",
    "skill_reload",
    "mcp_reload",
    "plugin_list",
    "plugin_search",
    "plugin_install",
    "plugin_set_enabled",
    "plugin_uninstall",
    "plugin_export",
    // View authoring & automation.
    "view_create",
    "view_list",
    "view_reload",
    "view_run",
    "view_compile_script",
    "view_call_script",
    "view_property_read",
    "view_property_discover",
    "view_property_write",
    "view_property_apply",
    "view_capture",
    "view_snapshot",
    "view_action",
    "view_wait",
    "view_console_read",
    "view_debug_eval",
];

fn tool_priority_rank(name: &str) -> usize {
    static RANKS: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    let ranks = RANKS.get_or_init(|| {
        TOOL_PRIORITY_ORDER
            .iter()
            .enumerate()
            .map(|(index, name)| (*name, index))
            .collect()
    });
    ranks
        .get(normalize_tool_name_key(name).as_str())
        .copied()
        .unwrap_or(TOOL_PRIORITY_ORDER.len())
}

/// Sorts tool names by [`TOOL_PRIORITY_ORDER`]; unknown tools keep a stable
/// alphabetical order after every known one.
pub fn sort_tool_names_by_priority(names: &mut [String]) {
    names.sort_by(|a, b| {
        tool_priority_rank(a)
            .cmp(&tool_priority_rank(b))
            .then_with(|| a.cmp(b))
    });
}

impl ToolRegistry {
    pub fn new() -> Self {
        ToolRegistry {
            tools: HashMap::new(),
            built_in_tools: HashSet::new(),
            load_modes: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn register(&mut self, tool: ToolDef) {
        let key = normalize_tool_name_key(&tool.name);
        self.load_modes.insert(key.clone(), ToolLoadMode::Skill);
        self.tools.insert(key, tool);
    }

    pub fn register_runtime(&mut self, tool: ToolDef, load_mode: ToolLoadMode) {
        let key = normalize_tool_name_key(&tool.name);
        self.load_modes.insert(key.clone(), load_mode);
        self.tools.insert(key, tool);
    }

    pub fn register_builtin(&mut self, tool: ToolDef) {
        let mode = default_load_mode_for_builtin_tool(&tool.name);
        self.register_builtin_with_load_mode(tool, mode);
    }

    pub fn register_builtin_with_load_mode(&mut self, tool: ToolDef, load_mode: ToolLoadMode) {
        let key = normalize_tool_name_key(&tool.name);
        self.built_in_tools.insert(key.clone());
        self.load_modes.insert(key.clone(), load_mode);
        self.tools.insert(key, tool);
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(&normalize_tool_name_key(name))
    }

    pub fn canonical_name(&self, name: &str) -> Option<String> {
        let normalized = normalize_tool_name_key(name);
        let lookup_name = if normalized == "task" && self.get("subagent").is_some() {
            "subagent"
        } else {
            normalized.as_str()
        };
        self.get(lookup_name)
            .map(|def| def.name.clone())
            .or_else(|| crate::commands::canonical_skill_package_tool_name(name))
    }

    pub fn tool_description(&self, name: &str) -> Option<(String, serde_json::Value)> {
        self.get(name)
            .map(|def| (def.description.clone(), def.parameters.clone()))
            .or_else(|| crate::commands::skill_package_tool_description_sync(name))
    }

    pub fn is_built_in(&self, name: &str) -> bool {
        self.built_in_tools.contains(&normalize_tool_name_key(name))
    }

    /// Whether a tool declares workspace mutations (drives undo tracking).
    /// Skill-package tools fall back to their manifest declaration; unknown
    /// tools default to not mutating.
    pub fn mutates_workspace(&self, name: &str) -> bool {
        self.get(name)
            .map(|def| def.mutates_workspace)
            .or_else(|| crate::commands::skill_package_tool_mutates_workspace_sync(name))
            .unwrap_or(false)
    }

    pub fn skill_tool_names(&self) -> Vec<String> {
        let mut names = self
            .tools
            .iter()
            .filter_map(|(key, def)| (!self.built_in_tools.contains(key)).then(|| def.name.clone()))
            .collect::<Vec<_>>();
        names.extend(crate::commands::skill_package_tool_names_sync());
        names.sort();
        names.dedup();
        names
    }

    /// Canonical names for every tool currently registered in this process.
    /// Dynamic MCP tools are maintained by the MCP manager and are appended by
    /// callers that need the complete agent-facing inventory.
    pub fn tool_names(&self) -> Vec<String> {
        let mut names = self
            .tools
            .values()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        names.extend(crate::commands::skill_package_tool_names_sync());
        names.sort();
        names.dedup();
        names
    }

    pub fn default_load_mode(&self, name: &str) -> ToolLoadMode {
        self.load_modes
            .get(&normalize_tool_name_key(name))
            .copied()
            .unwrap_or(ToolLoadMode::Skill)
    }

    pub fn resolve_api_tool(&self, name: &str) -> Option<serde_json::Value> {
        self.get(name)
            .map(|def| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": def.name,
                        "description": def.description,
                        "parameters": def.parameters,
                    }
                })
            })
            .or_else(|| crate::commands::resolve_skill_package_api_tool_sync(name))
    }

    pub fn resolve_api_tools(&self, tool_names: &[String]) -> Vec<serde_json::Value> {
        tool_names
            .iter()
            .filter_map(|name| self.resolve_api_tool(name))
            .collect()
    }

    #[allow(dead_code)]
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
            None => match crate::commands::execute_skill_package_tool_by_api_name(
                name,
                arguments.clone(),
                context,
            )
            .await
            {
                Some(result) => result,
                None => ToolResult {
                    output: format!("Tool '{}' not found", name),
                    is_error: true,
                },
            },
        }
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        builtins::register_all(&mut registry);
        registry
    }

    pub fn register_subagent_tool(&mut self, subagents: &[(String, String)]) {
        let agent_list: String = subagents
            .iter()
            .map(|(id, desc)| format!("- {}: {}", id, desc))
            .collect::<Vec<_>>()
            .join("\n");

        let description = crate::prompt::tools::SUBAGENT.replace("{agent_list}", &agent_list);

        let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
            Box::pin(async {
                ToolResult {
                    output: "Error: subagent tool should be intercepted by agent loop, not executed directly".to_string(),
                    is_error: true,
                }
            })
        });

        self.register_builtin(ToolDef {
            name: "subagent".to_string(),
            description,
            // Subagents run their own tracked rounds via the shared
            // UndoManager; tracking the parent `subagent` round as well would
            // double-record the same changes.
            mutates_workspace: false,
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
    use super::{
        sort_tool_names_by_priority, tool_priority_rank, ToolDef, ToolExecutionContext,
        ToolLoadMode, ToolRegistry, ToolResult, ToolRuntimeState, TOOL_PRIORITY_ORDER,
    };
    use std::sync::Arc;

    #[test]
    fn priority_sort_puts_high_frequency_tools_first_and_unknown_tools_last() {
        let mut names = vec![
            "web_fetch".to_string(),
            "zz_custom_skill_tool".to_string(),
            "unity_execute".to_string(),
            "knowledge_query".to_string(),
            "a_custom_skill_tool".to_string(),
            "edit".to_string(),
            "read".to_string(),
        ];
        sort_tool_names_by_priority(&mut names);
        assert_eq!(
            names,
            vec![
                "read".to_string(),
                "edit".to_string(),
                "unity_execute".to_string(),
                "knowledge_query".to_string(),
                "web_fetch".to_string(),
                "a_custom_skill_tool".to_string(),
                "zz_custom_skill_tool".to_string(),
            ]
        );
    }

    #[test]
    fn every_builtin_tool_has_an_explicit_priority_rank() {
        let mut registry = ToolRegistry::with_builtins();
        registry.register_subagent_tool(&[]);
        for key in &registry.built_in_tools {
            assert!(
                tool_priority_rank(key) < TOOL_PRIORITY_ORDER.len(),
                "built-in tool `{key}` is missing from TOOL_PRIORITY_ORDER"
            );
        }
    }

    #[test]
    fn registry_resolves_api_tools_with_canonical_name_case_insensitively() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolDef {
            name: "edit".to_string(),
            description: "Edit a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            mutates_workspace: true,
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
        assert_eq!(registry.canonical_name(" EDIT ").as_deref(), Some("edit"));
    }

    #[tokio::test]
    async fn registry_executes_tool_names_case_insensitively() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolDef {
            name: "edit".to_string(),
            description: "Edit a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            mutates_workspace: true,
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
    fn registry_tracks_default_load_modes() {
        let mut registry = ToolRegistry::new();
        registry.register(ToolDef {
            name: "skill_tool".to_string(),
            description: "Skill tool".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            mutates_workspace: false,
            execute: Arc::new(|_, _| {
                Box::pin(async {
                    ToolResult {
                        output: String::new(),
                        is_error: false,
                    }
                })
            }),
        });
        registry.register_builtin_with_load_mode(
            ToolDef {
                name: "builtin_lazy".to_string(),
                description: "Builtin lazy".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                mutates_workspace: false,
                execute: Arc::new(|_, _| {
                    Box::pin(async {
                        ToolResult {
                            output: String::new(),
                            is_error: false,
                        }
                    })
                }),
            },
            ToolLoadMode::Lazy,
        );

        assert_eq!(
            registry.default_load_mode("skill_tool"),
            ToolLoadMode::Skill
        );
        assert_eq!(
            registry.default_load_mode("builtin_lazy"),
            ToolLoadMode::Lazy
        );
    }

    #[test]
    fn builtins_register_web_fetch_as_lazy_without_legacy_name() {
        let registry = ToolRegistry::with_builtins();

        assert_eq!(
            registry.canonical_name("web_fetch").as_deref(),
            Some("web_fetch")
        );
        assert_eq!(registry.default_load_mode("web_fetch"), ToolLoadMode::Lazy);
        assert_eq!(registry.canonical_name("webfetch"), None);
    }

    #[test]
    fn builtins_register_unity_capture_viewport_as_lazy() {
        let registry = ToolRegistry::with_builtins();

        assert_eq!(
            registry.canonical_name("unity_capture_viewport").as_deref(),
            Some("unity_capture_viewport")
        );
        assert_eq!(
            registry.default_load_mode("unity_capture_viewport"),
            ToolLoadMode::Lazy
        );
    }

    #[test]
    fn registry_maps_legacy_task_name_to_subagent() {
        let mut registry = ToolRegistry::with_builtins();
        registry.register_subagent_tool(&[]);

        assert_eq!(registry.canonical_name("task").as_deref(), Some("subagent"));
        assert_eq!(
            registry.canonical_name("subagent").as_deref(),
            Some("subagent")
        );
    }

    #[test]
    fn removed_display_tools_are_not_registered() {
        let registry = ToolRegistry::with_builtins();

        for removed in ["sheet", "graph_view"] {
            assert_eq!(registry.canonical_name(removed), None);
        }
    }

    #[test]
    fn builtins_register_low_frequency_code_tools_as_lazy() {
        let registry = ToolRegistry::with_builtins();

        assert_eq!(
            registry.default_load_mode("code_find_references"),
            ToolLoadMode::Direct
        );
        assert_eq!(
            registry.default_load_mode("code_diagnostics"),
            ToolLoadMode::Direct
        );
    }

    #[test]
    fn builtins_register_only_knowledge_query() {
        let registry = ToolRegistry::with_builtins();

        assert_eq!(
            registry.canonical_name("knowledge_query").as_deref(),
            Some("knowledge_query")
        );
        for removed in [
            "knowledge_list",
            "knowledge_read",
            "knowledge_create",
            "knowledge_edit",
            "knowledge_move",
            "knowledge_delete",
        ] {
            assert_eq!(registry.canonical_name(removed), None);
        }
    }

    #[test]
    fn builtins_register_skill_lifecycle_tools_as_skill_loaded() {
        let registry = ToolRegistry::with_builtins();

        assert_eq!(registry.canonical_name("skill_create"), None);
        assert_eq!(
            registry.canonical_name("create_skill_package").as_deref(),
            Some("create_skill_package")
        );
        assert_eq!(
            registry.default_load_mode("create_skill_package"),
            ToolLoadMode::Skill
        );
        assert_eq!(
            registry.default_load_mode("skill_list"),
            ToolLoadMode::Skill
        );
        assert_eq!(
            registry.default_load_mode("skill_reload"),
            ToolLoadMode::Skill
        );
    }

    #[test]
    fn unity_asset_read_redirects_only_once_for_same_file() {
        let context = ToolExecutionContext {
            app_handle: None,
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(true),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
            ..Default::default()
        };

        assert!(context.should_redirect_unity_asset_read("Assets/Test/MyAsset.asset"));
        assert!(!context.should_redirect_unity_asset_read("Assets\\Test\\MyAsset.asset"));
        assert!(!context.should_redirect_unity_asset_read("C:/Project/Assets/Test/MyAsset.asset"));
    }

    #[test]
    fn unity_asset_read_redirect_requires_connection_and_supported_extension() {
        let disconnected = ToolExecutionContext {
            app_handle: None,
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(false),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
            ..Default::default()
        };
        assert!(!disconnected.should_redirect_unity_asset_read("Assets/Test/MyAsset.asset"));

        let connected = ToolExecutionContext {
            app_handle: None,
            working_dir: Some("C:/Project".to_string()),
            unity_connected: Some(true),
            runtime_state: Some(Arc::new(ToolRuntimeState::default())),
            ..Default::default()
        };
        assert!(!connected.should_redirect_unity_asset_read("src/main.rs"));
    }
}
