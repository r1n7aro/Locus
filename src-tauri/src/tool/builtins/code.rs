//! Semantic C# navigation tools backed by the Roslyn language server
//! (`crate::csharp_lsp`). These tools are only offered to the agent while the
//! C# code analysis feature is enabled — see
//! `AgentInstance::resolve_effective_tool_names`.

use super::{make_exec, ToolDef, ToolResult};

fn require_workspace(ctx: &super::ToolExecutionContext) -> Result<String, ToolResult> {
    match ctx.working_dir.as_deref().map(str::trim) {
        Some(dir) if !dir.is_empty() => Ok(dir.to_string()),
        _ => Err(ToolResult {
            output: "This tool requires a selected workspace directory.".to_string(),
            is_error: true,
        }),
    }
}

fn string_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolResult> {
    match args.get(key).and_then(|v| v.as_str()).map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(ToolResult {
            output: format!("Missing required parameter: {key}"),
            is_error: true,
        }),
    }
}

fn line_arg(args: &serde_json::Value) -> Result<u32, ToolResult> {
    match args.get("line").and_then(|v| v.as_u64()) {
        Some(value) if value >= 1 => Ok(value as u32),
        _ => Err(ToolResult {
            output: "Missing or invalid parameter: line (1-based integer)".to_string(),
            is_error: true,
        }),
    }
}

fn format_locations(locations: &[crate::csharp_lsp::CodeLocation]) -> String {
    let mut output = String::new();
    let mut current_file = "";
    for location in locations {
        if location.path != current_file {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&location.path);
            output.push('\n');
            current_file = &location.path;
        }
        if location.line > 0 {
            output.push_str(&format!("  {}: {}\n", location.line, location.text));
        } else {
            output.push_str("  (external)\n");
        }
    }
    output
}

// ─── code_find_references ───────────────────────────────────────────────────

pub(super) fn code_find_references() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CODE_FIND_REFERENCES);
    ToolDef {
        name: "code_find_references".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let workspace = match require_workspace(&ctx) {
                    Ok(dir) => dir,
                    Err(result) => return result,
                };
                let file_path = match string_arg(&args, "file_path") {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let line = match line_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let symbol = match string_arg(&args, "symbol") {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let include_declaration = args
                    .get("include_declaration")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                match crate::csharp_lsp::find_references(
                    &workspace,
                    &file_path,
                    line,
                    &symbol,
                    include_declaration,
                )
                .await
                {
                    Ok(result) => {
                        if result.locations.is_empty() {
                            return ToolResult {
                                output: format!(
                                    "No references to '{symbol}' found{}.",
                                    if include_declaration {
                                        ""
                                    } else {
                                        " (declaration excluded)"
                                    }
                                ),
                                is_error: false,
                            };
                        }
                        let mut output = format!(
                            "{} reference{} to '{symbol}'{}\n\n",
                            result.locations.len(),
                            if result.locations.len() == 1 { "" } else { "s" },
                            if include_declaration {
                                " (declaration included)"
                            } else {
                                ""
                            }
                        );
                        output.push_str(&format_locations(&result.locations));
                        if result.truncated {
                            output.push_str("\n(Results truncated; narrow the query.)\n");
                        }
                        ToolResult {
                            output,
                            is_error: false,
                        }
                    }
                    Err(message) => ToolResult {
                        output: message,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── code_goto_definition ───────────────────────────────────────────────────

pub(super) fn code_goto_definition() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CODE_GOTO_DEFINITION);
    ToolDef {
        name: "code_goto_definition".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let workspace = match require_workspace(&ctx) {
                    Ok(dir) => dir,
                    Err(result) => return result,
                };
                let file_path = match string_arg(&args, "file_path") {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let line = match line_arg(&args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let symbol = match string_arg(&args, "symbol") {
                    Ok(value) => value,
                    Err(result) => return result,
                };

                match crate::csharp_lsp::goto_definition(&workspace, &file_path, line, &symbol)
                    .await
                {
                    Ok(locations) => {
                        if locations.is_empty() {
                            return ToolResult {
                                output: format!(
                                    "No definition found for '{symbol}'. It may be defined in a compiled assembly."
                                ),
                                is_error: false,
                            };
                        }
                        ToolResult {
                            output: format!(
                                "Definition of '{symbol}'\n\n{}",
                                format_locations(&locations)
                            ),
                            is_error: false,
                        }
                    }
                    Err(message) => ToolResult {
                        output: message,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── code_symbol_search ─────────────────────────────────────────────────────

pub(super) fn code_symbol_search() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CODE_SYMBOL_SEARCH);
    ToolDef {
        name: "code_symbol_search".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let workspace = match require_workspace(&ctx) {
                    Ok(dir) => dir,
                    Err(result) => return result,
                };
                let query = match string_arg(&args, "query") {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let limit = args
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.clamp(1, 200) as usize)
                    .unwrap_or(50);

                match crate::csharp_lsp::workspace_symbols(&workspace, &query, limit).await {
                    Ok(symbols) => {
                        if symbols.is_empty() {
                            return ToolResult {
                                output: format!("No symbols matching '{query}'."),
                                is_error: false,
                            };
                        }
                        let mut output = format!(
                            "{} symbol{} matching '{query}'\n\n",
                            symbols.len(),
                            if symbols.len() == 1 { "" } else { "s" }
                        );
                        for symbol in &symbols {
                            let container = symbol
                                .container
                                .as_deref()
                                .map(|c| format!(" in {c}"))
                                .unwrap_or_default();
                            let location = if symbol.line > 0 {
                                format!("{}:{}", symbol.path, symbol.line)
                            } else {
                                symbol.path.clone()
                            };
                            output.push_str(&format!(
                                "{} ({}{}) — {}\n",
                                symbol.name, symbol.kind, container, location
                            ));
                        }
                        ToolResult {
                            output,
                            is_error: false,
                        }
                    }
                    Err(message) => ToolResult {
                        output: message,
                        is_error: true,
                    },
                }
            })
        }),
    }
}
