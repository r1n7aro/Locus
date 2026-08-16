use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const GIT_VERSION_TIMEOUT: Duration = Duration::from_millis(1500);
const GITHUB_CLI_VERSION_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessOwner {
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub working_dir: Option<String>,
}

impl ProcessOwner {
    pub fn session(session_id: impl Into<String>, working_dir: impl Into<String>) -> Self {
        Self {
            session_id: Some(session_id.into()),
            task_id: None,
            working_dir: Some(working_dir.into()),
        }
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }
}

struct ManagedProcessEntry {
    owner: ProcessOwner,
    controller: Arc<PlatformProcessTree>,
}

#[derive(Default)]
struct ManagedProcessRegistry {
    entries: std::collections::HashMap<String, ManagedProcessEntry>,
}

fn managed_process_registry() -> &'static Mutex<ManagedProcessRegistry> {
    static REGISTRY: OnceLock<Mutex<ManagedProcessRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(ManagedProcessRegistry::default()))
}

static MANAGED_PROCESS_SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

struct ManagedProcessRegistration {
    id: String,
    controller: Arc<PlatformProcessTree>,
}

impl Drop for ManagedProcessRegistration {
    fn drop(&mut self) {
        managed_process_registry()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entries
            .remove(&self.id);
    }
}

pub struct ManagedChild {
    child: tokio::process::Child,
    _registration: ManagedProcessRegistration,
}

impl ManagedChild {
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.stdin.take()
    }

    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    pub async fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }

    pub async fn wait_with_output(self) -> io::Result<std::process::Output> {
        let Self {
            child,
            _registration,
        } = self;
        child.wait_with_output().await
    }

    pub fn terminate_tree(&mut self) -> io::Result<()> {
        self._registration.controller.terminate();
        match self.child.start_kill() {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn register_managed_process(
    owner: ProcessOwner,
    controller: Arc<PlatformProcessTree>,
) -> ManagedProcessRegistration {
    let id = uuid::Uuid::new_v4().simple().to_string();
    managed_process_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .insert(
            id.clone(),
            ManagedProcessEntry {
                owner,
                controller: controller.clone(),
            },
        );
    ManagedProcessRegistration { id, controller }
}

fn terminate_managed_processes_matching(predicate: impl Fn(&ProcessOwner) -> bool) -> usize {
    let controllers = managed_process_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .values()
        .filter(|entry| predicate(&entry.owner))
        .map(|entry| entry.controller.clone())
        .collect::<Vec<_>>();
    for controller in &controllers {
        controller.terminate();
    }
    controllers.len()
}

pub fn terminate_managed_processes_for_session(session_id: &str) -> usize {
    terminate_managed_processes_matching(|owner| owner.session_id.as_deref() == Some(session_id))
}

pub fn terminate_managed_processes_for_workspace(working_dir: &str) -> usize {
    let target = process_owner_path_key(working_dir);
    terminate_managed_processes_matching(|owner| {
        owner
            .working_dir
            .as_deref()
            .is_some_and(|path| process_owner_path_key(path) == target)
    })
}

pub fn terminate_all_managed_processes() -> usize {
    terminate_managed_processes_matching(|_| true)
}

pub fn begin_managed_process_shutdown() -> usize {
    MANAGED_PROCESS_SHUTTING_DOWN.store(true, Ordering::SeqCst);
    terminate_all_managed_processes()
}

pub fn managed_process_count() -> usize {
    managed_process_registry()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entries
        .len()
}

pub async fn wait_for_managed_processes(timeout: Duration) -> bool {
    let started = Instant::now();
    loop {
        if managed_process_count() == 0 {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn process_owner_path_key(path: &str) -> String {
    let normalized = path
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if cfg!(target_os = "windows") {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    }
}

pub fn spawn_managed(
    mut command: tokio::process::Command,
    owner: ProcessOwner,
) -> io::Result<ManagedChild> {
    if MANAGED_PROCESS_SHUTTING_DOWN.load(Ordering::SeqCst) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Locus is shutting down; managed process startup was cancelled",
        ));
    }

    prepare_managed_command(&mut command);
    command.kill_on_drop(true);
    let mut child = command.spawn()?;
    let controller = match PlatformProcessTree::attach(&child) {
        Ok(controller) => Arc::new(controller),
        Err(error) => {
            let _ = child.start_kill();
            return Err(error);
        }
    };

    #[cfg(target_os = "windows")]
    if let Err(error) = resume_managed_process(&child) {
        controller.terminate();
        let _ = child.start_kill();
        return Err(error);
    }

    let registration = register_managed_process(owner, controller);
    Ok(ManagedChild {
        child,
        _registration: registration,
    })
}

#[cfg(target_os = "windows")]
fn prepare_managed_command(command: &mut tokio::process::Command) {
    const CREATE_SUSPENDED: u32 = 0x0000_0004;
    command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
}

#[cfg(unix)]
fn prepare_managed_command(command: &mut tokio::process::Command) {
    command.process_group(0);
}

#[cfg(not(any(target_os = "windows", unix)))]
fn prepare_managed_command(_command: &mut tokio::process::Command) {}

#[cfg(target_os = "windows")]
struct PlatformProcessTree {
    job: windows::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
unsafe impl Send for PlatformProcessTree {}
#[cfg(target_os = "windows")]
unsafe impl Sync for PlatformProcessTree {}

#[cfg(target_os = "windows")]
impl PlatformProcessTree {
    fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let job = unsafe { CreateJobObjectW(None, windows_core::PCWSTR::null()) }
            .map_err(windows_io_error)?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Err(error) = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
            return Err(windows_io_error(error));
        }
        let raw_process = child.raw_handle().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "spawned process handle is unavailable",
            )
        })?;
        if let Err(error) = unsafe { AssignProcessToJobObject(job, HANDLE(raw_process)) } {
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(job) };
            return Err(windows_io_error(error));
        }
        Ok(Self { job })
    }

    fn terminate(&self) {
        let _ = unsafe { windows::Win32::System::JobObjects::TerminateJobObject(self.job, 1) };
    }
}

