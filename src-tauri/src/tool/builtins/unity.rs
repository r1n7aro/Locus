use std::sync::Arc;
use std::time::Duration;

use super::{make_exec, ToolDef, ToolResult};

// ─── unity_test_list / unity_test_run ──────────────────────────────────────

pub(super) fn unity_test_list() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_TEST_LIST);
    ToolDef {
        name: "unity_test_list".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path,
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_test_list' requires a selected Unity project working directory."
                                .to_string(),
                            is_error: true,
                        };
                    }
                };
                match crate::unity_bridge::unity_test_list(&project_path, &args).await {
                    Ok(output) => ToolResult {
                        output,
                        is_error: false,
                    },
                    Err(output) => ToolResult {
                        output,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn unity_test_run() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_TEST_RUN);
    ToolDef {
        name: "unity_test_run".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Test code is arbitrary project code and may modify assets or scenes.
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path,
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_test_run' requires a selected Unity project working directory."
                                .to_string(),
                            is_error: true,
                        };
                    }
                };
                let timeout_ms = args
                    .get("timeout_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(600_000)
                    .clamp(1_000, 3_600_000);
                match crate::unity_bridge::unity_test_run_controlled(
                    &project_path,
                    &args,
                    Duration::from_millis(timeout_ms),
                    ctx.cancel_rx,
                    ctx.progress,
                )
                .await
                {
                    Ok(snapshot) => {
                        let is_error = snapshot.status != "passed";
                        ToolResult {
                            output: serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| {
                                format!("Unity Test run {} {}", snapshot.run_id, snapshot.status)
                            }),
                            is_error,
                        }
                    }
                    Err(output) => ToolResult {
                        output,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── unity_set_play_mode ───────────────────────────────────────────────────

pub(super) fn unity_set_play_mode() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_SET_PLAY_MODE);
    ToolDef {
        name: "unity_set_play_mode".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Changes transient Editor state without modifying tracked project files.
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let mode = match args
                    .get("mode")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    Some(mode) => mode,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: mode".to_string(),
                            is_error: true,
                        };
                    }
                };
                let requested_status = match crate::unity_bridge::play_mode_target_status(mode) {
                    Ok(status) => status,
                    Err(output) => {
                        return ToolResult {
                            output,
                            is_error: true,
                        };
                    }
                };
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path.trim().to_string(),
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_set_play_mode' requires a selected Unity project working directory.".to_string(),
                            is_error: true,
                        };
                    }
                };

                let (connected, current_status, _scene) =
                    crate::unity_bridge::query_unity_status(&project_path).await;
                if !connected {
                    return ToolResult {
                        output: "Unity Editor not connected".to_string(),
                        is_error: true,
                    };
                }
                if current_status == requested_status {
                    return ToolResult {
                        output: crate::unity_bridge::format_play_mode_tool_result(mode, false),
                        is_error: false,
                    };
                }

                match crate::unity_bridge::set_editor_status(&project_path, requested_status).await
                {
                    Ok(()) => ToolResult {
                        output: crate::unity_bridge::format_play_mode_tool_result(mode, true),
                        is_error: false,
                    },
                    Err(error) => ToolResult {
                        output: format!("Failed to change Unity Editor mode: {error}"),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── unity_execute ───────────────────────────────────────────────────────────

pub(super) fn unity_execute() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_EXECUTE);
    ToolDef {
        name: "unity_execute".to_string(),
        description: format!(
            "Use `unity_set_play_mode` whenever the task only needs to start, resume, or stop Play Mode. Reserve `unity_execute` for C# operations that inspect or change Unity objects, assets, scenes, or editor data.\n\n{}",
            prompt.description
        ),
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
                let enable_non_public_access =
                    match crate::csharp_compile::resolve_tool_non_public_access(&args) {
                        Ok(value) => value,
                        Err(output) => {
                            return ToolResult {
                                output,
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

                match crate::unity_bridge::unity_execute_code_with_non_public_access(
                    &project_path,
                    &code,
                    enable_non_public_access,
                )
                .await
                {
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
                let enable_non_public_access =
                    match crate::csharp_compile::resolve_tool_non_public_access(&args) {
                        Ok(value) => value,
                        Err(output) => {
                            return ToolResult {
                                output,
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

                if let Err(error) = crate::unity_bridge::compile_run_states_with_non_public_access(
                    &project_path,
                    &args,
                    enable_non_public_access,
                )
                .await
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

                match crate::unity_bridge::unity_run_states_with_non_public_access(
                    &project_path,
                    &args,
                    enable_non_public_access,
                )
                .await
                {
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

// ─── unity_get_console_log ──────────────────────────────────────────────────

pub(super) fn unity_get_console_log() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_GET_CONSOLE_LOG);
    ToolDef {
        name: "unity_get_console_log".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let project_path = match ctx.working_dir {
                    Some(path) if !path.trim().is_empty() => path.trim().to_string(),
                    _ => {
                        return ToolResult {
                            output: "Tool 'unity_get_console_log' requires a selected Unity project working directory.".to_string(),
                            is_error: true,
                        };
                    }
                };
                let request = match serde_json::to_string(&args) {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Failed to serialize Console log request: {error}"),
                            is_error: true,
                        };
                    }
                };
                let response = match crate::unity_bridge::send_message(
                    &project_path,
                    "unity_get_console_log",
                    &request,
                )
                .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        return ToolResult {
                            output: error,
                            is_error: true,
                        };
                    }
                };
                if !response.ok {
                    return ToolResult {
                        output: response
                            .error
                            .unwrap_or_else(|| "Failed to read Unity Console".to_string()),
                        is_error: true,
                    };
                }

                let output = match serde_json::from_str::<serde_json::Value>(
                    response.message.as_deref().unwrap_or_default(),
                ) {
                    Ok(output) => output,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Failed to parse Unity Console response: {error}"),
                            is_error: true,
                        };
                    }
                };
                match serde_json::to_string_pretty(&output) {
                    Ok(output) => ToolResult {
                        output,
                        is_error: false,
                    },
                    Err(error) => ToolResult {
                        output: format!("Failed to serialize Unity Console logs: {error}"),
                        is_error: true,
                    },
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
