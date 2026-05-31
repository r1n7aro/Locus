use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use super::unix_now_ms;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityBackgroundHookState {
    Disabled,
    Inactive,
    Patched,
    Failed,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityBackgroundHookStatus {
    pub enabled: bool,
    pub supported: bool,
    pub state: UnityBackgroundHookState,
    pub patched: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_process_path: Option<String>,
    pub symbol_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub updated_at_ms: u64,
}

impl UnityBackgroundHookStatus {
    fn disabled() -> Self {
        Self {
            enabled: false,
            supported: cfg!(target_os = "windows"),
            state: UnityBackgroundHookState::Disabled,
            patched: false,
            process_id: None,
            editor_process_path: None,
            symbol_count: 0,
            error: None,
            updated_at_ms: unix_now_ms(),
        }
    }

    fn inactive(enabled: bool) -> Self {
        Self {
            enabled,
            supported: cfg!(target_os = "windows"),
            state: if cfg!(target_os = "windows") {
                UnityBackgroundHookState::Inactive
            } else {
                UnityBackgroundHookState::Unsupported
            },
            patched: false,
            process_id: None,
            editor_process_path: None,
            symbol_count: 0,
            error: if cfg!(target_os = "windows") {
                None
            } else {
                Some("Unity background hook is only supported on Windows".to_string())
            },
            updated_at_ms: unix_now_ms(),
        }
    }
}

#[derive(Debug, Clone)]
struct PatchRecord {
    symbol: &'static str,
    address: u64,
    original: Vec<u8>,
    managed_original: bool,
}

#[derive(Debug, Clone)]
struct ProcessPatch {
    process_id: u32,
    editor_process_path: String,
    records: Vec<PatchRecord>,
}

#[derive(Debug)]
struct HookRuntime {
    enabled: bool,
    patches: Vec<ProcessPatch>,
    last_status: UnityBackgroundHookStatus,
}

fn runtime() -> &'static Mutex<HookRuntime> {
    static RUNTIME: OnceLock<Mutex<HookRuntime>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        Mutex::new(HookRuntime {
            enabled: true,
            patches: Vec::new(),
            last_status: UnityBackgroundHookStatus::inactive(true),
        })
    })
}

pub fn initialize(enabled: bool) {
    let mut rt = runtime()
        .lock()
        .expect("unity background hook runtime poisoned");
    rt.enabled = enabled;
    rt.last_status = if enabled {
        UnityBackgroundHookStatus::inactive(true)
    } else {
        UnityBackgroundHookStatus::disabled()
    };
}

pub fn enabled() -> bool {
    runtime().lock().map(|rt| rt.enabled).unwrap_or(false)
}

pub fn status() -> UnityBackgroundHookStatus {
    runtime()
        .lock()
        .map(|rt| rt.last_status.clone())
        .unwrap_or_else(|_| UnityBackgroundHookStatus {
            enabled: false,
            supported: cfg!(target_os = "windows"),
            state: UnityBackgroundHookState::Failed,
            patched: false,
            process_id: None,
            editor_process_path: None,
            symbol_count: 0,
            error: Some("Unity background hook runtime is unavailable".to_string()),
            updated_at_ms: unix_now_ms(),
        })
}

pub fn set_enabled(value: bool) -> Result<UnityBackgroundHookStatus, String> {
    let mut rt = runtime()
        .lock()
        .map_err(|e| format!("unity background hook runtime lock poisoned: {e}"))?;
    rt.enabled = value;
    if !value {
        let restore_result = restore_all_locked(&mut rt);
        rt.last_status = UnityBackgroundHookStatus::disabled();
        restore_result?;
        return Ok(rt.last_status.clone());
    }
    rt.last_status = UnityBackgroundHookStatus::inactive(true);
    Ok(rt.last_status.clone())
}

pub fn restore_runtime_patches() -> Result<(), String> {
    let mut rt = runtime()
        .lock()
        .map_err(|e| format!("unity background hook runtime lock poisoned: {e}"))?;
    let restore_result = restore_all_locked(&mut rt);
    rt.last_status = if rt.enabled {
        UnityBackgroundHookStatus::inactive(true)
    } else {
        UnityBackgroundHookStatus::disabled()
    };
    restore_result
}

