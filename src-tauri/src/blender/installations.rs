//! Blender installation table (app level).
//!
//! Sources: Program Files, Steam libraries, PATH, the `.blend` file
//! association, running processes, and paths added in Settings. Each
//! `blender.exe` is probed once by running Blender itself with `probe.py`
//! (Blender reports its version, Python and user scripts directory, which
//! custom builds name differently); results are cached by path, size and
//! modification time in `blender_installations.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::blend_header::parse_version;

pub const INSTALLATIONS_FILE: &str = "blender_installations.json";
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
pub const MIN_SUPPORTED: (u32, u32) = (4, 2);
/// Newest Blender line verified with this host; newer ones are "unverified".
pub const TESTED_MAX: (u32, u32) = (5, 2);

/// `probe.py` prints snake_case keys; the cache stores camelCase. Both read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    pub version: Vec<u32>,
    #[serde(alias = "version_string")]
    pub version_string: String,
    #[serde(default, alias = "version_cycle")]
    pub version_cycle: Option<String>,
    #[serde(default, alias = "build_hash")]
    pub build_hash: String,
    #[serde(default, alias = "build_branch")]
    pub build_branch: String,
    #[serde(default, alias = "build_date")]
    pub build_date: String,
    pub python: String,
    #[serde(alias = "user_scripts")]
    pub user_scripts: String,
    #[serde(default, alias = "user_config")]
    pub user_config: String,
    #[serde(default)]
    pub portable: bool,
    #[serde(default)]
    pub binary: String,
}

impl ProbeInfo {
    pub fn major_minor(&self) -> (u32, u32) {
        (
            self.version.first().copied().unwrap_or(0),
            self.version.get(1).copied().unwrap_or(0),
        )
    }

    /// `4.2.1`, plus the build branch for custom builds.
    pub fn display_version(&self) -> String {
        let numbers = self
            .version
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        let custom = self
            .version_string
            .split_whitespace()
            .next()
            .and_then(|head| head.strip_prefix(&numbers))
            .map(|rest| rest.trim_start_matches('-'))
            .filter(|rest| !rest.is_empty());
        match custom {
            Some(custom) => format!("{numbers} {custom}"),
            None => numbers,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProbeCacheEntry {
    size: u64,
    modified_ms: i64,
    #[serde(default)]
    probe: Option<ProbeInfo>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredState {
    #[serde(default)]
    manual: Vec<String>,
    #[serde(default)]
    hidden: Vec<String>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    probes: BTreeMap<String, ProbeCacheEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationSource {
    Manual,
    Running,
    ProgramFiles,
    Steam,
    Path,
    FileAssociation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportStatus {
    Supported,
    Unverified,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Ready,
    Pending,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installation {
    /// Normalized path; stable id of the installation.
    pub id: String,
    pub path: String,
    pub sources: Vec<InstallationSource>,
    pub hidden: bool,
    pub is_default: bool,
    pub probe_state: ProbeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<ProbeInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe_error: Option<String>,
    pub support: SupportStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_version: Option<String>,
    /// Host add-on state in this installation's user scripts directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addon: Option<super::addon::AddonState>,
}

pub fn support_status(probe: Option<&ProbeInfo>) -> SupportStatus {
    let Some(probe) = probe else {
        return SupportStatus::Unknown;
    };
    let version = probe.major_minor();
    if version < MIN_SUPPORTED {
        SupportStatus::Unsupported
    } else if version > TESTED_MAX {
        SupportStatus::Unverified
    } else {
        SupportStatus::Supported
    }
}

pub fn normalize_path(path: &Path) -> String {
    let resolved = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolved.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
}

fn state_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn state_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(INSTALLATIONS_FILE))
}

fn load_state() -> StoredState {
    state_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_state(state: &StoredState) -> Result<(), String> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::config::atomic_write_config(
        &path,
        &serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?,
    )
}

fn file_stamp(path: &Path) -> Option<(u64, i64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    Some((metadata.len(), modified))
}

// ── discovery ──────────────────────────────────────────────────────────────

fn push(found: &mut BTreeMap<String, (PathBuf, BTreeSet<InstallationSource>)>, path: PathBuf, source: InstallationSource) {
    if !path.is_file() {
        return;
    }
    let key = normalize_path(&path);
    found
        .entry(key)
        .or_insert_with(|| (path.clone(), BTreeSet::new()))
        .1
        .insert(source);
}

fn program_files_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        let Some(base) = std::env::var_os(var).map(PathBuf::from) else {
            continue;
        };
        let foundation = base.join("Blender Foundation");
        if let Ok(entries) = std::fs::read_dir(&foundation) {
            for entry in entries.flatten() {
                out.push(entry.path().join("blender.exe"));
            }
        }
    }
    out
}

fn steam_candidates() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(r"C:\Program Files (x86)\Steam")];
    if let Some(base) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(base).join("Steam"));
    }
    let mut libraries = BTreeSet::new();
    for root in &roots {
        libraries.insert(root.clone());
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(vdf) {
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("\"path\"") {
                    let value = rest.trim().trim_matches('"').replace("\\\\", "\\");
                    if !value.is_empty() {
                        libraries.insert(PathBuf::from(value));
                    }
                }
            }
        }
    }
    libraries
        .into_iter()
        .map(|library| library.join("steamapps").join("common").join("Blender").join("blender.exe"))
        .collect()
}

