//! macOS-only IPC contract shared by the desktop client and native broker.
//! This module is never compiled into the Windows backends.
#![allow(dead_code)] // Each consumer uses only its client or server half.
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SNAPSHOT_LIMIT: usize = 128 * 1024;
pub fn project_key(project: &str) -> String {
    let canonical = fs::canonicalize(project).unwrap_or_else(|_| PathBuf::from(project));
    hash_prefix(
        canonical.to_string_lossy().trim_end_matches('/').as_bytes(),
        16,
    )
}
fn hash_prefix(value: &[u8], count: usize) -> String {
    Sha256::digest(value)
        .iter()
        .take(count)
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn directory() -> PathBuf {
    PathBuf::from(format!("/tmp/locus-{}", unsafe { libc::geteuid() }))
}
pub fn endpoint(project: &str) -> String {
    let namespace = std::env::var("LOCUS_UNITY_TEST_PIPE_NAMESPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| format!("-{}", hash_prefix(s.as_bytes(), 8)))
        .unwrap_or_default();
    directory()
        .join(format!("{}{namespace}.sock", project_key(project)))
        .to_string_lossy()
        .into_owned()
}
pub fn state_path(endpoint: &str) -> PathBuf {
    Path::new(endpoint).with_extension("state.json")
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
pub fn validate_endpoint(project: &str, endpoint: &str) -> io::Result<()> {
    if !Path::new(project).is_absolute() {
        return Err(invalid("project must be absolute"));
    }
    let path = Path::new(endpoint);
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("endpoint filename"))?;
    let stem = name
        .strip_suffix(".sock")
        .ok_or_else(|| invalid("endpoint extension"))?;
    let key = project_key(project);
    let suffix = stem
        .strip_prefix(&key)
        .ok_or_else(|| invalid("endpoint project mismatch"))?;
    if path.parent() != Some(directory().as_path())
        || endpoint.len() >= 104
        || !(suffix.is_empty()
            || (suffix.len() == 17
                && suffix.starts_with('-')
                && suffix[1..]
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))))
    {
        return Err(invalid("endpoint outside current-user project namespace"));
    }
    Ok(())
}
fn validate_directory(create: bool) -> io::Result<()> {
    let path = directory();
    if create {
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
        return Err(invalid(
            "IPC directory must be owned by current user with mode 0700",
        ));
    }
    Ok(())
}
fn private_file(meta: &fs::Metadata) -> io::Result<()> {
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
        || meta.nlink() != 1
    {
        return Err(invalid(
            "IPC file must be private, regular, and unlinked elsewhere",
        ));
    }
    Ok(())
}
pub fn process_identity(pid: u32) -> Option<(u64, u64)> {
    if pid == 0 || pid > i32::MAX as u32 {
        return None;
    }
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let len = std::mem::size_of_val(&info);
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            len as i32,
        )
    };
    (read == len as i32
        && info.pbi_pid == pid
        && info.pbi_uid == unsafe { libc::geteuid() }
        && info.pbi_start_tvsec > 0
        && info.pbi_status != libc::SZOMB)
        .then_some((info.pbi_start_tvsec, info.pbi_start_tvusec))
}
pub fn peer_identity(stream: &impl AsRawFd) -> io::Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0
        || uid != unsafe { libc::geteuid() }
    {
        return Err(invalid("socket peer UID mismatch"));
    }
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut pid as *mut _ as *mut _,
            &mut len,
        )
    } != 0
        || pid <= 0
    {
        return Err(invalid("socket peer PID unavailable"));
    }
    Ok(pid as u32)
}
pub fn validate_socket(project: &str, endpoint: &str) -> io::Result<()> {
    validate_endpoint(project, endpoint)?;
    validate_directory(false)?;
    let meta = fs::symlink_metadata(endpoint)?;
    if !meta.file_type().is_socket()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o600
    {
        return Err(invalid("socket ownership/type/mode mismatch"));
    }
    Ok(())
}
pub struct EndpointGuard {
    endpoint: String,
    lock: File,
    inode: u64,
}
impl Drop for EndpointGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.endpoint)
            .map(|m| m.ino() == self.inode)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&self.endpoint);
            let _ = fs::remove_file(state_path(&self.endpoint));
        }
        // Keep the lock inode: unlinking it would let another process bypass flock.
        let _ = unsafe { libc::flock(self.lock.as_raw_fd(), libc::LOCK_UN) };
    }
}
pub fn bind_endpoint(
    project: &str,
    endpoint: &str,
) -> io::Result<(std::os::unix::net::UnixListener, EndpointGuard)> {
    validate_endpoint(project, endpoint)?;
    validate_directory(true)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(Path::new(endpoint).with_extension("lock"))?;
    private_file(&lock.metadata()?)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another broker owns endpoint",
        ));
    }
    if fs::symlink_metadata(endpoint).is_ok() {
        validate_socket(project, endpoint)?;
        // An unowned leftover may only be removed if no live listener remains.
        if std::os::unix::net::UnixStream::connect(endpoint).is_ok() {
            return Err(io::Error::new(io::ErrorKind::AddrInUse, "live endpoint"));
        }
        fs::remove_file(endpoint)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(endpoint)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(endpoint, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let inode = fs::symlink_metadata(endpoint)?.ino();
    Ok((
        listener,
        EndpointGuard {
            endpoint: endpoint.to_owned(),
            lock,
            inode,
        },
    ))
}
pub fn write_snapshot(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > SNAPSHOT_LIMIT {
        return Err(invalid("snapshot too large"));
    }
    validate_directory(false)?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let temp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        file.write_all(bytes)?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub fn read_snapshot(project: &str, endpoint: &str) -> io::Result<serde_json::Value> {
    validate_socket(project, endpoint)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(state_path(endpoint))?;
    private_file(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.take((SNAPSHOT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > SNAPSHOT_LIMIT {
        return Err(invalid("snapshot too large"));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    let pid = value["processId"]
        .as_u64()
        .filter(|p| *p <= i32::MAX as u64)
        .ok_or_else(|| invalid("snapshot PID"))? as u32;
    let expected = (
        value["processStartSecs"].as_u64(),
        value["processStartMicros"].as_u64(),
    );
    let identity =
        process_identity(pid).ok_or_else(|| invalid("snapshot process no longer exists"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let observed = value["observedAtMs"]
        .as_u64()
        .ok_or_else(|| invalid("snapshot timestamp"))?;
    if expected != (Some(identity.0), Some(identity.1))
        || value["stateVersion"] != 1
        || value["pipeName"] != endpoint
        || value["project"].as_str().map(project_key) != Some(project_key(project))
        || observed > now.saturating_add(1000)
        || now.saturating_sub(observed) > 3000
    {
        return Err(invalid("snapshot identity, project, or freshness mismatch"));
    }
    Ok(value)
}
pub async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    out: &mut Vec<u8>,
    limit: usize,
) -> io::Result<usize> {
    use tokio::io::AsyncBufReadExt;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if out.is_empty() {
                Ok(0)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete frame",
                ))
            };
        }
        let count = available
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(available.len());
        if out.len().saturating_add(count) > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        let finished = available[count - 1] == b'\n';
        out.extend_from_slice(&available[..count]);
        reader.consume(count);
        if finished {
            return Ok(out.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_reject_other_projects_users_and_path_traversal() {
        let project = "/locus-test/Project";
        let valid = endpoint(project);
        assert!(validate_endpoint(project, &valid).is_ok());
        assert!(valid.len() < 104);
        assert!(validate_endpoint("/locus-test/Another", &valid).is_err());
        assert!(validate_endpoint("relative", &valid).is_err());
        assert!(validate_endpoint(
            project,
            &format!("/tmp/other/{}.sock", project_key(project))
        )
        .is_err());
        assert!(validate_endpoint(
            project,
            &format!("{}/../{}.sock", directory().display(), project_key(project))
        )
        .is_err());
        assert!(validate_endpoint(project, &valid.replace(".sock", "-bad.sock")).is_err());
    }
    #[tokio::test]
    async fn framing_rejects_oversized_or_truncated_messages_without_unbounded_growth() {
        let bytes = b"12345678901234567890";
        let mut reader = tokio::io::BufReader::new(&bytes[..]);
        let mut output = Vec::new();
        assert_eq!(
            read_frame(&mut reader, &mut output, 8)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(output.len() <= 8);
        let mut reader = tokio::io::BufReader::new(&b"abc"[..]);
        output.clear();
        assert_eq!(
            read_frame(&mut reader, &mut output, 8)
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        let mut reader = tokio::io::BufReader::new(&b"abc\ndef\n"[..]);
        output.clear();
        assert_eq!(read_frame(&mut reader, &mut output, 8).await.unwrap(), 4);
        assert_eq!(output, b"abc\n");
    }
    #[test]
    fn stale_identity_symlink_and_unsafe_snapshot_permissions_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let project = std::env::temp_dir().join(format!(
            "locus-snapshot-test-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&project).unwrap();
        let project = project.to_string_lossy().into_owned();
        let endpoint = endpoint(&project);
        let (_listener, guard) = bind_endpoint(&project, &endpoint).unwrap();
        let identity = process_identity(std::process::id()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut value = serde_json::json!({"stateVersion":1,"processId":std::process::id(),
            "processStartSecs":identity.0,"processStartMicros":identity.1,
            "pipeName":endpoint,"project":project,"observedAtMs":now});
        let path = state_path(&endpoint);
        write_snapshot(&path, value.to_string().as_bytes()).unwrap();
        assert!(read_snapshot(&project, &endpoint).is_ok());
        assert!(bind_endpoint(&project, &endpoint).is_err());
        value["processStartSecs"] = (identity.0 + 1).into();
        write_snapshot(&path, value.to_string().as_bytes()).unwrap();
        assert!(read_snapshot(&project, &endpoint).is_err());
        value["processStartSecs"] = identity.0.into();
        value["observedAtMs"] = 1.into();
        write_snapshot(&path, value.to_string().as_bytes()).unwrap();
        assert!(read_snapshot(&project, &endpoint).is_err());
        value["observedAtMs"] = now.into();
        write_snapshot(&path, value.to_string().as_bytes()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_snapshot(&project, &endpoint).is_err());
        fs::remove_file(&path).unwrap();
        symlink("/etc/passwd", &path).unwrap();
        assert!(read_snapshot(&project, &endpoint).is_err());
        drop(guard);
        fs::remove_file(Path::new(&endpoint).with_extension("lock")).unwrap();
        fs::remove_dir(project).unwrap();
    }
}
