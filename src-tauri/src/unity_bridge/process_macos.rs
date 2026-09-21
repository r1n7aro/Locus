//! macOS Editor discovery and lifecycle. No Win32 probing, shell parsing, or
//! process-name-only termination is shared with the Windows backend.

/// `KERN_PROCARGS2` is argc, executable path + padding, then exactly argc
/// NUL-delimited arguments. Environment strings following argv are never read.
fn parse_proc_args(bytes: &[u8]) -> Result<Vec<String>, String> {
    let count_bytes: [u8; 4] = bytes
        .get(..4)
        .ok_or("Missing macOS argc")?
        .try_into()
        .map_err(|_| "Invalid macOS argc")?;
    let count = i32::from_ne_bytes(count_bytes);
    if !(1..=65_536).contains(&count) {
        return Err("Invalid macOS argc".into());
    }
    let mut offset = 4;
    offset += bytes
        .get(offset..)
        .and_then(|tail| tail.iter().position(|b| *b == 0))
        .ok_or("Missing macOS executable terminator")?
        + 1;
    while bytes.get(offset) == Some(&0) {
        offset += 1;
    }
    let mut args = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let tail = bytes.get(offset..).ok_or("Truncated macOS argv")?;
        let length = tail
            .iter()
            .position(|b| *b == 0)
            .ok_or("Unterminated macOS argument")?;
        args.push(
            std::str::from_utf8(&tail[..length])
                .map_err(|_| "macOS argument is not UTF-8")?
                .to_string(),
        );
        offset += length + 1;
    }
    Ok(args)
}

fn is_editor_image(path: &std::path::Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("Unity" | "Tuanjie")
    ) && path
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "MacOS")
        && path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "Contents")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProcessIdentity {
    pid: u32,
    uid: u32,
    start_seconds: u64,
    start_micros: u64,
    executable: std::path::PathBuf,
    project: String,
}

/// libproc uses 0 for failure and returns bytes, not a PID count.
fn returned_pid_count(bytes: i32, capacity: usize) -> Result<Option<usize>, String> {
    if bytes <= 0 || bytes as usize % std::mem::size_of::<i32>() != 0 {
        return Err("Invalid or failed macOS process enumeration".into());
    }
    let count = bytes as usize / std::mem::size_of::<i32>();
    Ok((count < capacity).then_some(count))
}

#[derive(Default)]
struct ObservationCache {
    identities: std::collections::HashMap<(String, u32, u64), ProcessIdentity>,
}

impl ObservationCache {
    fn insert(&mut self, identity: ProcessIdentity, checked_at_ms: u64) {
        let created_at_ms = identity
            .start_seconds
            .checked_mul(1000)
            .and_then(|seconds| seconds.checked_add(identity.start_micros / 1000));
        // A late callback carrying an old observation must not associate its
        // timestamp with a newer process that happens to reuse the same PID.
        if !created_at_ms.is_some_and(|created| created <= checked_at_ms) {
            return;
        }
        self.identities
            .entry((identity.project.clone(), identity.pid, checked_at_ms))
            .or_insert(identity);
        if self.identities.len() > 4096 {
            if let Some(oldest) = self.identities.keys().min_by_key(|key| key.2).cloned() {
                self.identities.remove(&oldest);
            }
        }
    }

    fn get(&self, project: &str, pid: u32, checked_at_ms: u64) -> Option<&ProcessIdentity> {
        self.identities
            .get(&(project.to_string(), pid, checked_at_ms))
    }
}

#[cfg(target_os = "macos")]
pub(crate) use native::*;

#[cfg(target_os = "macos")]
mod native {
    use super::super::{
        launch_mode_from_args, project_path_from_args, unity_process_args_are_worker, unix_now_ms,
        UnityEditorProcessInfo, UnityProcessIdentityLiveness, UnityProjectProcessCloseResult,
    };
    use super::{
        is_editor_image, parse_proc_args, returned_pid_count, ObservationCache, ProcessIdentity,
    };
    use crate::unity_bridge::{managed_editor::EditorResource, UnityLaunchMode};
    use std::{
        path::{Path, PathBuf},
        sync::{Mutex, OnceLock},
        time::{Duration, Instant},
    };

