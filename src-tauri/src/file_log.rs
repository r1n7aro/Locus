//! Persistent file sink for the unified app log.
//!
//! Design goals, in priority order:
//! 1. Crash durability: every line handed to the OS via `write_all` survives a
//!    process crash (only an OS/power failure can lose it), so the worker never
//!    buffers formatted lines in user space across batches and panics are
//!    appended synchronously from the panic hook.
//! 2. Hot-path cost: callers normally only clone the entry and `try_send` on a
//!    bounded channel. Overflow writes the full archive synchronously instead
//!    of discarding it; only the rotating diagnostic preview can drop lines.
//!
//! A single worker drains the channel, writes full records to the session
//! archive, and batches the rotating diagnostic preview without `fsync`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::logging::AppLogEntry;

pub const LOG_FILE_NAME: &str = "locus.log";
const ROTATED_FILE_NAME: &str = "locus.log.1";
const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const CHANNEL_CAPACITY: usize = 8_192;
const MAX_BATCH_MESSAGES: usize = 1_024;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const WORKER_THREAD_NAME: &str = "locus-file-log";
const PANIC_FLUSH_TIMEOUT: Duration = Duration::from_millis(300);

enum SinkMsg {
    Entry(AppLogEntry),
    Flush(SyncSender<()>),
}

#[derive(Debug)]
struct FileState {
    file: File,
    written: u64,
}

#[derive(Debug)]
struct SharedFile {
    path: PathBuf,
    rotated_path: PathBuf,
    max_bytes: u64,
    state: Mutex<Option<FileState>>,
}

/// Full text for this process lifetime. The small rotating diagnostic log and
/// the UI ring are previews; neither is a source for exporting complete logs.
#[derive(Debug)]
struct SessionArchive {
    path: PathBuf,
    state: Mutex<SessionArchiveState>,
}

#[derive(Debug)]
struct SessionArchiveState {
    file: File,
    write_error: Option<String>,
}

impl SessionArchive {
    fn create(dir: &Path) -> Result<Self, String> {
        let dir = dir.join("sessions");
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let path = dir.join(format!(
            "locus-{}-{}.log",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            uuid::Uuid::new_v4()
        ));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("failed to create session log {}: {error}", path.display()))?;
        Ok(Self {
            path,
            state: Mutex::new(SessionArchiveState {
                file,
                write_error: None,
            }),
        })
    }

    fn append(&self, text: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Err(error) = state.file.write_all(text.as_bytes()) {
            state.write_error = Some(error.to_string());
        }
    }

    fn append_nonblocking(&self, text: &str) {
        match self.state.try_lock() {
            Ok(mut state) => {
                if let Err(error) = state.file.write_all(text.as_bytes()) {
                    state.write_error = Some(error.to_string());
                }
            }
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let mut state = poisoned.into_inner();
                if let Err(error) = state.file.write_all(text.as_bytes()) {
                    state.write_error = Some(error.to_string());
                }
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                // Panic reporting must not deadlock on the interrupted writer.
                if let Ok(mut file) = OpenOptions::new().append(true).open(&self.path) {
                    let _ = file.write_all(text.as_bytes());
                }
            }
        }
    }

    fn export_to(&self, destination: &Path) -> Result<(), String> {
        // Read only the stable prefix present when export starts. Appends can
        // continue while io::copy streams the file without buffering it in RAM.
        let length = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(error) = &state.write_error {
                return Err(format!("session log could not be fully written: {error}"));
            }
            state
                .file
                .metadata()
                .map_err(|error| error.to_string())?
                .len()
        };
        let mut source = File::open(&self.path)
            .map_err(|error| error.to_string())?
            .take(length);
        let parent = destination
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        // Stage the output so a failed copy leaves an existing export intact.
        let mut output =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        let copied =
            std::io::copy(&mut source, output.as_file_mut()).map_err(|error| error.to_string())?;
        if copied != length {
            return Err("session log changed while exporting".to_string());
        }
        output
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        output
            .persist(destination)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

