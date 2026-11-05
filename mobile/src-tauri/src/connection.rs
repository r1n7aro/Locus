//! The live connection to the active desktop: one WebSocket carrying RPC
//! requests and workspace events, with automatic reconnects. Payloads are
//! relayed to the page as raw JSON (no parse/re-encode of session data).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Notify};
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::WebSocketStream;
use zeroize::Zeroizing;

use locus_remote_protocol::{self as protocol, error_codes, methods, CandidateAddress, ClientFrame, RpcError};

use crate::client::HttpClient;
use crate::tls;

const TCP_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const PING_INTERVAL: Duration = Duration::from_secs(25);
const SILENCE_LIMIT: Duration = Duration::from_secs(75);
const CALL_TIMEOUT: Duration = Duration::from_secs(130);
const WAIT_FOR_LINK: Duration = Duration::from_secs(15);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

type Socket = WebSocketStream<TlsStream<TcpStream>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    Connecting,
    Connected,
    Reconnecting,
    Revoked,
    Closed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub state: LinkState,
    pub desktop_name: String,
    pub device_name: String,
    pub error: Option<String>,
}

pub type RpcOutcome = Result<String, RpcError>;

fn failure(code: &str, message: impl Into<String>) -> RpcError {
    RpcError { code: code.into(), message: message.into(), data: None }
}

/// Server frame, parsed without materializing results or event payloads.
#[derive(Deserialize)]
struct Incoming<'a> {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    ok: Option<bool>,
    #[serde(default, borrow)]
    result: Option<&'a RawValue>,
    #[serde(default)]
    error: Option<RpcError>,
    #[serde(default, borrow)]
    payload: Option<&'a RawValue>,
    #[serde(default)]
    reason: Option<String>,
}

enum Ended {
    Stopped,
    Lost,
    Closing(String),
}

enum ConnectError {
    Unauthorized,
    PinMismatch,
    Failed(String),
}

/// Pairing ended on the desktop side; retrying cannot succeed.
fn is_terminal(reason: &str) -> bool {
    matches!(reason, "revoked" | "unpaired" | "certificate_reset")
}

struct ActiveLink {
    address: CandidateAddress,
    http: HttpClient,
}

