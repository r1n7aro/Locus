mod filesystem;
mod knowledge;
mod misc;
mod search;
mod shell;
mod unity;

use std::path::Path;
use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolExecutionContext, ToolRegistry, ToolResult};

pub use shell::shell_display_name;

pub fn register_all(registry: &mut ToolRegistry) {
    registry.register(filesystem::read());
    registry.register(filesystem::write());
    registry.register(filesystem::edit());
    registry.register(shell::bash());
    registry.register(search::grep());
    registry.register(unity::unity_asset_search());
    registry.register(misc::webfetch());
    registry.register(misc::todowrite());

    registry.register(filesystem::list());
    registry.register(unity::unity_execute());
    registry.register(unity::unity_run_states());
    registry.register(unity::unity_recompile());
    registry.register(unity::unity_ref_search());
    registry.register(unity::unity_yaml_list());
    registry.register(unity::unity_yaml_search());
    registry.register(unity::unity_yaml_read());
    registry.register(misc::ask());
    registry.register(misc::canvas());
    registry.register(knowledge::knowledge_list_tool());
    registry.register(knowledge::knowledge_query_tool());
    registry.register(knowledge::knowledge_read_tool());
    registry.register(knowledge::knowledge_create_tool());
    registry.register(knowledge::knowledge_delete_tool());
    registry.register(knowledge::knowledge_move_tool());
    registry.register(knowledge::knowledge_edit_tool());
    registry.register(config_query_tool());
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