pub fn sync_for_process(
    process_id: u32,
    editor_process_path: &str,
) -> Result<UnityBackgroundHookStatus, String> {
    let mut rt = runtime()
        .lock()
        .map_err(|e| format!("unity background hook runtime lock poisoned: {e}"))?;

    if !rt.enabled {
        rt.last_status = UnityBackgroundHookStatus::disabled();
        return Ok(rt.last_status.clone());
    }

    let editor_process_path = editor_process_path.trim();
    if editor_process_path.is_empty() {
        rt.last_status = failed_status(
            true,
            Some(process_id),
            None,
            "Unity process path is unavailable".to_string(),
        );
        return Ok(rt.last_status.clone());
    }

    if let Some(existing) = rt.patches.iter().find(|patch| {
        patch.process_id == process_id && patch.editor_process_path == editor_process_path
    }) {
        rt.last_status = patched_status(existing);
        return Ok(rt.last_status.clone());
    }

    match patch_process(process_id, editor_process_path) {
        Ok(process_patch) => {
            let status = patched_status(&process_patch);
            rt.patches.push(process_patch);
            rt.last_status = status;
        }
        Err(error) => {
            rt.last_status = failed_status(
                true,
                Some(process_id),
                Some(editor_process_path.to_string()),
                error,
            );
        }
    }

    Ok(rt.last_status.clone())
}

fn patched_status(process_patch: &ProcessPatch) -> UnityBackgroundHookStatus {
    UnityBackgroundHookStatus {
        enabled: true,
        supported: cfg!(target_os = "windows"),
        state: UnityBackgroundHookState::Patched,
        patched: true,
        process_id: Some(process_patch.process_id),
        editor_process_path: Some(process_patch.editor_process_path.clone()),
        symbol_count: process_patch.records.len() as u32,
        error: None,
        updated_at_ms: unix_now_ms(),
    }
}

fn failed_status(
    enabled: bool,
    process_id: Option<u32>,
    editor_process_path: Option<String>,
    error: String,
) -> UnityBackgroundHookStatus {
    UnityBackgroundHookStatus {
        enabled,
        supported: cfg!(target_os = "windows"),
        state: if cfg!(target_os = "windows") {
            UnityBackgroundHookState::Failed
        } else {
            UnityBackgroundHookState::Unsupported
        },
        patched: false,
        process_id,
        editor_process_path,
        symbol_count: 0,
        error: Some(error),
        updated_at_ms: unix_now_ms(),
    }
}

