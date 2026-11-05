//! Background (`-b`) Blender instances that Locus launches for a checkout.
//!
//! - Launched as `blender -b --factory-startup [file] --python bootstrap.py`
//!   with the checkout root as working directory; the host starts from
//!   `bootstrap.py`, no add-on install needed.
//! - Owned through `process_util::spawn_managed` (job object; killed when
//!   Locus exits or the entry is dropped).
//! - Global cap (`blender_max_background_instances`) and idle reclaim
//!   (`blender_background_idle_timeout_secs`); an instance with a running
//!   execution is never reclaimed.
//! - Interactive instances are never launched or closed here.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::hub::{Hub, Instance};
use super::installations::Installation;
use crate::process_util::{ManagedChild, ProcessOwner};

/// How long a fresh background instance may take to register and connect.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Grace period after `shutdown_host` before the process is terminated.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const LOG_KEEP: usize = 10;

pub struct BackgroundEntry {
    pub pid: u32,
    pub checkout: String,
    pub installation: String,
    pub version: String,
    pub file: Option<String>,
    pub log: Option<PathBuf>,
    pub started: Instant,
    child: Mutex<Option<ManagedChild>>,
}

impl BackgroundEntry {
    fn exited(&self) -> bool {
        let Ok(mut child) = self.child.lock() else {
            return true;
        };
        match child.as_mut() {
            Some(child) => !matches!(child.try_wait(), Ok(None)),
            None => true,
        }
    }

