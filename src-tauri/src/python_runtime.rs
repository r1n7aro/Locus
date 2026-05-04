use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "python_runtime_config.json";
const MANAGED_RESOURCE_DIR: &str = "managed-python";
const MANAGED_WINDOWS_X64_ID: &str = "managed:windows-x64";
const PY_LAUNCHER_TIMEOUT: Duration = Duration::from_millis(1500);
const PY_RUNTIME_PROBE_TIMEOUT: Duration = Duration::from_millis(2200);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PythonRuntimeSource {
    Managed,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonRuntimeInfo {
    pub id: String,
    pub label: String,
    pub path: String,
    pub version: Option<String>,
    pub source: PythonRuntimeSource,
    pub selected: bool,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PythonRuntimeState {
    pub runtimes: Vec<PythonRuntimeInfo>,
    pub selected_id: Option<String>,
    pub effective: Option<PythonRuntimeInfo>,
    pub missing_selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PythonRuntimeConfig {
    #[serde(default)]
    selected_id: Option<String>,
    #[serde(default)]
    selected_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ResolvedPythonRuntime {
    pub path: PathBuf,
    pub version: Option<String>,
    pub source: PythonRuntimeSource,
}

pub fn config_file_name() -> &'static str {
    CONFIG_FILE
}

pub fn python_runtime_state(app_handle: Option<&AppHandle>) -> Result<PythonRuntimeState, String> {
    let config = load_config().unwrap_or_default();
    let mut runtimes = discover_python_runtimes(app_handle);

    if let Some(path) = config
        .selected_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        let already_listed = runtimes
            .iter()
            .any(|runtime| same_path_str(&runtime.path, &path));
        if !already_listed {
            if let Some(info) = probe_python_runtime(
                runtime_id_for_path(&path),
                runtime_label(PythonRuntimeSource::System, None),
                PythonRuntimeSource::System,
                path,
            ) {
                runtimes.push(info);
            }
        }
    }

    let selected_id = config.selected_id.clone();
    let effective_id = selected_id
        .as_deref()
        .and_then(|id| {
            runtimes
                .iter()
                .find(|runtime| runtime.available && runtime.id == id)
                .map(|runtime| runtime.id.clone())
        })
        .or_else(|| {
            runtimes
                .iter()
                .find(|runtime| runtime.available && runtime.source == PythonRuntimeSource::Managed)
                .map(|runtime| runtime.id.clone())
        })
        .or_else(|| {
            runtimes
                .iter()
                .find(|runtime| runtime.available)
                .map(|runtime| runtime.id.clone())
        });

    let missing_selected = selected_id
        .as_deref()
        .map(|id| {
            !runtimes
                .iter()
                .any(|runtime| runtime.available && runtime.id == id)
        })
        .unwrap_or(false);

    for runtime in &mut runtimes {
        runtime.selected = effective_id
            .as_deref()
            .map(|id| runtime.id == id)
            .unwrap_or(false);
    }

    let effective = effective_id
        .as_deref()
        .and_then(|id| runtimes.iter().find(|runtime| runtime.id == id))
        .cloned();

    Ok(PythonRuntimeState {
        runtimes,
        selected_id: effective_id,
        effective,
        missing_selected,
    })
}

pub fn save_python_runtime_selection(
    selected_id: &str,
    app_handle: Option<&AppHandle>,
) -> Result<PythonRuntimeState, String> {
    let trimmed = selected_id.trim();
    if trimmed.is_empty() {
        return Err("Python runtime selection cannot be empty".to_string());
    }

    let state = python_runtime_state(app_handle)?;
    let Some(selected) = state
        .runtimes
        .iter()
        .find(|runtime| runtime.id == trimmed && runtime.available)
    else {
        return Err("Selected Python runtime is unavailable".to_string());
    };

    let config = PythonRuntimeConfig {
        selected_id: Some(selected.id.clone()),
        selected_path: Some(selected.path.clone()),
    };
    save_config(&config)?;

    python_runtime_state(app_handle)
}

pub fn resolve_effective_python(app_handle: Option<&AppHandle>) -> Option<ResolvedPythonRuntime> {
    let state = python_runtime_state(app_handle).ok()?;
    let effective = state.effective?;
    if !effective.available {
        return None;
    }
    Some(ResolvedPythonRuntime {
        path: PathBuf::from(effective.path),
        version: effective.version,
        source: effective.source,
    })
}

pub fn python_prompt_display(app_handle: Option<&AppHandle>) -> String {
    match resolve_effective_python(app_handle) {
        Some(runtime) => {
            let source = match runtime.source {
                PythonRuntimeSource::Managed => "managed",
                PythonRuntimeSource::System => "system",
            };
            let version = runtime.version.unwrap_or_else(|| "unknown".to_string());
            format!("{} Python {} ({})", source, version, runtime.path.display())
        }
        None => "unavailable".to_string(),
    }
}

pub fn ensure_python_shim_dir(python_path: &Path) -> Option<PathBuf> {
    let dir = crate::commands::persistent_config_dir()
        .ok()?
        .join("runtime-shims")
        .join("python");
    std::fs::create_dir_all(&dir).ok()?;

    #[cfg(target_os = "windows")]
    {
        let target = python_path.display().to_string();
        let shim = format!("@echo off\r\n\"{}\" %*\r\n", target);
        std::fs::write(dir.join("python.cmd"), &shim).ok()?;
        std::fs::write(dir.join("python3.cmd"), shim).ok()?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let target = shell_quote_posix(&python_path.display().to_string());
        let shim = format!("#!/bin/sh\nexec {} \"$@\"\n", target);
        let python = dir.join("python");
        let python3 = dir.join("python3");
        std::fs::write(&python, &shim).ok()?;
        std::fs::write(&python3, &shim).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            let _ = std::fs::set_permissions(&python, mode.clone());
            let _ = std::fs::set_permissions(&python3, mode);
        }
    }

    Some(dir)
}

pub fn prepend_python_to_path(
    current_path: Option<OsString>,
    python_path: &Path,
) -> Option<OsString> {
    let mut paths = Vec::new();
    if let Some(shim_dir) = ensure_python_shim_dir(python_path) {
        paths.push(shim_dir);
    }
    if let Some(parent) = python_path.parent() {
        paths.push(parent.to_path_buf());
    }

    crate::process_util::prepend_paths(current_path, paths)
}

pub fn sh_python_function_prefix(python_path: &Path) -> String {
    let executable = shell_quote_posix(&python_path.display().to_string().replace('\\', "/"));
    format!(
        "python() {{ {} \"$@\"; }}\npython3() {{ {} \"$@\"; }}\n",
        executable, executable
    )
}

fn config_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(CONFIG_FILE))
}

