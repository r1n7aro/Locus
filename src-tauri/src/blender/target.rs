//! Which Blender a call runs in: workspace ownership of instances, the
//! per-session target (memory only; cleared when Locus restarts), explicit
//! per-call targets, background instances and interactive launches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;

use super::background::LaunchRequest;
use super::hub::{ConnectionState, Hub, Instance, InstanceSnapshot};
use super::installations::{self, Installation};

/// Header scan cap for the inferred workspace version.
const VERSION_SCAN_LIMIT: usize = 400;
pub const INTERACTIVE_LAUNCH_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a launch waits for the requested file to finish loading.
const FILE_LOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The checkout (and session) a call is made for.
#[derive(Debug, Clone)]
pub struct Scope {
    pub checkout_id: String,
    pub root: PathBuf,
    pub extra_dirs: Vec<PathBuf>,
    pub session_id: Option<String>,
}

impl Scope {
    pub fn new(checkout_id: &str, root: &Path, session_id: Option<&str>) -> Self {
        let extra_dirs = crate::extra_workdirs::load_entries(&root.to_string_lossy())
            .into_iter()
            .map(|entry| PathBuf::from(entry.path))
            .collect();
        Self {
            checkout_id: checkout_id.to_string(),
            root: root.to_path_buf(),
            extra_dirs,
            session_id: session_id.map(str::to_string).filter(|id| !id.trim().is_empty()),
        }
    }

    fn contains(&self, file: &str) -> bool {
        let file = installations::normalize_path(Path::new(file));
        std::iter::once(&self.root).chain(self.extra_dirs.iter()).any(|dir| {
            let dir = installations::normalize_path(dir);
            let dir = dir.trim_end_matches(['/', '\\']);
            file.len() > dir.len()
                && file.starts_with(dir)
                && matches!(file.as_bytes()[dir.len()], b'/' | b'\\')
        })
    }
}

// ── session targets ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSpec {
    Instance(String),
    Pid(u32),
    File(String),
    Background { version: Option<String> },
}

impl TargetSpec {
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("target is empty".to_string());
        }
        if text.eq_ignore_ascii_case("background") {
            return Ok(Self::Background { version: None });
        }
        if let Some(version) = text.strip_prefix("background:") {
            let version = version.trim();
            return Ok(Self::Background {
                version: (!version.is_empty()).then(|| version.to_string()),
            });
        }
        if let Some(pid) = text.strip_prefix("pid:").or(Some(text)).and_then(|v| v.parse::<u32>().ok()) {
            return Ok(Self::Pid(pid));
        }
        if super::detect::is_blend_file_name(text) || text.contains(['/', '\\']) {
            return Ok(Self::File(text.to_string()));
        }
        Ok(Self::Instance(text.to_string()))
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Instance(key) => key.clone(),
            Self::Pid(pid) => format!("pid {pid}"),
            Self::File(file) => file.clone(),
            Self::Background { version: None } => "background".to_string(),
            Self::Background { version: Some(version) } => format!("background:{version}"),
        }
    }
}

#[derive(Debug, Clone)]
struct SessionTarget {
    checkout_id: String,
    spec: TargetSpec,
}

fn targets() -> &'static Mutex<HashMap<String, SessionTarget>> {
    static TARGETS: OnceLock<Mutex<HashMap<String, SessionTarget>>> = OnceLock::new();
    TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A concrete instance is stored by key so a later PID reuse or a reopened
/// file never redirects the session silently.
pub fn set_session_target(scope: &Scope, spec: TargetSpec) -> Result<(), String> {
    let session = scope
        .session_id
        .clone()
        .ok_or_else(|| "a session is required to set a Blender target".to_string())?;
    if let Ok(mut targets) = targets().lock() {
        targets.insert(session, SessionTarget { checkout_id: scope.checkout_id.clone(), spec });
    }
    notify_targets_changed();
    Ok(())
}

/// Target picked in a new chat before it had a session id ("Set as target"
/// in the draft). The first `chat` call applies it right after creating the
/// session, before the run starts, so the first `blender_execute` already
/// uses it. Ignored while the integration is off.
pub fn set_initial_session_target(checkout_id: &str, root: &Path, session_id: &str, spec: &str) -> Result<(), String> {
    if !super::blender_integration_enabled() {
        return Ok(());
    }
    let spec = TargetSpec::parse(spec)?;
    set_session_target(&Scope::new(checkout_id, root, Some(session_id)), spec)
}

