use std::{collections::HashMap, path::Path, sync::OnceLock, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::{strip_extended_path_prefix, unix_now_ms};

const UNITY_PROCESS_PROBE_CACHE_TTL_MS: u64 = 15_000;
const UNITY_PROCESS_PROBE_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityEditorProcessState {
    Running,
    NotRunning,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct UnityEditorProcessInfo {
    pub state: UnityEditorProcessState,
    pub process_id: Option<u32>,
    pub executable_path: Option<String>,
    pub project_path: Option<String>,
    pub checked_at_ms: u64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UnityProjectProcessCloseResult {
    pub process_ids: Vec<u32>,
    pub forced_process_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
struct Win32UnityProcess {
    process_id: u32,
    executable_path: Option<String>,
    command_line: Option<String>,
}

/// Manifest the Unity editor (2017.1+) writes to `Library/EditorInstance.json`
/// while it has the project open and removes again on clean shutdown. Extra
/// fields (`version`, `app_contents_path`) are ignored.
#[cfg(windows)]
#[derive(Debug, Deserialize)]
struct EditorInstanceManifest {
    process_id: u32,
    #[serde(default)]
    app_path: Option<String>,
}

fn unity_process_probe_cache() -> &'static Mutex<HashMap<String, UnityEditorProcessInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<String, UnityEditorProcessInfo>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl UnityEditorProcessInfo {
    pub fn inferred_running(checked_at_ms: u64) -> Self {
        Self {
            state: UnityEditorProcessState::Running,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: None,
        }
    }

    fn not_running(checked_at_ms: u64) -> Self {
        Self {
            state: UnityEditorProcessState::NotRunning,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: None,
        }
    }

    fn unknown(checked_at_ms: u64, error: impl Into<String>) -> Self {
        Self {
            state: UnityEditorProcessState::Unknown,
            process_id: None,
            executable_path: None,
            project_path: None,
            checked_at_ms,
            last_error: Some(error.into()),
        }
    }

    fn running(checked_at_ms: u64, process: Win32UnityProcess, project_path: String) -> Self {
        Self {
            state: UnityEditorProcessState::Running,
            process_id: Some(process.process_id),
            executable_path: process.executable_path,
            project_path: Some(project_path),
            checked_at_ms,
            last_error: None,
        }
    }
}

pub async fn close_current_project_unity_processes(
    project_path: &str,
    timeout: Duration,
) -> Result<UnityProjectProcessCloseResult, String> {
    close_current_project_unity_processes_inner(project_path, timeout, false).await
}

pub async fn force_close_current_project_unity_processes(
    project_path: &str,
    timeout: Duration,
) -> Result<UnityProjectProcessCloseResult, String> {
    close_current_project_unity_processes_inner(project_path, timeout, true).await
}

async fn close_current_project_unity_processes_inner(
    project_path: &str,
    timeout: Duration,
    force_first: bool,
) -> Result<UnityProjectProcessCloseResult, String> {
    let project_path = project_path.to_string();
    let project_path_for_task = project_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        close_current_project_unity_processes_sync(&project_path_for_task, timeout, force_first)
    })
    .await
    .map_err(|error| format!("Unity process close task failed: {error}"))??;

    let key = process_cache_key(project_path.as_str());
    let mut cache = unity_process_probe_cache().lock().await;
    cache.remove(&key);
    Ok(result)
}

pub(super) async fn refresh_known_project_editor_process_liveness(
    project_path: &str,
    known_process: Option<UnityEditorProcessInfo>,
) -> Option<UnityEditorProcessInfo> {
    let key = process_cache_key(project_path);
    let cached = {
        let cache = unity_process_probe_cache().lock().await;
        cache.get(&key).cloned()
    };
    let previous = known_process
        .filter(|info| info.process_id.is_some())
        .or_else(|| cached.filter(|info| info.process_id.is_some()))?;
    let process_id = previous.process_id?;
    let checked_at_ms = unix_now_ms();

    let refreshed = match is_process_alive(process_id) {
        Ok(true) => UnityEditorProcessInfo {
            state: UnityEditorProcessState::Running,
            process_id: previous.process_id,
            executable_path: previous.executable_path,
            project_path: previous.project_path,
            checked_at_ms,
            last_error: None,
        },
        Ok(false) => UnityEditorProcessInfo::not_running(checked_at_ms),
        Err(error) => UnityEditorProcessInfo::unknown(checked_at_ms, error),
    };

    let mut cache = unity_process_probe_cache().lock().await;
    cache.insert(key, refreshed.clone());
    Some(refreshed)
}

#[cfg(windows)]
fn is_process_alive(process_id: u32) -> Result<bool, String> {
    Ok(probe_native::query_process_facts(process_id)?.alive)
}

#[cfg(not(windows))]
fn is_process_alive(_process_id: u32) -> Result<bool, String> {
    Err("Unity process liveness detection is only supported on Windows".to_string())
}

fn normalize_project_identity(path: &str) -> Option<String> {
    let trimmed = strip_extended_path_prefix(path)
        .trim()
        .trim_matches('"')
        .trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized =
        dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf());
    let mut value = normalized.to_string_lossy().replace('/', "\\");
    while value.len() > 3 && value.ends_with('\\') {
        value.pop();
    }

    if value.is_empty() {
        return None;
    }

    #[cfg(windows)]
    {
        value = value.to_ascii_lowercase();
    }

    Some(value)
}