    fn kill(&self) {
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.terminate_tree();
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundSnapshot {
    pub pid: u32,
    pub checkout: String,
    pub installation: String,
    pub version: String,
    pub file: Option<String>,
    pub log: Option<String>,
    pub uptime_secs: u64,
}

#[derive(Default)]
pub struct BackgroundPool {
    entries: Mutex<Vec<Arc<BackgroundEntry>>>,
    launch: tokio::sync::Mutex<()>,
    wake: Arc<tokio::sync::Notify>,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_instances: usize,
    pub idle_timeout: Duration,
}

pub fn limits() -> Limits {
    let configured = super::hub::app_handle()
        .and_then(|app| {
            use tauri::Manager;
            app.try_state::<Arc<crate::resource_policy::ResourcePolicyStore>>()
                .map(|store| store.configured_limits())
        })
        .unwrap_or_default();
    Limits {
        max_instances: configured.blender_max_background_instances.max(1) as usize,
        idle_timeout: Duration::from_secs(configured.blender_background_idle_timeout_secs.max(1)),
    }
}

/// What a caller wants launched or reused.
pub struct LaunchRequest<'a> {
    pub checkout_id: &'a str,
    pub checkout_root: &'a Path,
    pub session_id: Option<&'a str>,
    pub installation: &'a Installation,
    pub file: Option<&'a Path>,
    /// Launch another instance even when a matching one exists.
    pub force_new: bool,
}

impl BackgroundPool {
    pub fn snapshots(&self) -> Vec<BackgroundSnapshot> {
        self.prune();
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| BackgroundSnapshot {
                        pid: entry.pid,
                        checkout: entry.checkout.clone(),
                        installation: entry.installation.clone(),
                        version: entry.version.clone(),
                        file: entry.file.clone(),
                        log: entry.log.as_ref().map(|path| path.display().to_string()),
                        uptime_secs: entry.started.elapsed().as_secs(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn owns(&self, pid: u32) -> bool {
        self.entries.lock().map(|entries| entries.iter().any(|entry| entry.pid == pid)).unwrap_or(false)
    }

    fn prune(&self) {
        let removed = self
            .entries
            .lock()
            .map(|mut entries| {
                let (gone, kept): (Vec<_>, Vec<_>) = entries.drain(..).partition(|entry| entry.exited());
                *entries = kept;
                gone
            })
            .unwrap_or_default();
        for entry in removed {
            entry.kill();
        }
    }

    fn list(&self) -> Vec<Arc<BackgroundEntry>> {
        self.entries.lock().map(|entries| entries.clone()).unwrap_or_default()
    }

    fn remove(&self, pid: u32) -> Option<Arc<BackgroundEntry>> {
        let mut entries = self.entries.lock().ok()?;
        let index = entries.iter().position(|entry| entry.pid == pid)?;
        Some(entries.remove(index))
    }

    /// Reuse this checkout's background instance of the installation (and
    /// file, when given), or launch one within the global cap.
    pub async fn acquire(&self, hub: &Arc<Hub>, request: LaunchRequest<'_>) -> Result<Arc<Instance>, String> {
        let _launch = self.launch.lock().await;
        self.prune();
        let file = request.file.map(|path| path.to_string_lossy().to_string());
        for entry in self.list().into_iter().filter(|_| !request.force_new) {
            if entry.checkout != request.checkout_id || entry.installation != request.installation.id {
                continue;
            }
            let Some(instance) = hub.instance_by_pid(entry.pid) else {
                continue;
            };
            let open = instance.registration().file;
            if file.is_some() && !same_file(open.as_deref(), file.as_deref()) {
                continue;
            }
            if instance.connection() == super::hub::ConnectionState::Connected {
                instance.touch();
                return Ok(instance);
            }
        }

        let limits = limits();
        while self.list().len() >= limits.max_instances {
            let victim = self
                .list()
                .into_iter()
                .filter_map(|entry| {
                    let instance = hub.instance_by_pid(entry.pid);
                    let busy = instance.as_ref().is_some_and(|i| i.busy.load(Ordering::SeqCst) > 0);
                    (!busy).then(|| (entry, instance.map(|i| i.idle_for()).unwrap_or(Duration::MAX)))
                })
                .max_by_key(|(_, idle)| *idle)
                .map(|(entry, _)| entry);
            let Some(victim) = victim else {
                return Err(format!(
                    "all {} background Blender instances are busy (Settings > Workspace resources raises the cap)",
                    limits.max_instances
                ));
            };
            self.close(hub, victim.pid).await;
        }

        let entry = launch(&request)?;
        let pid = entry.pid;
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(Arc::clone(&entry));
        }
        self.wake.notify_one();
        match hub.wait_for_pid(pid, LAUNCH_TIMEOUT).await {
            Ok(instance) => {
                instance.touch();
                Ok(instance)
            }
            Err(error) => {
                self.remove(pid);
                entry.kill();
                let tail = entry.log.as_deref().map(log_tail).unwrap_or_default();
                Err(if tail.is_empty() { error } else { format!("{error}\n{tail}") })
            }
        }
    }

    /// Ask the host to exit, then terminate after a grace period.
    pub async fn close(&self, hub: &Arc<Hub>, pid: u32) {
        let Some(entry) = self.remove(pid) else {
            return;
        };
        if let Some(instance) = hub.instance_by_pid(pid) {
            let _ = instance.request("shutdown_host", "", Duration::from_secs(2)).await;
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < deadline && !entry.exited() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        entry.kill();
        hub.scan(&super::resources::registry_dir());
    }

    pub async fn close_for_checkout(&self, hub: &Arc<Hub>, checkout_id: &str) {
        let pids = self
            .list()
            .into_iter()
            .filter(|entry| entry.checkout == checkout_id)
            .map(|entry| entry.pid)
            .collect::<Vec<_>>();
        for pid in pids {
            self.close(hub, pid).await;
        }
    }

    pub fn shutdown_all_blocking(&self) {
        let entries = self.entries.lock().map(|mut entries| std::mem::take(&mut *entries)).unwrap_or_default();
        for entry in entries {
            entry.kill();
        }
        self.wake.notify_one();
    }
}

/// Idle reclaim. Sleeps until the earliest possible expiry (or a new
/// launch), holding only a weak hub reference while asleep.
pub(crate) async fn reaper(hub: Weak<Hub>) {
    let wake = match hub.upgrade() {
        Some(hub) => Arc::clone(&hub.background.wake),
        None => return,
    };
    loop {
        let next = {
            let Some(hub) = hub.upgrade() else {
                return;
            };
            hub.background.prune();
            let timeout = limits().idle_timeout;
            let mut next: Option<Duration> = None;
            let mut expired = Vec::new();
            for entry in hub.background.list() {
                let Some(instance) = hub.instance_by_pid(entry.pid) else {
                    // Not registered yet (starting) or gone: re-check soon.
                    next = Some(next.map_or(Duration::from_secs(30), |n| n.min(Duration::from_secs(30))));
                    continue;
                };
                if instance.busy.load(Ordering::SeqCst) > 0 {
                    next = Some(next.map_or(timeout, |n| n.min(timeout)));
                    continue;
                }
                let idle = instance.idle_for();
                if idle >= timeout {
                    expired.push(entry.pid);
                } else {
                    let remaining = timeout - idle;
                    next = Some(next.map_or(remaining, |n| n.min(remaining)));
                }
            }
            for pid in expired {
                eprintln!("[Locus] closing idle background Blender (pid {pid})");
                hub.background.close(&hub, pid).await;
            }
            next
        };
        match next {
            Some(delay) => {
                tokio::select! {
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(delay + Duration::from_millis(50)) => {}
                }
            }
            None => wake.notified().await,
        }
    }
}

fn same_file(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            super::installations::normalize_path(Path::new(a)) == super::installations::normalize_path(Path::new(b))
        }
        (None, None) => true,
        _ => false,
    }
}

fn launch(request: &LaunchRequest<'_>) -> Result<Arc<BackgroundEntry>, String> {
    let bootstrap = super::resources::bootstrap_script()
        .ok_or_else(|| "Blender host sources are missing (bootstrap.py)".to_string())?;
    let exe = PathBuf::from(&request.installation.path);
    if !exe.is_file() {
        return Err(format!("Blender executable not found: {}", exe.display()));
    }
    let mut command = tokio::process::Command::new(&exe);
    command.arg("-b").arg("--factory-startup");
    if let Some(file) = request.file {
        command.arg(file);
    }
    command.arg("--python").arg(&bootstrap);
    command.current_dir(request.checkout_root);
    command.env_remove("PYTHONHOME").env_remove("PYTHONPATH");
    command.env(
        "LOCUS_BLENDER_LAUNCHED_BY",
        serde_json::json!({
            "checkout": request.checkout_id,
            "session": request.session_id,
            "locusPid": std::process::id(),
        })
        .to_string(),
    );
    command.stdin(std::process::Stdio::null());
    let log = open_log();
    match log
        .as_ref()
        .and_then(|(_, file)| Some((file.try_clone().ok()?, file.try_clone().ok()?)))
    {
        Some((out, err)) => {
            command.stdout(std::process::Stdio::from(out));
            command.stderr(std::process::Stdio::from(err));
        }
        None => {
            command.stdout(std::process::Stdio::null());
            command.stderr(std::process::Stdio::null());
        }
    }
    let owner = ProcessOwner {
        session_id: None,
        task_id: Some(format!("blender-background:{}", request.checkout_id)),
        working_dir: Some(request.checkout_root.display().to_string()),
    };
    let child = crate::process_util::spawn_managed(command, owner)
        .map_err(|error| format!("failed to start Blender {}: {error}", exe.display()))?;
    let pid = child.id().ok_or_else(|| "Blender exited immediately".to_string())?;
    eprintln!(
        "[Locus] launched background Blender {} (pid {pid}) for checkout {}",
        request.installation.display_version.as_deref().unwrap_or("?"),
        request.checkout_id
    );
    Ok(Arc::new(BackgroundEntry {
        pid,
        checkout: request.checkout_id.to_string(),
        installation: request.installation.id.clone(),
        version: request
            .installation
            .probe
            .as_ref()
            .map(|probe| probe.version_string.clone())
            .unwrap_or_default(),
        file: request.file.map(|path| path.to_string_lossy().to_string()),
        log: log.map(|(path, _)| path),
        started: Instant::now(),
        child: Mutex::new(Some(child)),
    }))
}

fn open_log() -> Option<(PathBuf, std::fs::File)> {
    let dir = super::resources::locus_blender_data_dir().join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    prune_logs(&dir);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let path = dir.join(format!("background-{stamp}.log"));
    let file = std::fs::File::create(&path).ok()?;
    Some((path, file))
}

fn prune_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs = entries
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("background-"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect::<Vec<_>>();
    if logs.len() < LOG_KEEP {
        return;
    }
    logs.sort();
    for (_, path) in logs.iter().take(logs.len() + 1 - LOG_KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

/// Last lines of a background log, for launch failures.
fn log_tail(path: &Path) -> String {
    let Ok(text) = std::fs::read_to_string(path) else {
        return String::new();
    };
    let lines = text.lines().filter(|line| !line.trim().is_empty()).collect::<Vec<_>>();
    let start = lines.len().saturating_sub(8);
    if lines.is_empty() {
        return String::new();
    }
    format!("Blender output ({}):\n{}", path.display(), lines[start..].join("\n"))
}
