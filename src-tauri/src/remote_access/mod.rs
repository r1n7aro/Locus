//! Mobile remote access: a paired Locus Remote app browses sessions, sends
//! messages and answers questions over the LAN, Tailscale, or a tunnel the
//! user chose.
//!
//! Off by default. While disabled nothing listens, no task runs and the
//! device store is not even loaded; `reconcile` only reads the settings file.

mod app_package;
mod blobs;
pub mod commands;
pub(crate) mod config;
mod connections;
mod devices;
mod file_watch;
mod invoke;
mod net;
mod pairing;
pub mod policy;
mod server;
mod tls;
mod ws;

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

pub use config::RemoteAccessSettings;
use connections::ConnectionRegistry;
use devices::DeviceStore;
use net::ListenerKind;
use pairing::PairingState;

/// Desktop UI: state changed, refetch with `remote_access_get_state`.
pub const CHANGED_EVENT: &str = "remote-access-changed";
/// Desktop UI: a device presented valid credentials and awaits approval.
pub const PAIRING_REQUEST_EVENT: &str = "remote-access-pairing-request";
/// Desktop UI: an approval request was answered or timed out.
pub const PAIRING_SETTLED_EVENT: &str = "remote-access-pairing-settled";

const MAX_CONNECTIONS: usize = 64;
const INTERFACE_POLL: Duration = Duration::from_secs(30);

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// State shared by listeners, sessions and commands. Created on first use.
pub(crate) struct Shared {
    /// The only AppHandle the service owns; worker threads borrow it and
    /// never clone it (see `invoke`).
    pub app: Arc<AppHandle>,
    settings: RwLock<RemoteAccessSettings>,
    pub devices: DeviceStore,
    pub pairing: PairingState,
    pub connections: Arc<ConnectionRegistry>,
    pub guard: server::AbuseGuard,
    pub connection_slots: Arc<Semaphore>,
    identity: RwLock<Option<Arc<tls::Identity>>>,
    build_id: OnceLock<String>,
    pub download: app_package::DownloadWindow,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequest {
    pub request_id: String,
    pub device_name: String,
    pub platform: String,
    pub address: String,
    pub expires_at_ms: i64,
}

impl Shared {
    fn new(app: AppHandle, settings: RemoteAccessSettings) -> Self {
        Self {
            app: Arc::new(app),
            settings: RwLock::new(settings),
            devices: DeviceStore::load_default(),
            pairing: PairingState::default(),
            connections: Arc::new(ConnectionRegistry::default()),
            guard: server::AbuseGuard::default(),
            connection_slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            identity: RwLock::new(None),
            build_id: OnceLock::new(),
            download: app_package::DownloadWindow::default(),
        }
    }

    pub fn settings_snapshot(&self) -> RemoteAccessSettings {
        self.settings.read().unwrap_or_else(|error| error.into_inner()).clone()
    }

    /// The Locus Remote APK of the `with_remote_app` installer flavor.
    pub fn app_package(&self) -> Option<&'static app_package::AppPackage> {
        app_package::bundled(&self.app)
    }

    /// Whether the QR landing page offers the bundled APK at all (otherwise
    /// it only explains that the code is meant for the app).
    pub fn app_download_enabled(&self) -> bool {
        self.settings.read().unwrap_or_else(|error| error.into_inner()).app_download
            && self.app_package().is_some()
    }

    /// Whether the download page offers the APK right now: while a pairing
    /// window is open and for a grace period after it opened.
    pub fn app_download_open(&self) -> bool {
        self.app_download_enabled() && (self.pairing.is_open() || self.download.is_open(Instant::now()))
    }

    fn set_settings(&self, settings: RemoteAccessSettings) {
        *self.settings.write().unwrap_or_else(|error| error.into_inner()) = settings;
    }

    fn identity(&self) -> Option<Arc<tls::Identity>> {
        self.identity.read().unwrap_or_else(|error| error.into_inner()).clone()
    }

