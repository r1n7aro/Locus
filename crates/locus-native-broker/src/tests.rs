use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::outbox::{Outbox, EVENT_FRAME_LIMIT, REPLY_BYTE_LIMIT};
use crate::broker::{
    handle_line, now_ms, Broker, InflightRequest, RequestPolicy, EXECUTION_PROGRESS_LIMIT,
    EXECUTION_PROGRESS_MAX_BYTES, MAX_PENDING_REQUESTS, MAX_REQUEST_BYTES, NO_CLIENT,
    REQUEST_DEADLINE_MS, STATUS_EVENT_BUFFER_LIMIT,
};
use crate::host::{ExecutionCommands, HostConfig, HostReply, HostRequest};
use locus_native_protocol::envelope::REQUEST_ACCEPTED_EVENT;
use locus_native_protocol::managed_state::{READY, RELOADING};
use locus_native_protocol::state_plane as layout;

const UNITY_EXECUTION: ExecutionCommands = ExecutionCommands {
    execute: &["execute_code", "execute_loaded"],
    wait: "execute_code_wait",
    cancel: "cancel_execute_code",
    progress: "execute_code_progress",
};

fn config(name: &str) -> HostConfig {
    HostConfig {
        host_kind: "unity",
        instance_key: format!("test-{name}"),
        project: format!("F:/{name}"),
        pipe_name: format!(r"\\.\pipe\locus_broker_test_{name}"),
        state_plane_name: String::new(),
        protocol_version: 1,
        quitting_error: "unity_process_exiting",
        default_status: "editing",
        extra_capabilities: &[],
        execution: UNITY_EXECUTION,
        request_hook: None,
        status_extras: None,
        on_shutdown: None,
        managed_namer: None,
    }
}

fn broker(name: &str) -> Broker {
    Broker::new(config(name))
}

fn enqueue(broker: &Broker, id: &str, raw: &[u8], reattachable: bool) -> Result<(), &'static str> {
    broker.enqueue_with(NO_CLIENT, id.to_string(), raw.to_vec(), RequestPolicy::ordinary(reattachable))
}

/// What the connection's writer would write next, read without a pipe.
struct Frames(Arc<Outbox>);

impl Frames {
    fn try_recv(&mut self) -> Result<Vec<u8>, ()> {
        let frame = self.0.take().ok_or(())?;
        self.0.written();
        Ok(frame)
    }

    fn drain(&mut self) -> Vec<Value> {
        std::iter::from_fn(|| self.try_recv().ok())
            .map(|frame| serde_json::from_slice(&frame).unwrap())
            .collect()
    }
}

fn with_client(broker: &Arc<Broker>) -> (u64, Frames) {
    let (client, outbox) = broker.register_client();
    (client, Frames(outbox))
}

fn broker_with_writer(name: &str) -> (Arc<Broker>, u64, Frames) {
    let broker = Arc::new(broker(name));
    let (client, rx) = with_client(&broker);
    (broker, client, rx)
}

fn next_frame(rx: &mut Frames) -> Value {
    serde_json::from_slice(&rx.try_recv().expect("broker frame")).unwrap()
}

fn next_response(rx: &mut Frames) -> Value {
    loop {
        let value = next_frame(rx);
        if value["type"] == "response" {
            return value;
        }
    }
}

#[test]
fn heartbeat_does_not_publish_or_read_a_clock_on_the_host_thread() {
    let broker = broker("heartbeat");
    broker.snapshot_dirty.store(false, Ordering::Relaxed);
    for _ in 0..10_000 {
        broker.heartbeat(1);
    }
    assert_eq!(broker.heartbeat_sequence.load(Ordering::Relaxed), 10_000);
    assert_eq!(broker.last_heartbeat_ms.load(Ordering::Relaxed), 0);
    assert!(!broker.snapshot_dirty.load(Ordering::Relaxed));
}

