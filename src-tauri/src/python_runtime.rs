use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const CONFIG_FILE: &str = "python_runtime_config.json";
const MANAGED_RESOURCE_DIR: &str = "managed-python";
const MANAGED_WINDOWS_X64_ID: &str = "managed:windows-x64";
const MANAGED_PIP_ZIPAPP: &str = "pip.pyz";
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
    pub home: Option<PathBuf>,
    pub package_dir: Option<PathBuf>,
    pub pip_zipapp: Option<PathBuf>,
}

type PythonRuntimeDiscoveryCache = Option<Vec<PythonRuntimeInfo>>;

pub fn config_file_name() -> &'static str {
    CONFIG_FILE
}

pub fn python_runtime_state(app_handle: Option<&AppHandle>) -> Result<PythonRuntimeState, String> {
    python_runtime_state_with_refresh(app_handle, false)
}

pub fn python_runtime_state_with_refresh(
    app_handle: Option<&AppHandle>,
    refresh: bool,
) -> Result<PythonRuntimeState, String> {
    python_runtime_state_with_options(app_handle, refresh, true)
}

pub fn python_runtime_state_with_options(
    app_handle: Option<&AppHandle>,
    refresh: bool,
    discover: bool,
) -> Result<PythonRuntimeState, String> {
    if !discover {
        return current_python_runtime_state(app_handle);
    }

    let config = load_config().unwrap_or_default();
    let mut runtimes = discover_python_runtimes_cached(app_handle, refresh);

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

fn current_python_runtime_state(
    app_handle: Option<&AppHandle>,
) -> Result<PythonRuntimeState, String> {
    let config = load_config().unwrap_or_default();
    let mut runtimes = Vec::new();
    let managed_path = managed_python_executable_path(app_handle);

    if let Some(path) = config
        .selected_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        let source = if config.selected_id.as_deref() == Some(MANAGED_WINDOWS_X64_ID)
            || managed_path
                .as_ref()
                .is_some_and(|managed| same_path_str(&path.display().to_string(), managed))
        {
            PythonRuntimeSource::Managed
        } else {
            PythonRuntimeSource::System
        };
        runtimes.push(lightweight_runtime_info(
            config
                .selected_id
                .clone()
                .unwrap_or_else(|| runtime_id_for_path(&path)),
            source,
            path,
        ));
        if let Some(managed) = managed_path {
            let already_listed = runtimes
                .iter()
                .any(|runtime| same_path_str(&runtime.path, &managed));
            if !already_listed {
                runtimes.push(lightweight_runtime_info(
                    MANAGED_WINDOWS_X64_ID.to_string(),
                    PythonRuntimeSource::Managed,
                    managed,
                ));
            }
        }
    } else if let Some(path) = managed_path {
        runtimes.push(lightweight_runtime_info(
            MANAGED_WINDOWS_X64_ID.to_string(),
            PythonRuntimeSource::Managed,
            path,
        ));
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

fn lightweight_runtime_info(
    id: String,
    source: PythonRuntimeSource,
    path: PathBuf,
) -> PythonRuntimeInfo {
    let available = path.is_file();
    let canonical = dunce::canonicalize(&path).unwrap_or(path);
    PythonRuntimeInfo {
        id,
        label: runtime_label(source.clone(), None),
        path: canonical.display().to_string(),
        version: None,
        source,
        selected: false,
        available,
    }
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

    python_runtime_state_with_refresh(app_handle, false)
}

pub fn resolve_effective_python(app_handle: Option<&AppHandle>) -> Option<ResolvedPythonRuntime> {
    let state = python_runtime_state(app_handle).ok()?;
    let effective = state.effective?;
    if !effective.available {
        return None;
    }
    let source = effective.source;
    let path = PathBuf::from(effective.path);
    let version = effective.version;
    let home = if source == PythonRuntimeSource::Managed {
        path.parent().map(Path::to_path_buf)
    } else {
        None
    };
    let package_dir = if source == PythonRuntimeSource::Managed {
        managed_python_package_dir(app_handle, version.as_deref())
    } else {
        None
    };
    let pip_zipapp = if source == PythonRuntimeSource::Managed {
        managed_python_pip_zipapp_path(app_handle)
    } else {
        None
    };
    Some(ResolvedPythonRuntime {
        path,
        version,
        source,
        home,
        package_dir,
        pip_zipapp,
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
        let pip_shim = format!("@echo off\r\n\"{}\" -m pip %*\r\n", target);
        std::fs::write(dir.join("python.cmd"), &shim).ok()?;
        std::fs::write(dir.join("python3.cmd"), shim).ok()?;
        std::fs::write(dir.join("pip.cmd"), &pip_shim).ok()?;
        std::fs::write(dir.join("pip3.cmd"), pip_shim).ok()?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let target = shell_quote_posix(&python_path.display().to_string());
        let shim = format!("#!/bin/sh\nexec {} \"$@\"\n", target);
        let pip_shim = format!("#!/bin/sh\nexec {} -m pip \"$@\"\n", target);
        let python = dir.join("python");
        let python3 = dir.join("python3");
        let pip = dir.join("pip");
        let pip3 = dir.join("pip3");
        std::fs::write(&python, &shim).ok()?;
        std::fs::write(&python3, &shim).ok()?;
        std::fs::write(&pip, &pip_shim).ok()?;
        std::fs::write(&pip3, &pip_shim).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            let _ = std::fs::set_permissions(&python, mode.clone());
            let _ = std::fs::set_permissions(&python3, mode);
            let _ = std::fs::set_permissions(&pip, mode.clone());
            let _ = std::fs::set_permissions(&pip3, mode);
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
        "python() {{ {} \"$@\"; }}\npython3() {{ {} \"$@\"; }}\npip() {{ {} -m pip \"$@\"; }}\npip3() {{ {} -m pip \"$@\"; }}\n",
        executable, executable, executable, executable
    )
}

pub fn ensure_runtime_package_environment(runtime: &ResolvedPythonRuntime) -> Result<(), String> {
    if !matches!(&runtime.source, PythonRuntimeSource::Managed) {
        return Ok(());
    }

    let Some(package_dir) = runtime.package_dir.as_ref() else {
        return Ok(());
    };

    std::fs::create_dir_all(package_dir).map_err(|e| {
        format!(
            "Failed to create managed Python package dir '{}': {}",
            package_dir.display(),
            e
        )
    })?;

    if let Some(pip_zipapp) = runtime.pip_zipapp.as_ref().filter(|path| path.is_file()) {
        write_pip_module_shim(package_dir, pip_zipapp)?;
    }

    Ok(())
}

pub fn managed_python_path_env(
    current_path: Option<OsString>,
    runtime: &ResolvedPythonRuntime,
) -> Option<OsString> {
    if !matches!(&runtime.source, PythonRuntimeSource::Managed) {
        return current_path;
    }
    let package_dir = runtime.package_dir.as_ref()?.to_path_buf();
    crate::process_util::prepend_paths(current_path, vec![package_dir])
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

fn discover_python_runtimes_cached(
    app_handle: Option<&AppHandle>,
    refresh: bool,
) -> Vec<PythonRuntimeInfo> {
    let cache = python_runtime_discovery_cache();
    if !refresh {
        if let Some(cached) = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
        {
            return cached;
        }
    }

    let runtimes = discover_python_runtimes(app_handle);
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = Some(runtimes.clone());
    runtimes
}

fn python_runtime_discovery_cache() -> &'static Mutex<PythonRuntimeDiscoveryCache> {
    static CACHE: OnceLock<Mutex<PythonRuntimeDiscoveryCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
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

fn managed_python_roots(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
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
    roots
}

fn managed_python_executable_path(app_handle: Option<&AppHandle>) -> Option<PathBuf> {
    find_managed_python_executable(&managed_python_roots(app_handle))
}

fn managed_python_pip_zipapp_path(app_handle: Option<&AppHandle>) -> Option<PathBuf> {
    managed_python_roots(app_handle)
        .into_iter()
        .map(|root| root.join(MANAGED_RESOURCE_DIR).join(MANAGED_PIP_ZIPAPP))
        .find(|candidate| candidate.is_file())
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

fn managed_python_package_dir(
    app_handle: Option<&AppHandle>,
    version: Option<&str>,
) -> Option<PathBuf> {
    let data_dir = if let Some(app) = app_handle {
        crate::commands::resolve_runtime_storage_dir(app).ok()
    } else {
        crate::commands::packaged_runtime_storage_dir()
            .ok()
            .flatten()
    }?;
    Some(
        data_dir
            .join(MANAGED_RESOURCE_DIR)
            .join(managed_python_platform_id())
            .join(managed_python_version_tag(version))
            .join("site-packages"),
    )
}

fn managed_python_platform_id() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x64"
    } else {
        "default"
    }
}

fn managed_python_version_tag(version: Option<&str>) -> String {
    let Some(version) = version.map(str::trim).filter(|value| !value.is_empty()) else {
        return "python".to_string();
    };
    let mut parts = version.split('.');
    let Some(major) = parts.next().filter(|value| !value.is_empty()) else {
        return "python".to_string();
    };
    let Some(minor) = parts.next().filter(|value| !value.is_empty()) else {
        return format!("python-{}", major);
    };
    format!("python-{}.{}", major, minor)
}

fn write_pip_module_shim(package_dir: &Path, pip_zipapp: &Path) -> Result<(), String> {
    let pip_dir = package_dir.join("pip");
    std::fs::create_dir_all(&pip_dir).map_err(|e| {
        format!(
            "Failed to create pip shim dir '{}': {}",
            pip_dir.display(),
            e
        )
    })?;

    let zipapp_literal = serde_json::to_string(&pip_zipapp.display().to_string())
        .map_err(|e| format!("Failed to serialize pip zipapp path: {}", e))?;
    let init_path = pip_dir.join("__init__.py");
    if !init_path.is_file() {
        std::fs::write(&init_path, "# Locus managed Python pip shim.\n")
            .map_err(|e| format!("Failed to write pip shim '{}': {}", init_path.display(), e))?;
    }

    let main = format!(
        "import os\nimport runpy\nimport sys\n\nPIP_ZIPAPP = {zipapp_literal}\nSTUB_ROOT = os.path.dirname(os.path.dirname(__file__))\nSTUB_ROOT_KEY = os.path.normcase(os.path.normpath(STUB_ROOT))\nsys.path = [entry for entry in sys.path if os.path.normcase(os.path.normpath(entry or os.curdir)) != STUB_ROOT_KEY]\nfor name in list(sys.modules):\n    if name == 'pip' or name.startswith('pip.'):\n        del sys.modules[name]\nif PIP_ZIPAPP not in sys.path:\n    sys.path.insert(0, PIP_ZIPAPP)\nsys.argv[0] = PIP_ZIPAPP\nrunpy.run_path(PIP_ZIPAPP, run_name='__main__')\n"
    );
    let main_path = pip_dir.join("__main__.py");
    std::fs::write(&main_path, main)
        .map_err(|e| format!("Failed to write pip shim '{}': {}", main_path.display(), e))
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
    if source == PythonRuntimeSource::Managed {
        if let Some(home) = candidate.parent() {
            command.env("PYTHONHOME", home);
            command.env("PYTHONNOUSERSITE", "1");
        }
        command.env_remove("PYTHONPATH");
    }
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
    crate::process_util::suppress_command_window(&mut command);
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
        find_managed_python_executable, managed_python_executable_under,
        managed_python_version_tag, sh_python_function_prefix,
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
        assert!(prefix.contains("pip()"));
        assert!(prefix.contains("pip3()"));
        assert!(prefix.contains("'C:/Tools/Python/python.exe'"));
    }

    #[test]
    fn managed_python_version_tag_uses_major_minor() {
        assert_eq!(managed_python_version_tag(Some("3.13.12")), "python-3.13");
        assert_eq!(managed_python_version_tag(Some("3")), "python-3");
        assert_eq!(managed_python_version_tag(None), "python");
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
