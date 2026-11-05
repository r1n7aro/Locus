//! Shared SSH stdio transport. Desktop configuration and tool execution stay with callers.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;
pub mod archive_pool;
mod archive_rpc;
mod heartbeat;
mod transfer;
pub mod diagnostics;
pub mod startup;
pub mod errors;
#[cfg(windows)]
pub mod pty_job;
pub const SSH_HOST_SOURCE: &str = include_str!("resources/ssh_host.py");
pub const NATIVE_MOD_SOURCE: &str = include_str!("resources/native_mod.js");
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct PendingCall {
    pub payload: Value,
    pub reply: oneshot::Sender<Value>,
    pub received_at: tokio::time::Instant,
}

/// Synchronous control receipts bypass the tool queue, exactly as on desktop.
#[derive(Clone)]
pub struct BridgeCalls {
    pub sender: mpsc::Sender<PendingCall>,
    pub route: Arc<dyn Fn(&Value) -> Option<Value> + Send + Sync>,
}

/// Resolved only when a macOS peer requests an unlock during the handshake.
pub type UnlockProvider = Arc<dyn Fn() -> Result<Option<Zeroizing<String>>, String> + Send + Sync>;
pub type StartupObserver = Arc<dyn Fn(&Value) + Send + Sync>;

const MAX_FRAME: usize = 32 * 1024 * 1024;
const PYTHON: &str = "exec python3 -u -c 'import base64,sys;_locus_source=base64.b64decode(sys.stdin.buffer.readline());exec(compile(_locus_source,\"<locus-ssh>\",\"exec\"))'";
const INSTALL_PYTHON: &str = "if ! command -v python3 >/dev/null 2>&1; then if [ \"$(id -u)\" = 0 ]; then SUDO=''; elif sudo -n true 2>/dev/null; then SUDO='sudo -n'; else echo 'Python 3 is missing; installation requires passwordless sudo or an administrator' >&2; exit 1; fi; if command -v apt-get >/dev/null 2>&1; then $SUDO apt-get update >&2 && $SUDO apt-get install -y python3 bash ca-certificates curl >&2 || exit 1; elif command -v dnf >/dev/null 2>&1; then $SUDO dnf install -y python3 bash ca-certificates curl >&2 || exit 1; elif command -v apk >/dev/null 2>&1; then $SUDO apk add python3 bash ca-certificates curl >&2 || exit 1; else echo 'Install Python 3 on this server first' >&2; exit 1; fi; fi; ";
const CHECK_PYTHON: &str = "if ! command -v python3 >/dev/null 2>&1; then printf '%s\\n' '{\"type\":\"hello\",\"protocol\":1,\"info\":{\"pythonInstalled\":false,\"cliSupported\":false,\"authenticated\":false}}'; exit 0; fi; ";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SshConfig {
    /// An OpenSSH config alias or user@hostname. Never a shell command.
    pub target: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub known_hosts_file: Option<String>,
    pub cli_path: Option<String>,
}
impl SshConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.target.is_empty()
            || self.target.starts_with('-')
            || self.target.len() > 512
            || !self
                .target
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.-@[]:%".contains(&c))
            || self.target.matches('@').count() > 1
            || self.port == Some(0)
        {
            return Err(
                "Enter an SSH alias or user@hostname, with the port in its own field".into(),
            );
        }
        for value in [&self.identity_file, &self.known_hosts_file, &self.cli_path]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.chars().any(char::is_control) {
                return Err("SSH paths must not be empty or contain control characters".into());
            }
        }
        Ok(())
    }
    pub fn fingerprint(&self) -> String {
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            &serde_json::to_vec(self).expect("SSH config serialization"),
        )
        .to_string()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    #[serde(default, skip_serializing)]
    pub history: Option<Value>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub keychain_unlocked: Option<bool>,
    #[serde(default)]
    pub keychain_unlock_tested: bool,
    pub python_installed: bool,
    pub cli_version: Option<String>,
    pub cli_supported: bool,
    pub model: Option<String>,
    pub authenticated: bool,
    pub home: Option<String>,
    pub session_root: Option<String>,
}
pub fn configured_command(config: &SshConfig) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("ssh");
    command.args(["-T", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
        "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2",
        "-o", "ClearAllForwardings=yes", "-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    if let Some(port) = config.port { command.args(["-p", &port.to_string()]); }
    if let Some(path) = &config.identity_file { command.args(["-i", path]); }
    if let Some(path) = &config.known_hosts_file { command.args(["-o", &format!("UserKnownHostsFile={path}")]); }
    command
}

pub struct Transport {
    trace: Arc<diagnostics::Trace>,
    #[cfg(windows)]
    job: pty_job::PtyJob,
    child: Mutex<tokio::process::Child>,
    send: mpsc::Sender<Value>,
    healthy: Arc<AtomicBool>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    pub diagnostic: Arc<Mutex<Vec<u8>>>,
    failure: Arc<Mutex<Option<String>>>,
    archive_rpc: Arc<archive_rpc::ArchiveRpc>,
    heartbeat: Arc<Mutex<heartbeat::Heartbeat>>,
}
impl Transport {
    pub fn failure(&self) -> String {
        let status = self.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait().ok().flatten();
        let error = self.failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
            .unwrap_or_else(|| errors::disconnected("SSH connection closed"));
        // The child watcher can observe exit before the reader gets scheduled.
        // Capture that failure too; intentional stop sets trace.stopped first.
        if status.is_some() || !self.healthy.load(Ordering::SeqCst) {
            self.trace.fail("closed", &error, status.and_then(|s| s.code()));
        }
        self.trace.observe_exit(status.and_then(|s| s.code()));
        error
    }
    pub fn latency_ms(&self) -> Option<f64> {
        if !self.alive() { return None; }
        self.heartbeat.lock().unwrap_or_else(|e| e.into_inner()).latency_ms(std::time::Instant::now())
    }
    /// Called when the hooks lease expires, not during environment bootstrap.
    /// A live SSH child or missing RTT is insufficient evidence either way.
    pub fn health_failure(&self) -> Option<String> {
        if !self.alive() { return Some(self.failure()); }
        let unresponsive = self.heartbeat.lock().unwrap_or_else(|e| e.into_inner())
            .unresponsive(std::time::Instant::now());
        unresponsive.then(|| {
            let error = errors::disconnected("SSH transport heartbeat received no response for 30s");
            self.trace.fail("heartbeat", &error, None);
            error
        })
    }
    pub fn alive(&self) -> bool {
        self.healthy.load(Ordering::SeqCst)
            && matches!(
                self.child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .try_wait(),
                Ok(None)
            )
    }
    pub fn stop(&self) {
        if self.trace.stopped.swap(true, Ordering::Relaxed) { return; }
        self.archive_rpc.close(&self.failure());
        self.healthy.store(false, Ordering::SeqCst);
        let _ = self.send.try_send(json!({"type":"close"}));
        #[cfg(windows)]
        self.job.terminate();
        let _ = self
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start_kill();
        // A stopped Session can remain in the cache until its next turn. Do not
        // retain its writer/reader tasks (or pending tool futures) until Drop.
        for (index, task) in self.tasks.iter().enumerate() {
            // On real failure only, drain already-written SSH stderr briefly.
            // Recovery and process termination do not wait for this, and all
            // tool/read/write/heartbeat tasks are still retired immediately.
            if index == 2 && self.trace.has_failed() {
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    let abort = task.abort_handle();
                    runtime.spawn(async move {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        abort.abort();
                    });
                    continue;
                }
            }
            task.abort();
        }
    }
    pub fn send(&self, value: Value) -> Result<(), String> {
        if !self.alive() {
            return Err(self.failure());
        }
        self.send
            .try_send(value)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Closed(_) => self.failure(),
                mpsc::error::TrySendError::Full(_) => "SSH input queue is full".into(),
            })
    }
    pub fn input(&self, text: &str) -> Result<(), String> {
        self.send(
            json!({"type":"input","data":base64::engine::general_purpose::STANDARD.encode(text)}),
        )
    }
    pub async fn start(
        config: &SshConfig,
        request: Value,
        calls: Option<BridgeCalls>,
        unlock_password: UnlockProvider,
    ) -> Result<(Self, RemoteInfo), String> {
        Self::start_observed(config,request,calls,unlock_password,None).await
    }
    pub async fn start_observed(
        config:&SshConfig,mut request:Value,calls:Option<BridgeCalls>,unlock_password:UnlockProvider,observer:Option<StartupObserver>,
    )->Result<(Self,RemoteInfo),String> {
        config.validate()?;
        if request.get("runtimeSettings").is_none() {
            request["runtimeSettings"] = json!({});
        }
        request["protocol"] = json!(1);
        request["cliPath"] = json!(config.cli_path);
        request["keychainUnlock"] = json!(true);
        let authorized = request["authorizedSetup"] == true;
        let archive_only = request["mode"] == "archive_read";
        let handshake = Arc::new(Mutex::new(startup::Handshake::new(&request)));
        let history_progress = (request["mode"] == "history").then(|| Arc::new(transfer::Progress::default()));
        let heartbeat = Arc::new(Mutex::new(heartbeat::Heartbeat::default()));
        let trace = diagnostics::Trace::new(config, &request, heartbeat.clone());
        let mut command = configured_command(config);
        // The remote shell reaching this marker proves SSH connected, including
        // when Python installation/bootstrap has not reached the helper yet.
        let bootstrap_stage = if matches!(request["mode"].as_str(), Some("history" | "archive" | "archive_read")) {
            "history"
        } else { "environment" };
        command.arg("--").arg(&config.target).arg(format!(
            "printf '%s\\n' '{{\"type\":\"progress\",\"protocol\":1,\"stage\":\"{bootstrap_stage}\",\"source\":\"shell\"}}'; {}{}",
            if authorized {
                INSTALL_PYTHON
            } else {
                CHECK_PYTHON
            },
            PYTHON
        ));
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|e| {
            let error = handshake.lock().unwrap_or_else(|e| e.into_inner())
                .context(format!("Could not start OpenSSH: {e}"));
            trace.fail("spawn", &error, None);
            error
        })?;
        if let Some(pid) = child.id() { trace.pid(pid); }
        #[cfg(windows)]
        let job = pty_job::PtyJob::attach(child.id().ok_or("SSH process ID unavailable")?)?;
        let mut stdin = child.stdin.take().ok_or("SSH stdin unavailable")?;
        let stdout = child.stdout.take().ok_or("SSH stdout unavailable")?;
        let mut stderr = child.stderr.take().ok_or("SSH stderr unavailable")?;
        let (send, mut receive) = mpsc::channel::<Value>(64);
        // Separate from ordinary frames so passwords cannot enter generic logs
        // or be retained as session input. The writer wipes its serialized bytes.
        let (unlock_send, mut unlock_receive) = mpsc::channel::<Option<Zeroizing<String>>>(1);
        let healthy = Arc::new(AtomicBool::new(true));
        let failure = Arc::new(Mutex::new(None));
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let error_buffer = diagnostic.clone();
        let stderr_trace = trace.clone();
        let stderr_task = tokio::spawn(async move {
            let mut bytes = [0; 8192];
            while let Ok(count) = stderr.read(&mut bytes).await {
                if count == 0 {
                    break;
                }
                stderr_trace.stderr(&bytes[..count]);
                append_diagnostic(&error_buffer, &bytes[..count]);
            }
        });
        let writer_health = healthy.clone();
        let writer_failure = failure.clone();
        let writer_trace = trace.clone();
        let writer_heartbeat = heartbeat.clone();
        let source = base64::engine::general_purpose::STANDARD.encode(SSH_HOST_SOURCE);
        let write_task = tokio::spawn(async move {
            let run = async {
                let bootstrap = format!("{source}\n{request}\n");
                stdin.write_all(bootstrap.as_bytes()).await?;
                writer_trace.sent(bootstrap.len());
                drop(bootstrap);
                stdin.flush().await?;
                loop {
                    let bytes = tokio::select! {
                        Some(password) = unlock_receive.recv() => {
                            #[derive(Serialize)]
                            struct Reply<'a> { r#type: &'static str, password: Option<&'a str> }
                            Zeroizing::new(serde_json::to_vec(&Reply {
                                r#type: "keychain_unlock", password: password.as_deref().map(String::as_str),
                            })?)
                        }
                        value = receive.recv() => {
                            let Some(value) = value else { break; };
                            if value["type"] == "ping" {
                                if let Some(id) = value["id"].as_u64() {
                                    writer_heartbeat.lock().unwrap_or_else(|e| e.into_inner()).sent(id, std::time::Instant::now());
                                }
                            }
                            Zeroizing::new(serde_json::to_vec(&value)?)
                        }
                    };
                    if bytes.len() > MAX_FRAME {
                        return Err(std::io::Error::other("SSH frame too large"));
                    }
                    stdin.write_all(&bytes).await?;
                    stdin.write_all(b"\n").await?;
                    writer_trace.sent(bytes.len() + 1);
                    stdin.flush().await?;
                }
                Ok::<_, std::io::Error>(())
            };
            if let Err(error) = run.await {
                let error = errors::disconnected(format!("SSH write failed: {error}"));
                writer_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(error.clone());
                if writer_trace.unexpected_end() { writer_trace.fail("write", &error, None); }
            }
            writer_health.store(false, Ordering::SeqCst);
        });
        let (hello_send, hello_receive) = oneshot::channel();
        let read_health = healthy.clone();
        let read_failure = failure.clone();
        let reply_send = send.clone();
        let read_buffer = diagnostic.clone();
        let archive_rpc=Arc::new(archive_rpc::ArchiveRpc::default());
        let read_archive_rpc=archive_rpc.clone();
        let read_heartbeat = heartbeat.clone();
        let read_handshake = handshake.clone();
        let read_history_progress = history_progress.clone();
        let read_trace = trace.clone();
        let read_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut hello = Some(hello_send);
            let mut unlock_requested = false;
            let mut pending = tokio::task::JoinSet::new();
            let mut next_transfer = None;
            loop {
                while pending.try_join_next().is_some() {}
                // Reading a frame is deliberately not raced against completed
                // replies: cancelling a partial read would discard its prefix.
                let mut incoming = next_transfer.take();
                let value = match read_frame_traced(&mut reader, Some(&read_trace), incoming.as_mut()).await {
                    Ok(Some(value)) => value,
                    result => {
                        let error = result.err().unwrap_or_else(|| errors::disconnected("SSH output closed"));
                        read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(error);
                        break;
                    }
                };
                read_trace.helper(&value["helper"]);
                if read_archive_rpc.route(&value) { continue; }
                match value["type"].as_str() {
                    Some("keychain_unlock_required") if hello.is_some()
                        && !unlock_requested && value["protocol"] == 1 && value["platform"] == "darwin" => {
                        unlock_requested = true;
                        // A Linux peer never asks for this and never loads the local secret.
                        match unlock_password() {
                            Ok(password) => { if unlock_send.send(password).await.is_err() { break; } }
                            Err(_) => {
                                if let Some(hello) = hello.take() {
                                    let _ = hello.send(Err("Could not read the saved macOS keychain password from this computer's credential store".into()));
                                }
                                break;
                            }
                        }
                    }
                    Some("hello") if value["protocol"] == 1 => {
                        if let Some(hello) = hello.take() {
                            let info = serde_json::from_value::<RemoteInfo>(value["info"].clone()).map_err(|e| e.to_string());
                            if info.is_ok() { read_trace.ready(); }
                            let _ = hello.send(info);
                        }
                    }
                    Some("terminal") => {
                        if let Some(data) = value["data"]
                            .as_str()
                            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
                        {
                            append_diagnostic(&read_buffer, &data);
                            if data.windows(4).any(|p| p == b"\x1b[6n") {
                                let _ = reply_send.try_send(json!({"type":"input","data":base64::engine::general_purpose::STANDARD.encode(b"\x1b[1;1R")}));
                            }
                        }
                    }
                    Some("bridge_health") => {
                        // One-way samples bypass tool queues and pending reply
                        // tasks. No remote receipt is needed for liveness: the
                        // lease advances only when this process receives it.
                        if value["payload"]["type"] == "bridge_health" {
                            if let Some(calls) = &calls {
                                (calls.route)(&value["payload"]);
                            }
                        }
                    }
                    Some("call") => {
                        // Each remote lane (the main agent, each native subagent)
                        // and each control event has at most one call in flight.
                        let Some(calls) = calls.clone().filter(|_| pending.len() < 256) else {
                            read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert("Unexpected or excessive SSH bridge calls".into());
                            break;
                        };
                        let Some(id) = value["id"].as_str().map(str::to_string) else {
                            read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert("SSH bridge call has no identity".into());
                            break;
                        };
                        let send = reply_send.clone();
                        pending.spawn(async move {
                            if let Some(result)=(calls.route)(&value["payload"]) {
                                let _=send.send(json!({"type":"reply","id":id,"value":result})).await;
                                return;
                            }
                            let (reply, receive) = oneshot::channel();
                            if calls.sender
                                .send(PendingCall {
                                    payload: value["payload"].clone(),
                                    reply,
                                    received_at: tokio::time::Instant::now(),
                                })
                                .await
                                .is_err()
                            {
                                return;
                            }
                            let result = receive.await.unwrap_or_else(
                                |_| json!({"bridgeError":"Locus tool handler closed"}),
                            );
                            let _ = send
                                .send(json!({"type":"reply","id":id,"value":result}))
                                .await;
                        });
                    }
                    Some("error") => {
                        let message = value["message"]
                            .as_str()
                            .unwrap_or("Remote Claude failed")
                            .to_string();
                        append_diagnostic(&read_buffer, message.as_bytes());
                        read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(message.clone());
                        if let Some(hello) = hello.take() {
                            let _ = hello.send(Err(message));
                        }
                        break;
                    }
                    Some("exit") => {
                        read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(||
                            errors::disconnected(format!("Remote Claude exited: {value}")));
                        break;
                    },
                    Some("pong") => {
                        if let Some(id) = value["id"].as_u64() {
                            read_heartbeat.lock().unwrap_or_else(|e| e.into_inner()).received(id, std::time::Instant::now());
                        }
                    }
                    Some("progress") => {
                        if let Some(observer)=&observer {observer(&value);}
                        read_trace.progress(&value);
                        read_handshake.lock().unwrap_or_else(|e| e.into_inner()).progress(&value);
                        let progress = if value["protocol"] == 1 && value["stage"] == "history_transfer" {
                            read_history_progress.as_ref().map(|progress|
                                transfer::announced_size(&value).and_then(|size|
                                    transfer::FrameProgress::new(progress.clone(), size, None))).transpose()
                        } else { read_archive_rpc.frame_progress(&value) };
                        match progress {
                            Ok(progress) => next_transfer = progress,
                            Err(error) => {
                                read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(error);
                                break;
                            }
                        }
                    }
                    _ => {
                        read_failure.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert("Unexpected SSH protocol frame".into());
                        break;
                    },
                }
            }
            read_health.store(false, Ordering::SeqCst);
            let error = read_failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
                .unwrap_or_else(|| errors::disconnected("SSH reader stopped"));
            if calls.is_some() { eprintln!("[Claude] transport ended: {error}"); }
            if read_trace.unexpected_end() { read_trace.fail("closed", &error, None); }
            read_archive_rpc.close(&error);
        });
        let heartbeat_health = healthy.clone();
        let heartbeat_send = send.clone();
        let heartbeat_archive = archive_rpc.clone();
        let idle_heartbeat = heartbeat.clone();
        let heartbeat_task = tokio::spawn(async move {
            let mut id = 0_u64;
            while heartbeat_health.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if archive_only && heartbeat_archive.busy() {
                    idle_heartbeat.lock().unwrap_or_else(|e| e.into_inner()).suspend();
                    continue;
                }
                id += 1;
                if heartbeat_send.send(json!({"type":"ping","id":id})).await.is_err() {
                    break;
                }
            }
        });
        let mut transport = Self {
            trace,
            archive_rpc,
            heartbeat,
            #[cfg(windows)]
            job,
            child: Mutex::new(child),
            send,
            healthy,
            tasks: vec![read_task, write_task, stderr_task, heartbeat_task],
            diagnostic,
            failure,
        };
        let timeout = Duration::from_secs(if authorized { crate::claude_launch::AUTHORIZED_STARTUP_SECONDS } else { 45 });
        let response = if let Some(progress) = history_progress {
            progress.wait(hello_receive, transfer::Limits::history(timeout)).await
        } else {
            tokio::time::timeout(timeout, hello_receive).await.map_err(|_| transfer::Timeout::Initial(timeout))
        };
        let info = match response {
            Ok(Ok(result)) => match result {
                Ok(info) => info,
                Err(error) => {
                    let error = handshake.lock().unwrap_or_else(|e| e.into_inner()).context(error);
                    transport.trace.fail("remote", &error, None);
                    return Err(error);
                }
            },
            result => {
                // stdout EOF can precede delivery of OpenSSH's stderr verdict.
                // Drain it briefly so permission/host-key failures are not
                // mislabeled as an unexplained bridge closure.
                let _ = tokio::time::timeout(Duration::from_millis(200), &mut transport.tasks[2]).await;
                let diagnostic = transport.diagnostic.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let failure = transport.failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
                    .or_else(|| result.as_ref().err().and_then(|timeout| match timeout {
                        transfer::Timeout::Initial(_) => None, // retain startup/auth diagnostics
                        timeout => Some(errors::disconnected(timeout.message("Native history transfer"))),
                    }));
                let error = handshake.lock().unwrap_or_else(|e| e.into_inner())
                    .failed(result.is_err(), timeout, failure, &diagnostic);
                let exit_code = transport.child.lock().unwrap_or_else(|e| e.into_inner()).try_wait().ok().flatten().and_then(|s| s.code());
                transport.trace.fail(if result.is_err() { "timeout" } else { "closed" }, &error, exit_code);
                return Err(error);
            }
        };
        Ok((transport, info))
    }
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}
fn append_diagnostic(buffer: &Mutex<Vec<u8>>, bytes: &[u8]) {
    let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
    buffer.extend_from_slice(bytes);
    if buffer.len() > 65536 {
        let excess = buffer.len() - 65536;
        buffer.drain(..excess);
    }
}
#[cfg(test)]
async fn read_frame(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> Result<Option<Value>, String> {
    read_frame_traced(reader, None, None).await
}
async fn read_frame_traced(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
    trace: Option<&diagnostics::Trace>,
    mut progress: Option<&mut transfer::FrameProgress>,
) -> Result<Option<Value>, String> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|e| errors::disconnected(format!("SSH read failed: {e}")))?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(errors::disconnected("Truncated SSH frame"))
            };
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(available.len());
        if bytes.len() + count > MAX_FRAME {
            return Err("SSH frame exceeds size limit".into());
        }
        let complete = available[count - 1] == b'\n';
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count);
        if let Some(trace) = trace { trace.received(count, bytes.len()); }
        if let Some(progress) = progress.as_mut() { progress.received(count)?; }
        if complete {
            if let Some(trace) = trace { trace.frame_complete(); }
            let value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if let Some(progress) = progress { progress.complete(&value)?; }
            return Ok(Some(value));
        }
    }
}


