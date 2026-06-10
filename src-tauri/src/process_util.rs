use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const GIT_VERSION_TIMEOUT: Duration = Duration::from_millis(1500);
const GITHUB_CLI_VERSION_TIMEOUT: Duration = Duration::from_millis(1500);

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

        let current = std::env::join_paths([PathBuf::from("/locus/bin"), PathBuf::from("/usr/bin")])
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