fn restore_all_locked(rt: &mut HookRuntime) -> Result<(), String> {
    let mut errors = Vec::new();
    for process_patch in rt.patches.drain(..) {
        if let Err(error) = restore_process_patch(&process_patch) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(not(target_os = "windows"))]
fn patch_process(_process_id: u32, _editor_process_path: &str) -> Result<ProcessPatch, String> {
    Err("Unity background hook is only supported on Windows".to_string())
}

#[cfg(not(target_os = "windows"))]
fn restore_process_patch(_process_patch: &ProcessPatch) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::ffi::{c_void, CString, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;

    use super::{PatchRecord, ProcessPatch};

    type Bool = i32;
    type Dword = u32;
    type Dword64 = u64;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const TH32CS_SNAPMODULE: Dword = 0x00000008;
    const TH32CS_SNAPMODULE32: Dword = 0x00000010;
    const MAX_MODULE_NAME32: usize = 255;
    const MAX_PATH: usize = 260;
    const PROCESS_QUERY_INFORMATION: Dword = 0x0400;
    const PROCESS_VM_OPERATION: Dword = 0x0008;
    const PROCESS_VM_READ: Dword = 0x0010;
    const PROCESS_VM_WRITE: Dword = 0x0020;
    const PAGE_EXECUTE_READWRITE: Dword = 0x40;
    const SYMOPT_UNDNAME: Dword = 0x00000002;
    const SYMOPT_DEFERRED_LOADS: Dword = 0x00000004;
    const SYMOPT_FAIL_CRITICAL_ERRORS: Dword = 0x00000200;
    const SYMOPT_AUTO_PUBLICS: Dword = 0x00010000;
    const MAX_SYM_NAME: usize = 2048;
    const PATCH_BYTES: [u8; 6] = [0xB8, 0x01, 0x00, 0x00, 0x00, 0xC3];
    const SYMBOLS: [&str; 2] = [
        "Unity!IsApplicationActive",
        "Unity!IsApplicationActiveOSImpl",
    ];

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
        fn OpenProcess(dwDesiredAccess: Dword, bInheritHandle: Bool, dwProcessId: Dword) -> Handle;
        fn ReadProcessMemory(
            hProcess: Handle,
            lpBaseAddress: *const c_void,
            lpBuffer: *mut c_void,
            nSize: usize,
            lpNumberOfBytesRead: *mut usize,
        ) -> Bool;
        fn WriteProcessMemory(
            hProcess: Handle,
            lpBaseAddress: *mut c_void,
            lpBuffer: *const c_void,
            nSize: usize,
            lpNumberOfBytesWritten: *mut usize,
        ) -> Bool;
        fn VirtualProtectEx(
            hProcess: Handle,
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

    struct SymSession {
        handle: Handle,
    }

    impl SymSession {
        fn new(search_path: &Path) -> Result<Self, String> {
            let handle = -1isize as Handle;
            let search = wide_null(search_path.as_os_str());
            unsafe {
                SymSetOptions(
                    SYMOPT_UNDNAME
                        | SYMOPT_DEFERRED_LOADS
                        | SYMOPT_FAIL_CRITICAL_ERRORS
                        | SYMOPT_AUTO_PUBLICS,
                );
                if SymInitializeW(handle, search.as_ptr(), FALSE) == 0 {
                    return Err(last_error("SymInitializeW"));
                }
            }
            Ok(Self { handle })
        }
    }

    impl Drop for SymSession {
        fn drop(&mut self) {
            unsafe {
                let _ = SymCleanup(self.handle);
            }
        }
    }

    #[derive(Debug, Clone)]
    struct ModuleInfo {
        name: String,
        base: u64,
        size: u32,
        path: String,
    }

    pub(super) fn patch_process(
        process_id: u32,
        editor_process_path: &str,
    ) -> Result<ProcessPatch, String> {
        let image_path = Path::new(editor_process_path);
        if !image_path.is_file() {
            return Err(format!(
                "Unity executable is unavailable: {editor_process_path}"
            ));
        }

        let module = find_unity_engine_module(process_id)?;
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
        let process = open_target_process(process_id)?;
        let mut records = Vec::new();

        for symbol in SYMBOLS {
            let address = resolve_symbol(image_path, symbol_dir, &module, symbol)?;
            if address < module.base || address >= module.base.saturating_add(module.size as u64) {
                rollback_partial(process.raw(), &records);
                return Err(format!(
                    "Resolved symbol {symbol} outside {} module: 0x{address:X}",
                    module.name
                ));
            }

            let original = read_remote(process.raw(), address, 16)?;
            let already_patched = original.starts_with(&PATCH_BYTES);
            if !already_patched {
                if let Err(error) = write_remote(process.raw(), address, &PATCH_BYTES) {
                    rollback_partial(process.raw(), &records);
                    return Err(error);
                }
            }

            records.push(PatchRecord {
                symbol,
                address,
                original,
                managed_original: !already_patched,
            });
        }

        Ok(ProcessPatch {
            process_id,
            editor_process_path: editor_process_path.to_string(),
            records,
        })
    }

    pub(super) fn restore_process_patch(process_patch: &ProcessPatch) -> Result<(), String> {
        let process = open_target_process(process_patch.process_id)?;
        let mut errors = Vec::new();
        for record in process_patch.records.iter().rev() {
            if !record.managed_original {
                continue;
            }
            let len = PATCH_BYTES.len().min(record.original.len());
            if len == 0 {
                continue;
            }
            if let Err(error) = write_remote(process.raw(), record.address, &record.original[..len])
            {
                errors.push(format!("{}: {}", record.symbol, error));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn rollback_partial(process: Handle, records: &[PatchRecord]) {
        for record in records.iter().rev() {
            if !record.managed_original {
                continue;
            }
            let len = PATCH_BYTES.len().min(record.original.len());
            if len > 0 {
                let _ = write_remote(process, record.address, &record.original[..len]);
            }
        }
    }

    fn find_unity_engine_module(process_id: u32) -> Result<ModuleInfo, String> {
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
        };
        let snapshot = OwnedHandle::new(snapshot).map_err(|error| {
            format!("Failed to create module snapshot for Unity PID {process_id}: {error}")
        })?;

        let mut entry: ModuleEntry32W = unsafe { zeroed() };
        entry.dwSize = size_of::<ModuleEntry32W>() as u32;
        let mut has_entry = unsafe { Module32FirstW(snapshot.raw(), &mut entry) != 0 };
        let mut exe_module = None;
        while has_entry {
            let name = wide_to_string(&entry.szModule);
            if name.eq_ignore_ascii_case("Unity.dll") {
                return Ok(module_info_from_entry(name, &entry));
            }
            if name.eq_ignore_ascii_case("Unity.exe") {
                exe_module = Some(module_info_from_entry(name, &entry));
            }
            has_entry = unsafe { Module32NextW(snapshot.raw(), &mut entry) != 0 };
        }
        exe_module.ok_or_else(|| format!("Unity engine module was not found in PID {process_id}"))
    }

    fn module_info_from_entry(name: String, entry: &ModuleEntry32W) -> ModuleInfo {
        ModuleInfo {
            name,
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
        let session = SymSession::new(symbol_path)?;
        let image = wide_null(image_path.as_os_str());
        let module_name = wide_null(OsStr::new("Unity"));
        let loaded = unsafe {
            SymLoadModuleExW(
                session.handle,
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
        let ok = unsafe { SymFromName(session.handle, name.as_ptr() as *const u8, symbol) != 0 };
        if !ok {
            return Err(format!(
                "Failed to resolve Unity symbol {symbol_name}: {}",
                last_error("SymFromName")
            ));
        }

        let address = unsafe { (*symbol).Address };
        Ok(address)
    }

    fn open_target_process(process_id: u32) -> Result<OwnedHandle, String> {
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_INFORMATION
                    | PROCESS_VM_OPERATION
                    | PROCESS_VM_READ
                    | PROCESS_VM_WRITE,
                FALSE,
                process_id,
            )
        };
        OwnedHandle::new(handle)
            .map_err(|error| format!("Failed to open Unity PID {process_id}: {error}"))
    }

    fn read_remote(process: Handle, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0u8; length];
        let mut read = 0usize;
        let ok = unsafe {
            ReadProcessMemory(
                process,
                address as usize as *const c_void,
                bytes.as_mut_ptr() as *mut c_void,
                length,
                &mut read,
            ) != 0
        };
        if !ok || read != length {
            return Err(format!(
                "Failed to read Unity memory at 0x{address:X}: {}, read={read}",
                last_error("ReadProcessMemory")
            ));
        }
        Ok(bytes)
    }

    fn write_remote(process: Handle, address: u64, bytes: &[u8]) -> Result<(), String> {
        let mut old_protect = 0u32;
        let ok = unsafe {
            VirtualProtectEx(
                process,
                address as usize as *mut c_void,
                bytes.len(),
                PAGE_EXECUTE_READWRITE,
                &mut old_protect,
            ) != 0
        };
        if !ok {
            return Err(format!(
                "Failed to change Unity memory protection at 0x{address:X}: {}",
                last_error("VirtualProtectEx")
            ));
        }

        let mut written = 0usize;
        let write_ok = unsafe {
            WriteProcessMemory(
                process,
                address as usize as *mut c_void,
                bytes.as_ptr() as *const c_void,
                bytes.len(),
                &mut written,
            ) != 0
        };
        let write_error = last_error("WriteProcessMemory");
        unsafe {
            let mut ignored = 0u32;
            let _ = FlushInstructionCache(process, address as usize as *const c_void, bytes.len());
            let _ = VirtualProtectEx(
                process,
                address as usize as *mut c_void,
                bytes.len(),
                old_protect,
                &mut ignored,
            );
        }

        if !write_ok || written != bytes.len() {
            return Err(format!(
                "Failed to patch Unity memory at 0x{address:X}: {write_error}, written={written}"
            ));
        }
        Ok(())
    }

    fn last_error(label: &str) -> String {
        let error = std::io::Error::last_os_error();
        format!("{label}: {error}")
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn wide_to_string(value: &[u16]) -> String {
        let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..len])
    }
}

#[cfg(target_os = "windows")]
fn patch_process(process_id: u32, editor_process_path: &str) -> Result<ProcessPatch, String> {
    windows_impl::patch_process(process_id, editor_process_path)
}

#[cfg(target_os = "windows")]
fn restore_process_patch(process_patch: &ProcessPatch) -> Result<(), String> {
    windows_impl::restore_process_patch(process_patch)
}