pub fn valid_name(name:&str)->bool {
    if name=="transcript.jsonl" { return true; }
    if name.contains('\\') || name.split('/').any(|p|p.is_empty()||p=="."||p=="..") {return false;}
    let parts:Vec<_>=name.split('/').collect();
    match parts.as_slice() {
        ["tasks"|"dispatch",file]=>file.strip_suffix(".json").is_some_and(|v|!v.is_empty() && v.len()<=128 && v.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c))),
        // Native subagent transcripts and their metadata (ssh_host.py archive_name).
        ["subagents",file]=>file.strip_prefix("agent-").and_then(|v|v.strip_suffix(".jsonl").or_else(||v.strip_suffix(".meta.json")))
            .is_some_and(|v|!v.is_empty() && v.len()<=128 && v.bytes().all(|c|c.is_ascii_alphanumeric()||b"_-".contains(&c))),
        ["attachments",input,file]=>input.strip_prefix("LOCUS_INPUT_").is_some_and(|v|v.len()==32&&v.bytes().all(|c|c.is_ascii_hexdigit()))
            && file.split_once('.').is_some_and(|(stem,ext)|matches!(ext,"png"|"jpg"|"gif"|"webp"|"txt") && stem.split_once('-').is_some_and(|(kind,n)|matches!(kind,"image"|"file")&&!n.is_empty()&&n.bytes().all(|b|b.is_ascii_digit()))),
        _=>false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> SshConfig {
        SshConfig { target: "user@host".into(), port: None, identity_file: None, known_hosts_file: None, cli_path: None }
    }
    #[tokio::test]
    async fn partial_history_bytes_remain_observable_before_a_frame_completes() {
        let trace = diagnostics::Trace::new(&config(), &json!({"mode":"history"}), Arc::new(Mutex::new(heartbeat::Heartbeat::default())));
        trace.progress(&json!({"type":"progress","protocol":1,"stage":"history_transfer","bytes":1000}));
        let (mut writer, reader) = tokio::io::duplex(128);
        writer.write_all(b"{\"info\":").await.unwrap();
        let mut reader = BufReader::new(reader);
        assert!(tokio::time::timeout(Duration::from_millis(10), read_frame_traced(&mut reader, Some(&trace), None)).await.is_err());
        let status = trace.snapshot();
        assert_eq!(status.partial_frame_bytes, 8);
        assert_eq!(status.received_bytes, 8);
        assert_eq!(status.expected_frame_bytes, Some(1000));
        assert!(status.last_receive_age_ms.is_some());
    }
    #[tokio::test]
    async fn frames_reject_truncation_and_invalid_json() {
        assert_eq!(
            read_frame(&mut &b"{\"type\":\"pong\"}\n"[..])
                .await
                .unwrap()
                .unwrap()["type"],
            "pong"
        );
        let truncated = read_frame(&mut &b"{}"[..]).await.unwrap_err();
        assert!(errors::is_disconnected(&truncated));
        let malformed = read_frame(&mut &b"oops\n"[..]).await.unwrap_err();
        assert!(!errors::is_disconnected(&malformed));
    }
    #[test]
    fn archives_keep_native_subagent_transcripts_and_metadata_only() {
        for name in ["subagents/agent-a92abd40066ed59f2.jsonl", "subagents/agent-a92abd40066ed59f2.meta.json"] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["subagents/agent-.jsonl", "subagents/a92abd.jsonl", "subagents/agent-a1.json", "subagents/agent-a1.jsonl.tmp",
            "subagents/x/agent-a1.jsonl", "subagents/agent-../a.jsonl"] {
            assert!(!valid_name(name), "{name}");
        }
    }
}