impl SharedFile {
    fn lock_state(&self) -> MutexGuard<'_, Option<FileState>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn open_state(path: &Path) -> Option<FileState> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()?;
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Some(FileState { file, written })
    }

    /// Appends `text` and rotates afterwards when the size cap is exceeded.
    /// All I/O failures are swallowed: logging must never take the app down.
    fn write_block(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let mut guard = self.lock_state();
        self.write_block_locked(&mut guard, text);
    }

    fn write_block_locked(&self, guard: &mut MutexGuard<'_, Option<FileState>>, text: &str) {
        if guard.is_none() {
            **guard = Self::open_state(&self.path);
        }
        let Some(state) = guard.as_mut() else {
            return;
        };
        if state.file.write_all(text.as_bytes()).is_err() {
            // Drop the handle so the next block retries a fresh open (the
            // volume may have been full or the file replaced externally).
            **guard = None;
            return;
        }
        state.written = state.written.saturating_add(text.len() as u64);
        if state.written >= self.max_bytes {
            self.rotate_locked(guard);
        }
    }

    fn rotate_locked(&self, guard: &mut MutexGuard<'_, Option<FileState>>) {
        // Close our handle first; Windows cannot rename a file we hold open
        // without FILE_SHARE_DELETE cooperation from every other handle.
        **guard = None;
        let _ = std::fs::remove_file(&self.rotated_path);
        if std::fs::rename(&self.path, &self.rotated_path).is_err() {
            // Rename can fail while another process holds the file; fall back
            // to truncating in place so the log cannot grow without bound.
            let _ = OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path);
        }
        **guard = Self::open_state(&self.path);
        if let Some(state) = guard.as_mut() {
            let marker = format!("{} ---- log rotated ----\n", format_local_now());
            if state.file.write_all(marker.as_bytes()).is_ok() {
                state.written = state.written.saturating_add(marker.len() as u64);
            }
        }
    }

    /// Best-effort append that must not deadlock even when called from a
    /// panic unwinding inside `write_block`: falls back to a throwaway
    /// append-mode handle when the mutex is unavailable.
    fn write_block_nonblocking(&self, text: &str) {
        match self.state.try_lock() {
            Ok(mut guard) => self.write_block_locked(&mut guard, text),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                let mut guard = poisoned.into_inner();
                self.write_block_locked(&mut guard, text);
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if let Ok(mut file) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)
                {
                    let _ = file.write_all(text.as_bytes());
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FileLogSink {
    tx: SyncSender<SinkMsg>,
    dropped: Arc<AtomicU64>,
    shared: Arc<SharedFile>,
    archive: Arc<SessionArchive>,
}

impl FileLogSink {
    /// Opens `<dir>/locus.log`, writes a session banner, and starts the worker.
    pub fn init(dir: &Path) -> Result<Arc<Self>, String> {
        Self::init_with_max_bytes(dir, DEFAULT_MAX_FILE_BYTES)
    }

    fn init_with_max_bytes(dir: &Path, max_bytes: u64) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create log dir {}: {error}", dir.display()))?;
        let archive = Arc::new(SessionArchive::create(dir)?);
        archive.append(&session_banner());
        let shared = Arc::new(SharedFile {
            path: dir.join(LOG_FILE_NAME),
            rotated_path: dir.join(ROTATED_FILE_NAME),
            max_bytes,
            state: Mutex::new(None),
        });

        {
            let mut guard = shared.lock_state();
            *guard = SharedFile::open_state(&shared.path);
            if guard.is_none() {
                return Err(format!("failed to open {}", shared.path.display()));
            }
            if guard
                .as_ref()
                .is_some_and(|state| state.written >= max_bytes)
            {
                shared.rotate_locked(&mut guard);
            }
            shared.write_block_locked(&mut guard, &session_banner());
        }

        let (tx, rx) = sync_channel::<SinkMsg>(CHANNEL_CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_shared = shared.clone();
        let worker_dropped = dropped.clone();
        let worker_archive = archive.clone();
        std::thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_string())
            .spawn(move || worker_loop(rx, worker_shared, worker_archive, worker_dropped))
            .map_err(|error| format!("failed to spawn log worker: {error}"))?;

        Ok(Arc::new(Self {
            tx,
            dropped,
            shared,
            archive,
        }))
    }

    /// Default location: `%APPDATA%/locus/logs/locus.log` (persistent config dir).
    /// Isolated runtimes can select an explicit log directory or inherit the
    /// runtime database directory.
    pub fn init_default() -> Result<Arc<Self>, String> {
        let dir = match crate::runtime_paths::runtime_log_dir_from_env()? {
            Some(log_dir) => log_dir,
            None => match crate::commands::runtime_storage_dir_from_env()? {
                Some(data_dir) => data_dir.join("logs"),
                None => crate::commands::persistent_config_dir()?.join("logs"),
            },
        };
        Self::init(&dir)
    }

    pub fn log_path(&self) -> &Path {
        &self.shared.path
    }

    pub fn export_to(&self, destination: &Path) -> Result<(), String> {
        let destination_absolute = if destination.exists() {
            dunce::canonicalize(destination).map_err(|error| error.to_string())?
        } else {
            std::path::absolute(destination).map_err(|error| error.to_string())?
        };
        // An export must never replace the active archive or rotating sink.
        for source in [
            &self.archive.path,
            &self.shared.path,
            &self.shared.rotated_path,
        ] {
            if let Ok(source) = dunce::canonicalize(source) {
                let same_path = if cfg!(windows) {
                    source
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&destination_absolute.to_string_lossy())
                } else {
                    source == destination_absolute
                };
                if same_path {
                    return Err("export destination is an active log file".to_string());
                }
            }
        }
        if !self.flush_blocking(Duration::from_secs(10)) {
            return Err("timed out while flushing session logs".to_string());
        }
        self.archive.export_to(destination)
    }

    /// Queue normally; on overflow preserve the full record in the archive.
    pub fn enqueue(&self, entry: AppLogEntry) {
        match self.tx.try_send(SinkMsg::Entry(entry)) {
            Ok(()) => {}
            Err(TrySendError::Full(SinkMsg::Entry(entry)))
            | Err(TrySendError::Disconnected(SinkMsg::Entry(entry))) => {
                self.archive.append(&format_full_entry_line(&entry));
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => unreachable!("enqueue only sends entries"),
        }
    }

    /// Asks the worker to drain everything queued so far. Returns false when
    /// the worker did not acknowledge within `timeout`.
    pub fn flush_blocking(&self, timeout: Duration) -> bool {
        let (ack_tx, ack_rx) = sync_channel::<()>(1);
        if self.tx.try_send(SinkMsg::Flush(ack_tx)).is_err() {
            return false;
        }
        ack_rx.recv_timeout(timeout).is_ok()
    }

    /// Synchronous append used by the panic hook; bypasses the queue so the
    /// text reaches the OS before the process dies.
    pub fn append_sync(&self, text: &str) {
        self.shared.write_block_nonblocking(text);
        self.archive.append_nonblocking(text);
    }
}

fn worker_loop(
    rx: Receiver<SinkMsg>,
    shared: Arc<SharedFile>,
    archive: Arc<SessionArchive>,
    dropped: Arc<AtomicU64>,
) {
    while let Ok(first) = rx.recv() {
        let mut batch = Vec::with_capacity(16);
        batch.push(first);
        while batch.len() < MAX_BATCH_MESSAGES {
            match rx.try_recv() {
                Ok(msg) => batch.push(msg),
                Err(_) => break,
            }
        }

        let mut acks = Vec::new();
        let mut buf = String::new();
        let dropped_now = dropped.swap(0, Ordering::Relaxed);
        if dropped_now > 0 {
            let marker = format_dropped_line(dropped_now);
            archive.append(&marker);
            buf.push_str(&marker);
        }
        for msg in batch {
            match msg {
                SinkMsg::Entry(entry) => {
                    archive.append(&format_full_entry_line(&entry));
                    buf.push_str(&format_entry_line(&entry));
                }
                SinkMsg::Flush(ack) => acks.push(ack),
            }
        }
        shared.write_block(&buf);
        for ack in acks {
            let _ = ack.send(());
        }
    }
}

/// Installs a panic hook that drains queued lines (bounded wait) and then
/// appends the panic report synchronously, so the crash cause itself can
/// never be lost to the async pipeline. Chains to the previous hook.
pub fn install_panic_hook(sink: Arc<FileLogSink>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let report = render_panic_report(info);
        let on_worker = std::thread::current()
            .name()
            .is_some_and(|name| name == WORKER_THREAD_NAME);
        if !on_worker {
            let _ = sink.flush_blocking(PANIC_FLUSH_TIMEOUT);
        }
        sink.append_sync(&report);
        previous(info);
    }));
}

