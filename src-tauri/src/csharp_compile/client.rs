//! JSON-RPC client for the Locus compile-server sidecar.
//!
//! Speaks Content-Length framed JSON-RPC over the child's stdio — the same
//! framing as `csharp_lsp::client`, with the parts the compile server does
//! not need (capability registration, document sync) removed.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock, Weak};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Compiles can be slow right after a sidecar cold start (Roslyn JIT +
/// first-time reference loading over a few hundred DLLs).
pub const COMPILE_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
pub const SCHEMA_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
const CANCEL_SIGNAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

struct PendingRequest {
    sender: oneshot::Sender<Result<Value, String>>,
    scope_key: Option<String>,
    _job_permit: Option<super::scheduler::CompileJobPermit>,
    timed_out: bool,
}

struct WriterCommand {
    frame: Vec<u8>,
    completion: oneshot::Sender<Result<(), String>>,
}

/// A running compile-server process plus the JSON-RPC plumbing.
pub struct CompileClient {
    writer_tx: mpsc::UnboundedSender<WriterCommand>,
    child: Mutex<Option<tokio::process::Child>>,
    pending: Mutex<HashMap<i64, PendingRequest>>,
    next_id: AtomicI64,
    exited_rx: watch::Receiver<bool>,
    unusable: AtomicBool,
    self_weak: OnceLock<Weak<CompileClient>>,
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
        let (writer_tx, writer_rx) = mpsc::unbounded_channel();

        let client = std::sync::Arc::new(CompileClient {
            writer_tx,
            child: Mutex::new(Some(child)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(0),
            exited_rx,
            unusable: AtomicBool::new(false),
            self_weak: OnceLock::new(),
        });
        let _ = client.self_weak.set(std::sync::Arc::downgrade(&client));

        let writer_client = std::sync::Arc::downgrade(&client);
        tokio::spawn(async move {
            Self::write_loop(stdin, writer_rx, writer_client).await;
        });

        let reader_client = std::sync::Arc::clone(&client);
        tokio::spawn(async move {
            reader_client.read_loop(stdout).await;
            let _ = exited_tx.send(true);
            reader_client.fail_all_pending("C# compile server exited");
        });

        Ok(client)
    }

