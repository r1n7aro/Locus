//! HTTP(S) listeners, request admission and routing.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{
    HeaderValue, AUTHORIZATION, CACHE_CONTROL, CONNECTION, CONTENT_TYPE, HOST, ORIGIN, UPGRADE,
};
use hyper::{Method, Request, Response, StatusCode, Version};
use hyper_util::rt::{TokioIo, TokioTimer};
use rand::RngExt;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use zeroize::Zeroize;

use locus_remote_protocol as protocol;
use protocol::{ErrorBody, Hello, PairRequest, PairResponse};

use super::app_package::{self, PageState};
use super::blobs;
use super::devices::{sanitize_device_name, sanitize_platform, DeviceRecord};
use super::net::{self, ListenerKind};
use super::pairing::{PairCheck, CONFIRM_TIMEOUT};
use super::{now_ms, Shared};

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// First byte of every TLS connection (ContentType::Handshake).
const TLS_HANDSHAKE_RECORD: u8 = 0x16;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP_BUFFER: usize = 64 * 1024;
const MAX_PAIR_BODY: usize = 4 * 1024;
const MAX_CONNECTIONS_PER_PEER: u32 = 16;
const FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const FAILURES_BEFORE_BAN: u32 = 20;
const BAN_DURATION: Duration = Duration::from_secs(5 * 60);

/// Content Security Policy of the mobile page. The page runs inside the
/// Locus Remote app with access to its IPC (`ipc:` / `http://ipc.localhost`
/// on Android) and to binary previews the app proxies from this desktop
/// (`locus-binary`, see `blobs`), and nothing else.
const MOBILE_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob: http://locus-binary.localhost locus-binary:; \
media-src 'self' data: blob: http://locus-binary.localhost locus-binary:; \
font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost; \
object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

type Body = Full<Bytes>;

// ----- per-peer admission ---------------------------------------------------

#[derive(Default)]
struct PeerState {
    connections: u32,
    failures: u32,
    window_start: Option<Instant>,
    banned_until: Option<Instant>,
}

impl PeerState {
    fn idle(&self, now: Instant) -> bool {
        self.connections == 0
            && self.banned_until.is_none_or(|until| until <= now)
            && self.window_start.is_none_or(|start| now.duration_since(start) > FAILURE_WINDOW)
    }
}

/// Caps concurrent connections per peer and bans peers that keep failing
/// authentication or pairing.
#[derive(Default)]
pub struct AbuseGuard {
    peers: Mutex<HashMap<IpAddr, PeerState>>,
}

pub struct PeerSlot {
    shared: Arc<Shared>,
    ip: IpAddr,
}

impl Drop for PeerSlot {
    fn drop(&mut self) {
        let mut peers = self.shared.guard.lock();
        let now = Instant::now();
        if let Some(state) = peers.get_mut(&self.ip) {
            state.connections = state.connections.saturating_sub(1);
            if state.idle(now) {
                peers.remove(&self.ip);
            }
        }
    }
}

impl AbuseGuard {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, PeerState>> {
        self.peers.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn admit(&self, shared: &Arc<Shared>, ip: IpAddr) -> Option<PeerSlot> {
        let now = Instant::now();
        let mut peers = self.lock();
        if peers.len() > 4096 {
            peers.retain(|_, state| !state.idle(now));
        }
        let state = peers.entry(ip).or_default();
        if state.banned_until.is_some_and(|until| until > now)
            || state.connections >= MAX_CONNECTIONS_PER_PEER
        {
            return None;
        }
        state.connections += 1;
        Some(PeerSlot { shared: shared.clone(), ip })
    }

    pub fn is_banned(&self, ip: IpAddr) -> bool {
        self.lock()
            .get(&ip)
            .and_then(|state| state.banned_until)
            .is_some_and(|until| until > Instant::now())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut peers = self.lock();
        let state = peers.entry(ip).or_default();
        if state.window_start.is_none_or(|start| now.duration_since(start) > FAILURE_WINDOW) {
            state.window_start = Some(now);
            state.failures = 0;
        }
        state.failures += 1;
        if state.failures >= FAILURES_BEFORE_BAN {
            state.banned_until = Some(now + BAN_DURATION);
            state.failures = 0;
            state.window_start = None;
            eprintln!("[RemoteAccess] temporarily blocking {ip} after repeated failures");
        }
    }
}

// ----- listeners -------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct ConnContext {
    pub local: SocketAddr,
    pub peer: SocketAddr,
    pub kind: ListenerKind,
}

