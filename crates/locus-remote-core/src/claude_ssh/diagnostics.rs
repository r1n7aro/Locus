//! Passive counters on the existing transport, plus bounded helper lifecycle
//! metadata. No polling, subprocesses or network probes on the healthy path.
//! Failure probes are bounded and detached from recovery; neither their result
//! nor the UI can authorize input replay.
use super::{heartbeat::Heartbeat, SshConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::VecDeque, sync::{Arc, Mutex, OnceLock, Weak, atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed}}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

#[path = "failure_probe.rs"]
mod failure_probe;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageSample {
    pub stage: &'static str,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperSnapshot {
    pub helper_version: String,
    pub source_hash: Option<String>,
    pub host_pid: u32,
    pub claude_pid: Option<u32>,
    pub started_at: u64,
    pub updated_at: u64,
    pub event: String,
    pub exit_reason: Option<String>,
    pub exit_code: Option<i32>,
    pub claude_exit_code: Option<i32>,
    pub error_type: Option<String>,
    pub reporting_error: Option<String>,
    pub cleanup_error: Option<String>,
}
impl HelperSnapshot {
    fn valid(&self) -> bool {
        let token = |s: &str| !s.is_empty() && s.len() <= 80
            && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c));
        self.host_pid > 0 && self.claude_pid != Some(0) && token(&self.helper_version)
            && matches!(self.event.as_str(), "started" | "ready" | "exit")
            && self.source_hash.as_ref().is_none_or(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            && [&self.exit_reason, &self.error_type, &self.reporting_error, &self.cleanup_error]
                .iter().all(|v| v.as_ref().is_none_or(|s| token(s)))
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportSnapshot {
    pub id: u64,
    pub pid: Option<u32>,
    pub helper: Option<HelperSnapshot>,
    pub operation: String,
    pub stage: &'static str,
    pub elapsed_ms: u64,
    pub stage_elapsed_ms: u64,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub last_receive_age_ms: Option<u64>,
    pub partial_frame_bytes: u64,
    pub expected_frame_bytes: Option<u64>,
    pub stage_received_bytes: u64,
    pub stage_bytes_per_second: Option<f64>,
    pub heartbeat_age_ms: Option<u64>,
    pub pending_heartbeats: usize,
    pub timeline: Vec<StageSample>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureSnapshot {
    pub at: u64,
    pub kind: &'static str,
    pub message: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub transport: TransportSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeCheck {
    pub name: &'static str,
    pub outcome: &'static str,
    pub elapsed_ms: u64,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeReport {
    pub failure_id: u64,
    pub at: u64,
    pub state: &'static str,
    pub checks: Vec<ProbeCheck>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub active: Option<TransportSnapshot>,
    pub last_failure: Option<FailureSnapshot>,
    pub probe: Option<ProbeReport>,
}

struct Stage {
    name: &'static str,
    since_ms: u64,
    received_at_start: u64,
    expected_bytes: Option<u64>,
    timeline: Vec<StageSample>,
}

#[derive(Default)]
struct Stderr {
    tail: Vec<u8>,
    pending: Vec<u8>,
}

pub(super) struct Trace {
    id: u64,
    config: SshConfig,
    operation: String,
    started: Instant,
    pid: AtomicU32,
    received: AtomicU64,
    sent: AtomicU64,
    last_receive: AtomicU64,
    partial_frame: AtomicU64,
    pub(super) stopped: AtomicBool,
    handshaken: AtomicBool,
    failed: AtomicBool,
    stage: Mutex<Stage>,
    stderr: Mutex<Stderr>,
    helper: Mutex<Option<HelperSnapshot>>,
    heartbeat: Arc<Mutex<Heartbeat>>,
}

impl Trace {
    pub(super) fn new(config: &SshConfig, request: &Value, heartbeat: Arc<Mutex<Heartbeat>>) -> Arc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let trace = Arc::new(Self {
            id: NEXT.fetch_add(1, Relaxed), config: config.clone(),
            operation: request["mode"].as_str().unwrap_or("session").into(), started: Instant::now(),
            pid: AtomicU32::new(0), received: AtomicU64::new(0), sent: AtomicU64::new(0),
            last_receive: AtomicU64::new(0), partial_frame: AtomicU64::new(0), stopped: AtomicBool::new(false),
            handshaken: AtomicBool::new(false), failed: AtomicBool::new(false),
            stage: Mutex::new(Stage { name: "ssh", since_ms: 0, received_at_start: 0, expected_bytes: None,
                timeline: vec![StageSample { stage: "ssh", elapsed_ms: 0 }] }),
            stderr: Mutex::new(Stderr::default()), helper: Mutex::new(None), heartbeat,
        });
        // A warm data helper is not a live conversation. Its caller reports
        // backup errors separately; it must not replace the chat health panel.
        if trace.operation == "archive_read" { return trace; }
        let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
        let target = target(&mut targets, config);
        target.live.retain(|trace| trace.strong_count() > 0);
        target.live.push(Arc::downgrade(&trace));
        if target.live.len() > 64 { target.live.remove(0); }
        trace
    }
    pub(super) fn pid(&self, pid: u32) { self.pid.store(pid, Relaxed); }
    pub(super) fn received(&self, bytes: usize, partial: usize) {
        self.received.fetch_add(bytes as u64, Relaxed);
        self.last_receive.store(self.elapsed_ms() + 1, Relaxed);
        self.partial_frame.store(partial as u64, Relaxed);
    }
    pub(super) fn frame_complete(&self) { self.partial_frame.store(0, Relaxed); }
    pub(super) fn sent(&self, bytes: usize) { self.sent.fetch_add(bytes as u64, Relaxed); }
    pub(super) fn stderr(&self, bytes: &[u8]) {
        let records = {
            let mut stderr = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
            stderr.tail.extend_from_slice(bytes);
            let excess = stderr.tail.len().saturating_sub(4096);
            if excess > 0 { stderr.tail.drain(..excess); }
            let mut records = Vec::new();
            // SSH reads may split UTF-8, JSON, or the marker across chunks.
            for &byte in bytes {
                if byte == b'\n' {
                    if let Some(json) = stderr.pending.strip_prefix(b"LOCUS_SSH_HELPER ") {
                        if let Ok(value) = serde_json::from_slice::<Value>(json) { records.push(value); }
                    }
                    stderr.pending.clear();
                } else if stderr.pending.len() <= 4096 {
                    stderr.pending.push(byte);
                }
            }
            records
        };
        for record in records { self.helper(&record); }
    }
    pub(super) fn helper(&self, value: &Value) {
        if !value.is_object() { return; }
        let Ok(record) = serde_json::from_value::<HelperSnapshot>(value.clone()) else { return; };
        if !record.valid() { return; }
        {
            let mut helper = self.helper.lock().unwrap_or_else(|e| e.into_inner());
            if helper.as_ref().is_some_and(|previous| previous.host_pid != record.host_pid
                || previous.updated_at > record.updated_at
                || (previous.event == "exit" && record.event != "exit")) { return; }
            *helper = Some(record.clone());
        }
        // Exit diagnostics can arrive on stderr after stdout EOF froze the
        // failure. Preserve its counters/cause while enriching only metadata.
        let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(failure) = targets.iter_mut().find(|t| t.config == self.config)
            .and_then(|t| t.last_failure.as_mut()).filter(|f| f.transport.id == self.id) {
            failure.transport.helper = Some(record);
        }
    }
    fn elapsed_ms(&self) -> u64 { self.started.elapsed().as_millis() as u64 }
    pub(super) fn progress(&self, frame: &Value) {
        if frame["protocol"] != 1 || frame["type"] != "progress" { return; }
        let name = if frame["source"] == "shell" { "remoteShell" } else {
            match frame["stage"].as_str() {
                Some("history") => "pythonReady",
                Some("environment") => "environment",
                Some("tui") => "tui",
                Some("history_lock") if self.operation == "history" => "historyLock",
                Some("history_read") if self.operation == "history" => "historyRead",
                Some("history_serialize") if self.operation == "history" => "historySerialize",
                Some("history_transfer") if self.operation == "history" => "historyTransfer",
                _ => return,
            }
        };
        self.stage(name, frame["bytes"].as_u64().filter(|n| *n <= super::MAX_FRAME as u64));
    }
    fn stage(&self, name: &'static str, expected_bytes: Option<u64>) {
        let mut stage = self.stage.lock().unwrap_or_else(|e| e.into_inner());
        if stage.name == name { return; }
        let elapsed_ms = self.elapsed_ms();
        stage.name = name;
        stage.since_ms = elapsed_ms;
        stage.received_at_start = self.received.load(Relaxed);
        stage.expected_bytes = expected_bytes;
        if stage.timeline.len() >= 12 { stage.timeline.remove(1); }
        stage.timeline.push(StageSample { stage: name, elapsed_ms });
    }
    pub(super) fn ready(&self) {
        self.handshaken.store(true, Relaxed);
        self.stage("ready", None);
    }
    pub(super) fn has_failed(&self) -> bool { self.failed.load(Relaxed) }
    pub(super) fn unexpected_end(&self) -> bool {
        !self.stopped.load(Relaxed) && self.handshaken.load(Relaxed)
            && matches!(self.operation.as_str(), "session" | "archive")
    }
    pub(super) fn snapshot(&self) -> TransportSnapshot {
        let elapsed_ms = self.elapsed_ms();
        let stage = self.stage.lock().unwrap_or_else(|e| e.into_inner());
        let received_bytes = self.received.load(Relaxed);
        let stage_elapsed_ms = elapsed_ms.saturating_sub(stage.since_ms);
        let stage_received_bytes = received_bytes.saturating_sub(stage.received_at_start);
        let last_receive = self.last_receive.load(Relaxed);
        let (heartbeat_age_ms, pending_heartbeats) = self.heartbeat.lock().unwrap_or_else(|e| e.into_inner()).diagnostic(Instant::now());
        TransportSnapshot {
            id: self.id, pid: Some(self.pid.load(Relaxed)).filter(|pid| *pid != 0),
            helper: self.helper.lock().unwrap_or_else(|e| e.into_inner()).clone(), operation: self.operation.clone(),
            stage: stage.name, elapsed_ms, stage_elapsed_ms, received_bytes, sent_bytes: self.sent.load(Relaxed),
            last_receive_age_ms: last_receive.checked_sub(1).map(|at| elapsed_ms.saturating_sub(at)),
            partial_frame_bytes: self.partial_frame.load(Relaxed), expected_frame_bytes: stage.expected_bytes,
            stage_received_bytes, stage_bytes_per_second: (stage_elapsed_ms >= 1000).then(|| stage_received_bytes as f64 * 1000.0 / stage_elapsed_ms as f64),
            heartbeat_age_ms, pending_heartbeats, timeline: stage.timeline.clone(),
        }
    }
    pub(super) fn observe_exit(&self, code: Option<i32>) {
        if self.stopped.load(Relaxed) || code.is_none() { return; }
        let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(failure) = targets.iter_mut().find(|t| t.config == self.config)
            .and_then(|t| t.last_failure.as_mut()).filter(|f| f.transport.id == self.id) {
            failure.exit_code = code;
        }
    }
    pub(super) fn fail(self: &Arc<Self>, kind: &'static str, message: &str, exit_code: Option<i32>) {
        if self.stopped.load(Relaxed) || self.failed.swap(true, Relaxed) { return; }
        if self.operation == "archive_read" { return; }
        let snapshot = self.snapshot();
        let kind = if kind == "timeout" && snapshot.stage == "historyTransfer" { "historyTransferTimeout" }
            else if kind == "timeout" && snapshot.stage == "historyLock" { "historyLockTimeout" } else { kind };
        let failure = FailureSnapshot { at: timestamp(), kind, message: bounded(message, 1200),
            stderr: String::new(), exit_code, transport: snapshot };
        let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
        let target = target(&mut targets, &self.config);
        target.last_failure = Some(failure);
        let probe = target.begin_probe(self.id, Instant::now());
        drop(targets);
        // Allow late OpenSSH stderr to arrive, without delaying recovery. A
        // clone holds counters only, not the SSH child or its task ownership.
        let trace = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let stderr = super::errors::plain_text(&String::from_utf8_lossy(&trace.stderr.lock().unwrap_or_else(|e| e.into_inner()).tail));
            let helper = trace.helper.lock().unwrap_or_else(|e| e.into_inner()).clone();
            {
                let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
                if let Some(failure) = targets.iter_mut().find(|t| t.config == trace.config)
                    .and_then(|t| t.last_failure.as_mut()).filter(|f| f.transport.id == trace.id) {
                    failure.stderr = stderr;
                    if helper.is_some() { failure.transport.helper = helper; }
                }
            }
            if probe {
                let report = failure_probe::run(&trace.config, trace.id).await;
                let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
                if let Some(target) = targets.iter_mut().find(|t| t.config == trace.config) {
                    if target.probe.as_ref().is_some_and(|p| p.failure_id == trace.id) { target.probe = Some(report); }
                }
            }
        });
    }
}

struct Target {
    config: SshConfig,
    live: Vec<Weak<Trace>>,
    last_failure: Option<FailureSnapshot>,
    probe: Option<ProbeReport>,
    last_probe: Option<Instant>,
}
impl Target {
    fn begin_probe(&mut self, id: u64, now: Instant) -> bool {
        if self.last_probe.is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(60)) { return false; }
        self.last_probe = Some(now);
        self.probe = Some(ProbeReport { failure_id: id, at: timestamp(), state: "running", checks: Vec::new() });
        true
    }
}
fn registry() -> &'static Mutex<VecDeque<Target>> {
    static REGISTRY: OnceLock<Mutex<VecDeque<Target>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
