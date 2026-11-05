use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Started { connection: usize, session: String, finish: oneshot::Sender<Result<Value, String>> }
#[derive(Default)]
struct Metrics { opened: AtomicUsize, dropped: AtomicUsize, fail_start: AtomicBool, idle_dead: AtomicBool }
struct FakeConnection { id: usize, metrics: Arc<Metrics>, calls: mpsc::UnboundedSender<Started> }
impl Drop for FakeConnection {
    fn drop(&mut self) { self.metrics.dropped.fetch_add(1, Ordering::SeqCst); }
}
impl ArchiveConnection for FakeConnection {
    fn idle_healthy(&self) -> bool { !self.metrics.idle_dead.load(Ordering::SeqCst) }
    fn download(&self, request: Value) -> BoxFuture<'_, Result<Value, String>> {
        Box::pin(async move {
            let (finish, receive) = oneshot::channel();
            self.calls.send(Started { connection: self.id, session: request["sessionKey"].as_str().unwrap().into(), finish }).unwrap();
            receive.await.unwrap_or_else(|_| Err(errors::disconnected("fixture connection closed")))
        })
    }
}
struct Fixture { handle: Handle, metrics: Arc<Metrics>, calls: mpsc::UnboundedReceiver<Started> }
impl Fixture {
    fn new(idle_for: Duration) -> Self {
        let metrics = Arc::new(Metrics::default());
        let (calls, receive) = mpsc::unbounded_channel();
        let state = metrics.clone();
        let connector: Connector = Arc::new(move || {
            let metrics = state.clone();
            let calls = calls.clone();
            Box::pin(async move {
                let id = metrics.opened.fetch_add(1, Ordering::SeqCst) + 1;
                if metrics.fail_start.load(Ordering::SeqCst) { return Err("fixture startup failed".into()); }
                Ok(Box::new(FakeConnection { id, metrics, calls }) as Connection)
            })
        });
        Self { handle: Handle::spawn(connector, idle_for), metrics, calls: receive }
    }
    fn request(&self, session: &str, priority: Priority) -> tokio::task::JoinHandle<Result<Value, String>> {
        let handle = self.handle.clone();
        let value = json!({"sessionKey": session});
        tokio::spawn(async move { handle.download(value, priority, Duration::from_secs(60)).await })
    }
    fn opened(&self) -> usize { self.metrics.opened.load(Ordering::SeqCst) }
    fn dropped(&self) -> usize { self.metrics.dropped.load(Ordering::SeqCst) }
}
impl Drop for Fixture { fn drop(&mut self) { self.handle.close(); } }
async fn settle() { for _ in 0..12 { tokio::task::yield_now().await; } }
fn complete(call: Started) { call.finish.send(Ok(json!({"ok":true}))).unwrap(); }

#[tokio::test(start_paused = true)]
async fn one_warm_socket_is_reused_across_conversations_without_a_second_claude() {
    let mut f = Fixture::new(Duration::from_secs(60));
    settle().await;
    assert_eq!(f.opened(), 1);
    let first = f.request("session-a", Priority::Background);
    let a = f.calls.recv().await.unwrap();
    assert_eq!(a.session, "session-a");
    let connection = a.connection;
    complete(a);
    first.await.unwrap().unwrap();
    let second = f.request("session-b", Priority::Foreground);
    let b = f.calls.recv().await.unwrap();
    assert_eq!(b.session, "session-b");
    assert_eq!(b.connection, connection);
    complete(b);
    second.await.unwrap().unwrap();
    settle().await;
    assert_eq!(f.opened(), 1);
    assert_eq!(f.dropped(), 0);
}

