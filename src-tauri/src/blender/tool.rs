//! `blender_execute`: argument parsing, the `exec` payload and the result
//! text/images. Used by the agent loop and the MCP server (both can return
//! images, which the generic `ToolResult` cannot).

use serde_json::Value;

use super::exec::{self, ExecFailure};
use super::hub::OutputChunk;
use super::target::Scope;
use crate::session::models::ImageData;

/// Images above this are left out of the result (same cap as `read`).
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGES: usize = 4;
/// Everything stays inline for the tool path (no shared memory reads here).
const TOOL_INLINE_LIMIT: u64 = 96 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct BlenderExecuteArgs {
    pub code: String,
    pub readonly: bool,
    pub target: Option<String>,
    pub session_namespace: bool,
    pub timeout_ms: Option<u64>,
}

pub fn parse_args(args: &Value) -> Result<BlenderExecuteArgs, String> {
    let code = args
        .get("code")
        .and_then(Value::as_str)
        .filter(|code| !code.trim().is_empty())
        .ok_or_else(|| "Missing required parameter: code".to_string())?
        .to_string();
    let readonly = args
        .get("readonly")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Missing required parameter: readonly (true or false)".to_string())?;
    let namespace = args.get("namespace").and_then(Value::as_str).map(str::trim).unwrap_or("fresh");
    let session_namespace = match namespace {
        "" | "fresh" => false,
        "session" => true,
        other => return Err(format!("Invalid namespace '{other}'. Allowed values: fresh, session.")),
    };
    Ok(BlenderExecuteArgs {
        code,
        readonly,
        target: args
            .get("target")
            .and_then(|value| match value {
                Value::String(text) => Some(text.trim().to_string()),
                Value::Number(number) => Some(number.to_string()),
                _ => None,
            })
            .filter(|text| !text.is_empty()),
        session_namespace,
        timeout_ms: args.get("timeout").and_then(Value::as_u64),
    })
}

/// Bind the checkout's Blender service for this call (starting it when the
/// workspace mode is lazy) and hold a lease so it is not reaped meanwhile.
pub async fn bind_service(
    execution: &crate::workspace_service::AgentExecutionContext,
) -> Result<crate::workspace_service::ResolvedServiceBinding, String> {
    use crate::workspace_service::ServiceKind;
    execution.ensure_service(ServiceKind::Blender).await?;
    execution
        .resolve_service(ServiceKind::Blender)
        .map_err(|error| format!("Blender service unavailable: {error}"))
}

/// Run the tool for an Agent execution context.
pub async fn run_in_execution(
    execution: &crate::workspace_service::AgentExecutionContext,
    session_id: Option<&str>,
    args: &Value,
    on_output: impl FnMut(String) + Send,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> ToolRun {
    let _binding = match bind_service(execution).await {
        Ok(binding) => binding,
        Err(error) => return ToolRun::error(error),
    };
    let scope = Scope::new(execution.checkout_id.as_str(), execution.root(), session_id);
    run(&scope, args, on_output, cancel).await
}

pub fn is_readonly(args: &Value) -> bool {
    args.get("readonly").and_then(Value::as_bool).unwrap_or(false)
}

pub struct ToolRun {
    pub output: String,
    pub is_error: bool,
    pub images: Vec<ImageData>,
}

impl ToolRun {
    fn error(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true, images: Vec::new() }
    }
}

fn undo_label(code: &str) -> String {
    let line = code
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("blender_execute");
    let mut label = String::from("Locus: ");
    label.extend(line.chars().take(48));
    label
}

pub async fn run(
    scope: &Scope,
    args: &Value,
    mut on_output: impl FnMut(String) + Send,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> ToolRun {
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(error) => return ToolRun::error(error),
    };
    let payload = serde_json::json!({
        "mode": "code",
        "code": parsed.code,
        "readonly": parsed.readonly,
        "namespace": if parsed.session_namespace { "session" } else { "fresh" },
        "lenient": true,
        "label": undo_label(&parsed.code),
        "inline_limit": TOOL_INLINE_LIMIT,
    });
    let timeout = exec::clamp_timeout(parsed.timeout_ms);
    let outcome = exec::execute(
        scope,
        parsed.target.as_deref(),
        payload,
        timeout,
        |chunk: OutputChunk| on_output(chunk.text),
        cancel,
    )
    .await;
    match outcome {
        Ok(success) => format_success(&success),
        Err(failure) => format_failure(&failure),
    }
}

fn format_failure(failure: &ExecFailure) -> ToolRun {
    ToolRun::error(failure.render())
}

