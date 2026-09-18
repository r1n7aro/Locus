use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Emitter};
use tracing::field::Field;
use tracing::{Level, Subscriber};
use tracing_subscriber::field::Visit;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

macro_rules! println {
    () => {{
        let (__module, __message, __level) =
            $crate::logging::prepare_print(module_path!(), false, String::new());
        match __level {
            tracing::Level::ERROR => tracing::error!(log_module = %__module, "{__message}"),
            tracing::Level::WARN => tracing::warn!(log_module = %__module, "{__message}"),
            tracing::Level::INFO => tracing::info!(log_module = %__module, "{__message}"),
            tracing::Level::DEBUG => tracing::debug!(log_module = %__module, "{__message}"),
            tracing::Level::TRACE => tracing::trace!(log_module = %__module, "{__message}"),
        }
    }};
    ($($arg:tt)*) => {{
        let (__module, __message, __level) =
            $crate::logging::prepare_print(module_path!(), false, format!($($arg)*));
        match __level {
            tracing::Level::ERROR => tracing::error!(log_module = %__module, "{__message}"),
            tracing::Level::WARN => tracing::warn!(log_module = %__module, "{__message}"),
            tracing::Level::INFO => tracing::info!(log_module = %__module, "{__message}"),
            tracing::Level::DEBUG => tracing::debug!(log_module = %__module, "{__message}"),
            tracing::Level::TRACE => tracing::trace!(log_module = %__module, "{__message}"),
        }
    }};
}

macro_rules! eprintln {
    () => {{
        let (__module, __message, __level) =
            $crate::logging::prepare_print(module_path!(), true, String::new());
        match __level {
            tracing::Level::ERROR => tracing::error!(log_module = %__module, "{__message}"),
            tracing::Level::WARN => tracing::warn!(log_module = %__module, "{__message}"),
            tracing::Level::INFO => tracing::info!(log_module = %__module, "{__message}"),
            tracing::Level::DEBUG => tracing::debug!(log_module = %__module, "{__message}"),
            tracing::Level::TRACE => tracing::trace!(log_module = %__module, "{__message}"),
        }
    }};
    ($($arg:tt)*) => {{
        let (__module, __message, __level) =
            $crate::logging::prepare_print(module_path!(), true, format!($($arg)*));
        match __level {
            tracing::Level::ERROR => tracing::error!(log_module = %__module, "{__message}"),
            tracing::Level::WARN => tracing::warn!(log_module = %__module, "{__message}"),
            tracing::Level::INFO => tracing::info!(log_module = %__module, "{__message}"),
            tracing::Level::DEBUG => tracing::debug!(log_module = %__module, "{__message}"),
            tracing::Level::TRACE => tracing::trace!(log_module = %__module, "{__message}"),
        }
    }};
}

pub(crate) const APP_LOG_BATCH_EVENT: &str = "app-log-batch";
pub(crate) const DEFAULT_LOG_CAPACITY: usize = 2_000;
const CONSOLE_MAX_MESSAGE_BYTES: usize = 16 * 1024;
const CONSOLE_MAX_FIELD_BYTES: usize = 512;
const CONSOLE_MAX_BYTES: usize = 2 * 1024 * 1024;
const FRONTEND_EVENT_MAX_PENDING: usize = 2_048;
const FRONTEND_EVENT_MAX_BATCH: usize = 128;
const FRONTEND_EVENT_MAX_BATCH_BYTES: usize = 128 * 1024;
const FRONTEND_EVENT_FLUSH_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogEntry {
    pub id: String,
    pub timestamp_ms: i64,
    pub level: String,
    pub source: String,
    pub module: String,
    pub target: String,
    pub message: String,
}

impl AppLogEntry {
    fn text_bytes(&self) -> usize {
        self.id.len()
            + self.level.len()
            + self.source.len()
            + self.module.len()
            + self.target.len()
            + self.message.len()
    }

    fn bound_console_text(&mut self) {
        truncate_console_text(&mut self.message, CONSOLE_MAX_MESSAGE_BYTES);
        truncate_console_text(&mut self.module, CONSOLE_MAX_FIELD_BYTES);
        truncate_console_text(&mut self.target, CONSOLE_MAX_FIELD_BYTES);
    }
}

