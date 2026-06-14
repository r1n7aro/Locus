//! Semantic C# navigation tools backed by the Roslyn language server
//! (`crate::csharp_lsp`). These tools are only offered to the agent while the
//! C# code analysis feature is enabled — see
//! `AgentInstance::resolve_effective_tool_names`.

use super::{make_exec, ToolDef, ToolResult};

pub(super) fn require_workspace(ctx: &super::ToolExecutionContext) -> Result<String, ToolResult> {
    match ctx.working_dir.as_deref().map(str::trim) {
        Some(dir) if !dir.is_empty() => Ok(dir.to_string()),
        _ => Err(ToolResult {
            output: "This tool requires a selected workspace directory.".to_string(),
            is_error: true,
        }),
    }
}

pub(super) fn string_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolResult> {
    match args.get(key).and_then(|v| v.as_str()).map(str::trim) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(ToolResult {
            output: format!("Missing required parameter: {key}"),
            is_error: true,
        }),
    }
}

/// Optional 1-based line hint. Position-based queries tolerate missing or
/// stale hints (see `csharp_lsp::locate_symbol_position`), so absence is
/// fine. Agents express "no line" as `0` rather than omitting the key, so
/// zero and negative values mean "no hint" too; only a non-integer errors.
fn line_arg(args: &serde_json::Value) -> Result<Option<u32>, ToolResult> {
    match args.get("line") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match value.as_i64() {
            Some(line) if line >= 1 => Ok(Some(line as u32)),
            Some(_) => Ok(None),
            None => Err(ToolResult {
                output: "Invalid parameter: line (1-based integer, or 0 for no hint)".to_string(),
                is_error: true,
            }),
        },
    }
}

