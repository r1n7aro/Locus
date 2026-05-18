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
    pub const UNITY_EXECUTE: &str = include_str!("../../tools/unity_execute.json");
    pub const UNITY_RUN_STATES: &str = include_str!("../../tools/unity_run_states.json");
    pub const UNITY_REF_SEARCH: &str = include_str!("../../tools/unity_ref_search.json");
    pub const UNITY_ASSET_SEARCH: &str = include_str!("../../tools/unity_asset_search.json");
    pub const UNITY_YAML_LIST: &str = include_str!("../../tools/unity_yaml_list.json");
    pub const UNITY_YAML_SEARCH: &str = include_str!("../../tools/unity_yaml_search.json");
    pub const UNITY_YAML_READ: &str = include_str!("../../tools/unity_yaml_read.json");
    pub const UNITY_RECOMPILE: &str = include_str!("../../tools/unity_recompile.json");
    pub const LIST: &str = include_str!("../../tools/list.json");
    pub const ASK: &str = include_str!("../../tools/ask.json");
    pub const CANVAS: &str = include_str!("../../tools/canvas.json");
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
            ("unity_execute", tools::UNITY_EXECUTE),
            ("unity_run_states", tools::UNITY_RUN_STATES),
            ("unity_ref_search", tools::UNITY_REF_SEARCH),
            ("unity_asset_search", tools::UNITY_ASSET_SEARCH),
            ("unity_yaml_list", tools::UNITY_YAML_LIST),
            ("unity_yaml_search", tools::UNITY_YAML_SEARCH),
            ("unity_yaml_read", tools::UNITY_YAML_READ),
            ("unity_recompile", tools::UNITY_RECOMPILE),
            ("list", tools::LIST),
            ("ask", tools::ASK),
            ("canvas", tools::CANVAS),
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
            ("config_query", tools::CONFIG_QUERY),
            ("tool_load", tools::TOOL_LOAD),
            ("tool_call", tools::TOOL_CALL),
        ];

        for (name, json_str) in tool_prompts {
            let prompt = parse_tool_prompt(json_str);
            assert_openai_compatible_tool_parameters(name, &prompt.parameters);
        }
    }
}
