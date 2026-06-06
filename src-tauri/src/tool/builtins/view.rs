use super::{make_exec, ToolDef, ToolExecutionContext, ToolResult};
use std::sync::Arc;
use tauri::Manager;

fn working_dir_or_error(ctx: &ToolExecutionContext, tool_name: &str) -> Result<String, ToolResult> {
    match ctx.working_dir.as_deref().map(str::trim) {
        Some(path) if !path.is_empty() => Ok(path.to_string()),
        _ => Err(ToolResult {
            output: format!(
                "Tool '{}' requires a selected Unity project working directory.",
                tool_name
            ),
            is_error: true,
        }),
    }
}

fn json_output<T: serde::Serialize>(value: &T) -> ToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(output) => ToolResult {
            output,
            is_error: false,
        },
        Err(error) => ToolResult {
            output: format!("Failed to serialize tool output: {}", error),
            is_error: true,
        },
    }
}

fn view_id_arg(args: &serde_json::Value) -> Result<String, ToolResult> {
    args.get("viewId")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolResult {
            output: "Missing required parameter: viewId".to_string(),
            is_error: true,
        })
}

fn app_handle_or_error(
    ctx: &ToolExecutionContext,
    tool_name: &str,
) -> Result<tauri::AppHandle, ToolResult> {
    ctx.app_handle.clone().ok_or_else(|| ToolResult {
        output: format!("Tool '{}' requires the Locus desktop runtime.", tool_name),
        is_error: true,
    })
}

async fn request_view_automation_tool(
    args: serde_json::Value,
    ctx: ToolExecutionContext,
    tool_name: &str,
    kind: &str,
    default_timeout_ms: u64,
) -> ToolResult {
    let working_dir = match working_dir_or_error(&ctx, tool_name) {
        Ok(path) => path,
        Err(result) => return result,
    };
    let view_id = match view_id_arg(&args) {
        Ok(value) => value,
        Err(result) => return result,
    };
    if let Err(error) = crate::view::read_view_sync(&working_dir, &view_id) {
        return ToolResult {
            output: error,
            is_error: true,
        };
    }
    let app_handle = match app_handle_or_error(&ctx, tool_name) {
        Ok(value) => value,
        Err(result) => return result,
    };
    let timeout_ms = args
        .get("timeoutMs")
        .or_else(|| args.get("timeout_ms"))
        .and_then(|value| value.as_u64())
        .unwrap_or(default_timeout_ms);
    match crate::view::request_view_automation(&app_handle, &view_id, kind, args, timeout_ms).await
    {
        Ok(result) => json_output(&result),
        Err(error) => ToolResult {
            output: error,
            is_error: true,
        },
    }
}

