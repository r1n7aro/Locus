//! JSON-RPC / LSP client for the Roslyn C# language server.
//!
//! Speaks LSP over the child process' stdio using `Content-Length` framing.
//! Protocol notes learned from probing `Microsoft.CodeAnalysis.LanguageServer`:
//!
//! - `--logLevel` and `--extensionLogDirectory` are required CLI arguments;
//!   `--stdio` selects stdio transport (default is a named-pipe handshake).
//! - The server sends `client/registerCapability`, `workspace/configuration`
//!   and `window/workDoneProgress/create` requests during startup; every one
//!   of them must receive a response or loading stalls.
//! - A request or notification with `"params": null` crashes the server's
//!   StreamJsonRpc reader (`Unexpected value kind: Null`); always send an
//!   object/array payload.
//! - Project loading is reported via the non-standard
//!   `workspace/projectInitializationComplete` notification after a
//!   `solution/open` / `project/open` notification.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, watch, Mutex as AsyncMutex};

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// A single running language-server process plus the JSON-RPC plumbing.
pub struct LspClient {
    stdin: AsyncMutex<tokio::process::ChildStdin>,
    child: Mutex<Option<tokio::process::Child>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    next_id: AtomicI64,
    project_loaded_rx: watch::Receiver<bool>,
    exited_rx: watch::Receiver<bool>,
    /// Count of distinct project files the loader has reported activity for —
    /// the closest thing the server offers to a load progress signal.
    project_progress_rx: watch::Receiver<u32>,
    project_seen: Mutex<std::collections::HashSet<String>>,
    last_server_error: Mutex<Option<String>>,
    /// Open document state: uri -> (version, blake3 content hash).
    open_docs: Mutex<HashMap<String, (i32, [u8; 32])>>,
}

impl LspClient {
    /// Spawn the server process and start the stdout reader loop.
    ///
    /// `program` + `args` should already include `--stdio`; `stderr_log` is a
    /// file path that receives the server's stderr stream.
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
            .map_err(|e| format!("Failed to start C# language server: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "C# language server stdin unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "C# language server stdout unavailable".to_string())?;

        let (loaded_tx, loaded_rx) = watch::channel(false);
        let (exited_tx, exited_rx) = watch::channel(false);
        let (progress_tx, progress_rx) = watch::channel(0u32);

        let client = std::sync::Arc::new(LspClient {
            stdin: AsyncMutex::new(stdin),
            child: Mutex::new(Some(child)),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicI64::new(0),
            project_loaded_rx: loaded_rx,
            exited_rx,
            project_progress_rx: progress_rx,
            project_seen: Mutex::new(std::collections::HashSet::new()),
            last_server_error: Mutex::new(None),
            open_docs: Mutex::new(HashMap::new()),
        });

        let reader_client = std::sync::Arc::clone(&client);
        tokio::spawn(async move {
            reader_client.read_loop(stdout, loaded_tx, progress_tx).await;
            let _ = exited_tx.send(true);
            reader_client.fail_all_pending("C# language server exited");
        });

        Ok(client)
    }

    pub fn has_exited(&self) -> bool {
        *self.exited_rx.borrow()
    }

    pub fn last_server_error(&self) -> Option<String> {
        self.last_server_error
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub fn open_document_count(&self) -> usize {
        self.open_docs.lock().map(|docs| docs.len()).unwrap_or(0)
    }

    /// Wait until the server reports `workspace/projectInitializationComplete`,
    /// invoking `on_progress` with the running distinct-project count as the
    /// loader works through the solution.
    pub async fn wait_project_loaded(
        &self,
        timeout: std::time::Duration,
        mut on_progress: impl FnMut(u32),
    ) -> bool {
        let mut loaded = self.project_loaded_rx.clone();
        let mut exited = self.exited_rx.clone();
        let mut progress = self.project_progress_rx.clone();
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            if *loaded.borrow() {
                return true;
            }
            if *exited.borrow() {
                return false;
            }
            tokio::select! {
                _ = loaded.changed() => {}
                _ = exited.changed() => {}
                changed = progress.changed() => {
                    if changed.is_ok() {
                        let completed = *progress.borrow();
                        on_progress(completed);
                    }
                }
                _ = &mut deadline => return *loaded.borrow(),
            }
        }
    }

    /// Record loader activity for a project file mentioned in a log line.
    /// Locale-independent: keys off the `.csproj` path tokens rather than the
    /// (localized) message template.
    fn track_project_activity(&self, message: &str, progress_tx: &watch::Sender<u32>) {
        if !message.contains("[LanguageServerProjectLoader]") || !message.contains(".csproj") {
            return;
        }
        let mut updated_count = None;
        if let Ok(mut seen) = self.project_seen.lock() {
            for token in message.split(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
                let token = token
                    .trim_end_matches(|c: char| !c.is_ascii_alphanumeric())
                    .to_ascii_lowercase();
                if !token.ends_with(".csproj") {
                    continue;
                }
                let name = token
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(&token)
                    .to_string();
                if seen.insert(name) {
                    updated_count = Some(seen.len() as u32);
                }
            }
        }
        if let Some(count) = updated_count {
            let _ = progress_tx.send(count);
        }
    }

    async fn read_loop(
        &self,
        stdout: tokio::process::ChildStdout,
        loaded_tx: watch::Sender<bool>,
        progress_tx: watch::Sender<u32>,
    ) {
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
            self.dispatch(message, &loaded_tx, &progress_tx).await;
        }
    }

