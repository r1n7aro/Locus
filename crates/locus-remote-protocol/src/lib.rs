//! Wire protocol shared by the Locus desktop remote service
//! (`src-tauri/src/remote_access`) and the Locus Remote mobile app
//! (`mobile/src-tauri`). Both sides must build against the same revision;
//! bump [`PROTOCOL_VERSION`] on every incompatible change.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Major protocol version. Clients refuse desktops that report another value.
pub const PROTOCOL_VERSION: u32 = 1;

pub const HELLO_PATH: &str = "/remote/v1/hello";
pub const PAIR_PATH: &str = "/remote/v1/pair";
pub const UNPAIR_PATH: &str = "/remote/v1/unpair";
pub const WS_PATH: &str = "/remote/v1/ws";
/// Mobile page assets (`mobile.html` and its bundle) live under this prefix.
pub const ASSET_PREFIX: &str = "/m/";
/// Binary previews (images, media) by blob id: `{BLOB_PREFIX}{uuid}`. Command
/// results reference them as `http://locus-binary.localhost/blob/{uuid}`.
pub const BLOB_PREFIX: &str = "/remote/v1/blob/";
/// URL prefix of binary previews inside command results.
pub const BLOB_URL_PREFIX: &str = "http://locus-binary.localhost/blob/";

pub const TOKEN_PREFIX: &str = "lrt_";
pub const TOKEN_BYTES: usize = 32;
pub const PAIRING_SECRET_BYTES: usize = 16;
pub const PAIRING_CODE_LEN: usize = 6;
pub const PAIRING_LINK_VERSION: u32 = 1;
/// Upper bound for one WebSocket message in either direction. Session pages
/// with inline images are the largest legitimate payloads.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Methods a paired device may call over the WebSocket. Anything else is
/// rejected by the desktop before dispatch.
pub mod methods {
    pub const LIST_PROJECT_CONTEXTS: &str = "list_project_contexts";
    pub const LIST_WORKSPACE_RUNTIMES: &str = "list_workspace_runtimes";
    pub const LIST_PROJECT_SESSIONS: &str = "list_project_sessions";
    pub const LIST_CHECKOUT_SESSIONS: &str = "list_checkout_sessions";
    pub const LOAD_SESSION_VIEW: &str = "load_session_view";
    pub const LOAD_SESSION_MESSAGE_PAGE: &str = "load_session_message_page";
    pub const LOAD_SESSION_MESSAGE_IMAGES: &str = "load_session_message_images";
    pub const GET_SESSION_ACTIVE_RUN: &str = "get_session_active_run";
    pub const CHAT: &str = "chat";
    pub const QUEUE_CHAT_INPUT: &str = "queue_chat_input";
    pub const INSERT_PENDING_CHAT_INPUT: &str = "insert_pending_chat_input";
    pub const DELETE_PENDING_CHAT_INPUT: &str = "delete_pending_chat_input";
    pub const CANCEL_CHAT: &str = "cancel_chat";
    pub const ANSWER_QUESTION: &str = "answer_question";
    pub const GET_TODOS: &str = "get_todos";
    pub const GET_SESSION_USAGE: &str = "get_session_usage";
    pub const GET_SESSION_RESUME_AVAILABLE: &str = "get_session_resume_available";
    pub const GET_SESSION_PLAN_STATE: &str = "get_session_plan_state";
    /// Default models; a new session started from the phone uses `mainModel`.
    pub const GET_MODEL_DEFAULTS: &str = "get_model_defaults";
    /// Replaces the connection's workspace event interests.
    pub const SET_INTERESTS: &str = "set_interests";
    /// Replaces the files and folders whose changes the connection receives
    /// (`{"watches": [FileWatch]}`); file-change events are not forwarded
    /// otherwise.
    pub const SET_FILE_WATCHES: &str = "set_file_watches";
    /// Returns [`super::DeviceInfo`] for the calling device.
    pub const GET_DEVICE_INFO: &str = "get_device_info";

    /// Syntactic check only: which methods a device may call is decided by
    /// the desktop (remote access policy) and reported in
    /// [`super::DeviceInfo::methods`].
    pub fn is_valid_name(method: &str) -> bool {
        (1..=80).contains(&method.len())
            && method.as_bytes()[0].is_ascii_lowercase()
            && method.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }
}

/// RPC error codes carried in [`RpcError::code`].
pub mod error_codes {
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const INVALID_PARAMS: &str = "invalid_params";
    pub const FORBIDDEN: &str = "forbidden";
    pub const FAILED: &str = "failed";
    pub const BUSY: &str = "busy";
    /// The result exceeds [`MAX_FRAME_BYTES`](super::MAX_FRAME_BYTES).
    pub const TOO_LARGE: &str = "too_large";
}