#[cfg(target_os = "windows")]
impl Drop for PlatformProcessTree {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.job) };
    }
}

#[cfg(target_os = "windows")]
fn resume_managed_process(child: &tokio::process::Child) -> io::Result<()> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let process_id = child
        .id()
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "spawned process id is unavailable"))?;
    let snapshot =
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(windows_io_error)?;
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0usize;
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    while has_entry {
        if entry.th32OwnerProcessID == process_id {
            if let Ok(thread) =
                unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
            {
                let previous = unsafe { ResumeThread(thread) };
                let _ = unsafe { CloseHandle(thread) };
                if previous != u32::MAX {
                    resumed += 1;
                }
            }
        }
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    let _ = unsafe { CloseHandle(snapshot) };
    if resumed == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("failed to resume suspended managed process {process_id}"),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn windows_io_error(error: windows_core::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, error.to_string())
}

#[cfg(unix)]
struct PlatformProcessTree {
    process_group_id: i32,
}

#[cfg(unix)]
impl PlatformProcessTree {
    fn attach(child: &tokio::process::Child) -> io::Result<Self> {
        let process_group_id = child.id().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "spawned process id is unavailable")
        })? as i32;
        Ok(Self { process_group_id })
    }

    fn terminate(&self) {
        unsafe {
            libc::kill(-self.process_group_id, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
impl Drop for PlatformProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(not(any(target_os = "windows", unix)))]
struct PlatformProcessTree;

#[cfg(not(any(target_os = "windows", unix)))]
impl PlatformProcessTree {
    fn attach(_child: &tokio::process::Child) -> io::Result<Self> {
        Ok(Self)
    }

    fn terminate(&self) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitDiscoverySource {
    EnvOverride,
    Managed,
    Path,
    CommonLocation,
}

impl GitDiscoverySource {
    pub fn as_str(self) -> &'static str {
        match self {
            GitDiscoverySource::EnvOverride => "envOverride",
            GitDiscoverySource::Managed => "managed",
            GitDiscoverySource::Path => "path",
            GitDiscoverySource::CommonLocation => "commonLocation",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedGit {
    pub path: PathBuf,
    pub source: GitDiscoverySource,
}

#[derive(Debug, Clone)]
pub struct GitRuntimeCandidate {
    pub path: PathBuf,
    pub source: GitDiscoverySource,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GithubCliDiscoverySource {
    EnvOverride,
    Managed,
    Path,
}

#[derive(Debug, Clone)]
pub struct ResolvedGithubCli {
    pub path: PathBuf,
    pub source: GithubCliDiscoverySource,
}

type GitResolutionCache = Option<Option<ResolvedGit>>;
type GithubCliResolutionCache = Option<Option<ResolvedGithubCli>>;

pub fn command(program: &str) -> std::process::Command {
    let mut cmd = Command::new(resolve_program(program));
    suppress_command_window(&mut cmd);
    crate::network::apply_proxy_env_to_command(&mut cmd);
    cmd
}

pub fn async_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(resolve_program(program));
    suppress_async_command_window(&mut cmd);
    crate::network::apply_proxy_env_to_async_command(&mut cmd);
    cmd
}

pub fn suppress_command_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn suppress_async_command_window(cmd: &mut tokio::process::Command) {
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn resolve_git() -> Option<ResolvedGit> {
    let cache = git_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(resolved) = cached.as_ref() {
        return resolved.clone();
    }

    let resolved = discover_git();
    *cached = Some(resolved.clone());
    resolved
}

pub fn refresh_git_resolution() -> Option<ResolvedGit> {
    let resolved = discover_git();
    let cache = git_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = Some(resolved.clone());
    resolved
}

pub fn clear_git_resolution_cache() {
    let cache = git_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = None;
}

pub fn set_managed_git_resource_dir(path: PathBuf) {
    let roots = managed_git_resource_dirs();
    let mut roots = roots
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !roots.iter().any(|existing| same_path(existing, &path)) {
        roots.push(path);
    }
    clear_git_resolution_cache();
}

pub fn set_managed_github_cli_resource_dir(path: PathBuf) {
    let roots = managed_github_cli_resource_dirs();
    let mut roots = roots
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !roots.iter().any(|existing| same_path(existing, &path)) {
        roots.push(path);
    }
    clear_github_cli_resolution_cache();
}

pub fn resolve_github_cli() -> Option<ResolvedGithubCli> {
    let cache = github_cli_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(resolved) = cached.as_ref() {
        return resolved.clone();
    }

    let resolved = discover_github_cli();
    *cached = Some(resolved.clone());
    resolved
}

pub fn refresh_github_cli_resolution() -> Option<ResolvedGithubCli> {
    let resolved = discover_github_cli();
    let cache = github_cli_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = Some(resolved.clone());
    resolved
}

pub fn clear_github_cli_resolution_cache() {
    let cache = github_cli_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = None;
}

pub fn discover_git_runtimes(include_env_override: bool) -> Vec<GitRuntimeCandidate> {
    let mut runtimes = Vec::new();

    if include_env_override {
        if let Some(raw) = git_env_override() {
            push_git_runtime_candidate(
                &mut runtimes,
                PathBuf::from(raw),
                GitDiscoverySource::EnvOverride,
            );
        }
    }

    for candidate in git_path_candidates() {
        push_git_runtime_candidate(&mut runtimes, candidate, GitDiscoverySource::Path);
    }
    for candidate in git_common_location_candidates() {
        push_git_runtime_candidate(&mut runtimes, candidate, GitDiscoverySource::CommonLocation);
    }
    for candidate in git_managed_resource_candidates() {
        push_git_runtime_candidate(&mut runtimes, candidate, GitDiscoverySource::Managed);
    }

    runtimes
}

pub fn probe_git_runtime(path: PathBuf, source: GitDiscoverySource) -> Option<GitRuntimeCandidate> {
    let mut runtimes = Vec::new();
    push_git_runtime_candidate(&mut runtimes, path, source);
    runtimes.pop()
}

pub fn git_runtime_key(path: &Path) -> String {
    let raw = git_runtime_identity_path(path);
    let text = raw.display().to_string().replace('\\', "/");
    if cfg!(target_os = "windows") {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

pub fn git_is_in_path() -> bool {
    resolve_git_from_path().is_some()
}

pub fn git_version() -> Option<String> {
    let resolved = resolve_git()?;
    git_version_for(&resolved.path)
}

pub fn git_env_override() -> Option<String> {
    std::env::var("LOCUS_GIT_PATH")
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

pub fn github_cli_env_override() -> Option<String> {
    std::env::var("LOCUS_GH_PATH")
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

pub fn normalize_git_path(path: &Path) -> Option<PathBuf> {
    normalize_git_candidate(path).filter(|candidate| git_version_for(candidate).is_some())
}

pub fn program_in_path(program_names: &[&str]) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&path_var) {
        for name in program_names {
            if dir.join(name).is_file() {
                return true;
            }
        }
    }

    false
}

pub fn augment_path_with_git(current_path: Option<OsString>) -> Option<OsString> {
    let git = resolve_git()?;
    let mut paths: Vec<PathBuf> = current_path
        .as_ref()
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();

    let mut changed = false;
    for git_dir in git_support_dirs(&git.path).into_iter().rev() {
        if paths.iter().any(|entry| same_path(entry, &git_dir)) {
            continue;
        }
        paths.insert(0, git_dir);
        changed = true;
    }

    if !changed {
        return current_path;
    }

    std::env::join_paths(paths).ok()
}

pub fn augment_path_with_github_cli(current_path: Option<OsString>) -> Option<OsString> {
    let gh = resolve_github_cli()?;
    let parent = gh.path.parent()?.to_path_buf();
    prepend_paths(current_path, vec![parent])
}

/// Appends the machine + user `Path` values from the Windows registry so
/// CLIs installed after Locus started (which only update the registry) are
/// found without a restart. Entries already present are skipped, so paths
/// prepended by Locus (git, gh, python) keep their precedence. No-op on
/// other platforms.
#[cfg(target_os = "windows")]
pub fn augment_path_with_registry_paths(current_path: Option<OsString>) -> Option<OsString> {
    let entries = read_registry_path_entries();
    if entries.is_empty() {
        return current_path;
    }
    append_paths(current_path, entries)
}

#[cfg(not(target_os = "windows"))]
pub fn augment_path_with_registry_paths(current_path: Option<OsString>) -> Option<OsString> {
    current_path
}

#[cfg(target_os = "windows")]
fn read_registry_path_entries() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let sources = [
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
        (HKEY_CURRENT_USER, r"Environment"),
    ];

    // PATH pieces may reference variables registered alongside them
    // (e.g. `%JAVA_HOME%\bin`), so expand against the registry env too.
    let extra = read_registry_env_entries();
    let mut entries = Vec::new();
    for (hive, key_path) in sources {
        let Ok(key) = RegKey::predef(hive).open_subkey(key_path) else {
            continue;
        };
        let Ok(raw) = key.get_value::<String, _>("Path") else {
            continue;
        };
        for piece in raw.split(';') {
            let expanded = expand_windows_env(piece.trim(), &extra);
            if expanded.is_empty() {
                continue;
            }
            entries.push(PathBuf::from(expanded));
        }
    }
    entries
}

/// All machine + user environment values from the Windows registry except
/// `Path`, expanded, with user values overriding machine values — the same
/// resolution a fresh Windows session performs.
#[cfg(target_os = "windows")]
pub fn read_registry_env_entries() -> Vec<(String, String)> {
    let raw = read_registry_env_raw();
    raw.iter()
        .map(|(key, value)| (key.clone(), expand_windows_env(value, &raw)))
        .collect()
}

#[cfg(target_os = "windows")]
fn read_registry_env_raw() -> Vec<(String, String)> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::types::FromRegValue;
    use winreg::RegKey;

    let sources = [
        (
            HKEY_LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
        ),
        (HKEY_CURRENT_USER, r"Environment"),
    ];

    let mut entries: Vec<(String, String)> = Vec::new();
    for (hive, key_path) in sources {
        let Ok(key) = RegKey::predef(hive).open_subkey(key_path) else {
            continue;
        };
        for (name, raw_value) in key.enum_values().flatten() {
            if name.eq_ignore_ascii_case("Path") {
                continue;
            }
            // Only string-typed values participate in the environment block.
            let Ok(value) = String::from_reg_value(&raw_value) else {
                continue;
            };
            if let Some(existing) = entries
                .iter_mut()
                .find(|(key, _)| key.eq_ignore_ascii_case(&name))
            {
                existing.1 = value;
            } else {
                entries.push((name, value));
            }
        }
    }
    entries
}

// Registry env values are often REG_EXPAND_SZ; winreg returns them
// unexpanded. Lookup prefers `extra` (same-batch registry entries) over the
// process environment.
#[cfg(target_os = "windows")]
fn expand_windows_env(value: &str, extra: &[(String, String)]) -> String {
    let lookup = |name: &str| -> Option<String> {
        extra
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
            .or_else(|| {
                std::env::vars_os()
                    .find(|(key, _)| key.to_str().is_some_and(|k| k.eq_ignore_ascii_case(name)))
                    .map(|(_, value)| value.to_string_lossy().into_owned())
            })
    };

    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            out.push('%');
            rest = after;
            continue;
        };
        let name = &after[..end];
        if name.is_empty() {
            out.push('%');
        } else if let Some(value) = lookup(name) {
            out.push_str(&value);
        } else {
            out.push('%');
            out.push_str(name);
            out.push('%');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub fn append_paths(current_path: Option<OsString>, entries: Vec<PathBuf>) -> Option<OsString> {
    let mut paths: Vec<PathBuf> = current_path
        .as_ref()
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();

    let mut changed = false;
    for entry in entries {
        if paths.iter().any(|existing| same_path(existing, &entry)) {
            continue;
        }
        paths.push(entry);
        changed = true;
    }

    if !changed {
        return current_path;
    }

    std::env::join_paths(paths).ok()
}

pub fn prepend_paths(current_path: Option<OsString>, entries: Vec<PathBuf>) -> Option<OsString> {
    let mut paths: Vec<PathBuf> = current_path
        .as_ref()
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default();

    let mut changed = false;
    for entry in entries.into_iter().rev() {
        if paths.iter().any(|existing| same_path(existing, &entry)) {
            continue;
        }
        paths.insert(0, entry);
        changed = true;
    }

    if !changed {
        return current_path;
    }

    std::env::join_paths(paths).ok()
}

fn resolve_program(program: &str) -> OsString {
    if program.eq_ignore_ascii_case("git") {
        if let Some(git) = resolve_git() {
            return git.path.into_os_string();
        }
    }
    if program.eq_ignore_ascii_case("gh") {
        if let Some(gh) = resolve_github_cli() {
            return gh.path.into_os_string();
        }
    }
    OsString::from(program)
}

fn git_resolution_cache() -> &'static Mutex<GitResolutionCache> {
    static CACHE: OnceLock<Mutex<GitResolutionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn managed_git_resource_dirs() -> &'static Mutex<Vec<PathBuf>> {
    static DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(Vec::new()))
}

fn github_cli_resolution_cache() -> &'static Mutex<GithubCliResolutionCache> {
    static CACHE: OnceLock<Mutex<GithubCliResolutionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn managed_github_cli_resource_dirs() -> &'static Mutex<Vec<PathBuf>> {
    static DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(Vec::new()))
}

fn discover_git() -> Option<ResolvedGit> {
    resolve_git_from_env()
        .or_else(resolve_git_from_path)
        .or_else(resolve_git_from_common_locations)
        .or_else(resolve_git_from_managed_resource)
}

fn discover_github_cli() -> Option<ResolvedGithubCli> {
    resolve_github_cli_from_env()
        .or_else(resolve_github_cli_from_managed_resource)
        .or_else(resolve_github_cli_from_path)
}

fn git_version_for(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        suppress_command_window(&mut cmd);
    }
    let output = command_output_with_timeout(cmd, GIT_VERSION_TIMEOUT)
        .ok()
        .flatten()?;

    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

fn github_cli_version_for(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .env("GH_TELEMETRY", "false")
        .env("DO_NOT_TRACK", "true")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        suppress_command_window(&mut cmd);
    }
    let output = command_output_with_timeout(cmd, GITHUB_CLI_VERSION_TIMEOUT)
        .ok()
        .flatten()?;

    if !output.status.success() {
        return None;
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    version.starts_with("gh version ").then_some(version)
}

fn command_output_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    command.stdin(Stdio::null());
    let mut child = command.spawn()?;
    let started_at = Instant::now();

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output().map(Some);
        }

        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }

        std::thread::sleep(Duration::from_millis(25));
    }
}