/// Accept loop of one bound socket. Runs until aborted.
pub async fn run_listener(
    shared: Arc<Shared>,
    listener: TcpListener,
    kind: ListenerKind,
    tls: Option<TlsAcceptor>,
) {
    let Ok(local) = listener.local_addr() else {
        return;
    };
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // EMFILE and friends: back off instead of spinning.
                eprintln!("[RemoteAccess] accept on {local} failed: {error}");
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        if !net::peer_allowed(kind, peer.ip()) {
            continue;
        }
        let Ok(permit) = shared.connection_slots.clone().try_acquire_owned() else {
            continue;
        };
        let slot = if kind.per_peer_limits() {
            let Some(slot) = shared.guard.admit(&shared, peer.ip()) else {
                continue;
            };
            Some(slot)
        } else {
            None
        };
        let _ = stream.set_nodelay(true);
        let shared = shared.clone();
        let tls = tls.clone();
        let context = ConnContext { local, peer, kind };
        tokio::spawn(async move {
            let _permit = permit;
            let _slot = slot;
            match tls {
                Some(acceptor) => {
                    // A TLS record starts with 0x16 (handshake). Anything else
                    // is a browser that opened the pairing QR code over plain
                    // HTTP: it gets the landing page (and the bundled APK
                    // while offered), never the API.
                    let mut first = [0u8; 1];
                    let Ok(Ok(1)) = tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await
                    else {
                        return;
                    };
                    if first[0] != TLS_HANDSHAKE_RECORD {
                        serve(shared, TokioIo::new(stream), context, Mode::DownloadOnly).await;
                        return;
                    }
                    let Ok(Ok(stream)) =
                        tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
                    else {
                        return;
                    };
                    serve(shared, TokioIo::new(stream), context, Mode::Full).await;
                }
                None => serve(shared, TokioIo::new(stream), context, Mode::Full).await,
            }
        });
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// TLS on NIC listeners, plain HTTP on loopback: the whole API.
    Full,
    /// Plain HTTP reaching a NIC listener: the QR landing page and the
    /// bundled APK only. Nothing authenticated is ever served here, so tokens
    /// never travel in the clear.
    DownloadOnly,
}

async fn serve<I>(shared: Arc<Shared>, io: I, context: ConnContext, mode: Mode)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request| {
        let shared = shared.clone();
        async move {
            let response = match mode {
                Mode::Full => handle(shared, request, context).await,
                Mode::DownloadOnly => handle_download_only(&shared, &request).await,
            };
            Ok::<_, Infallible>(response)
        }
    });
    let _ = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_READ_TIMEOUT)
        .max_buf_size(MAX_HTTP_BUFFER)
        .serve_connection(io, service)
        .with_upgrades()
        .await;
}

// ----- responses --------------------------------------------------------------

fn base_response(status: StatusCode) -> hyper::http::response::Builder {
    Response::builder()
        .status(status)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
}

fn json_response(status: StatusCode, value: &impl serde::Serialize) -> Response<Body> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    base_response(status)
        .header(CONTENT_TYPE, "application/json")
        .header(CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from(body)))
        .expect("static response builds")
}

fn error_response(status: StatusCode, code: &str, message: impl Into<String>) -> Response<Body> {
    json_response(status, &ErrorBody { code: code.into(), message: message.into() })
}

// ----- routing ----------------------------------------------------------------