fn render_panic_report(info: &std::panic::PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>").to_string();
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .map(str::to_string)
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "<unknown location>".to_string());
    let backtrace = std::backtrace::Backtrace::force_capture();

    let mut report = format!(
        "{} [PANIC] [backend] [panic] thread '{thread_name}' panicked at {location}: {payload}\n",
        format_local_now()
    );
    for line in backtrace.to_string().lines() {
        report.push_str("    ");
        report.push_str(line);
        report.push('\n');
    }
    report
}

fn session_banner() -> String {
    format!(
        "\n======== Locus {} pid={} session started {} ========\n",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        format_local_now()
    )
}

fn format_dropped_line(count: u64) -> String {
    format!(
        "{} [WARN] [backend] [FileLog] {count} log line(s) dropped from diagnostic preview (full text retained in session log)\n",
        format_local_now()
    )
}

fn format_local_now() -> String {
    format_local_timestamp(chrono::Utc::now().timestamp_millis())
}

fn format_local_timestamp(timestamp_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S%.3f %:z")
                .to_string()
        })
        .unwrap_or_else(|| format!("<ts {timestamp_ms}>"))
}

/// One entry rendered in the export format: header line plus indented
/// continuation lines. Oversized messages are truncated at a char boundary.
fn format_entry_line(entry: &AppLogEntry) -> String {
    let mut message = entry.message.replace("\r\n", "\n").replace('\r', "\n");
    if message.len() > MAX_MESSAGE_BYTES {
        let mut cut = MAX_MESSAGE_BYTES;
        while cut > 0 && !message.is_char_boundary(cut) {
            cut -= 1;
        }
        let removed = message.len() - cut;
        message.truncate(cut);
        message.push_str(&format!(" …(truncated {removed} bytes)"));
    }

    render_entry_message(entry, &message)
}