fn process_cache_key(project_path: &str) -> String {
    normalize_project_identity(project_path).unwrap_or_else(|| {
        strip_extended_path_prefix(project_path)
            .trim()
            .to_ascii_lowercase()
    })
}

pub async fn query_current_project_editor_process(project_path: &str) -> UnityEditorProcessInfo {
    let key = process_cache_key(project_path);
    let now = unix_now_ms();

    {
        let cache = unity_process_probe_cache().lock().await;
        if let Some(cached) = cache.get(&key) {
            if now.saturating_sub(cached.checked_at_ms) <= UNITY_PROCESS_PROBE_CACHE_TTL_MS {
                return cached.clone();
            }
        }
    }

    let project_path = project_path.to_string();
    let probe = match tokio::time::timeout(
        Duration::from_secs(UNITY_PROCESS_PROBE_TIMEOUT_SECS),
        tokio::task::spawn_blocking(move || {
            query_current_project_editor_process_uncached(project_path)
        }),
    )
    .await
    {
        Ok(Ok(info)) => info,
        Ok(Err(join_error)) => UnityEditorProcessInfo::unknown(
            unix_now_ms(),
            format!("Unity process probe task failed: {}", join_error),
        ),
        Err(_) => UnityEditorProcessInfo::unknown(
            unix_now_ms(),
            format!(
                "Unity process probe timed out after {}s",
                UNITY_PROCESS_PROBE_TIMEOUT_SECS
            ),
        ),
    };

    let mut cache = unity_process_probe_cache().lock().await;
    cache.insert(key, probe.clone());
    probe
}

/// Process creation time (ms since the Unix epoch) for `process_id`, used to
/// detect PID reuse across editor restarts and to derive "recently launched".
/// `None` when it cannot be read (process gone, access denied, non-Windows).
#[cfg(windows)]
pub(crate) fn process_created_at_unix_ms(process_id: u32) -> Option<u64> {
    probe_native::query_process_facts(process_id)
        .ok()
        .and_then(|facts| facts.created_at_unix_ms)
}

#[cfg(not(windows))]
pub(crate) fn process_created_at_unix_ms(_process_id: u32) -> Option<u64> {
    None
}

pub(super) async fn cached_project_editor_process(
    project_path: &str,
) -> Option<UnityEditorProcessInfo> {
    let key = process_cache_key(project_path);
    let cache = unity_process_probe_cache().lock().await;
    cache.get(&key).cloned()
}

pub(super) async fn cache_project_editor_process(
    project_path: &str,
    process_info: UnityEditorProcessInfo,
) {
    let key = process_cache_key(project_path);
    let mut cache = unity_process_probe_cache().lock().await;
    cache.insert(key, process_info);
}

#[cfg(windows)]
fn query_current_project_editor_process_uncached(project_path: String) -> UnityEditorProcessInfo {
    let checked_at_ms = unix_now_ms();
    let target = match normalize_project_identity(&project_path) {
        Some(value) => value,
        None => {
            return UnityEditorProcessInfo::unknown(
                checked_at_ms,
                "Current workspace path is empty",
            )
        }
    };

    // Fast path: trust the manifest the editor itself maintains inside the
    // project; it identifies the owning editor instance without touching any
    // other process on the machine.
    if let Some(info) = probe_editor_instance_manifest(&project_path, checked_at_ms) {
        return info;
    }

    // Fallback: enumerate Unity.exe processes and match their -projectPath
    // argument, mirroring the semantics of the previous WMI-based probe.
    let records = match query_unity_processes() {
        Ok(records) => records,
        Err(error) => return UnityEditorProcessInfo::unknown(checked_at_ms, error),
    };

    if records.is_empty() {
        return UnityEditorProcessInfo::not_running(checked_at_ms);
    }

    let mut missing_command_lines = 0usize;
    let mut readable_command_lines = 0usize;

    for process in records {
        let command_line = match process.command_line.as_deref() {
            Some(value) if !value.trim().is_empty() => value,
            _ => {
                missing_command_lines += 1;
                continue;
            }
        };

        readable_command_lines += 1;
        let args = split_windows_command_line(command_line);
        if unity_process_args_are_worker(&args) {
            continue;
        }
        let Some(project_arg) = project_path_from_args(&args) else {
            continue;
        };
        let Some(candidate) = normalize_project_identity(project_arg) else {
            continue;
        };

        if candidate == target {
            return UnityEditorProcessInfo::running(
                checked_at_ms,
                process,
                project_arg.to_string(),
            );
        }
    }

    if readable_command_lines == 0 && missing_command_lines > 0 {
        return UnityEditorProcessInfo::unknown(
            checked_at_ms,
            "Unity process command line is unavailable",
        );
    }

    UnityEditorProcessInfo::not_running(checked_at_ms)
}