async fn handle(
    shared: Arc<Shared>,
    request: Request<Incoming>,
    context: ConnContext,
) -> Response<Body> {
    if context.kind.per_peer_limits() && shared.guard.is_banned(context.peer.ip()) {
        return error_response(StatusCode::TOO_MANY_REQUESTS, "blocked", "Too many failed attempts");
    }
    // Browsers are not supported clients; any Origin means a web page is
    // trying to reach the service (CSRF, DNS rebinding).
    if request.headers().contains_key(ORIGIN) {
        return error_response(StatusCode::FORBIDDEN, "origin_forbidden", "Browser requests are not accepted");
    }
    let host = request.headers().get(HOST).and_then(|value| value.to_str().ok());
    let hostnames = shared.settings_snapshot().hostnames;
    if !net::host_allowed(host, &hostnames) {
        return error_response(StatusCode::MISDIRECTED_REQUEST, "bad_host", "Unknown host");
    }
    let path = request.uri().path().to_string();
    if let Some(response) = download_route(&shared, &request).await {
        return response;
    }
    match (request.method(), path.as_str()) {
        (&Method::GET, protocol::HELLO_PATH) => hello(&shared),
        (&Method::POST, protocol::PAIR_PATH) => pair(&shared, request, context).await,
        (&Method::POST, protocol::UNPAIR_PATH) => match authenticate(&shared, &request, context) {
            Ok(device) => {
                let _ = shared.devices.revoke(&device.id, now_ms());
                shared.connections.close_where("unpaired", |c| c.device_id == device.id);
                shared.notify_changed();
                json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
            }
            Err(response) => response,
        },
        (&Method::GET, protocol::WS_PATH) => match authenticate(&shared, &request, context) {
            Ok(device) => upgrade(shared, request, device, context),
            Err(response) => response,
        },
        (&Method::GET, _) if is_asset_path(&path) => match authenticate(&shared, &request, context) {
            Ok(_) => assets::serve(&shared, &path, request.uri().query()).await,
            Err(response) => response,
        },
        (&Method::GET, _) if path.starts_with(protocol::BLOB_PREFIX) => match authenticate(&shared, &request, context) {
            Ok(_) => serve_blob(&shared, &path[protocol::BLOB_PREFIX.len()..]),
            Err(response) => response,
        },
        _ => error_response(StatusCode::NOT_FOUND, "not_found", "Not found"),
    }
}

/// One binary preview (see `blobs`). Ids are random UUIDs handed out in
/// command results; the device token is still required.
fn serve_blob(shared: &Shared, id: &str) -> Response<Body> {
    if !blobs::is_blob_id(id) {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Not found");
    }
    let cache = tauri::Manager::try_state::<Arc<crate::binary_cache::BinaryCache>>(shared.app.as_ref());
    let Some((bytes, mime)) = cache.and_then(|cache| cache.get(id)) else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "Preview expired");
    };
    base_response(StatusCode::OK)
        .header(CONTENT_TYPE, blobs::served_mime(&mime))
        .header(CACHE_CONTROL, "private, max-age=600")
        .header("Content-Security-Policy", "sandbox; default-src 'none'")
        .body(Full::new(Bytes::from(bytes)))
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "failed", "Invalid preview"))
}

// ----- app download ---------------------------------------------------------------

async fn handle_download_only(shared: &Shared, request: &Request<Incoming>) -> Response<Body> {
    let host = request.headers().get(HOST).and_then(|value| value.to_str().ok());
    if !net::host_allowed(host, &shared.settings_snapshot().hostnames) {
        return error_response(StatusCode::MISDIRECTED_REQUEST, "bad_host", "Unknown host");
    }
    if let Some(response) = download_route(shared, request).await {
        return response;
    }
    if request.method() == Method::GET && request.uri().path() == "/" {
        return base_response(StatusCode::FOUND)
            .header(hyper::header::LOCATION, app_package::PAGE_PATH)
            .header(CACHE_CONTROL, "no-store")
            .body(Full::new(Bytes::new()))
            .expect("static response builds");
    }
    error_response(StatusCode::NOT_FOUND, "not_found", "Not found")
}

fn header_str<'a>(request: &'a Request<Incoming>, name: hyper::header::HeaderName) -> &'a str {
    request.headers().get(name).and_then(|value| value.to_str().ok()).unwrap_or("")
}