fn format_full_entry_line(entry: &AppLogEntry) -> String {
    let message = entry.message.replace("\r\n", "\n").replace('\r', "\n");
    render_entry_message(entry, &message)
}

fn render_entry_message(entry: &AppLogEntry, message: &str) -> String {
    let mut lines = message.split('\n');
    let first = lines.next().unwrap_or_default();
    let mut rendered = format!(
        "{} [{}] [{}] [{}] {first}\n",
        format_local_timestamp(entry.timestamp_ms),
        entry.level.to_ascii_uppercase(),
        entry.source,
        entry.module,
    );
    for line in lines {
        rendered.push_str("    ");
        rendered.push_str(line);
        rendered.push('\n');
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: &str, module: &str, message: &str) -> AppLogEntry {
        AppLogEntry {
            id: "test".to_string(),
            timestamp_ms: 1_750_000_000_000,
            level: level.to_string(),
            source: "backend".to_string(),
            module: module.to_string(),
            target: module.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn format_entry_line_renders_header_and_indents_continuations() {
        let rendered = format_entry_line(&entry("warn", "AssetDb", "first\r\nsecond\nthird"));
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("[WARN] [backend] [AssetDb] first"));
        assert_eq!(lines[1], "    second");
        assert_eq!(lines[2], "    third");
    }

    #[test]
    fn format_entry_line_truncates_oversized_messages() {
        let rendered = format_entry_line(&entry("info", "M", &"x".repeat(MAX_MESSAGE_BYTES + 50)));
        assert!(rendered.contains("…(truncated"));
        assert!(rendered.len() < MAX_MESSAGE_BYTES + 1_024);
    }

    #[test]
    fn sink_writes_banner_entries_and_sync_appends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sink = FileLogSink::init(dir.path()).expect("init sink");

        sink.enqueue(entry("info", "Boot", "hello file log"));
        sink.enqueue(entry("error", "Boot", "something failed"));
        assert!(sink.flush_blocking(Duration::from_secs(5)));
        sink.append_sync("PANIC-MARKER direct append\n");

        let content = std::fs::read_to_string(dir.path().join(LOG_FILE_NAME)).expect("read log");
        assert!(content.contains("session started"));
        assert!(content.contains("[INFO] [backend] [Boot] hello file log"));
        assert!(content.contains("[ERROR] [backend] [Boot] something failed"));
        assert!(content.ends_with("PANIC-MARKER direct append\n"));
    }

    #[test]
    fn sink_rotates_when_size_cap_is_exceeded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sink = FileLogSink::init_with_max_bytes(dir.path(), 2_048).expect("init sink");

        for index in 0..64 {
            sink.enqueue(entry(
                "info",
                "Rotate",
                &format!("line {index} {}", "y".repeat(80)),
            ));
        }
        assert!(sink.flush_blocking(Duration::from_secs(5)));

        let rotated = dir.path().join(ROTATED_FILE_NAME);
        assert!(rotated.is_file(), "expected rotated file to exist");
        let current_len = std::fs::metadata(dir.path().join(LOG_FILE_NAME))
            .expect("current metadata")
            .len();
        assert!(
            current_len < 2_048 + 4_096,
            "current file should restart small"
        );
        let content = std::fs::read_to_string(dir.path().join(LOG_FILE_NAME)).expect("read log");
        assert!(content.contains("---- log rotated ----"));
    }

    #[test]
    fn startup_rotates_an_oversized_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(LOG_FILE_NAME), "z".repeat(4_096)).expect("seed log");

        let sink = FileLogSink::init_with_max_bytes(dir.path(), 1_024).expect("init sink");
        assert!(dir.path().join(ROTATED_FILE_NAME).is_file());
        let content =
            std::fs::read_to_string(sink.log_path()).expect("read log after startup rotate");
        assert!(!content.contains('z'));
        assert!(content.contains("session started"));
    }

    #[test]
    fn dropped_line_mentions_count() {
        let line = format_dropped_line(42);
        assert!(line.contains("42 log line(s) dropped"));
        assert!(line.contains("[FileLog]"));
    }

    #[test]
    fn export_keeps_full_bodies_and_records_evicted_or_cleared_from_the_console() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileLogSink::init_with_max_bytes(dir.path(), 1024).unwrap();
        let store = crate::logging::AppLogStore::new(2);
        store.attach_file_sink(sink.clone());
        let full_message = format!("{}COMPLETE_TAIL", "日志正文".repeat(100_000));
        store.push_backend(tracing::Level::DEBUG, "test", None, full_message.clone());
        for index in 0..2500 {
            store.push_backend(
                tracing::Level::INFO,
                "test",
                None,
                format!("record-{index}"),
            );
        }
        assert_eq!(store.snapshot(2000).len(), 2);
        store.clear();
        let output = dir.path().join("export.log");
        sink.export_to(&output).unwrap();
        let content = std::fs::read_to_string(output).unwrap();
        assert!(content.contains(&full_message));
        assert!(!content.contains("truncated"));
        for index in 0..2500 {
            assert!(content.contains(&format!("record-{index}\n")));
        }
        assert!(content.find("COMPLETE_TAIL").unwrap() < content.find("record-0\n").unwrap());
        assert!(store.snapshot(2000).is_empty());
    }

    #[test]
    fn session_exports_exclude_previous_runs_and_include_sync_panic_lines() {
        let dir = tempfile::tempdir().unwrap();
        let previous = FileLogSink::init(dir.path()).unwrap();
        previous.enqueue(entry("info", "previous", "PREVIOUS_RUN"));
        assert!(previous.flush_blocking(Duration::from_secs(5)));
        let current = FileLogSink::init(dir.path()).unwrap();
        assert_ne!(previous.archive.path, current.archive.path);
        current.enqueue(entry("info", "current", "CURRENT_RUN"));
        current.append_sync("PANIC_FULL_TEXT\n");
        let output = dir.path().join("current.log");
        current.export_to(&output).unwrap();
        let content = std::fs::read_to_string(output).unwrap();
        assert!(!content.contains("PREVIOUS_RUN"));
        assert!(content.contains("CURRENT_RUN"));
        assert!(content.contains("PANIC_FULL_TEXT"));
    }

    #[test]
    fn export_refuses_active_paths_and_preserves_existing_files_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileLogSink::init(dir.path()).unwrap();
        sink.enqueue(entry("info", "test", "DO_NOT_OVERWRITE"));
        assert!(sink.export_to(&sink.archive.path).is_err());
        assert!(sink.export_to(sink.log_path()).is_err());
        let output = dir.path().join("existing.log");
        std::fs::write(&output, "existing export").unwrap();
        sink.archive.state.lock().unwrap().write_error = Some("disk full".to_string());
        assert!(sink.export_to(&output).is_err());
        assert_eq!(std::fs::read_to_string(output).unwrap(), "existing export");
    }

    #[test]
    fn queue_overflow_preserves_full_text_in_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let sink = FileLogSink::init(dir.path()).unwrap();
        let (tx, _rx) = sync_channel(1);
        let (ack_tx, _ack_rx) = sync_channel(1);
        tx.try_send(SinkMsg::Flush(ack_tx)).ok().unwrap();
        let overflow_sink = FileLogSink {
            tx,
            dropped: Arc::new(AtomicU64::new(0)),
            shared: sink.shared.clone(),
            archive: sink.archive.clone(),
        };
        let message = format!("{}OVERFLOW_TAIL", "x".repeat(100_000));
        overflow_sink.enqueue(entry("warn", "overflow", &message));
        let content = std::fs::read_to_string(&sink.archive.path).unwrap();
        assert!(content.contains(&message));
        assert_eq!(overflow_sink.dropped.load(Ordering::Relaxed), 1);
    }
}