pub struct Connection {
    pub desktop_id: String,
    desktop_name: String,
    device_name: String,
    addresses: Vec<CandidateAddress>,
    fingerprint: String,
    token: Zeroizing<String>,
    status: watch::Sender<ConnectionStatus>,
    outgoing: Mutex<Option<mpsc::Sender<String>>>,
    active: Mutex<Option<Arc<ActiveLink>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<RpcOutcome>>>,
    next_id: AtomicU64,
    /// Delivers JSON messages to the page; returns false once the page is gone.
    subscriber: Mutex<Option<Box<dyn Fn(String) -> bool + Send + Sync>>>,
    /// Last `set_interests` params, replayed after every reconnect.
    interests: Mutex<Option<Value>>,
    /// Last `set_file_watches` params, replayed the same way.
    file_watches: Mutex<Option<Value>>,
    wake: Notify,
    stop: watch::Sender<bool>,
    on_connected: Box<dyn Fn(&str) + Send + Sync>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Connection {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        desktop_id: String,
        desktop_name: String,
        device_name: String,
        addresses: Vec<CandidateAddress>,
        fingerprint: String,
        token: Zeroizing<String>,
        on_connected: impl Fn(&str) + Send + Sync + 'static,
    ) -> Arc<Self> {
        let (status, _) = watch::channel(ConnectionStatus {
            state: LinkState::Connecting,
            desktop_name: desktop_name.clone(),
            device_name: device_name.clone(),
            error: None,
        });
        let (stop, _) = watch::channel(false);
        Arc::new(Self {
            desktop_id,
            desktop_name,
            device_name,
            addresses,
            fingerprint,
            token,
            status,
            outgoing: Mutex::new(None),
            active: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            subscriber: Mutex::new(None),
            interests: Mutex::new(None),
            file_watches: Mutex::new(None),
            wake: Notify::new(),
            stop,
            on_connected: Box::new(on_connected),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let connection = self.clone();
        tauri::async_runtime::spawn(async move { connection.run().await });
    }

    pub fn stop(&self) {
        let _ = self.stop.send(true);
        lock(&self.subscriber).take();
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status.borrow().clone()
    }

    pub fn subscribe(&self, sink: impl Fn(String) -> bool + Send + Sync + 'static) {
        *lock(&self.subscriber) = Some(Box::new(sink));
        self.push_status();
    }

    // ----- page notifications

    fn push(&self, json: String) {
        let mut subscriber = lock(&self.subscriber);
        if let Some(sink) = subscriber.as_ref() {
            if !sink(json) {
                // The page navigated away; it subscribes again on load.
                subscriber.take();
            }
        }
    }

    fn push_status(&self) {
        let status = self.status();
        if let Ok(json) = serde_json::to_string(&serde_json::json!({ "kind": "status", "status": status })) {
            self.push(json);
        }
    }

    fn set_state(&self, state: LinkState, error: Option<String>) {
        self.status.send_modify(|status| {
            status.state = state;
            status.error = error;
        });
        self.push_status();
    }

    // ----- RPC

    /// Connection-level settings the desktop forgets on disconnect: the last
    /// value is kept here and sent again first thing after every reconnect.
    fn replayed(&self, method: &str) -> Option<&Mutex<Option<Value>>> {
        match method {
            methods::SET_INTERESTS => Some(&self.interests),
            methods::SET_FILE_WATCHES => Some(&self.file_watches),
            _ => None,
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> RpcOutcome {
        if !methods::is_valid_name(method) {
            return Err(failure(error_codes::UNKNOWN_METHOD, format!("Unknown method '{method}'")));
        }
        if let Some(replayed) = self.replayed(method) {
            *lock(replayed) = Some(params.clone());
            // Replayed on (re)connect; nothing to wait for while offline.
            if lock(&self.outgoing).is_none() {
                return Ok("null".into());
            }
        }
        let sender = self.wait_for_link().await?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let frame = serde_json::to_string(&ClientFrame::Request { id, method: method.to_string(), params })
            .map_err(|error| failure(error_codes::INVALID_PARAMS, error.to_string()))?;
        let (reply, response) = oneshot::channel();
        lock(&self.pending).insert(id, reply);
        if sender.send(frame).await.is_err() {
            lock(&self.pending).remove(&id);
            return Err(failure("disconnected", "与电脑的连接已断开"));
        }
        match tokio::time::timeout(CALL_TIMEOUT, response).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(failure("disconnected", "与电脑的连接已断开")),
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(failure("timeout", "电脑长时间没有响应"))
            }
        }
    }

    async fn wait_for_link(&self) -> Result<mpsc::Sender<String>, RpcError> {
        let deadline = tokio::time::Instant::now() + WAIT_FOR_LINK;
        let mut status = self.status.subscribe();
        loop {
            if let Some(sender) = lock(&self.outgoing).clone() {
                return Ok(sender);
            }
            let state = status.borrow().state;
            if matches!(state, LinkState::Revoked | LinkState::Closed) {
                return Err(failure("closed", "与电脑的连接已关闭"));
            }
            if tokio::time::timeout_at(deadline, status.changed()).await.is_err() {
                return Err(failure("offline", "暂时连不上电脑"));
            }
        }
    }

    /// HTTP client and token for the address the socket is connected to.
    pub async fn asset_client(&self) -> Result<(HttpClient, Zeroizing<String>), String> {
        let deadline = tokio::time::Instant::now() + WAIT_FOR_LINK;
        let mut status = self.status.subscribe();
        loop {
            if let Some(link) = lock(&self.active).clone() {
                return Ok((link.http.clone(), self.token.clone()));
            }
            if matches!(status.borrow().state, LinkState::Revoked | LinkState::Closed) {
                return Err("与电脑的连接已关闭".into());
            }
            if tokio::time::timeout_at(deadline, status.changed()).await.is_err() {
                return Err("暂时连不上电脑".into());
            }
        }
    }

    fn fail_pending(&self) {
        let pending: Vec<_> = lock(&self.pending).drain().collect();
        for (_, reply) in pending {
            let _ = reply.send(Err(failure("disconnected", "与电脑的连接已断开")));
        }
    }

    // ----- connection loop

    async fn run(self: Arc<Self>) {
        let mut stop = self.stop.subscribe();
        let mut failures: u32 = 0;
        let mut connected_before = false;
        loop {
            if *stop.borrow() {
                break;
            }
            let retry_state = if connected_before { LinkState::Reconnecting } else { LinkState::Connecting };
            if self.status().state != retry_state {
                let error = self.status().error;
                self.set_state(retry_state, error);
            }
            match self.connect_any().await {
                Ok((socket, address)) => {
                    failures = 0;
                    match self.serve(socket, address, connected_before, &mut stop).await {
                        Ended::Stopped => break,
                        Ended::Closing(reason) if is_terminal(&reason) => {
                            self.set_state(LinkState::Revoked, Some(reason.clone()));
                            self.push_closing(&reason);
                            break;
                        }
                        Ended::Closing(reason) => {
                            self.set_state(LinkState::Reconnecting, Some(reason));
                        }
                        Ended::Lost => {}
                    }
                    connected_before = true;
                    // One immediate retry after losing a working link.
                    continue;
                }
                Err(ConnectError::Unauthorized) => {
                    self.set_state(LinkState::Revoked, Some("revoked".into()));
                    self.push_closing("revoked");
                    break;
                }
                Err(ConnectError::PinMismatch) => {
                    self.set_state(retry_state, Some(
                        "证书与配对时不一致：电脑可能已重置证书，或连接被冒充。请重新配对。".into(),
                    ));
                }
                Err(ConnectError::Failed(message)) => self.set_state(retry_state, Some(message)),
            }
            failures = failures.saturating_add(1);
            let delay = Duration::from_secs(1u64 << failures.min(5)).min(MAX_BACKOFF);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = self.wake.notified() => { failures = 0; }
                _ = stop.changed() => {}
            }
        }
        lock(&self.outgoing).take();
        lock(&self.active).take();
        self.fail_pending();
        if !matches!(self.status().state, LinkState::Revoked) {
            self.set_state(LinkState::Closed, None);
        }
    }

    fn push_closing(&self, reason: &str) {
        if let Ok(json) = serde_json::to_string(&serde_json::json!({ "kind": "closing", "reason": reason })) {
            self.push(json);
        }
    }

    async fn connect_any(&self) -> Result<(Socket, CandidateAddress), ConnectError> {
        let mut pin_mismatch = false;
        let mut last_error = String::from("没有可用的地址");
        for address in &self.addresses {
            match self.connect_one(address).await {
                Ok(socket) => return Ok((socket, address.clone())),
                Err(ConnectError::Unauthorized) => return Err(ConnectError::Unauthorized),
                Err(ConnectError::PinMismatch) => pin_mismatch = true,
                Err(ConnectError::Failed(error)) => last_error = error,
            }
        }
        Err(if pin_mismatch { ConnectError::PinMismatch } else { ConnectError::Failed(last_error) })
    }

    async fn connect_one(&self, address: &CandidateAddress) -> Result<Socket, ConnectError> {
        let failed = |error: String| ConnectError::Failed(error);
        let url = url::Url::parse(&address.url).map_err(|error| failed(error.to_string()))?;
        let host = url.host_str().ok_or_else(|| failed("地址缺少主机".into()))?;
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
        let port = url.port_or_known_default().unwrap_or(443);
        let pinned = address.pinned.then_some(self.fingerprint.as_str());
        let config = tls::client_config(pinned).map_err(failed)?;

        let tcp = tokio::time::timeout(TCP_TIMEOUT, TcpStream::connect((host.as_str(), port)))
            .await
            .map_err(|_| failed(format!("{} 连接超时", address.url)))?
            .map_err(|error| failed(format!("{}：{error}", address.url)))?;
        let _ = tcp.set_nodelay(true);
        let server_name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|error| failed(error.to_string()))?;
        let tls_stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, TlsConnector::from(config).connect(server_name, tcp))
            .await
            .map_err(|_| failed(format!("{} TLS 握手超时", address.url)))?
            .map_err(|error| {
                if tls::is_pin_mismatch(&error) {
                    ConnectError::PinMismatch
                } else {
                    failed(format!("{}：{error}", address.url))
                }
            })?;