fn resolve_git_from_env() -> Option<ResolvedGit> {
    let raw = git_env_override()?;
    let path = PathBuf::from(raw);
    normalize_git_path(&path).map(|path| ResolvedGit {
        path,
        source: GitDiscoverySource::EnvOverride,
    })
}

fn resolve_github_cli_from_env() -> Option<ResolvedGithubCli> {
    let raw = github_cli_env_override()?;
    let path = PathBuf::from(raw);
    normalize_github_cli_candidate(&path).map(|path| ResolvedGithubCli {
        path,
        source: GithubCliDiscoverySource::EnvOverride,
    })
}

fn push_git_runtime_candidate(
    target: &mut Vec<GitRuntimeCandidate>,
    candidate: PathBuf,
    source: GitDiscoverySource,
) {
    let Some(path) = normalize_git_candidate(&candidate) else {
        return;
    };
    let path = dunce::canonicalize(&path).unwrap_or(path);
    if target
        .iter()
        .any(|existing| git_runtime_key(&existing.path) == git_runtime_key(&path))
    {
        return;
    }
    let Some(version) = git_version_for(&path) else {
        return;
    };

    target.push(GitRuntimeCandidate {
        path,
        source,
        version,
    });
}

fn resolve_first_github_cli_candidate(
    candidates: Vec<PathBuf>,
    source: GithubCliDiscoverySource,
) -> Option<ResolvedGithubCli> {
    for candidate in candidates {
        let Some(path) = normalize_github_cli_candidate(&candidate) else {
            continue;
        };
        let path = dunce::canonicalize(&path).unwrap_or(path);
        if github_cli_version_for(&path).is_some() {
            return Some(ResolvedGithubCli { path, source });
        }
    }
    None
}