#[tokio::test(start_paused = true)]
async fn background_is_serial_foreground_overtakes_and_total_sockets_are_bounded() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let a = f.request("background-a", Priority::Background);
    let ca = f.calls.recv().await.unwrap();
    let b = f.request("background-b", Priority::Background);
    settle().await;
    assert!(f.calls.try_recv().is_err());
    assert_eq!(f.opened(), 1);
    let c = f.request("foreground-c", Priority::Foreground);
    let cc = f.calls.recv().await.unwrap();
    assert_eq!(cc.session, "foreground-c");
    assert_ne!(cc.connection, ca.connection);
    let d = f.request("foreground-d", Priority::Foreground);
    settle().await;
    assert_eq!(f.opened(), 2);
    assert!(f.calls.try_recv().is_err());
    complete(cc);
    c.await.unwrap().unwrap();
    let cd = f.calls.recv().await.unwrap();
    assert_eq!(cd.session, "foreground-d");
    complete(ca);
    a.await.unwrap().unwrap();
    let cb = f.calls.recv().await.unwrap();
    assert_eq!(cb.session, "background-b");
    complete(cb);
    complete(cd);
    b.await.unwrap().unwrap(); d.await.unwrap().unwrap();
    settle().await;
    assert_eq!(f.opened(), 2);
    assert_eq!(f.dropped(), 1, "extra idle sockets converge to one");
}

#[tokio::test(start_paused = true)]
async fn long_bulk_transfer_prewarms_a_spare_and_does_not_use_the_chat_heartbeat_lease() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let background = f.request("large-images", Priority::Background);
    let bulk = f.calls.recv().await.unwrap();
    tokio::time::advance(LONG_TRANSFER).await;
    settle().await;
    assert_eq!(f.opened(), 2);
    tokio::time::advance(Duration::from_secs(50)).await;
    settle().await;
    assert!(!background.is_finished(), "a live bulk transfer can outlast the 30/45s chat leases");
    let foreground = f.request("another-session", Priority::Foreground);
    let small = f.calls.recv().await.unwrap();
    assert_ne!(small.connection, bulk.connection);
    complete(small);
    foreground.await.unwrap().unwrap();
    complete(bulk);
    background.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn cancelling_an_active_transfer_destroys_its_socket_before_reuse() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let cancelled = f.request("cancelled", Priority::Background);
    let old = f.calls.recv().await.unwrap();
    cancelled.abort();
    let _ = cancelled.await;
    settle().await;
    assert_eq!(f.dropped(), 1);
    assert!(old.finish.send(Ok(json!({"late":true}))).is_err());
    let next = f.request("next", Priority::Background);
    let new = f.calls.recv().await.unwrap();
    assert_ne!(new.connection, old.connection);
    complete(new);
    next.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn queued_cancellation_never_sends_input_and_failures_do_not_reuse_a_dirty_socket() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let first = f.request("first", Priority::Background);
    let call = f.calls.recv().await.unwrap();
    let cancelled = f.request("never-send", Priority::Background);
    settle().await;
    cancelled.abort(); let _ = cancelled.await;
    call.finish.send(Err("fixture invalid archive".into())).unwrap();
    assert_eq!(first.await.unwrap().unwrap_err(), "fixture invalid archive");
    settle().await;
    assert_eq!(f.dropped(), 1);
    assert!(f.calls.try_recv().is_err());
    let next = f.request("fresh", Priority::Foreground);
    let call = f.calls.recv().await.unwrap();
    assert_eq!(call.session, "fresh");
    complete(call);
    next.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn demand_extends_idle_retention_but_unused_pools_end_without_polling_or_reconnecting() {
    let f = Fixture::new(Duration::from_secs(60));
    settle().await;
    tokio::time::advance(Duration::from_secs(50)).await;
    f.handle.touch(Duration::from_secs(60)); settle().await;
    tokio::time::advance(Duration::from_secs(50)).await; settle().await;
    assert!(!f.handle.closed());
    assert_eq!(f.opened(), 1);
    tokio::time::advance(Duration::from_secs(11)).await; settle().await;
    assert!(f.handle.closed());
    assert_eq!(f.dropped(), 1);
    tokio::time::advance(Duration::from_secs(600)).await; settle().await;
    assert_eq!(f.opened(), 1);
}