fn truncate_console_text(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    const SUFFIX: &str = " …(truncated)";
    let mut end = max_bytes - SUFFIX.len();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value.push_str(SUFFIX);
    // Truncating the length alone retains a potentially multi-megabyte allocation.
    value.shrink_to_fit();
}

#[derive(Debug, Default)]
struct ConsoleLogBuffer {
    entries: VecDeque<AppLogEntry>,
    text_bytes: usize,
}

impl ConsoleLogBuffer {
    fn push(&mut self, entry: AppLogEntry, capacity: usize) -> u64 {
        self.text_bytes += entry.text_bytes();
        self.entries.push_back(entry);
        let mut dropped = 0;
        while self.entries.len() > capacity || self.text_bytes > CONSOLE_MAX_BYTES {
            self.pop_front();
            dropped += 1;
        }
        dropped
    }

    fn pop_front(&mut self) -> Option<AppLogEntry> {
        let entry = self.entries.pop_front()?;
        self.text_bytes -= entry.text_bytes();
        Some(entry)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppLogBatchEvent {
    entries: Vec<AppLogEntry>,
    dropped_count: u64,
}

#[derive(Debug, Default)]
struct FrontendEventQueue {
    pending: ConsoleLogBuffer,
    dropped_count: u64,
    flush_scheduled: bool,
}

impl FrontendEventQueue {
    fn push(&mut self, entry: AppLogEntry) -> bool {
        self.dropped_count = self
            .dropped_count
            .saturating_add(self.pending.push(entry, FRONTEND_EVENT_MAX_PENDING));
        if self.flush_scheduled {
            false
        } else {
            self.flush_scheduled = true;
            true
        }
    }

    fn take_batch(&mut self) -> Option<AppLogBatchEvent> {
        if self.pending.entries.is_empty() {
            self.flush_scheduled = false;
            return None;
        }
        let mut entries = Vec::new();
        let mut text_bytes = 0;
        while let Some(next) = self.pending.entries.front() {
            if entries.len() >= FRONTEND_EVENT_MAX_BATCH
                || (!entries.is_empty()
                    && text_bytes + next.text_bytes() > FRONTEND_EVENT_MAX_BATCH_BYTES)
            {
                break;
            }
            text_bytes += next.text_bytes();
            entries.push(self.pending.pop_front().expect("front entry exists"));
        }
        let dropped_count = std::mem::take(&mut self.dropped_count);
        Some(AppLogBatchEvent {
            entries,
            dropped_count,
        })
    }
}

#[derive(Debug)]
pub struct AppLogStore {
    capacity: usize,
    next_id: AtomicU64,
    entries: Mutex<ConsoleLogBuffer>,
    app_handle: Mutex<Option<AppHandle>>,
    file_sink: OnceLock<Arc<crate::file_log::FileLogSink>>,
    self_weak: OnceLock<Weak<AppLogStore>>,
    frontend_events: Mutex<FrontendEventQueue>,
}

impl AppLogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_id: AtomicU64::new(1),
            entries: Mutex::new(ConsoleLogBuffer::default()),
            app_handle: Mutex::new(None),
            file_sink: OnceLock::new(),
            self_weak: OnceLock::new(),
            frontend_events: Mutex::new(FrontendEventQueue::default()),
        }
    }

    pub fn attach_app_handle(self: &Arc<Self>, app_handle: AppHandle) {
        let _ = self.self_weak.set(Arc::downgrade(self));
        if let Ok(mut slot) = self.app_handle.lock() {
            *slot = Some(app_handle);
        }
    }

    pub fn attach_file_sink(&self, sink: Arc<crate::file_log::FileLogSink>) {
        let _ = self.file_sink.set(sink);
    }

    pub fn file_sink(&self) -> Option<&Arc<crate::file_log::FileLogSink>> {
        self.file_sink.get()
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            *entries = ConsoleLogBuffer::default();
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<AppLogEntry> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        let total = entries.entries.len();
        let start = total.saturating_sub(limit);
        entries.entries.iter().skip(start).cloned().collect()
    }

    pub fn push_backend(
        &self,
        level: Level,
        target: &str,
        module_override: Option<String>,
        message: String,
    ) {
        let display_target = normalize_target(target);
        let (module, message) =
            normalize_module_and_message(&display_target, module_override, message);
        let mut entry = AppLogEntry {
            id: format!("backend-{}", self.next_id.fetch_add(1, Ordering::Relaxed)),
            timestamp_ms: Utc::now().timestamp_millis(),
            level: normalize_level(level).to_string(),
            source: "backend".to_string(),
            module,
            target: display_target,
            message,
        };

        if let Some(sink) = self.file_sink.get() {
            sink.enqueue(entry.clone());
        }

        // The persistent file keeps its own, larger limit. Bound the console
        // copy before both storage and IPC: a row preview cannot prevent the
        // WebView from parsing and retaining an oversized request-body log.
        entry.bound_console_text();
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone(), self.capacity);
        }

        self.enqueue_frontend_event(entry);
    }

    fn enqueue_frontend_event(&self, entry: AppLogEntry) {
        let has_app_handle = self
            .app_handle
            .lock()
            .map(|handle| handle.is_some())
            .unwrap_or(false);
        if !has_app_handle {
            return;
        }
        let should_schedule = self
            .frontend_events
            .lock()
            .map(|mut queue| queue.push(entry))
            .unwrap_or(false);
        if !should_schedule {
            return;
        }
        let Some(store) = self.self_weak.get().and_then(Weak::upgrade) else {
            if let Ok(mut queue) = self.frontend_events.lock() {
                queue.flush_scheduled = false;
            }
            return;
        };
        tauri::async_runtime::spawn(async move {
            store.flush_frontend_event_loop().await;
        });
    }

    async fn flush_frontend_event_loop(self: Arc<Self>) {
        loop {
            tokio::time::sleep(FRONTEND_EVENT_FLUSH_INTERVAL).await;
            let batch = self
                .frontend_events
                .lock()
                .ok()
                .and_then(|mut queue| queue.take_batch());
            let Some(batch) = batch else {
                return;
            };
            let app_handle = self
                .app_handle
                .lock()
                .ok()
                .and_then(|handle| handle.clone());
            if let Some(app_handle) = app_handle {
                let _ = app_handle.emit(APP_LOG_BATCH_EVENT, batch);
            }
        }
    }
}