fn format_success(success: &exec::ExecSuccess) -> ToolRun {
    let result = &success.result;
    let ok = result.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let mut text = exec::header_line(&success.instance);
    let stdout = result.get("stdout").and_then(Value::as_str).unwrap_or("").trim_end();
    let stderr = result.get("stderr").and_then(Value::as_str).unwrap_or("").trim_end();
    if !stdout.is_empty() {
        text.push('\n');
        text.push_str(stdout);
    }
    if !stderr.is_empty() {
        text.push_str("\nstderr:\n");
        text.push_str(stderr);
    }
    let mut images = Vec::new();
    let mut warnings = success.warnings.clone();
    if ok {
        let value = result.get("value").unwrap_or(&Value::Null);
        collect_images(value, &mut images, &mut warnings);
        if !value.is_null() {
            let repr = result.get("value_repr").and_then(Value::as_str).unwrap_or("");
            text.push_str("\nReturn value: ");
            text.push_str(repr);
        } else if stdout.is_empty() && stderr.is_empty() {
            text.push_str("\nExecuted (no output, returned None).");
        }
    } else if let Some(error) = result.get("error") {
        let kind = error.get("type").and_then(Value::as_str).unwrap_or("Error");
        let message = error.get("message").and_then(Value::as_str).unwrap_or("");
        text.push_str(&format!("\nError: {kind}: {message}"));
        if let Some(traceback) = error.get("traceback").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
            text.push('\n');
            text.push_str(traceback.trim_end());
        }
        if result.get("cancelled").and_then(Value::as_bool) == Some(true) {
            text.push_str("\nThe execution was cancelled; changes made before the cancel remain.");
        }
    }
    if let Some(attachments) = result.get("attachments").and_then(Value::as_array) {
        for attachment in attachments {
            collect_images(attachment, &mut images, &mut warnings);
        }
    }
    if images.len() > MAX_IMAGES {
        warnings.push(format!("{} images were produced; only the first {MAX_IMAGES} are attached", images.len()));
        images.truncate(MAX_IMAGES);
    }
    if !images.is_empty() {
        text.push_str(&format!("\n[{} image(s) attached]", images.len()));
    }
    for warning in result.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        if let Some(warning) = warning.as_str() {
            warnings.push(warning.to_string());
        }
    }
    if !warnings.is_empty() {
        text.push_str("\nWarnings:");
        for warning in &warnings {
            text.push_str("\n- ");
            text.push_str(warning);
        }
    }
    ToolRun { output: text, is_error: !ok, images }
}

/// Top-level image, or a list/tuple of images.
fn collect_images(value: &Value, images: &mut Vec<ImageData>, warnings: &mut Vec<String>) {
    if let Some(image) = value.get("$img") {
        let mime = image.get("mime").and_then(Value::as_str).unwrap_or("image/png").to_string();
        match image.get("data").and_then(|data| data.get("b64")).and_then(Value::as_str) {
            Some(data) if data.len() / 4 * 3 <= MAX_IMAGE_BYTES => images.push(ImageData {
                data: data.to_string(),
                mime_type: mime,
                capture: None,
            }),
            Some(_) => warnings.push("an image above 20 MiB was not attached; lower max_long_edge".to_string()),
            None => warnings.push("an image was not attached (not inline)".to_string()),
        }
        return;
    }
    let items = value
        .as_array()
        .or_else(|| value.get("$t").and_then(Value::as_array));
    if let Some(items) = items {
        for item in items {
            if item.get("$img").is_some() {
                collect_images(item, images, warnings);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_require_code_and_readonly() {
        assert!(parse_args(&serde_json::json!({"code": "x"})).is_err());
        assert!(parse_args(&serde_json::json!({"readonly": true})).is_err());
        let parsed = parse_args(&serde_json::json!({
            "code": "return 1", "readonly": true, "target": 1234, "namespace": "session", "timeout": 5000
        }))
        .unwrap();
        assert_eq!(parsed.target.as_deref(), Some("1234"));
        assert!(parsed.session_namespace);
        assert_eq!(parsed.timeout_ms, Some(5000));
        assert!(parse_args(&serde_json::json!({"code": "x", "readonly": true, "namespace": "global"})).is_err());
    }

    #[test]
    fn images_come_from_values_lists_and_tuples() {
        let image = serde_json::json!({"$img": {"mime": "image/png", "data": {"b64": "iVBORw0KGgo="}}});
        let mut images = Vec::new();
        let mut warnings = Vec::new();
        collect_images(&image, &mut images, &mut warnings);
        collect_images(&serde_json::json!([image.clone(), 3]), &mut images, &mut warnings);
        collect_images(&serde_json::json!({"$t": [image.clone()]}), &mut images, &mut warnings);
        collect_images(&serde_json::json!({"$img": {"data": {"shm": "x", "size": 3}}}), &mut images, &mut warnings);
        assert_eq!(images.len(), 3);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn undo_label_uses_the_first_code_line() {
        assert_eq!(undo_label("# comment\n\nbpy.ops.mesh.primitive_cube_add()\n"), "Locus: bpy.ops.mesh.primitive_cube_add()");
    }
}