/// Upper bound of [`methods::SET_FILE_WATCHES`] entries per connection.
pub const MAX_FILE_WATCHES: usize = 64;

/// A workspace path a device shows: a file, or a folder whose entries it lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileWatch {
    pub checkout_id: String,
    /// Workspace-relative, `/`-separated; empty for the workspace root.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidPairingLink(&'static str),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPairingLink(reason) => write!(f, "invalid pairing link: {reason}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

/// `GET /remote/v1/hello`. Unauthenticated, so it carries only what a client
/// needs before pairing: the protocol version, the random desktop id (to
/// confirm it reached the desktop named in the QR code) and whether pairing
/// is open. Names and build details are returned only after authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol: u32,
    pub desktop_id: String,
    pub pairing: bool,
}

/// `POST /remote/v1/pair`. Exactly one of `secret` (QR) and `code` (typed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub device_name: String,
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    pub device_id: String,
    pub token: String,
    pub desktop_id: String,
    pub desktop_name: String,
    pub full_control: bool,
}

/// Result of [`methods::GET_DEVICE_INFO`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device_id: String,
    pub device_name: String,
    /// May approve tool calls and use commands that change things.
    pub full_control: bool,
    pub desktop_id: String,
    pub desktop_name: String,
    /// Changes whenever the desktop's mobile page bundle changes.
    pub build_id: String,
    /// Every method this device may call right now (depends on
    /// `full_control`).
    pub methods: Vec<String>,
}

/// JSON body of every non-2xx HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

/// One address the app may try, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateAddress {
    /// Origin such as `https://192.168.1.20:47831`.
    #[serde(rename = "u")]
    pub url: String,
    /// `true`: trust exactly the pinned certificate (direct NIC listener).
    /// `false`: normal CA validation (e.g. a `tailscale serve` hostname).
    #[serde(rename = "pin")]
    pub pinned: bool,
}

/// Contents of the pairing QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingLink {
    #[serde(rename = "v")]
    pub version: u32,
    #[serde(rename = "s")]
    pub secret: String,
    /// [`certificate_fingerprint`] of the desktop's self-signed certificate.
    #[serde(rename = "fp")]
    pub fingerprint: String,
    #[serde(rename = "a")]
    pub addresses: Vec<CandidateAddress>,
    #[serde(rename = "id")]
    pub desktop_id: String,
    #[serde(rename = "n")]
    pub desktop_name: String,
    /// Unix epoch milliseconds.
    #[serde(rename = "x")]
    pub expires_at_ms: i64,
}

impl PairingLink {
    /// `https://<first address>/m/#p=<base64url(json)>`. The secret stays in
    /// the fragment, which clients never send to a server.
    pub fn to_url(&self) -> Result<String, ProtocolError> {
        self.validate()?;
        let base = self.addresses[0].url.trim_end_matches('/');
        self.to_url_at(&format!("{base}{ASSET_PREFIX}"))
    }

    /// `<carrier>#p=<base64url(json)>`. The app reads only the fragment, so
    /// the carrier is free to point at whatever a plain camera scan should
    /// open (the desktop's download page). It is never trusted: addresses
    /// and pin come from the payload.
    pub fn to_url_at(&self, carrier: &str) -> Result<String, ProtocolError> {
        self.validate()?;
        let carrier_ok = (carrier.starts_with("https://") || carrier.starts_with("http://"))
            && !carrier.contains(['#', ' '])
            && !carrier.chars().any(char::is_control);
        if !carrier_ok {
            return Err(ProtocolError::InvalidPairingLink("invalid carrier URL"));
        }
        let json = serde_json::to_vec(self)
            .map_err(|_| ProtocolError::InvalidPairingLink("serialization failed"))?;
        Ok(format!("{carrier}#p={}", URL_SAFE_NO_PAD.encode(json)))
    }

