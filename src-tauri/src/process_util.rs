use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitDiscoverySource {
    EnvOverride,
    Path,
    CommonLocation,
}

impl GitDiscoverySource {
    pub fn as_str(self) -> &'static str {
        match self {
            GitDiscoverySource::EnvOverride => "envOverride",
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

pub fn command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(resolve_program(program));
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn async_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(resolve_program(program));
    #[cfg(target_os = "windows")]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

pub fn resolve_git() -> Option<ResolvedGit> {
    static CACHED: OnceLock<Option<ResolvedGit>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            resolve_git_from_env()
                .or_else(resolve_git_from_path)
                .or_else(resolve_git_from_common_locations)
        })
        .clone()
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

fn resolve_program(program: &str) -> OsString {
    if program.eq_ignore_ascii_case("git") {
        if let Some(git) = resolve_git() {
            return git.path.into_os_string();
        }
    }
    OsString::from(program)
}

fn git_version_for(path: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(path);
    cmd.arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;

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

fn resolve_git_from_env() -> Option<ResolvedGit> {
    let raw = git_env_override()?;
    let path = PathBuf::from(raw);
    normalize_git_path(&path).map(|path| ResolvedGit {
        path,
        source: GitDiscoverySource::EnvOverride,
    })
}

fn resolve_git_from_path() -> Option<ResolvedGit> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        for name in git_binary_names() {
            let candidate = dir.join(name);
            if candidate.is_file() && git_version_for(&candidate).is_some() {
                return Some(ResolvedGit {
                    path: candidate,
                    source: GitDiscoverySource::Path,
                });
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn resolve_git_from_common_locations() -> Option<ResolvedGit> {
    let mut candidates = Vec::new();

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

    for candidate in candidates {
        if candidate.is_file() && git_version_for(&candidate).is_some() {
            return Some(ResolvedGit {
                path: candidate,
                source: GitDiscoverySource::CommonLocation,
            });
        }
    }

    None
}

#[cfg(not(target_os = "windows"))]
fn resolve_git_from_common_locations() -> Option<ResolvedGit> {
    None
}

#[cfg(target_os = "windows")]
fn push_git_root_candidates(target: &mut Vec<PathBuf>, root: &Path) {
    target.push(root.join("cmd").join("git.exe"));
    target.push(root.join("bin").join("git.exe"));
    target.push(root.join("mingw64").join("bin").join("git.exe"));
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