    async fn dispatch(
        &self,
        message: Value,
        loaded_tx: &watch::Sender<bool>,
        progress_tx: &watch::Sender<u32>,
    ) {
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
                            "server error {}: {}",
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
            // Server -> client request: must always be answered.
            (Some(id), Some(method)) => {
                let response = match method {
                    "workspace/configuration" => {
                        // Default everything to null (server-side defaults),
                        // except automatic NuGet restore: Unity-generated
                        // csproj contain no restorable packages, yet the
                        // per-project `dotnet restore` sweep dominated load
                        // time (measured 48s -> 4.4s on a 107-project
                        // solution with it disabled).
                        let values: Vec<Value> = message
                            .get("params")
                            .and_then(|p| p.get("items"))
                            .and_then(|items| items.as_array())
                            .map(|items| {
                                items
                                    .iter()
                                    .map(|item| {
                                        match item.get("section").and_then(|s| s.as_str()) {
                                            Some("projects.dotnet_enable_automatic_restore") => {
                                                Value::Bool(false)
                                            }
                                            _ => Value::Null,
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        json!({ "jsonrpc": "2.0", "id": id, "result": values })
                    }
                    "client/registerCapability" | "client/unregisterCapability" => {
                        json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null })
                    }
                    "window/workDoneProgress/create" => {
                        json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null })
                    }
                    "workspace/applyEdit" => {
                        json!({ "jsonrpc": "2.0", "id": id, "result": { "applied": false } })
                    }
                    _ => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("method not handled: {method}") }
                    }),
                };
                let _ = self.write_message(&response).await;
            }
            // Notification from the server.
            (None, Some(method)) => match method {
                "workspace/projectInitializationComplete" => {
                    let _ = loaded_tx.send(true);
                }
                "window/logMessage" | "window/showMessage" => {
                    let params = message.get("params");
                    let kind = params
                        .and_then(|p| p.get("type"))
                        .and_then(|t| t.as_i64())
                        .unwrap_or(4);
                    let text = params
                        .and_then(|p| p.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or_default();
                    self.track_project_activity(text, progress_tx);
                    // 1 = Error, 2 = Warning.
                    if kind == 1 && !text.is_empty() {
                        if let Ok(mut guard) = self.last_server_error.lock() {
                            let mut snippet: String = text.chars().take(400).collect();
                            if text.chars().count() > 400 {
                                snippet.push('…');
                            }
                            *guard = Some(snippet);
                        }
                    }
                }
                _ => {}
            },
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
            .map_err(|e| format!("C# language server write failed: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("C# language server flush failed: {e}"))?;
        Ok(())
    }

    /// Send a request and await its response. `params` must never be
    /// `Value::Null` (the server's JSON-RPC reader rejects it).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
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
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("C# language server dropped the request".to_string()),
            Err(_) => {
                if let Ok(mut pending) = self.pending.lock() {
                    pending.remove(&id);
                }
                Err(format!("C# language server request '{method}' timed out"))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.write_message(&message).await
    }

    /// Run the LSP `initialize` handshake and open the given solution or
    /// project files.
    pub async fn initialize_workspace(
        &self,
        workspace_root: &Path,
        project: &super::ProjectTarget,
    ) -> Result<(), String> {
        let root_uri = path_to_uri(workspace_root)?;
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "workspace": {
                    "configuration": true,
                    "didChangeWatchedFiles": { "dynamicRegistration": true },
                    "workspaceFolders": true,
                    "symbol": {}
                },
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "references": {},
                    "definition": {},
                    "hover": { "contentFormat": ["plaintext", "markdown"] }
                },
                "window": { "workDoneProgress": true }
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": workspace_root
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".to_string())
            }]
        });
        self.request("initialize", init_params).await?;
        self.notify("initialized", json!({})).await?;

        match project {
            super::ProjectTarget::Solution(path) => {
                self.notify("solution/open", json!({ "solution": path_to_uri(path)? }))
                    .await?;
            }
            super::ProjectTarget::Projects(paths) => {
                let uris = paths
                    .iter()
                    .map(|p| path_to_uri(p))
                    .collect::<Result<Vec<_>, _>>()?;
                self.notify("project/open", json!({ "projects": uris }))
                    .await?;
            }
        }
        Ok(())
    }

    /// Make sure the server sees the current on-disk content of `path`.
    /// Opens the document on first use and pushes a full-text change when the
    /// content hash differs from what was last sent.
    pub async fn sync_document(&self, path: &Path) -> Result<String, String> {
        let uri = path_to_uri(path)?;
        let text = read_text_lossy(path)?;
        let hash = *blake3::hash(text.as_bytes()).as_bytes();

        enum SyncAction {
            None,
            Open,
            Change(i32),
        }

        let action = {
            let mut docs = self
                .open_docs
                .lock()
                .map_err(|_| "document state poisoned".to_string())?;
            match docs.get_mut(&uri) {
                None => {
                    docs.insert(uri.clone(), (1, hash));
                    SyncAction::Open
                }
                Some((version, stored)) if *stored != hash => {
                    *version += 1;
                    *stored = hash;
                    SyncAction::Change(*version)
                }
                Some(_) => SyncAction::None,
            }
        };

        match action {
            SyncAction::None => {}
            SyncAction::Open => {
                self.notify(
                    "textDocument/didOpen",
                    json!({
                        "textDocument": {
                            "uri": uri,
                            "languageId": "csharp",
                            "version": 1,
                            "text": text
                        }
                    }),
                )
                .await?;
            }
            SyncAction::Change(version) => {
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": { "uri": uri, "version": version },
                        "contentChanges": [{ "text": text }]
                    }),
                )
                .await?;
            }
        }
        Ok(uri)
    }

    /// Re-sync a document only when it is already open. Open documents shadow
    /// the disk state on the server side, so external edits must be pushed —
    /// but files we never opened are the server's own business (it follows
    /// `didChangeWatchedFiles`), and opening them here would pin stale copies.
    pub async fn sync_document_if_open(&self, path: &Path) -> Result<(), String> {
        let uri = path_to_uri(path)?;
        let is_open = self
            .open_docs
            .lock()
            .map(|docs| docs.contains_key(&uri))
            .unwrap_or(false);
        if !is_open {
            return Ok(());
        }
        self.sync_document(path).await.map(|_| ())
    }

    /// Close a document we previously opened so the server falls back to the
    /// (possibly deleted) disk state instead of our pinned copy.
    pub async fn close_document_if_open(&self, path: &Path) -> Result<(), String> {
        let uri = path_to_uri(path)?;
        let was_open = self
            .open_docs
            .lock()
            .map(|mut docs| docs.remove(&uri).is_some())
            .unwrap_or(false);
        if !was_open {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    /// Forward file events so the server refreshes non-open documents and
    /// project files. `changes` items are `(uri, kind)` with LSP change kinds
    /// (1 created / 2 changed / 3 deleted).
    pub async fn notify_watched_files(&self, changes: Vec<(String, u8)>) -> Result<(), String> {
        if changes.is_empty() {
            return Ok(());
        }
        let payload: Vec<Value> = changes
            .into_iter()
            .map(|(uri, kind)| json!({ "uri": uri, "type": kind }))
            .collect();
        self.notify(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": payload }),
        )
        .await
    }

    /// Synchronous best-effort kill for app-exit paths where no async
    /// runtime is available. The graceful path is `shutdown`.
    pub fn kill_process(&self) {
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

/// Read a text file as UTF-8 (lossy), stripping a leading BOM so that LSP
/// positions line up with what editors and the server itself use.
pub fn read_text_lossy(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if text.starts_with('\u{feff}') {
        text.remove(0);
    }
    Ok(text)
}

pub fn path_to_uri(path: &Path) -> Result<String, String> {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .map_err(|_| format!("Cannot convert path to URI: {}", path.display()))
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.scheme() != "file" {
        return None;
    }
    parsed.to_file_path().ok().map(|p| dunce::simplified(&p).to_path_buf())
}
