//! Download-only SSH connections, independent of every interactive Claude TUI.
//! One warm connection per SSH identity, at most two sockets, one background
//! transfer at a time. An actor exists only while there is demand or retained
//! state; no global polling task, spare Claude process, or provider request.
use super::{errors, SshConfig, Transport};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{sync::{mpsc, oneshot, watch}, task::JoinSet, time::Instant};

const CAPACITY: usize = 2;
const QUEUE_LIMIT: usize = 64;
const LONG_TRANSFER: Duration = Duration::from_secs(5);
const RETRY_BACKOFF: Duration = Duration::from_secs(60);
const CLOSED: &str = "Claude archive connection pool was closed";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority { Foreground, Background }

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ready: usize,
    pub starting: usize,
    pub busy: usize,
    pub queued: usize,
    pub last_error: Option<String>,
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type Connection = Box<dyn ArchiveConnection>;
type Connector = Arc<dyn Fn() -> BoxFuture<'static, Result<Connection, String>> + Send + Sync>;

// The small transport boundary permits offline, deterministic scheduling tests.
trait ArchiveConnection: Send + Sync {
    fn idle_healthy(&self) -> bool;
    fn download(&self, request: Value) -> BoxFuture<'_, Result<Value, String>>;
}
impl ArchiveConnection for Transport {
    fn idle_healthy(&self) -> bool { self.health_failure().is_none() }
    fn download(&self, request: Value) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(self.archive_request(request))
    }
}

struct Request {
    value: Value,
    priority: Priority,
    reply: oneshot::Sender<Result<Value, String>>,
}
enum Command { Touch(Duration), Download(Request, Duration) }
#[derive(Clone)]
struct Handle {
    send: mpsc::Sender<Command>,
    stop: watch::Sender<bool>,
    status: watch::Receiver<Snapshot>,
}
impl Handle {
    fn spawn(connector: Connector, idle_for: Duration) -> Self {
        let (send, receive) = mpsc::channel(32);
        let (stop, shutdown) = watch::channel(false);
        let (publish, status) = watch::channel(Snapshot::default());
        tokio::spawn(Actor::new(connector, idle_for, publish).run(receive, shutdown));
        Self { send, stop, status }
    }
    fn touch(&self, idle_for: Duration) { let _ = self.send.try_send(Command::Touch(idle_for)); }
    fn close(&self) { self.stop.send_replace(true); }
    fn closed(&self) -> bool { self.send.is_closed() || *self.stop.borrow() }
    async fn download(&self, value: Value, priority: Priority, idle_for: Duration) -> Result<Value, String> {
        let (reply, receive) = oneshot::channel();
        self.send.send(Command::Download(Request { value, priority, reply }, idle_for)).await.map_err(|_| CLOSED)?;
        receive.await.map_err(|_| CLOSED)?
    }
}

fn registry() -> &'static Mutex<HashMap<String, Handle>> {
    static POOLS: OnceLock<Mutex<HashMap<String, Handle>>> = OnceLock::new();
    POOLS.get_or_init(Default::default)
}
fn pool(config: &SshConfig, idle_for: Duration) -> Handle {
    let mut pools = registry().lock().unwrap_or_else(|e| e.into_inner());
    pools.retain(|_, handle| !handle.closed());
    pools.entry(config.fingerprint()).or_insert_with(|| {
        let config = config.clone();
        let connector: Connector = Arc::new(move || {
            let config = config.clone();
            Box::pin(async move {
                // This mode never installs anything, unlocks credentials or starts
                // Claude. In particular it never borrows a conversation's socket.
                let (transport, info) = Transport::start(&config, json!({"mode":"archive_read"}), None,
                    Arc::new(|| Ok(None))).await?;
                if !info.python_installed { return Err("Remote Python 3 is unavailable for archive downloads".into()); }
                Ok(Box::new(transport) as Connection)
            })
        });
        Handle::spawn(connector, idle_for)
    }).clone()
}

/// Called alongside existing CLI demand, not from a new timer or UI poll.
pub fn prewarm(config: &SshConfig, idle_for: Duration) {
    if config.validate().is_err() || tokio::runtime::Handle::try_current().is_err() { return; }
    pool(config, idle_for).touch(idle_for);
}

