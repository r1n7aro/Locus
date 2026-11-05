//! Tauri commands behind the desktop "Remote access" settings page.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use locus_remote_protocol::{self as protocol, CandidateAddress, PairingLink};

use super::app_package::{self, AppPackage, AppPackageView};
use super::devices::DeviceView;
use super::net::{self, ListenerKind, NetworkInterface};
use super::pairing::{self, PairingStatus};
use super::{apply, config, now_ms, reconcile, ListenerError, RemoteAccessHandle, RemoteAccessSettings, Shared};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerView {
    pub address: String,
    pub url: String,
    pub kind: ListenerKind,
    pub secure: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStateView {
    #[serde(flatten)]
    pub device: DeviceView,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessState {
    pub settings: RemoteAccessSettings,
    pub desktop_name: String,
    pub running: bool,
    pub listeners: Vec<ListenerView>,
    pub listener_errors: Vec<ListenerError>,
    pub last_error: Option<String>,
    pub interfaces: Vec<NetworkInterface>,
    /// `AB:CD:...` form of the pinned certificate, once generated.
    pub fingerprint: Option<String>,
    pub devices: Vec<DeviceStateView>,
    pub connections: usize,
    pub pairing: Option<PairingStatus>,
    pub tailscale_serve_command: String,
    /// Locus Remote APK bundled with this installer, offered on the LAN.
    pub app_package: Option<AppPackageView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    pub session_id: String,
    pub code: String,
    /// Whether the code can be used at all: only through a registered
    /// CA-trusted hostname. Direct connections must scan the QR code.
    pub code_usable: bool,
    pub link: String,
    pub qr_svg: String,
    pub expires_at_ms: i64,
    pub fingerprint: String,
    pub addresses: Vec<CandidateAddress>,
    /// Scanning with a phone camera opens this desktop's download page.
    pub app_download: bool,
}

/// Editable subset; `desktop_id` is never taken from the UI.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInput {
    pub enabled: bool,
    pub port: u16,
    pub loopback: bool,
    pub interfaces: Vec<String>,
    pub hostnames: Vec<String>,
    pub require_confirmation: bool,
    pub token_idle_days: u32,
    #[serde(default = "default_true")]
    pub app_download: bool,
}

fn default_true() -> bool {
    true
}

fn listener_url(address: &std::net::SocketAddr, kind: ListenerKind) -> String {
    let scheme = if kind.secure() { "https" } else { "http" };
    format!("{scheme}://{address}")
}

async fn build_state(handle: &RemoteAccessHandle, package: Option<&'static AppPackage>) -> RemoteAccessState {
    let settings = handle
        .shared()
        .map(|shared| shared.settings_snapshot())
        .unwrap_or_else(config::load_settings);
    let runtime = handle.runtime.lock().await;
    let shared = handle.shared();
    let connected = shared.map(|shared| shared.connections.connected_devices()).unwrap_or_default();
    let mut listeners: Vec<ListenerView> = runtime
        .listeners
        .iter()
        .map(|listener| ListenerView {
            address: listener.address.to_string(),
            url: listener_url(&listener.address, listener.kind),
            kind: listener.kind,
            secure: listener.kind.secure(),
        })
        .collect();
    listeners.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.address.cmp(&b.address)));
    RemoteAccessState {
        desktop_name: config::desktop_name(),
        running: !runtime.listeners.is_empty(),
        listeners,
        listener_errors: runtime.errors.clone(),
        last_error: runtime.last_error.clone(),
        interfaces: net::discover(),
        fingerprint: shared
            .and_then(|shared| shared.identity())
            .map(|identity| protocol::display_fingerprint(&identity.fingerprint)),
        devices: shared
            .map(|shared| {
                shared
                    .devices
                    .list()
                    .into_iter()
                    .map(|device| DeviceStateView { connected: connected.contains(&device.id), device })
                    .collect()
            })
            .unwrap_or_default(),
        connections: shared.map(|shared| shared.connections.count()).unwrap_or(0),
        pairing: shared.and_then(|shared| shared.pairing.status()),
        tailscale_serve_command: format!("tailscale serve --bg {}", settings.port),
        app_package: package.map(AppPackage::view),
        settings,
    }
}

/// Shared state for commands that act on devices or pairing; created on
/// demand so the settings page works before the service was ever enabled.
/// Takes the command's own AppHandle by value (no clone off the main thread).
fn shared_for(app: AppHandle, handle: &RemoteAccessHandle) -> Arc<Shared> {
    handle.shared_or_init(app, &config::load_settings())
}

#[tauri::command]
pub async fn remote_access_get_state(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
) -> Result<RemoteAccessState, String> {
    Ok(build_state(&handle, app_package::bundled(&app)).await)
}

/// Locus Remote APK bundled with this installer (the update check keeps
/// installers with and without it apart).
#[tauri::command]
pub fn remote_access_app_package(app: AppHandle) -> Option<AppPackageView> {
    app_package::bundled(&app).map(AppPackage::view)
}

#[tauri::command]
pub async fn remote_access_save_settings(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
    settings: SettingsInput,
) -> Result<RemoteAccessState, String> {
    let package = app_package::bundled(&app);
    let current = config::load_settings();
    let next = RemoteAccessSettings {
        enabled: settings.enabled,
        port: settings.port,
        loopback: settings.loopback,
        interfaces: settings.interfaces,
        hostnames: settings.hostnames,
        require_confirmation: settings.require_confirmation,
        token_idle_days: settings.token_idle_days,
        app_download: settings.app_download,
        desktop_id: current.desktop_id,
    }
    .normalized()?;
    config::save_settings(&next)?;
    reconcile(app).await;
    Ok(build_state(&handle, package).await)
}