/// `/app` (the QR landing page, on every listener) and
/// `/app/locus-remote.apk` (while the bundled app is offered); `None` for
/// other paths.
async fn download_route(shared: &Shared, request: &Request<Incoming>) -> Option<Response<Body>> {
    let path = request.uri().path();
    let is_page = path == app_package::PAGE_PATH || path == "/app/";
    let is_apk = path == app_package::APK_PATH;
    if !(is_page || is_apk) || !matches!(*request.method(), Method::GET | Method::HEAD) {
        return None;
    }
    let package = shared.app_package().filter(|_| shared.app_download_enabled());
    let open = shared.app_download_open();
    if is_page {
        let state = match package {
            Some(package) if open => PageState::Available(package),
            Some(_) => PageState::Closed,
            None => PageState::NotOffered,
        };
        let html = app_package::render_page(
            state,
            header_str(request, hyper::header::USER_AGENT),
            header_str(request, hyper::header::ACCEPT_LANGUAGE),
        );
        return Some(
            base_response(StatusCode::OK)
                .header(CONTENT_TYPE, "text/html; charset=utf-8")
                .header(CACHE_CONTROL, "no-store")
                .header("Content-Security-Policy", app_package::page_csp())
                .body(Full::new(Bytes::from(html)))
                .expect("static response builds"),
        );
    }
    // No APK offered: the path does not exist (404 from the caller).
    let package = package?;
    if !open {
        return Some(error_response(StatusCode::GONE, "download_closed", "Open a pairing window on the desktop first"));
    }
    match shared.download.bytes(package).await {
        Ok(bytes) => Some(
            base_response(StatusCode::OK)
                .header(CONTENT_TYPE, "application/vnd.android.package-archive")
                .header(
                    hyper::header::CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}\"", package.file_name()),
                )
                .header(CACHE_CONTROL, "no-store")
                .body(Full::new(bytes))
                .expect("static response builds"),
        ),
        Err(error) => {
            eprintln!("[RemoteAccess] bundled app unavailable: {error}");
            Some(error_response(StatusCode::INTERNAL_SERVER_ERROR, "app_unavailable", "The bundled app is damaged"))
        }
    }
}

fn hello(shared: &Shared) -> Response<Body> {
    json_response(
        StatusCode::OK,
        &Hello {
            protocol: protocol::PROTOCOL_VERSION,
            desktop_id: shared.settings_snapshot().desktop_id,
            pairing: shared.pairing.is_open(),
        },
    )
}

fn record_failure(shared: &Shared, context: ConnContext) {
    if context.kind.per_peer_limits() {
        shared.guard.record_failure(context.peer.ip());
    }
}