pub fn init_tracing(debug_flag: Arc<std::sync::atomic::AtomicBool>, log_store: Arc<AppLogStore>) {
    let stderr_filter = tracing_subscriber::filter::filter_fn({
        let debug_flag = debug_flag.clone();
        move |metadata| allow_level(metadata.level(), metadata.target(), &debug_flag)
    });
    let capture_filter = tracing_subscriber::filter::filter_fn(move |metadata| {
        allow_level(metadata.level(), metadata.target(), &debug_flag)
    });

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_filter(stderr_filter);

    let capture_layer = AppLogLayer::new(log_store).with_filter(capture_filter);

    if let Err(error) = tracing_subscriber::registry()
        .with(stderr_layer)
        .with(capture_layer)
        .try_init()
    {
        std::eprintln!("[logging] failed to initialize tracing subscriber: {error}");
    }
}

pub fn prepare_print(
    module_path: &'static str,
    is_stderr: bool,
    rendered: String,
) -> (String, String, Level) {
    let normalized = rendered.trim_end_matches(['\r', '\n']).to_string();
    let (module, message) =
        normalize_module_and_message(&normalize_target(module_path), None, normalized);
    (
        module,
        message.clone(),
        classify_print_level(&message, is_stderr),
    )
}

fn allow_level(level: &Level, target: &str, debug_flag: &std::sync::atomic::AtomicBool) -> bool {
    if debug_flag.load(Ordering::Relaxed) {
        !is_third_party_verbose(level, target)
    } else {
        !matches!(*level, Level::DEBUG | Level::TRACE)
    }
}

fn is_third_party_verbose(level: &Level, target: &str) -> bool {
    matches!(*level, Level::DEBUG | Level::TRACE) && !is_app_target(target)
}

fn is_app_target(target: &str) -> bool {
    matches!(target, "locus" | "locus_lib")
        || target.starts_with("locus::")
        || target.starts_with("locus_lib::")
}

fn normalize_level(level: Level) -> &'static str {
    match level {
        Level::TRACE => "trace",
        Level::DEBUG => "debug",
        Level::INFO => "info",
        Level::WARN => "warn",
        Level::ERROR => "error",
    }
}

