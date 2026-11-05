//! Forwards permitted remote calls into Tauri's own command dispatcher.
//!
//! Going through `WebviewWindow::on_message` keeps argument decoding exactly
//! as for the desktop frontend (camelCase names, Option defaults, State
//! injection), so command signature changes need no glue here. The request
//! is attributed to the primary application window, which is why commands that
//! use their calling window are never in the remote [`policy`](super::policy).
//!
//! Dispatch happens on the main thread, like a WebView IPC message: on
//! Windows the runtime handle inside `AppHandle`/`WebviewWindow` holds a
//! non-atomic `Rc`, so the window lookup (a handle clone) must not run on
//! HTTP worker threads.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeError, InvokeResponse, InvokeResponseBody};
use tauri::{AppHandle, Manager};

use super::policy;

const INVOKE_TIMEOUT: Duration = Duration::from_secs(120);

/// A failed command: its message, and its own error value when it has one
/// (Locus `AppError` JSON), so remote clients can handle it like a local
/// invoke error.
#[derive(Debug, PartialEq)]
pub struct Failure {
    pub message: String,
    pub data: Option<Value>,
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self { message, data: None }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self { message: message.into(), data: None }
    }
}

/// Runs `command` with JSON `args` and returns its result as JSON text,
/// exactly as the command serialized it (no re-encoding on the way out).
/// The caller has checked the device against the policy.
pub async fn call(app: &Arc<AppHandle>, command: &str, args: Value) -> Result<String, Failure> {
    if policy::rule_for(command).is_none() {
        return Err(format!("Command '{command}' is not available remotely").into());
    }
    let args = match args {
        Value::Null => Value::Object(Default::default()),
        Value::Object(map) => Value::Object(map),
        _ => return Err("Command arguments must be an object".into()),
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<InvokeResponse, String>>();
    let handle = Arc::clone(app);
    let cmd = command.to_string();
    app.run_on_main_thread(move || {
        let window = crate::workspace_window::primary_application_window(&handle)
            .or_else(|| handle.webview_windows().into_values().next());
        let Some(window) = window else {
            let _ = tx.send(Err("Locus has no window to run the command in".into()));
            return;
        };
        let url = match window.url() {
            Ok(url) => url,
            Err(error) => {
                let _ = tx.send(Err(error.to_string()));
                return;
            }
        };
        let request = tauri::webview::InvokeRequest {
            cmd,
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url,
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: handle.invoke_key().to_string(),
        };
        window.on_message(
            request,
            Box::new(move |_webview, _command, response, _callback, _error| {
                let _ = tx.send(Ok(response));
            }),
        );
    })
    .map_err(|error| Failure::from(format!("Locus main thread unavailable: {error}")))?;
    let response = tokio::time::timeout(INVOKE_TIMEOUT, rx)
        .await
        .map_err(|_| Failure::from(format!("Command '{command}' timed out")))?
        .map_err(|_| Failure::from(format!("Command '{command}' was dropped")))?
        .map_err(Failure::from)?;
    decode(response)
}

fn decode(response: InvokeResponse) -> Result<String, Failure> {
    match response {
        InvokeResponse::Ok(InvokeResponseBody::Json(json)) => Ok(json),
        // No remotely available command returns raw bytes today; if one
        // does, it arrives as base64 text.
        InvokeResponse::Ok(InvokeResponseBody::Raw(bytes)) => {
            use base64::Engine;
            serde_json::to_string(&base64::engine::general_purpose::STANDARD.encode(bytes))
                .map_err(|error| Failure::from(error.to_string()))
        }
        InvokeResponse::Err(InvokeError(value)) => Err(Failure { message: error_message(&value), data: Some(value) }),
    }
}

/// AppError serializes as `{code, message, ...}`; plain errors as strings.
fn error_message(value: &Value) -> String {
    match value {
        Value::String(message) => message.clone(),
        Value::Object(map) => map
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string()),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_results_and_keeps_command_errors() {
        assert_eq!(
            decode(InvokeResponse::Ok(InvokeResponseBody::Json("{\"a\":1}".into()))).unwrap(),
            "{\"a\":1}"
        );
        assert_eq!(
            decode(InvokeResponse::Ok(InvokeResponseBody::Raw(vec![1, 2]))).unwrap(),
            "\"AQI=\""
        );
        let app_error = json!({"code":"x","message":"boom","detail":{"path":"a"}});
        assert_eq!(
            decode(InvokeResponse::Err(InvokeError(app_error.clone()))),
            Err(Failure { message: "boom".into(), data: Some(app_error) })
        );
        assert_eq!(
            decode(InvokeResponse::Err(InvokeError(json!("plain")))),
            Err(Failure { message: "plain".into(), data: Some(json!("plain")) })
        );
    }
}