fn path_candidates() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).map(|dir| dir.join("blender.exe")).collect())
        .unwrap_or_default()
}

/// `blender.exe` behind the `.blend` association (`blender-launcher.exe`
/// maps to its sibling `blender.exe`).
fn file_association_candidates() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        for key in [r"HKCR\blendfile\shell\open\command", r"HKCU\Software\Classes\blendfile\shell\open\command"] {
            let Ok(output) = crate::process_util::command("reg")
                .args(["query", key, "/ve"])
                .output()
            else {
                continue;
            };
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let Some(index) = line.find("REG_") else { continue };
                let value = line[index..].splitn(2, char::is_whitespace).nth(1).unwrap_or("").trim();
                let exe = if let Some(stripped) = value.strip_prefix('"') {
                    stripped.split('"').next().unwrap_or("")
                } else {
                    value.split_whitespace().next().unwrap_or("")
                };
                if exe.is_empty() {
                    continue;
                }
                let path = PathBuf::from(exe);
                let blender = path.with_file_name("blender.exe");
                out.push(blender);
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn running_candidates() -> Vec<PathBuf> {
    super::process::list_named("blender.exe")
        .into_iter()
        .filter_map(|process| process.image_path.map(PathBuf::from))
        .collect()
}

fn discover(state: &StoredState) -> BTreeMap<String, (PathBuf, BTreeSet<InstallationSource>)> {
    let mut found = BTreeMap::new();
    for path in &state.manual {
        push(&mut found, PathBuf::from(path), InstallationSource::Manual);
    }
    for path in running_candidates() {
        push(&mut found, path, InstallationSource::Running);
    }
    for path in program_files_candidates() {
        push(&mut found, path, InstallationSource::ProgramFiles);
    }
    for path in steam_candidates() {
        push(&mut found, path, InstallationSource::Steam);
    }
    for path in path_candidates() {
        push(&mut found, path, InstallationSource::Path);
    }
    for path in file_association_candidates() {
        push(&mut found, path, InstallationSource::FileAssociation);
    }
    found
}

// ── probing ────────────────────────────────────────────────────────────────

/// Run `exe -b --factory-startup --python probe.py` and parse its line.
pub async fn probe(exe: &Path) -> Result<ProbeInfo, String> {
    let script = super::resources::probe_script()
        .ok_or_else(|| "Blender host sources are missing (probe.py)".to_string())?;
    let mut command = crate::process_util::async_command(&exe.to_string_lossy());
    command
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .map_err(|_| format!("probe timed out after {}s", PROBE_TIMEOUT.as_secs()))?
        .map_err(|e| format!("failed to start {}: {e}", exe.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("LOCUS_BLENDER_PROBE "))
        .ok_or_else(|| {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            if detail.is_empty() {
                "not a Blender executable (no probe output)".to_string()
            } else {
                format!("probe failed: {detail}")
            }
        })?;
    serde_json::from_str::<ProbeInfo>(line).map_err(|e| format!("invalid probe output: {e}"))
}

fn cached_probe(state: &StoredState, key: &str, path: &Path) -> Option<ProbeCacheEntry> {
    let entry = state.probes.get(key)?;
    let (size, modified) = file_stamp(path)?;
    (entry.size == size && entry.modified_ms == modified).then(|| entry.clone())
}

fn probe_in_flight() -> &'static Mutex<BTreeSet<String>> {
    static SET: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    SET.get_or_init(|| Mutex::new(BTreeSet::new()))
}

async fn probe_and_store(key: String, path: PathBuf) -> Result<ProbeInfo, String> {
    let result = probe(&path).await;
    let (size, modified) = file_stamp(&path).unwrap_or((0, 0));
    let _guard = state_lock().lock().map_err(|e| e.to_string())?;
    let mut state = load_state();
    state.probes.insert(
        key,
        ProbeCacheEntry {
            size,
            modified_ms: modified,
            probe: result.as_ref().ok().cloned(),
            error: result.as_ref().err().cloned(),
        },
    );
    save_state(&state)?;
    result
}

// ── public API ─────────────────────────────────────────────────────────────

fn build_list(state: &StoredState) -> (Vec<Installation>, Vec<(String, PathBuf)>) {
    let found = discover(state);
    let hidden = state.hidden.iter().map(|p| normalize_path(Path::new(p))).collect::<BTreeSet<_>>();
    let default = state.default.as_ref().map(|p| normalize_path(Path::new(p)));
    let mut list = Vec::new();
    let mut to_probe = Vec::new();
    for (key, (path, sources)) in found {
        let cached = cached_probe(state, &key, &path);
        let (probe_state, probe, probe_error) = match cached {
            Some(entry) => match entry.probe {
                Some(probe) => (ProbeState::Ready, Some(probe), None),
                None => (ProbeState::Failed, None, entry.error),
            },
            None => {
                to_probe.push((key.clone(), path.clone()));
                (ProbeState::Pending, None, None)
            }
        };
        let addon = probe.as_ref().map(|probe| super::addon::addon_state(Path::new(&probe.user_scripts)));
        list.push(Installation {
            id: key.clone(),
            path: path.to_string_lossy().to_string(),
            sources: sources.into_iter().collect(),
            hidden: hidden.contains(&key),
            is_default: default.as_deref() == Some(key.as_str()),
            probe_state,
            support: support_status(probe.as_ref()),
            display_version: probe.as_ref().map(ProbeInfo::display_version),
            probe,
            probe_error,
            addon,
        });
    }
    list.sort_by(|a, b| {
        let va = a.probe.as_ref().map(|p| p.version.clone()).unwrap_or_default();
        let vb = b.probe.as_ref().map(|p| p.version.clone()).unwrap_or_default();
        vb.cmp(&va).then_with(|| a.path.cmp(&b.path))
    });
    (list, to_probe)
}

/// Current table. Unprobed (or changed) executables are probed in the
/// background; `on_probed` runs after each probe so callers can push the
/// updated table.
pub fn list(on_probed: Option<std::sync::Arc<dyn Fn() + Send + Sync>>) -> Vec<Installation> {
    let state = load_state();
    let (list, to_probe) = build_list(&state);
    for (key, path) in to_probe {
        let fresh = probe_in_flight().lock().map(|mut set| set.insert(key.clone())).unwrap_or(false);
        if !fresh {
            continue;
        }
        let on_probed = on_probed.clone();
        tauri::async_runtime::spawn(async move {
            let _ = probe_and_store(key.clone(), path).await;
            if let Ok(mut set) = probe_in_flight().lock() {
                set.remove(&key);
            }
            if let Some(callback) = on_probed {
                callback();
            }
        });
    }
    list
}

/// Wait until every listed executable has a probe result (used by tools and
/// the SDK, which need versions now).
pub async fn list_probed() -> Vec<Installation> {
    let state = load_state();
    let (_, to_probe) = build_list(&state);
    for (key, path) in to_probe {
        let _ = probe_and_store(key, path).await;
    }
    build_list(&load_state()).0
}

/// Why Settings > Blender > Add refused a path (mapped to error codes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    NotFound,
    /// Not laid out like a Blender installation; never executed.
    NotBlender,
    AlreadyAdded,
    TooOld(String),
    ProbeFailed(String),
    Internal(String),
}