        let ws_url = protocol::websocket_url(&address.url).ok_or_else(|| failed("地址无效".into()))?;
        let mut request = ws_url.into_client_request().map_err(|error| failed(error.to_string()))?;
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", self.token.as_str()))
            .map_err(|_| failed("令牌无效".into()))?;
        authorization.set_sensitive(true);
        request.headers_mut().insert("Authorization", authorization);
        let mut config = WebSocketConfig::default();
        config.max_message_size = Some(protocol::MAX_FRAME_BYTES);
        config.max_frame_size = Some(protocol::MAX_FRAME_BYTES);
        let handshake = tokio_tungstenite::client_async_with_config(request, tls_stream, Some(config));
        match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
            Err(_) => Err(failed(format!("{} 握手超时", address.url))),
            Ok(Ok((socket, _))) => Ok(socket),
            Ok(Err(tungstenite::Error::Http(response))) if response.status() == 401 => {
                Err(ConnectError::Unauthorized)
            }
            Ok(Err(error)) => Err(failed(format!("{}：{error}", address.url))),
        }
    }

    async fn serve(
        &self,
        socket: Socket,
        address: CandidateAddress,
        reconnected: bool,
        stop: &mut watch::Receiver<bool>,
    ) -> Ended {
        let http = match HttpClient::new(&address.url, address.pinned.then_some(self.fingerprint.as_str())) {
            Ok(http) => http,
            Err(error) => {
                self.set_state(LinkState::Reconnecting, Some(error));
                return Ended::Lost;
            }
        };
        let (mut sink, mut stream) = socket.split();
        let (sender, mut queue) = mpsc::channel::<String>(256);

        // Interests and file watches first, so no event of the restored
        // subscription is missed.
        for method in [methods::SET_INTERESTS, methods::SET_FILE_WATCHES] {
            let Some(params) = self.replayed(method).and_then(|slot| lock(slot).clone()) else {
                continue;
            };
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let frame = ClientFrame::Request { id, method: method.into(), params };
            if let Ok(text) = serde_json::to_string(&frame) {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    return Ended::Lost;
                }
            }
        }
        *lock(&self.outgoing) = Some(sender);
        *lock(&self.active) = Some(Arc::new(ActiveLink { address: address.clone(), http }));
        (self.on_connected)(&address.url);
        self.set_state(LinkState::Connected, None);
        if reconnected {
            // Events may have been missed while offline.
            self.push(r#"{"kind":"resync","reason":"reconnected"}"#.to_string());
        }

        let mut ping = tokio::time::interval(PING_INTERVAL);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_heard = Instant::now();
        let mut nonce = 0u64;
        let ended = loop {
            tokio::select! {
                _ = stop.changed() => {
                    let _ = sink.send(Message::Close(None)).await;
                    break Ended::Stopped;
                }
                Some(text) = queue.recv() => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break Ended::Lost;
                    }
                }
                _ = ping.tick() => {
                    if last_heard.elapsed() > SILENCE_LIMIT {
                        break Ended::Lost;
                    }
                    nonce += 1;
                    if let Ok(text) = serde_json::to_string(&ClientFrame::Ping { nonce }) {
                        if sink.send(Message::Text(text.into())).await.is_err() {
                            break Ended::Lost;
                        }
                    }
                }
                message = stream.next() => match message {
                    Some(Ok(Message::Text(text))) => {
                        last_heard = Instant::now();
                        if let Some(ended) = self.handle_frame(text.as_str()) {
                            break ended;
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => last_heard = Instant::now(),
                    Some(Ok(Message::Binary(_) | Message::Frame(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break Ended::Lost,
                }
            }
        };
        lock(&self.outgoing).take();
        lock(&self.active).take();
        self.fail_pending();
        ended
    }

    fn handle_frame(&self, text: &str) -> Option<Ended> {
        let Ok(frame) = serde_json::from_str::<Incoming<'_>>(text) else {
            return None;
        };
        match frame.kind.as_str() {
            "response" => {
                let id = frame.id?;
                let reply = lock(&self.pending).remove(&id)?;
                let outcome = if frame.ok == Some(true) {
                    Ok(frame.result.map(|raw| raw.get().to_string()).unwrap_or_else(|| "null".into()))
                } else {
                    Err(frame.error.unwrap_or_else(|| failure(error_codes::FAILED, "请求失败")))
                };
                let _ = reply.send(outcome);
            }
            "event" => {
                if let Some(payload) = frame.payload {
                    self.push(format!(r#"{{"kind":"event","event":{}}}"#, payload.get()));
                }
            }
            "resync" => {
                let reason = frame.reason.unwrap_or_else(|| "overflow".into());
                if let Ok(json) = serde_json::to_string(&serde_json::json!({ "kind": "resync", "reason": reason })) {
                    self.push(json);
                }
            }
            "closing" => return Some(Ended::Closing(frame.reason.unwrap_or_default())),
            _ => {}
        }
        None
    }

    pub fn desktop_name(&self) -> &str {
        &self.desktop_name
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn active_address(&self) -> Option<String> {
        lock(&self.active).as_ref().map(|link| link.address.url.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_frames_keep_payloads_raw() {
        let text = r#"{"type":"response","id":3,"ok":true,"result":{"big":[1,2,3]}}"#;
        let frame: Incoming<'_> = serde_json::from_str(text).unwrap();
        assert_eq!(frame.kind, "response");
        assert_eq!(frame.result.unwrap().get(), r#"{"big":[1,2,3]}"#);

        let text = r#"{"type":"event","payload":{"eventName":"stream-event","payload":{"type":"textDelta"}}}"#;
        let frame: Incoming<'_> = serde_json::from_str(text).unwrap();
        assert_eq!(frame.payload.unwrap().get(), r#"{"eventName":"stream-event","payload":{"type":"textDelta"}}"#);

        let text = r#"{"type":"response","id":4,"ok":false,"error":{"code":"forbidden","message":"no"}}"#;
        let frame: Incoming<'_> = serde_json::from_str(text).unwrap();
        assert_eq!(frame.error.unwrap().code, "forbidden");
    }

    #[test]
    fn terminal_reasons() {
        assert!(is_terminal("revoked"));
        assert!(is_terminal("certificate_reset"));
        assert!(!is_terminal("disabled"));
        assert!(!is_terminal("listener_closed"));
    }
}
