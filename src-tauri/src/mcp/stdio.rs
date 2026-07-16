//! stdio transport for MCP servers.
//!
//! Spawns the configured command with piped stdin/stdout and speaks
//! newline-delimited JSON-RPC 2.0 over the pipes (MCP stdio framing — not
//! the `Content-Length` LSP framing the compile sidecar uses). stderr is a
//! log channel; a bounded tail is kept for diagnostics.

use std::collections::{HashMap, VecDeque};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, watch};

use super::config::{expand_env_refs, McpServerConfig};
use super::types::{JsonRpcNotification, JsonRpcRequest};

const STDERR_TAIL_LINES: usize = 40;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

type PendingMap = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

pub struct StdioMcpClient {
    child: Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    pending: PendingMap,
    next_id: AtomicI64,
    exited_rx: watch::Receiver<bool>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    server_id: String,
    /// Set by deliberate teardown so the auto-restart watcher can tell a
    /// crash from a shutdown.
    expected_exit: Arc<std::sync::atomic::AtomicBool>,
}

impl StdioMcpClient {
    /// Spawns the configured process. On Windows a bare program name that
    /// resolves to a `.cmd`/`.bat` shim (uvx, npx, ...) is not spawnable via
    /// CreateProcess, so a NotFound error triggers one retry through
    /// `cmd /c`.
    pub async fn spawn(config: &McpServerConfig) -> Result<Self, String> {
        let child = spawn_child(config)?;
        Ok(Self::wrap(child, config.id.clone()))
    }

    fn wrap(mut child: Child, server_id: String) -> Self {
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (exited_tx, exited_rx) = watch::channel(false);
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));

        if let Some(stdout) = stdout {
            let pending = pending.clone();
            let id = server_id.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => handle_line(&id, &line, &pending),
                        Ok(None) => break,
                        Err(e) => {
                            eprintln!("[Mcp:{id}] stdout read error: {e}");
                            break;
                        }
                    }
                }
                let _ = exited_tx.send(true);
                fail_all_pending(&pending, "MCP server exited");
            });
        }

        if let Some(stderr) = stderr {
            let tail = stderr_tail.clone();
            let id = server_id.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("[Mcp:{id}:stderr] {line}");
                    let mut tail = tail.lock().unwrap_or_else(|p| p.into_inner());
                    if tail.len() >= STDERR_TAIL_LINES {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
        }

        Self {
            child: Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(stdin),
            pending,
            next_id: AtomicI64::new(1),
            exited_rx,
            stderr_tail,
            server_id,
            expected_exit: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn has_exited(&self) -> bool {
        *self.exited_rx.borrow()
    }

    /// Resolves when the server process exits (immediately if it already
    /// has). Drives the optional auto-restart watcher.
    pub async fn wait_exited(&self) {
        let mut rx = self.exited_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// True when the exit was caused by shutdown()/kill_for_exit() rather
    /// than the process dying on its own.
    pub fn exit_was_expected(&self) -> bool {
        self.expected_exit.load(Ordering::SeqCst)
    }

    /// Last stderr lines, newest last — appended to connection errors so a
    /// crashing server's own words reach the settings UI.
    pub fn stderr_tail(&self) -> String {
        let tail = self.stderr_tail.lock().unwrap_or_else(|p| p.into_inner());
        tail.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    async fn write_line(&self, payload: String) -> Result<(), String> {
        let mut guard = self.stdin.lock().await;
        let stdin = guard.as_mut().ok_or("MCP server stdin is closed")?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| format!("Failed to write to MCP server: {e}"))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| format!("Failed to write to MCP server: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush MCP server stdin: {e}"))
    }

    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        let msg = JsonRpcNotification::new(method, params);
        let payload = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
        self.write_line(payload).await
    }

    pub async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.request_cancellable(method, params, timeout, None).await
    }

    /// Sends one request; an optional cancel watcher aborts the wait, posts
    /// `notifications/cancelled` for the in-flight id (best effort) and
    /// returns the shared cancellation marker.
    pub async fn request_cancellable(
        &self,
        method: &str,
        params: Option<Value>,
        timeout: Duration,
        cancel: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Result<Value, String> {
        if self.has_exited() {
            return Err(self.error_with_stderr("MCP server has exited"));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, tx);

        let msg = JsonRpcRequest::new(id, method, params);
        let payload = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
        if let Err(e) = self.write_line(payload).await {
            self.pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&id);
            return Err(self.error_with_stderr(&e));
        }

        let cancel_wait = async {
            match cancel {
                Some(mut rx) => {
                    let _ = rx.changed().await;
                }
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            outcome = tokio::time::timeout(timeout, rx) => match outcome {
                Ok(Ok(result)) => result.map_err(|e| self.error_with_stderr(&e)),
                Ok(Err(_)) => Err(self.error_with_stderr("MCP request channel dropped")),
                Err(_) => {
                    self.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&id);
                    Err(format!(
                        "MCP request '{method}' timed out after {}s",
                        timeout.as_secs()
                    ))
                }
            },
            _ = cancel_wait => {
                self.pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        Some(serde_json::json!({
                            "requestId": id,
                            "reason": "User requested cancellation",
                        })),
                    )
                    .await;
                Err(super::types::MCP_CALL_CANCELLED.to_string())
            }
        }
    }

    fn error_with_stderr(&self, base: &str) -> String {
        let tail = self.stderr_tail();
        if tail.is_empty() {
            base.to_string()
        } else {
            format!("{base}\nserver stderr:\n{tail}")
        }
    }

    /// Kills a wedged process WITHOUT marking the exit as expected: the
    /// ping keepalive uses this so an auto-restart watcher sees a crash.
    pub fn kill_unresponsive(&self) {
        let mut guard = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(child) = guard.as_mut() {
            let _ = child.start_kill();
        }
    }

    /// Synchronous best-effort kill for app exit paths that cannot await
    /// (mirrors csharp_compile's kill_for_exit).
    pub fn kill_for_exit(&self) {
        self.expected_exit.store(true, Ordering::SeqCst);
        let mut guard = self.child.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(child) = guard.as_mut() {
            let _ = child.start_kill();
        }
    }

    /// MCP stdio shutdown: close stdin, give the process a short grace
    /// period to exit on its own, then kill.
    pub async fn shutdown(&self) {
        self.expected_exit.store(true, Ordering::SeqCst);
        {
            let mut stdin = self.stdin.lock().await;
            *stdin = None;
        }
        let child = {
            let mut guard = self.child.lock().unwrap_or_else(|p| p.into_inner());
            guard.take()
        };
        let Some(mut child) = child else {
            return;
        };
        match tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await {
            Ok(_) => {}
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
        eprintln!("[Mcp:{}] server process closed", self.server_id);
    }
}