    /// Accepts the full URL, the `p=` fragment, or the bare base64 payload.
    pub fn parse(input: &str) -> Result<Self, ProtocolError> {
        let input = input.trim();
        let payload = match input.split_once('#') {
            Some((_, fragment)) => fragment,
            None => input,
        };
        let payload = payload
            .split('&')
            .find_map(|part| part.strip_prefix("p="))
            .unwrap_or(payload);
        let bytes = URL_SAFE_NO_PAD
            .decode(payload.as_bytes())
            .map_err(|_| ProtocolError::InvalidPairingLink("payload is not base64url"))?;
        let link: PairingLink = serde_json::from_slice(&bytes)
            .map_err(|_| ProtocolError::InvalidPairingLink("payload is not a pairing link"))?;
        link.validate()?;
        Ok(link)
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.version != PAIRING_LINK_VERSION {
            return Err(ProtocolError::InvalidPairingLink("unsupported version"));
        }
        let secret_ok = URL_SAFE_NO_PAD
            .decode(self.secret.as_bytes())
            .is_ok_and(|bytes| bytes.len() == PAIRING_SECRET_BYTES);
        if !secret_ok {
            return Err(ProtocolError::InvalidPairingLink("malformed secret"));
        }
        if !is_fingerprint(&self.fingerprint) {
            return Err(ProtocolError::InvalidPairingLink("malformed fingerprint"));
        }
        if self.addresses.is_empty() || self.addresses.len() > 16 {
            return Err(ProtocolError::InvalidPairingLink("no usable address"));
        }
        if !self
            .addresses
            .iter()
            .all(|address| is_https_origin(&address.url))
        {
            return Err(ProtocolError::InvalidPairingLink("addresses must be https origins"));
        }
        if self.desktop_id.is_empty() || self.desktop_id.len() > 128 {
            return Err(ProtocolError::InvalidPairingLink("malformed desktop id"));
        }
        Ok(())
    }
}

/// `https://host[:port]` without path, query, fragment or credentials.
pub fn is_https_origin(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    let rest = rest.trim_end_matches('/');
    !rest.is_empty()
        && !rest.contains(['/', '?', '#', '@', ' '])
        && rest.len() <= 255
}

/// `wss://host:port/remote/v1/ws` for an `https://host:port` origin.
pub fn websocket_url(origin: &str) -> Option<String> {
    let rest = origin.trim_end_matches('/').strip_prefix("https://")?;
    Some(format!("wss://{rest}{WS_PATH}"))
}

/// Frames sent by the app.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientFrame {
    Request {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
    Ping {
        #[serde(default)]
        nonce: u64,
    },
}