impl AddError {
    pub fn code(&self) -> &'static str {
        match self {
            AddError::NotFound => "blender.add.not_found",
            AddError::NotBlender => "blender.add.not_blender",
            AddError::AlreadyAdded => "blender.add.duplicate",
            AddError::TooOld(_) => "blender.add.too_old",
            AddError::ProbeFailed(_) => "blender.add.probe_failed",
            AddError::Internal(_) => "blender.add.failed",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            AddError::NotFound => "blender.exe was not found at this location".to_string(),
            AddError::NotBlender => "this file is not a Blender executable".to_string(),
            AddError::AlreadyAdded => "this Blender is already in the list".to_string(),
            AddError::TooOld(version) => version.clone(),
            AddError::ProbeFailed(detail) | AddError::Internal(detail) => detail.clone(),
        }
    }
}

/// Every Blender build ships `<major>.<minor>/scripts` next to the
/// executable. Checked before probing so a wrong pick is never executed with
/// Blender's arguments.
fn has_blender_layout(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let is_version = name
            .to_string_lossy()
            .split_once('.')
            .is_some_and(|(major, minor)| {
                !major.is_empty()
                    && !minor.is_empty()
                    && major.bytes().all(|b| b.is_ascii_digit())
                    && minor.bytes().all(|b| b.is_ascii_digit())
            });
        is_version && entry.path().join("scripts").is_dir()
    })
}

