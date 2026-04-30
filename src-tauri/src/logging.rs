use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

pub(crate) const APP_LOG_EVENT: &str = "app-log";
pub(crate) const DEFAULT_LOG_CAPACITY: usize = 2_000;

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

#[derive(Debug)]
pub struct AppLogStore {
    capacity: usize,
    next_id: AtomicU64,
    entries: Mutex<VecDeque<AppLogEntry>>,
    app_handle: Mutex<Option<AppHandle>>,
}

impl AppLogStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_id: AtomicU64::new(1),
            entries: Mutex::new(VecDeque::with_capacity(capacity.min(64))),
            app_handle: Mutex::new(None),
        }
    }

    pub fn attach_app_handle(&self, app_handle: AppHandle) {
        if let Ok(mut slot) = self.app_handle.lock() {
            *slot = Some(app_handle);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
    }

    pub fn snapshot(&self, limit: usize) -> Vec<AppLogEntry> {
        let Ok(entries) = self.entries.lock() else {
            return Vec::new();
        };
        let total = entries.len();
        let start = total.saturating_sub(limit);
        entries.iter().skip(start).cloned().collect()
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
        let entry = AppLogEntry {
            id: format!("backend-{}", self.next_id.fetch_add(1, Ordering::Relaxed)),
            timestamp_ms: Utc::now().timestamp_millis(),
            level: normalize_level(level).to_string(),
            source: "backend".to_string(),
            module,
            target: display_target,
            message,
        };

        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= self.capacity {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }

        if let Ok(handle_guard) = self.app_handle.lock() {
            if let Some(handle) = handle_guard.as_ref() {
                let _ = handle.emit(APP_LOG_EVENT, entry);
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
        !is_third_party_trace(level, target)
    } else {
        !matches!(*level, Level::DEBUG | Level::TRACE)
    }
}

fn is_third_party_trace(level: &Level, target: &str) -> bool {
    matches!(*level, Level::TRACE) && !is_app_target(target)
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
    if lower.contains("panic")
        || lower.contains(" failed")
        || lower.contains(" error")
        || lower.contains("exception")
    {
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
    }

    #[test]
    fn debug_mode_filters_third_party_trace() {
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
        assert!(allow_level(
            &Level::DEBUG,
            "tokenizers::tokenizer",
            &debug_flag
        ));
    }
}