/// Frames sent by the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerFrame {
    Response {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
    /// A routed workspace event (the payload Tauri windows receive).
    Event { payload: Value },
    /// Events were dropped; the client must reload what it shows.
    Resync { reason: String },
    Pong { nonce: u64 },
    /// The desktop is closing this connection (revoked, disabled, shutdown).
    Closing { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
    /// The command's own error value (Locus `AppError` JSON) when the
    /// command itself failed, so clients handle it like a local invoke error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ServerFrame {
    pub fn ok(id: u64, result: Value) -> Self {
        Self::Response { id, ok: true, result: Some(result), error: None }
    }

    pub fn err(id: u64, code: &str, message: impl Into<String>) -> Self {
        Self::err_with_data(id, code, message, None)
    }

    pub fn err_with_data(id: u64, code: &str, message: impl Into<String>, data: Option<Value>) -> Self {
        Self::Response {
            id,
            ok: false,
            result: None,
            error: Some(RpcError { code: code.to_string(), message: message.into(), data }),
        }
    }
}

/// Lowercase hex SHA-256 of a DER certificate.
pub fn certificate_fingerprint(der: &[u8]) -> String {
    hex(&Sha256::digest(der))
}

pub fn is_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `AB:CD:...` grouping for display and manual comparison.
pub fn display_fingerprint(fingerprint: &str) -> String {
    fingerprint
        .as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

/// `lrt_` + base64url of [`TOKEN_BYTES`] random bytes.
pub fn encode_token(bytes: &[u8; TOKEN_BYTES]) -> String {
    format!("{TOKEN_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn is_token(value: &str) -> bool {
    value
        .strip_prefix(TOKEN_PREFIX)
        .and_then(|body| URL_SAFE_NO_PAD.decode(body.as_bytes()).ok())
        .is_some_and(|bytes| bytes.len() == TOKEN_BYTES)
}

/// Lowercase hex SHA-256 of a token; the desktop stores only this.
pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

pub fn encode_pairing_secret(bytes: &[u8; PAIRING_SECRET_BYTES]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn is_pairing_code(value: &str) -> bool {
    value.len() == PAIRING_CODE_LEN && value.bytes().all(|b| b.is_ascii_digit())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn link() -> PairingLink {
        PairingLink {
            version: PAIRING_LINK_VERSION,
            secret: encode_pairing_secret(&[7u8; PAIRING_SECRET_BYTES]),
            fingerprint: certificate_fingerprint(b"certificate"),
            addresses: vec![
                CandidateAddress { url: "https://192.168.1.20:47831".into(), pinned: true },
                CandidateAddress { url: "https://pc.tail1234.ts.net".into(), pinned: false },
            ],
            desktop_id: "desk-1".into(),
            desktop_name: "Workstation".into(),
            expires_at_ms: 1_800_000_000_000,
        }
    }

    #[test]
    fn fingerprint_matches_known_sha256_vector() {
        assert_eq!(
            certificate_fingerprint(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(is_fingerprint(&certificate_fingerprint(b"abc")));
        assert!(!is_fingerprint("BA7816BF"));
        assert_eq!(&display_fingerprint("ba78")[..], "BA:78");
    }

    #[test]
    fn pairing_link_round_trips_through_url_fragment() {
        let original = link();
        let url = original.to_url().unwrap();
        assert!(url.starts_with("https://192.168.1.20:47831/m/#p="));
        assert_eq!(PairingLink::parse(&url).unwrap(), original);
        let fragment = url.split_once('#').unwrap().1;
        assert_eq!(PairingLink::parse(fragment).unwrap(), original);
        let payload = fragment.strip_prefix("p=").unwrap();
        assert_eq!(PairingLink::parse(payload).unwrap(), original);

        // Download-page carrier: same payload, any host.
        let carried = original.to_url_at("http://192.168.1.20:47831/app").unwrap();
        assert!(carried.starts_with("http://192.168.1.20:47831/app#p="));
        assert_eq!(PairingLink::parse(&carried).unwrap(), original);
        assert!(original.to_url_at("http://x/app#y").is_err());
        assert!(original.to_url_at("javascript:alert(1)").is_err());
    }

    #[test]
    fn pairing_link_rejects_unsafe_content() {
        let mut bad = link();
        bad.addresses[0].url = "http://192.168.1.20:47831".into();
        assert!(bad.to_url().is_err());
        let mut bad = link();
        bad.secret = "short".into();
        assert!(bad.validate().is_err());
        let mut bad = link();
        bad.fingerprint = "00".into();
        assert!(bad.validate().is_err());
        let mut bad = link();
        bad.addresses.clear();
        assert!(bad.validate().is_err());
        assert!(PairingLink::parse("https://x/m/#p=!!!").is_err());
    }

    #[test]
    fn origins_and_websocket_urls() {
        assert!(is_https_origin("https://10.0.0.2:4000"));
        assert!(is_https_origin("https://pc.example.ts.net/"));
        assert!(!is_https_origin("https://user@host"));
        assert!(!is_https_origin("https://host/path"));
        assert!(!is_https_origin("http://host"));
        assert_eq!(
            websocket_url("https://10.0.0.2:4000/").as_deref(),
            Some("wss://10.0.0.2:4000/remote/v1/ws")
        );
    }

    #[test]
    fn tokens_and_codes() {
        let token = encode_token(&[1u8; TOKEN_BYTES]);
        assert!(token.starts_with(TOKEN_PREFIX));
        assert!(is_token(&token));
        assert!(!is_token("lrt_short"));
        assert_eq!(token_hash(&token).len(), 64);
        assert!(is_pairing_code("012345"));
        assert!(!is_pairing_code("12345"));
        assert!(!is_pairing_code("12a456"));
    }

    #[test]
    fn frames_use_tagged_camel_case_json() {
        let request: ClientFrame = serde_json::from_value(
            json!({"type": "request", "id": 3, "method": "chat", "params": {"text": "hi"}}),
        )
        .unwrap();
        assert_eq!(
            request,
            ClientFrame::Request { id: 3, method: "chat".into(), params: json!({"text": "hi"}) }
        );
        assert_eq!(
            serde_json::to_value(ServerFrame::ok(3, json!(true))).unwrap(),
            json!({"type": "response", "id": 3, "ok": true, "result": true})
        );
        assert_eq!(
            serde_json::to_value(ServerFrame::err(4, error_codes::FORBIDDEN, "no")).unwrap(),
            json!({"type": "response", "id": 4, "ok": false, "error": {"code": "forbidden", "message": "no"}})
        );
        assert_eq!(
            serde_json::to_value(ServerFrame::Resync { reason: "overflow".into() }).unwrap(),
            json!({"type": "resync", "reason": "overflow"})
        );
    }

    #[test]
    fn method_names() {
        assert!(methods::is_valid_name(methods::CHAT));
        assert!(methods::is_valid_name("workspace_file_preview"));
        assert!(!methods::is_valid_name(""));
        assert!(!methods::is_valid_name("plugin:fs|read"));
        assert!(!methods::is_valid_name("Chat"));
        assert!(!methods::is_valid_name("_x"));
        assert!(!methods::is_valid_name(&"a".repeat(81)));
    }
}