fn bearer(request: &Request<Incoming>) -> Option<&str> {
    let value = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

#[allow(clippy::result_large_err)]
fn authenticate(
    shared: &Shared,
    request: &Request<Incoming>,
    context: ConnContext,
) -> Result<DeviceRecord, Response<Body>> {
    let idle_days = shared.settings_snapshot().token_idle_days;
    let now = now_ms();
    match bearer(request).and_then(|token| shared.devices.authenticate(token, idle_days, now)) {
        Some(device) => {
            shared.devices.touch(&device.id, Some(context.peer.ip().to_string()), now);
            Ok(device)
        }
        None => {
            record_failure(shared, context);
            Err(error_response(StatusCode::UNAUTHORIZED, "unauthorized", "Not paired"))
        }
    }
}

async fn pair(shared: &Arc<Shared>, request: Request<Incoming>, context: ConnContext) -> Response<Body> {
    let body = match Limited::new(request.into_body(), MAX_PAIR_BODY).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return error_response(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "Request too large"),
    };
    let Ok(pair) = serde_json::from_slice::<PairRequest>(&body) else {
        return error_response(StatusCode::BAD_REQUEST, "bad_request", "Malformed pairing request");
    };
    // A typed code is only safe where TLS is validated by a CA: requests that
    // arrive through a local proxy such as `tailscale serve`. Direct NIC
    // listeners use the self-signed certificate, which only the QR code
    // (fingerprint + secret) can authenticate.
    if pair.code.is_some() && context.kind.secure() {
        return error_response(
            StatusCode::FORBIDDEN,
            "code_not_allowed",
            "Scan the QR code to pair over a direct connection",
        );
    }
    if pair.code.as_deref().is_some_and(|code| !protocol::is_pairing_code(code)) {
        return error_response(StatusCode::BAD_REQUEST, "bad_request", "Malformed pairing code");
    }
    let session_id = match shared.pairing.verify(pair.secret.as_deref(), pair.code.as_deref()) {
        PairCheck::Accepted { session_id } => session_id,
        PairCheck::Rejected { remaining } => {
            record_failure(shared, context);
            shared.notify_changed();
            return error_response(
                StatusCode::FORBIDDEN,
                "invalid_credentials",
                format!("Wrong pairing credentials ({remaining} attempts left)"),
            );
        }
        PairCheck::Busy => {
            return error_response(StatusCode::CONFLICT, "pairing_busy", "Another device is pairing");
        }
        PairCheck::Closed => {
            return error_response(StatusCode::NOT_FOUND, "pairing_closed", "Pairing is not open");
        }
    };
    let device_name = sanitize_device_name(&pair.device_name);
    let platform = sanitize_platform(&pair.platform);
    let address = context.peer.ip().to_string();
    shared.notify_changed();

    let settings = shared.settings_snapshot();
    if settings.require_confirmation {
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        let decision = shared.pairing.await_confirmation(&request_id);
        shared.announce_pairing_request(&request_id, &device_name, &platform, &address);
        let allowed = matches!(
            tokio::time::timeout(CONFIRM_TIMEOUT, decision).await,
            Ok(Ok(true))
        );
        shared.pairing.drop_confirmation(&request_id);
        shared.pairing_request_settled(&request_id);
        if !allowed {
            shared.pairing.finish(&session_id);
            shared.notify_changed();
            return error_response(StatusCode::FORBIDDEN, "denied", "Pairing was not approved on the desktop");
        }
    }

    let mut secret = [0u8; protocol::TOKEN_BYTES];
    rand::rng().fill(&mut secret);
    let token = protocol::encode_token(&secret);
    secret.zeroize();
    let now = now_ms();
    let record = DeviceRecord {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: device_name.clone(),
        platform,
        paired_at_ms: now,
        last_seen_at_ms: now,
        last_address: Some(address),
        token_hash: protocol::token_hash(&token),
        full_control: false,
    };
    shared.pairing.finish(&session_id);
    if let Err(error) = shared.devices.add(record.clone(), now) {
        shared.notify_changed();
        return error_response(StatusCode::CONFLICT, "device_limit", error);
    }
    shared.notify_changed();
    shared.notify_desktop("Locus 远程访问", &format!("新设备已配对：{device_name}"));
    json_response(
        StatusCode::OK,
        &PairResponse {
            device_id: record.id,
            token,
            desktop_id: settings.desktop_id,
            desktop_name: super::config::desktop_name(),
            full_control: false,
        },
    )
}

fn upgrade(
    shared: Arc<Shared>,
    mut request: Request<Incoming>,
    device: DeviceRecord,
    context: ConnContext,
) -> Response<Body> {
    use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
    let headers = request.headers();
    let has_token = |name, token: &str| {
        headers
            .get(name)
            .and_then(|value: &HeaderValue| value.to_str().ok())
            .is_some_and(|value| {
                value.split(',').any(|part| part.trim().eq_ignore_ascii_case(token))
            })
    };
    let valid = request.version() == Version::HTTP_11
        && has_token(CONNECTION, "upgrade")
        && has_token(UPGRADE, "websocket")
        && headers
            .get("sec-websocket-version")
            .is_some_and(|value| value == "13");
    let Some(key) = headers.get("sec-websocket-key").filter(|_| valid) else {
        return error_response(StatusCode::BAD_REQUEST, "bad_upgrade", "WebSocket upgrade required");
    };
    let accept = derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => super::ws::run(shared, TokioIo::new(upgraded), device, context).await,
            Err(error) => eprintln!("[RemoteAccess] WebSocket upgrade failed: {error}"),
        }
    });
    base_response(StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header("Sec-WebSocket-Accept", accept)
        .body(Full::default())
        .expect("static response builds")
}

fn is_asset_path(path: &str) -> bool {
    path == "/m" || path.starts_with(protocol::ASSET_PREFIX) || assets::dev_or_bundle_path(path)
}

// ----- mobile page assets -----------------------------------------------------

mod assets {
    use super::*;
    use sha2::{Digest, Sha256};