/// The file a user picked → the `blender.exe` to probe: a folder or
/// `blender-launcher.exe` maps to the `blender.exe` beside it.
fn manual_executable(path: &str) -> Result<PathBuf, AddError> {
    let mut exe = PathBuf::from(path.trim());
    if exe.is_dir() {
        exe = exe.join("blender.exe");
    } else if exe
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("blender-launcher.exe"))
    {
        exe = exe.with_file_name("blender.exe");
    }
    if !exe.is_file() {
        return Err(AddError::NotFound);
    }
    if !has_blender_layout(&exe) {
        return Err(AddError::NotBlender);
    }
    Ok(exe)
}

/// Settings > Blender > Add: accept `blender.exe` or its folder; probe now.
pub async fn add_manual(path: &str) -> Result<Installation, AddError> {
    let exe = manual_executable(path)?;
    let key = normalize_path(&exe);
    {
        let state = load_state();
        if build_list(&state).0.iter().any(|item| item.id == key && item.sources.contains(&InstallationSource::Manual)) {
            return Err(AddError::AlreadyAdded);
        }
    }
    let probe = probe_and_store(key.clone(), exe.clone()).await.map_err(AddError::ProbeFailed)?;
    if probe.major_minor() < MIN_SUPPORTED {
        return Err(AddError::TooOld(probe.display_version()));
    }
    {
        let _guard = state_lock().lock().map_err(|e| AddError::Internal(e.to_string()))?;
        let mut state = load_state();
        if !state.manual.iter().any(|p| normalize_path(Path::new(p)) == key) {
            state.manual.push(exe.to_string_lossy().to_string());
        }
        state.hidden.retain(|p| normalize_path(Path::new(p)) != key);
        save_state(&state).map_err(AddError::Internal)?;
    }
    build_list(&load_state())
        .0
        .into_iter()
        .find(|item| item.id == key)
        .ok_or_else(|| AddError::Internal("installation disappeared after adding".to_string()))
}

