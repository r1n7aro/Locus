//! Security attributes for the broker pipe: only the current user (and
//! SYSTEM) may open it. Combined with `first_pipe_instance`, another process
//! can neither squat the name before the broker nor connect as someone else.

use std::ffi::c_void;
use std::ptr::null_mut;

type Bool = i32;
type Handle = *mut c_void;

const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const SDDL_REVISION_1: u32 = 1;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: Bool,
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> Bool;
    fn GetTokenInformation(token: Handle, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> Bool;
    fn ConvertSidToStringSidW(sid: *mut c_void, string_sid: *mut *mut u16) -> Bool;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> Bool;
}

unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn CloseHandle(handle: Handle) -> Bool;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

/// Owns the descriptor memory; keep it alive across `CreateNamedPipeW`.
pub(crate) struct PipeSecurity {
    attributes: SecurityAttributes,
}

unsafe impl Send for PipeSecurity {}

impl PipeSecurity {
    /// `None` when the user SID cannot be read; the caller then falls back to
    /// the default descriptor (same-session access) and still uses
    /// `first_pipe_instance`.
    pub(crate) fn current_user_only() -> Option<Self> {
        let sid = current_user_sid_string()?;
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor: *mut c_void = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return None;
        }
        Some(Self {
            attributes: SecurityAttributes {
                length: std::mem::size_of::<SecurityAttributes>() as u32,
                security_descriptor: descriptor,
                inherit_handle: 0,
            },
        })
    }

    pub(crate) fn as_raw(&mut self) -> *mut c_void {
        &mut self.attributes as *mut SecurityAttributes as *mut c_void
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(self.attributes.security_descriptor);
        }
    }
}

fn current_user_sid_string() -> Option<String> {
    unsafe {
        let mut token: Handle = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TOKEN_USER_CLASS, null_mut(), 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            return None;
        }
        // TOKEN_USER starts with SID_AND_ATTRIBUTES { PSID Sid; DWORD Attributes; }.
        let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            token,
            TOKEN_USER_CLASS,
            buffer.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        );
        let _ = CloseHandle(token);
        if ok == 0 {
            return None;
        }
        let sid = *(buffer.as_ptr() as *const *mut c_void);
        let mut string_sid: *mut u16 = null_mut();
        if ConvertSidToStringSidW(sid, &mut string_sid) == 0 || string_sid.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *string_sid.add(len) != 0 {
            len += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(string_sid, len));
        let _ = LocalFree(string_sid as *mut c_void);
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_current_user_descriptor() {
        let sid = current_user_sid_string().expect("current user SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
        let mut security = PipeSecurity::current_user_only().expect("descriptor");
        assert!(!security.as_raw().is_null());
    }
}
