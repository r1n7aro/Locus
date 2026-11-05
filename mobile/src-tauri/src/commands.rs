//! App state and the commands of the two pages:
//! - the bundled launcher (`tauri.localhost`): pairing and desktop management;
//! - the desktop-served page (`locus-remote.localhost`): `remote_*` only.
//! Management commands check the calling page, so code served by a desktop
//! can never read tokens, pair, or forget desktops.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tauri::ipc::{Channel, InvokeResponseBody, Response};
use tauri::{AppHandle, Manager, State, Url, Webview};

use locus_remote_protocol::{self as protocol, CandidateAddress, PairRequest, PairingLink, RpcError};

use crate::asset_cache::AssetCache;
use crate::client::{ClientError, HttpClient};
use crate::connection::{Connection, ConnectionStatus};
use crate::secure::Sealer;
use crate::store::{DesktopRecord, DesktopStore, DesktopView};

pub struct AppState {
    store: Arc<Mutex<DesktopStore>>,
    sealer: Sealer,
    active: Mutex<Option<Arc<Connection>>>,
    pub cache: AssetCache,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

impl AppState {
    pub fn new(store: DesktopStore, sealer: Sealer, cache: AssetCache) -> Self {
        Self { store: Arc::new(Mutex::new(store)), sealer, active: Mutex::new(None), cache }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, DesktopStore> {
        self.store.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn active_connection(&self) -> Option<Arc<Connection>> {
        self.active.lock().unwrap_or_else(|error| error.into_inner()).clone()
    }

    fn replace_active(&self, next: Option<Arc<Connection>>) {
        let previous = std::mem::replace(&mut *self.active.lock().unwrap_or_else(|error| error.into_inner()), next);
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    /// Disconnects and shows the launcher again.
    pub fn leave(&self, app: &AppHandle) {
        self.replace_active(None);
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(url) = launcher_url(app) {
                let _ = window.navigate(url);
            }
        }
    }
}

pub fn launcher_url(app: &AppHandle) -> Result<Url, String> {
    if tauri::is_dev() {
        if let Some(url) = app.config().build.dev_url.clone() {
            return Ok(url);
        }
    }
    let url = if cfg!(any(windows, target_os = "android")) { "http://tauri.localhost/" } else { "tauri://localhost/" };
    Url::parse(url).map_err(|error| error.to_string())
}

/// The bundled launcher, not the desktop-served page.
fn require_launcher(app: &AppHandle, webview: &Webview) -> Result<(), String> {
    let url = webview.url().map_err(|error| error.to_string())?;
    let launcher = launcher_url(app)?;
    let same_origin = url.scheme() == launcher.scheme()
        && url.host_str() == launcher.host_str()
        && url.port_or_known_default() == launcher.port_or_known_default();
    if same_origin {
        Ok(())
    } else {
        Err("Not allowed from this page".into())
    }
}

fn sanitize_device_name(raw: &str) -> String {
    let name: String = raw.chars().filter(|ch| !ch.is_control()).take(64).collect();
    let name = name.trim();
    if name.is_empty() { "手机".into() } else { name.to_string() }
}

fn platform() -> &'static str {
    if cfg!(target_os = "android") { "android" } else { std::env::consts::OS }
}

fn pairing_error(error: ClientError) -> String {
    match error {
        ClientError::Rejected { code, message, .. } => match code.as_str() {
            "denied" => "电脑上拒绝了这次配对".into(),
            "invalid_credentials" => format!("配对凭据不正确（{message}）"),
            "pairing_closed" => "电脑上的配对已结束，请重新生成二维码".into(),
            "pairing_busy" => "另一台设备正在配对，请稍后再试".into(),
            "code_not_allowed" => "直连地址需要扫码配对".into(),
            "device_limit" => message,
            _ => message,
        },
        other => other.to_string(),
    }
}

async fn finish_pairing(
    state: &AppState,
    response: protocol::PairResponse,
    addresses: Vec<CandidateAddress>,
    fingerprint: String,
    device_name: String,
    used_address: &str,
) -> Result<DesktopView, String> {
    let mut record = DesktopRecord {
        id: response.desktop_id,
        name: response.desktop_name,
        addresses,
        fingerprint,
        device_id: response.device_id,
        device_name,
        paired_at_ms: now_ms(),
        last_address: Some(used_address.to_string()),
        sealed_token: String::new(),
    };
    record.sealed_token = state.sealer.seal(&record.key_alias(), &response.token).await?;
    let view = DesktopView::from(&record);
    state.store().upsert(record)?;
    Ok(view)
}

#[tauri::command]
pub async fn launcher_list_desktops(app: AppHandle, webview: Webview, state: State<'_, AppState>) -> Result<Vec<DesktopView>, String> {
    require_launcher(&app, &webview)?;
    Ok(state.store().list())
}

/// Pairs through the QR code: certificate pinned to the fingerprint it carries.
#[tauri::command]
pub async fn launcher_pair_with_link(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    link: String,
    device_name: String,
) -> Result<DesktopView, String> {
    require_launcher(&app, &webview)?;
    let link = PairingLink::parse(&link).map_err(|_| "这不是 Locus 的配对二维码".to_string())?;
    // A minute of slack for phone/desktop clock skew; the desktop enforces
    // the real expiry.
    if now_ms() > link.expires_at_ms + 60_000 {
        return Err("二维码已过期，请在电脑上重新生成".into());
    }
    let device_name = sanitize_device_name(&device_name);
    let mut pin_mismatch = false;
    let mut errors = Vec::new();
    for address in &link.addresses {
        let client = HttpClient::new(&address.url, address.pinned.then_some(link.fingerprint.as_str()))?;
        let hello = match client.hello().await {
            Ok(hello) => hello,
            Err(ClientError::PinMismatch) => {
                pin_mismatch = true;
                continue;
            }
            Err(error) => {
                errors.push(error.to_string());
                continue;
            }
        };
        if hello.protocol != protocol::PROTOCOL_VERSION {
            return Err("电脑上的 Locus 与此 App 版本不兼容，请升级两端到同一版本".into());
        }
        if hello.desktop_id != link.desktop_id {
            errors.push(format!("{} 不是二维码中的电脑", address.url));
            continue;
        }
        if !hello.pairing {
            return Err("电脑上的配对已结束，请重新生成二维码".into());
        }
        let request = PairRequest {
            secret: Some(link.secret.clone()),
            code: None,
            device_name: device_name.clone(),
            platform: platform().into(),
        };
        let response = client.pair(&request).await.map_err(pairing_error)?;
        if response.desktop_id != link.desktop_id {
            return Err("配对响应来自另一台电脑".into());
        }
        return finish_pairing(&state, response, link.addresses.clone(), link.fingerprint.clone(), device_name, &address.url).await;
    }
    if pin_mismatch {
        return Err("证书与二维码不一致：可能存在中间人攻击，已停止配对".into());
    }
    Err(format!("连不上电脑：{}", errors.join("；")))
}

/// Pairs with the 6-digit code, only over CA-validated HTTPS.
#[tauri::command]
pub async fn launcher_pair_with_code(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    url: String,
    code: String,
    device_name: String,
) -> Result<DesktopView, String> {
    require_launcher(&app, &webview)?;
    let origin = url.trim().trim_end_matches('/').to_string();
    if !protocol::is_https_origin(&origin) {
        return Err("请输入 https:// 开头的地址，例如 https://my-pc.tailnet.ts.net".into());
    }
    let code = code.trim().replace(' ', "");
    if !protocol::is_pairing_code(&code) {
        return Err("配对码是 6 位数字".into());
    }
    let device_name = sanitize_device_name(&device_name);
    let client = HttpClient::new(&origin, None)?;
    let hello = client.hello().await.map_err(|error| error.to_string())?;
    if hello.protocol != protocol::PROTOCOL_VERSION {
        return Err("电脑上的 Locus 与此 App 版本不兼容，请升级两端到同一版本".into());
    }
    if !hello.pairing {
        return Err("电脑上没有正在进行的配对".into());
    }
    let request = PairRequest { secret: None, code: Some(code), device_name: device_name.clone(), platform: platform().into() };
    let response = client.pair(&request).await.map_err(pairing_error)?;
    let addresses = vec![CandidateAddress { url: origin.clone(), pinned: false }];
    finish_pairing(&state, response, addresses, String::new(), device_name, &origin).await
}

/// Starts the connection and shows the desktop's page.
#[tauri::command]
pub async fn launcher_connect(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    desktop_id: String,
) -> Result<(), String> {
    require_launcher(&app, &webview)?;
    let record = state.store().get(&desktop_id).ok_or("找不到这台电脑")?;
    let token = state.sealer.open(&record.key_alias(), &record.sealed_token).await?;
    let store = state.store.clone();
    let id = record.id.clone();
    let connection = Connection::new(
        record.id.clone(),
        record.name.clone(),
        record.device_name.clone(),
        record.ordered_addresses(),
        record.fingerprint.clone(),
        token,
        move |url| store.lock().unwrap_or_else(|error| error.into_inner()).set_last_address(&id, url),
    );
    connection.start();
    state.replace_active(Some(connection));
    state.cache.prune();
    let url = Url::parse(crate::proxy::page_url()).map_err(|error| error.to_string())?;
    webview.navigate(url).map_err(|error| error.to_string())
}

/// Unpairs on the desktop when reachable, then forgets the desktop locally.
#[tauri::command]
pub async fn launcher_forget(
    app: AppHandle,
    webview: Webview,
    state: State<'_, AppState>,
    desktop_id: String,
) -> Result<(), String> {
    require_launcher(&app, &webview)?;
    if state.active_connection().is_some_and(|connection| connection.desktop_id == desktop_id) {
        state.replace_active(None);
    }
    let Some(record) = state.store().get(&desktop_id) else {
        return Ok(());
    };
    if let Ok(token) = state.sealer.open(&record.key_alias(), &record.sealed_token).await {
        for address in record.ordered_addresses() {
            let pinned = address.pinned.then_some(record.fingerprint.as_str());
            if let Ok(client) = HttpClient::new(&address.url, pinned) {
                if client.unpair(&token).await.is_ok() {
                    break;
                }
            }
        }
    }
    state.sealer.forget(&record.key_alias()).await;
    state.cache.forget_desktop(&desktop_id);
    state.store().remove(&desktop_id)?;
    Ok(())
}

#[tauri::command]
pub fn remote_subscribe(state: State<'_, AppState>, on_message: Channel<InvokeResponseBody>) -> Result<(), String> {
    let connection = state.active_connection().ok_or("尚未连接电脑")?;
    connection.subscribe(move |json| on_message.send(InvokeResponseBody::Json(json)).is_ok());
    Ok(())
}

/// Relays one RPC; the result is the desktop's JSON, passed through as is.
#[tauri::command]
pub async fn remote_invoke(state: State<'_, AppState>, method: String, params: Value) -> Result<Response, RpcError> {
    let connection = state
        .active_connection()
        .ok_or_else(|| RpcError { code: "closed".into(), message: "尚未连接电脑".into(), data: None })?;
    let json = connection.call(&method, params).await?;
    Ok(Response::new(json))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    #[serde(flatten)]
    status: ConnectionStatus,
    address: Option<String>,
}

#[tauri::command]
pub fn remote_status(state: State<'_, AppState>) -> Result<RemoteStatus, String> {
    let connection = state.active_connection().ok_or("尚未连接电脑")?;
    Ok(RemoteStatus { status: connection.status(), address: connection.active_address() })
}

#[tauri::command]
pub fn remote_wake(state: State<'_, AppState>) {
    if let Some(connection) = state.active_connection() {
        connection.wake();
    }
}

#[tauri::command]
pub fn remote_leave(app: AppHandle, state: State<'_, AppState>) {
    state.leave(&app);
}