fn resolve_first_git_candidate(
    candidates: Vec<PathBuf>,
    source: GitDiscoverySource,
) -> Option<ResolvedGit> {
    for candidate in candidates {
        if let Some(runtime) = probe_git_runtime(candidate, source) {
            return Some(ResolvedGit {
                path: runtime.path,
                source: runtime.source,
            });
        }
    }
    None
}

fn resolve_github_cli_from_managed_resource() -> Option<ResolvedGithubCli> {
    resolve_first_github_cli_candidate(
        github_cli_managed_resource_candidates(),
        GithubCliDiscoverySource::Managed,
    )
}

#[cfg(target_os = "windows")]
fn resolve_git_from_managed_resource() -> Option<ResolvedGit> {
    resolve_first_git_candidate(
        git_managed_resource_candidates(),
        GitDiscoverySource::Managed,
    )
}

#[cfg(not(target_os = "windows"))]
fn resolve_git_from_managed_resource() -> Option<ResolvedGit> {
    None
}

#[cfg(target_os = "windows")]
fn managed_git_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(registered) = managed_git_resource_dirs().lock() {
        for root in registered.iter() {
            push_managed_git_root_candidates(&mut roots, root);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            push_managed_git_root_candidates(&mut roots, exe_dir);
            push_managed_git_root_candidates(&mut roots, &exe_dir.join("resources"));
        }
    }

    #[cfg(debug_assertions)]
    push_managed_git_root_candidates(
        &mut roots,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("gen"),
    );

    let mut unique: Vec<PathBuf> = Vec::new();
    for root in roots {
        if root.is_dir() && !unique.iter().any(|existing| same_path(existing, &root)) {
            unique.push(root);
        }
    }
    unique
}

