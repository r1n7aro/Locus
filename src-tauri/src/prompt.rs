pub mod commit {
    pub const COMMIT_MESSAGE: &str = include_str!("../../prompt/commit-message.md");
}

pub mod plan {
    pub const PLAN_REMINDER: &str = include_str!("../../prompt/plan-reminder.md");
}

/// Tool definition JSON（description + parameters schema）
pub mod tools {
    pub const TASK: &str = include_str!("../../tools/task.md");

    pub const READ: &str = include_str!("../../tools/read.json");
    pub const WRITE: &str = include_str!("../../tools/write.json");
    pub const EDIT: &str = include_str!("../../tools/edit.json");
    pub const BASH: &str = include_str!("../../tools/bash.json");
    pub const GREP: &str = include_str!("../../tools/grep.json");
    pub const WEB_FETCH: &str = include_str!("../../tools/web_fetch.json");
    pub const TODOWRITE: &str = include_str!("../../tools/todowrite.json");
    pub const GRAPH_VIEW: &str = include_str!("../../tools/graph_view.json");
    pub const UNITY_EXECUTE: &str = include_str!("../../tools/unity_execute.json");
    pub const UNITY_RUN_STATES: &str = include_str!("../../tools/unity_run_states.json");
    pub const UNITY_CAPTURE_VIEWPORT: &str =
        include_str!("../../tools/unity_capture_viewport.json");
    pub const UNITY_REF_SEARCH: &str = include_str!("../../tools/unity_ref_search.json");
    pub const UNITY_ASSET_SEARCH: &str = include_str!("../../tools/unity_asset_search.json");
    pub const UNITY_YAML_LIST: &str = include_str!("../../tools/unity_yaml_list.json");
    pub const UNITY_YAML_SEARCH: &str = include_str!("../../tools/unity_yaml_search.json");
    pub const UNITY_YAML_READ: &str = include_str!("../../tools/unity_yaml_read.json");
    pub const UNITY_RECOMPILE: &str = include_str!("../../tools/unity_recompile.json");
    pub const CODE_FIND_REFERENCES: &str = include_str!("../../tools/code_find_references.json");
    pub const CODE_GOTO_DEFINITION: &str = include_str!("../../tools/code_goto_definition.json");
    pub const CODE_SYMBOL_SEARCH: &str = include_str!("../../tools/code_symbol_search.json");
    pub const LIST: &str = include_str!("../../tools/list.json");
    pub const ASK: &str = include_str!("../../tools/ask.json");
    pub const SHEET: &str = include_str!("../../tools/sheet.json");
    pub const KNOWLEDGE_LIST: &str = include_str!("../../tools/knowledge_list.json");
    pub const KNOWLEDGE_QUERY: &str = include_str!("../../tools/knowledge_query.json");
    pub const KNOWLEDGE_READ: &str = include_str!("../../tools/knowledge_read.json");
    pub const KNOWLEDGE_CREATE: &str = include_str!("../../tools/knowledge_create.json");
    pub const KNOWLEDGE_DELETE: &str = include_str!("../../tools/knowledge_delete.json");
    pub const KNOWLEDGE_MOVE: &str = include_str!("../../tools/knowledge_move.json");
    pub const KNOWLEDGE_EDIT: &str = include_str!("../../tools/knowledge_edit.json");
    pub const SKILL_CREATE: &str = include_str!("../../tools/skill_create.json");
    pub const SKILL_RELOAD: &str = include_str!("../../tools/skill_reload.json");
    pub const SKILL_LIST: &str = include_str!("../../tools/skill_list.json");
    pub const PLUGIN_LIST: &str = include_str!("../../tools/plugin_list.json");
    pub const PLUGIN_SEARCH: &str = include_str!("../../tools/plugin_search.json");
    pub const PLUGIN_INSTALL: &str = include_str!("../../tools/plugin_install.json");
    pub const PLUGIN_UNINSTALL: &str = include_str!("../../tools/plugin_uninstall.json");
    pub const PLUGIN_SET_ENABLED: &str = include_str!("../../tools/plugin_set_enabled.json");
    pub const PLUGIN_EXPORT: &str = include_str!("../../tools/plugin_export.json");
    pub const VIEW_CREATE: &str = include_str!("../../tools/view_create.json");
    pub const VIEW_LIST: &str = include_str!("../../tools/view_list.json");
    pub const VIEW_RELOAD: &str = include_str!("../../tools/view_reload.json");
    pub const VIEW_RUN: &str = include_str!("../../tools/view_run.json");
    pub const VIEW_COMPILE_SCRIPT: &str = include_str!("../../tools/view_compile_script.json");
    pub const VIEW_CALL_SCRIPT: &str = include_str!("../../tools/view_call_script.json");
    pub const VIEW_PROPERTY_READ: &str = include_str!("../../tools/view_property_read.json");
    pub const VIEW_PROPERTY_DISCOVER: &str =
        include_str!("../../tools/view_property_discover.json");
    pub const VIEW_PROPERTY_WRITE: &str = include_str!("../../tools/view_property_write.json");
    pub const VIEW_PROPERTY_APPLY: &str = include_str!("../../tools/view_property_apply.json");
    pub const VIEW_CAPTURE: &str = include_str!("../../tools/view_capture.json");
    pub const VIEW_SNAPSHOT: &str = include_str!("../../tools/view_snapshot.json");
    pub const VIEW_ACTION: &str = include_str!("../../tools/view_action.json");
    pub const VIEW_WAIT: &str = include_str!("../../tools/view_wait.json");
    pub const VIEW_CONSOLE_READ: &str = include_str!("../../tools/view_console_read.json");
    pub const VIEW_DEBUG_EVAL: &str = include_str!("../../tools/view_debug_eval.json");
    pub const CONFIG_QUERY: &str = include_str!("../../tools/config_query.json");
    pub const TOOL_LOAD: &str = include_str!("../../tools/tool_load.json");
    pub const TOOL_CALL: &str = include_str!("../../tools/tool_call.json");
}