    #[derive(Clone)]
    struct EditorProcess {
        identity: ProcessIdentity,
        args: Vec<String>,
    }

    struct EditorScan {
        verified: Vec<EditorProcess>,
        uncertain: Vec<String>,
    }

    fn observed_identities() -> &'static Mutex<ObservationCache> {
        static OBSERVED: OnceLock<Mutex<ObservationCache>> = OnceLock::new();
        OBSERVED.get_or_init(|| Mutex::new(ObservationCache::default()))
    }

    fn remember_identity(identity: ProcessIdentity, checked_at_ms: u64) {
        if let Ok(mut observed) = observed_identities().lock() {
            // One observation cannot silently change generation, even if a PID
            // gets reused while another project is being enumerated.
            observed.insert(identity, checked_at_ms);
        }
    }

    pub(crate) fn normalize_project_identity(path: &str) -> Option<String> {
        // APFS may be case-sensitive. Resolve symlinks but never lowercase or
        // convert '/' to '\\' on macOS. Missing projects are not safe identities.
        let path = Path::new(path);
        if !path.is_absolute() {
            return None;
        }
        std::fs::canonicalize(path)
            .ok()?
            .to_str()
            .map(str::to_owned)
    }

    fn bsd_info(pid: u32) -> Result<Option<libc::proc_bsdinfo>, String> {
        let pid = i32::try_from(pid).map_err(|_| "Invalid macOS process ID")?;
        if pid <= 0 {
            return Err("Invalid macOS process ID".into());
        }
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        let read = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            )
        };
        if read == size {
            if info.pbi_pid != pid as u32 {
                return Err(format!("macOS process identity changed for PID {pid}"));
            }
            if info.pbi_status == libc::SZOMB {
                return Ok(None);
            }
            return Ok(Some(info));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        // A failed or partial read does not prove process exit.
        Err(format!("Cannot verify macOS process {pid}: {error}"))
    }

    fn process_args(pid: u32) -> Result<Vec<String>, String> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
        let mut bytes = vec![0u8; 1024 * 1024];
        let mut size = bytes.len();
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                bytes.as_mut_ptr() as *mut libc::c_void,
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(format!(
                "Cannot read macOS process {pid} arguments: {}",
                std::io::Error::last_os_error()
            ));
        }
        bytes.truncate(size);
        parse_proc_args(&bytes)
    }

    fn executable(pid: u32) -> Result<PathBuf, String> {
        let mut bytes = vec![0u8; 4096];
        let read = unsafe {
            libc::proc_pidpath(
                pid as i32,
                bytes.as_mut_ptr() as *mut libc::c_void,
                bytes.len() as u32,
            )
        };
        if read <= 0 {
            return Err(format!(
                "Cannot read macOS process {pid} executable: {}",
                std::io::Error::last_os_error()
            ));
        }
        let length = bytes.iter().position(|b| *b == 0).unwrap_or(read as usize);
        let path =
            std::str::from_utf8(&bytes[..length]).map_err(|_| "Editor executable is not UTF-8")?;
        std::fs::canonicalize(path)
            .map_err(|e| format!("Cannot canonicalize Editor executable: {e}"))
    }

    fn editor_process(pid: u32) -> Result<Option<EditorProcess>, String> {
        let Some(before) = bsd_info(pid)? else {
            return Ok(None);
        };
        if before.pbi_uid != unsafe { libc::geteuid() } {
            return Ok(None);
        }
        // Filter unrelated processes before attempting their restricted argv.
        let name = before
            .pbi_name
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| *b as u8)
            .collect::<Vec<_>>();
        let comm = before
            .pbi_comm
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| *b as u8)
            .collect::<Vec<_>>();
        if ![name.as_slice(), comm.as_slice()]
            .iter()
            .any(|name| matches!(*name, b"Unity" | b"Tuanjie"))
        {
            return Ok(None);
        }
        let executable = executable(pid)?;
        if !is_editor_image(&executable) {
            return Ok(None);
        }
        let args = process_args(pid)?;
        let project = project_path_from_args(&args)
            .and_then(normalize_project_identity)
            .ok_or_else(|| format!("Cannot verify project identity for Editor {pid}"))?;
        let Some(after) = bsd_info(pid)? else {
            return Ok(None);
        };
        if before.pbi_start_tvsec != after.pbi_start_tvsec
            || before.pbi_start_tvusec != after.pbi_start_tvusec
            || before.pbi_uid != after.pbi_uid
        {
            return Err(format!("Editor {pid} changed generation during discovery"));
        }
        Ok(Some(EditorProcess {
            identity: ProcessIdentity {
                pid,
                uid: before.pbi_uid,
                start_seconds: before.pbi_start_tvsec,
                start_micros: before.pbi_start_tvusec,
                executable,
                project,
            },
            args,
        }))
    }

    fn editor_processes() -> Result<EditorScan, String> {
        let mut processes = Vec::new();
        let mut uncertain = Vec::new();
        // XNU sys/proc_info.h: PROC_UID_ONLY = 4. `proc_listpids` returns
        // BYTES (unlike `proc_listallpids`), and returns 0 on syscall failure.
        // Pre-filtering here avoids trying restricted TBSDINFO on root's pids.
        const PROC_UID_ONLY: u32 = 4;
        let uid = unsafe { libc::geteuid() };
        // Leave headroom, and retry if a growing process list filled the buffer.
        for attempt in 0..3 {
            let bytes = unsafe { libc::proc_listpids(PROC_UID_ONLY, uid, std::ptr::null_mut(), 0) };
            if bytes <= 0 {
                return Err(format!(
                    "Cannot enumerate macOS processes: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut pids =
                vec![0i32; bytes as usize / std::mem::size_of::<i32>() + 128 * (attempt + 1)];
            let read = unsafe {
                libc::proc_listpids(
                    PROC_UID_ONLY,
                    uid,
                    pids.as_mut_ptr() as *mut libc::c_void,
                    (pids.len() * std::mem::size_of::<i32>()) as i32,
                )
            };
            let count = returned_pid_count(read, pids.len())
                .map_err(|error| format!("{error}: {}", std::io::Error::last_os_error()))?;
            let Some(count) = count else {
                continue;
            };
            for pid in pids.into_iter().take(count).filter(|pid| *pid > 0) {
                match editor_process(pid as u32) {
                    Ok(Some(process)) => processes.push(process),
                    Ok(None) => {}
                    Err(error) => uncertain.push(error),
                }
            }
            return Ok(EditorScan {
                verified: processes,
                uncertain,
            });
        }
        Err("macOS process list kept growing during discovery".into())
    }

    fn same_generation(expected: &ProcessIdentity) -> Result<bool, String> {
        let Some(actual) = editor_process(expected.pid)? else {
            return Ok(false);
        };
        Ok(actual.identity == *expected)
    }

    pub(crate) fn remember_process_generation(pid: u32, project: &str, checked_at_ms: u64) {
        let Some(expected_project) = normalize_project_identity(project) else {
            return;
        };
        let Ok(Some(process)) = editor_process(pid) else {
            return;
        };
        if process.identity.project != expected_project
            || unity_process_args_are_worker(&process.args)
        {
            return;
        }
        remember_identity(process.identity, checked_at_ms);
    }

    pub(crate) fn known_process_is_alive(
        project: &str,
        previous: &UnityEditorProcessInfo,
        checked_at_ms: u64,
    ) -> Result<bool, String> {
        let project =
            normalize_project_identity(project).ok_or("Cannot resolve Unity project identity")?;
        let pid = previous.process_id.ok_or("Editor process ID is unknown")?;
        let expected = observed_identities()
            .lock()
            .map_err(|_| "Editor identity cache unavailable")?
            .get(&project, pid, previous.checked_at_ms)
            .cloned()
            .ok_or("Editor process generation has not been verified")?;
        let alive = same_generation(&expected)?;
        if alive {
            remember_identity(expected, checked_at_ms);
        }
        Ok(alive)
    }

    pub(crate) fn process_created_at_unix_ms(pid: u32) -> Option<u64> {
        let info = bsd_info(pid).ok()??;
        info.pbi_start_tvsec
            .checked_mul(1000)?
            .checked_add(info.pbi_start_tvusec / 1000)
    }

    pub(crate) fn query_process_identity_liveness(
        pid: u32,
        expected_created_at_ms: Option<u64>,
    ) -> Result<UnityProcessIdentityLiveness, String> {
        let Some(info) = bsd_info(pid)? else {
            return Ok(UnityProcessIdentityLiveness::Exited);
        };
        let expected = expected_created_at_ms.ok_or("Editor process creation time is unknown")?;
        let actual = info
            .pbi_start_tvsec
            .checked_mul(1000)
            .and_then(|s| s.checked_add(info.pbi_start_tvusec / 1000))
            .ok_or("Invalid Editor creation timestamp")?;
        Ok(if actual == expected {
            UnityProcessIdentityLiveness::Alive
        } else {
            UnityProcessIdentityLiveness::Replaced
        })
    }

    pub(crate) fn query_current_project_editor_process_uncached(
        project: String,
    ) -> UnityEditorProcessInfo {
        let now = unix_now_ms();
        let Some(identity) = normalize_project_identity(&project) else {
            return UnityEditorProcessInfo::unknown(now, "Cannot resolve Unity project identity");
        };
        match editor_processes() {
            Ok(scan) => {
                match scan.verified.into_iter().find(|p| {
                    p.identity.project == identity && !unity_process_args_are_worker(&p.args)
                }) {
                    Some(process) => {
                        remember_identity(process.identity.clone(), now);
                        UnityEditorProcessInfo {
                            state: super::super::UnityEditorProcessState::Running,
                            process_id: Some(process.identity.pid),
                            executable_path: Some(
                                process.identity.executable.to_string_lossy().into_owned(),
                            ),
                            project_path: Some(process.identity.project),
                            checked_at_ms: now,
                            last_error: None,
                        }
                    }
                    None if scan.uncertain.is_empty() => UnityEditorProcessInfo::not_running(now),
                    None => UnityEditorProcessInfo::unknown(now, scan.uncertain.join("; ")),
                }
            }
            Err(error) => UnityEditorProcessInfo::unknown(now, error),
        }
    }

    pub(crate) fn main_editor_process_count() -> Result<usize, String> {
        let scan = editor_processes()?;
        // Unknown Editor candidates still consume admission capacity; a
        // welcome window or unreadable argv never becomes a free launch slot.
        Ok(scan
            .verified
            .iter()
            .filter(|p| !unity_process_args_are_worker(&p.args))
            .count()
            + scan.uncertain.len())
    }

    pub(crate) async fn query_unity_editor_launch_mode(
        pid: Option<u32>,
    ) -> Option<UnityLaunchMode> {
        let pid = pid?;
        tokio::task::spawn_blocking(move || {
            editor_process(pid)
                .ok()
                .flatten()
                .map(|p| launch_mode_from_args(&p.args))
        })
        .await
        .ok()
        .flatten()
    }

    pub(crate) fn explicit_editor_log_path(pid: u32) -> Option<PathBuf> {
        let process = editor_process(pid).ok()??;
        for (index, arg) in process.args.iter().enumerate() {
            let value = if arg.eq_ignore_ascii_case("-logFile") {
                process.args.get(index + 1).map(String::as_str)
            } else {
                arg.split_once('=')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("-logFile"))
                    .map(|(_, value)| value)
            };
            if let Some(value) = value {
                return (!value.is_empty() && value != "-").then(|| PathBuf::from(value));
            }
        }
        None
    }

    fn resident_bytes(pid: u32) -> Option<u64> {
        let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        (unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTASKINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            )
        } == size)
            .then_some(info.pti_resident_size)
    }

    pub(crate) fn editor_resources() -> Result<Vec<EditorResource>, String> {
        let scan = editor_processes()?;
        let processes = scan.verified;
        let mut resources = Vec::new();
        for process in processes
            .iter()
            .filter(|p| !unity_process_args_are_worker(&p.args))
        {
            let members = processes
                .iter()
                .filter(|p| {
                    p.identity.project == process.identity.project
                        && (p.identity.pid == process.identity.pid
                            || unity_process_args_are_worker(&p.args))
                })
                .collect::<Vec<_>>();
            let bytes = members.iter().try_fold(0u64, |sum, p| {
                sum.checked_add(resident_bytes(p.identity.pid)?)
            });
            resources.push(EditorResource {
                project_path: process.identity.project.clone(),
                process_id: process.identity.pid,
                mode: launch_mode_from_args(&process.args),
                managed: false,
                working_set_bytes: bytes,
                import_worker_count: members.len().saturating_sub(1),
                last_error: (!scan.uncertain.is_empty())
                    .then(|| "Some Editor process identities could not be verified".to_string()),
            });
        }
        resources.sort_by(|a, b| {
            a.project_path
                .cmp(&b.project_path)
                .then(a.process_id.cmp(&b.process_id))
        });
        Ok(resources)
    }

    fn signal_verified(process: &ProcessIdentity, signal: i32) -> Result<(), String> {
        if !same_generation(process)? {
            return Ok(());
        }
        // Never signal a process group, child tree, unknown PID, or a replaced
        // Editor. Revalidate start time, uid, image and project before each signal.
        if unsafe { libc::kill(process.pid as i32, signal) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("Cannot close Editor {}: {error}", process.pid));
            }
        }
        Ok(())
    }

    fn wait_for_exit(
        processes: &[ProcessIdentity],
        timeout: Duration,
    ) -> Result<Vec<ProcessIdentity>, String> {
        let start = Instant::now();
        loop {
            let mut remaining = Vec::new();
            for process in processes {
                if same_generation(process)? {
                    remaining.push(process.clone());
                }
            }
            if remaining.is_empty() || start.elapsed() >= timeout {
                return Ok(remaining);
            }
            std::thread::sleep(
                Duration::from_millis(100).min(timeout.saturating_sub(start.elapsed())),
            );
        }
    }

    pub(crate) fn close_current_project_unity_processes_sync(
        project: &str,
        timeout: Duration,
        force_first: bool,
    ) -> Result<UnityProjectProcessCloseResult, String> {
        let target = normalize_project_identity(project)
            .ok_or("Cannot resolve Unity project identity; close refused")?;
        let scan = editor_processes()?;
        if !scan.uncertain.is_empty() {
            return Err(format!(
                "Cannot prove the project is safe to close or replace: {}",
                scan.uncertain.join("; ")
            ));
        }
        let processes = scan
            .verified
            .into_iter()
            .filter(|p| p.identity.project == target)
            .map(|p| p.identity)
            .collect::<Vec<_>>();
        let ids = processes.iter().map(|p| p.pid).collect::<Vec<_>>();
        let mut forced = Vec::new();
        for process in &processes {
            signal_verified(
                process,
                if force_first {
                    libc::SIGKILL
                } else {
                    libc::SIGTERM
                },
            )?;
        }
        let grace = if force_first {
            timeout
        } else {
            timeout.min(Duration::from_secs(20))
        };
        let mut remaining = wait_for_exit(&processes, grace)?;
        if force_first {
            forced = ids.clone();
        } else if !remaining.is_empty() {
            for process in &remaining {
                signal_verified(process, libc::SIGKILL)?;
            }
            forced = remaining.iter().map(|p| p.pid).collect();
            remaining = wait_for_exit(
                &remaining,
                timeout.saturating_sub(grace).max(Duration::from_secs(5)),
            )?;
        }
        if !remaining.is_empty() {
            return Err(format!(
                "Unity process(es) did not exit before plugin install: {:?}",
                remaining.iter().map(|p| p.pid).collect::<Vec<_>>()
            ));
        }
        // A new Editor generation may have opened the same project while the
        // selected generation was closing. Do not install over it or kill it.
        let after = editor_processes()?;
        if !after.uncertain.is_empty() {
            return Err(format!(
                "Cannot prove the project is safe to replace: {}",
                after.uncertain.join("; ")
            ));
        }
        if after
            .verified
            .iter()
            .any(|process| process.identity.project == target)
        {
            return Err(
                "Another Editor generation is now using this project; plugin replacement refused"
                    .into(),
            );
        }
        Ok(UnityProjectProcessCloseResult {
            process_ids: ids,
            forced_process_ids: forced,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn argv(args: &[&str]) -> Vec<u8> {
        let mut bytes = (args.len() as i32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/Applications/Unity.app/Contents/MacOS/Unity\0\0\0");
        for arg in args {
            bytes.extend_from_slice(arg.as_bytes());
            bytes.push(0);
        }
        bytes.extend_from_slice(b"IGNORED_ENV=-projectPath\0/other\0");
        bytes
    }
    #[test]
    fn argv_preserves_spaces_empty_arguments_and_ignores_environment() {
        assert_eq!(
            parse_proc_args(&argv(&[
                "Unity",
                "-projectPath",
                "/Users/me/Game With Spaces",
                ""
            ]))
            .unwrap(),
            vec!["Unity", "-projectPath", "/Users/me/Game With Spaces", ""]
        );
    }
    #[test]
    fn argv_rejects_partial_or_non_utf8_input() {
        assert!(parse_proc_args(&[]).is_err());
        let mut bytes = argv(&["Unity"]);
        bytes.truncate(8);
        assert!(parse_proc_args(&bytes).is_err());
        let mut bytes = 1i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"exe\0\xff\0");
        assert!(parse_proc_args(&bytes).is_err());
    }
    #[test]
    fn only_editor_bundle_images_qualify() {
        assert!(is_editor_image(std::path::Path::new(
            "/Applications/Unity/Unity.app/Contents/MacOS/Unity"
        )));
        assert!(!is_editor_image(std::path::Path::new("/tmp/Unity")));
        assert!(!is_editor_image(std::path::Path::new(
            "/Applications/Unity.app/Contents/MacOS/Unity Hub"
        )));
    }
    #[test]
    fn libproc_zero_or_partial_bytes_cannot_be_treated_as_an_empty_process_list() {
        assert!(returned_pid_count(0, 10).is_err());
        assert!(returned_pid_count(-1, 10).is_err());
        assert!(returned_pid_count(7, 10).is_err());
        assert_eq!(returned_pid_count(8, 10).unwrap(), Some(2));
        assert_eq!(returned_pid_count(40, 10).unwrap(), None);
    }

    #[test]
    fn pid_reuse_in_another_project_cannot_replace_a_prior_liveness_observation() {
        let old = ProcessIdentity {
            pid: 42,
            uid: 501,
            start_seconds: 10,
            start_micros: 1,
            executable: "/Applications/Unity.app/Contents/MacOS/Unity".into(),
            project: "/A".into(),
        };
        let mut new = old.clone();
        new.project = "/B".into();
        new.start_seconds = 20;
        let mut restarted = old.clone();
        restarted.start_seconds = 30;
        let mut cache = ObservationCache::default();
        cache.insert(old.clone(), 10_100);
        cache.insert(new.clone(), 20_100);
        cache.insert(restarted.clone(), 30_100);
        cache.insert(restarted.clone(), 10_200);
        assert_eq!(cache.get("/A", 42, 10_100), Some(&old));
        assert_eq!(cache.get("/B", 42, 20_100), Some(&new));
        assert_eq!(cache.get("/A", 42, 30_100), Some(&restarted));
        assert!(cache.get("/A", 42, 20_100).is_none());
        assert!(cache.get("/A", 42, 10_200).is_none());
    }
    #[test]
    fn identity_requires_generation_uid_image_and_case_sensitive_project() {
        let original = ProcessIdentity {
            pid: 42,
            uid: 501,
            start_seconds: 10,
            start_micros: 123,
            executable: "/Applications/Unity.app/Contents/MacOS/Unity".into(),
            project: "/Users/me/Game".into(),
        };
        for variant in 0..5 {
            let mut changed = original.clone();
            match variant {
                0 => changed.start_micros += 1,
                1 => changed.uid += 1,
                2 => changed.project = "/Users/me/game".into(),
                3 => changed.executable = "/other/Unity".into(),
                _ => changed.pid += 1,
            }
            assert_ne!(original, changed);
        }
    }
}