    fn load_identity(&self) -> Result<Arc<tls::Identity>, String> {
        if let Some(identity) = self.identity() {
            return Ok(identity);
        }
        let identity = Arc::new(tls::load_or_create()?);
        *self.identity.write().unwrap_or_else(|error| error.into_inner()) = Some(identity.clone());
        Ok(identity)
    }

    fn replace_identity(&self) -> Result<Arc<tls::Identity>, String> {
        let identity = Arc::new(tls::regenerate()?);
        *self.identity.write().unwrap_or_else(|error| error.into_inner()) = Some(identity.clone());
        Ok(identity)
    }

    pub fn build_id(&self) -> String {
        self.build_id.get_or_init(|| server::build_id(self)).clone()
    }

    pub fn notify_changed(&self) {
        let _ = self.app.emit(CHANGED_EVENT, ());
    }

    pub fn announce_pairing_request(&self, request_id: &str, device_name: &str, platform: &str, address: &str) {
        let request = PairingRequest {
            request_id: request_id.into(),
            device_name: device_name.into(),
            platform: platform.into(),
            address: address.into(),
            expires_at_ms: now_ms() + pairing::CONFIRM_TIMEOUT.as_millis() as i64,
        };
        let _ = self.app.emit(PAIRING_REQUEST_EVENT, request);
        self.notify_desktop(
            "Locus 远程访问",
            &format!("{device_name}（{address}）请求配对，请在 Locus 中确认"),
        );
    }

    pub fn pairing_request_settled(&self, request_id: &str) {
        let _ = self.app.emit(PAIRING_SETTLED_EVENT, request_id);
    }

    /// System notification. Built on the main thread because the command
    /// takes an owned AppHandle.
    pub fn notify_desktop(&self, title: &str, body: &str) {
        let handle = Arc::clone(&self.app);
        let (title, body) = (title.to_string(), body.to_string());
        let _ = self.app.run_on_main_thread(move || {
            if let Err(error) =
                crate::commands::send_system_notification(handle.as_ref().clone(), title, Some(body))
            {
                eprintln!("[RemoteAccess] notification failed: {error}");
            }
        });
    }
}

struct RunningListener {
    address: SocketAddr,
    kind: ListenerKind,
    fingerprint: Option<String>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerError {
    pub address: String,
    pub error: String,
}

#[derive(Default)]
struct Runtime {
    listeners: Vec<RunningListener>,
    errors: Vec<ListenerError>,
    monitor: Option<tokio::task::JoinHandle<()>>,
    last_error: Option<String>,
}

/// Tauri-managed handle.
#[derive(Default)]
pub struct RemoteAccessHandle {
    shared: OnceLock<Arc<Shared>>,
    runtime: tokio::sync::Mutex<Runtime>,
}

impl RemoteAccessHandle {
    fn shared(&self) -> Option<&Arc<Shared>> {
        self.shared.get()
    }

