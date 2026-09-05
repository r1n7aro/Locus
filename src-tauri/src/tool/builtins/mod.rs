mod agent;
mod code;
mod code_unity;
mod filesystem;
mod knowledge;
mod mcp;
mod misc;
mod plugin;
mod python;
mod read_outline;
mod search;
mod search_core;
mod shell;
mod skill;
mod unity;
mod view;

use std::path::Path;
use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolExecutionContext, ToolLoadMode, ToolRegistry, ToolResult};

pub(crate) use python::is_readonly as python_is_readonly;
pub use shell::{powershell_runtime_env_prompt, shell_display_name};

pub fn register_all(registry: &mut ToolRegistry) {
    registry.register_builtin(filesystem::read());
    registry.register_builtin(filesystem::write());
    registry.register_builtin(filesystem::edit());
    registry.register_builtin(shell::bash());
    registry.register_builtin(python::python());
    registry.register_builtin(search::grep());
    registry.register_builtin(unity::unity_asset_search());
    registry.register_builtin(misc::web_fetch());
    registry.register_builtin(misc::todowrite());

    registry.register_builtin(filesystem::list());
    registry.register_builtin(unity::unity_lock());
    registry.register_builtin(unity::unity_release());
    registry.register_builtin(unity::unity_set_play_mode());
    registry.register_builtin(unity::unity_execute());
    registry.register_builtin(unity::unity_run_states());
    registry.register_builtin(unity::unity_capture_viewport());
    registry.register_builtin(unity::unity_get_console_log());
    registry.register_builtin(unity::unity_test_list());
    registry.register_builtin(unity::unity_test_run());
    registry.register_builtin(unity::unity_recompile());
    registry.register_builtin(unity::unity_hot_reload());
    registry.register_builtin(unity::unity_ref_search());
    registry.register_builtin(code::code_find_references());
    registry.register_builtin(code::code_goto_definition());
    registry.register_builtin(code::code_symbol_search());
    registry.register_builtin(code::code_diagnostics());
    registry.register_builtin(code::code_hover());
    registry.register_builtin(code_unity::unity_code_usages());
    registry.register_builtin(unity::unity_yaml_search());
    registry.register_builtin(unity::unity_yaml_read());
    registry.register_builtin(misc::ask());
    registry.register_builtin(knowledge::knowledge_query_tool());
    registry
        .register_builtin_with_load_mode(skill::create_skill_package_tool(), ToolLoadMode::Skill);
    registry.register_builtin(skill::skill_list_tool());
    registry.register_builtin_with_load_mode(agent::agent_reload(), ToolLoadMode::Skill);
    registry.register_builtin(mcp::mcp_reload_tool());
    registry.register_builtin_with_load_mode(plugin::plugin_list(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(plugin::plugin_search(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(plugin::plugin_install(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(plugin::plugin_set_enabled(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(plugin::plugin_uninstall(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(plugin::plugin_export(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_create(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_list(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_reload(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_run(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_compile_script(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_call_script(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_property_read(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_property_discover(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_property_write(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_property_apply(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_capture(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_snapshot(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_action(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_wait(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_console_read(), ToolLoadMode::Skill);
    registry.register_builtin_with_load_mode(view::view_debug_eval(), ToolLoadMode::Skill);
    registry.register_builtin(config_query_tool());
    registry.register_builtin(tool_load_tool());
    registry.register_builtin(tool_call_tool());
    registry.register_builtin(exit_plan_mode_tool());
}

pub(super) fn should_skip_generated_root_entry(root: &Path, path: &Path) -> bool {
    search_core::should_skip_generated_root_entry(root, path)
}

/// Only offered to the LLM while the session is in plan mode (the agent loop
/// appends it to the request tool list); execution is intercepted by the
/// agent loop, which owns the approval dialog and the plan-mode state flip.
fn exit_plan_mode_tool() -> ToolDef {
    let execute: ToolExecuteFn = std::sync::Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: exit_plan_mode tool should be intercepted by agent loop"
                    .to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::EXIT_PLAN_MODE);
    ToolDef {
        name: "exit_plan_mode".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
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
        mutates_workspace: false,
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
        mutates_workspace: false,
        execute,
    }
}

fn tool_call_tool() -> ToolDef {
    let execute: ToolExecuteFn = std::sync::Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: tool_call tool should be dispatched by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::TOOL_CALL);
    ToolDef {
        name: "tool_call".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
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