#[cfg(target_os = "windows")]
fn push_managed_git_root_candidates(target: &mut Vec<PathBuf>, base: &Path) {
    target.push(base.join("managed-git").join("windows-x64"));
}

fn managed_github_cli_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    if let Ok(registered) = managed_github_cli_resource_dirs().lock() {
        for root in registered.iter() {
            push_managed_github_cli_root_candidates(&mut roots, root);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            push_managed_github_cli_root_candidates(&mut roots, exe_dir);
            push_managed_github_cli_root_candidates(&mut roots, &exe_dir.join("resources"));
        }
    }

    #[cfg(debug_assertions)]
    push_managed_github_cli_root_candidates(
        &mut roots,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("gen"),
    );

    let mut unique: Vec<PathBuf> = Vec::new();
    for root in roots {
        if root.is_dir() && !unique.iter().any(|existing| same_path(existing, &root)) {
            unique.push(root);
        }
    }
    unique
}

fn push_managed_github_cli_root_candidates(target: &mut Vec<PathBuf>, base: &Path) {
    if let Some(runtime_id) = github_cli_runtime_id() {
        target.push(base.join("gh-runtime").join(runtime_id));
    }
}

fn github_cli_managed_resource_candidates() -> Vec<PathBuf> {
    managed_github_cli_roots()
        .into_iter()
        .flat_map(|root| github_cli_candidates_inside(&root))
        .collect()
}

