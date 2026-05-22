use super::{make_exec, ToolDef, ToolExecutionContext, ToolResult};

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
                let request = match serde_json::from_value::<crate::view::ViewCreateRequest>(args) {
                    Ok(request) => request,
                    Err(error) => {
                        return ToolResult {
                            output: format!("Error parsing view_create arguments: {}", error),
                            is_error: true,
                        }
                    }
                };

                match crate::view::create_view_sync(&working_dir, request) {
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
                match crate::view::open_view_window(app_handle, &working_dir, &view_id) {
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
                            }
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
                            }
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

pub(super) fn view_binding_read() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_BINDING_READ);
    ToolDef {
        name: "view_binding_read".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_binding_read") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request =
                    match serde_json::from_value::<crate::view::ViewBindingReadRequest>(args) {
                        Ok(request) => request,
                        Err(error) => {
                            return ToolResult {
                                output: format!(
                                    "Error parsing view_binding_read arguments: {}",
                                    error
                                ),
                                is_error: true,
                            }
                        }
                    };

                match crate::view::view_binding_read(&working_dir, request).await {
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

pub(super) fn view_binding_write() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_BINDING_WRITE);
    ToolDef {
        name: "view_binding_write".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_binding_write") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request =
                    match serde_json::from_value::<crate::view::ViewBindingWriteRequest>(args) {
                        Ok(request) => request,
                        Err(error) => {
                            return ToolResult {
                                output: format!(
                                    "Error parsing view_binding_write arguments: {}",
                                    error
                                ),
                                is_error: true,
                            }
                        }
                    };

                match crate::view::view_binding_write(&working_dir, request).await {
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

pub(super) fn view_binding_apply() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::VIEW_BINDING_APPLY);
    ToolDef {
        name: "view_binding_apply".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let working_dir = match working_dir_or_error(&ctx, "view_binding_apply") {
                    Ok(path) => path,
                    Err(result) => return result,
                };
                let request =
                    match serde_json::from_value::<crate::view::ViewBindingApplyRequest>(args) {
                        Ok(request) => request,
                        Err(error) => {
                            return ToolResult {
                                output: format!(
                                    "Error parsing view_binding_apply arguments: {}",
                                    error
                                ),
                                is_error: true,
                            }
                        }
                    };

                match crate::view::view_binding_apply(&working_dir, request).await {
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