fn normalize_target(target: &str) -> String {
    target
        .strip_prefix("locus_lib::")
        .or_else(|| target.strip_prefix("locus::"))
        .unwrap_or(target)
        .to_string()
}

fn normalize_module_and_message(
    fallback_target: &str,
    module_override: Option<String>,
    message: String,
) -> (String, String) {
    if let Some((module, stripped)) = extract_bracket_prefix(&message) {
        return (module, stripped.to_string());
    }

    let module = module_override
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback_target.to_string());

    (module, message)
}

fn extract_bracket_prefix(message: &str) -> Option<(String, &str)> {
    let trimmed = message.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }

    let end = trimmed.find(']')?;
    let first_module = trimmed[1..end].trim();
    if first_module.is_empty() {
        return None;
    }

    let mut module = first_module.to_string();
    let mut rest = trimmed[end + 1..].trim_start();
    if matches!(first_module, "DEBUG" | "TRACE" | "INFO" | "WARN" | "ERROR")
        && rest.starts_with('[')
    {
        let second_end = rest.find(']')?;
        let second_module = rest[1..second_end].trim();
        if !second_module.is_empty() {
            module = second_module.to_string();
            rest = rest[second_end + 1..].trim_start();
        }
    }

    Some((module, rest))
}

fn classify_print_level(message: &str, is_stderr: bool) -> Level {
    let lower = message.to_ascii_lowercase();
    if lower.starts_with("[debug")
        || lower.contains("[debug][")
        || lower.contains(" debug]")
        || lower.contains(" trace]")
    {
        return Level::DEBUG;
    }
    let benign_error_state = lower.contains("error=none")
        || lower.contains("last_error=none")
        || lower.contains("process_error=none")
        || lower.contains("\"error\":null")
        || lower.contains("error: none");
    let benign_failure_count =
        lower.contains("0 failed") || lower.contains("\"failed\":0") || lower.contains("failed=0");
    let narrative_panic_reference =
        lower.contains("panic/early-return") || lower.contains("residual risk");
    let has_error_signal =
        (lower.contains(" error") || lower.starts_with("error")) && !benign_error_state;
    let has_failure_signal = lower.contains(" failed") && !benign_failure_count;
    let has_panic_signal = lower.contains("panic") && !narrative_panic_reference;

    if has_panic_signal || has_failure_signal || has_error_signal || lower.contains("exception") {
        return Level::ERROR;
    }
    if lower.contains("warning") || lower.contains("warn") {
        return Level::WARN;
    }
    if is_stderr && lower.contains("retry") {
        return Level::WARN;
    }
    Level::INFO
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    module: Option<String>,
    fields: Vec<String>,
}

impl Visit for EventVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = Some(value.to_string()),
            "log_module" => self.module = Some(value.to_string()),
            _ => self.fields.push(format!("{}={}", field.name(), value)),
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        match field.name() {
            "message" => self.message = Some(format!("{value:?}")),
            "log_module" => self.module = Some(format!("{value:?}").trim_matches('"').to_string()),
            _ => self.fields.push(format!("{}={value:?}", field.name())),
        }
    }
}

#[derive(Clone)]
struct AppLogLayer {
    log_store: Arc<AppLogStore>,
}

impl AppLogLayer {
    fn new(log_store: Arc<AppLogStore>) -> Self {
        Self { log_store }
    }
}