    async fn write_loop(
        mut stdin: tokio::process::ChildStdin,
        mut commands: mpsc::UnboundedReceiver<WriterCommand>,
        client: Weak<CompileClient>,
    ) {
        while let Some(command) = commands.recv().await {
            let result: Result<(), String> = async {
                stdin
                    .write_all(&command.frame)
                    .await
                    .map_err(|e| format!("C# compile server write failed: {e}"))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| format!("C# compile server flush failed: {e}"))?;
                Ok(())
            }
            .await;

            match result {
                Ok(()) => {
                    // Completion is advisory for the writer. A dropped caller
                    // must not cancel a frame once it has entered this queue.
                    let _ = command.completion.send(Ok(()));
                }
                Err(error) => {
                    if let Some(client) = client.upgrade() {
                        client.handle_writer_failure(&error);
                    }
                    let _ = command.completion.send(Err(error.clone()));
                    // A failed Content-Length frame may be partial, so the
                    // stream cannot safely accept any subsequent frame.
                    while let Ok(queued) = commands.try_recv() {
                        let _ = queued.completion.send(Err(error.clone()));
                    }
                    return;
                }
            }
        }
    }

    pub fn has_exited(&self) -> bool {
        self.unusable.load(Ordering::Relaxed) || *self.exited_rx.borrow()
    }

    pub async fn wait_for_process_exit(&self) {
        if *self.exited_rx.borrow() {
            return;
        }
        let mut exited = self.exited_rx.clone();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            while !*exited.borrow() {
                if exited.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
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
                let pending_request = self
                    .pending
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&id));
                if let Some(pending_request) = pending_request {
                    let scoped = pending_request.scope_key.is_some();
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
                    if !pending_request.timed_out {
                        if let Some(scope_key) = pending_request.scope_key.as_deref() {
                            super::scheduler::clear_scope_poisoned(scope_key);
                        }
                    }
                    if pending_request.sender.send(outcome).is_err() && scoped {
                        // The caller abandoned an already-issued stateful
                        // request. Its late registry mutations cannot be
                        // distinguished from successfully consumed output.
                        self.begin_scoped_timeout_recovery(id, "cancelled caller");
                    }
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
            for (_, pending_request) in pending.drain() {
                if let Some(scope_key) = pending_request.scope_key.as_deref() {
                    super::scheduler::clear_scope_poisoned(scope_key);
                }
                let _ = pending_request.sender.send(Err(reason.to_string()));
            }
        }
    }

    fn has_pending(&self, id: i64) -> bool {
        self.pending
            .lock()
            .map(|pending| pending.contains_key(&id))
            .unwrap_or(false)
    }

    fn mark_pending_timed_out(&self, id: i64) {
        if let Ok(mut pending) = self.pending.lock() {
            if let Some(request) = pending.get_mut(&id) {
                request.timed_out = true;
            }
        }
    }

    fn spawn_request_watchdog(
        &self,
        id: i64,
        method: String,
        timeout: std::time::Duration,
        scope_key: Option<String>,
        kill_on_timeout: bool,
    ) {
        let Some(client) = self.self_weak.get().and_then(Weak::upgrade) else {
            return;
        };
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if !client.has_pending(id) {
                return;
            }
            if scope_key.is_some() {
                client.begin_scoped_timeout_recovery(id, &method);
            } else if kill_on_timeout {
                client.kill_after_timeout(&method);
            }
        });
    }

    fn begin_scoped_timeout_recovery(&self, id: i64, method: &str) {
        // One scoped timeout makes the shared process state untrustworthy: a
        // late Roslyn response may already have committed an image that the
        // caller never loaded. Drain this client immediately so no other
        // checkout can successfully rely on registries that are about to be
        // discarded, then give the C# task a short cooperative-cancel grace.
        if self.unusable.swap(true, Ordering::AcqRel) {
            return;
        }
        self.mark_pending_timed_out(id);
        let reason = format!(
            "C# compile server scoped request '{method}' timed out; recovering shared sidecar"
        );
        eprintln!("[CsharpCompile] {reason}");
        super::notify_active_scope_loss(&reason);
        self.fail_all_pending(&reason);
        super::scheduler::clear_all_poisoned();
        super::emit_status_in_background();

        let Some(client) = self.self_weak.get().and_then(Weak::upgrade) else {
            return;
        };
        tokio::spawn(async move {
            let _ = tokio::time::timeout(
                CANCEL_SIGNAL_TIMEOUT,
                client.notify("$/cancelRequest", json!({ "id": id })),
            )
            .await;
            if crate::unity_hotreload::coordinator::total_active_patches().await > 0 {
                crate::unity_hotreload::note_sidecar_session_lost();
            }
            if let Ok(mut guard) = client.child.lock() {
                if let Some(child) = guard.as_mut() {
                    let _ = child.start_kill();
                }
            }
        });
    }

    async fn write_message(&self, message: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        frame.extend_from_slice(&body);
        let (completion, written) = oneshot::channel();
        if self
            .writer_tx
            .send(WriterCommand { frame, completion })
            .is_err()
        {
            let error = "C# compile server writer is unavailable".to_string();
            self.handle_writer_failure(&error);
            return Err(error);
        }
        match written.await {
            Ok(result) => result,
            Err(_) => {
                let error = "C# compile server writer stopped".to_string();
                self.handle_writer_failure(&error);
                Err(error)
            }
        }
    }

    fn handle_writer_failure(&self, error: &str) {
        if self.unusable.swap(true, Ordering::AcqRel) {
            return;
        }
        let reason = format!("{error}; recovering shared sidecar");
        eprintln!("[CsharpCompile] {reason}");
        super::notify_active_scope_loss(&reason);
        self.fail_all_pending(&reason);
        super::scheduler::clear_all_poisoned();
        super::emit_status_in_background();
        if let Ok(mut guard) = self.child.lock() {
            if let Some(child) = guard.as_mut() {
                let _ = child.start_kill();
            }
        }
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
        let scope_key = params.get("scopeId").and_then(|scope| {
            let checkout_id = scope.get("checkoutId")?.as_str()?.trim();
            let workspace_generation = scope.get("workspaceGeneration")?.as_u64()?;
            let service_generation = scope.get("serviceGeneration")?.as_u64()?;
            let editor_session_id = scope.get("unityEditorSessionId")?.as_str()?.trim();
            (!checkout_id.is_empty()
                && workspace_generation > 0
                && service_generation > 0
                && !editor_session_id.is_empty())
            .then(|| {
                format!(
                    "{checkout_id}\0{workspace_generation}\0{service_generation}\0{editor_session_id}"
                )
            })
        });
        let job_permit = match scope_key.as_ref() {
            Some(scope_key) if method == "scope/release" => {
                Some(super::scheduler::acquire_control(scope_key.clone()).await?)
            }
            Some(scope_key) => Some(super::scheduler::acquire(scope_key.clone()).await?),
            None => None,
        };
        if self.has_exited() {
            return Err("C# compile server is recovering".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(
                id,
                PendingRequest {
                    sender: tx,
                    scope_key: scope_key.clone(),
                    _job_permit: job_permit,
                    timed_out: false,
                },
            );
        }
        // Arm before the first await so aborting a caller during a blocked pipe
        // write cannot strand the pending entry or its compile-job lease.
        self.spawn_request_watchdog(
            id,
            method.to_string(),
            timeout,
            scope_key.clone(),
            kill_on_timeout,
        );
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(error) = self.write_message(&message).await {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }
        let mut rx = rx;
        match tokio::time::timeout(timeout, &mut rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("C# compile server dropped the request".to_string()),
            Err(_) => {
                let is_scoped = scope_key.is_some();
                if scope_key.is_some() {
                    // The synchronous Roslyn call may outlive the caller's
                    // timeout. Keep both the scheduler job lease and response
                    // registration alive until the server confirms completion;
                    // meanwhile reject new work for this scope explicitly.
                    self.begin_scoped_timeout_recovery(id, method);
                } else {
                    if let Ok(mut pending) = self.pending.lock() {
                        pending.remove(&id);
                    }
                }
                if kill_on_timeout && !is_scoped {
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
        super::notify_active_scope_loss(&reason);
        self.unusable.store(true, Ordering::Relaxed);
        self.fail_all_pending(&reason);
        super::scheduler::clear_all_poisoned();
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
