use std::ffi::OsString;

use super::misc::truncate_utf8_middle;
use super::shell::{decode_console_bytes, run_captured_command_with_input};
use super::{make_exec, ToolDef, ToolResult};
use crate::process_util::{async_command, ProcessOwner};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 1_800_000;

const HELP_OVERVIEW: &str = include_str!("../../../../prompt/python-sdk/overview.md");
const HELP_AGENTS: &str = include_str!("../../../../prompt/python-sdk/agents.md");
const HELP_SESSIONS: &str = include_str!("../../../../prompt/python-sdk/sessions.md");
const HELP_TOOLS: &str = include_str!("../../../../prompt/python-sdk/tools.md");
const HELP_TASKS: &str = include_str!("../../../../prompt/python-sdk/tasks.md");
const HELP_UNITY: &str = include_str!("../../../../prompt/python-sdk/unity.md");
const HELP_CALLBACKS: &str = include_str!("../../../../prompt/python-sdk/callbacks.md");

fn help_topic(topic: Option<&str>) -> Result<&'static str, String> {
    match topic.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("overview") => Ok(HELP_OVERVIEW),
        Some("agents") => Ok(HELP_AGENTS),
        Some("sessions") => Ok(HELP_SESSIONS),
        Some("tools") => Ok(HELP_TOOLS),
        Some("tasks") => Ok(HELP_TASKS),
        Some("unity") => Ok(HELP_UNITY),
        Some("callbacks") => Ok(HELP_CALLBACKS),
        Some(value) => Err(format!(
            "Unknown Python SDK help topic '{value}'. Use overview, agents, sessions, tools, tasks, unity, or callbacks."
        )),
    }
}