#[tokio::test(start_paused = true)]
async fn shutdown_releases_active_and_queued_callers_and_closes_all_connections() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let active = f.request("active", Priority::Background);
    let _call = f.calls.recv().await.unwrap();
    let queued = f.request("queued", Priority::Background);
    settle().await;
    f.handle.close();
    assert_eq!(active.await.unwrap().unwrap_err(), CLOSED);
    assert_eq!(queued.await.unwrap().unwrap_err(), CLOSED);
    settle().await;
    assert_eq!(f.opened(), f.dropped());
}

#[tokio::test(start_paused = true)]
async fn failed_prewarms_back_off_and_waiters_receive_the_startup_error() {
    let f = Fixture::new(Duration::from_secs(300));
    f.metrics.fail_start.store(true, Ordering::SeqCst);
    settle().await;
    assert_eq!(f.opened(), 1);
    for _ in 0..10 { f.handle.touch(Duration::from_secs(300)); settle().await; }
    assert_eq!(f.opened(), 1);
    let waiting = f.request("explicit", Priority::Foreground);
    assert_eq!(waiting.await.unwrap().unwrap_err(), "fixture startup failed");
    settle().await;
    assert_eq!(f.opened(), 2);
    tokio::time::advance(Duration::from_secs(30)).await; settle().await;
    assert_eq!(f.opened(), 2);
}

#[tokio::test(start_paused = true)]
async fn failed_expansion_returns_an_error_without_a_reconnect_loop_or_stopping_the_active_backup() {
    let mut f = Fixture::new(Duration::from_secs(60));
    let background = f.request("active", Priority::Background);
    let call = f.calls.recv().await.unwrap();
    f.metrics.fail_start.store(true, Ordering::SeqCst);
    let foreground = f.request("waiting", Priority::Foreground);
    assert_eq!(foreground.await.unwrap().unwrap_err(), "fixture startup failed");
    settle().await;
    assert_eq!(f.opened(), 2);
    assert!(!background.is_finished());
    tokio::time::advance(Duration::from_secs(20)).await; settle().await;
    assert_eq!(f.opened(), 2);
    complete(call);
    background.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn pool_snapshot_is_separate_from_chat_and_reads_do_not_launch_new_sockets() {
    let mut f = Fixture::new(Duration::from_secs(60));
    settle().await;
    assert_eq!(f.handle.status.borrow().ready, 1);
    let active = f.request("active", Priority::Background);
    let call = f.calls.recv().await.unwrap();
    let queued = f.request("queued", Priority::Background);
    settle().await;
    let status = f.handle.status.borrow().clone();
    assert_eq!((status.busy, status.queued, status.ready), (1, 1, 0));
    for _ in 0..100 { let _ = f.handle.status.borrow().clone(); }
    assert_eq!(f.opened(), 1);
    queued.abort(); let _ = queued.await;
    complete(call);
    active.await.unwrap().unwrap();
    settle().await;
    assert_eq!(f.handle.status.borrow().ready, 1);
}

#[tokio::test(start_paused = true)]
async fn a_helper_that_dies_immediately_after_hello_does_not_create_an_unbounded_connect_loop() {
    let f = Fixture::new(Duration::from_secs(300));
    f.metrics.idle_dead.store(true, Ordering::SeqCst);
    settle().await;
    assert_eq!(f.opened(), 1);
    assert_eq!(f.dropped(), 1);
    let request = f.request("waiting", Priority::Foreground);
    assert!(errors::is_disconnected(&request.await.unwrap().unwrap_err()));
    settle().await;
    assert_eq!(f.opened(), 2);
    assert_eq!(f.dropped(), 2);
    for _ in 0..20 { f.handle.touch(Duration::from_secs(300)); settle().await; }
    assert_eq!(f.opened(), 2);
}

#[test]
fn pool_keys_include_authentication_and_port_not_only_hostname() {
    let config = SshConfig { target: "unused.invalid".into(), port: None, identity_file: None, known_hosts_file: None, cli_path: None };
    for changed in [SshConfig { identity_file: Some("other-key".into()), ..config.clone() },
        SshConfig { port: Some(2222), ..config.clone() },
        SshConfig { known_hosts_file: Some("other-hosts".into()), ..config.clone() }] {
        assert_ne!(config.fingerprint(), changed.fingerprint());
    }
}
