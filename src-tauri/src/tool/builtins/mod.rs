mod filesystem;
mod knowledge;
mod misc;
mod search;
mod shell;
mod skill;
mod unity;

use std::path::Path;
use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolExecutionContext, ToolRegistry, ToolResult};

pub use shell::shell_display_name;

pub fn register_all(registry: &mut ToolRegistry) {
    registry.register_builtin(filesystem::read());
    registry.register_builtin(filesystem::write());
    registry.register_builtin(filesystem::edit());
    registry.register_builtin(shell::bash());
    registry.register_builtin(search::grep());
    registry.register_builtin(unity::unity_asset_search());
    registry.register_builtin(misc::web_fetch());
    registry.register_builtin(misc::todowrite());

    registry.register_builtin(filesystem::list());
    registry.register_builtin(unity::unity_execute());
    registry.register_builtin(unity::unity_run_states());
    registry.register_builtin(unity::unity_recompile());
    registry.register_builtin(unity::unity_ref_search());
    registry.register_builtin(unity::unity_yaml_list());
    registry.register_builtin(unity::unity_yaml_search());
    registry.register_builtin(unity::unity_yaml_read());
    registry.register_builtin(misc::ask());
    registry.register_builtin(misc::canvas());
    registry.register_builtin(knowledge::knowledge_list_tool());
    registry.register_builtin(knowledge::knowledge_query_tool());
    registry.register_builtin(knowledge::knowledge_read_tool());
    registry.register_builtin(knowledge::knowledge_create_tool());
    registry.register_builtin(knowledge::knowledge_delete_tool());
    registry.register_builtin(knowledge::knowledge_move_tool());
    registry.register_builtin(knowledge::knowledge_edit_tool());
    registry.register_builtin(skill::skill_create_tool());
    registry.register_builtin(skill::skill_reload_tool());
    registry.register_builtin(skill::skill_list_tool());
    registry.register_builtin(config_query_tool());
    registry.register_builtin(tool_load_tool());
    registry.register_builtin(tool_call_tool());
}

pub(super) fn should_skip_generated_root_entry(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };

    let Some(first_component) = relative.components().next() else {
        return false;
    };

    let name = first_component.as_os_str().to_string_lossy();
    let lower = name.trim().to_ascii_lowercase();

    matches!(
        lower.as_str(),
        "library" | "temp" | "obj" | "logs" | "usersettings" | "memorycaptures" | "recordings"
    ) || lower.starts_with("build")
}

fn config_query_tool() -> ToolDef {
    let execute: ToolExecuteFn = std::sync::Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: config_query tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CONFIG_QUERY);
    ToolDef {
        name: "config_query".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

fn tool_load_tool() -> ToolDef {
    let execute: ToolExecuteFn = std::sync::Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: tool_load tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::TOOL_LOAD);
    ToolDef {
        name: "tool_load".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

fn tool_call_tool() -> ToolDef {
    let execute: ToolExecuteFn = std::sync::Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: tool_call tool should be rewritten by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::TOOL_CALL);
    ToolDef {
        name: "tool_call".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute,
    }
}

fn make_exec<F>(f: F) -> ToolExecuteFn
where
    F: Fn(
            serde_json::Value,
            ToolExecutionContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolResult> + Send>>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

#[cfg(test)]
mod tests {
    use super::should_skip_generated_root_entry;
    use std::path::Path;

    #[test]
    fn generated_root_entry_detection_is_root_scoped() {
        let root = Path::new("C:/Project");

        assert!(should_skip_generated_root_entry(
            root,
            Path::new("C:/Project/Library/Artifacts")
        ));
        assert!(should_skip_generated_root_entry(
            root,
            Path::new("C:/Project/BuildPlayer/output.log")
        ));
        assert!(!should_skip_generated_root_entry(
            root,
            Path::new("C:/Project")
        ));
        assert!(!should_skip_generated_root_entry(
            root,
            Path::new("C:/Project/Assets/Scripts/BuildPipeline")
        ));
    }
}
