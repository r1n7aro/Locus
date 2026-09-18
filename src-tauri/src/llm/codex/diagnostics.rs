//! Per-attempt transport evidence. Logs contain metadata only; raw payloads use
//! the existing session raw-attempt storage, without credentials or new schema.
use super::*;
use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub(crate) struct Capture(Arc<StdMutex<Vec<AttemptRecord>>>);

impl Capture {
    pub(super) fn next_attempt(&self) -> u32 {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).len() as u32 + 1
    }
    pub(super) fn record(&self, record: AttemptRecord) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(record);
    }

    pub(super) fn evidence(&self) -> (String, String) {
        let records = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let request = records
            .last()
            .map(|r| r.raw_request.clone())
            .unwrap_or_default();
        let response = serde_json::to_string(&serde_json::json!({
            "codex_stream_attempts": &*records,
        }))
        .unwrap_or_default();
        (request, response)
    }

    pub(super) fn validation_error(&self, error: &str) {
        if let Some(last) = self.0.lock().unwrap_or_else(|e| e.into_inner()).last_mut() {
            last.stage = "validate";
            last.error = Some(error.to_string());
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct AttemptRecord {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub iteration: Option<usize>,
    pub kind: &'static str,
    pub attempt: u32,
    pub transport: &'static str,
    pub stage: &'static str,
    pub connection_reused: bool,
    pub previous_response_id: Option<String>,
    pub response_id: Option<String>,
    pub last_event: Option<String>,
    pub request_bytes: usize,
    pub elapsed_ms: u64,
    pub idle_ms: u64,
    pub error: Option<String>,
    pub raw_request: String,
    pub raw_response: String,
}

pub(super) struct Attempt {
    record: AttemptRecord,
    started: Instant,
    last_activity: Instant,
}

impl Attempt {
    pub(super) fn new(
        session_id: Option<&str>,
        options: &CodexStreamOptions,
        attempt: u32,
        transport: CodexTransportMode,
    ) -> Self {
        Self {
            record: AttemptRecord {
                session_id: session_id.map(str::to_owned),
                run_id: options.request_context.as_ref().map(|c| c.0.clone()),
                iteration: options.request_context.as_ref().map(|c| c.1),
                kind: if options.remote_compaction_v2 {
                    "compaction"
                } else {
                    "sampling"
                },
                attempt,
                transport: if transport == CodexTransportMode::Websocket {
                    "websocket"
                } else {
                    "http"
                },
                stage: "build",
                connection_reused: false,
                previous_response_id: None,
                response_id: None,
                last_event: None,
                request_bytes: 0,
                elapsed_ms: 0,
                idle_ms: 0,
                error: None,
                raw_request: String::new(),
                raw_response: String::new(),
            },
            started: Instant::now(),
            last_activity: Instant::now(),
        }
    }

    pub(super) fn request(&mut self, body: &serde_json::Value) {
        self.record.raw_request = serde_json::to_string_pretty(body).unwrap_or_default();
        self.record.request_bytes = serde_json::to_vec(body).map_or(0, |b| b.len());
        self.record.previous_response_id = body["previous_response_id"].as_str().map(str::to_owned);
    }

    pub(super) fn stage(&mut self, stage: &'static str) {
        self.record.stage = stage;
    }
    pub(super) fn reused(&mut self, reused: bool) {
        self.record.connection_reused = reused;
    }
    pub(super) fn request_text(&self) -> &str {
        &self.record.raw_request
    }
    pub(super) fn response_text(&self) -> &str {
        &self.record.raw_response
    }
    pub(super) fn push_response(&mut self, text: &str) {
        self.record.raw_response.push_str(text);
    }
    pub(super) fn activity(&mut self) {
        self.last_activity = Instant::now();
    }
    pub(super) fn is_compaction(&self) -> bool {
        self.record.kind == "compaction"
    }
    pub(super) fn sync_state(&mut self, state: &CodexStreamState) {
        self.record.last_event = state.last_event.clone();
        self.record.response_id = state.response_id.clone();
    }

    pub(super) fn observe(&mut self, event: &serde_json::Value) {
        self.record.last_event = event["type"].as_str().map(str::to_owned);
        if let Some(id) = event["response"]["id"].as_str() {
            self.record.response_id = Some(id.to_string());
        }
    }

    pub(super) fn finish(mut self, error: Option<&str>, capture: Option<&Capture>) {
        self.record.elapsed_ms =
            self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.record.idle_ms = self
            .last_activity
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.record.error = error.map(str::to_owned);
        if error.is_none() && self.record.stage != "fallback" {
            self.record.stage = "completed";
        }
        if error.is_some() || capture.is_some() {
            eprintln!("[OpenAI Codex] stream attempt: session={} run={} iteration={:?} kind={} attempt={} transport={} stage={} reused={} previous_response={} response_id={} last_event={} request_bytes={} elapsed_ms={} idle_ms={} error={}",
                self.record.session_id.as_deref().unwrap_or("-"), self.record.run_id.as_deref().unwrap_or("-"),
                self.record.iteration, self.record.kind, self.record.attempt, self.record.transport,
                self.record.stage, self.record.connection_reused, self.record.previous_response_id.is_some(),
                self.record.response_id.as_deref().unwrap_or("-"), self.record.last_event.as_deref().unwrap_or("-"),
                self.record.request_bytes, self.record.elapsed_ms, self.record.idle_ms, error.unwrap_or("none"));
        }
        if let Some(capture) = capture {
            capture.record(self.record);
        }
    }
}