/// Remove a manual entry, or hide an auto-detected one.
pub fn remove_or_hide(id: &str) -> Result<(), String> {
    let _guard = state_lock().lock().map_err(|e| e.to_string())?;
    let mut state = load_state();
    let before = state.manual.len();
    state.manual.retain(|p| normalize_path(Path::new(p)) != id);
    if state.manual.len() == before && !state.hidden.iter().any(|p| normalize_path(Path::new(p)) == id) {
        state.hidden.push(id.to_string());
    }
    if state.default.as_deref().map(|p| normalize_path(Path::new(p))).as_deref() == Some(id) {
        state.default = None;
    }
    save_state(&state)
}

pub fn unhide(id: &str) -> Result<(), String> {
    let _guard = state_lock().lock().map_err(|e| e.to_string())?;
    let mut state = load_state();
    state.hidden.retain(|p| normalize_path(Path::new(p)) != id);
    save_state(&state)
}

pub fn set_default(id: Option<&str>) -> Result<(), String> {
    let _guard = state_lock().lock().map_err(|e| e.to_string())?;
    let mut state = load_state();
    state.default = id.map(str::to_string);
    save_state(&state)
}

/// Forget cached probes so the next listing probes every executable again
/// (only changed executables are re-probed otherwise).
pub fn invalidate_probes() -> Result<(), String> {
    let _guard = state_lock().lock().map_err(|e| e.to_string())?;
    let mut state = load_state();
    state.probes.clear();
    save_state(&state)
}