/// One-line preamble telling the agent where a forgiving line hint actually
/// landed, so its mental line numbers self-correct.
fn anchor_note(symbol: &str, anchor: &crate::csharp_lsp::SymbolAnchor) -> String {
    match anchor.requested_line {
        Some(requested) if anchor.adjusted() => format!(
            "(Note: '{symbol}' was not on line {requested}; used the nearest occurrence at line {}.)\n\n",
            anchor.line
        ),
        None => format!("(Resolved '{symbol}' at line {}.)\n\n", anchor.line),
        _ => String::new(),
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
                                    "{}No references to '{symbol}' found{}.",
                                    anchor_note(&symbol, &result.anchor),
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
                            "{}{} reference{} to '{symbol}'{}\n\n",
                            anchor_note(&symbol, &result.anchor),
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
                    Ok((locations, anchor)) => {
                        if locations.is_empty() {
                            return ToolResult {
                                output: format!(
                                    "{}No definition found for '{symbol}'. It may be defined in a compiled assembly.",
                                    anchor_note(&symbol, &anchor)
                                ),
                                is_error: false,
                            };
                        }
                        ToolResult {
                            output: format!(
                                "{}Definition of '{symbol}'\n\n{}",
                                anchor_note(&symbol, &anchor),
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

// ─── code_diagnostics ───────────────────────────────────────────────────────

/// Append the project string-reference findings (merged from the former
/// unity_project_refs_check tool) to a code_diagnostics file-scope output.
/// Silent when the file contains no such references at all.
fn push_project_refs_section(
    output: &mut String,
    report: &super::code_unity::ProjectRefsReport,
    max_ref_problems: usize,
) {
    let total_refs: usize = report.counts.iter().sum();
    if total_refs == 0 && report.problems.is_empty() {
        return;
    }
    let counts_text = format!(
        "{} tag, {} layer, {} scene, {} resource, {} input",
        report.counts[0], report.counts[1], report.counts[2], report.counts[3], report.counts[4]
    );
    if report.problems.is_empty() {
        output.push_str(&format!(
            "\n\nProject string refs: all {total_refs} resolve ({counts_text})."
        ));
    } else {
        let shown = report.problems.len().min(max_ref_problems);
        output.push_str(&format!(
            "\n\nProject string refs: {} problem{}{} ({counts_text} checked)\n",
            report.problems.len(),
            if report.problems.len() == 1 { "" } else { "s" },
            if shown < report.problems.len() {
                format!(", showing first {shown}")
            } else {
                String::new()
            }
        ));
        for problem in report.problems.iter().take(max_ref_problems) {
            output.push_str(&format!(
                "  {}:{} — {}\n",
                problem.path, problem.line, problem.message
            ));
        }
    }
    if report.unvalidated_axis_refs > 0 {
        output.push_str(&format!(
            "\n  ({} Input.GetAxis/GetButton ref(s) not validated: InputManager.asset has no axes.)",
            report.unvalidated_axis_refs
        ));
    }
}

fn severity_rank(label: &str) -> Option<u8> {
    match label {
        "error" => Some(1),
        "warning" => Some(2),
        "info" => Some(3),
        "hint" => Some(4),
        _ => None,
    }
}

fn format_code_diagnostic_output(
    scope_label: &str,
    min_severity: u8,
    max_results: usize,
    all: Vec<crate::csharp_lsp::CodeDiagnostic>,
) -> String {
    let below_threshold = all.iter().filter(|d| d.severity > min_severity).count();
    let matching: Vec<_> = all
        .into_iter()
        .filter(|d| d.severity <= min_severity)
        .collect();
    if matching.is_empty() {
        let mut output = format!(
            "No {} diagnostics in {scope_label}.",
            match min_severity {
                1 => "error",
                2 => "error/warning",
                3 => "error/warning/info",
                _ => "",
            }
        );
        if below_threshold > 0 {
            output.push_str(&format!(
                " ({below_threshold} below the severity threshold.)"
            ));
        }
        return output;
    }

    let errors = matching.iter().filter(|d| d.severity == 1).count();
    let warnings = matching.iter().filter(|d| d.severity == 2).count();
    let others = matching.len() - errors - warnings;
    let shown = matching.len().min(max_results);
    let mut output = format!(
        "{} diagnostic{} in {scope_label}: {errors} error{}, {warnings} warning{}{}{}\n\n",
        matching.len(),
        if matching.len() == 1 { "" } else { "s" },
        if errors == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" },
        if others > 0 {
            format!(", {others} info/hint")
        } else {
            String::new()
        },
        if shown < matching.len() {
            format!(" (showing first {shown})")
        } else {
            String::new()
        }
    );
    let mut current_file = "";
    for diagnostic in matching.iter().take(max_results) {
        if diagnostic.path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(&diagnostic.path);
            output.push('\n');
            current_file = &diagnostic.path;
        }
        output.push_str(&format!(
            "  {}:{} {}{}: {}\n",
            diagnostic.line,
            diagnostic.column,
            crate::csharp_lsp::severity_label(diagnostic.severity),
            diagnostic
                .code
                .as_deref()
                .map(|code| format!(" {code}"))
                .unwrap_or_default(),
            diagnostic.message.replace('\n', " ")
        ));
    }
    output
}

pub(super) async fn file_diagnostics_output(
    workspace: &str,
    file_path: &str,
    min_severity: u8,
    max_results: usize,
    max_project_ref_problems: usize,
) -> Result<String, String> {
    let all = crate::csharp_lsp::document_diagnostics(workspace, file_path).await?;
    let mut output = format_code_diagnostic_output(file_path, min_severity, max_results, all);

    // File scope also validates project string references
    // (tags/layers/scenes/Resources/Input) — these compile fine and only fail
    // at runtime, so they belong in the same post-edit verification pass.
    let refs_workspace = workspace.to_string();
    let refs_file_path = file_path.to_string();
    let scanned = tokio::task::spawn_blocking(move || {
        super::code_unity::scan_project_refs(&refs_workspace, Some(&refs_file_path))
    })
    .await;
    if let Ok(Ok(report)) = scanned {
        push_project_refs_section(&mut output, &report, max_project_ref_problems);
    }

    Ok(output)
}

pub(super) fn code_diagnostics() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CODE_DIAGNOSTICS);
    ToolDef {
        name: "code_diagnostics".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let workspace = match require_workspace(&ctx) {
                    Ok(dir) => dir,
                    Err(result) => return result,
                };
                let file_path = args
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);
                let scope = match args
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    Some("file") => "file",
                    Some("workspace") => "workspace",
                    Some(other) => {
                        return ToolResult {
                            output: format!(
                                "Invalid scope '{other}'. Must be 'file' or 'workspace'."
                            ),
                            is_error: true,
                        };
                    }
                    None => {
                        if file_path.is_some() {
                            "file"
                        } else {
                            "workspace"
                        }
                    }
                };
                let min_severity = match args
                    .get("min_severity")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    Some(label) => match severity_rank(label) {
                        Some(rank) => rank,
                        None => {
                            return ToolResult {
                                output: format!(
                                    "Invalid min_severity '{label}'. Must be 'error', 'warning', 'info' or 'hint'."
                                ),
                                is_error: true,
                            };
                        }
                    },
                    None => 2,
                };
                let max_results = args
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.clamp(1, 400) as usize)
                    .unwrap_or(100);

                let output = match scope {
                    "file" => {
                        let Some(file_path) = file_path.as_deref() else {
                            return ToolResult {
                                output: "scope 'file' requires file_path.".to_string(),
                                is_error: true,
                            };
                        };
                        match file_diagnostics_output(
                            &workspace,
                            file_path,
                            min_severity,
                            max_results,
                            50,
                        )
                        .await
                        {
                            Ok(output) => output,
                            Err(message) => {
                                return ToolResult {
                                    output: message,
                                    is_error: true,
                                };
                            }
                        }
                    }
                    _ => {
                        let all = match crate::csharp_lsp::workspace_diagnostics(&workspace).await {
                            Ok(diagnostics) => diagnostics,
                            Err(message) => {
                                return ToolResult {
                                    output: message,
                                    is_error: true,
                                };
                            }
                        };
                        format_code_diagnostic_output("workspace", min_severity, max_results, all)
                    }
                };

                ToolResult {
                    output,
                    is_error: false,
                }
            })
        }),
    }
}

// ─── code_hover ─────────────────────────────────────────────────────────────

pub(super) fn code_hover() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::CODE_HOVER);
    ToolDef {
        name: "code_hover".to_string(),
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

                match crate::csharp_lsp::hover(&workspace, &file_path, line, &symbol).await {
                    Ok((Some(contents), anchor)) => ToolResult {
                        output: format!(
                            "{}Hover for '{symbol}' ({file_path}:{})\n\n{contents}",
                            anchor_note(&symbol, &anchor),
                            anchor.line
                        ),
                        is_error: false,
                    },
                    Ok((None, anchor)) => ToolResult {
                        output: format!(
                            "No hover information for '{symbol}' at {file_path}:{}.",
                            anchor.line
                        ),
                        is_error: false,
                    },
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
                            // Roslyn's containerName already starts with "in"
                            // for members ("in Foo (project Bar)") but not for
                            // types ("project Bar") — avoid doubling it.
                            let container = symbol
                                .container
                                .as_deref()
                                .map(|c| {
                                    let c = c.trim();
                                    if c.to_ascii_lowercase().starts_with("in ") {
                                        format!(" {c}")
                                    } else {
                                        format!(" in {c}")
                                    }
                                })
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
