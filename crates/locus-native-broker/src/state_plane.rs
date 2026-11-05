//! Win32 named shared-memory writer for the state plane. The byte layout and
//! the slot protocol live in `locus_native_protocol::state_plane`.

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

use locus_native_protocol::state_plane as layout;

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const PAGE_READWRITE: Dword = 0x04;
const FILE_MAP_WRITE: Dword = 0x0002;

unsafe extern "system" {
    fn CreateFileMappingW(
        hFile: Handle,
        lpFileMappingAttributes: *mut c_void,
        flProtect: Dword,
        dwMaximumSizeHigh: Dword,
        dwMaximumSizeLow: Dword,
        lpName: *const u16,
    ) -> Handle;
    fn MapViewOfFile(
        hFileMappingObject: Handle,
        dwDesiredAccess: Dword,
        dwFileOffsetHigh: Dword,
        dwFileOffsetLow: Dword,
        dwNumberOfBytesToMap: usize,
    ) -> *mut c_void;
    fn UnmapViewOfFile(lpBaseAddress: *const c_void) -> Bool;
    fn CloseHandle(hObject: Handle) -> Bool;
}

pub(crate) struct StatePlane {
    handle: Handle,
    view: *mut u8,
    total_size: usize,
}

unsafe impl Send for StatePlane {}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

impl StatePlane {
    pub(crate) fn create(name: &str) -> Option<Self> {
        if name.trim().is_empty() {
            return None;
        }
        let total_size = layout::total_size();
        let wide_name = wide_null(name);
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                null_mut(),
                PAGE_READWRITE,
                0,
                total_size.min(u32::MAX as usize) as Dword,
                wide_name.as_ptr(),
            )
        };
        if handle.is_null() {
            eprintln!("[locus-native] failed to create state plane: {name}");
            return None;
        }
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, total_size) as *mut u8 };
        if view.is_null() {
            unsafe {
                let _ = CloseHandle(handle);
            }
            eprintln!("[locus-native] failed to map state plane: {name}");
            return None;
        }
        let mut plane = Self {
            handle,
            view,
            total_size,
        };
        let now = crate::broker::now_ms().max(0) as u64;
        layout::write_header(plane.bytes_mut(), std::process::id(), now);
        Some(plane)
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // The mapping is exclusively written by this broker; readers in other
        // processes validate sequences around every copy (torn-read safe).
        unsafe { std::slice::from_raw_parts_mut(self.view, self.total_size) }
    }

    pub(crate) fn write_json(&mut self, seq: u64, observed_at_ms: i64, json: &str) {
        let bytes = json.as_bytes();
        if bytes.is_empty() || bytes.len() > layout::MAX_PAYLOAD {
            eprintln!(
                "[locus-native] state plane payload too large: {} bytes (max {})",
                bytes.len(),
                layout::MAX_PAYLOAD
            );
            return;
        }
        let observed = observed_at_ms.max(0) as u64;
        layout::write_slot(self.bytes_mut(), seq.max(1), observed, layout::SOURCE_BROKER, bytes);
    }
}

impl Drop for StatePlane {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view as *const c_void);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
impl StatePlane {
    /// # Safety
    /// Test-only read of the mapped view.
    pub(crate) unsafe fn bytes_for_test(&self) -> &[u8] {
        std::slice::from_raw_parts(self.view, self.total_size)
    }
}