#[cfg(target_os = "windows")]
fn git_managed_resource_candidates() -> Vec<PathBuf> {
    managed_git_roots()
        .into_iter()
        .flat_map(|root| git_candidates_inside(&root))
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn git_managed_resource_candidates() -> Vec<PathBuf> {
    Vec::new()
}

fn resolve_git_from_path() -> Option<ResolvedGit> {
    resolve_first_git_candidate(git_path_candidates(), GitDiscoverySource::Path)
}

fn resolve_github_cli_from_path() -> Option<ResolvedGithubCli> {
    resolve_first_github_cli_candidate(github_cli_path_candidates(), GithubCliDiscoverySource::Path)
}

fn git_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(path_var) = std::env::var_os("PATH") else {
        return candidates;
    };

    for dir in std::env::split_paths(&path_var) {
        for name in git_binary_names() {
            candidates.push(dir.join(name));
        }
    }

    candidates
}

fn github_cli_path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(path_var) = std::env::var_os("PATH") else {
        return candidates;
    };

    for dir in std::env::split_paths(&path_var) {
        for name in github_cli_binary_names() {
            candidates.push(dir.join(name));
        }
    }

    candidates
}

fn resolve_git_from_common_locations() -> Option<ResolvedGit> {
    resolve_first_git_candidate(
        git_common_location_candidates(),
        GitDiscoverySource::CommonLocation,
    )
}

#[cfg(target_os = "windows")]
fn git_common_location_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    push_git_registry_candidates(&mut candidates);

    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        push_git_root_candidates(&mut candidates, &PathBuf::from(program_files).join("Git"));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        push_git_root_candidates(
            &mut candidates,
            &PathBuf::from(program_files_x86).join("Git"),
        );
    }
    if let Some(local_app_data) = std::env::var_os("LocalAppData") {
        let local_app_data = PathBuf::from(local_app_data);
        push_git_root_candidates(
            &mut candidates,
            &local_app_data.join("Programs").join("Git"),
        );
        push_github_desktop_candidates(&mut candidates, &local_app_data.join("GitHubDesktop"));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        push_git_root_candidates(
            &mut candidates,
            &PathBuf::from(user_profile)
                .join("scoop")
                .join("apps")
                .join("git")
                .join("current"),
        );
    }
    if let Some(choco_root) = std::env::var_os("ChocolateyInstall") {
        candidates.push(PathBuf::from(choco_root).join("bin").join("git.exe"));
    }

    candidates
}