    fn shared_or_init(&self, app: AppHandle, settings: &RemoteAccessSettings) -> Arc<Shared> {
        self.shared
            .get_or_init(|| Arc::new(Shared::new(app, settings.clone())))
            .clone()
    }
}

fn handle(app: &AppHandle) -> Arc<RemoteAccessHandle> {
    app.state::<Arc<RemoteAccessHandle>>().inner().clone()
}

/// Applies the persisted settings. Called at startup and after every change.
pub async fn reconcile(app: AppHandle) {
    let settings = config::load_settings();
    let handle = handle(&app);
    if !settings.enabled && handle.shared().is_none() {
        return;
    }
    let shared = handle.shared_or_init(app, &settings);
    apply(&handle, &shared, settings).await;
}

/// Applies `settings` to an initialized service.
pub(crate) async fn apply(handle: &RemoteAccessHandle, shared: &Arc<Shared>, settings: RemoteAccessSettings) {
    shared.set_settings(settings.clone());
    let mut runtime = handle.runtime.lock().await;
    if !settings.enabled {
        shutdown(shared, &mut runtime);
        shared.notify_changed();
        return;
    }
    match shared.load_identity() {
        Ok(identity) => {
            runtime.last_error = None;
            apply_listeners(shared, &mut runtime, &settings, &identity).await;
            ensure_monitor(shared, &mut runtime);
        }
        Err(error) => {
            eprintln!("[RemoteAccess] cannot load TLS identity: {error}");
            shutdown(shared, &mut runtime);
            runtime.last_error = Some(error);
        }
    }
    shared.notify_changed();
}

fn shutdown(shared: &Shared, runtime: &mut Runtime) {
    if let Some(monitor) = runtime.monitor.take() {
        monitor.abort();
    }
    for listener in runtime.listeners.drain(..) {
        listener.task.abort();
    }
    runtime.errors.clear();
    shared.pairing.cancel();
    shared.download.close();
    shared.connections.close_all("disabled");
    shared.devices.flush(now_ms());
}

/// Converges the running listeners on the desired set. Connections that
/// arrived through a listener that goes away (or whose certificate changed)
/// are closed; everything else keeps running.
async fn apply_listeners(
    shared: &Arc<Shared>,
    runtime: &mut Runtime,
    settings: &RemoteAccessSettings,
    identity: &Arc<tls::Identity>,
) {
    let interfaces = net::discover();
    let desired = net::desired_binds(settings.port, settings.loopback, &settings.interfaces, &interfaces);
    let fingerprint_for = |kind: ListenerKind| kind.secure().then(|| identity.fingerprint.clone());

    let mut removed = Vec::new();
    runtime.listeners.retain(|listener| {
        let keep = desired.contains(&(listener.address, listener.kind))
            && listener.fingerprint == fingerprint_for(listener.kind);
        if !keep {
            listener.task.abort();
            removed.push(listener.address);
        }
        keep
    });
    if !removed.is_empty() {
        shared
            .connections
            .close_where("listener_closed", |connection| removed.contains(&connection.listener));
    }

    let tls_config = match identity.server_config() {
        Ok(config) => Some(tokio_rustls::TlsAcceptor::from(config)),
        Err(error) => {
            runtime.last_error = Some(error);
            None
        }
    };
    runtime.errors.clear();
    for (address, kind) in desired {
        if runtime
            .listeners
            .iter()
            .any(|listener| listener.address == address && listener.kind == kind)
        {
            continue;
        }
        let acceptor = if kind.secure() {
            match &tls_config {
                Some(acceptor) => Some(acceptor.clone()),
                None => continue,
            }
        } else {
            None
        };
        match TcpListener::bind(address).await {
            Ok(listener) => {
                let task = tokio::spawn(server::run_listener(shared.clone(), listener, kind, acceptor));
                runtime.listeners.push(RunningListener {
                    address,
                    kind,
                    fingerprint: fingerprint_for(kind),
                    task,
                });
            }
            Err(error) => {
                eprintln!("[RemoteAccess] cannot listen on {address}: {error}");
                runtime.errors.push(ListenerError {
                    address: address.to_string(),
                    error: error.to_string(),
                });
            }
        }
    }
}

/// Re-checks adapters periodically (DHCP renewals, VPN up/down) while enabled.
fn ensure_monitor(shared: &Arc<Shared>, runtime: &mut Runtime) {
    if runtime.monitor.as_ref().is_some_and(|task| !task.is_finished()) {
        return;
    }
    let shared = Arc::downgrade(shared);
    runtime.monitor = Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERFACE_POLL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let Some(shared) = shared.upgrade() else {
                return;
            };
            let settings = shared.settings_snapshot();
            if !settings.enabled {
                return;
            }
            let handle = handle(&shared.app);
            let Ok(mut runtime) = handle.runtime.try_lock() else {
                continue;
            };
            let Some(identity) = shared.identity() else {
                continue;
            };
            let before: Vec<_> = runtime.listeners.iter().map(|l| (l.address, l.kind)).collect();
            apply_listeners(&shared, &mut runtime, &settings, &identity).await;
            let after: Vec<_> = runtime.listeners.iter().map(|l| (l.address, l.kind)).collect();
            if before != after {
                shared.notify_changed();
            }
        }
    }));
}
