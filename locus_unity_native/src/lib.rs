//! `locus_unity_native` — the in-process Unity Editor broker DLL.
//!
//! Locus talks to the Unity Editor over a Windows named pipe served from this
//! DLL, which Unity loads via P/Invoke and keeps loaded across domain reloads.
//! The generic broker (pipe server, request queue, lifecycle, heartbeat,
//! execution registers, state plane, main-thread stack capture) lives in the
//! shared `locus-native-broker` crate; this crate configures it for Unity and
//! adds the Unity-only parts:
//!
//!   * the recompile result register (`get_compile_result` answered natively),
//!   * the in-process background hook,
//!   * the overlay control client,
//!   * WebView2 embedding and GPU output,
//!   * Mono frame naming for stack captures.
//!
//! The exported symbol list is pinned by `exports.txt` (checked after every
//! build by `scripts/build-locus-unity-native.mjs`). The DLL was named
//! `locus_native.dll` before the crate split; names, signatures, the pipe
//! name and the state plane layout are unchanged.

use std::os::raw::c_int;

use locus_native_broker::ffi::{slice_from_raw, string_from_raw};
use locus_native_broker::host::{ExecutionCommands, HostConfig};
use locus_native_broker::protocol::{capabilities, errors, host_kind, names};

#[cfg(windows)]
mod gpu_output;

#[cfg(windows)]
mod mono_names;

/// Unity request types that run snippets through the managed executor.
const UNITY_EXECUTION: ExecutionCommands = ExecutionCommands {
    execute: &["execute_code", "execute_loaded"],
    wait: "execute_code_wait",
    cancel: "cancel_execute_code",
    progress: "execute_code_progress",
};

const UNITY_EXTRA_CAPABILITIES: &[&str] = &[capabilities::COMPILE_RESULT_V1];

/// Configuration for `locus_init(project, pipe, protocol)`.
fn unity_host_config(project: String, pipe: String, protocol_version: i32) -> Option<HostConfig> {
    Some(HostConfig {
        host_kind: host_kind::UNITY,
        instance_key: names::project_key(&project),
        state_plane_name: names::unity_state_plane_name(&project),
        pipe_name: names::normalize_pipe_name(&pipe),
        project,
        protocol_version,
        quitting_error: errors::UNITY_PROCESS_EXITING,
        default_status: "editing",
        extra_capabilities: UNITY_EXTRA_CAPABILITIES,
        execution: UNITY_EXECUTION,
        request_hook: Some(compile_result::answer),
        status_extras: Some(unity_status_extras),
        on_shutdown: Some(unity_on_shutdown),
        #[cfg(windows)]
        managed_namer: Some(&mono_names::MONO_NAMER),
        #[cfg(not(windows))]
        managed_namer: None,
    })
}

fn unity_status_extras(map: &mut serde_json::Map<String, serde_json::Value>) {
    #[cfg(windows)]
    {
        let (background_patched, background_symbols) = hook::snapshot();
        map.insert("backgroundPatched".into(), background_patched.into());
        map.insert("backgroundSymbols".into(), background_symbols.into());
        map.insert("overlayConnected".into(), overlay::connected().into());
    }
    #[cfg(not(windows))]
    let _ = map;
}

fn unity_on_shutdown() {
    #[cfg(windows)]
    {
        let _ = hook::set_active(false);
        overlay::shutdown();
    }
}

locus_native_broker::export_common_ffi!(unity_host_config);

/// Mirror of the managed recompile register `(operation id, result)`. The
/// managed side publishes every change; once published, the broker answers
/// `get_compile_result` itself so a compile, import or post-reload
/// initializer that holds the main thread cannot hide the result. Plugins
/// that never publish keep the managed path.
mod compile_result {
    use std::sync::Mutex;

    use locus_native_broker::host::{HostReply, HostRequest};

    pub(crate) const NO_ACTIVE_RECOMPILE_ERROR: &str = "Unity 没有开始编译。未找到活动重编译请求。";