#[cfg(windows)]
fn close_current_project_unity_processes_sync(
    project_path: &str,
    timeout: Duration,
    force_first: bool,
) -> Result<UnityProjectProcessCloseResult, String> {
    let processes = query_project_unity_processes(project_path)?;
    let process_ids = processes
        .iter()
        .map(|process| process.process_id)
        .collect::<Vec<_>>();
    if process_ids.is_empty() {
        return Ok(UnityProjectProcessCloseResult {
            process_ids,
            forced_process_ids: Vec::new(),
        });
    }

    if force_first {
        for process_id in &process_ids {
            let _ = request_taskkill(*process_id, true);
        }
        let remaining = wait_for_process_exit(&process_ids, timeout);
        if !remaining.is_empty() {
            return Err(format!(
                "Unity process(es) did not exit before plugin install: {}",
                remaining
                    .iter()
                    .map(|process_id| process_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        return Ok(UnityProjectProcessCloseResult {
            process_ids: process_ids.clone(),
            forced_process_ids: process_ids,
        });
    }

    for process_id in &process_ids {
        let _ = request_taskkill(*process_id, false);
    }

    let graceful_timeout = timeout.min(Duration::from_secs(20));
    let mut remaining = wait_for_process_exit(&process_ids, graceful_timeout);
    let mut forced_process_ids = Vec::new();
    if !remaining.is_empty() {
        forced_process_ids = remaining.clone();
        for process_id in &remaining {
            let _ = request_taskkill(*process_id, true);
        }
        let forced_timeout = timeout
            .saturating_sub(graceful_timeout)
            .max(Duration::from_secs(5));
        remaining = wait_for_process_exit(&remaining, forced_timeout);
    }

    if !remaining.is_empty() {
        return Err(format!(
            "Unity process(es) did not exit before plugin install: {}",
            remaining
                .iter()
                .map(|process_id| process_id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(UnityProjectProcessCloseResult {
        process_ids,
        forced_process_ids,
    })
}

#[cfg(not(windows))]
fn close_current_project_unity_processes_sync(
    _project_path: &str,
    _timeout: Duration,
    _force_first: bool,
) -> Result<UnityProjectProcessCloseResult, String> {
    Ok(UnityProjectProcessCloseResult {
        process_ids: Vec::new(),
        forced_process_ids: Vec::new(),
    })
}

#[cfg(not(windows))]
fn query_current_project_editor_process_uncached(_project_path: String) -> UnityEditorProcessInfo {
    UnityEditorProcessInfo::unknown(
        unix_now_ms(),
        "Unity editor process detection is only supported on Windows",
    )
}

#[cfg(windows)]
fn editor_instance_manifest_path(project_path: &str) -> std::path::PathBuf {
    Path::new(strip_extended_path_prefix(project_path))
        .join("Library")
        .join("EditorInstance.json")
}

/// Lower-cases and backslash-normalizes an executable path for comparison;
/// `EditorInstance.json` records forward slashes while Win32 APIs return
/// backslashes.
#[cfg(windows)]
fn normalize_executable_path(path: &str) -> String {
    strip_extended_path_prefix(path)
        .trim()
        .trim_matches('"')
        .replace('/', "\\")
        .to_ascii_lowercase()
}

/// Probes `Library/EditorInstance.json`. Returns `Some` only for a positively
/// verified running editor; every inconclusive outcome (missing or malformed
/// manifest, dead or unverifiable process) returns `None` so the caller falls
/// back to process enumeration.
#[cfg(windows)]
fn probe_editor_instance_manifest(
    project_path: &str,
    checked_at_ms: u64,
) -> Option<UnityEditorProcessInfo> {
    const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
    // FAT volumes round mtimes to 2s granularity; allow that much skew before
    // deciding the probed process started after the manifest was written.
    const MANIFEST_MTIME_SLACK_MS: u64 = 2_000;

    let manifest_path = editor_instance_manifest_path(project_path);
    let metadata = std::fs::metadata(&manifest_path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return None;
    }
    let manifest_modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64);

    let raw = std::fs::read_to_string(&manifest_path).ok()?;
    let manifest: EditorInstanceManifest = serde_json::from_str(&raw).ok()?;
    if manifest.process_id == 0 {
        return None;
    }

    let facts = probe_native::query_process_facts(manifest.process_id).ok()?;
    if !facts.alive {
        return None;
    }
    let image_path = facts.image_path?;

    // Guard against PID reuse after a crash left a stale manifest: the process
    // must still run the executable the editor recorded (or at least a Unity
    // editor binary), and it must not have started after the manifest was
    // written — the writer necessarily predates its own file.
    let normalized_image = normalize_executable_path(&image_path);
    match manifest
        .app_path
        .as_deref()
        .map(normalize_executable_path)
        .filter(|expected| !expected.is_empty())
    {
        Some(expected) if normalized_image != expected => return None,
        Some(_) => {}
        None if !crate::unity_bridge::flavor::path_ends_with_editor_exe(&normalized_image) => {
            return None
        }
        None => {}
    }
    if let (Some(created_ms), Some(modified_ms)) = (facts.created_at_unix_ms, manifest_modified_ms)
    {
        if created_ms > modified_ms.saturating_add(MANIFEST_MTIME_SLACK_MS) {
            return None;
        }
    }

    Some(UnityEditorProcessInfo {
        state: UnityEditorProcessState::Running,
        process_id: Some(manifest.process_id),
        executable_path: Some(image_path),
        project_path: Some(strip_extended_path_prefix(project_path).trim().to_string()),
        checked_at_ms,
        last_error: None,
    })
}

/// In-process replacement for the previous `powershell.exe` + WMI query.
/// `command_line: None` (e.g. an elevated editor we cannot read) keeps the
/// caller's "command line unavailable" accounting identical to WMI returning
/// a NULL `CommandLine`.
#[cfg(windows)]
fn query_unity_processes() -> Result<Vec<Win32UnityProcess>, String> {
    let mut records = Vec::new();
    for process_id in probe_native::unity_editor_process_ids()? {
        // The process may exit between the snapshot and this query; skip it
        // then, as if it had never been enumerated.
        let Ok(facts) = probe_native::query_process_facts(process_id) else {
            continue;
        };
        if !facts.alive {
            continue;
        }
        records.push(Win32UnityProcess {
            process_id,
            executable_path: facts.image_path,
            command_line: probe_native::read_process_command_line(process_id).ok(),
        });
    }
    Ok(records)
}

#[cfg(windows)]
fn query_project_unity_processes(project_path: &str) -> Result<Vec<Win32UnityProcess>, String> {
    let target = normalize_project_identity(project_path)
        .ok_or_else(|| "Current workspace path is empty".to_string())?;
    let mut records = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(info) = probe_editor_instance_manifest(project_path, unix_now_ms()) {
        if let Some(process_id) = info.process_id {
            if seen.insert(process_id) {
                records.push(Win32UnityProcess {
                    process_id,
                    executable_path: info.executable_path,
                    command_line: None,
                });
            }
        }
    }

    for process in query_unity_processes()? {
        let Some(command_line) = process.command_line.as_deref() else {
            continue;
        };
        let args = split_windows_command_line(command_line);
        let Some(project_arg) = project_path_from_args(&args) else {
            continue;
        };
        let Some(candidate) = normalize_project_identity(project_arg) else {
            continue;
        };
        if candidate == target && seen.insert(process.process_id) {
            records.push(process);
        }
    }

    records.sort_by_key(|process| {
        let worker = process
            .command_line
            .as_deref()
            .map(|line| unity_process_args_are_worker(&split_windows_command_line(line)))
            .unwrap_or(false);
        if worker {
            0
        } else {
            1
        }
    });
    Ok(records)
}

#[cfg(windows)]
fn request_taskkill(process_id: u32, force: bool) -> Result<(), String> {
    let mut command = std::process::Command::new("taskkill");
    command
        .arg("/PID")
        .arg(process_id.to_string())
        .arg("/T")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if force {
        command.arg("/F");
    }

    let status = command.status().map_err(|error| {
        format!(
            "Failed to request Unity process {} close via taskkill: {}",
            process_id, error
        )
    })?;
    if status.success() || !is_process_alive(process_id).unwrap_or(false) {
        Ok(())
    } else {
        Err(format!(
            "taskkill returned {} for Unity process {}",
            status, process_id
        ))
    }
}

#[cfg(windows)]
fn wait_for_process_exit(process_ids: &[u32], timeout: Duration) -> Vec<u32> {
    let started = std::time::Instant::now();
    let mut remaining = process_ids.to_vec();
    loop {
        remaining.retain(|process_id| is_process_alive(*process_id).unwrap_or(true));
        if remaining.is_empty() || started.elapsed() >= timeout {
            return remaining;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Minimal hand-rolled Win32/NT bindings for the Unity process probe,
/// following the FFI style of `unity_bridge::background_hook`.
#[cfg(windows)]
mod probe_native {
    use std::ffi::c_void;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;
    type NtStatus = i32;

    const FALSE: Bool = 0;
    const PROCESS_QUERY_LIMITED_INFORMATION: Dword = 0x1000;
    const PROCESS_QUERY_INFORMATION: Dword = 0x0400;
    const PROCESS_VM_READ: Dword = 0x0010;
    const STILL_ACTIVE: Dword = 259;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    const ERROR_NO_MORE_FILES: i32 = 18;
    const TH32CS_SNAPPROCESS: Dword = 0x0000_0002;
    const PROCESS_BASIC_INFORMATION_CLASS: u32 = 0;
    /// Sized for `\\?\`-style long paths (in UTF-16 units).
    const IMAGE_PATH_CAPACITY: usize = 32_768;
    /// Milliseconds between the FILETIME epoch (1601-01-01) and the Unix epoch.
    const FILETIME_UNIX_EPOCH_DIFF_MS: u64 = 11_644_473_600_000;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: Dword,
        _cnt_usage: Dword,
        th32_process_id: Dword,
        _th32_default_heap_id: usize,
        _th32_module_id: Dword,
        _cnt_threads: Dword,
        _th32_parent_process_id: Dword,
        _pc_pri_class_base: i32,
        _dw_flags: Dword,
        sz_exe_file: [u16; 260],
    }

    #[repr(C)]
    struct Filetime {
        dw_low_date_time: Dword,
        dw_high_date_time: Dword,
    }

    /// `UNICODE_STRING` (winternl.h); `length` is in bytes.
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        _maximum_length: u16,
        buffer: *mut u16,
    }

    /// `PROCESS_BASIC_INFORMATION` (winternl.h).
    #[repr(C)]
    struct ProcessBasicInformation {
        _exit_status: NtStatus,
        peb_base_address: *mut c_void,
        _affinity_mask: usize,
        _base_priority: i32,
        _unique_process_id: usize,
        _inherited_from_unique_process_id: usize,
    }

    /// Leading fields of the `PEB` as documented in winternl.h; only
    /// `process_parameters` is read.
    #[repr(C)]
    struct PebPartial {
        _reserved1: [u8; 2],
        _being_debugged: u8,
        _reserved2: [u8; 1],
        _reserved3: [*mut c_void; 2],
        _ldr: *mut c_void,
        process_parameters: *mut c_void,
    }

    /// Leading fields of `RTL_USER_PROCESS_PARAMETERS` as documented in
    /// winternl.h.
    #[repr(C)]
    struct RtlUserProcessParametersPartial {
        _reserved1: [u8; 16],
        _reserved2: [*mut c_void; 10],
        _image_path_name: UnicodeString,
        command_line: UnicodeString,
    }

    unsafe extern "system" {
        fn CloseHandle(h_object: Handle) -> Bool;
        fn OpenProcess(
            dw_desired_access: Dword,
            b_inherit_handle: Bool,
            dw_process_id: Dword,
        ) -> Handle;
        fn GetExitCodeProcess(h_process: Handle, lp_exit_code: *mut Dword) -> Bool;
        fn GetProcessTimes(
            h_process: Handle,
            lp_creation_time: *mut Filetime,
            lp_exit_time: *mut Filetime,
            lp_kernel_time: *mut Filetime,
            lp_user_time: *mut Filetime,
        ) -> Bool;
        fn QueryFullProcessImageNameW(
            h_process: Handle,
            dw_flags: Dword,
            lp_exe_name: *mut u16,
            lpdw_size: *mut Dword,
        ) -> Bool;
        fn CreateToolhelp32Snapshot(dw_flags: Dword, th32_process_id: Dword) -> Handle;
        fn Process32FirstW(h_snapshot: Handle, lppe: *mut ProcessEntry32W) -> Bool;
        fn Process32NextW(h_snapshot: Handle, lppe: *mut ProcessEntry32W) -> Bool;
        fn ReadProcessMemory(
            h_process: Handle,
            lp_base_address: *const c_void,
            lp_buffer: *mut c_void,
            n_size: usize,
            lp_number_of_bytes_read: *mut usize,
        ) -> Bool;
    }

    // ntdll is not linked by default; raw-dylib avoids depending on a Windows
    // SDK import library.
    #[link(name = "ntdll", kind = "raw-dylib")]
    unsafe extern "system" {
        fn NtQueryInformationProcess(
            process_handle: Handle,
            process_information_class: u32,
            process_information: *mut c_void,
            process_information_length: u32,
            return_length: *mut u32,
        ) -> NtStatus;
    }

    struct OwnedHandle(Handle);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub(super) struct ProcessFacts {
        pub(super) alive: bool,
        pub(super) image_path: Option<String>,
        pub(super) created_at_unix_ms: Option<u64>,
    }

    /// Queries liveness, image path and creation time through a single
    /// `PROCESS_QUERY_LIMITED_INFORMATION` handle. A process id that no longer
    /// exists yields `alive: false` rather than an error.
    pub(super) fn query_process_facts(process_id: u32) -> Result<ProcessFacts, String> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process_id) };
        if handle.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER) {
                return Ok(ProcessFacts {
                    alive: false,
                    image_path: None,
                    created_at_unix_ms: None,
                });
            }
            return Err(format!(
                "Failed to open Unity process {} for probing: {}",
                process_id, error
            ));
        }
        let handle = OwnedHandle(handle);

        let mut exit_code: Dword = 0;
        if unsafe { GetExitCodeProcess(handle.0, &mut exit_code) } == 0 {
            return Err(format!(
                "Failed to query Unity process {} exit code: {}",
                process_id,
                std::io::Error::last_os_error()
            ));
        }
        if exit_code != STILL_ACTIVE {
            return Ok(ProcessFacts {
                alive: false,
                image_path: None,
                created_at_unix_ms: None,
            });
        }

        Ok(ProcessFacts {
            alive: true,
            image_path: query_image_path(&handle),
            created_at_unix_ms: query_creation_unix_ms(&handle),
        })
    }

    fn query_image_path(handle: &OwnedHandle) -> Option<String> {
        let mut buffer = vec![0u16; IMAGE_PATH_CAPACITY];
        let mut size = buffer.len() as Dword;
        if unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut size) } == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..size as usize]))
    }

    fn query_creation_unix_ms(handle: &OwnedHandle) -> Option<u64> {
        let mut creation: Filetime = unsafe { std::mem::zeroed() };
        let mut exit: Filetime = unsafe { std::mem::zeroed() };
        let mut kernel: Filetime = unsafe { std::mem::zeroed() };
        let mut user: Filetime = unsafe { std::mem::zeroed() };
        if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return None;
        }
        let ticks =
            (u64::from(creation.dw_high_date_time) << 32) | u64::from(creation.dw_low_date_time);
        (ticks / 10_000).checked_sub(FILETIME_UNIX_EPOCH_DIFF_MS)
    }

    /// Lists process ids whose executable name is a known editor image
    /// (`Unity.exe`, or Tuanjie's `Tuanjie.exe`).
    pub(super) fn unity_editor_process_ids() -> Result<Vec<u32>, String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot as isize == -1 {
            return Err(format!(
                "Failed to snapshot processes for Unity probe: {}",
                std::io::Error::last_os_error()
            ));
        }
        let snapshot = OwnedHandle(snapshot);

        let mut entry: ProcessEntry32W = unsafe { std::mem::zeroed() };
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as Dword;

        let mut process_ids = Vec::new();
        if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES) {
                return Ok(process_ids);
            }
            return Err(format!("Failed to enumerate processes: {}", error));
        }

        loop {
            if crate::unity_bridge::flavor::is_editor_process_image(&wide_to_string(
                &entry.sz_exe_file,
            )) {
                process_ids.push(entry.th32_process_id);
            }
            if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
                break;
            }
        }
        Ok(process_ids)
    }

    /// Reads a process command line from its PEB
    /// (`PEB -> RTL_USER_PROCESS_PARAMETERS -> CommandLine`). Fails for
    /// processes we lack `PROCESS_VM_READ` access to, e.g. an elevated editor.
    pub(super) fn read_process_command_line(process_id: u32) -> Result<String, String> {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
                FALSE,
                process_id,
            )
        };
        if handle.is_null() {
            return Err(format!(
                "Failed to open Unity process {} for command line read: {}",
                process_id,
                std::io::Error::last_os_error()
            ));
        }
        let handle = OwnedHandle(handle);

        let mut info: ProcessBasicInformation = unsafe { std::mem::zeroed() };
        let mut return_length: u32 = 0;
        let status = unsafe {
            NtQueryInformationProcess(
                handle.0,
                PROCESS_BASIC_INFORMATION_CLASS,
                &mut info as *mut ProcessBasicInformation as *mut c_void,
                std::mem::size_of::<ProcessBasicInformation>() as u32,
                &mut return_length,
            )
        };
        if status != 0 {
            return Err(format!(
                "NtQueryInformationProcess failed for Unity process {}: status 0x{:08X}",
                process_id, status as u32
            ));
        }
        if info.peb_base_address.is_null() {
            return Err(format!("Unity process {} reported no PEB", process_id));
        }

        let peb: PebPartial = read_remote(&handle, process_id, info.peb_base_address)?;
        if peb.process_parameters.is_null() {
            return Err(format!(
                "Unity process {} reported no process parameters",
                process_id
            ));
        }
        let parameters: RtlUserProcessParametersPartial =
            read_remote(&handle, process_id, peb.process_parameters)?;
        read_remote_unicode_string(&handle, process_id, &parameters.command_line)
    }

    fn read_remote<T>(
        handle: &OwnedHandle,
        process_id: u32,
        address: *const c_void,
    ) -> Result<T, String> {
        let mut value: T = unsafe { std::mem::zeroed() };
        let mut bytes_read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                handle.0,
                address,
                &mut value as *mut T as *mut c_void,
                std::mem::size_of::<T>(),
                &mut bytes_read,
            )
        };
        if ok == 0 || bytes_read != std::mem::size_of::<T>() {
            return Err(format!(
                "Failed to read Unity process {} memory: {}",
                process_id,
                std::io::Error::last_os_error()
            ));
        }
        Ok(value)
    }

    fn read_remote_unicode_string(
        handle: &OwnedHandle,
        process_id: u32,
        value: &UnicodeString,
    ) -> Result<String, String> {
        let byte_len = usize::from(value.length) & !1usize;
        if byte_len == 0 {
            return Ok(String::new());
        }
        if value.buffer.is_null() {
            return Err(format!(
                "Unity process {} command line buffer is null",
                process_id
            ));
        }

        let mut buffer = vec![0u16; byte_len / 2];
        let mut bytes_read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                handle.0,
                value.buffer as *const c_void,
                buffer.as_mut_ptr() as *mut c_void,
                byte_len,
                &mut bytes_read,
            )
        };
        if ok == 0 || bytes_read != byte_len {
            return Err(format!(
                "Failed to read Unity process {} command line: {}",
                process_id,
                std::io::Error::last_os_error()
            ));
        }
        Ok(String::from_utf16_lossy(&buffer))
    }

    fn wide_to_string(value: &[u16]) -> String {
        let length = value.iter().position(|&c| c == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..length])
    }
}

