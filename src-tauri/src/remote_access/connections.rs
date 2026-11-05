//! Live WebSocket connections and the workspace event pump feeding them.
//!
//! The pump subscribes to the workspace event router only while at least one
//! phone is connected, so an enabled-but-idle remote service adds no work to
//! event publication.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager};
use tokio::sync::{mpsc, watch, Notify};
use tokio_tungstenite::tungstenite::Utf8Bytes;

use crate::workspace_service::event::{RoutedWorkspaceEvent, WindowEventInterests};
use crate::workspace_service::ProjectRegistry;

/// Queued event frames per connection before the client must resync.
pub const EVENT_QUEUE_FRAMES: usize = 2048;
pub const EVENT_QUEUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_INTEREST_ENTRIES: usize = 512;
const MAX_WATCH_PATH: usize = 1024;
const FILE_CHANGED_EVENT: &str = "workspace-file-changed";

/// A watched path, normalized like the file-change payloads it is compared to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Watch {
    checkout_id: String,
    path: String,
}

fn normalize_watch_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut path = path.as_str();
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    let path = path.trim_matches('/');
    if path == "." {
        String::new()
    } else {
        path.to_lowercase()
    }
}

/// Whether a change at `changed` can alter what a device shows for
/// `watched`: the path itself, an entry directly inside it, or an ancestor
/// (moved, deleted or rescanned). An empty path is the workspace root.
fn change_affects(changed: &str, watched: &str) -> bool {
    if changed.is_empty() || changed == watched {
        return true;
    }
    let ancestor = watched.len() > changed.len()
        && watched.starts_with(changed)
        && watched.as_bytes()[changed.len()] == b'/';
    let parent = changed.rsplit_once('/').map_or("", |(parent, _)| parent);
    ancestor || parent == watched
}

pub struct Connection {
    pub id: String,
    pub device_id: String,
    pub listener: SocketAddr,
    interests: Mutex<WindowEventInterests>,
    /// File-change events are high volume (builds, installs); a device only
    /// receives the ones for paths it shows.
    file_watches: Mutex<Vec<Watch>>,
    /// Desktop file watchers kept running for those paths: checkout id →
    /// lease token (see `file_watch`).
    pub(super) watcher_leases: Mutex<HashMap<String, String>>,
    events: mpsc::Sender<Utf8Bytes>,
    pending_bytes: AtomicUsize,
    overflowed: AtomicBool,
    overflow_notify: Notify,
    close: watch::Sender<Option<String>>,
}

impl Connection {
    pub fn new(
        device_id: String,
        listener: SocketAddr,
    ) -> (Arc<Self>, mpsc::Receiver<Utf8Bytes>, watch::Receiver<Option<String>>) {
        let (events, events_rx) = mpsc::channel(EVENT_QUEUE_FRAMES);
        let (close, close_rx) = watch::channel(None);
        let connection = Arc::new(Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            device_id,
            listener,
            interests: Mutex::new(WindowEventInterests::default()),
            file_watches: Mutex::new(Vec::new()),
            watcher_leases: Mutex::new(HashMap::new()),
            events,
            pending_bytes: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
            overflow_notify: Notify::new(),
            close,
        });
        (connection, events_rx, close_rx)
    }

    pub fn set_interests(&self, interests: WindowEventInterests) -> Result<(), String> {
        let size = interests.checkouts.len()
            + interests.sessions.len()
            + interests.stream_checkouts.len();
        if size > MAX_INTEREST_ENTRIES {
            return Err("Too many event interests".into());
        }
        *self.interests.lock().unwrap_or_else(|error| error.into_inner()) = interests;
        Ok(())
    }

    pub fn set_file_watches(&self, watches: Vec<locus_remote_protocol::FileWatch>) -> Result<(), String> {
        if watches.len() > locus_remote_protocol::MAX_FILE_WATCHES {
            return Err("Too many file watches".into());
        }
        if watches.iter().any(|watch| watch.path.len() > MAX_WATCH_PATH || watch.checkout_id.is_empty()) {
            return Err("Invalid file watch".into());
        }
        let watches = watches
            .into_iter()
            .map(|watch| Watch { checkout_id: watch.checkout_id, path: normalize_watch_path(&watch.path) })
            .collect();
        *self.file_watches.lock().unwrap_or_else(|error| error.into_inner()) = watches;
        Ok(())
    }

    /// Checkouts with at least one watched path.
    pub fn watched_checkouts(&self) -> HashSet<String> {
        self.file_watches
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|watch| watch.checkout_id.clone())
            .collect()
    }

    /// Owner name of this connection's file watcher leases.
    pub fn watcher_owner(&self) -> String {
        format!("remote:{}", self.id)
    }

    fn wants(&self, event: &RoutedWorkspaceEvent) -> bool {
        if event.event_name == FILE_CHANGED_EVENT {
            return self.watches_change(event);
        }
        self.interests
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .matches_remote(event)
    }

    fn watches_change(&self, event: &RoutedWorkspaceEvent) -> bool {
        let watches = self.file_watches.lock().unwrap_or_else(|error| error.into_inner());
        if watches.is_empty() {
            return false;
        }
        let Some(path) = event.envelope.payload.get("path").and_then(|value| value.as_str()) else {
            return false;
        };
        let changed = normalize_watch_path(path);
        let checkout = event.envelope.checkout_id.as_str();
        watches
            .iter()
            .any(|watch| watch.checkout_id == checkout && change_affects(&changed, &watch.path))
    }

    /// Never blocks the pump: a full queue marks the connection for resync.
    fn push_event(&self, frame: Utf8Bytes) {
        if self.overflowed.load(Ordering::Acquire) {
            return;
        }
        let size = frame.as_str().len();
        let pending = self.pending_bytes.fetch_add(size, Ordering::AcqRel) + size;
        if pending > EVENT_QUEUE_BYTES || self.events.try_send(frame).is_err() {
            self.pending_bytes.fetch_sub(size, Ordering::AcqRel);
            self.overflowed.store(true, Ordering::Release);
            self.overflow_notify.notify_one();
        }
    }

    /// Called by the writer for every dequeued frame.
    pub fn event_sent(&self, frame: &Utf8Bytes) {
        self.pending_bytes.fetch_sub(frame.as_str().len(), Ordering::AcqRel);
    }

    pub async fn overflowed(&self) {
        self.overflow_notify.notified().await
    }

    /// True once per overflow; the writer then drains and sends `Resync`.
    pub fn take_overflow(&self) -> bool {
        self.overflowed.swap(false, Ordering::AcqRel)
    }

    pub fn request_close(&self, reason: &str) {
        let _ = self.close.send(Some(reason.to_string()));
    }
}