#[cfg(not(target_os = "windows"))]
fn git_common_location_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn push_git_root_candidates(target: &mut Vec<PathBuf>, root: &Path) {
    target.push(root.join("cmd").join("git.exe"));
    target.push(root.join("bin").join("git.exe"));
    target.push(root.join("mingw64").join("bin").join("git.exe"));
}

#[cfg(target_os = "windows")]
fn push_git_registry_candidates(target: &mut Vec<PathBuf>) {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        for key_path in [
            r"SOFTWARE\GitForWindows",
            r"SOFTWARE\WOW6432Node\GitForWindows",
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Git_is1",
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Git_is1",
        ] {
            let Ok(key) = root.open_subkey(key_path) else {
                continue;
            };

            for value_name in ["InstallPath", "InstallLocation"] {
                let Ok(raw) = key.get_value::<String, _>(value_name) else {
                    continue;
                };
                let trimmed = raw.trim().trim_matches('"');
                if trimmed.is_empty() {
                    continue;
                }
                let path = PathBuf::from(trimmed);
                if path.is_dir() {
                    push_git_root_candidates(target, &path);
                } else {
                    target.push(path);
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn push_github_desktop_candidates(target: &mut Vec<PathBuf>, github_desktop_root: &Path) {
    let Ok(entries) = std::fs::read_dir(github_desktop_root) else {
        return;
    };

    let mut app_dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("app-"))
                .unwrap_or(false)
        })
        .collect();

    app_dirs.sort();
    app_dirs.reverse();

    for dir in app_dirs {
        let git_root = dir.join("resources").join("app").join("git");
        push_git_root_candidates(target, &git_root);
    }
}

fn normalize_git_candidate(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }

    if path.is_dir() {
        for candidate in git_candidates_inside(path) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        return None;
    }

    for candidate in git_candidates_from_hint(path) {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn normalize_github_cli_candidate(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return Some(path.to_path_buf());
    }

    if path.is_dir() {
        for candidate in github_cli_candidates_inside(path) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        return None;
    }

    for candidate in github_cli_candidates_from_hint(path) {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn git_candidates_inside(root: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        vec![
            root.join("cmd").join("git.exe"),
            root.join("bin").join("git.exe"),
            root.join("mingw64").join("bin").join("git.exe"),
            root.join("git.exe"),
            root.join("git.cmd"),
            root.join("git.bat"),
        ]
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![root.join("git"), root.join("bin").join("git")]
    }
}

fn github_cli_candidates_inside(root: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        vec![
            root.join("bin").join("gh.exe"),
            root.join("gh.exe"),
            root.join("gh.cmd"),
            root.join("gh.bat"),
        ]
    }

    #[cfg(not(target_os = "windows"))]
    {
        vec![root.join("bin").join("gh"), root.join("gh")]
    }
}

fn git_candidates_from_hint(path: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if path.extension().is_none() {
            return vec![
                path.with_extension("exe"),
                path.with_extension("cmd"),
                path.with_extension("bat"),
                path.to_path_buf(),
            ];
        }
    }

    vec![path.to_path_buf()]
}

fn github_cli_candidates_from_hint(path: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if path.extension().is_none() {
            return vec![
                path.with_extension("exe"),
                path.with_extension("cmd"),
                path.with_extension("bat"),
                path.to_path_buf(),
            ];
        }
    }

    vec![path.to_path_buf()]
}

fn git_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["git.exe", "git.cmd", "git.bat"]
    }

    #[cfg(not(target_os = "windows"))]
    {
        &["git"]
    }
}

fn github_cli_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["gh.exe", "gh.cmd", "gh.bat"]
    }

    #[cfg(not(target_os = "windows"))]
    {
        &["gh"]
    }
}

fn github_cli_runtime_id() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x64");
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Some("windows-arm64");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("macos-x64");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("macos-arm64");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-arm64");
    }
    #[allow(unreachable_code)]
    None
}

fn git_support_dirs(git_path: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(parent) = git_path.parent() {
        dirs.push(parent.to_path_buf());
    }

    #[cfg(target_os = "windows")]
    if let Some(root) = git_root_from_path(git_path) {
        for rel in [
            PathBuf::from("cmd"),
            PathBuf::from("bin"),
            PathBuf::from("usr").join("bin"),
            PathBuf::from("mingw64").join("bin"),
        ] {
            let dir = root.join(rel);
            if dir.is_dir() && !dirs.iter().any(|existing| same_path(existing, &dir)) {
                dirs.push(dir);
            }
        }
    }

    dirs
}