fn project_path_from_args(args: &[String]) -> Option<&str> {
    const PROJECT_PATH_ARG: &str = "-projectpath";

    for (index, arg) in args.iter().enumerate() {
        let lower = arg.to_ascii_lowercase();
        if lower == PROJECT_PATH_ARG {
            return args.get(index + 1).map(String::as_str);
        }

        if lower.starts_with("-projectpath=") {
            return arg.get(PROJECT_PATH_ARG.len() + 1..);
        }
    }

    None
}

fn unity_process_args_are_worker(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg.to_ascii_lowercase().contains("assetimportworker"))
}

fn split_windows_command_line(command_line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut backslashes = 0usize;
    let mut has_current = false;

    for ch in command_line.chars() {
        if ch == '\\' {
            backslashes += 1;
            has_current = true;
            continue;
        }

        if ch == '"' {
            current.extend(std::iter::repeat('\\').take(backslashes / 2));
            if backslashes % 2 == 0 {
                in_quotes = !in_quotes;
            } else {
                current.push('"');
            }
            backslashes = 0;
            has_current = true;
            continue;
        }

        if backslashes > 0 {
            current.extend(std::iter::repeat('\\').take(backslashes));
            backslashes = 0;
        }

        if ch.is_whitespace() && !in_quotes {
            if has_current {
                args.push(std::mem::take(&mut current));
                has_current = false;
            }
            continue;
        }

        current.push(ch);
        has_current = true;
    }

    if backslashes > 0 {
        current.extend(std::iter::repeat('\\').take(backslashes));
    }

    if has_current {
        args.push(current);
    }

    args
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_project_identity, project_path_from_args, split_windows_command_line,
        unity_process_args_are_worker,
    };

    #[test]
    fn parses_unity_hub_project_path_argument_case_insensitively() {
        let args = split_windows_command_line(
            r#"E:\2022.3.58f1\Editor\Unity.exe -projectpath "J:\My project (2)" -useHub -hubIPC"#,
        );

        assert_eq!(project_path_from_args(&args), Some(r#"J:\My project (2)"#));
    }

    #[test]
    fn parses_locus_project_path_argument() {
        let args = split_windows_command_line(
            r#""C:\Program Files\Unity\Editor\Unity.exe" -projectPath D:\Projects\Game -batchmode"#,
        );

        assert_eq!(project_path_from_args(&args), Some(r#"D:\Projects\Game"#));
    }

    #[test]
    fn parses_project_path_equals_form() {
        let args =
            split_windows_command_line(r#"Unity.exe -projectPath="D:\Projects\Game With Spaces""#);

        assert_eq!(
            project_path_from_args(&args),
            Some(r#"D:\Projects\Game With Spaces"#)
        );
    }

    #[test]
    fn detects_unity_asset_import_worker_processes() {
        let worker_args = split_windows_command_line(
            r#""C:\Unity\2022.3.47f1\Editor\Unity.exe" "-adb2" "-batchMode" "-noUpm" "-name" "AssetImportWorker1" "-projectPath" "D:/Projects/Game""#,
        );
        let editor_args = split_windows_command_line(
            r#"C:\Unity\2022.3.47f1\Editor\Unity.exe -projectpath "D:\Projects\Game" -useHub -hubIPC"#,
        );

        assert!(unity_process_args_are_worker(&worker_args));
        assert!(!unity_process_args_are_worker(&editor_args));
    }

    #[test]
    fn normalizes_extended_path_prefixes() {
        let normalized = normalize_project_identity(r#"\\?\D:\Projects\Game\"#).unwrap();

        #[cfg(windows)]
        assert_eq!(normalized, r#"d:\projects\game"#);

        #[cfg(not(windows))]
        assert_eq!(normalized, r#"D:\Projects\Game"#);
    }

    #[cfg(windows)]
    mod windows_probe {
        use super::super::{normalize_executable_path, probe_native, EditorInstanceManifest};

        #[test]
        fn parses_editor_instance_manifest_sample() {
            let raw = r#"{
	"process_id" : 571648,
	"version" : "2022.3.47f1",
	"app_path" : "E:/2022.3.47f1/Editor/Unity.exe",
	"app_contents_path" : "E:/2022.3.47f1/Editor/Data"
}"#;

            let manifest: EditorInstanceManifest = serde_json::from_str(raw).unwrap();

            assert_eq!(manifest.process_id, 571648);
            assert_eq!(
                manifest.app_path.as_deref(),
                Some("E:/2022.3.47f1/Editor/Unity.exe")
            );
        }

        #[test]
        fn editor_instance_manifest_tolerates_missing_app_path() {
            let manifest: EditorInstanceManifest =
                serde_json::from_str(r#"{"process_id": 42}"#).unwrap();

            assert_eq!(manifest.process_id, 42);
            assert!(manifest.app_path.is_none());
        }

        #[test]
        fn normalizes_executable_paths_across_slash_and_case() {
            assert_eq!(
                normalize_executable_path("E:/2022.3.47f1/Editor/Unity.exe"),
                normalize_executable_path(r"e:\2022.3.47f1\editor\UNITY.EXE")
            );
        }

        #[test]
        fn query_process_facts_reports_current_process_alive() {
            let facts = probe_native::query_process_facts(std::process::id()).unwrap();

            assert!(facts.alive);

            let image_path = facts.image_path.expect("image path should be readable");
            let current_exe_name = std::env::current_exe()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            assert!(
                normalize_executable_path(&image_path).ends_with(&current_exe_name),
                "image path {:?} should end with {:?}",
                image_path,
                current_exe_name
            );

            let created_at = facts
                .created_at_unix_ms
                .expect("creation time should be readable");
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            assert!(created_at <= now_ms);
        }

        #[test]
        fn reads_current_process_command_line() {
            let command_line = probe_native::read_process_command_line(std::process::id()).unwrap();

            let current_exe_name = std::env::current_exe()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            assert!(
                command_line
                    .to_ascii_lowercase()
                    .contains(&current_exe_name),
                "command line {:?} should mention {:?}",
                command_line,
                current_exe_name
            );
        }

        #[test]
        fn enumerates_unity_process_ids_without_error() {
            probe_native::unity_editor_process_ids().unwrap();
        }
    }
}