#[test]
fn stalled_main_thread_rejects_pending_work_but_preserves_actual_execution() {
    let broker = broker("stalled");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.execution_activity(true);
    enqueue(&broker, "waiting", b"{}", true).unwrap();
    broker.heartbeat_age_ms.store(31_000, Ordering::Relaxed);
    broker.reject_stalled_queue();
    assert!(broker.queue.lock().unwrap().is_empty());
    assert_eq!(broker.active_executions.load(Ordering::Relaxed), 1);
    assert_eq!(enqueue(&broker, "later", b"{}", true), Err("native_main_thread_unresponsive"));
    broker.heartbeat_age_ms.store(0, Ordering::Relaxed);
    assert!(enqueue(&broker, "recovered", b"{}", true).is_ok());
    broker.execution_activity(false);
    assert_eq!(broker.active_executions.load(Ordering::Relaxed), 0);
}

fn answer_register(request: &HostRequest<'_>) -> Option<HostReply> {
    (request.kind == "get_register").then(|| match request.message {
        "fail" => Err("register_failed".to_string()),
        message => Ok(Some(format!("value:{message}"))),
    })
}

#[test]
fn host_hook_answers_its_commands_without_the_main_thread() {
    let mut config = config("hook");
    config.request_hook = Some(answer_register);
    let broker = Arc::new(Broker::new(config));
    let (client, mut rx) = with_client(&broker);
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.heartbeat_age_ms.store(45_000, Ordering::Relaxed);

    handle_line(&broker, client, br#"{"id":"h1","type":"get_register","message":"x"}"#);
    let ok = next_response(&mut rx);
    assert_eq!(ok["reply_to"], "h1");
    assert_eq!(ok["message"], "value:x");
    handle_line(&broker, client, br#"{"id":"h2","type":"get_register","message":"fail"}"#);
    assert_eq!(next_response(&mut rx)["error"], "register_failed");
    assert!(broker.queue.lock().unwrap().is_empty());

    // Other commands still reach the managed executor (or fail fast).
    handle_line(&broker, client, br#"{"id":"h3","type":"select_asset","message":""}"#);
    assert_eq!(next_response(&mut rx)["error"], "native_main_thread_unresponsive");
}

#[test]
fn cancel_is_signaled_off_the_main_thread_and_answered_when_stalled() {
    let (broker, client, mut rx) = broker_with_writer("cancel");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    let start = broker.cancel_seq.load(Ordering::SeqCst);

    handle_line(&broker, client, br#"{"id":"c1","type":"cancel_execute_code","message":"exec-a"}"#);
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
    assert!(broker.execution_cancel_requested("exec-a", start));
    assert!(!broker.execution_cancel_requested("exec-b", start));
    while rx.try_recv().is_ok() {}

    broker.heartbeat_age_ms.store(31_000, Ordering::Relaxed);
    let before_b = broker.cancel_seq.load(Ordering::SeqCst);
    handle_line(&broker, client, br#"{"id":"c2","type":"cancel_execute_code","message":"exec-b"}"#);
    let response = next_response(&mut rx);
    assert_eq!(response["reply_to"], "c2");
    assert_eq!(response["ok"], true);
    assert!(broker.execution_cancel_requested("exec-b", before_b));
    let after_b = broker.cancel_seq.load(Ordering::SeqCst);
    assert!(broker.execution_cancel_requested("exec-b", after_b));
    assert!(!broker.execution_cancel_requested("exec-z", after_b));

    handle_line(&broker, client, br#"{"id":"c3","type":"cancel_execute_code","message":""}"#);
    assert!(broker.execution_cancel_requested("exec-z", after_b));
    let after_all = broker.cancel_seq.load(Ordering::SeqCst);
    assert!(!broker.execution_cancel_requested("exec-later", after_all));
}

#[test]
fn execution_progress_is_answered_without_the_main_thread_once_published() {
    let (broker, client, mut rx) = broker_with_writer("progress");
    broker.set_managed_state(READY, 1, Some("editing".into()));

    handle_line(&broker, client, br#"{"id":"p0","type":"execute_code_progress","message":"exec-a"}"#);
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
    while rx.try_recv().is_ok() {}
    assert_eq!(broker.execution_client_poll_age_ms("exec-a"), -1);

    broker.publish_execution(
        "exec-a",
        r#"{"active":true,"title":"Executing snippet","revision":7,"idleMs":1200,"stopRequested":false}"#,
    );
    broker.heartbeat_age_ms.store(45_000, Ordering::Relaxed);
    handle_line(&broker, client, br#"{"id":"p1","type":"execute_code_progress","message":" exec-a "}"#);
    let response = next_response(&mut rx);
    assert_eq!(response["reply_to"], "p1");
    assert_eq!(response["ok"], true);
    let snapshot: Value = serde_json::from_str(response["message"].as_str().unwrap()).unwrap();
    assert_eq!(snapshot["revision"], 7);
    assert_eq!(snapshot["servedBy"], "native_broker");
    assert_eq!(snapshot["mainThreadStallMs"], 45_000);
    assert!(snapshot["publishedAgeMs"].as_i64().unwrap() >= 0);
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
    assert!(broker.execution_client_poll_age_ms("exec-a") >= 0);

    broker.publish_execution("exec-a", r#"{"active":true,"revision":8}"#);
    assert!(broker.execution_client_poll_age_ms("exec-a") >= 0);

    broker.publish_execution("exec-b", "[1,2]");
    broker.publish_execution("exec-b", "not json");
    assert!(!broker.has_execution("exec-b"));

    broker.end_execution("exec-a");
    assert!(!broker.has_execution("exec-a"));
    assert_eq!(broker.execution_client_poll_age_ms("exec-a"), -1);
}

#[test]
fn waits_for_a_publishing_execution_survive_a_stalled_main_thread() {
    let (broker, client, mut rx) = broker_with_writer("waits");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.publish_execution("exec-live", r#"{"active":true,"revision":1}"#);
    broker.heartbeat_age_ms.store(31_000, Ordering::Relaxed);

    handle_line(&broker, client, br#"{"id":"w1","type":"execute_code_wait","message":"exec-live"}"#);
    handle_line(&broker, client, br#"{"id":"w2","type":"execute_code_wait","message":"exec-gone"}"#);
    let rejected = next_response(&mut rx);
    assert_eq!(rejected["reply_to"], "w2");
    assert_eq!(rejected["error"], "native_main_thread_unresponsive");
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
    assert_eq!(broker.queue.lock().unwrap().front().unwrap().id, "w1");

    broker.reject_stalled_queue();
    assert_eq!(broker.queue.lock().unwrap().len(), 1);

    broker.end_execution("exec-live");
    broker.reject_stalled_queue();
    assert!(broker.queue.lock().unwrap().is_empty());
    assert_eq!(*broker.queued_bytes.lock().unwrap(), 0);
    let late = next_response(&mut rx);
    assert_eq!(late["reply_to"], "w1");
    assert_eq!(late["error"], "native_main_thread_unresponsive");
}

#[test]
fn a_pulse_after_the_stall_sample_ends_the_stall_immediately() {
    let broker = broker("pulse");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.heartbeat_sequence.store(5, Ordering::Relaxed);
    broker.sampled_heartbeat_sequence.store(5, Ordering::Relaxed);
    broker.heartbeat_age_ms.store(35_000, Ordering::Relaxed);
    assert!(broker.main_thread_unresponsive());
    assert_eq!(broker.main_thread_stall_ms(), 35_000);
    assert_eq!(enqueue(&broker, "during", b"{}", false), Err("native_main_thread_unresponsive"));

    broker.heartbeat(1);
    assert!(!broker.main_thread_unresponsive());
    assert_eq!(broker.main_thread_stall_ms(), 0);
    assert!(enqueue(&broker, "after", b"{}", false).is_ok());
}

#[test]
fn execution_requests_outlive_the_ordinary_request_deadline() {
    let broker = Arc::new(broker("deadline"));
    broker.set_managed_state(READY, 1, Some("editing".into()));
    handle_line(&broker, NO_CLIENT, br#"{"id":"e1","type":"execute_loaded","message":"{}"}"#);
    handle_line(&broker, NO_CLIENT, br#"{"id":"o1","type":"select_asset","message":""}"#);
    assert!(broker.take_next_request().is_some());
    assert!(broker.take_next_request().is_some());
    let now = now_ms();
    let inflight = broker.inflight.lock().unwrap();
    assert!(inflight["e1"].reattachable);
    assert!(inflight["e1"].deadline_ms - now > REQUEST_DEADLINE_MS);
    assert!(!inflight["o1"].reattachable);
    assert!(inflight["o1"].deadline_ms - now <= REQUEST_DEADLINE_MS);
}

#[test]
fn reload_forgets_execution_progress_and_status_lists_live_executions() {
    let broker = broker("reload");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.publish_execution(
        "exec-a",
        r#"{"idleMs":5,"idleLimitMs":30000,"mainThreadBlockedMs":12,"stopRequested":false}"#,
    );
    let (_, json) = broker.shared_memory_status_json();
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["executions"][0]["executionId"], "exec-a");
    assert_eq!(value["executions"][0]["idleLimitMs"], 30000);
    assert_eq!(value["hostKind"], "unity");
    assert!(value["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|cap| cap == "broker_execution_progress_v1"));

    broker.set_managed_state(RELOADING, 1, None);
    assert!(!broker.has_execution("exec-a"));
    broker.set_managed_state(READY, 2, Some("editing".into()));
    broker.publish_execution("exec-b", "{}");
    broker.set_managed_state(READY, 3, Some("editing".into()));
    assert!(!broker.has_execution("exec-b"));
}

#[test]
fn execution_register_is_bounded() {
    let broker = broker("bounded");
    for index in 0..EXECUTION_PROGRESS_LIMIT + 4 {
        broker.publish_execution(&format!("exec-{index}"), "{}");
    }
    assert_eq!(broker.executions.lock().unwrap().len(), EXECUTION_PROGRESS_LIMIT);
    let oversized = format!(r#"{{"info":"{}"}}"#, "x".repeat(EXECUTION_PROGRESS_MAX_BYTES));
    broker.end_execution("exec-0");
    broker.publish_execution("exec-big", &oversized);
    assert!(!broker.has_execution("exec-big"));
}

#[test]
fn main_thread_stall_is_unknown_outside_the_ready_state() {
    let broker = broker("stall");
    broker.heartbeat_age_ms.store(12_000, Ordering::Relaxed);
    assert_eq!(broker.main_thread_stall_ms(), -1);
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.heartbeat_age_ms.store(12_000, Ordering::Relaxed);
    assert_eq!(broker.main_thread_stall_ms(), 12_000);
    broker.set_managed_state(RELOADING, 1, None);
    assert_eq!(broker.main_thread_stall_ms(), -1);
}

#[test]
fn complete_drops_stale_or_already_interrupted_requests() {
    let (broker, client, mut rx) = broker_with_writer("complete");
    broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
    assert!(rx.try_recv().is_err());

    broker.inflight.lock().unwrap().insert(
        "req-1".to_string(),
        InflightRequest {
            client,
            deadline_ms: now_ms() + REQUEST_DEADLINE_MS,
            reattachable: true,
        },
    );
    broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
    assert_eq!(rx.try_recv().unwrap(), b"{\"reply_to\":\"req-1\",\"ok\":true}\n");

    broker.complete("req-1", br#"{"reply_to":"req-1","ok":true}"#.to_vec());
    assert!(rx.try_recv().is_err());
}

#[test]
fn enqueue_applies_pending_and_payload_limits() {
    let broker = broker("limits");
    assert_eq!(
        enqueue(&broker, "too-big", &vec![0; MAX_REQUEST_BYTES + 1], false),
        Err("native_payload_too_large")
    );
    for i in 0..MAX_PENDING_REQUESTS {
        enqueue(&broker, &format!("req-{i}"), br#"{"id":"req","type":"ping"}"#, false).unwrap();
    }
    assert_eq!(
        enqueue(&broker, "overflow", br#"{"id":"overflow","type":"ping"}"#, false),
        Err("native_queue_full")
    );
}

#[test]
fn managed_request_emits_acceptance_after_enqueue() {
    let (broker, client, mut rx) = broker_with_writer("accepted");
    broker.set_managed_state(READY, 1, Some("editing".to_string()));
    handle_line(&broker, client, br#"{"id":"req-accepted","type":"execute_code","message":"code"}"#);
    let envelope = next_frame(&mut rx);
    assert_eq!(envelope["type"], REQUEST_ACCEPTED_EVENT);
    let payload: Value = serde_json::from_str(envelope["message"].as_str().unwrap()).unwrap();
    assert_eq!(payload["requestId"], "req-accepted");
    assert_eq!(payload["requestType"], "execute_code");
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
}

#[test]
fn disconnect_retains_only_the_clients_reattachable_execution_requests() {
    let broker = Arc::new(broker("disconnect"));
    let (a, _rx_a) = with_client(&broker);
    let (b, _rx_b) = with_client(&broker);
    broker
        .enqueue_with(a, "ordinary".into(), br#"{"type":"select_asset"}"#.to_vec(), RequestPolicy::ordinary(false))
        .unwrap();
    broker
        .enqueue_with(a, "execute".into(), br#"{"type":"execute_code"}"#.to_vec(), RequestPolicy::ordinary(true))
        .unwrap();
    broker
        .enqueue_with(b, "other-client".into(), br#"{"type":"select_asset"}"#.to_vec(), RequestPolicy::ordinary(false))
        .unwrap();
    broker.inflight.lock().unwrap().insert(
        "ordinary-inflight".to_string(),
        InflightRequest {
            client: a,
            deadline_ms: now_ms() + REQUEST_DEADLINE_MS,
            reattachable: false,
        },
    );

    broker.unregister_client(a);

    let ids = broker
        .queue
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["execute".to_string(), "other-client".to_string()]);
    assert!(!broker.inflight.lock().unwrap().contains_key("ordinary-inflight"));
}

#[test]
fn responses_go_to_the_requesting_client_and_events_to_all() {
    let broker = Arc::new(broker("multi"));
    broker.set_managed_state(READY, 1, Some("editing".into()));
    let (a, mut rx_a) = with_client(&broker);
    let (b, mut rx_b) = with_client(&broker);

    handle_line(&broker, a, br#"{"id":"ping-a","type":"ping"}"#);
    assert_eq!(next_response(&mut rx_a)["reply_to"], "ping-a");
    assert!(rx_b.try_recv().is_err());

    handle_line(&broker, b, br#"{"id":"exec-b","type":"execute_code","message":"x"}"#);
    assert_eq!(next_frame(&mut rx_b)["type"], REQUEST_ACCEPTED_EVENT);
    let request = broker.take_next_request().unwrap();
    assert_eq!(request.id, "exec-b");
    broker.complete("exec-b", br#"{"reply_to":"exec-b","type":"response","ok":true}"#.to_vec());
    assert_eq!(next_response(&mut rx_b)["reply_to"], "exec-b");
    assert!(rx_a.try_recv().is_err());

    broker.emit_event("editor-update", "{}".into());
    assert_eq!(next_frame(&mut rx_a)["type"], "editor-update");
    assert_eq!(next_frame(&mut rx_b)["type"], "editor-update");

    // A reattachable execution whose client left reaches the others.
    handle_line(&broker, b, br#"{"id":"exec-c","type":"execute_code","message":"y"}"#);
    let _ = next_frame(&mut rx_b);
    assert!(broker.take_next_request().is_some());
    broker.unregister_client(b);
    broker.complete("exec-c", br#"{"reply_to":"exec-c","type":"response","ok":true}"#.to_vec());
    assert_eq!(next_response(&mut rx_a)["reply_to"], "exec-c");
}

#[test]
fn a_request_id_in_use_by_another_client_is_rejected() {
    let broker = Arc::new(broker("conflict"));
    broker.set_managed_state(READY, 1, Some("editing".into()));
    let (a, _rx_a) = with_client(&broker);
    let (b, mut rx_b) = with_client(&broker);
    handle_line(&broker, a, br#"{"id":"req-1","type":"select_asset"}"#);
    handle_line(&broker, b, br#"{"id":"req-1","type":"select_asset"}"#);
    assert_eq!(next_response(&mut rx_b)["error"], "native_request_id_conflict");
    assert_eq!(broker.queue.lock().unwrap().len(), 1);
}

#[test]
fn wait_request_blocks_until_work_arrives_or_times_out() {
    let broker = Arc::new(broker("wait"));
    broker.set_managed_state(READY, 1, Some("idle".into()));
    let started = std::time::Instant::now();
    assert!(!broker.wait_request(Duration::from_millis(50)));
    assert!(started.elapsed() >= Duration::from_millis(45));

    let waiter = {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || broker.wait_request(Duration::from_secs(5)))
    };
    std::thread::sleep(Duration::from_millis(50));
    handle_line(&broker, NO_CLIENT, br#"{"id":"w","type":"exec","message":"x"}"#);
    assert!(waiter.join().unwrap());

    let waiter = {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            broker.take_next_request();
            (broker.wait_request(Duration::from_secs(5)), started.elapsed())
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    broker.shutdown();
    let (ready, elapsed) = waiter.join().unwrap();
    assert!(!ready);
    assert!(elapsed < Duration::from_secs(4));
}

#[test]
fn a_token_protected_broker_requires_hello_first() {
    let broker = Arc::new(broker("token"));
    assert!(!broker.requires_hello());
    broker.set_auth_token("s3cret");
    assert!(broker.requires_hello());
    let (client, mut rx) = with_client(&broker);
    assert!(!broker.authenticate(client, br#"{"id":"p","type":"ping"}"#));
    assert_eq!(next_response(&mut rx)["error"], "native_auth_required");
    assert!(!broker.authenticate(client, br#"{"id":"h0","type":"hello","message":"wrong"}"#));
    assert_eq!(next_response(&mut rx)["error"], "native_auth_required");
    assert!(broker.authenticate(client, br#"{"id":"h1","type":"hello","message":"s3cret"}"#));
    let hello = next_response(&mut rx);
    assert_eq!(hello["ok"], true);
    let info: Value = serde_json::from_str(hello["message"].as_str().unwrap()).unwrap();
    assert_eq!(info["hostKind"], "unity");
    assert!(super::broker::constant_time_eq_for_test(b"abc", b"abc"));
    assert!(!super::broker::constant_time_eq_for_test(b"abc", b"abd"));
    assert!(!super::broker::constant_time_eq_for_test(b"abc", b"ab"));
}

#[test]
fn status_names_the_host_and_merges_host_info() {
    let broker = broker("hostinfo");
    broker.set_host_info(r#"{"version":"4.2.1","file":"C:/a.blend","dirty":true}"#);
    let (_, json) = broker.shared_memory_status_json();
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["host"]["version"], "4.2.1");
    assert_eq!(value["instanceKey"], "test-hostinfo");
    broker.set_host_info("not json");
    let (_, json) = broker.shared_memory_status_json();
    assert!(serde_json::from_str::<Value>(&json).unwrap().get("host").is_none());
}

#[test]
fn shared_memory_status_contains_events_and_cursor() {
    let broker = broker("events");
    broker.set_managed_state(READY, 1, Some("editing".to_string()));
    let (_, initial_json) = broker.shared_memory_status_json();
    let initial: Value = serde_json::from_str(&initial_json).unwrap();
    let cursor = initial["cursor"].as_u64().unwrap();
    assert!(cursor >= 1);
    assert_eq!(initial["events"].as_array().unwrap().len(), 1);

    broker.set_managed_state(RELOADING, 1, Some("editing".to_string()));
    let (_, with_events_json) = broker.shared_memory_status_json();
    let with_events: Value = serde_json::from_str(&with_events_json).unwrap();
    let event = with_events["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["seq"].as_u64().unwrap_or(0) > cursor)
        .expect("new lifecycle event")
        .clone();
    assert_eq!(event["kind"], "managed_state_changed");
    assert_eq!(event["from"], "ready");
    assert_eq!(event["to"], "reloading");
    assert!(with_events["cursor"].as_u64().unwrap() > cursor);
}

#[test]
fn shared_memory_status_trims_events_to_payload_budget() {
    let broker = broker("trim");
    for i in 0..STATUS_EVENT_BUFFER_LIMIT {
        broker.push_status_event(
            "managed_state_changed",
            "ready",
            "reloading",
            i as i64,
            "editing".repeat(1024),
            now_ms(),
        );
    }
    let (_, json) = broker.shared_memory_status_json();
    assert!(json.len() <= layout::MAX_PAYLOAD);
    let value: Value = serde_json::from_str(&json).unwrap();
    assert!(value["nativeAlive"].as_bool().unwrap());
    assert_eq!(value["managedState"], "initializing");
    assert!(value["events"].as_array().unwrap().len() < STATUS_EVENT_BUFFER_LIMIT);
}

#[test]
fn state_plane_is_readable_with_the_protocol_reader() {
    let mut config = config("plane");
    config.state_plane_name = format!(r"Local\LocusBrokerTestPlane_{}", std::process::id());
    let broker = Broker::new(config);
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.flush_status_snapshot();
    let guard = broker.state_plane.lock().unwrap();
    let plane = guard.as_ref().expect("plane created");
    let bytes = unsafe { plane.bytes_for_test() };
    let snapshot = layout::read_latest(bytes).expect("snapshot");
    let value: Value = serde_json::from_slice(&snapshot.payload).unwrap();
    assert_eq!(value["managedState"], "ready");
    assert_eq!(snapshot.writer_process_id, std::process::id());
}

fn editor_update(broker: &Broker, seq: usize) {
    broker.emit_event("unity-editor-update", format!(r#"{{"sequence":{seq}}}"#));
}

#[test]
fn a_reading_client_receives_every_event() {
    let (broker, _client, mut rx) = broker_with_writer("events-all");
    for seq in 0..EVENT_FRAME_LIMIT {
        editor_update(&broker, seq);
    }
    assert_eq!(rx.drain().len(), EVENT_FRAME_LIMIT);
}

#[test]
fn a_client_that_stops_reading_keeps_every_reply_and_only_its_latest_events() {
    let (broker, client, mut rx) = broker_with_writer("events-stalled");
    broker.set_managed_state(READY, 1, Some("editing".into()));
    broker.emit_event("open-script", r#"{"path":"A.cs"}"#.into());
    let total = 3 * EVENT_FRAME_LIMIT;
    for seq in 0..total {
        editor_update(&broker, seq);
        if seq == EVENT_FRAME_LIMIT {
            handle_line(&broker, client, br#"{"id":"ping-1","type":"ping"}"#);
            handle_line(&broker, client, br#"{"id":"exec-1","type":"execute_code","message":"x"}"#);
        }
        if seq == 2 * EVENT_FRAME_LIMIT {
            assert_eq!(broker.take_next_request().unwrap().id, "exec-1");
            broker.complete("exec-1", br#"{"reply_to":"exec-1","type":"response","ok":true}"#.to_vec());
        }
    }
    let status = broker.status_json_value(now_ms(), Vec::new(), 0);
    assert_eq!(status["clients"][0]["queuedReplies"], 3);
    assert_eq!(status["clients"][0]["queuedEvents"], EVENT_FRAME_LIMIT);
    assert_eq!(status["clients"][0]["supersededEvents"], total + 1 - EVENT_FRAME_LIMIT);

    let frames = rx.drain();
    let replies = frames
        .iter()
        .filter(|frame| frame["type"] != "unity-editor-update" && frame["type"] != "open-script")
        .map(|frame| frame["reply_to"].as_str().map(str::to_owned).unwrap_or_else(|| frame["type"].as_str().unwrap().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(replies, ["ping-1", REQUEST_ACCEPTED_EVENT, "exec-1"]);
    let sequences = frames
        .iter()
        .filter(|frame| frame["type"] == "unity-editor-update")
        .map(|frame| serde_json::from_str::<Value>(frame["message"].as_str().unwrap()).unwrap()["sequence"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(sequences.len(), EVENT_FRAME_LIMIT - 1);
    assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(*sequences.last().unwrap(), total as u64 - 1);
    // A different event type is not superseded by editor updates.
    assert!(frames.iter().any(|frame| frame["type"] == "open-script"));
}

#[test]
fn a_client_too_far_behind_on_replies_is_disconnected() {
    let (broker, client, mut rx) = broker_with_writer("reply-budget");
    let large = "x".repeat(REPLY_BYTE_LIMIT + 1);
    // One reply larger than the budget is still delivered.
    broker.respond_ok(client, "big", Some(large.clone()));
    assert_eq!(next_response(&mut rx)["reply_to"], "big");

    let quarter = "x".repeat(REPLY_BYTE_LIMIT / 4);
    for i in 0..4 {
        broker.respond_ok(client, &format!("r{i}"), Some(quarter.clone()));
    }
    assert!(rx.0.is_closed());
    assert!(rx.try_recv().is_err());
    // Later replies to the closed connection are ignored, not queued.
    broker.respond_ok(client, "late", None);
    assert!(rx.try_recv().is_err());
}

#[test]
fn request_state_reports_queued_inflight_and_finished_requests() {
    let broker = Arc::new(broker("request-state"));
    broker.set_managed_state(READY, 1, Some("editing".into()));
    let (a, mut rx_a) = with_client(&broker);
    let (b, _rx_b) = with_client(&broker);
    handle_line(&broker, a, br#"{"id":"e1","type":"execute_code","message":"x"}"#);
    handle_line(&broker, a, br#"{"id":"q1","type":"select_asset","message":""}"#);
    handle_line(&broker, b, br#"{"id":"b1","type":"select_asset","message":""}"#);
    assert_eq!(broker.take_next_request().unwrap().id, "e1");
    rx_a.drain();

    handle_line(&broker, a, br#"{"id":"s1","type":"request_state","message":"[\"e1\",\"q1\",\"b1\",\"gone\"]"}"#);
    let states: Value = serde_json::from_str(next_response(&mut rx_a)["message"].as_str().unwrap()).unwrap();
    assert_eq!(states, serde_json::json!({"e1":"inflight","q1":"queued","b1":"none","gone":"none"}));

    broker.complete("e1", br#"{"reply_to":"e1","type":"response","ok":true}"#.to_vec());
    handle_line(&broker, a, br#"{"id":"s2","type":"request_state","message":"[\"e1\"]"}"#);
    // The execute reply is written before the state that reports it done.
    assert_eq!(next_response(&mut rx_a)["reply_to"], "e1");
    let after = next_response(&mut rx_a);
    assert_eq!(after["reply_to"], "s2");
    assert_eq!(after["message"], r#"{"e1":"none"}"#);

    handle_line(&broker, a, br#"{"id":"s3","type":"request_state","message":"e1"}"#);
    assert_eq!(next_response(&mut rx_a)["error"], "native_invalid_request");
    assert!(broker
        .capabilities_string()
        .split(',')
        .any(|cap| cap == locus_native_protocol::capabilities::REQUEST_STATE_V1));
}

#[test]
fn the_writer_waits_for_frames_and_stops_when_closed() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let outbox = Arc::new(Outbox::new(7));
        let reader = Arc::clone(&outbox);
        let writer = tokio::spawn(async move {
            let mut frames = Vec::new();
            while let Some(frame) = reader.next().await {
                reader.written();
                frames.push(frame);
            }
            frames
        });
        tokio::task::yield_now().await;
        assert!(outbox.push_reply(b"a\n".to_vec()));
        tokio::task::yield_now().await;
        outbox.push_event("tick", b"b\n".to_vec());
        tokio::time::sleep(Duration::from_millis(20)).await;
        outbox.close();
        assert_eq!(writer.await.unwrap(), vec![b"a\n".to_vec(), b"b\n".to_vec()]);
        tokio::time::timeout(Duration::from_secs(1), outbox.closed()).await.unwrap();
    });
}