fn target<'a>(targets: &'a mut VecDeque<Target>, config: &SshConfig) -> &'a mut Target {
    if let Some(index) = targets.iter().position(|t| &t.config == config) { return &mut targets[index]; }
    if targets.len() >= 16 { targets.pop_front(); }
    targets.push_back(Target { config: config.clone(), live: Vec::new(), last_failure: None, probe: None, last_probe: None });
    targets.back_mut().unwrap()
}

/// Pure local snapshot, shared by the existing one-second UI poll. The newest
/// operation is explicit: one pool may have several healthy conversations even
/// while a different conversation's history recovery fails.
pub fn overview(config: &SshConfig) -> Overview {
    let (active, last_failure, probe) = {
        let mut targets = registry().lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = targets.iter_mut().find(|t| &t.config == config) else { return Overview::default(); };
        target.live.retain(|trace| trace.strong_count() > 0);
        (target.live.iter().rev().filter_map(Weak::upgrade).find(|t| !t.stopped.load(Relaxed) && !t.failed.load(Relaxed)),
            target.last_failure.clone(), target.probe.clone())
    };
    Overview { active: active.map(|t| t.snapshot()), last_failure, probe }
}
fn timestamp() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
fn bounded(text: &str, length: usize) -> String { text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').take(length).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn config() -> SshConfig { SshConfig { target: uuid::Uuid::new_v4().to_string(), port: None, identity_file: None, known_hosts_file: None, cli_path: None } }
    #[test]
    fn healthy_snapshots_are_passive_and_partial_frames_are_visible() {
        let config = config();
        let trace = Trace::new(&config, &json!({"mode":"history"}), Arc::new(Mutex::new(Heartbeat::default())));
        trace.progress(&json!({"type":"progress","protocol":1,"stage":"history_transfer","bytes":1000}));
        trace.received(200, 200);
        let status = overview(&config);
        let active = status.active.unwrap();
        assert_eq!((active.partial_frame_bytes, active.expected_frame_bytes, active.stage_received_bytes), (200, Some(1000), 200));
        assert_eq!(active.stage, "historyTransfer");
        assert!(status.probe.is_none() && status.last_failure.is_none());
        trace.frame_complete();
        assert_eq!(trace.snapshot().partial_frame_bytes, 0);
        trace.ready();
        assert!(!trace.unexpected_end(), "normal history EOF must not trigger diagnostics");
        trace.stopped.store(true, Relaxed);
        assert!(overview(&config).active.is_none());
        assert!(overview(&self::config()).active.is_none());
    }
    #[test]
    fn download_pool_does_not_replace_conversation_health_or_trigger_failure_probes() {
        let config = config();
        let chat = Trace::new(&config, &json!({"mode":"session"}), Arc::new(Mutex::new(Heartbeat::default())));
        let data = Trace::new(&config, &json!({"mode":"archive_read"}), Arc::new(Mutex::new(Heartbeat::default())));
        data.received(262144, 200000);
        data.fail("timeout", "bulk transfer stalled", None);
        let status = overview(&config);
        assert_eq!(status.active.unwrap().id, chat.id);
        assert!(status.last_failure.is_none() && status.probe.is_none());
    }
    #[test]
    fn failure_probes_are_rate_limited_and_tied_to_the_triggering_failure() {
        let mut targets = VecDeque::new();
        let config = config();
        let target = target(&mut targets, &config);
        let now = Instant::now();
        assert!(target.begin_probe(1, now));
        assert!(!target.begin_probe(2, now + Duration::from_secs(47)));
        assert_eq!(target.probe.as_ref().unwrap().failure_id, 1);
        assert!(target.begin_probe(3, now + Duration::from_secs(61)));
        assert_eq!(target.probe.as_ref().unwrap().failure_id, 3);
    }
    #[tokio::test]
    async fn transfer_failure_freezes_evidence_and_does_not_reprobe_during_cooldown() {
        let config = config();
        let trace = Trace::new(&config, &json!({"mode":"history"}), Arc::new(Mutex::new(Heartbeat::default())));
        trace.progress(&json!({"type":"progress","protocol":1,"stage":"history_transfer","bytes":1000}));
        trace.received(200, 200);
        trace.stderr(b"SSH stderr only");
        // Explicitly enter the cooldown: this test never spawns network probes.
        target(&mut registry().lock().unwrap(), &config).last_probe = Some(Instant::now());
        trace.fail("timeout", "45s expired", None);
        trace.observe_exit(Some(255));
        trace.fail("closed", "later EOF must not overwrite the original cause", None);
        trace.received(100, 300);
        tokio::time::sleep(Duration::from_millis(230)).await;
        let status = overview(&config);
        let failure = status.last_failure.unwrap();
        assert_eq!(failure.kind, "historyTransferTimeout");
        assert_eq!(failure.message, "45s expired");
        assert_eq!(failure.stderr, "SSH stderr only");
        assert_eq!(failure.transport.partial_frame_bytes, 200);
        assert_eq!(failure.exit_code, Some(255));
        assert!(status.probe.is_none());
        assert!(status.active.is_none());
    }
    fn helper_record(event: &str, at: u64) -> Value {
        json!({"helperVersion":"2", "sourceHash":"ab".repeat(32), "hostPid":61196,
            "claudePid":61210, "startedAt":1000, "updatedAt":at, "event":event})
    }
    #[test]
    fn helper_identity_is_remote_not_the_local_ssh_pid_and_stderr_is_chunk_safe() {
        let trace = Trace::new(&config(), &json!({"mode":"session"}), Arc::new(Mutex::new(Heartbeat::default())));
        trace.pid(123);
        let started = helper_record("started", 1000);
        trace.helper(&started);
        assert_eq!(trace.snapshot().pid, Some(123));
        assert_eq!(trace.snapshot().helper.unwrap().host_pid, 61196);
        let mut exit = helper_record("exit", 2000);
        exit["exitCode"] = json!(130);
        exit["exitReason"] = json!("keyboard_interrupt");
        let record = format!("LOCUS_SSH_HELPER {exit}\n");
        for chunk in record.as_bytes().chunks(7) { trace.stderr(chunk); }
        trace.helper(&started); // stdout/stderr delivery order is independent
        let helper = trace.snapshot().helper.unwrap();
        assert_eq!(helper.exit_reason.as_deref(), Some("keyboard_interrupt"));
        assert_eq!(helper.exit_code, Some(130));
        assert_eq!(helper.claude_pid, Some(61210));
        trace.stderr(&vec![b'x'; 20000]);
        assert_eq!(trace.stderr.lock().unwrap().pending.len(), 4097);
        trace.stderr(b"\n");
        trace.stderr(record.as_bytes());
        assert_eq!(trace.snapshot().helper.unwrap().event, "exit");
        assert!(trace.stderr.lock().unwrap().tail.len() <= 4096);
    }
    #[test]
    fn helper_metadata_is_optional_bounded_and_rejects_foreign_pids() {
        let trace = Trace::new(&config(), &json!({}), Arc::new(Mutex::new(Heartbeat::default())));
        trace.helper(&Value::Null); // legacy peers remain valid
        assert!(trace.snapshot().helper.is_none());
        let mut record = helper_record("started", 1000);
        record["helperVersion"] = json!("x".repeat(10000));
        trace.helper(&record);
        assert!(trace.snapshot().helper.is_none());
        trace.helper(&helper_record("ready", 1100));
        record = helper_record("exit", 2000);
        record["hostPid"] = json!(999);
        trace.helper(&record);
        assert_eq!(trace.snapshot().helper.unwrap().event, "ready");
    }
    #[tokio::test]
    async fn late_stderr_enriches_only_its_own_frozen_failure() {
        let config = config();
        let trace = Trace::new(&config, &json!({"mode":"session"}), Arc::new(Mutex::new(Heartbeat::default())));
        trace.helper(&helper_record("started", 1000));
        target(&mut registry().lock().unwrap(), &config).last_probe = Some(Instant::now());
        trace.fail("closed", "original EOF", None);
        let mut exit = helper_record("exit", 2000);
        exit["exitCode"] = json!(1);
        exit["exitReason"] = json!("stdin_error");
        trace.stderr(format!("LOCUS_SSH_HELPER {exit}\n").as_bytes());
        let failure = overview(&config).last_failure.unwrap();
        assert_eq!(failure.message, "original EOF");
        assert_eq!(failure.transport.helper.unwrap().exit_reason.as_deref(), Some("stdin_error"));
        let other = Trace::new(&config, &json!({"mode":"session"}), Arc::new(Mutex::new(Heartbeat::default())));
        other.fail("closed", "new failure", None);
        trace.stderr(format!("LOCUS_SSH_HELPER {exit}\n").as_bytes());
        let failure = overview(&config).last_failure.unwrap();
        assert_eq!(failure.message, "new failure");
        assert!(failure.transport.helper.is_none());
    }
    #[test]
    fn intentional_stop_and_one_shot_completion_are_not_connection_failures() {
        let trace = Trace::new(&config(), &json!({"mode":"session"}), Arc::new(Mutex::new(Heartbeat::default())));
        assert!(!trace.unexpected_end());
        trace.ready();
        assert!(trace.unexpected_end());
        trace.stopped.store(true, Relaxed);
        trace.fail("closed", "intentional close", None); // no runtime or probe required
        assert!(!trace.failed.load(Relaxed));
        assert!(!trace.unexpected_end());
    }
}