fn handle_line(server_id: &str, line: &str, pending: &PendingMap) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            // Misbehaving servers print banners to stdout; keep a note but
            // don't fail the connection over it.
            eprintln!("[Mcp:{server_id}] non-JSON line on stdout: {trimmed}");
            return;
        }
    };
    let Some(id) = value.get("id").and_then(Value::as_i64) else {
        // Server-initiated notification (tools/list_changed, logging, ...).
        if let Some(method) = value.get("method").and_then(Value::as_str) {
            super::manager::handle_server_notification(server_id, method, value.get("params"));
        }
        return;
    };
    let sender = pending
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&id);
    let Some(sender) = sender else {
        return;
    };
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let _ = sender.send(Err(format!("MCP error {code}: {message}")));
    } else {
        let result = value.get("result").cloned().unwrap_or(Value::Null);
        let _ = sender.send(Ok(result));
    }
}

fn fail_all_pending(pending: &PendingMap, reason: &str) {
    let mut map = pending.lock().unwrap_or_else(|p| p.into_inner());
    for (_, sender) in map.drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
}

fn spawn_child(config: &McpServerConfig) -> Result<Child, String> {
    match try_spawn(config, false) {
        Ok(child) => Ok(child),
        Err(e) if should_retry_via_cmd(config, &e) => try_spawn(config, true)
            .map_err(|retry| format!("Failed to start '{}': {retry}", config.command)),
        Err(e) => Err(format!("Failed to start '{}': {e}", config.command)),
    }
}

fn should_retry_via_cmd(config: &McpServerConfig, error: &std::io::Error) -> bool {
    cfg!(target_os = "windows")
        && error.kind() == std::io::ErrorKind::NotFound
        && !config.command.contains('/')
        && !config.command.contains('\\')
}

fn try_spawn(config: &McpServerConfig, via_cmd_shell: bool) -> std::io::Result<Child> {
    let mut cmd = if via_cmd_shell {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/c").arg(&config.command).args(&config.args);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new(&config.command);
        cmd.args(&config.args);
        cmd
    };
    if !config.cwd.is_empty() {
        cmd.current_dir(&config.cwd);
    }
    for (key, value) in &config.env {
        cmd.env(key, expand_env_refs(value));
    }
    crate::process_util::suppress_async_command_window(&mut cmd);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd.spawn()
}
