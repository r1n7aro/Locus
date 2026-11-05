//! Internal Locus Server adapter. Existing Claude event and local-tool consumers
//! are reused; only the bounded transport operations differ from direct SSH.
use super::mod_bridge::{ControlSignals, PendingCall};
use futures::{SinkExt, StreamExt};
use locus_remote_core::claude_launch::{
    BridgeError, Capabilities, ClientMessage, LaunchOptions, BRIDGE_STARTUP_SECONDS,
    PROTOCOL_VERSION,
};
use locus_remote_core::claude_ssh::RemoteInfo;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

pub struct Transport {
    send: mpsc::Sender<Value>,
    healthy: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
    pub diagnostic: Arc<Mutex<Vec<u8>>>,
    failure: Arc<Mutex<Option<BridgeError>>>,
    latency: Arc<Mutex<Option<f64>>>,
    binding_id: String,
    client_session_id: String,
}
impl Transport {
    pub async fn start(
        model: &str,
        client_session_id: &str,
        resume: Option<&str>,
        initial: Value,
        launch: LaunchOptions,
        prewarm: bool,
        calls: mpsc::Sender<PendingCall>,
        controls: Arc<ControlSignals>,
        options: Arc<std::sync::RwLock<Value>>,
    ) -> Result<(Self, RemoteInfo), String> {
        let (binding, model) =
            crate::server_client::binding_for_model(model)?.ok_or("Missing server binding")?;
        if binding.kind != "claude" {
            return Err("This server binding is not a Claude connection".into());
        }
        let connection = crate::server_client::credential_for_binding(&binding.id).await?;
        Self::start_bound(
            connection,
            &model,
            client_session_id,
            resume,
            initial,
            launch,
            prewarm,
            calls,
            controls,
            options,
        )
        .await
    }
    async fn start_bound(
        connection: crate::server_client::Connection,
        model: &str,
        client_session_id: &str,
        resume: Option<&str>,
        initial: Value,
        launch: LaunchOptions,
        prewarm: bool,
        calls: mpsc::Sender<PendingCall>,
        controls: Arc<ControlSignals>,
        options: Arc<std::sync::RwLock<Value>>,
    ) -> Result<(Self, RemoteInfo), String> {
        let mut url = url::Url::parse(&connection.server_url).map_err(|_| "Invalid server URL")?;
        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .map_err(|_| "Invalid server URL")?;
        url.set_path(&format!("/api/claude/{}/ws", connection.upstream_id));
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| "Invalid server connection")?;
        let credential = zeroize::Zeroizing::new(connection.access_token);
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {}", credential.as_str())
                .parse()
                .map_err(|_| "Invalid access token")?,
        );
        let (mut socket, _) = tokio::time::timeout(
            Duration::from_secs(20),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .map_err(|_| "Locus Server connection timed out")?
        .map_err(|_| "Could not connect to Locus Server; check your login and upstream access")?;
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let announcement = tokio::time::timeout(Duration::from_secs(10), receive_json(&mut socket))
            .await
            .map_err(|_| "Claude Server capability handshake timed out; update the server")??;
        let server_caps: Capabilities =
            serde_json::from_value(announcement["capabilities"].clone()).map_err(|_| {
                "Claude Server capability protocol is unavailable; update the server"
            })?;
        Capabilities::current().validate_peer(&server_caps)?;
        let start = ClientMessage::Start {
            capabilities: Capabilities::current(),
            launch,
            prewarm,
            client_session_id: client_session_id.into(),
            resume: resume.map(str::to_owned),
            model: super::normalize_claude_model(&model),
            system_prompt: initial["systemPrompt"].as_str().unwrap_or_default().into(),
            tools: initial["tools"].clone(),
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&start)
                    .map_err(|_| "Invalid Claude start")?
                    .into(),
            ))
            .await
            .map_err(|_| {
                super::turn_retry::disconnected("Locus Server disconnected during startup")
            })?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(BRIDGE_STARTUP_SECONDS);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
        let hello = loop {
            let message = tokio::select! {
                _=tokio::time::sleep_until(deadline)=>return Err("Claude Server startup exceeded the shared startup deadline".into()),
                _=heartbeat.tick()=>{socket.send(Message::Ping(Vec::new().into())).await.map_err(|_|super::turn_retry::disconnected("Locus Server startup heartbeat failed"))?;continue;},
                value=receive_json(&mut socket)=>value?,
            };
            match message["type"].as_str() {
                Some("progress") => append_diagnostic(
                    &diagnostic,
                    &format!(
                        "Claude startup: {}",
                        message["stage"].as_str().unwrap_or("starting")
                    ),
                ),
                Some("error") => return Err(parse_error(&message).client_message()),
                Some("hello") => break message,
                _ => return Err("Invalid Claude Server startup protocol".into()),
            }
        };
        if hello["protocol"] != PROTOCOL_VERSION {
            return Err("Claude Server protocol version changed during startup".into());
        }
        let caps: Capabilities = serde_json::from_value(hello["capabilities"].clone())
            .map_err(|_| "Invalid Claude Server capabilities")?;
        Capabilities::current().validate_peer(&caps)?;
        let info = serde_json::from_value(hello["info"].clone())
            .map_err(|_| "Invalid server readiness response")?;
        let (send, mut receive) = mpsc::channel::<Value>(64);
        let replies = send.clone();
        let healthy = Arc::new(AtomicBool::new(true));
        let running = healthy.clone();
        let failure = Arc::new(Mutex::new(None));
        let task_failure = failure.clone();
        let latency = Arc::new(Mutex::new(None));
        let task_latency = latency.clone();
        let task_diagnostic = diagnostic.clone();
        let task = tokio::spawn(async move {
            let mut pending = tokio::task::JoinSet::new();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
            let mut last_received = tokio::time::Instant::now();
            let mut last_ping = tokio::time::Instant::now();
            let work = async {
                loop {
                    while pending.try_join_next().is_some() {}
                    tokio::select! {
                        _=heartbeat.tick()=>{
                            if last_received.elapsed()>Duration::from_secs(30) {return Err(());}
                            last_ping=tokio::time::Instant::now();
                            socket.send(Message::Ping(Vec::new().into())).await.map_err(|_|())?;
                        }
                        value=receive.recv()=>{
                            let Some(value)=value else{return Ok(());};
                            socket.send(Message::Text(value.to_string().into())).await.map_err(|_|())?;
                        }
                        frame=socket.next()=>{
                            last_received=tokio::time::Instant::now();
                            let Some(Ok(frame))=frame else{return Err(());};
                            match frame {
                                Message::Text(text)=>{
                                    if text.len()>32*1024*1024 {return Err(());}
                                    let value:Value=serde_json::from_str(&text).map_err(|_|())?;
                                    match value["type"].as_str() {
                                        Some("error")=>{
                                            let error=parse_error(&value);append_diagnostic(&task_diagnostic,&error.message);
                                            *task_failure.lock().unwrap_or_else(|e|e.into_inner())=Some(error);return Err(());
                                        }
                                        Some("event")=>{controls.route(&value["payload"],&options);}
                                        Some("call")=>{
                                            if pending.len()>=64 {return Err(());}
                                            let id=value["id"].as_str().ok_or(())?.to_owned();
                                            let payload=value["payload"].clone();
                                            let reply_send=replies.clone();
                                            let calls=calls.clone();let controls=controls.clone();let options=options.clone();
                                            pending.spawn(async move {
                                                let result=if let Some(receipt)=controls.route(&payload,&options) {receipt} else {
                                                    let (reply,received)=oneshot::channel();
                                                    if calls.send(PendingCall{payload,reply,received_at:tokio::time::Instant::now()}).await.is_err(){return;}
                                                    received.await.unwrap_or_else(|_|json!({"deny":"Locus tool handler closed"}))
                                                };
                                                let _=reply_send.send(json!({"type":"reply","id":id,"value":result})).await;
                                            });
                                        }
                                        _=>return Err(()),
                                    }
                                }
                                Message::Ping(bytes)=>{socket.send(Message::Pong(bytes)).await.map_err(|_|())?;}
                                Message::Pong(_)=>{*task_latency.lock().unwrap_or_else(|e|e.into_inner())=Some(last_ping.elapsed().as_secs_f64()*1000.0);},
                                _=>return Err(()),
                            }
                        }
                    }
                }
            };
            let _ = work.await;
            running.store(false, Ordering::SeqCst);
            let _ = tokio::time::timeout(Duration::from_secs(2), socket.close(None)).await;
        });
        Ok((
            Self {
                send,
                healthy,
                task,
                diagnostic,
                failure,
                latency,
                binding_id: connection.binding_id,
                client_session_id: client_session_id.into(),
            },
            info,
        ))
    }
    pub fn alive(&self) -> bool {
        self.healthy.load(Ordering::SeqCst) && !self.task.is_finished()
    }
    pub fn failure(&self) -> String {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(BridgeError::client_message)
            .unwrap_or_else(|| super::turn_retry::disconnected("Locus Server connection closed"))
    }
    pub fn stop(&self) {
        self.healthy.store(false, Ordering::SeqCst);
        let _ = self.send.try_send(json!({"type":"close"}));
        self.task.abort();
    }
    pub fn latency_ms(&self) -> Option<f64> {
        *self.latency.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn health_failure(&self) -> Option<String> {
        (!self.alive()).then(|| self.failure())
    }
    fn enqueue(&self, value: Value) -> Result<(), String> {
        if !self.alive() {
            return Err(self.failure());
        }
        self.send
            .try_send(value)
            .map_err(|_| "Locus Server input queue is full or closed".into())
    }
    pub fn input(&self, text: &str) -> Result<(), String> {
        self.enqueue(input_operation(text)?)
    }
    pub fn send(&self, value: Value) -> Result<(), String> {
        self.enqueue(client_operation(value)?)
    }
    pub async fn archive_request(&self, value: Value) -> Result<Value, String> {
        operation(&self.binding_id, &self.client_session_id, "archive", value).await
    }
}
fn append_diagnostic(buffer: &Arc<Mutex<Vec<u8>>>, message: &str) {
    let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
    buffer.extend_from_slice(message.as_bytes());
    buffer.push(b'\n');
    if buffer.len() > 8192 {
        let excess = buffer.len() - 8192;
        buffer.drain(..excess);
    }
}
fn parse_error(value: &Value) -> BridgeError {
    serde_json::from_value(value["error"].clone()).unwrap_or_else(|_| {
        BridgeError::classify(
            "Claude Server protocol version is incompatible; update both applications",
        )
    })
}
async fn receive_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Result<Value, String> {
    while let Some(message) = socket.next().await {
        match message
            .map_err(|_| super::turn_retry::disconnected("Claude Server startup connection lost"))?
        {
            Message::Text(text) if text.len() <= 32 * 1024 * 1024 => {
                return serde_json::from_str(&text).map_err(|_| "Invalid Claude Server JSON".into())
            }
            Message::Ping(value) => socket.send(Message::Pong(value)).await.map_err(|_| {
                super::turn_retry::disconnected("Claude Server startup connection lost")
            })?,
            Message::Pong(_) => {}
            _ => {
                return Err(super::turn_retry::disconnected(
                    "Claude Server startup connection closed",
                ))
            }
        }
    }
    Err(super::turn_retry::disconnected(
        "Claude Server startup connection closed",
    ))
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.stop();
    }
}
fn input_operation(text: &str) -> Result<Value, String> {
    if let Some(path) = text
        .strip_prefix("\x1b[200~\"")
        .and_then(|text| text.strip_suffix(&format!("\"{}\x1b[201~", " ".repeat(1024))))
    {
        let (input_id, file) = path
            .strip_prefix("locus-server/attachments/")
            .and_then(|path| path.split_once('/'))
            .ok_or("Invalid server image path")?;
        let index = file
            .strip_prefix("image-")
            .and_then(|name| name.split_once('.'))
            .and_then(|(index, _)| index.parse::<usize>().ok())
            .and_then(|index| index.checked_sub(1))
            .ok_or("Invalid image slot")?;
        // File slot and image ordinal can differ when text files precede images.
        return Ok(json!({"type":"pasteImage","inputId":input_id,"index":index}));
    }
    let kind = match text {
        "\r" => "submit",
        "\x1b" => "cancel",
        "\x18\x13" => "resumeQueued",
        _ => return Err("Raw terminal input is not exposed by Locus Server".into()),
    };
    Ok(json!({"type":kind}))
}
fn client_operation(value: Value) -> Result<Value, String> {
    if value["type"] != "configure" {
        return Err("Unsupported Claude Server operation".into());
    }
    let mut options = value["options"].clone();
    if !options["control"].is_null() && options["control"] != "compact" {
        return Err("Unsupported Claude command".into());
    }
    let object = options
        .as_object_mut()
        .ok_or("Invalid Claude configuration")?;
    for field in [
        "transport",
        "terminalText",
        "imageCount",
        "expectedImageCount",
        "attachmentMappings",
    ] {
        object.remove(field);
    }
    if let Some(insertion) = options["insertion"].as_object_mut() {
        for field in ["terminalText", "imageCount", "expectedImageCount"] {
            insertion.remove(field);
        }
    }
    if let Some(model) = options["model"].as_str() {
        if let Some((_, model)) = crate::server_client::binding_for_model(model)? {
            options["model"] = json!(super::normalize_claude_model(&model));
        }
    }
    Ok(
        json!({"type":"configure","options":options,"uploads":value["uploads"].as_array().cloned().unwrap_or_default(),"uploadId":value["uploadId"]}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_connection(port: u16) -> crate::server_client::Connection {
        crate::server_client::Connection {
            server_url: format!("http://127.0.0.1:{port}"),
            access_token: "fixture-token".into(),
            upstream_id: uuid::Uuid::new_v4().to_string(),
            user_id: "fixture-user".into(),
            binding_id: uuid::Uuid::new_v4().to_string(),
        }
    }
    fn fixture_launch() -> LaunchOptions {
        LaunchOptions {
            permission_mode: super::super::ssh::PermissionMode::BypassPermissions,
            runtime_settings: super::super::runtime_settings::RuntimeSettings {
                idle_compaction: true,
                telemetry: true,
                auto_compact_window: Some(350_000),
            },
            native_tool_search: false,
        }
    }
    #[tokio::test]
    async fn real_adapter_keeps_a_slow_start_alive_and_preserves_fatal_diagnostics() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"capabilities","capabilities":Capabilities::current()})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let request: Value = serde_json::from_str(
                socket
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .into_text()
                    .unwrap()
                    .as_str(),
            )
            .unwrap();
            assert_eq!(request["prewarm"], true);
            assert_eq!(
                request["launch"],
                serde_json::to_value(fixture_launch()).unwrap()
            );
            socket
                .send(Message::Text(
                    json!({"type":"progress","stage":"environment"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(61);
            let mut pings = 0;
            loop {
                tokio::select! {
                    _=tokio::time::sleep_until(deadline)=>break,
                    frame=socket.next()=>match frame.unwrap().unwrap(){Message::Ping(value)=>{pings+=1;socket.send(Message::Pong(value)).await.unwrap();},Message::Pong(_)=>{},_=>panic!("unexpected startup operation")}
                }
            }
            assert!(pings >= 10, "startup did not maintain heartbeats");
            socket.send(Message::Text(json!({"type":"hello","protocol":PROTOCOL_VERSION,"capabilities":Capabilities::current(),"info":RemoteInfo::default()}).to_string().into())).await.unwrap();
            socket.send(Message::Text(json!({"type":"error","error":BridgeError::classify("SSH connection: Permission denied (publickey)")}).to_string().into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        });
        let (calls, _) = mpsc::channel(8);
        let (transport, _) = Transport::start_bound(
            fixture_connection(port),
            "claude_code/fixture",
            &uuid::Uuid::new_v4().to_string(),
            None,
            json!({"systemPrompt":"","tools":[]}),
            fixture_launch(),
            true,
            calls,
            Arc::new(ControlSignals::default()),
            Arc::new(std::sync::RwLock::new(json!({}))),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while transport.alive() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(transport.failure().contains("publickey"));
        assert!(!super::super::turn_retry::is_disconnected(
            &transport.failure()
        ));
        assert!(
            String::from_utf8_lossy(&transport.diagnostic.lock().unwrap()).contains("environment")
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn real_adapter_rejects_incompatible_server_before_sending_start() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut caps = Capabilities::current();
            caps.protocol = 1;
            socket
                .send(Message::Text(
                    json!({"type":"capabilities","capabilities":caps})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            assert!(!matches!(socket.next().await, Some(Ok(Message::Text(_)))));
        });
        let (calls, _) = mpsc::channel(8);
        let result = Transport::start_bound(
            fixture_connection(port),
            "claude_code/fixture",
            &uuid::Uuid::new_v4().to_string(),
            None,
            json!({"systemPrompt":"","tools":[]}),
            fixture_launch(),
            true,
            calls,
            Arc::new(ControlSignals::default()),
            Arc::new(std::sync::RwLock::new(json!({}))),
        )
        .await;
        assert!(result.err().unwrap().contains("capabilities differ"));
        server.await.unwrap();
    }
    #[test]
    fn terminal_input_is_reduced_to_typed_operations() {
        assert_eq!(input_operation("\r").unwrap(), json!({"type":"submit"}));
        assert_eq!(input_operation("\x1b").unwrap(), json!({"type":"cancel"}));
        let path =
            "locus-server/attachments/LOCUS_INPUT_0123456789abcdef0123456789abcdef/image-1.png";
        let paste = super::super::attachments::image_paste(path).unwrap();
        assert_eq!(
            input_operation(&paste).unwrap(),
            json!({"type":"pasteImage","inputId":"LOCUS_INPUT_0123456789abcdef0123456789abcdef","index":0})
        );
        for raw in ["/login\r", "\n/bin/sh\r", "\x1b[200~evil\x1b[201~"] {
            assert!(input_operation(raw).is_err());
        }
    }
    #[test]
    fn adapter_drops_terminal_fields_and_rejects_raw_commands() {
        let value=client_operation(json!({"type":"configure","options":{"transport":"tui","active":true,"inputId":"id","terminalText":"evil","control":null,"imageCount":0,"expectedImageCount":0,"attachmentMappings":[]}})).unwrap();
        assert!(value["options"].get("terminalText").is_none());
        assert_eq!(
            client_operation(json!({"type":"configure","options":{},"uploads":[{}]})).unwrap()
                ["uploads"],
            json!([{}])
        );
        assert!(
            client_operation(json!({"type":"configure","options":{"control":"compact"}})).is_ok()
        );
        assert!(client_operation(json!({"type":"input","data":"evil"})).is_err());
    }
}

pub(crate) async fn operation(
    binding: &str,
    client_session: &str,
    operation: &str,
    body: Value,
) -> Result<Value, String> {
    if ![
        "history",
        "archive",
        "fork",
        "rewind",
        "reset",
        "restore-begin",
        "restore-chunk",
        "restore-commit",
    ]
    .contains(&operation)
    {
        return Err("Unknown Claude Server operation".into());
    }
    uuid::Uuid::parse_str(client_session).map_err(|_| "Invalid client session identity")?;
    let connection = crate::server_client::credential_for_binding(binding).await?;
    let endpoint = format!(
        "{}/api/claude/{}/sessions/{}/{}",
        connection.server_url, connection.upstream_id, client_session, operation
    );
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(180))
            .follow_redirects(false),
    )?;
    let mut response = client
        .post(endpoint)
        .bearer_auth(connection.access_token)
        .json(&body)
        .send()
        .await
        .map_err(|_| super::turn_retry::disconnected("Claude Server operation failed"))?;
    let success = response.status().is_success();
    let limit = if success {
        400 * 1024 * 1024
    } else {
        64 * 1024
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| super::turn_retry::disconnected("Claude Server response interrupted"))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("Claude Server response exceeds the size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() && success {
        return Ok(json!({"ok":true}));
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid Claude Server response")?;
    if !success {
        return Err(value["error"]["message"]
            .as_str()
            .unwrap_or("Claude Server operation was rejected")
            .into());
    }
    Ok(value)
}