pub fn clear_session_target(scope: &Scope) {
    if let Some(session) = scope.session_id.as_deref() {
        if let Ok(mut targets) = targets().lock() {
            targets.remove(session);
        }
        notify_targets_changed();
    }
}

pub fn session_target(scope: &Scope) -> Option<TargetSpec> {
    let session = scope.session_id.as_deref()?;
    targets().lock().ok()?.get(session).map(|target| target.spec.clone())
}

fn notify_targets_changed() {
    if let Some(app) = super::hub::app_handle() {
        use tauri::Emitter;
        let _ = app.emit("blender-instances-changed", ());
    }
}

/// Does a session other than `scope`'s target `instance`? Parallel sessions
/// should each work in their own instance.
pub fn targeted_by_other_session(scope: &Scope, instance: &Instance) -> bool {
    let current = scope.session_id.as_deref();
    let specs = targets()
        .lock()
        .map(|targets| {
            targets
                .iter()
                .filter(|(session, _)| Some(session.as_str()) != current)
                .map(|(_, target)| target.spec.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    specs.iter().any(|spec| spec_matches(spec, instance))
}

/// Instances some session of this checkout targets explicitly.
fn targeted_keys(checkout_id: &str) -> Vec<String> {
    targets()
        .lock()
        .map(|targets| {
            targets
                .values()
                .filter(|target| target.checkout_id == checkout_id)
                .filter_map(|target| match &target.spec {
                    TargetSpec::Instance(key) => Some(key.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── ownership ──────────────────────────────────────────────────────────────

/// An instance belongs to the workspace when its open file is inside the
/// checkout (or an attached directory), Locus launched it for the checkout,
/// or a session of the checkout targets it.
pub fn owned_by(scope: &Scope, instance: &Instance) -> bool {
    let registration = instance.registration();
    if registration.launched_by_checkout().as_deref() == Some(scope.checkout_id.as_str()) {
        return true;
    }
    if registration.file.as_deref().is_some_and(|file| scope.contains(file)) {
        return true;
    }
    targeted_keys(&scope.checkout_id).iter().any(|key| key == &instance.key)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedInstance {
    #[serde(flatten)]
    pub instance: InstanceSnapshot,
    pub in_workspace: bool,
    pub is_target: bool,
    /// Another session targets this instance.
    pub other_session_target: bool,
    pub owned_background: bool,
    /// Other instances have the same file open (saves overwrite each other).
    pub shared_file: bool,
}

pub fn scoped_instances(hub: &Hub, scope: &Scope) -> Vec<ScopedInstance> {
    let target = session_target(scope);
    let instances = hub.instances();
    let snapshots = instances.iter().map(|instance| instance.snapshot()).collect::<Vec<_>>();
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    for snapshot in &snapshots {
        if let Some(file) = snapshot.file.as_deref() {
            *file_counts.entry(installations::normalize_path(Path::new(file))).or_default() += 1;
        }
    }
    instances
        .iter()
        .zip(snapshots)
        .map(|(instance, snapshot)| {
            let shared_file = snapshot
                .file
                .as_deref()
                .is_some_and(|file| file_counts.get(&installations::normalize_path(Path::new(file))).copied().unwrap_or(0) > 1);
            ScopedInstance {
                in_workspace: owned_by(scope, instance),
                is_target: target.as_ref().is_some_and(|spec| spec_matches(spec, instance)),
                other_session_target: targeted_by_other_session(scope, instance),
                owned_background: hub.background.owns(snapshot.pid),
                shared_file,
                instance: snapshot,
            }
        })
        .collect()
}

fn spec_matches(spec: &TargetSpec, instance: &Instance) -> bool {
    let registration = instance.registration();
    match spec {
        TargetSpec::Instance(key) => &instance.key == key,
        TargetSpec::Pid(pid) => registration.pid == *pid,
        TargetSpec::File(file) => registration.file.as_deref().is_some_and(|open| {
            installations::normalize_path(Path::new(open)) == installations::normalize_path(Path::new(file))
        }),
        TargetSpec::Background { .. } => false,
    }
}

// ── resolution ─────────────────────────────────────────────────────────────

/// No usable target; carries the instances the caller can choose from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetUnavailable {
    pub code: &'static str,
    pub message: String,
    pub candidates: Vec<InstanceSnapshot>,
    /// Keys of the candidates another session already works in.
    pub other_session_targets: Vec<String>,
}

impl TargetUnavailable {
    /// Message plus one line per candidate (tool output).
    pub fn render(&self) -> String {
        let mut text = self.message.clone();
        if self.candidates.is_empty() {
            text.push_str("\nNo Blender instance with the Locus host is running. Start Blender (with the Locus add-on installed) or use target=\"background\".");
        } else {
            text.push_str("\nAvailable instances (pass one as target):");
            for candidate in &self.candidates {
                text.push_str(&format!("\n- {}", describe_snapshot(candidate)));
                if self.other_session_targets.contains(&candidate.key) {
                    text.push_str(" · target of another session (avoid unless asked)");
                }
            }
            text.push_str("\n- background (Locus-managed background instance)");
        }
        text
    }
}

pub fn describe_snapshot(snapshot: &InstanceSnapshot) -> String {
    let file = snapshot
        .file
        .as_deref()
        .map(|file| Path::new(file).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| file.to_string()))
        .unwrap_or_else(|| "(unsaved)".to_string());
    format!(
        "{} · Blender {} · PID {} · {} · {}{}",
        snapshot.key,
        snapshot.version_string,
        snapshot.pid,
        file,
        snapshot.mode,
        match snapshot.connection {
            ConnectionState::Connected => "",
            ConnectionState::Connecting => " (connecting)",
            ConnectionState::Disconnected => " (disconnected)",
            ConnectionState::HostError => " (host error)",
        }
    )
}

pub enum Resolved {
    Instance(Arc<Instance>),
    Background { version: Option<String> },
}

/// Explicit target, else the session target, else the single interactive
/// instance of this workspace. Never falls back to a background instance.
pub fn resolve(hub: &Hub, scope: &Scope, explicit: Option<&str>) -> Result<Resolved, TargetUnavailable> {
    let unavailable = |message: String| {
        let mut other_session_targets = Vec::new();
        let candidates = hub
            .instances()
            .iter()
            .filter(|instance| instance.connection() != ConnectionState::HostError || owned_by(scope, instance))
            .map(|instance| {
                if targeted_by_other_session(scope, instance) {
                    other_session_targets.push(instance.key.clone());
                }
                instance.snapshot()
            })
            .collect::<Vec<_>>();
        TargetUnavailable { code: "target_unavailable", message, candidates, other_session_targets }
    };
    let (spec, source) = match explicit.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => (Some(TargetSpec::parse(text).map_err(unavailable)?), "target"),
        None => (session_target(scope), "session target"),
    };
    if let Some(spec) = spec {
        if let TargetSpec::Background { version } = spec {
            return Ok(Resolved::Background { version });
        }
        let found = hub.instances().into_iter().filter(|instance| spec_matches(&spec, instance)).collect::<Vec<_>>();
        return match found.as_slice() {
            [instance] => Ok(Resolved::Instance(Arc::clone(instance))),
            [] => Err(unavailable(format!("The {source} \"{}\" is not a running Blender instance.", spec.describe()))),
            _ => Err(unavailable(format!("The {source} \"{}\" matches several instances; pass an instance id.", spec.describe()))),
        };
    }
    let interactive = hub
        .instances()
        .into_iter()
        .filter(|instance| !instance.registration().is_background() && owned_by(scope, instance))
        .collect::<Vec<_>>();
    // Never wander into the instance another session works in; sharing one
    // takes an explicit target.
    let free = interactive
        .iter()
        .filter(|instance| !targeted_by_other_session(scope, instance))
        .cloned()
        .collect::<Vec<_>>();
    match free.as_slice() {
        [instance] => Ok(Resolved::Instance(Arc::clone(instance))),
        [] if !interactive.is_empty() => Err(unavailable(
            "Every Blender instance of this workspace is another session's target. Pass target= to share one, or launch another Blender (locus.blender.launch).".to_string(),
        )),
        [] => Err(unavailable(
            "No interactive Blender instance belongs to this workspace (none has a file from this workspace open).".to_string(),
        )),
        _ => Err(unavailable(
            "Several Blender instances belong to this workspace; pick one with target= or set a session target.".to_string(),
        )),
    }
}

// ── workspace version ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceVersion {
    /// Pinned in the workspace's Blender service config.
    pub pinned: Option<String>,
    /// Most common saved version among the workspace's `.blend` files.
    pub inferred: Option<String>,
    pub effective: Option<String>,
    pub distribution: super::blend_header::VersionDistribution,
}

fn session_store() -> Option<Arc<crate::session::store::SessionStore>> {
    use tauri::Manager;
    super::hub::app_handle()?
        .try_state::<Arc<crate::session::store::SessionStore>>()
        .map(|state| Arc::clone(state.inner()))
}

pub fn pinned_version(checkout_id: &str) -> Option<String> {
    session_store()?
        .list_workspace_services(checkout_id)
        .ok()?
        .into_iter()
        .find(|record| record.service_kind == "blender")?
        .local_config
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|version| !version.trim().is_empty())
}

pub fn set_pinned_version(checkout_id: &str, version: Option<&str>) -> Result<(), String> {
    let store = session_store().ok_or_else(|| "session store unavailable".to_string())?;
    let mut record = store
        .list_workspace_services(checkout_id)?
        .into_iter()
        .find(|record| record.service_kind == "blender")
        .ok_or_else(|| "this workspace has no Blender service record yet".to_string())?;
    let mut config = record.local_config.as_object().cloned().unwrap_or_default();
    match version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(version) => {
            config.insert("version".to_string(), serde_json::Value::from(version));
        }
        None => {
            config.remove("version");
        }
    }
    record.local_config = serde_json::Value::Object(config);
    store.upsert_workspace_service(&record)
}

pub fn workspace_version(scope: &Scope) -> WorkspaceVersion {
    let distribution = super::blend_header::workspace_distribution(&scope.root, VERSION_SCAN_LIMIT);
    let pinned = pinned_version(&scope.checkout_id);
    let inferred = distribution.most_common().map(str::to_string);
    WorkspaceVersion {
        effective: pinned.clone().or_else(|| inferred.clone()),
        pinned,
        inferred,
        distribution,
    }
}

/// Installation for a launch: explicit installation, then the file's saved
/// version, the requested version, the workspace version, the default, the
/// newest supported one.
pub async fn choose_installation(
    scope: &Scope,
    installation: Option<&str>,
    version: Option<&str>,
    file: Option<&Path>,
) -> Result<Installation, String> {
    let list = installations::list_probed().await;
    if list.is_empty() {
        return Err("No Blender installation found. Add one in Settings > Blender.".to_string());
    }
    let file_version = file
        .and_then(|file| super::blend_header::header_cached(file).ok())
        .map(|header| header.version);
    let wanted = version
        .map(str::to_string)
        .or(file_version)
        .or_else(|| workspace_version(scope).effective);
    installations::choose(&list, installation, wanted.as_deref())
        .cloned()
        .ok_or_else(|| "No supported Blender installation (4.2 or newer) is available.".to_string())
}

/// Get (or launch) this checkout's background instance.
pub async fn background_instance(
    hub: &Arc<Hub>,
    scope: &Scope,
    version: Option<&str>,
    file: Option<&Path>,
) -> Result<Arc<Instance>, String> {
    background_instance_with(hub, scope, None, version, file, false).await
}

pub async fn background_instance_with(
    hub: &Arc<Hub>,
    scope: &Scope,
    installation: Option<&str>,
    version: Option<&str>,
    file: Option<&Path>,
    force_new: bool,
) -> Result<Arc<Instance>, String> {
    let installation = choose_installation(scope, installation, version, file).await?;
    hub.background
        .acquire(
            hub,
            LaunchRequest {
                checkout_id: &scope.checkout_id,
                checkout_root: &scope.root,
                session_id: scope.session_id.as_deref(),
                installation: &installation,
                file,
                force_new,
            },
        )
        .await
}

/// Start an interactive Blender (owned by the user, not by Locus) with the
/// host loaded through `--python bootstrap.py`, so it connects even when the
/// add-on is not installed. Waits until it connects.
pub async fn launch_interactive(
    hub: &Arc<Hub>,
    scope: &Scope,
    file: Option<&Path>,
    installation: Option<&str>,
    version: Option<&str>,
    allow_downgrade: bool,
) -> Result<Arc<Instance>, String> {
    let installation = choose_installation(scope, installation, version, file).await?;
    if let (Some(file), Some(probe)) = (file, installation.probe.as_ref()) {
        if let Ok(header) = super::blend_header::header_cached(file) {
            match super::blend_header::open_compatibility(&header.version, &probe.version_string) {
                super::blend_header::OpenCompatibility::Ok => {}
                super::blend_header::OpenCompatibility::Refused => {
                    return Err(format!(
                        "{} was saved by Blender {}; Blender {} cannot open 5.x files.",
                        file.display(),
                        header.version,
                        probe.version_string
                    ))
                }
                super::blend_header::OpenCompatibility::NeedsDowngrade if !allow_downgrade => {
                    return Err(format!(
                        "{} was saved by Blender {} (newer than {}); pass allow_downgrade=True to open it anyway.",
                        file.display(),
                        header.version,
                        probe.version_string
                    ))
                }
                super::blend_header::OpenCompatibility::NeedsDowngrade => {}
            }
        }
    }
    let bootstrap = super::resources::bootstrap_script()
        .ok_or_else(|| "Blender host sources are missing (bootstrap.py)".to_string())?;
    let mut command = std::process::Command::new(&installation.path);
    if let Some(file) = file {
        command.arg(file);
    }
    command.arg("--python").arg(&bootstrap);
    command.current_dir(&scope.root);
    command.env_remove("PYTHONHOME").env_remove("PYTHONPATH");
    command.env(
        "LOCUS_BLENDER_LAUNCHED_BY",
        serde_json::json!({
            "checkout": scope.checkout_id,
            "session": scope.session_id,
            "locusPid": std::process::id(),
            "interactive": true,
        })
        .to_string(),
    );
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let child = spawn_user_owned(&mut command)
        .map_err(|error| format!("failed to start Blender {}: {error}", installation.path))?;
    let pid = child.id();
    drop(child);
    let instance = hub.wait_for_pid(pid, INTERACTIVE_LAUNCH_TIMEOUT).await?;
    if let Some(file) = file {
        // Large files take a while; past the limit the caller still gets the
        // instance and sees which file it reports.
        hub.wait_for_file(&instance, file, FILE_LOAD_TIMEOUT).await;
    }
    Ok(instance)
}

/// Interactive Blender belongs to the user and must outlive Locus. When
/// Locus itself runs inside a kill-on-close job (some terminals and task
/// runners), leave that job; without permission to, start it normally.
fn spawn_user_owned(command: &mut std::process::Command) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const ERROR_ACCESS_DENIED: i32 = 5;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        match command.spawn() {
            Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
                command.creation_flags(CREATE_NO_WINDOW);
                command.spawn()
            }
            result => result,
        }
    }
    #[cfg(not(windows))]
    {
        command.spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_specs_parse_by_shape() {
        assert_eq!(TargetSpec::parse("background").unwrap(), TargetSpec::Background { version: None });
        assert_eq!(
            TargetSpec::parse("background:4.2").unwrap(),
            TargetSpec::Background { version: Some("4.2".into()) }
        );
        assert_eq!(TargetSpec::parse("1234").unwrap(), TargetSpec::Pid(1234));
        assert_eq!(TargetSpec::parse("pid:77").unwrap(), TargetSpec::Pid(77));
        assert_eq!(TargetSpec::parse("art/hero.blend").unwrap(), TargetSpec::File("art/hero.blend".into()));
        assert_eq!(TargetSpec::parse("1234-998877").unwrap(), TargetSpec::Instance("1234-998877".into()));
        assert!(TargetSpec::parse("  ").is_err());
    }

    #[test]
    fn instances_other_sessions_work_in_are_flagged() {
        let registration = super::super::hub::Registration { pid: 4242, pipe: Some("p".into()), ..Default::default() };
        let instance = Instance::for_test("4242_other", registration);
        let first = Scope {
            checkout_id: "c-other".into(),
            root: PathBuf::from("F:/Work/Other"),
            extra_dirs: Vec::new(),
            session_id: Some("session-other-1".into()),
        };
        let second = Scope { session_id: Some("session-other-2".into()), ..first.clone() };
        set_session_target(&first, TargetSpec::Instance("4242_other".into())).unwrap();
        assert!(!targeted_by_other_session(&first, &instance));
        assert!(targeted_by_other_session(&second, &instance));

        let unavailable = TargetUnavailable {
            code: "target_unavailable",
            message: "pick one".into(),
            candidates: vec![instance.snapshot()],
            other_session_targets: vec!["4242_other".into()],
        };
        assert!(unavailable.render().contains("4242_other · Blender") && unavailable.render().contains("target of another session"));
        clear_session_target(&first);
        assert!(!targeted_by_other_session(&second, &instance));
    }

    #[test]
    fn scope_contains_only_paths_below_its_roots() {
        let scope = Scope {
            checkout_id: "c".into(),
            root: PathBuf::from("F:/Work/Game"),
            extra_dirs: vec![PathBuf::from("D:/Art")],
            session_id: None,
        };
        assert!(scope.contains("F:/Work/Game/art/hero.blend"));
        assert!(scope.contains("f:\\work\\game\\hero.blend"));
        assert!(scope.contains("D:/Art/a.blend"));
        assert!(!scope.contains("F:/Work/GameOther/hero.blend"));
        assert!(!scope.contains("F:/Work/Game"));
    }
}
