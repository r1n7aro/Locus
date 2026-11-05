use std::ffi::OsString;

use super::misc::truncate_utf8_middle;
use super::shell::{decode_console_bytes, run_captured_command_with_input};
use super::{make_exec, ToolDef, ToolResult};
use crate::process_util::{async_command, ProcessOwner};

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 1_800_000;

/// Virtual file name of the script body. Registered in `linecache` so
/// tracebacks show source lines and `inspect.getsource` works for functions
/// defined in the script (`@locus.blender.function` ships their source).
pub(crate) const SCRIPT_FILENAME: &str = "<locus-python>";

/// A condition Locus reported on purpose (stale snapshot, busy checkout, ...)
/// prints as the script's own frames, the message and its next step. Every
/// other exception, including unexpected SDK failures, is re-raised untouched
/// so its complete traceback stays available for debugging.
const REPORTED_ERROR_HANDLER: &str = "__locus_report = getattr(locus, \"_print_reported_error\", None)\n    \
if __locus_report is None or not __locus_report(__locus_error):\n        \
raise\n    \
raise SystemExit(1) from None";

fn build_python_source(
    code: &str,
    project: &str,
    checkout_id: &str,
    workspace_generation: u64,
    materialization_epoch: u64,
) -> Result<String, String> {
    let project = serde_json::to_string(project)
        .map_err(|error| format!("Failed to encode checkout path: {error}"))?;
    let checkout_id = serde_json::to_string(checkout_id)
        .map_err(|error| format!("Failed to encode checkout id: {error}"))?;
    // The script runs as the body of `async def __locus_python_main__` so a
    // top-level `await` and `return` work. Its parsed statements become that
    // body; re-indenting the text would also indent the continuation lines
    // of multi-line string literals and change their values. The registered
    // source is the script itself, so line numbers match what was submitted.
    let source = serde_json::to_string(code)
        .map_err(|error| format!("Failed to encode Python code: {error}"))?;
    let filename = serde_json::to_string(SCRIPT_FILENAME).unwrap_or_default();
    Ok(format!(
        "import ast as __locus_ast\n\
import asyncio as __locus_asyncio\n\
import linecache as __locus_linecache\n\
import locus as locus\n\
project = {project}\n\
workspace_ref = locus.WorkspaceRef(checkout_id={checkout_id}, expected_generation={workspace_generation}, expected_materialization_epoch={materialization_epoch})\n\
__locus_source__ = {source}\n\
__locus_linecache.cache[{filename}] = (len(__locus_source__), None, __locus_source__.splitlines(True), {filename})\n\
__locus_main__ = __locus_ast.parse(\"async def __locus_python_main__(): pass\")\n\
__locus_main__.body[0].body = __locus_ast.parse(__locus_source__, {filename}).body or __locus_main__.body[0].body\n\
exec(compile(__locus_main__, {filename}, \"exec\"))\n\
del __locus_source__, __locus_main__\n\
\n\
try:\n    \
__locus_asyncio.run(__locus_python_main__())\n\
except locus.LocusRpcError as __locus_error:\n    \
{REPORTED_ERROR_HANDLER}\n"
    ))
}

/// Images from `locus.attach_image` go to the caller's image sink (agent
/// loop); without one they are saved as files and listed.
fn deliver_images(images: Vec<crate::session::models::ImageData>, token: &str, output: &mut String) {
    if images.is_empty() {
        return;
    }
    if let Some(sink) = crate::tool::python_call::current_image_sink() {
        let count = images.len();
        if let Ok(mut sink) = sink.lock() {
            sink.extend(images);
        }
        output.push_str(&format!("\n[{count} image(s) attached]"));
        return;
    }
    let dir = std::env::temp_dir().join("locus-attachments").join(token);
    let paths = crate::tool::python_call::save_images(&images, &dir);
    output.push_str("\nAttached images were saved (view them with `read`):");
    for path in paths {
        output.push_str(&format!("\n- {}", path.display()));
    }
}

