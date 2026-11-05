//! Process facts for Blender instances: enumeration by image name, identity
//! (PID + creation time, so a reused PID is never mistaken for an instance)
//! and identity-checked termination of Locus-owned background instances.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub image_path: Option<String>,
    pub created_at_ms: Option<u64>,
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use super::ProcessInfo;

    type Bool = i32;
    type Dword = u32;
    type Handle = *mut c_void;

    const FALSE: Bool = 0;
    const PROCESS_QUERY_LIMITED_INFORMATION: Dword = 0x1000;
    const PROCESS_TERMINATE: Dword = 0x0001;
    const SYNCHRONIZE: Dword = 0x0010_0000;
    const STILL_ACTIVE: Dword = 259;

    #[repr(C)]
    struct Filetime {
        low: Dword,
        high: Dword,
    }

    unsafe extern "system" {
        fn OpenProcess(access: Dword, inherit: Bool, process_id: Dword) -> Handle;
        fn CloseHandle(handle: Handle) -> Bool;
        fn QueryFullProcessImageNameW(process: Handle, flags: Dword, name: *mut u16, size: *mut Dword) -> Bool;
        fn GetProcessTimes(
            process: Handle,
            creation: *mut Filetime,
            exit: *mut Filetime,
            kernel: *mut Filetime,
            user: *mut Filetime,
        ) -> Bool;
        fn GetExitCodeProcess(process: Handle, code: *mut Dword) -> Bool;
        fn TerminateProcess(process: Handle, code: u32) -> Bool;
        fn WaitForSingleObject(handle: Handle, milliseconds: Dword) -> Dword;
    }

    struct OwnedHandle(Handle);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn open(pid: u32, access: Dword) -> Option<OwnedHandle> {
        let raw = unsafe { OpenProcess(access, FALSE, pid) };
        (!raw.is_null()).then_some(OwnedHandle(raw))
    }

    fn image_path(handle: &OwnedHandle) -> Option<String> {
        let mut buffer = vec![0u16; 32 * 1024];
        let mut size = buffer.len() as Dword;
        (unsafe { QueryFullProcessImageNameW(handle.0, 0, buffer.as_mut_ptr(), &mut size) } != 0)
            .then(|| String::from_utf16_lossy(&buffer[..size as usize]))
    }

    fn created_at_ms(handle: &OwnedHandle) -> Option<u64> {
        let mut creation = Filetime { low: 0, high: 0 };
        let mut exit = Filetime { low: 0, high: 0 };
        let mut kernel = Filetime { low: 0, high: 0 };
        let mut user = Filetime { low: 0, high: 0 };
        if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let ticks = ((creation.high as u64) << 32) | creation.low as u64;
        (ticks / 10_000).checked_sub(11_644_473_600_000)
    }

    fn alive(handle: &OwnedHandle) -> bool {
        let mut code: Dword = 0;
        unsafe { GetExitCodeProcess(handle.0, &mut code) != 0 && code == STILL_ACTIVE }
    }

    /// Toolhelp enumeration through the `windows` crate (other modules
    /// declare the same entry points; raw re-declarations would clash).
    pub fn list_named(image_name: &str) -> Vec<ProcessInfo> {
        use windows::Win32::Foundation::CloseHandle as CloseSnapshot;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return Vec::new();
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut out = Vec::new();
        let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(image_name) {
                let pid = entry.th32ProcessID;
                let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION);
                out.push(ProcessInfo {
                    pid,
                    image_path: handle.as_ref().and_then(image_path),
                    created_at_ms: handle.as_ref().and_then(created_at_ms),
                });
            }
            ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        unsafe {
            let _ = CloseSnapshot(snapshot);
        }
        out
    }

    pub fn identity(pid: u32) -> Option<ProcessInfo> {
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        if !alive(&handle) {
            return None;
        }
        Some(ProcessInfo {
            pid,
            image_path: image_path(&handle),
            created_at_ms: created_at_ms(&handle),
        })
    }

    pub fn terminate(pid: u32, expected_created_at_ms: u64, timeout_ms: u32) -> Result<(), String> {
        let handle = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | SYNCHRONIZE)
            .ok_or_else(|| format!("cannot open Blender process {pid}: {}", std::io::Error::last_os_error()))?;
        // Compare and terminate through one handle: PID reuse in between
        // cannot redirect the termination to another process.
        if created_at_ms(&handle) != Some(expected_created_at_ms) {
            return Err("Blender process identity changed; it was not terminated".to_string());
        }
        if unsafe { TerminateProcess(handle.0, 1) } == 0 {
            return Err(format!("failed to terminate Blender: {}", std::io::Error::last_os_error()));
        }
        unsafe {
            WaitForSingleObject(handle.0, timeout_ms);
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ProcessInfo;

    pub fn list_named(_image_name: &str) -> Vec<ProcessInfo> {
        Vec::new()
    }
    pub fn identity(_pid: u32) -> Option<ProcessInfo> {
        None
    }
    pub fn terminate(_pid: u32, _created: u64, _timeout_ms: u32) -> Result<(), String> {
        Err("unsupported platform".to_string())
    }
}

pub use imp::{identity, list_named, terminate};

/// The process `pid` is still the one started at `created_at_ms`.
pub fn is_same_process(pid: u32, created_at_ms: u64) -> bool {
    identity(pid).is_some_and(|info| match info.created_at_ms {
        // Registration files store the start time the process reported for
        // itself; allow the rounding difference between the two readers.
        Some(actual) => actual.abs_diff(created_at_ms) <= 2,
        None => false,
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn current_process_identity_is_stable() {
        let me = identity(std::process::id()).expect("own process");
        let created = me.created_at_ms.expect("creation time");
        assert!(is_same_process(std::process::id(), created));
        assert!(!is_same_process(std::process::id(), created + 10_000));
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_string();
        assert!(list_named(&name).iter().any(|p| p.pid == std::process::id()));
    }
}
