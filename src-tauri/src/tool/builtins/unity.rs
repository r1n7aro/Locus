use std::sync::Arc;

use super::{make_exec, ToolDef, ToolResult};

// ─── unity_execute ───────────────────────────────────────────────────────────

pub(super) fn unity_execute() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_EXECUTE);
    ToolDef {
        name: "unity_execute".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let code = match args.get("code").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: code".to_string(),
                            is_error: true,
                        };
                    }
                };

                let requested_status = match args
                    .get("request_editor_status")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(status) => status,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: request_editor_status".to_string(),
                            is_error: true,
                        };
                    }
                };

                if requested_status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED
                    || !crate::unity_bridge::is_known_editor_status(requested_status)
                {
                    return ToolResult {
                        output: format!(
                            "Invalid request_editor_status: '{}'. Allowed values: editing, playing, playing_paused.",
                            requested_status
                        ),
                        is_error: true,
                    };
                }

                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path.trim().to_string(),
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_execute' requires a selected Unity project working directory.".to_string(),
                            is_error: true,
                        }
                    }
                };

                let (connected, actual_status, _scene) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if !connected {
                    return ToolResult {
                        output: "Unity Editor not connected".to_string(),
                        is_error: true,
                    };
                }

                if actual_status != requested_status {
                    return ToolResult {
                        output: format!(
                            "Unity Editor status is \"{}\". `unity_execute` requires \"{}\".",
                            actual_status, requested_status
                        ),
                        is_error: true,
                    };
                }

                match crate::unity_bridge::unity_execute_code(&project_path, &code).await {
                    Ok(output) => {
                        let trimmed = output.trim();
                        ToolResult {
                            output: if trimmed.is_empty() {
                                "Code executed successfully (no output).".to_string()
                            } else {
                                trimmed.to_string()
                            },
                            is_error: false,
                        }
                    }
                    Err(e) => ToolResult {
                        output: e,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── unity_run_states ───────────────────────────────────────────────────────

pub(super) fn unity_run_states() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_RUN_STATES);
    ToolDef {
        name: "unity_run_states".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path,
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_run_states' requires a selected Unity project working directory.".to_string(),
                            is_error: true,
                        };
                    }
                };

                let requested_status = match args
                    .get("request_editor_status")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(status) => status,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: request_editor_status".to_string(),
                            is_error: true,
                        };
                    }
                };

                let (connected, _actual_status, _) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if !connected {
                    return ToolResult {
                        output: "Unity Editor not connected".to_string(),
                        is_error: true,
                    };
                }

                if let Err(error) =
                    crate::unity_bridge::compile_run_states(&project_path, &args).await
                {
                    return ToolResult {
                        output: error,
                        is_error: true,
                    };
                }

                let (connected, actual_status, _) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if !connected {
                    return ToolResult {
                        output: "Unity Editor not connected".to_string(),
                        is_error: true,
                    };
                }

                if actual_status != requested_status {
                    return ToolResult {
                        output: format!(
                            "Unity Editor status is \"{}\". `unity_run_states` requires \"{}\".",
                            actual_status, requested_status
                        ),
                        is_error: true,
                    };
                }

                match crate::unity_bridge::unity_run_states(&project_path, &args).await {
                    Ok(output) => ToolResult {
                        output: output.trim().to_string(),
                        is_error: false,
                    },
                    Err(e) => ToolResult {
                        output: e,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── unity_ref_search ──────────────────────────────────────────────────────

pub(super) fn unity_ref_search() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_REF_SEARCH);
    ToolDef {
        name: "unity_ref_search".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let Some(app_handle) = ctx.app_handle.as_ref() else {
                    return ToolResult {
                        output: "unity_ref_search requires the Locus app context.".to_string(),
                        is_error: true,
                    };
                };
                crate::agent::instance::AgentInstance::execute_unity_ref_search(app_handle, &args)
            })
        }),
    }
}

// ─── unity_asset_search ─────────────────────────────────────────────────────

pub(super) fn unity_asset_search() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_ASSET_SEARCH);
    ToolDef {
        name: "unity_asset_search".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let Some(app_handle) = ctx.app_handle.as_ref() else {
                    return ToolResult {
                        output: "unity_asset_search requires the Locus app context.".to_string(),
                        is_error: true,
                    };
                };
                crate::agent::instance::AgentInstance::execute_unity_asset_search(app_handle, &args)
            })
        }),
    }
}

// ─── unity_capture_viewport ─────────────────────────────────────────────────

pub(super) fn unity_capture_viewport() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_CAPTURE_VIEWPORT);
    ToolDef {
        name: "unity_capture_viewport".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        // Stays a stub: the real implementation returns images, which
        // ToolResult cannot carry. Both the agent loop and the MCP server
        // call AgentInstance::execute_unity_capture_viewport directly.
        execute: Arc::new(|_args, _ctx| {
            Box::pin(async {
                ToolResult {
                    output: "Error: unity_capture_viewport must be executed through the agent loop or the MCP server (its result carries images that ToolResult cannot).".to_string(),
                    is_error: true,
                }
            })
        }),
    }
}

// ─── Unity YAML tools ────────────────────────────────────────────────────────

