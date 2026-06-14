//! JSON-RPC client for the Locus compile-server sidecar.
//!
//! Speaks Content-Length framed JSON-RPC over the child's stdio — the same
//! framing as `csharp_lsp::client`, with the parts the compile server does
//! not need (capability registration, document sync) removed.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};

pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Compiles can be slow right after a sidecar cold start (Roslyn JIT +
/// first-time reference loading over a few hundred DLLs).
pub const COMPILE_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
pub const SCHEMA_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// A running compile-server process plus the JSON-RPC plumbing.
pub struct CompileClient {
    stdin: AsyncMutex<tokio::process::ChildStdin>,
    child: Mutex<Option<tokio::process::Child>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicI64,
    exited_rx: watch::Receiver<bool>,
    unusable: AtomicBool,
}

impl CompileClient {
    /// Spawn the server process and start the stdout reader loop.
    pub async fn spawn(
        program: &Path,
        args: &[String],
        envs: &[(String, String)],
        stderr_log: &Path,
    ) -> Result<std::sync::Arc<Self>, String> {
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        for (key, value) in envs {
            cmd.env(key, value);
        }
        match std::fs::File::create(stderr_log) {
            Ok(file) => {
                cmd.stderr(Stdio::from(file));
            }
            Err(_) => {
                cmd.stderr(Stdio::null());
            }
        }
        crate::process_util::suppress_async_command_window(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start C# compile server: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "C# compile server stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "C# compile server stdout unavailable".to_string())?;

        let (exited_tx, exited_rx) = watch::channel(false);

        let client = std::sync::Arc::new(CompileClient {
            stdin: AsyncMutex::new(stdin),
            child: Mutex::new(Some(child)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(0),
            exited_rx,
            unusable: AtomicBool::new(false),
        });

        let reader_client = std::sync::Arc::clone(&client);
        tokio::spawn(async move {
            reader_client.read_loop(stdout).await;
            let _ = exited_tx.send(true);
            reader_client.fail_all_pending("C# compile server exited");
        });

        Ok(client)
    }

    pub fn has_exited(&self) -> bool {
        self.unusable.load(Ordering::Relaxed) || *self.exited_rx.borrow()
    }

    async fn read_loop(&self, stdout: tokio::process::ChildStdout) {
        let mut reader = BufReader::new(stdout);
        let mut header_line = String::new();
        loop {
            let mut content_length: usize = 0;
            loop {
                header_line.clear();
                match reader.read_line(&mut header_line).await {
                    Ok(0) => return,
                    Ok(_) => {}
                    Err(_) => return,
                }
                let trimmed = header_line.trim();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed
                    .strip_prefix("Content-Length:")
                    .or_else(|| trimmed.strip_prefix("content-length:"))
                {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            if content_length == 0 {
                continue;
            }
            let mut body = vec![0u8; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
            let Ok(message) = serde_json::from_slice::<Value>(&body) else {
                continue;
            };
            self.dispatch(message).await;
        }
    }

    async fn dispatch(&self, message: Value) {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(|m| m.as_str());

        match (id, method) {
            // Response to one of our requests.
            (Some(id), None) => {
                let Some(id) = id.as_i64() else { return };
                let sender = self
                    .pending
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&id));
                if let Some(sender) = sender {
                    let outcome = if let Some(error) = message.get("error") {
                        Err(format!(
                            "compile server error {}: {}",
                            error.get("code").and_then(|c| c.as_i64()).unwrap_or(0),
                            error
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown")
                        ))
                    } else {
                        Ok(message.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = sender.send(outcome);
                }
            }
            // The server issues no requests today; answer anything anyway so
            // a future server version cannot stall on a missing response.
            (Some(id), Some(method)) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("method not handled: {method}") }
                });
                let _ = self.write_message(&response).await;
            }
            // Notifications from the server are informational only.
            _ => {}
        }
    }

    fn fail_all_pending(&self, reason: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            for (_, sender) in pending.drain() {
                let _ = sender.send(Err(reason.to_string()));
            }
        }
    }

    async fn write_message(&self, message: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(&body);
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&frame)
            .await
            .map_err(|e| format!("C# compile server write failed: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("C# compile server flush failed: {e}"))?;
        Ok(())
    }

    pub async fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, String> {
        self.request_with_timeout_inner(method, params, timeout, true)
            .await
    }

    pub async fn request_with_timeout_no_kill(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, String> {
        self.request_with_timeout_inner(method, params, timeout, false)
            .await
    }

    async fn request_with_timeout_inner(
        &self,
        method: &str,
        params: Value,
        timeout: std::time::Duration,
        kill_on_timeout: bool,
    ) -> Result<Value, String> {
        if self.has_exited() {
            return Err("C# compile server is not running".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(id, tx);
        }
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(error) = self.write_message(&message).await {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("C# compile server dropped the request".to_string()),
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                if kill_on_timeout {
                    self.kill_after_timeout(method);
                }
                Err(format!("C# compile server request '{method}' timed out"))
            }
        }
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_timeout(method, params, DEFAULT_REQUEST_TIMEOUT)
            .await
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.write_message(&message).await
    }

    /// Synchronous best-effort kill for app-exit paths.
    pub fn kill_process(&self) {
        self.unusable.store(true, Ordering::Relaxed);
        self.fail_all_pending("C# compile server stopped");
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.start_kill();
            }
        }
    }

    fn kill_after_timeout(&self, method: &str) {
        let reason = format!("C# compile server request '{method}' timed out; restarting sidecar");
        eprintln!("[CsharpCompile] {reason}");
        self.unusable.store(true, Ordering::Relaxed);
        self.fail_all_pending(&reason);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.start_kill();
            }
        }
    }

    /// Graceful shutdown; the process is killed if it lingers.
    pub async fn shutdown(&self) {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            self.request("shutdown", json!({})),
        )
        .await;
        let _ = self.notify("exit", json!({})).await;
        let child = self.child.lock().ok().and_then(|mut guard| guard.take());
        if let Some(mut child) = child {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await;
            let _ = child.start_kill();
        }
    }
}
