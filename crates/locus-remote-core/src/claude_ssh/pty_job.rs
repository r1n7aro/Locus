//! The terminal's process tree must not survive a crashed Locus process.
#[cfg(windows)]
pub struct PtyJob(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for PtyJob {}
#[cfg(windows)]
unsafe impl Sync for PtyJob {}
#[cfg(windows)]
impl PtyJob {
    pub fn attach(pid: u32) -> Result<Self, String> {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::{JobObjects::*, Threading::*},
        };
        let job = unsafe { CreateJobObjectW(None, windows_core::PCWSTR::null()) }
            .map_err(|e| e.to_string())?;
        let owned = Self(job);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        }
        .map_err(|e| e.to_string())?;
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }
            .map_err(|e| e.to_string())?;
        let result = unsafe { AssignProcessToJobObject(job, process) }.map_err(|e| e.to_string());
        let _ = unsafe { CloseHandle(process) };
        result?;
        Ok(owned)
    }
    pub fn terminate(&self) {
        let _ = unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
    }
}
#[cfg(windows)]
impl Drop for PtyJob {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}