pub(super) fn view_create() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_CREATE);
    ToolDef {
        name: "view_create".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_create") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let (request, temporary) = match crate::view::parse_view_create_request(args) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Error parsing view_create arguments: {}", error),
                            is_error: true,
                        };
                    }
                };

                match crate::view::create_view_sync_with_scope(&working_dir, request, temporary) {
                    Ok(detail) => {
                        if let Some(app_handle) = ctx.app_handle.as_ref() {
                            crate::view::emit_view_reload(app_handle, &detail.summary);
                        }
                        json_output(&serde_json::json!({
                            "summary": detail.summary,
                            "manifest": detail.manifest
                        }))
                    }
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_list() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_LIST);
    ToolDef {
        name: "view_list".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|_args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_list") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                match crate::view::list_views_sync(&working_dir) {
                    Ok(views) => json_output(&views),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_reload() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_RELOAD);
    ToolDef {
        name: "view_reload".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_reload") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let view_id = match view_id_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                match crate::view::reload_view_sync(&working_dir, &view_id) {
                    Ok(summary) => json_output(&summary),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_run() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_RUN);
    ToolDef {
        name: "view_run".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_run") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let view_id = match view_id_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let Some(app_handle) = ctx.app_handle.as_ref() else {
                    return ToolResult {
                        output: "Tool 'view_run' requires the Locus desktop runtime.".to_string(),
                        is_error: true,
                    };
                };
                let app_handle = app_handle.clone();
                let view_windows_above_main = app_handle
                    .try_state::<Arc<crate::config::AppConfig>>()
                    .map(|config| config.view_windows_above_main_enabled())
                    .unwrap_or(false);
                let view_open_in_existing_window = app_handle
                    .try_state::<Arc<crate::config::AppConfig>>()
                    .map(|config| config.view_open_in_existing_window_enabled())
                    .unwrap_or(true);
                match crate::view::open_view_window(
                    &app_handle,
                    &working_dir,
                    &view_id,
                    view_windows_above_main,
                    view_open_in_existing_window,
                )
                .await
                {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_compile_script() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_COMPILE_SCRIPT);
    ToolDef {
        name: "view_compile_script".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_compile_script") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request =
                    match serde_json::from_value::<crate::view::ViewCompileScriptRequest>(args) {
                        Ok(request) => request,
                        Err(error) => {
                            return ToolResult {
                                output: format!(
                                    "Error parsing view_compile_script arguments: {}",
                                    error
                                ),
                                is_error: true,
                            };
                        }
                    };

                match crate::view::compile_view_script(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_call_script() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_CALL_SCRIPT);
    ToolDef {
        name: "view_call_script".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_call_script") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request =
                    match serde_json::from_value::<crate::view::ViewCallScriptRequest>(args) {
                        Ok(request) => request,
                        Err(error) => {
                            return ToolResult {
                                output: format!(
                                    "Error parsing view_call_script arguments: {}",
                                    error
                                ),
                                is_error: true,
                            };
                        }
                    };

                match crate::view::call_view_script(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_property_read() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_PROPERTY_READ);
    ToolDef {
        name: "view_property_read".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_property_read") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request = match serde_json::from_value::<
                    crate::unity_serialized_property::UnitySerializedPropertyReadRequest,
                >(args)
                {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Error parsing view_property_read arguments: {}",
                                error
                            ),
                            is_error: true,
                        };
                    }
                };

                match crate::unity_serialized_property::read(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_property_discover() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_PROPERTY_DISCOVER);
    ToolDef {
        name: "view_property_discover".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_property_discover") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request = match serde_json::from_value::<
                    crate::unity_serialized_property::UnitySerializedPropertyDiscoverRequest,
                >(args)
                {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Error parsing view_property_discover arguments: {}",
                                error
                            ),
                            is_error: true,
                        };
                    }
                };

                match crate::unity_serialized_property::discover(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_property_write() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_PROPERTY_WRITE);
    ToolDef {
        name: "view_property_write".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_property_write") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request = match serde_json::from_value::<
                    crate::unity_serialized_property::UnitySerializedPropertyWriteRequest,
                >(args)
                {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Error parsing view_property_write arguments: {}",
                                error
                            ),
                            is_error: true,
                        };
                    }
                };

                match crate::unity_serialized_property::write(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_property_apply() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_PROPERTY_APPLY);
    ToolDef {
        name: "view_property_apply".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_property_apply") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request = match serde_json::from_value::<
                    crate::unity_serialized_property::UnitySerializedPropertyApplyRequest,
                >(args)
                {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!(
                                "Error parsing view_property_apply arguments: {}",
                                error
                            ),
                            is_error: true,
                        };
                    }
                };

                match crate::unity_serialized_property::apply(&working_dir, request).await {
                    Ok(result) => json_output(&result),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_snapshot() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_SNAPSHOT);
    ToolDef {
        name: "view_snapshot".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                request_view_automation_tool(args, ctx, "view_snapshot", "snapshot", 5_000).await
            })
        }),
    }
}

pub(super) fn view_action() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_ACTION);
    ToolDef {
        name: "view_action".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                request_view_automation_tool(args, ctx, "view_action", "action", 5_000).await
            })
        }),
    }
}

pub(super) fn view_wait() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_WAIT);
    ToolDef {
        name: "view_wait".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                request_view_automation_tool(args, ctx, "view_wait", "wait", 10_000).await
            })
        }),
    }
}

pub(super) fn view_debug_eval() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_DEBUG_EVAL);
    ToolDef {
        name: "view_debug_eval".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                request_view_automation_tool(args, ctx, "view_debug_eval", "debugEval", 5_000).await
            })
        }),
    }
}

pub(super) fn view_console_read() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_CONSOLE_READ);
    ToolDef {
        name: "view_console_read".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_console_read") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let view_id = match view_id_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let limit = args
                    .get("limit")
                    .and_then(|value| value.as_u64())
                    .map(|value| value as usize);
                let level = args
                    .get("level")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let contains = args
                    .get("contains")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_ascii_lowercase());
                let request = crate::view::ViewFrontendLogReadRequest { view_id, limit };
                match crate::view::read_view_frontend_log_sync(&working_dir, request) {
                    Ok(entries) => {
                        let entries = entries
                            .into_iter()
                            .filter(|entry| {
                                level
                                    .as_deref()
                                    .map(|level| entry.level.eq_ignore_ascii_case(level))
                                    .unwrap_or(true)
                            })
                            .filter(|entry| {
                                contains
                                    .as_deref()
                                    .map(|needle| {
                                        entry.message.to_ascii_lowercase().contains(needle)
                                    })
                                    .unwrap_or(true)
                            })
                            .collect::<Vec<_>>();
                        json_output(&entries)
                    }
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

pub(super) fn view_capture() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_CAPTURE);
    ToolDef {
        name: "view_capture".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_capture") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let view_id = match view_id_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                if let Err(error) = crate::view::read_view_sync(&working_dir, &view_id) {
                    return ToolResult {
                        output: error,
                        is_error: true,
                    };
                }
                let app_handle = match app_handle_or_error(&ctx, "view_capture") {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                match crate::view::capture_view_window(&app_handle, &view_id).await {
                    Ok(capture) => json_output(&serde_json::json!({
                        "status": "captured",
                        "viewId": capture.view_id,
                        "windowLabel": capture.window_label,
                        "format": capture.format,
                        "mimeType": capture.mime_type,
                        "width": capture.width,
                        "height": capture.height,
                        "byteSize": capture.byte_size,
                        "image": "captured"
                    })),
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}