#[tauri::command]
pub async fn remote_access_start_pairing(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
) -> Result<PairingView, String> {
    let shared = shared_for(app, &handle);
    let settings = shared.settings_snapshot();
    if !settings.enabled {
        return Err("请先开启远程访问".into());
    }
    let identity = shared.load_identity()?;
    let mut addresses: Vec<CandidateAddress> = {
        let runtime = handle.runtime.lock().await;
        let mut secure: Vec<_> = runtime
            .listeners
            .iter()
            .filter(|listener| listener.kind.secure())
            .map(|listener| (listener.kind, listener.address))
            .collect();
        // LAN first: it is the direct path when the phone is on the same network.
        secure.sort_by_key(|(kind, address)| (*kind != ListenerKind::Lan, *address));
        secure
            .into_iter()
            .map(|(kind, address)| CandidateAddress { url: listener_url(&address, kind), pinned: true })
            .collect()
    };
    addresses.extend(settings.hostnames.iter().map(|host| CandidateAddress {
        url: format!("https://{host}"),
        pinned: false,
    }));
    if addresses.is_empty() {
        return Err("没有可供手机连接的地址：请选择至少一个网卡，或登记一个 HTTPS 主机名".into());
    }
    let ticket = shared.pairing.start(now_ms());
    let link = PairingLink {
        version: protocol::PAIRING_LINK_VERSION,
        secret: ticket.secret.clone(),
        fingerprint: identity.fingerprint.clone(),
        addresses: addresses.clone(),
        desktop_id: settings.desktop_id.clone(),
        desktop_name: config::desktop_name(),
        expires_at_ms: ticket.expires_at_ms,
    };
    // The app reads only the fragment. A phone camera opens the carrier in a
    // browser, which would reject the pinned self-signed certificate, so the
    // carrier is the landing page: plain HTTP on a NIC address, the
    // CA-trusted name for registered hostnames. With the bundled app it is
    // also the download link.
    let app_download = shared.app_download_enabled();
    if app_download {
        shared.download.open(std::time::Instant::now());
    }
    let first = &addresses[0];
    let base = if first.pinned { first.url.replacen("https://", "http://", 1) } else { first.url.clone() };
    let url = link
        .to_url_at(&format!("{base}{}", app_package::PAGE_PATH))
        .map_err(|error| error.to_string())?;
    let qr_svg = pairing::qr_svg(&url)?;
    shared.notify_changed();
    Ok(PairingView {
        session_id: ticket.session_id,
        code: ticket.code,
        code_usable: !settings.hostnames.is_empty(),
        link: url,
        qr_svg,
        expires_at_ms: ticket.expires_at_ms,
        fingerprint: protocol::display_fingerprint(&identity.fingerprint),
        addresses,
        app_download,
    })
}

#[tauri::command]
pub async fn remote_access_cancel_pairing(
    handle: State<'_, Arc<RemoteAccessHandle>>,
) -> Result<(), String> {
    if let Some(shared) = handle.shared() {
        shared.pairing.cancel();
        shared.notify_changed();
    }
    Ok(())
}

#[tauri::command]
pub async fn remote_access_resolve_pairing(
    handle: State<'_, Arc<RemoteAccessHandle>>,
    request_id: String,
    allow: bool,
) -> Result<bool, String> {
    Ok(handle
        .shared()
        .is_some_and(|shared| shared.pairing.resolve_confirmation(&request_id, allow)))
}

#[tauri::command]
pub async fn remote_access_set_device_permission(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
    device_id: String,
    full_control: bool,
) -> Result<(), String> {
    let shared = shared_for(app, &handle);
    shared.devices.set_full_control(&device_id, full_control, now_ms())?;
    shared.notify_changed();
    Ok(())
}

#[tauri::command]
pub async fn remote_access_rename_device(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
    device_id: String,
    name: String,
) -> Result<(), String> {
    let shared = shared_for(app, &handle);
    shared.devices.rename(&device_id, &name, now_ms())?;
    shared.notify_changed();
    Ok(())
}

#[tauri::command]
pub async fn remote_access_revoke_device(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
    device_id: String,
) -> Result<(), String> {
    let shared = shared_for(app, &handle);
    shared.devices.revoke(&device_id, now_ms())?;
    shared.connections.close_where("revoked", |connection| connection.device_id == device_id);
    shared.notify_changed();
    Ok(())
}

#[tauri::command]
pub async fn remote_access_revoke_all_devices(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
) -> Result<(), String> {
    let shared = shared_for(app, &handle);
    shared.devices.revoke_all(now_ms())?;
    shared.connections.close_all("revoked");
    shared.notify_changed();
    Ok(())
}

/// New certificate: every pinned app would fail anyway, so all pairings end.
#[tauri::command]
pub async fn remote_access_reset_certificate(
    app: AppHandle,
    handle: State<'_, Arc<RemoteAccessHandle>>,
) -> Result<RemoteAccessState, String> {
    let package = app_package::bundled(&app);
    let shared = shared_for(app, &handle);
    shared.pairing.cancel();
    shared.replace_identity()?;
    shared.devices.revoke_all(now_ms())?;
    shared.connections.close_all("certificate_reset");
    apply(&handle, &shared, config::load_settings()).await;
    Ok(build_state(&handle, package).await)
}

/// Opens "Allow an app through Windows Firewall".
#[tauri::command]
pub async fn remote_access_open_firewall_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("control.exe")
            .args(["/name", "Microsoft.WindowsFirewall", "/page", "pageConfigureApps"])
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("无法打开防火墙设置：{error}"))
    }
    #[cfg(not(windows))]
    {
        Err("仅在 Windows 上可用".into())
    }
}