    const PAGE: &str = "mobile.html";
    /// Vite dev-server paths the page needs while developing.
    const DEV_PREFIXES: &[&str] = &["/src/", "/node_modules/", "/@vite/", "/@id/", "/@fs/"];

    pub fn dev_or_bundle_path(path: &str) -> bool {
        path.starts_with("/assets/")
            || (tauri::is_dev() && DEV_PREFIXES.iter().any(|prefix| path.starts_with(prefix)))
    }

    fn page_path(path: &str) -> bool {
        matches!(path, "/m" | "/m/" | "/m/index.html")
    }

    fn dev_url(shared: &Shared) -> Option<String> {
        if !tauri::is_dev() {
            return None;
        }
        shared
            .app
            .config()
            .build
            .dev_url
            .as_ref()
            .map(|url| url.as_str().trim_end_matches('/').to_string())
    }

    /// Build id: hash of the bundled page (changes with every frontend build).
    pub fn build_id(shared: &Shared) -> String {
        if tauri::is_dev() {
            return "dev".into();
        }
        shared
            .app
            .asset_resolver()
            .get(PAGE.to_string())
            .map(|asset| {
                let digest = Sha256::digest(&asset.bytes);
                digest[..8].iter().map(|byte| format!("{byte:02x}")).collect()
            })
            .unwrap_or_else(|| "missing".into())
    }

    fn dev_client() -> &'static reqwest::Client {
        static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
        CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default()
        })
    }

    pub async fn serve(shared: &Shared, path: &str, query: Option<&str>) -> Response<Body> {
        let is_page = page_path(path);
        if !is_page && path.contains("..") {
            return error_response(StatusCode::NOT_FOUND, "not_found", "Not found");
        }
        if let Some(base) = dev_url(shared) {
            let upstream_path = if is_page { "/mobile.html" } else { path };
            let query = query.map(|query| format!("?{query}")).unwrap_or_default();
            return match dev_client().get(format!("{base}{upstream_path}{query}")).send().await {
                Ok(upstream) => {
                    let status = StatusCode::from_u16(upstream.status().as_u16())
                        .unwrap_or(StatusCode::BAD_GATEWAY);
                    let mime = upstream
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("application/octet-stream")
                        .to_string();
                    let bytes = match upstream.bytes().await {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            return error_response(StatusCode::BAD_GATEWAY, "dev_server", error.to_string())
                        }
                    };
                    base_response(status)
                        .header(CONTENT_TYPE, mime)
                        .header(CACHE_CONTROL, "no-store")
                        .body(Full::new(bytes))
                        .expect("static response builds")
                }
                Err(error) => error_response(StatusCode::BAD_GATEWAY, "dev_server", error.to_string()),
            };
        }
        let key = if is_page { PAGE } else { path.trim_start_matches('/') };
        let Some(asset) = shared.app.asset_resolver().get(key.to_string()) else {
            return error_response(StatusCode::NOT_FOUND, "not_found", "Mobile page is not bundled");
        };
        let builder = base_response(StatusCode::OK).header(CONTENT_TYPE, asset.mime_type);
        let builder = if is_page {
            builder
                .header(CACHE_CONTROL, "no-store")
                .header("Content-Security-Policy", MOBILE_CSP)
        } else {
            // Vite emits content-hashed file names under /assets.
            builder.header(CACHE_CONTROL, "private, max-age=31536000, immutable")
        };
        builder
            .body(Full::new(Bytes::from(asset.bytes)))
            .expect("static response builds")
    }
}

pub use assets::build_id;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_state_idle_rules() {
        let now = Instant::now();
        let mut state = PeerState::default();
        assert!(state.idle(now));
        state.connections = 1;
        assert!(!state.idle(now));
        state.connections = 0;
        state.banned_until = Some(now + BAN_DURATION);
        assert!(!state.idle(now));
    }

    #[test]
    fn asset_paths() {
        assert!(is_asset_path("/m/"));
        assert!(is_asset_path("/m"));
        assert!(is_asset_path("/assets/mobile-abc.js"));
        assert!(!is_asset_path("/index.html"));
        assert!(!is_asset_path("/window.html"));
    }
}