fn git_runtime_identity_path(path: &Path) -> PathBuf {
    let path = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());

    #[cfg(target_os = "windows")]
    if let Some(root) = git_root_from_path(&path) {
        return root;
    }

    path
}

#[cfg(target_os = "windows")]
fn git_root_from_path(git_path: &Path) -> Option<PathBuf> {
    let mut current = git_path.parent();
    for _ in 0..4 {
        let dir = current?;
        if dir.join("cmd").join("git.exe").is_file()
            || dir.join("bin").join("git.exe").is_file()
            || dir.join("mingw64").join("bin").join("git.exe").is_file()
        {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

fn same_path(left: &Path, right: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        let left = left.to_string_lossy().to_ascii_lowercase();
        let right = right.to_string_lossy().to_ascii_lowercase();
        left == right
    }

    #[cfg(not(target_os = "windows"))]
    {
        left == right
    }
}

#[cfg(test)]
fn set_git_resolution_cache_for_test(value: GitResolutionCache) {
    let cache = git_resolution_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = value;
}

#[cfg(test)]
mod tests {
    use super::{
        refresh_git_resolution, resolve_git, set_git_resolution_cache_for_test, GitDiscoverySource,
    };

    #[test]
    fn append_paths_skips_existing_and_appends_new_entries() {
        use std::path::PathBuf;

        let current =
            std::env::join_paths([PathBuf::from("/locus/bin"), PathBuf::from("/usr/bin")])
                .expect("join test paths");
        let result = super::append_paths(
            Some(current),
            vec![PathBuf::from("/usr/bin"), PathBuf::from("/opt/new/bin")],
        )
        .expect("paths should join");

        let parts: Vec<PathBuf> = std::env::split_paths(&result).collect();
        assert_eq!(
            parts,
            vec![
                PathBuf::from("/locus/bin"),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/opt/new/bin"),
            ]
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_windows_env_expands_known_variables() {
        let system_root = std::env::var("SystemRoot").expect("SystemRoot should be set");
        assert_eq!(
            super::expand_windows_env("%SystemRoot%\\system32", &[]),
            format!("{}\\system32", system_root)
        );
        assert_eq!(
            super::expand_windows_env("%LOCUS_NOT_A_REAL_VAR%", &[]),
            "%LOCUS_NOT_A_REAL_VAR%"
        );
        assert_eq!(super::expand_windows_env("plain", &[]), "plain");

        // Same-batch registry entries win over the process environment and
        // match case-insensitively.
        let extra = vec![("JAVA_HOME".to_string(), "C:\\jdk".to_string())];
        assert_eq!(
            super::expand_windows_env("%java_home%\\bin", &extra),
            "C:\\jdk\\bin"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_path_entries_are_available() {
        let entries = super::read_registry_path_entries();
        assert!(
            !entries.is_empty(),
            "machine/user registry PATH should produce entries"
        );
        assert!(
            entries.iter().all(|entry| !entry.as_os_str().is_empty()),
            "expanded entries should not be empty"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn registry_env_entries_exclude_path() {
        let entries = super::read_registry_env_entries();
        assert!(
            !entries.is_empty(),
            "machine/user registry env should produce entries"
        );
        assert!(
            entries
                .iter()
                .all(|(key, _)| !key.eq_ignore_ascii_case("Path")),
            "Path merges separately and must not appear"
        );
    }

    #[test]
    fn refresh_git_resolution_replaces_cached_missing_result() {
        let Some(actual) = refresh_git_resolution() else {
            return;
        };

        set_git_resolution_cache_for_test(Some(None));

        let refreshed = refresh_git_resolution().expect("git should be rediscovered");
        assert_eq!(refreshed.path, actual.path);
        assert_eq!(refreshed.source, actual.source);

        let cached = resolve_git().expect("refreshed git should be cached");
        assert_eq!(cached.path, actual.path);
        assert_eq!(cached.source, actual.source);
    }

    #[test]
    fn resolve_git_uses_refreshed_env_override_cache() {
        let Some(actual) = refresh_git_resolution() else {
            return;
        };

        set_git_resolution_cache_for_test(Some(Some(super::ResolvedGit {
            path: actual.path.clone(),
            source: GitDiscoverySource::EnvOverride,
        })));

        let resolved = resolve_git().expect("cached git should resolve");
        assert_eq!(resolved.path, actual.path);
        assert_eq!(resolved.source, GitDiscoverySource::EnvOverride);

        set_git_resolution_cache_for_test(Some(Some(actual)));
    }
}