pub(crate) fn is_readonly(args: &serde_json::Value) -> bool {
    args.get("readonly")
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
                    execution.workspace.materialization_epoch(),
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
                command.env(
                    "LOCUS_MATERIALIZATION_EPOCH",
                    OsString::from(execution.workspace.materialization_epoch().to_string()),
                );

                let owner = ctx.process_owner.clone().unwrap_or_else(|| ProcessOwner {
                    working_dir: Some(project.clone()),
                    ..Default::default()
                });
                if let Some(session_id) = owner.session_id.as_deref() {
                    command.env("LOCUS_SESSION_ID", OsString::from(session_id));
                }
                let _sdk_delegation =
                    crate::agent::workspace_execution_lock::register_sdk_delegation(
                        execution.workspace.as_ref(),
                        owner.session_id.as_deref(),
                        owner.task_id.as_deref(),
                    );
                if let Some(delegation) = &_sdk_delegation {
                    command.env(
                        "LOCUS_SDK_EXECUTION_DELEGATION",
                        OsString::from(&delegation.token),
                    );
                }

                let python_call = crate::tool::python_call::register(
                    is_readonly(&args),
                    owner.session_id.clone(),
                    ctx.output.clone(),
                );
                command.env(crate::tool::python_call::ENV_CALL, python_call.token());
                if is_readonly(&args) {
                    command.env(crate::tool::python_call::ENV_READONLY, "1");
                }
                // Blender instances changed by this call push their one undo
                // step when the call ends, on every exit path.
                let _blender_batch = crate::blender::exec::BatchGuard::new(python_call.token());

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
                        let mut output = format!("Exit code: {exit_code}\n{text}");
                        deliver_images(python_call.take_images(), python_call.token(), &mut output);
                        ToolResult {
                            output,
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
    use super::{build_python_source, is_readonly};

    #[test]
    fn source_injects_checkout_scope_and_async_body() {
        let source = build_python_source(
            "status = await locus.get_unity_editor_status(project=project)\nprint(status.ready)",
            r"F:\Project",
            "checkout-7",
            9,
            3,
        )
        .expect("source");
        assert!(source.contains("project = \"F:\\\\Project\""));
        assert!(source.contains("checkout_id=\"checkout-7\""));
        assert!(source.contains("expected_generation=9"));
        assert!(source.contains("expected_materialization_epoch=3"));
        assert!(source.contains(
            "__locus_source__ = \"status = await locus.get_unity_editor_status(project=project)\\nprint(status.ready)\"\n"
        ));
        assert!(source.contains("__locus_main__.body[0].body = __locus_ast.parse(__locus_source__, \"<locus-python>\").body"));
        assert!(source.contains("__locus_asyncio.run(__locus_python_main__())"));
        // Only reported Locus conditions are shortened; everything else re-raises.
        assert!(source.contains(
            "try:\n    __locus_asyncio.run(__locus_python_main__())\nexcept locus.LocusRpcError as __locus_error:\n    __locus_report = "
        ));
        assert!(source.contains("__locus_report = getattr(locus, \"_print_reported_error\", None)\n    if __locus_report is None or not __locus_report(__locus_error):"));
        assert!(source.contains(":\n        raise\n    raise SystemExit(1) from None\n"));
    }

    #[test]
    fn source_keeps_multiline_string_literals_verbatim() {
        let code = "text = \"\"\"first\nsecond\n    third\"\"\"\nif text:\n    print(text)";
        let source = build_python_source(code, r"F:\Project", "checkout-7", 9, 3).expect("source");
        let literal = serde_json::to_string(code).expect("literal");
        assert!(source.contains(&format!("__locus_source__ = {literal}\n")));
        assert!(!source.contains("    second"));
    }

    #[test]
    fn readonly_requires_an_explicit_boolean() {
        assert!(is_readonly(&serde_json::json!({"readonly": true})));
        assert!(!is_readonly(&serde_json::json!({"readonly": false})));
        assert!(!is_readonly(&serde_json::json!({"readonly": "true"})));
        assert!(!is_readonly(&serde_json::json!({"action": "help"})));
        assert!(!is_readonly(&serde_json::json!({})));
    }

    #[tokio::test]
    async fn python_runs_directly_and_requires_code_and_readonly() {
        let tool = super::python();
        let properties = tool.parameters["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 3);
        assert_eq!(
            tool.parameters["required"],
            serde_json::json!(["code", "readonly"])
        );
        for (args, error) in [
            (
                serde_json::json!({"code": "print(1)"}),
                "Missing required parameter: readonly",
            ),
            (
                serde_json::json!({"readonly": true}),
                "Missing required parameter: code",
            ),
            (
                serde_json::json!({"readonly": true, "code": "  "}),
                "Missing required parameter: code",
            ),
            (
                serde_json::json!({"readonly": true, "code": "print(1)"}),
                "Python requires a checkout-scoped execution context.",
            ),
        ] {
            let result = (tool.execute)(args, crate::tool::ToolExecutionContext::default()).await;
            assert!(result.is_error);
            assert_eq!(result.output, error);
        }
    }

    #[tokio::test]
    async fn python_sdk_topic_is_readable_through_the_existing_read_tool() {
        let docs = crate::python_runtime::locus_python_sdk_docs_dir(None).expect("docs directory");
        let registry = crate::tool::ToolRegistry::with_builtins();
        let (description, _) = registry.tool_description("python").expect("Python schema");
        assert!(description.starts_with("Run Python "));
        assert!(!description.contains("{python_"));
        assert!(!description.contains("overview.md"));
        assert_eq!(
            registry.resolve_api_tool("python").unwrap()["function"]["description"],
            description
        );
        let result = registry
            .execute(
                "read",
                &serde_json::json!({
                    "filePath": docs.join("agents.md").to_string_lossy(),
                    "offset": 1,
                    "limit": 12,
                }),
            )
            .await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Agent workflows"));
        assert!(result.output.contains("await locus.list_agents()"));
    }
}