#[derive(Default)]
pub struct ConnectionRegistry {
    connections: Mutex<HashMap<String, Arc<Connection>>>,
    pump: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ConnectionRegistry {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Connection>>> {
        self.connections.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub fn snapshot(&self) -> Vec<Arc<Connection>> {
        self.map().values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        self.map().len()
    }

    pub fn connected_devices(&self) -> HashSet<String> {
        self.map().values().map(|connection| connection.device_id.clone()).collect()
    }

    pub fn insert(self: &Arc<Self>, app: &AppHandle, connection: Arc<Connection>) {
        self.map().insert(connection.id.clone(), connection);
        self.ensure_pump(app);
    }

    pub fn remove(&self, id: &str) {
        self.map().remove(id);
        let mut pump = self.pump.lock().unwrap_or_else(|error| error.into_inner());
        if self.count() == 0 {
            if let Some(task) = pump.take() {
                task.abort();
            }
        }
    }

    pub fn close_where(&self, reason: &str, predicate: impl Fn(&Connection) -> bool) -> usize {
        let targets: Vec<_> = self
            .snapshot()
            .into_iter()
            .filter(|connection| predicate(connection))
            .collect();
        for connection in &targets {
            connection.request_close(reason);
        }
        targets.len()
    }

    pub fn close_all(&self, reason: &str) {
        self.close_where(reason, |_| true);
    }

    fn ensure_pump(self: &Arc<Self>, app: &AppHandle) {
        let mut pump = self.pump.lock().unwrap_or_else(|error| error.into_inner());
        if pump.as_ref().is_some_and(|task| !task.is_finished()) || self.count() == 0 {
            return;
        }
        let Some(projects) = app.try_state::<Arc<ProjectRegistry>>() else {
            return;
        };
        let mut events = projects.event_router().subscribe();
        let registry = Arc::downgrade(self);
        *pump = Some(tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let Some(registry) = registry.upgrade() else {
                    break;
                };
                deliver(&registry.snapshot(), &event);
            }
        }));
    }
}

/// Wire shape of [`locus_remote_protocol::ServerFrame::Event`], serialized
/// straight from the routed event without an intermediate `Value`.
#[derive(serde::Serialize)]
struct EventFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    payload: &'a RoutedWorkspaceEvent,
}