fn load_config() -> Result<PythonRuntimeConfig, String> {
    let path = config_path()?;
    let Some(raw) = std::fs::read_to_string(&path).ok() else {
        return Ok(PythonRuntimeConfig::default());
    };
    serde_json::from_str(&raw).map_err(|e| format!("Invalid Python runtime config: {}", e))
}

fn save_config(config: &PythonRuntimeConfig) -> Result<(), String> {
    let path = config_path()?;
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize Python runtime config: {}", e))?;
    std::fs::write(&path, json).map_err(|e| {
        format!(
            "Failed to save Python runtime config '{}': {}",
            path.display(),
            e
        )
    })
}

fn discover_python_runtimes(app_handle: Option<&AppHandle>) -> Vec<PythonRuntimeInfo> {
    let mut runtimes = Vec::new();
    if let Some(managed) = managed_python_runtime(app_handle) {
        runtimes.push(managed);
    }

    let mut seen = HashSet::new();
    for path in system_python_candidates() {
        let key = normalize_path_key(&path);
        if !seen.insert(key) {
            continue;
        }
        if let Some(info) = probe_python_runtime(
            runtime_id_for_path(&path),
            runtime_label(PythonRuntimeSource::System, None),
            PythonRuntimeSource::System,
            path,
        ) {
            runtimes.push(info);
        }
    }

    runtimes
}

fn managed_python_runtime(app_handle: Option<&AppHandle>) -> Option<PythonRuntimeInfo> {
    let path = managed_python_executable_path(app_handle)?;
    if let Some(mut info) = probe_python_runtime(
        MANAGED_WINDOWS_X64_ID.to_string(),
        runtime_label(PythonRuntimeSource::Managed, None),
        PythonRuntimeSource::Managed,
        path.clone(),
    ) {
        info.id = MANAGED_WINDOWS_X64_ID.to_string();
        return Some(info);
    }

    Some(PythonRuntimeInfo {
        id: MANAGED_WINDOWS_X64_ID.to_string(),
        label: runtime_label(PythonRuntimeSource::Managed, None),
        path: path.display().to_string(),
        version: None,
        source: PythonRuntimeSource::Managed,
        selected: false,
        available: false,
    })
}

fn managed_python_executable_path(app_handle: Option<&AppHandle>) -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Some(app) = app_handle {
        if let Ok(resource_dir) = app.path().resource_dir() {
            roots.push(resource_dir);
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            roots.push(exe_dir.to_path_buf());
            roots.push(exe_dir.join("resources"));
        }
    }

    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("gen"));

    find_managed_python_executable(&roots)
}

fn find_managed_python_executable(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .map(|root| managed_python_executable_under(root))
        .find(|candidate| candidate.is_file())
}

fn managed_python_executable_under(root: &Path) -> PathBuf {
    let base = root.join(MANAGED_RESOURCE_DIR).join("windows-x64");
    if cfg!(target_os = "windows") {
        base.join("python.exe")
    } else {
        base.join("bin").join("python3")
    }
}