    static REGISTER: Mutex<Option<(String, String)>> = Mutex::new(None);

    pub(crate) fn set(operation_id: String, result: String) {
        if let Ok(mut guard) = REGISTER.lock() {
            *guard = Some((operation_id.trim().to_string(), result));
        }
    }

    /// Answer `get_compile_result` exactly like the managed handler
    /// (`LocusBridge.cs`, case "get_compile_result"), without the editor main
    /// thread. Terminal results stay readable (last-value register); the next
    /// `request_recompile` publishes "starting".
    pub(crate) fn answer(request: &HostRequest<'_>) -> Option<HostReply> {
        if request.kind != "get_compile_result" {
            return None;
        }
        let (active_operation_id, result) = REGISTER.lock().ok()?.clone()?;
        let requested = request.message.trim();
        if !requested.is_empty() && requested != active_operation_id {
            return Some(Err("recompile_result_operation_mismatch".to_string()));
        }
        Some(match result.as_str() {
            "" => Err(NO_ACTIVE_RECOMPILE_ERROR.to_string()),
            "awaiting_reload" => Ok(Some("pending".to_string())),
            error if error.starts_with("error:") => Err(error["error:".len()..].to_string()),
            _ => Ok(Some(result.clone())),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn ask(message: &str) -> Option<HostReply> {
            answer(&HostRequest {
                id: "r",
                kind: "get_compile_result",
                message,
                raw: b"{}",
            })
        }

        #[test]
        fn compile_result_is_answered_from_the_published_register() {
            // Older managed plugins never publish: keep the managed path.
            assert!(ask("op-1").is_none());
            assert!(answer(&HostRequest { id: "r", kind: "ping", message: "", raw: b"{}" }).is_none());

            set("op-1".into(), "awaiting_reload".into());
            assert_eq!(ask("op-1"), Some(Ok(Some("pending".to_string()))));
            set("op-1".into(), "ok".into());
            assert_eq!(ask("op-1"), Some(Ok(Some("ok".to_string()))));
            assert_eq!(
                ask("op-2"),
                Some(Err("recompile_result_operation_mismatch".to_string()))
            );
            set("op-3".into(), "error:CS1002: ; expected".into());
            assert_eq!(ask(""), Some(Err("CS1002: ; expected".to_string())));
            set("op-4".into(), String::new());
            assert_eq!(ask("op-4"), Some(Err(NO_ACTIVE_RECOMPILE_ERROR.to_string())));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// In-process background hook (migration Phase 6)
// ─────────────────────────────────────────────────────────────────────────────
//
// Unity stops pumping the editor loop when its window is not the foreground OS
// app (`IsApplicationActive` / `IsApplicationActiveOSImpl` return 0). Locus
// needs the editor to keep ticking while the Locus window has focus, so those
// two engine functions are patched to unconditionally return 1.
//
// The Tauri side already does this cross-process (OpenProcess +
// WriteProcessMemory; see `unity_bridge/background_hook.rs`). Doing it *here* —
// inside the Unity process, from the broker DLL that persists across domain
// reloads — means the patch lives in native engine memory: it survives domain
// reloads with no re-sync and needs no cross-process memory rights. The managed
// side calls `locus_set_background_active(1)` when the feature marker is present;
// Tauri only falls back to its cross-process patch when this path is inactive,
// so the in-process hook is a pure optimization that fails open.
//
// The symbol list and patch bytes MUST stay identical to
// `background_hook.rs` (a parity test pins this).

#[cfg(windows)]
mod hook {
    use std::ffi::{c_void, CString, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use std::sync::{Mutex, OnceLock};

    type Bool = i32;
    type Dword = u32;
    type Dword64 = u64;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const TH32CS_SNAPMODULE: Dword = 0x0000_0008;
    const TH32CS_SNAPMODULE32: Dword = 0x0000_0010;
    const MAX_MODULE_NAME32: usize = 255;
    const MAX_PATH: usize = 260;
    const PAGE_EXECUTE_READWRITE: Dword = 0x40;
    const SYMOPT_UNDNAME: Dword = 0x0000_0002;
    const SYMOPT_DEFERRED_LOADS: Dword = 0x0000_0004;
    const SYMOPT_FAIL_CRITICAL_ERRORS: Dword = 0x0000_0200;
    const SYMOPT_AUTO_PUBLICS: Dword = 0x0001_0000;
    const MAX_SYM_NAME: usize = 2048;

    /// `mov eax, 1; ret` — the patched function unconditionally reports "active".
    /// MUST match `background_hook.rs::PATCH_BYTES`.
    const PATCH_BYTES: [u8; 6] = [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3];
    /// MUST match `background_hook.rs::SYMBOLS`.
    const SYMBOLS: [&str; 2] = [
        "Unity!IsApplicationActive",
        "Unity!IsApplicationActiveOSImpl",
    ];

    /// A dbghelp "process" key private to this module. Resolution is
    /// non-invasive (`fInvadeProcess = FALSE` plus an explicit
    /// `SymLoadModuleExW` base/size), so the key need not be a real process
    /// handle — using a private sentinel instead of `GetCurrentProcess()`
    /// guarantees our `SymInitialize`/`SymCleanup` never tears down a dbghelp
    /// session Unity itself opened on the real current-process handle.
    const SYM_HANDLE: Handle = 0x4C4F_4355 as Handle; // 'LOCU'

    #[allow(non_snake_case)]
    #[repr(C)]
    struct ModuleEntry32W {
        dwSize: Dword,
        th32ModuleID: Dword,
        th32ProcessID: Dword,
        GlblcntUsage: Dword,
        ProccntUsage: Dword,
        modBaseAddr: *mut u8,
        modBaseSize: Dword,
        hModule: Handle,
        szModule: [u16; MAX_MODULE_NAME32 + 1],
        szExePath: [u16; MAX_PATH],
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfo {
        SizeOfStruct: Dword,
        TypeIndex: Dword,
        Reserved: [Dword64; 2],
        Index: Dword,
        Size: Dword,
        ModBase: Dword64,
        Flags: Dword,
        Value: Dword64,
        Address: Dword64,
        Register: Dword,
        Scope: Dword,
        Tag: Dword,
        NameLen: Dword,
        MaxNameLen: Dword,
        Name: [u8; 1],
    }

    #[allow(non_snake_case)]
    #[repr(C)]
    struct SymbolInfoBuffer {
        Symbol: SymbolInfo,
        NameBuffer: [u8; MAX_SYM_NAME],
    }

    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn SymInitializeW(
            hProcess: Handle,
            UserSearchPath: *const u16,
            fInvadeProcess: Bool,
        ) -> Bool;
        fn SymSetOptions(SymOptions: Dword) -> Dword;
        fn SymLoadModuleExW(
            hProcess: Handle,
            hFile: Handle,
            ImageName: *const u16,
            ModuleName: *const u16,
            BaseOfDll: Dword64,
            DllSize: Dword,
            Data: *mut c_void,
            Flags: Dword,
        ) -> Dword64;
        fn SymFromName(hProcess: Handle, Name: *const u8, Symbol: *mut SymbolInfo) -> Bool;
        fn SymCleanup(hProcess: Handle) -> Bool;
    }

    unsafe extern "system" {
        fn CloseHandle(hObject: Handle) -> Bool;
        fn CreateToolhelp32Snapshot(dwFlags: Dword, th32ProcessID: Dword) -> Handle;
        fn Module32FirstW(hSnapshot: Handle, lpme: *mut ModuleEntry32W) -> Bool;
        fn Module32NextW(hSnapshot: Handle, lpme: *mut ModuleEntry32W) -> Bool;
        fn GetCurrentProcess() -> Handle;
        fn GetCurrentProcessId() -> Dword;
        fn VirtualProtect(
            lpAddress: *mut c_void,
            dwSize: usize,
            flNewProtect: Dword,
            lpflOldProtect: *mut Dword,
        ) -> Bool;
        fn FlushInstructionCache(
            hProcess: Handle,
            lpBaseAddress: *const c_void,
            dwSize: usize,
        ) -> Bool;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn new(handle: Handle) -> Result<Self, String> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                Err(last_error("handle open"))
            } else {
                Ok(Self(handle))
            }
        }
        fn raw(&self) -> Handle {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    struct SymSession;

    impl SymSession {
        fn new(search_path: &Path) -> Result<Self, String> {
            let search = wide_null(search_path.as_os_str());
            unsafe {
                SymSetOptions(
                    SYMOPT_UNDNAME
                        | SYMOPT_DEFERRED_LOADS
                        | SYMOPT_FAIL_CRITICAL_ERRORS
                        | SYMOPT_AUTO_PUBLICS,
                );
                if SymInitializeW(SYM_HANDLE, search.as_ptr(), FALSE) == 0 {
                    return Err(last_error("SymInitializeW"));
                }
            }
            Ok(Self)
        }
    }

    impl Drop for SymSession {
        fn drop(&mut self) {
            unsafe {
                let _ = SymCleanup(SYM_HANDLE);
            }
        }
    }

    struct ModuleInfo {
        base: u64,
        size: u32,
        path: String,
    }

    struct PatchRecord {
        address: u64,
        original: [u8; PATCH_BYTES.len()],
        /// True when *we* wrote the patch (so restore should put the original
        /// back). False when the bytes were already patched on entry.
        applied: bool,
    }

    #[derive(Default)]
    struct HookState {
        records: Vec<PatchRecord>,
        symbol_count: u32,
        last_error: Option<String>,
    }

    fn state() -> &'static Mutex<HookState> {
        static STATE: OnceLock<Mutex<HookState>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(HookState::default()))
    }

    /// `(patched, symbol_count)` for the broker status JSON.
    pub fn snapshot() -> (bool, u32) {
        state()
            .lock()
            .map(|s| (!s.records.is_empty(), s.symbol_count))
            .unwrap_or((false, 0))
    }

    /// Apply (or restore) the in-process background patch. Idempotent: a second
    /// `set_active(true)` returns the cached patch state without resolving PDB
    /// symbols or touching code pages again.
    pub fn set_active(active: bool) -> Result<u32, String> {
        let mut st = state()
            .lock()
            .map_err(|e| format!("background hook state poisoned: {e}"))?;
        if !active {
            restore_locked(&mut st);
            st.last_error = None;
            return Ok(0);
        }
        if !st.records.is_empty() {
            st.last_error = None;
            return Ok(st.symbol_count);
        }

        st.records.clear();
        match apply_all() {
            Ok(records) => {
                st.symbol_count = records.len() as u32;
                st.records = records;
                st.last_error = None;
                Ok(st.symbol_count)
            }
            Err(error) => {
                st.symbol_count = 0;
                st.last_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn restore_locked(st: &mut HookState) {
        for record in st.records.drain(..).rev() {
            if record.applied {
                let _ = write_code(record.address, &record.original);
            }
        }
        st.symbol_count = 0;
    }

    fn apply_all() -> Result<Vec<PatchRecord>, String> {
        let module = find_unity_engine_module()?;
        let image_path = Path::new(&module.path);
        let symbol_dir = image_path.parent().ok_or_else(|| {
            format!(
                "Unity engine module has no parent directory: {}",
                module.path
            )
        })?;
        let pdb_path = symbol_dir.join("unity_x64.pdb");
        if !pdb_path.is_file() {
            return Err(format!("Unity PDB is missing: {}", pdb_path.display()));
        }

        let mut records: Vec<PatchRecord> = Vec::new();
        for symbol in SYMBOLS {
            let address = match resolve_symbol(image_path, symbol_dir, &module, symbol) {
                Ok(address) => address,
                Err(error) => {
                    rollback(&records);
                    return Err(error);
                }
            };
            if address < module.base || address >= module.base.saturating_add(module.size as u64) {
                rollback(&records);
                return Err(format!(
                    "Resolved symbol {symbol} outside Unity module: 0x{address:X}"
                ));
            }

            let original = read_code(address);
            let already_patched = original.starts_with(&PATCH_BYTES);
            if !already_patched {
                if let Err(error) = write_code(address, &PATCH_BYTES) {
                    rollback(&records);
                    return Err(error);
                }
            }
            records.push(PatchRecord {
                address,
                original,
                applied: !already_patched,
            });
        }
        Ok(records)
    }

    fn rollback(records: &[PatchRecord]) {
        for record in records.iter().rev() {
            if record.applied {
                let _ = write_code(record.address, &record.original);
            }
        }
    }

    fn find_unity_engine_module() -> Result<ModuleInfo, String> {
        let pid = unsafe { GetCurrentProcessId() };
        let snapshot =
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) };
        let snapshot = OwnedHandle::new(snapshot)
            .map_err(|error| format!("Failed to snapshot own modules (PID {pid}): {error}"))?;

        let mut entry: ModuleEntry32W = unsafe { zeroed() };
        entry.dwSize = size_of::<ModuleEntry32W>() as u32;
        let mut has_entry = unsafe { Module32FirstW(snapshot.raw(), &mut entry) != 0 };
        let mut exe_module = None;
        while has_entry {
            let name = wide_to_string(&entry.szModule);
            if name.eq_ignore_ascii_case("Unity.dll") {
                return Ok(module_info_from_entry(&entry));
            }
            if name.eq_ignore_ascii_case("Unity.exe") {
                exe_module = Some(module_info_from_entry(&entry));
            }
            has_entry = unsafe { Module32NextW(snapshot.raw(), &mut entry) != 0 };
        }
        exe_module.ok_or_else(|| "Unity engine module was not found in this process".to_string())
    }

    fn module_info_from_entry(entry: &ModuleEntry32W) -> ModuleInfo {
        ModuleInfo {
            base: entry.modBaseAddr as usize as u64,
            size: entry.modBaseSize,
            path: wide_to_string(&entry.szExePath),
        }
    }

    fn resolve_symbol(
        image_path: &Path,
        symbol_path: &Path,
        module: &ModuleInfo,
        symbol_name: &str,
    ) -> Result<u64, String> {
        // Held for its Drop (SymCleanup); resolution uses the SYM_HANDLE const.
        let _session = SymSession::new(symbol_path)?;
        let image = wide_null(image_path.as_os_str());
        let module_name = wide_null(OsStr::new("Unity"));
        let loaded = unsafe {
            SymLoadModuleExW(
                SYM_HANDLE,
                null_mut(),
                image.as_ptr(),
                module_name.as_ptr(),
                module.base,
                module.size,
                null_mut(),
                0,
            )
        };
        if loaded == 0 {
            return Err(last_error("SymLoadModuleExW"));
        }

        let mut storage: Box<SymbolInfoBuffer> = unsafe { Box::new(zeroed()) };
        let symbol = &mut storage.Symbol as *mut SymbolInfo;
        unsafe {
            (*symbol).SizeOfStruct = size_of::<SymbolInfo>() as u32;
            (*symbol).MaxNameLen = MAX_SYM_NAME as u32;
        }
        let name = CString::new(symbol_name)
            .map_err(|_| format!("Symbol name contains interior nul: {symbol_name}"))?;
        let ok = unsafe { SymFromName(SYM_HANDLE, name.as_ptr() as *const u8, symbol) != 0 };
        if !ok {
            return Err(format!(
                "Failed to resolve Unity symbol {symbol_name}: {}",
                last_error("SymFromName")
            ));
        }
        Ok(unsafe { (*symbol).Address })
    }

    /// Read the leading `PATCH_BYTES.len()` bytes of an in-process code address.
    fn read_code(address: u64) -> [u8; PATCH_BYTES.len()] {
        unsafe { std::ptr::read_unaligned(address as usize as *const [u8; PATCH_BYTES.len()]) }
    }

    /// Overwrite an in-process code address, flipping page protection around the
    /// write and flushing the instruction cache. Own address space, so no
    /// cross-process memory APIs are needed.
    fn write_code(address: u64, bytes: &[u8]) -> Result<(), String> {
        let ptr = address as usize as *mut c_void;
        let mut old_protect = 0u32;
        let unprotected = unsafe {
            VirtualProtect(ptr, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old_protect) != 0
        };
        if !unprotected {
            return Err(format!(
                "Failed to unprotect Unity code at 0x{address:X}: {}",
                last_error("VirtualProtect")
            ));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), address as usize as *mut u8, bytes.len());
            let mut ignored = 0u32;
            let _ = FlushInstructionCache(GetCurrentProcess(), ptr, bytes.len());
            let _ = VirtualProtect(ptr, bytes.len(), old_protect, &mut ignored);
        }
        Ok(())
    }

    fn last_error(label: &str) -> String {
        format!("{label}: {}", std::io::Error::last_os_error())
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn wide_to_string(value: &[u16]) -> String {
        let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..len])
    }

    #[cfg(test)]
    mod tests {
        use super::{PATCH_BYTES, SYMBOLS};

        /// Parity with `unity_bridge/background_hook.rs`: the cross-process and
        /// in-process patches must touch the same symbols with the same bytes.
        #[test]
        fn patch_constants_match_cross_process_hook() {
            assert_eq!(PATCH_BYTES, [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3]);
            assert_eq!(
                SYMBOLS,
                [
                    "Unity!IsApplicationActive",
                    "Unity!IsApplicationActiveOSImpl",
                ]
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Overlay control client (migration Phase 5)
// ─────────────────────────────────────────────────────────────────────────────
//
// The Locus overlay is positioned by control messages the Unity editor pushes
// to a Tauri-hosted pipe (`\\.\pipe\locus_tauri_unity_embed_{proj}`). Today the
// managed `LocusEditorWindow` owns that client connection, so it drops on every
// domain reload and the overlay flickers/desyncs until the new domain
// reconnects. Here the broker DLL owns a *write-only* client that persists
// across reloads: the managed side still computes all geometry and builds the
// same JSON, but pushes it through `locus_overlay_push` instead of owning the
// pipe. On a (re)connect the last open/update line is replayed so the overlay
// re-syncs immediately, and because the connection never drops on reload the
// managed side can send a `managedOverlayState=reloading` message that the Tauri
// server uses to hold the overlay in place. Fails open: when the broker or this
// client is unavailable the managed side keeps using its own pipe client.

#[cfg(windows)]
mod overlay {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use tokio::io::AsyncWriteExt;
    use tokio::net::windows::named_pipe::ClientOptions;
    use tokio::sync::{mpsc, Notify};

    const OVERLAY_WRITER_CHANNEL_LIMIT: usize = 256;

    struct OverlayClient {
        pipe_name: String,
        shutdown: AtomicBool,
        shutdown_notify: Notify,
        connected: AtomicBool,
        /// Writer channel for the current connection; `None` while disconnected.
        tx: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
        /// The last non-`close` control line (no trailing newline). Replayed
        /// right after a (re)connect so a freshly (re)started Tauri overlay
        /// server re-syncs the overlay without waiting for the next push.
        last_sync: Mutex<Option<Vec<u8>>>,
    }

    static OVERLAY: OnceLock<Arc<OverlayClient>> = OnceLock::new();

    fn frame(mut line: Vec<u8>) -> Vec<u8> {
        line.push(b'\n');
        line
    }

    /// A `close` message ends the overlay session and must not be replayed on a
    /// later reconnect. Everything else (`open`/`update`/reloading markers) is
    /// the current desired state and is cached for replay.
    fn is_close(line: &[u8]) -> bool {
        // Managed JSON is produced by `JsonUtility.ToJson` (no spaces), so this
        // exact-substring match is stable.
        line.windows(14).any(|w| w == br#""type":"close""#)
    }

    /// Idempotent: a second call (e.g. after a domain reload) keeps the live
    /// connection. Returns 0 on success, -1 on a bad pipe name.
    pub fn connect(pipe_name: String) -> i32 {
        if pipe_name.trim().is_empty() {
            return -1;
        }
        // The managed caller hands us the bare control-pipe name (no prefix),
        // but `ClientOptions::open` needs the full `\\.\pipe\…` path. Normalize
        // exactly like the broker server so the client actually connects instead
        // of looping forever — otherwise pushes are silently buffered and the
        // overlay never mounts (the editor placeholder stays up).
        let pipe_name = locus_native_broker::normalize_pipe_name(&pipe_name);
        if OVERLAY.get().is_some() {
            return 0;
        }
        let client = Arc::new(OverlayClient {
            pipe_name: pipe_name.clone(),
            shutdown: AtomicBool::new(false),
            shutdown_notify: Notify::new(),
            connected: AtomicBool::new(false),
            tx: Mutex::new(None),
            last_sync: Mutex::new(None),
        });
        if OVERLAY.set(client.clone()).is_err() {
            return 0;
        }
        let spawn = std::thread::Builder::new()
            .name("locus-native-overlay".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("[locus-native] overlay runtime build failed: {e}");
                        return;
                    }
                };
                runtime.block_on(client_loop(client));
            });
        match spawn {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[locus-native] failed to spawn overlay thread: {e}");
                -1
            }
        }
    }

    /// Forward a control line to the Tauri overlay server, caching it for replay
    /// unless it is a `close`. No-op (the line is still cached) when the client
    /// is momentarily disconnected — the cached line is replayed on reconnect.
    pub fn push(line: Vec<u8>) {
        let Some(client) = OVERLAY.get() else {
            return;
        };
        if is_close(&line) {
            if let Ok(mut guard) = client.last_sync.lock() {
                *guard = None;
            }
        } else if let Ok(mut guard) = client.last_sync.lock() {
            *guard = Some(line.clone());
        }
        if let Ok(guard) = client.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                match tx.try_send(frame(line)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        eprintln!(
                            "[locus-native] overlay writer queue full (limit {})",
                            OVERLAY_WRITER_CHANNEL_LIMIT
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {}
                }
            }
        }
    }

    pub fn shutdown() {
        if let Some(client) = OVERLAY.get() {
            client.shutdown.store(true, Ordering::SeqCst);
            client.shutdown_notify.notify_waiters();
        }
    }

    pub fn connected() -> bool {
        OVERLAY
            .get()
            .map(|c| c.connected.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    async fn client_loop(client: Arc<OverlayClient>) {
        eprintln!(
            "[locus-native] overlay client targeting {}",
            client.pipe_name
        );
        loop {
            if client.shutdown.load(Ordering::SeqCst) {
                break;
            }
            match ClientOptions::new().open(&client.pipe_name) {
                Ok(pipe) => serve(&client, pipe).await,
                Err(_) => {
                    tokio::select! {
                        _ = client.shutdown_notify.notified() => break,
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    }
                }
            }
        }
        eprintln!("[locus-native] overlay client loop exited");
    }

    async fn serve(
        client: &Arc<OverlayClient>,
        mut pipe: tokio::net::windows::named_pipe::NamedPipeClient,
    ) {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OVERLAY_WRITER_CHANNEL_LIMIT);
        if let Ok(mut guard) = client.tx.lock() {
            *guard = Some(tx.clone());
        }
        client.connected.store(true, Ordering::SeqCst);

        // Replay the current overlay geometry so the (re)connected server syncs
        // immediately rather than waiting for the next geometry change.
        if let Some(last) = client.last_sync.lock().ok().and_then(|g| g.clone()) {
            let _ = tx.send(frame(last));
        }

        loop {
            let next = tokio::select! {
                _ = client.shutdown_notify.notified() => break,
                msg = rx.recv() => msg,
            };
            let Some(bytes) = next else { break };
            if pipe.write_all(&bytes).await.is_err() {
                break;
            }
            if pipe.flush().await.is_err() {
                break;
            }
        }

        client.connected.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = client.tx.lock() {
            *guard = None;
        }
    }

    #[cfg(test)]
    mod tests {
        use super::is_close;

        #[test]
        fn is_close_matches_only_close_messages() {
            assert!(is_close(br#"{"type":"close","reason":"windowClosed"}"#));
            assert!(!is_close(br#"{"type":"update","x":1,"y":2}"#));
            assert!(!is_close(br#"{"type":"open","visible":true}"#));
            assert!(!is_close(b""));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Unity-only C ABI. The common entry points come from `export_common_ffi!`.
// ─────────────────────────────────────────────────────────────────────────────

/// Publish the managed recompile register (operation id + result string, as
/// stored in `SessionState`). Called on every change; after the first call the
/// broker answers `get_compile_result` without the editor main thread.
///
/// # Safety
/// `operation_id`/`result` must each be valid for their stated lengths or null.
#[no_mangle]
pub unsafe extern "C" fn locus_set_compile_result(
    operation_id: *const u8,
    operation_id_len: i32,
    result: *const u8,
    result_len: i32,
) {
    compile_result::set(
        string_from_raw(operation_id, operation_id_len),
        string_from_raw(result, result_len),
    );
}

/// Apply (`active != 0`) or restore (`active == 0`) the in-process background
/// hook that keeps the Unity editor ticking while its window is not focused.
/// Returns the number of patched engine symbols on success, or `-1` on failure.
/// On failure the managed caller leaves the cross-process Tauri hook to do the
/// patch instead — this path fails open and is a pure optimization.
///
/// # Safety
/// Takes no pointers; safe to call from any managed (editor) thread.
#[no_mangle]
pub extern "C" fn locus_set_background_active(active: c_int) -> c_int {
    #[cfg(windows)]
    {
        match hook::set_active(active != 0) {
            Ok(count) => count as c_int,
            Err(error) => {
                eprintln!("[locus-native] background hook set_active failed: {error}");
                -1
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = active;
        -1
    }
}

/// Open (idempotently) the persistent write-only client to the Tauri overlay
/// control pipe. The connection survives domain reloads, so the managed side
/// stops owning it. Returns 0 on success, -1 on a bad pipe name / spawn failure.
///
/// # Safety
/// `pipe` must be valid for `pipe_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_overlay_connect(pipe: *const u8, pipe_len: i32) -> c_int {
    #[cfg(windows)]
    {
        overlay::connect(string_from_raw(pipe, pipe_len))
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe, pipe_len);
        -1
    }
}

/// Forward one overlay control line (a full `EmbedControlMessage` JSON, no
/// trailing newline) to the Tauri overlay server, caching it for replay on the
/// next reconnect unless it is a `close`. No-op when the overlay client is not
/// connected (the cached line is replayed once it reconnects).
///
/// # Safety
/// `line` must be valid for `line_len` bytes or null.
#[no_mangle]
pub unsafe extern "C" fn locus_overlay_push(line: *const u8, line_len: i32) {
    #[cfg(windows)]
    {
        let bytes = slice_from_raw(line, line_len).to_vec();
        if !bytes.is_empty() {
            overlay::push(bytes);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (line, line_len);
    }
}

#[cfg(windows)]
mod webview;