fn deliver(connections: &[Arc<Connection>], event: &RoutedWorkspaceEvent) {
    // Serialized lazily and at most once; `Utf8Bytes` clones share the buffer.
    let mut frame: Option<Utf8Bytes> = None;
    for connection in connections {
        if !connection.wants(event) {
            continue;
        }
        let frame = match &frame {
            Some(frame) => frame.clone(),
            None => {
                let Ok(text) = serde_json::to_string(&EventFrame { kind: "event", payload: event })
                else {
                    return;
                };
                frame.insert(Utf8Bytes::from(text)).clone()
            }
        };
        connection.push_event(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_service::event::WorkspaceEventEnvelope;
    use crate::workspace_service::{CheckoutId, ProjectId};
    use serde_json::json;

    fn addr() -> SocketAddr {
        "127.0.0.1:1".parse().unwrap()
    }

    fn event(session: &str) -> RoutedWorkspaceEvent {
        RoutedWorkspaceEvent {
            event_name: "stream-event".into(),
            stream_revision: 1,
            envelope: WorkspaceEventEnvelope {
                project_id: ProjectId::new("p").unwrap(),
                checkout_id: CheckoutId::new("c").unwrap(),
                workspace_generation: 1,
                materialization_epoch: None,
                service_instance_id: None,
                service_generation: None,
                payload: json!({"type": "textDelta", "sessionId": session, "text": "x"}),
            },
        }
    }

    fn interested(session: &str) -> WindowEventInterests {
        WindowEventInterests {
            sessions: [session.to_string()].into_iter().collect(),
            ..Default::default()
        }
    }

    #[test]
    fn delivers_only_to_interested_connections() {
        let (a, mut a_rx, _) = Connection::new("d1".into(), addr());
        let (b, mut b_rx, _) = Connection::new("d2".into(), addr());
        a.set_interests(interested("s1")).unwrap();
        b.set_interests(interested("s2")).unwrap();
        deliver(&[a.clone(), b.clone()], &event("s1"));
        let frame = a_rx.try_recv().unwrap();
        let parsed: locus_remote_protocol::ServerFrame = serde_json::from_str(frame.as_str()).unwrap();
        let locus_remote_protocol::ServerFrame::Event { payload } = parsed else {
            panic!("expected an event frame");
        };
        assert_eq!(payload["eventName"], "stream-event");
        assert_eq!(payload["payload"]["type"], "textDelta");
        a.event_sent(&frame);
        assert!(b_rx.try_recv().is_err());
    }

    #[test]
    fn full_queue_marks_overflow_once_until_taken() {
        let (connection, mut rx, _) = Connection::new("d".into(), addr());
        connection.set_interests(interested("s")).unwrap();
        for _ in 0..EVENT_QUEUE_FRAMES + 3 {
            deliver(&[connection.clone()], &event("s"));
        }
        assert!(connection.take_overflow());
        assert!(!connection.take_overflow());
        let mut drained = 0;
        while let Ok(frame) = rx.try_recv() {
            connection.event_sent(&frame);
            drained += 1;
        }
        assert_eq!(drained, EVENT_QUEUE_FRAMES);
        assert_eq!(connection.pending_bytes.load(Ordering::Acquire), 0);
    }

    fn file_change(path: &str) -> RoutedWorkspaceEvent {
        let mut routed = event("s");
        routed.event_name = FILE_CHANGED_EVENT.into();
        routed.envelope.payload = json!({"seq": 1, "generation": 1, "path": path, "kind": "upsert", "source": "os_watcher"});
        routed
    }

    fn watch(path: &str) -> locus_remote_protocol::FileWatch {
        locus_remote_protocol::FileWatch { checkout_id: "c".into(), path: path.into() }
    }

    #[test]
    fn file_changes_reach_only_connections_watching_them() {
        let (connection, _, _) = Connection::new("d".into(), addr());
        // A checkout-wide interest alone never forwards file changes.
        connection
            .set_interests(WindowEventInterests {
                checkouts: vec![serde_json::from_value(json!({"checkoutId": "c"})).unwrap()],
                ..Default::default()
            })
            .unwrap();
        assert!(!connection.wants(&file_change("README.md")));
        connection.set_file_watches(vec![watch("docs\\Guide.md"), watch("")]).unwrap();
        assert!(connection.wants(&file_change("docs/guide.md")));
        assert!(connection.wants(&file_change("README.md")), "entry of the watched root");
        assert!(connection.wants(&file_change("docs")), "ancestor of a watched file");
        assert!(connection.wants(&file_change(".")), "whole checkout");
        assert!(!connection.wants(&file_change("docs/other.md")));
        assert!(!connection.wants(&file_change("target/debug/build.log")));
        let mut elsewhere = file_change("docs/guide.md");
        elsewhere.envelope.checkout_id = CheckoutId::new("other").unwrap();
        assert!(!connection.wants(&elsewhere));
        connection.set_file_watches(Vec::new()).unwrap();
        assert!(!connection.wants(&file_change("docs/guide.md")));
        let many = (0..=locus_remote_protocol::MAX_FILE_WATCHES).map(|i| watch(&i.to_string())).collect();
        assert!(connection.set_file_watches(many).is_err());
    }

    #[test]
    fn interest_size_is_capped() {
        let (connection, _, _) = Connection::new("d".into(), addr());
        let many = WindowEventInterests {
            sessions: (0..=MAX_INTEREST_ENTRIES).map(|i| i.to_string()).collect(),
            ..Default::default()
        };
        assert!(connection.set_interests(many).is_err());
    }

    #[test]
    fn close_requests_reach_the_watch_channel() {
        let registry = ConnectionRegistry::default();
        let (connection, _, close_rx) = Connection::new("d".into(), addr());
        registry.map().insert(connection.id.clone(), connection.clone());
        assert_eq!(registry.close_where("revoked", |c| c.device_id == "d"), 1);
        assert_eq!(close_rx.borrow().as_deref(), Some("revoked"));
        registry.remove(&connection.id);
        assert_eq!(registry.count(), 0);
    }
}