pub async fn download(config: &SshConfig, request: Value, priority: Priority, idle_for: Duration) -> Result<Value, String> {
    config.validate()?;
    for field in ["sessionKey", "nativeId"] {
        let id = request[field].as_str().ok_or("Archive download requires a session and native identity")?;
        uuid::Uuid::parse_str(id).map_err(|_| "Invalid archive download identity")?;
    }
    // Only reads can be retried, once, after losing an idle/busy transport.
    // Explicit shutdown, remote archive validation and queue errors do not retry.
    let handle = pool(config, idle_for);
    let first = handle.download(request.clone(), priority, idle_for).await;
    match first {
        Err(error) if error == CLOSED && !*handle.stop.borrow() =>
            pool(config, idle_for).download(request, priority, idle_for).await,
        Err(error) if errors::is_disconnected(&error) && !handle.closed() =>
            handle.download(request, priority, idle_for).await,
        result => result,
    }
}

/// Local observation only: opening the status panel never creates a pool.
pub fn snapshot(config: &SshConfig) -> Option<Snapshot> {
    registry().lock().unwrap_or_else(|e| e.into_inner()).get(&config.fingerprint())
        .filter(|handle| !handle.closed()).map(|handle| handle.status.borrow().clone())
}

/// Configuration changes cannot reuse sockets authenticated under another key.
pub fn retire_except(keep: Option<&SshConfig>) {
    let keep = keep.map(SshConfig::fingerprint);
    registry().lock().unwrap_or_else(|e| e.into_inner()).retain(|key, handle| {
        if keep.as_ref() == Some(key) { true } else { handle.close(); false }
    });
}
pub fn shutdown() { retire_except(None); }