fn indent_python_body(code: &str) -> String {
    code.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_python_source(
    code: &str,
    project: &str,
    checkout_id: &str,
    workspace_generation: u64,
) -> Result<String, String> {
    let project = serde_json::to_string(project)
        .map_err(|error| format!("Failed to encode checkout path: {error}"))?;
    let checkout_id = serde_json::to_string(checkout_id)
        .map_err(|error| format!("Failed to encode checkout id: {error}"))?;
    Ok(format!(
        "import asyncio as __locus_asyncio\n\
import locus as locus\n\
project = {project}\n\
workspace_ref = locus.WorkspaceRef(checkout_id={checkout_id}, expected_generation={workspace_generation})\n\
\n\
async def __locus_python_main__():\n{}\n\
\n\
__locus_asyncio.run(__locus_python_main__())\n",
        indent_python_body(code)
    ))
}

fn action(args: &serde_json::Value) -> &str {
    args.get("action")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("run")
}

pub(crate) fn is_readonly(args: &serde_json::Value) -> bool {
    action(args) == "help"
        || args
            .get("readonly")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

pub(super) fn python() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PYTHON);
    ToolDef {
        name: "python".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Arbitrary Python and nested SDK tool calls may mutate the checkout.
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                match action(&args) {
                    "help" => {
                        return match help_topic(
                            args.get("topic").and_then(serde_json::Value::as_str),
                        ) {
                            Ok(output) => ToolResult {
                                output: output.trim().to_string(),
                                is_error: false,
                            },
                            Err(output) => ToolResult {
                                output,
                                is_error: true,
                            },
                        };
                    }
                    "run" => {}
                    value => {
                        return ToolResult {
                            output: format!("Unknown Python action '{value}'. Use run or help."),
                            is_error: true,
                        };
                    }
                }

                if args
                    .get("readonly")
                    .and_then(serde_json::Value::as_bool)
                    .is_none()
                {
                    return ToolResult {
                        output: "Missing required parameter: readonly".to_string(),
                        is_error: true,
                    };
                }
                let Some(code) = args
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return ToolResult {
                        output: "Missing required parameter: code".to_string(),
                        is_error: true,
                    };
                };
                let timeout_ms = args
                    .get("timeout")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(DEFAULT_TIMEOUT_MS);
                if timeout_ms == 0 || timeout_ms > MAX_TIMEOUT_MS {
                    return ToolResult {
                        output: format!("timeout must be between 1 and {MAX_TIMEOUT_MS}ms"),
                        is_error: true,
                    };
                }
                let Some(execution) = ctx.execution.as_ref() else {
                    return ToolResult {
                        output: "Python requires a checkout-scoped execution context.".to_string(),
                        is_error: true,
                    };
                };
                let project = execution.root().to_string_lossy().to_string();
                let source = match build_python_source(
                    code,
                    &project,
                    execution.checkout_id.as_str(),
                    execution.workspace_generation,
                ) {
                    Ok(source) => source,
                    Err(output) => {
                        return ToolResult {
                            output,
                            is_error: true,
                        };
                    }
                };

                let Some(runtime) =
                    crate::python_runtime::resolve_effective_python(ctx.app_handle.as_ref())
                else {
                    return ToolResult {
                        output: "No Python runtime is available. Select or install one in Settings > General."
                            .to_string(),
                        is_error: true,
                    };
                };
                if let Err(output) =
                    crate::python_runtime::ensure_runtime_package_environment(&runtime)
                {
                    return ToolResult {
                        output,
                        is_error: true,
                    };
                }

                let mut command = async_command(&runtime.path.to_string_lossy());
                command
                    .arg("-u")
                    .arg("-")
                    .current_dir(&project)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
                for (key, value) in crate::python_runtime::python_process_env(&runtime) {
                    command.env(key, value);
                }
                command.env("LOCUS_PROJECT", OsString::from(&project));
                command.env(
                    "LOCUS_CHECKOUT_ID",
                    OsString::from(execution.checkout_id.as_str()),
                );
                command.env(
                    "LOCUS_WORKSPACE_GENERATION",
                    OsString::from(execution.workspace_generation.to_string()),
                );

                let owner = ctx.process_owner.clone().unwrap_or_else(|| ProcessOwner {
                    working_dir: Some(project.clone()),
                    ..Default::default()
                });
                if let Some(session_id) = owner.session_id.as_deref() {
                    command.env("LOCUS_SESSION_ID", OsString::from(session_id));
                }

                if let Some(report) = ctx.progress.as_ref() {
                    report("Python workflow running".to_string());
                }
                let execution = run_captured_command_with_input(
                    command,
                    Some(source.into_bytes()),
                    ctx.output.clone(),
                    owner,
                    ctx.output_path.clone(),
                );

                let result = if ctx.background {
                    if let Some(mut cancel_rx) = ctx.cancel_rx.clone() {
                        tokio::select! {
                            result = execution => result,
                            _ = cancel_rx.changed() => {
                                return ToolResult {
                                    output: "Python workflow cancelled.".to_string(),
                                    is_error: true,
                                };
                            }
                        }
                    } else {
                        execution.await
                    }
                } else {
                    let timed = tokio::time::timeout(
                        std::time::Duration::from_millis(timeout_ms),
                        execution,
                    );
                    if let Some(mut cancel_rx) = ctx.cancel_rx.clone() {
                        tokio::select! {
                            result = timed => result,
                            _ = cancel_rx.changed() => {
                                return ToolResult {
                                    output: "Python workflow cancelled.".to_string(),
                                    is_error: true,
                                };
                            }
                        }
                    } else {
                        timed.await
                    }
                    .map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("Python workflow timed out after {timeout_ms}ms"),
                        )
                    })
                    .and_then(|result| result)
                };

                match result {
                    Ok(output) => {
                        let mut text = decode_console_bytes(&output.bytes);
                        if text.len() > 50_000 {
                            let total_bytes = text.len();
                            text = format!(
                                "{}\n\n(output truncated, {} bytes total)",
                                truncate_utf8_middle(&text, 50_000),
                                total_bytes
                            );
                        }
                        if text.is_empty() {
                            text = "(no output)".to_string();
                        }
                        let exit_code = output.status.code().unwrap_or(-1);
                        ToolResult {
                            output: format!("Exit code: {exit_code}\n{text}"),
                            is_error: exit_code != 0,
                        }
                    }
                    Err(error) => ToolResult {
                        output: format!("Failed to execute Python workflow: {error}"),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_python_source, help_topic, is_readonly};

    #[test]
    fn source_injects_checkout_scope_and_async_body() {
        let source = build_python_source(
            "status = await locus.get_unity_editor_status(project=project)\nprint(status.ready)",
            r"F:\Project",
            "checkout-7",
            9,
        )
        .expect("source");
        assert!(source.contains("project = \"F:\\\\Project\""));
        assert!(source.contains("checkout_id=\"checkout-7\""));
        assert!(source.contains("expected_generation=9"));
        assert!(source.contains("    status = await locus.get_unity_editor_status"));
        assert!(source.contains("__locus_asyncio.run(__locus_python_main__())"));
    }

    #[test]
    fn help_topics_expand_without_running_python() {
        assert!(help_topic(Some("agents"))
            .unwrap()
            .contains("Agent workflows"));
        assert!(help_topic(Some("unity"))
            .unwrap()
            .contains("restart_unity_editor"));
        assert!(help_topic(Some("missing")).is_err());
        assert!(is_readonly(&serde_json::json!({"action": "help"})));
        assert!(is_readonly(
            &serde_json::json!({"action": "run", "readonly": true})
        ));
        assert!(!is_readonly(
            &serde_json::json!({"action": "run", "readonly": false})
        ));
    }
}
