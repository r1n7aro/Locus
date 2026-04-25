use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use super::{get_pipe_name, PipeResponse};

// ── Windows: named-pipe transport ────────────────────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicU64, Ordering},
            OnceLock,
        },
    };
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
        net::windows::named_pipe::{ClientOptions, NamedPipeClient},
        sync::oneshot,
    };

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub(super) struct PipeEnvelope {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<String>,

        #[serde(default, rename = "reply_to", skip_serializing_if = "Option::is_none")]
        pub reply_to: Option<String>,

        #[serde(default, rename = "type")]
        pub kind: String,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub ok: Option<bool>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub message: Option<String>,

        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub error: Option<String>,
    }

    struct UnityPipeConnection {
        pipe_name: String,
        writer: Mutex<WriteHalf<NamedPipeClient>>,
        pending: Mutex<HashMap<String, oneshot::Sender<Result<PipeEnvelope, String>>>>,
    }

    static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<UnityPipeConnection>>>> =
        OnceLock::new();
    static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

    fn connections() -> &'static Mutex<HashMap<String, Arc<UnityPipeConnection>>> {
        CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn next_request_id() -> String {
        format!("req-{}", REQUEST_SEQ.fetch_add(1, Ordering::Relaxed))
    }

    async fn open_client_with_retry(pipe_name: &str) -> Result<NamedPipeClient, String> {
        const MAX_RETRIES: u32 = 5;
        const ERROR_PIPE_BUSY: i32 = 231;

        let mut last_err = String::new();

        for attempt in 0..MAX_RETRIES {
            match ClientOptions::new().open(pipe_name) {
                Ok(client) => return Ok(client),
                Err(e)
                    if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempt + 1 < MAX_RETRIES =>
                {
                    last_err = format!("Failed to connect to Unity Editor ({}): {}", pipe_name, e);
                    tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1))).await;
                }
                Err(e) => {
                    return Err(format!(
                        "Failed to connect to Unity Editor ({}): {}",
                        pipe_name, e
                    ));
                }
            }
        }

        Err(last_err)
    }

    async fn remove_connection_if_same(pipe_name: &str, conn: &Arc<UnityPipeConnection>) {
        let mut map = connections().lock().await;
        if map
            .get(pipe_name)
            .map(|existing| Arc::ptr_eq(existing, conn))
            .unwrap_or(false)
        {
            map.remove(pipe_name);
        }
    }

    async fn fail_all_pending(conn: &Arc<UnityPipeConnection>, reason: String) {
        let mut pending = conn.pending.lock().await;
        for (_, tx) in pending.drain() {
            let _ = tx.send(Err(reason.clone()));
        }
    }

    fn handle_unsolicited_message(env: &PipeEnvelope) {
        eprintln!(
            "[Locus] unsolicited Unity message: type={}, message={:?}, error={:?}",
            env.kind, env.message, env.error
        );
    }

    async fn reader_loop(conn: Arc<UnityPipeConnection>, reader: ReadHalf<NamedPipeClient>) {
        let pipe_name = conn.pipe_name.clone();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();

            let n = match reader.read_line(&mut line).await {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("[Locus] pipe read error ({}): {}", pipe_name, e);
                    break;
                }
            };

            if n == 0 {
                eprintln!("[Locus] pipe disconnected: {}", pipe_name);
                break;
            }

            let trimmed = line.trim().trim_start_matches('\u{FEFF}');
            if trimmed.is_empty() {
                continue;
            }

            let env: PipeEnvelope = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!(
                        "[Locus] failed to parse pipe message ({}): {} | raw={}",
                        pipe_name, e, trimmed
                    );
                    continue;
                }
            };

            if let Some(reply_to) = env.reply_to.clone() {
                let tx = {
                    let mut pending = conn.pending.lock().await;
                    pending.remove(&reply_to)
                };

                if let Some(tx) = tx {
                    let _ = tx.send(Ok(env));
                } else {
                    eprintln!(
                        "[Locus] received response for unknown request id: {}",
                        reply_to
                    );
                }
            } else {
                handle_unsolicited_message(&env);
            }
        }

        remove_connection_if_same(&pipe_name, &conn).await;
        fail_all_pending(&conn, format!("Unity pipe disconnected: {}", pipe_name)).await;
    }

    async fn get_or_connect(project_path: &str) -> Result<Arc<UnityPipeConnection>, String> {
        let pipe_name = get_pipe_name(project_path);

        {
            let map = connections().lock().await;
            if let Some(conn) = map.get(&pipe_name) {
                return Ok(conn.clone());
            }
        }

        let client = open_client_with_retry(&pipe_name).await?;
        let (reader, writer) = tokio::io::split(client);

        let new_conn = Arc::new(UnityPipeConnection {
            pipe_name: pipe_name.clone(),
            writer: Mutex::new(writer),
            pending: Mutex::new(HashMap::new()),
        });

        {
            let mut map = connections().lock().await;
            if let Some(existing) = map.get(&pipe_name) {
                return Ok(existing.clone());
            }
            map.insert(pipe_name.clone(), new_conn.clone());
        }

        tokio::spawn(reader_loop(new_conn.clone(), reader));
        Ok(new_conn)
    }

    async fn send_message_inner(
        project_path: &str,
        msg_type: &str,
        message: &str,
        timeout: Option<Duration>,
    ) -> Result<PipeResponse, String> {
        let conn = get_or_connect(project_path).await?;
        let request_id = next_request_id();

        let env = PipeEnvelope {
            id: Some(request_id.clone()),
            reply_to: None,
            kind: msg_type.to_string(),
            ok: None,
            message: Some(message.to_string()),
            error: None,
        };

        let json =
            serde_json::to_string(&env).map_err(|e| format!("Serialization failed: {}", e))?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = conn.pending.lock().await;
            pending.insert(request_id.clone(), tx);
        }

        let write_result = async {
            let mut writer = conn.writer.lock().await;
            writer
                .write_all(json.as_bytes())
                .await
                .map_err(|e| format!("Pipe write failed: {}", e))?;
            writer
                .write_all(b"\n")
                .await
                .map_err(|e| format!("Newline write failed: {}", e))?;
            writer
                .flush()
                .await
                .map_err(|e| format!("Pipe flush failed: {}", e))
        }
        .await;

        if let Err(err) = write_result {
            {
                let mut pending = conn.pending.lock().await;
                pending.remove(&request_id);
            }
            remove_connection_if_same(&conn.pipe_name, &conn).await;
            return Err(err);
        }

        let env = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(Ok(env))) => env,
                Ok(Ok(Err(e))) => return Err(e),
                Ok(Err(_)) => {
                    return Err("Unity response failed: response channel closed".to_string())
                }
                Err(_) => {
                    let mut pending = conn.pending.lock().await;
                    pending.remove(&request_id);
                    return Err("Unity response timed out".to_string());
                }
            }
        } else {
            match rx.await {
                Ok(Ok(env)) => env,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err("Unity response failed: response channel closed".to_string()),
            }
        };

        Ok(PipeResponse {
            ok: env.ok.unwrap_or(false),
            error: env.error,
            message: env.message,
        })
    }

    pub async fn send_message_with_timeout(
        project_path: &str,
        msg_type: &str,
        message: &str,
        timeout: Duration,
    ) -> Result<PipeResponse, String> {
        send_message_inner(project_path, msg_type, message, Some(timeout)).await
    }

    pub async fn send_message_without_timeout(
        project_path: &str,
        msg_type: &str,
        message: &str,
    ) -> Result<PipeResponse, String> {
        send_message_inner(project_path, msg_type, message, None).await
    }

    pub async fn send_message(
        project_path: &str,
        msg_type: &str,
        message: &str,
    ) -> Result<PipeResponse, String> {
        send_message_with_timeout(project_path, msg_type, message, Duration::from_secs(35)).await
    }

    pub async fn disconnect(project_path: &str) {
        let pipe_name = get_pipe_name(project_path);
        let mut map = connections().lock().await;
        if let Some(conn) = map.remove(&pipe_name) {
            fail_all_pending(&conn, "disconnected for recompile".to_string()).await;
        }
    }
}

// ── Public dispatch ──────────────────────────────────────────────────

#[cfg(target_os = "windows")]
pub async fn send_message(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    windows_impl::send_message(project_path, msg_type, message).await
}

#[cfg(target_os = "windows")]
pub async fn send_message_with_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
    timeout: Duration,
) -> Result<PipeResponse, String> {
    windows_impl::send_message_with_timeout(project_path, msg_type, message, timeout).await
}

#[cfg(target_os = "windows")]
pub async fn send_message_without_timeout(
    project_path: &str,
    msg_type: &str,
    message: &str,
) -> Result<PipeResponse, String> {
    windows_impl::send_message_without_timeout(project_path, msg_type, message).await
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message_with_timeout(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
    _timeout: Duration,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(not(target_os = "windows"))]
pub async fn send_message_without_timeout(
    _project_path: &str,
    _msg_type: &str,
    _message: &str,
) -> Result<PipeResponse, String> {
    Err("Unity bridge is only supported on Windows (named pipes)".to_string())
}

#[cfg(target_os = "windows")]
pub async fn disconnect(project_path: &str) {
    windows_impl::disconnect(project_path).await;
}

#[cfg(not(target_os = "windows"))]
pub async fn disconnect(_project_path: &str) {}