/// Shared closure body for the three unity_yaml tools: resolve the app
/// handle + working dir from the execution context, then run the same
/// implementation the agent loop calls.
macro_rules! unity_yaml_tool_def {
    ($name:literal, $prompt:expr, $impl_fn:ident) => {{
        let prompt = crate::prompt::parse_tool_prompt($prompt);
        ToolDef {
            name: $name.to_string(),
            description: prompt.description,
            parameters: prompt.parameters,
            mutates_workspace: false,
            execute: make_exec(|args, ctx| {
                Box::pin(async move {
                    let Some(app_handle) = ctx.app_handle.as_ref() else {
                        return ToolResult {
                            output: concat!($name, " requires the Locus app context.").to_string(),
                            is_error: true,
                        };
                    };
                    let working_dir = match ctx.working_dir {
                        Some(ref wd) if !wd.trim().is_empty() => wd.trim().to_string(),
                        _ => {
                            return ToolResult {
                                output: concat!(
                                    "Tool '",
                                    $name,
                                    "' requires a selected Unity project working directory."
                                )
                                .to_string(),
                                is_error: true,
                            };
                        }
                    };
                    crate::agent::instance::AgentInstance::$impl_fn(
                        app_handle,
                        &working_dir,
                        &args,
                    )
                    .await
                })
            }),
        }
    }};
}

pub(super) fn unity_yaml_list() -> ToolDef {
    unity_yaml_tool_def!(
        "unity_yaml_list",
        crate::prompt::tools::UNITY_YAML_LIST,
        execute_unity_yaml_list
    )
}

pub(super) fn unity_yaml_search() -> ToolDef {
    unity_yaml_tool_def!(
        "unity_yaml_search",
        crate::prompt::tools::UNITY_YAML_SEARCH,
        execute_unity_yaml_search
    )
}

pub(super) fn unity_yaml_read() -> ToolDef {
    unity_yaml_tool_def!(
        "unity_yaml_read",
        crate::prompt::tools::UNITY_YAML_READ,
        execute_unity_yaml_read
    )
}

// ─── unity_hot_reload ────────────────────────────────────────────────────────

pub(super) fn unity_hot_reload() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_HOT_RELOAD);
    ToolDef {
        name: "unity_hot_reload".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Redirects methods in the running editor; tracked files already
        // changed through write/edit.
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path.trim().to_string(),
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_hot_reload' requires a selected Unity project working directory.".to_string(),
                            is_error: true,
                        };
                    }
                };

                let (connected, _status, _scene) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if !connected {
                    return ToolResult {
                        output: "Unity Editor not connected".to_string(),
                        is_error: true,
                    };
                }

                let paths = args
                    .get("paths")
                    .and_then(|value| value.as_array())
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_str())
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    });

                match crate::unity_hotreload::coordinator::hot_reload(&project_path, paths).await {
                    Ok(output) => ToolResult {
                        output,
                        is_error: false,
                    },
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── unity_recompile ─────────────────────────────────────────────────────────

pub(super) fn unity_recompile() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_RECOMPILE);
    ToolDef {
        name: "unity_recompile".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Triggers compilation only; doesn't change tracked source files.
        mutates_workspace: false,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let claimed_status = match args.get("editor_status").and_then(|v| v.as_str()) {
                    Some(s) => s.to_string(),
                    None => {
                        return ToolResult {
                            output: format!(
                                "Missing required parameter: editor_status. You must pass the current Unity Editor status ({}) exactly as shown in the Environment section.",
                                crate::unity_bridge::UNITY_EDITOR_STATUS_SCHEMA
                            ),
                            is_error: true,
                        };
                    }
                };

                if !crate::unity_bridge::is_known_editor_status(&claimed_status) {
                    return ToolResult {
                        output: format!(
                            "Invalid editor_status: \"{}\". Allowed values: {}.",
                            claimed_status,
                            crate::unity_bridge::UNITY_EDITOR_STATUS_SCHEMA
                        ),
                        is_error: true,
                    };
                }

                let project_path = match args.get("project_path").and_then(|v| v.as_str()) {
                    Some(path) if !path.trim().is_empty() => path.trim().to_string(),
                    _ => {
                        return ToolResult {
                            output: "Missing required parameter: project_path".to_string(),
                            is_error: true,
                        };
                    }
                };

                // Verify editor_status matches actual Unity state
                let (_connected, actual_status, _scene) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if claimed_status != actual_status {
                    return ToolResult {
                        output: format!(
                            "editor_status mismatch: you claimed \"{}\", but the actual editor status is \"{}\". Re-read the current editor state and try again.",
                            claimed_status, actual_status
                        ),
                        is_error: true,
                    };
                }

                if actual_status == crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED {
                    return ToolResult {
                        output: "Unity Editor status is \"disconnected\". `unity_recompile` is unavailable until the Editor reconnects.".to_string(),
                        is_error: true,
                    };
                }

                if crate::unity_bridge::is_play_mode_status(actual_status) {
                    return ToolResult {
                        output: format!(
                            "Unity Editor status is \"{}\". Exit Play Mode before calling `unity_recompile`.",
                            actual_status
                        ),
                        is_error: true,
                    };
                }

                match crate::unity_bridge::recompile_and_wait(&project_path).await {
                    Ok(msg) => ToolResult {
                        output: msg,
                        is_error: false,
                    },
                    Err(e) => ToolResult {
                        output: format!("Compilation failed:\n{}", e),
                        is_error: true,
                    },
                }
            })
        }),
    }
}