struct Active { priority: Priority, started: Instant }
enum Event {
    Connected(Result<Connection, String>),
    Finished(u64, Option<Connection>, Option<String>),
}
struct Actor {
    connector: Connector,
    idle_for: Duration,
    expires: Instant,
    idle: Vec<Connection>,
    foreground: VecDeque<Request>,
    background: VecDeque<Request>,
    active: HashMap<u64, Active>,
    starting: usize,
    next_id: u64,
    warm_retry: Option<Instant>,
    last_error: Option<String>,
    publish: watch::Sender<Snapshot>,
    jobs: JoinSet<Event>,
}
impl Actor {
    fn new(connector: Connector, idle_for: Duration, publish: watch::Sender<Snapshot>) -> Self {
        Self { connector, idle_for, expires: Instant::now() + idle_for, idle: Vec::new(),
            foreground: VecDeque::new(), background: VecDeque::new(), active: HashMap::new(),
            starting: 0, next_id: 0, warm_retry: None, last_error: None, publish, jobs: JoinSet::new() }
    }
    fn touch(&mut self, idle_for: Duration) {
        self.idle_for = idle_for;
        self.expires = Instant::now() + idle_for;
    }
    fn background_busy(&self) -> bool { self.active.values().any(|job| job.priority == Priority::Background) }
    fn slots(&self) -> usize { self.idle.len() + self.active.len() + self.starting }
    fn connect(&mut self) {
        self.starting += 1;
        let connector = self.connector.clone();
        self.jobs.spawn(async move { Event::Connected(connector().await) });
    }
    fn start(&mut self, connection: Connection, mut request: Request) {
        self.next_id += 1;
        let id = self.next_id;
        self.active.insert(id, Active { priority: request.priority, started: Instant::now() });
        self.jobs.spawn(async move {
            let result = tokio::select! {
                biased;
                _ = request.reply.closed() => None,
                result = connection.download(request.value) => Some(result),
            };
            // Cancellation leaves the helper writing the old stream; dropping
            // its connection is mandatory. Never put unread bytes back in a pool.
            let error = result.as_ref().and_then(|result| result.as_ref().err()).map(|error| error.chars().take(1200).collect());
            let reusable = if let Some(result) = result {
                let ok = result.is_ok();
                request.reply.send(result).is_ok() && ok && connection.idle_healthy()
            } else { false };
            Event::Finished(id, reusable.then_some(connection), error)
        });
    }
    fn queue(&mut self, request: Request) {
        if self.foreground.len() + self.background.len() >= QUEUE_LIMIT {
            let _ = request.reply.send(Err("Claude archive queue is full; the previous backup was retained".into()));
        } else if request.priority == Priority::Foreground { self.foreground.push_back(request); }
        else { self.background.push_back(request); }
    }
    fn fail_waiters(&mut self, error: &str) {
        for request in self.foreground.drain(..).chain(self.background.drain(..)) {
            let _ = request.reply.send(Err(error.to_string()));
        }
    }
    fn schedule(&mut self) {
        self.foreground.retain(|r| !r.reply.is_closed());
        self.background.retain(|r| !r.reply.is_closed());
        let before = self.idle.len();
        self.idle.retain(|connection| connection.idle_healthy());
        if self.idle.len() != before {
            self.warm_retry = Some(Instant::now() + RETRY_BACKOFF);
            if self.idle.is_empty() && self.starting == 0 {
                let error = errors::disconnected("Idle archive helper is no longer responsive");
                self.last_error = Some(error.clone());
                self.fail_waiters(&error);
            }
        }
        while !self.idle.is_empty() {
            let next = self.foreground.pop_front().or_else(|| {
                if self.background_busy() { None } else { self.background.pop_front() }
            });
            let Some(request) = next else { break; };
            let connection = self.idle.pop().unwrap();
            self.start(connection, request);
        }
        // Idle returns converge to one socket. Do not create a new spare for
        // each short backup; expand only for foreground demand or a long job.
        self.idle.truncate(1);
        let waiting = self.foreground.len() + usize::from(!self.background_busy() && !self.background.is_empty());
        while self.slots() < CAPACITY && self.starting + self.idle.len() < waiting.min(CAPACITY) {
            self.connect();
        }
        if self.warm_retry.is_none_or(|retry| Instant::now() >= retry) && self.slots() == 0 {
            self.connect();
        } else if self.long_deadline().is_some_and(|deadline| Instant::now() >= deadline) {
            self.connect();
        }
    }
    fn long_deadline(&self) -> Option<Instant> {
        if self.slots() != 1 || self.active.len() != 1
            || self.warm_retry.is_some_and(|retry| Instant::now() < retry) { return None; }
        self.active.values().next().map(|job| job.started + LONG_TRANSFER)
    }
    fn event(&mut self, event: Event) {
        match event {
            Event::Connected(result) => {
                self.starting -= 1;
                match result {
                    Ok(connection) => { self.idle.push(connection); self.warm_retry = None; }
                    Err(error) => {
                        self.warm_retry = Some(Instant::now() + RETRY_BACKOFF);
                        self.last_error = Some(error.chars().take(1200).collect());
                        // A failed expansion must not immediately reopen another
                        // socket for the same waiters in a tight retry loop.
                        if self.starting == 0 && self.idle.is_empty() {
                            self.fail_waiters(&error);
                        }
                    }
                }
            }
            Event::Finished(id, connection, error) => {
                self.active.remove(&id);
                if let Some(error) = error { self.last_error = Some(error); }
                if let Some(connection) = connection { self.idle.push(connection); self.last_error = None; }
                self.expires = Instant::now() + self.idle_for;
            }
        }
    }
    async fn run(mut self, mut commands: mpsc::Receiver<Command>, mut shutdown: watch::Receiver<bool>) {
        loop {
            if *shutdown.borrow() { break; }
            // Retain state while work is queued/in flight, but do not run an
            // idle-expiry timer repeatedly against an active transfer.
            let can_expire = self.jobs.is_empty() && self.foreground.is_empty() && self.background.is_empty();
            if can_expire && Instant::now() >= self.expires { break; }
            self.schedule();
            let snapshot = Snapshot { ready: self.idle.len(), starting: self.starting, busy: self.active.len(),
                queued: self.foreground.len() + self.background.len(), last_error: self.last_error.clone() };
            self.publish.send_if_modified(|status| {
                if *status == snapshot { false } else { *status = snapshot; true }
            });
            let can_expire = self.jobs.is_empty() && self.foreground.is_empty() && self.background.is_empty();
            let long = self.long_deadline();
            tokio::select! {
                biased;
                _ = shutdown.changed() => break,
                command = commands.recv() => match command {
                    Some(Command::Touch(idle_for)) => self.touch(idle_for),
                    Some(Command::Download(request, idle_for)) => { self.touch(idle_for); self.queue(request); }
                    None => break,
                },
                event = self.jobs.join_next(), if !self.jobs.is_empty() => match event {
                    Some(Ok(event)) => self.event(event),
                    _ => break, // Fail closed after a task panic, never leak slot accounting.
                },
                _ = tokio::time::sleep_until(self.expires), if can_expire => {},
                _ = tokio::time::sleep_until(long.unwrap_or(self.expires)), if long.is_some() => {},
            }
        }
        // JoinSet aborts startup/transfers; their owned transports kill SSH and
        // its helper. Dropped replies release all callers, including queue waits.
        self.jobs.abort_all();
    }
}

#[cfg(test)]
#[path = "archive_pool_tests.rs"]
mod tests;