impl<S> Layer<S> for AppLogLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        let mut message = visitor.message.unwrap_or_default();
        if !visitor.fields.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&visitor.fields.join(" "));
        }
        if message.is_empty() {
            message = metadata.name().to_string();
        }

        self.log_store.push_backend(
            *metadata.level(),
            metadata.target(),
            visitor.module,
            message,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        allow_level, classify_print_level, extract_bracket_prefix, normalize_module_and_message,
        AppLogEntry, AppLogStore, FrontendEventQueue, CONSOLE_MAX_BYTES, CONSOLE_MAX_FIELD_BYTES,
        CONSOLE_MAX_MESSAGE_BYTES, DEFAULT_LOG_CAPACITY, FRONTEND_EVENT_MAX_BATCH,
        FRONTEND_EVENT_MAX_BATCH_BYTES, FRONTEND_EVENT_MAX_PENDING,
    };
    use std::sync::atomic::AtomicBool;
    use tracing::Level;

    #[test]
    fn extract_bracket_prefix_splits_module_and_message() {
        let (module, message) = extract_bracket_prefix("[AssetDb] watcher started").unwrap();
        assert_eq!(module, "AssetDb");
        assert_eq!(message, "watcher started");
    }

    #[test]
    fn normalize_module_and_message_prefers_message_prefix() {
        let (module, message) = normalize_module_and_message(
            "commands::workspace",
            Some("workspace".to_string()),
            "[Unity] connected".to_string(),
        );
        assert_eq!(module, "Unity");
        assert_eq!(message, "connected");
    }

    #[test]
    fn classify_print_level_detects_errors_and_debug_messages() {
        assert_eq!(
            classify_print_level("[debug] request body", true),
            Level::DEBUG
        );
        assert_eq!(
            classify_print_level("storage migration failed", true),
            Level::ERROR
        );
        assert_eq!(
            classify_print_level("queued changed Unity assets", true),
            Level::INFO
        );
        assert_eq!(
            classify_print_level("Finished: 14 passed, 0 failed", false),
            Level::INFO
        );
        assert_eq!(
            classify_print_level(
                "DIAG process state=Running last_error=none process_error=none error=none",
                false
            ),
            Level::INFO
        );
        assert_eq!(
            classify_print_level(
                "LOCUS_DRIVER_JSON {\"event\":\"suite_event\",\"payload\":{\"failed\":0,\"line\":\"Finished: 28 passed, 0 failed\"}}",
                false
            ),
            Level::INFO
        );
        assert_eq!(
            classify_print_level(
                "Each suspend is RAII-guarded; a probe panic/early-return still resumes the thread. Residual risk is bounded.",
                false
            ),
            Level::INFO
        );
        assert_eq!(
            classify_print_level(
                "unity_execute progress poll returned error after 18ms",
                false
            ),
            Level::ERROR
        );
    }

    #[test]
    fn debug_mode_filters_third_party_debug_and_trace() {
        let debug_flag = AtomicBool::new(true);

        assert!(!allow_level(
            &Level::TRACE,
            "tokenizers::tokenizer",
            &debug_flag
        ));
        assert!(allow_level(
            &Level::TRACE,
            "locus_lib::knowledge_index",
            &debug_flag
        ));
        assert!(!allow_level(
            &Level::DEBUG,
            "tokenizers::tokenizer",
            &debug_flag
        ));
        assert!(allow_level(
            &Level::DEBUG,
            "locus_lib::knowledge_index",
            &debug_flag
        ));
        assert!(allow_level(&Level::INFO, "ignore::walk", &debug_flag));
    }

    #[test]
    fn frontend_event_queue_batches_and_bounds_log_bursts() {
        let mut queue = FrontendEventQueue::default();
        for index in 0..(FRONTEND_EVENT_MAX_PENDING + 100) {
            let scheduled = queue.push(AppLogEntry {
                id: format!("backend-{index}"),
                timestamp_ms: index as i64,
                level: "debug".to_string(),
                source: "backend".to_string(),
                module: "logging-test".to_string(),
                target: "locus_lib::logging".to_string(),
                message: format!("entry {index}"),
            });
            assert_eq!(scheduled, index == 0);
        }

        assert_eq!(queue.pending.entries.len(), FRONTEND_EVENT_MAX_PENDING);
        let first = queue.take_batch().expect("first batch");
        assert_eq!(first.entries.len(), FRONTEND_EVENT_MAX_BATCH);
        assert_eq!(first.dropped_count, 100);
        assert_eq!(first.entries[0].id, "backend-100");

        while queue.take_batch().is_some() {}
        assert!(!queue.flush_scheduled);
        assert!(queue.push(AppLogEntry {
            id: "backend-next".to_string(),
            timestamp_ms: 0,
            level: "info".to_string(),
            source: "backend".to_string(),
            module: "logging-test".to_string(),
            target: "locus_lib::logging".to_string(),
            message: "next".to_string(),
        }));
    }

    #[test]
    fn console_bounds_large_utf8_logs_before_storage_without_truncating_the_file_copy() {
        let dir = tempfile::tempdir().unwrap();
        let sink = crate::file_log::FileLogSink::init(dir.path()).unwrap();
        let store = AppLogStore::new(DEFAULT_LOG_CAPACITY);
        store.attach_file_sink(sink.clone());
        let message = format!("{}FILE_ONLY_TAIL", "日志".repeat(4_000));
        store.push_backend(Level::DEBUG, &"模块".repeat(1_000), None, message);

        let snapshot = store.snapshot(DEFAULT_LOG_CAPACITY);
        let entry = &snapshot[0];
        assert!(entry.message.len() <= CONSOLE_MAX_MESSAGE_BYTES);
        assert!(entry.message.ends_with(" …(truncated)"));
        assert!(!entry.message.contains("FILE_ONLY_TAIL"));
        assert!(entry.message.capacity() <= CONSOLE_MAX_MESSAGE_BYTES);
        assert!(
            store.entries.lock().unwrap().entries[0].message.capacity()
                <= CONSOLE_MAX_MESSAGE_BYTES
        );
        assert!(entry.module.len() <= CONSOLE_MAX_FIELD_BYTES);
        assert!(entry.target.len() <= CONSOLE_MAX_FIELD_BYTES);
        assert!(sink.flush_blocking(std::time::Duration::from_secs(5)));
        assert!(std::fs::read_to_string(sink.log_path())
            .unwrap()
            .contains("FILE_ONLY_TAIL"));
    }

    #[test]
    fn console_snapshot_has_a_byte_budget_and_retains_latest_entries() {
        let store = AppLogStore::new(DEFAULT_LOG_CAPACITY);
        for index in 0..DEFAULT_LOG_CAPACITY {
            store.push_backend(
                Level::DEBUG,
                "test",
                None,
                format!("{index}: {}", "x".repeat(CONSOLE_MAX_MESSAGE_BYTES)),
            );
        }
        let snapshot = store.snapshot(DEFAULT_LOG_CAPACITY);
        assert!(snapshot.len() < DEFAULT_LOG_CAPACITY);
        assert!(snapshot.iter().map(AppLogEntry::text_bytes).sum::<usize>() <= CONSOLE_MAX_BYTES);
        assert!(snapshot.last().unwrap().message.starts_with("1999:"));
        assert_eq!(store.snapshot(1).len(), 1);
        store.clear();
        assert!(store.snapshot(DEFAULT_LOG_CAPACITY).is_empty());
        assert_eq!(store.entries.lock().unwrap().text_bytes, 0);
        store.push_backend(Level::INFO, "test", None, "after clear".to_string());
        assert_eq!(store.snapshot(1)[0].message, "after clear");
    }

    #[test]
    fn frontend_event_queue_bounds_bytes_and_reports_overflow_once() {
        let store = AppLogStore::new(1);
        store.push_backend(Level::DEBUG, "test", None, "x".repeat(1_433_542));
        let entry = store.snapshot(1).pop().unwrap();
        let mut queue = FrontendEventQueue::default();
        for _ in 0..1_000 {
            queue.push(entry.clone());
        }
        assert!(queue.pending.text_bytes <= CONSOLE_MAX_BYTES);
        let retained = queue.pending.entries.len();
        let first = queue.take_batch().unwrap();
        assert_eq!(first.dropped_count, (1_000 - retained) as u64);
        assert!(
            first
                .entries
                .iter()
                .map(AppLogEntry::text_bytes)
                .sum::<usize>()
                <= FRONTEND_EVENT_MAX_BATCH_BYTES
        );
        let mut delivered = first.entries.len();
        while let Some(batch) = queue.take_batch() {
            assert_eq!(batch.dropped_count, 0);
            assert!(batch.entries.len() <= FRONTEND_EVENT_MAX_BATCH);
            assert!(
                batch
                    .entries
                    .iter()
                    .map(AppLogEntry::text_bytes)
                    .sum::<usize>()
                    <= FRONTEND_EVENT_MAX_BATCH_BYTES
            );
            delivered += batch.entries.len();
        }
        assert_eq!(delivered, retained);
        assert_eq!(queue.pending.text_bytes, 0);
        assert!(!queue.flush_scheduled);
    }
}