fn system_python_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(value) = std::env::var("LOCUS_PYTHON_PATH") {
        let trimmed = value.trim().trim_matches('"');
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    candidates.extend(py_launcher_candidates());

    #[cfg(target_os = "windows")]
    let names: &[&str] = &["python.exe", "python3.exe"];
    #[cfg(not(target_os = "windows"))]
    let names: &[&str] = &["python3", "python"];

    candidates.extend(find_programs_in_path(names));
    candidates
}

fn find_programs_in_path(names: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(path_var) = std::env::var_os("PATH") else {
        return out;
    };
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                out.push(candidate);
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn py_launcher_candidates() -> Vec<PathBuf> {
    let launcher = find_programs_in_path(&["py.exe", "py"]).into_iter().next();
    let Some(launcher) = launcher else {
        return Vec::new();
    };
    let mut command = Command::new(launcher);
    command.arg("-0p");
    let output = command_output_with_timeout(command, PY_LAUNCHER_TIMEOUT);
    let Ok(output) = output else {
        return Vec::new();
    };
    let Some(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    parse_py_launcher_paths(&text)
        .into_iter()
        .map(PathBuf::from)
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn py_launcher_candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn parse_py_launcher_paths(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let bytes = trimmed.as_bytes();
            for index in 0..bytes.len().saturating_sub(2) {
                if bytes[index].is_ascii_alphabetic()
                    && bytes[index + 1] == b':'
                    && matches!(bytes[index + 2], b'\\' | b'/')
                {
                    return Some(trimmed[index..].trim().to_string());
                }
            }
            None
        })
        .collect()
}

fn probe_python_runtime(
    id: String,
    fallback_label: String,
    source: PythonRuntimeSource,
    candidate: PathBuf,
) -> Option<PythonRuntimeInfo> {
    if !candidate.is_file() {
        return None;
    }

    let mut command = Command::new(&candidate);
    command
        .arg("-c")
        .arg("import sys; print('{}.{}.{}'.format(*sys.version_info[:3])); print(sys.executable)");
    let output = command_output_with_timeout(command, PY_RUNTIME_PROBE_TIMEOUT)
        .ok()
        .flatten()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let version = lines.next().map(str::to_string);
    let executable = lines
        .next()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or(candidate);
    let path = dunce::canonicalize(&executable).unwrap_or(executable);
    let label = runtime_label(source.clone(), version.as_deref());

    Some(PythonRuntimeInfo {
        id,
        label: if label.is_empty() {
            fallback_label
        } else {
            label
        },
        path: path.display().to_string(),
        version,
        source,
        selected: false,
        available: true,
    })
}

fn command_output_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
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

fn runtime_label(source: PythonRuntimeSource, version: Option<&str>) -> String {
    let source_label = match source {
        PythonRuntimeSource::Managed => "Managed Python",
        PythonRuntimeSource::System => "System Python",
    };
    match version {
        Some(version) if !version.trim().is_empty() => format!("{} {}", source_label, version),
        _ => source_label.to_string(),
    }
}

fn runtime_id_for_path(path: &Path) -> String {
    format!("system:{}", normalize_path_key(path))
}

fn normalize_path_key(path: &Path) -> String {
    let raw = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = raw.display().to_string().replace('\\', "/");
    if cfg!(target_os = "windows") {
        text.to_ascii_lowercase()
    } else {
        text
    }
}

fn same_path_str(left: &str, right: &Path) -> bool {
    let left_key = normalize_path_key(Path::new(left));
    let right_key = normalize_path_key(right);
    left_key == right_key
}

fn shell_quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{
        find_managed_python_executable, managed_python_executable_under, sh_python_function_prefix,
    };
    use std::path::Path;

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_py_launcher_paths_with_spaces() {
        use super::parse_py_launcher_paths;

        let output = r#"
 -V:3.13 *        C:\Users\Test User\AppData\Local\Programs\Python\Python313\python.exe
 -V:3.12          C:\Python312\python.exe
"#;
        assert_eq!(
            parse_py_launcher_paths(output),
            vec![
                r"C:\Users\Test User\AppData\Local\Programs\Python\Python313\python.exe",
                r"C:\Python312\python.exe"
            ]
        );
    }

    #[test]
    fn sh_prefix_defines_python_and_python3_functions() {
        let prefix = sh_python_function_prefix(Path::new("C:/Tools/Python/python.exe"));
        assert!(prefix.contains("python()"));
        assert!(prefix.contains("python3()"));
        assert!(prefix.contains("'C:/Tools/Python/python.exe'"));
    }

    #[test]
    fn managed_python_lookup_ignores_missing_runtime() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("gen");
        std::fs::create_dir_all(&root).expect("create root");

        assert_eq!(find_managed_python_executable(&[root]), None);
    }

    #[test]
    fn managed_python_lookup_returns_existing_runtime() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().to_path_buf();
        let expected = managed_python_executable_under(&root);
        std::fs::create_dir_all(expected.parent().expect("runtime parent"))
            .expect("create runtime parent");
        std::fs::write(&expected, b"").expect("write runtime marker");

        assert_eq!(find_managed_python_executable(&[root]), Some(expected));
    }
}