/// Pick the installation to launch: explicit id or version first, then the
/// file's saved version, the workspace version, the default, the newest
/// supported one. Hidden and unsupported installations are skipped.
pub fn choose<'a>(
    installations: &'a [Installation],
    explicit: Option<&str>,
    wanted_version: Option<&str>,
) -> Option<&'a Installation> {
    let usable = |item: &&Installation| {
        !item.hidden && matches!(item.support, SupportStatus::Supported | SupportStatus::Unverified)
    };
    if let Some(explicit) = explicit {
        let key = normalize_path(Path::new(explicit));
        if let Some(found) = installations.iter().find(|item| item.id == key || item.id == explicit) {
            return Some(found);
        }
    }
    if let Some(version) = wanted_version {
        let wanted = parse_version(version);
        // Same major.minor first, newest patch; then the closest newer line.
        let same = installations
            .iter()
            .filter(usable)
            .filter(|item| item.probe.as_ref().is_some_and(|p| p.major_minor() == (wanted.0, wanted.1)))
            .max_by_key(|item| item.probe.as_ref().map(|p| p.version.clone()));
        if same.is_some() {
            return same;
        }
        let newer = installations
            .iter()
            .filter(usable)
            .filter(|item| item.probe.as_ref().is_some_and(|p| p.major_minor() > (wanted.0, wanted.1)))
            .min_by_key(|item| item.probe.as_ref().map(|p| p.version.clone()));
        if newer.is_some() {
            return newer;
        }
    }
    installations
        .iter()
        .filter(usable)
        .find(|item| item.is_default)
        .or_else(|| installations.iter().filter(usable).max_by_key(|item| item.probe.as_ref().map(|p| p.version.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(version: &[u32], text: &str) -> ProbeInfo {
        ProbeInfo {
            version: version.to_vec(),
            version_string: text.into(),
            version_cycle: None,
            build_hash: String::new(),
            build_branch: String::new(),
            build_date: String::new(),
            python: "3.11.7".into(),
            user_scripts: String::new(),
            user_config: String::new(),
            portable: false,
            binary: String::new(),
        }
    }

    fn installation(id: &str, version: &[u32], default: bool) -> Installation {
        let probe = probe(version, "x");
        Installation {
            id: id.into(),
            path: id.into(),
            sources: vec![InstallationSource::ProgramFiles],
            hidden: false,
            is_default: default,
            probe_state: ProbeState::Ready,
            support: support_status(Some(&probe)),
            display_version: Some(probe.display_version()),
            probe: Some(probe),
            probe_error: None,
            addon: None,
        }
    }

    #[test]
    fn probe_output_and_cache_formats_both_parse() {
        let printed = r#"{"version":[4,2,1],"version_string":"4.2.1 LTS","build_hash":"abc","python":"3.11.7","user_scripts":"C:/u/scripts","resources":{}}"#;
        let probe: ProbeInfo = serde_json::from_str(printed).unwrap();
        assert_eq!(probe.version_string, "4.2.1 LTS");
        assert_eq!(probe.user_scripts, "C:/u/scripts");
        let cached = serde_json::to_string(&probe).unwrap();
        assert!(cached.contains("versionString"));
        assert_eq!(serde_json::from_str::<ProbeInfo>(&cached).unwrap(), probe);
    }

    #[test]
    fn manual_add_requires_blender_layout_before_running_anything() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notepad.exe");
        std::fs::write(&other, b"MZ").unwrap();
        assert_eq!(manual_executable(&other.to_string_lossy()), Err(AddError::NotBlender));
        assert_eq!(
            manual_executable(&dir.path().join("missing.exe").to_string_lossy()),
            Err(AddError::NotFound)
        );

        let install = dir.path().join("blender-5.2.3");
        std::fs::create_dir_all(install.join("5.2").join("scripts")).unwrap();
        let exe = install.join("blender.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        std::fs::write(install.join("blender-launcher.exe"), b"MZ").unwrap();
        assert_eq!(manual_executable(&exe.to_string_lossy()).unwrap(), exe);
        assert_eq!(manual_executable(&install.to_string_lossy()).unwrap(), exe);
        assert_eq!(
            manual_executable(&install.join("blender-launcher.exe").to_string_lossy()).unwrap(),
            exe
        );
        // A renamed executable in a real installation is still accepted.
        let renamed = install.join("blender-custom.exe");
        std::fs::write(&renamed, b"MZ").unwrap();
        assert_eq!(manual_executable(&renamed.to_string_lossy()).unwrap(), renamed);
    }

    #[test]
    fn support_and_display_version() {
        assert_eq!(support_status(Some(&probe(&[4, 1, 0], ""))), SupportStatus::Unsupported);
        assert_eq!(support_status(Some(&probe(&[4, 2, 1], ""))), SupportStatus::Supported);
        assert_eq!(support_status(Some(&probe(&[5, 2, 3], ""))), SupportStatus::Supported);
        assert_eq!(support_status(Some(&probe(&[5, 3, 0], ""))), SupportStatus::Unverified);
        assert_eq!(support_status(Some(&probe(&[9, 0, 0], ""))), SupportStatus::Unverified);
        assert_eq!(support_status(None), SupportStatus::Unknown);
        assert_eq!(probe(&[4, 2, 1], "4.2.1-farlocus-0.1 LTS").display_version(), "4.2.1 farlocus-0.1");
        assert_eq!(probe(&[4, 5, 3], "4.5.3 LTS").display_version(), "4.5.3");
    }

    #[test]
    fn choose_prefers_version_then_default_then_newest() {
        let list = vec![
            installation("a", &[4, 2, 1], false),
            installation("b", &[4, 2, 9], false),
            installation("c", &[5, 0, 0], true),
            installation("old", &[3, 6, 0], false),
        ];
        assert_eq!(choose(&list, Some("a"), None).unwrap().id, "a");
        assert_eq!(choose(&list, None, Some("4.2")).unwrap().id, "b");
        assert_eq!(choose(&list, None, Some("4.4")).unwrap().id, "c");
        assert_eq!(choose(&list, None, None).unwrap().id, "c");
        assert_eq!(choose(&list, None, Some("3.6")).unwrap().id, "a");
    }
}