#[derive(serde::Deserialize)]
pub struct ToolPrompt {
    pub description: String,
    pub parameters: serde_json::Value,
}

pub fn parse_tool_prompt(json_str: &str) -> ToolPrompt {
    serde_json::from_str(json_str).expect("invalid tool prompt JSON (compile-time embedded)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_openai_compatible_tool_parameters(name: &str, schema: &serde_json::Value) {
        let object = schema
            .as_object()
            .unwrap_or_else(|| panic!("tool `{name}` parameters must be a JSON object"));

        assert_eq!(
            object.get("type").and_then(serde_json::Value::as_str),
            Some("object"),
            "tool `{name}` parameters must declare top-level type=object"
        );

        for keyword in ["oneOf", "anyOf", "allOf", "enum", "not"] {
            assert!(
                !object.contains_key(keyword),
                "tool `{name}` parameters must not contain top-level `{keyword}`"
            );
        }
    }

    #[test]
    fn embedded_tool_parameter_schemas_stay_openai_compatible() {
        let tool_prompts = [
            ("read", tools::READ),
            ("write", tools::WRITE),
            ("edit", tools::EDIT),
            ("bash", tools::BASH),
            ("grep", tools::GREP),
            ("web_fetch", tools::WEB_FETCH),
            ("todowrite", tools::TODOWRITE),
            ("graph_view", tools::GRAPH_VIEW),
            ("unity_execute", tools::UNITY_EXECUTE),
            ("unity_run_states", tools::UNITY_RUN_STATES),
            ("unity_capture_viewport", tools::UNITY_CAPTURE_VIEWPORT),
            ("unity_ref_search", tools::UNITY_REF_SEARCH),
            ("unity_asset_search", tools::UNITY_ASSET_SEARCH),
            ("unity_yaml_list", tools::UNITY_YAML_LIST),
            ("unity_yaml_search", tools::UNITY_YAML_SEARCH),
            ("unity_yaml_read", tools::UNITY_YAML_READ),
            ("unity_recompile", tools::UNITY_RECOMPILE),
            ("list", tools::LIST),
            ("ask", tools::ASK),
            ("sheet", tools::SHEET),
            ("knowledge_list", tools::KNOWLEDGE_LIST),
            ("knowledge_query", tools::KNOWLEDGE_QUERY),
            ("knowledge_read", tools::KNOWLEDGE_READ),
            ("knowledge_create", tools::KNOWLEDGE_CREATE),
            ("knowledge_delete", tools::KNOWLEDGE_DELETE),
            ("knowledge_move", tools::KNOWLEDGE_MOVE),
            ("knowledge_edit", tools::KNOWLEDGE_EDIT),
            ("skill_create", tools::SKILL_CREATE),
            ("skill_reload", tools::SKILL_RELOAD),
            ("skill_list", tools::SKILL_LIST),
            ("plugin_list", tools::PLUGIN_LIST),
            ("plugin_search", tools::PLUGIN_SEARCH),
            ("plugin_install", tools::PLUGIN_INSTALL),
            ("plugin_uninstall", tools::PLUGIN_UNINSTALL),
            ("plugin_set_enabled", tools::PLUGIN_SET_ENABLED),
            ("plugin_export", tools::PLUGIN_EXPORT),
            ("view_create", tools::VIEW_CREATE),
            ("view_list", tools::VIEW_LIST),
            ("view_reload", tools::VIEW_RELOAD),
            ("view_run", tools::VIEW_RUN),
            ("view_compile_script", tools::VIEW_COMPILE_SCRIPT),
            ("view_call_script", tools::VIEW_CALL_SCRIPT),
            ("view_property_read", tools::VIEW_PROPERTY_READ),
            ("view_property_discover", tools::VIEW_PROPERTY_DISCOVER),
            ("view_property_write", tools::VIEW_PROPERTY_WRITE),
            ("view_property_apply", tools::VIEW_PROPERTY_APPLY),
            ("view_capture", tools::VIEW_CAPTURE),
            ("view_snapshot", tools::VIEW_SNAPSHOT),
            ("view_action", tools::VIEW_ACTION),
            ("view_wait", tools::VIEW_WAIT),
            ("view_console_read", tools::VIEW_CONSOLE_READ),
            ("view_debug_eval", tools::VIEW_DEBUG_EVAL),
            ("config_query", tools::CONFIG_QUERY),
            ("tool_load", tools::TOOL_LOAD),
            ("tool_call", tools::TOOL_CALL),
        ];

        for (name, json_str) in tool_prompts {
            let prompt = parse_tool_prompt(json_str);
            assert_openai_compatible_tool_parameters(name, &prompt.parameters);
        }
    }

    #[test]
    fn unity_run_states_profiler_skill_reference_resolves() {
        let prompt = parse_tool_prompt(tools::UNITY_RUN_STATES);
        assert!(
            prompt.description.contains("skill/profiler.md"),
            "unity_run_states should direct the agent to the profiler skill"
        );

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("knowledge")
            .join("skill")
            .join("profiler.md");
        assert!(
            path.is_file(),
            "skill/profiler.md is referenced by the unity_run_states tool prompt but missing at {:?}",
            path
        );
    }
}
