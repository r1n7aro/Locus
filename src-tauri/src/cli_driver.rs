use std::{
    future::Future,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Listener};

use crate::{
    unity_bridge::{
        self, PluginStatus, UnityConnectionStatus, UnityEditorProcessState,
        UNITY_EDITOR_STATUS_EDITING,
    },
    workspace::Workspace,
};

const DRIVER_NAME: &str = "unity-test";
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_SUITE_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_POLL_MS: u64 = 500;
const DEFAULT_NO_PROGRESS_TIMEOUT_MS: u64 = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliDriverSuite {
    Connect,
    StateProbe,
    NativeBridge,
    HotReload,
}

impl CliDriverSuite {
    fn as_str(self) -> &'static str {
        match self {
            CliDriverSuite::Connect => "connect",
            CliDriverSuite::StateProbe => "state-probe",
            CliDriverSuite::NativeBridge => "native-bridge",
            CliDriverSuite::HotReload => "hot-reload",
        }
    }

    fn event_name(self) -> Option<&'static str> {
        match self {
            CliDriverSuite::Connect => None,
            CliDriverSuite::StateProbe => Some("unity-state-probe-selftest"),
            CliDriverSuite::NativeBridge => Some("unity-native-bridge-selftest"),
            CliDriverSuite::HotReload => Some("unity-hotreload-selftest"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliDriverConfig {
    pub project_path: Option<String>,
    pub suites: Vec<CliDriverSuite>,
    pub open_unity: bool,
    pub install_plugin: bool,
    pub force_edit_mode: bool,
    pub connect_timeout: Duration,
    pub suite_timeout: Duration,
    pub poll_interval: Duration,
    pub no_progress_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverEvent<'a, T: Serialize> {
    event: &'a str,
    payload: T,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTestEvent {
    #[serde(default)]
    running: bool,
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    line: Option<String>,
    #[serde(default)]
    passed: u32,
    #[serde(default)]
    failed: u32,
}

#[derive(Debug, Clone)]
struct SelfTestSummary {
    suite: CliDriverSuite,
    passed: u32,
    failed: u32,
}

impl CliDriverConfig {
    pub fn from_env_args() -> Option<Result<Self, String>> {
        Self::parse(std::env::args().skip(1).collect())
    }

    fn parse(args: Vec<String>) -> Option<Result<Self, String>> {
        let mut driver_requested = false;
        let mut project_path = None;
        let mut suites = Vec::new();
        let mut open_unity = true;
        let mut install_plugin = false;
        let mut force_edit_mode = true;
        let mut connect_timeout = Duration::from_millis(DEFAULT_CONNECT_TIMEOUT_MS);
        let mut suite_timeout = Duration::from_millis(DEFAULT_SUITE_TIMEOUT_MS);
        let mut poll_interval = Duration::from_millis(DEFAULT_POLL_MS);
        let mut no_progress_timeout = Duration::from_millis(DEFAULT_NO_PROGRESS_TIMEOUT_MS);

        let mut index = 0usize;
        while index < args.len() {
            let arg = &args[index];
            match split_arg(arg) {
                Some(("--locus-driver", value)) | Some(("--locus-cli", value)) => {
                    driver_requested = driver_requested || value == DRIVER_NAME;
                    if !value.is_empty() && value != DRIVER_NAME {
                        return Some(Err(format!(
                            "Unsupported Locus CLI driver '{}'; expected '{}'",
                            value, DRIVER_NAME
                        )));
                    }
                    if value.is_empty() {
                        let Some(next) = args.get(index + 1) else {
                            return Some(Err(format!("{arg} requires a value")));
                        };
                        driver_requested = driver_requested || next == DRIVER_NAME;
                        if next != DRIVER_NAME {
                            return Some(Err(format!(
                                "Unsupported Locus CLI driver '{}'; expected '{}'",
                                next, DRIVER_NAME
                            )));
                        }
                        index += 1;
                    }
                }
                Some(("--project", value)) => {
                    let value = match read_option_value("--project", value, &args, &mut index) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    project_path = Some(value);
                }
                Some(("--suite", value)) => {
                    let value = match read_option_value("--suite", value, &args, &mut index) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    for suite in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                        if let Err(error) = push_suite(&mut suites, suite) {
                            return Some(Err(error));
                        }
                    }
                }
                Some(("--timeout-ms", value)) | Some(("--suite-timeout-ms", value)) => {
                    let name = arg_name(arg);
                    let value = match read_option_value(name, value, &args, &mut index) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    suite_timeout = match parse_millis(name, &value) {
                        Ok(value) => Duration::from_millis(value),
                        Err(error) => return Some(Err(error)),
                    };
                }
                Some(("--connect-timeout-ms", value)) => {
                    let value =
                        match read_option_value("--connect-timeout-ms", value, &args, &mut index) {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                    connect_timeout = match parse_millis("--connect-timeout-ms", &value) {
                        Ok(value) => Duration::from_millis(value),
                        Err(error) => return Some(Err(error)),
                    };
                }
                Some(("--poll-ms", value)) => {
                    let value = match read_option_value("--poll-ms", value, &args, &mut index) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    poll_interval = match parse_millis("--poll-ms", &value) {
                        Ok(value) => Duration::from_millis(value),
                        Err(error) => return Some(Err(error)),
                    };
                }
                Some(("--no-progress-timeout-ms", value)) => {
                    let value = match read_option_value(
                        "--no-progress-timeout-ms",
                        value,
                        &args,
                        &mut index,
                    ) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    no_progress_timeout = match parse_millis("--no-progress-timeout-ms", &value) {
                        Ok(value) => Duration::from_millis(value),
                        Err(error) => return Some(Err(error)),
                    };
                }
                _ if arg == "--locus-unity-test" => {
                    driver_requested = true;
                }
                _ if arg == "--open-unity" => open_unity = true,
                _ if arg == "--no-open-unity" => open_unity = false,
                _ if arg == "--install-plugin" => install_plugin = true,
                _ if arg == "--force-edit-mode" => force_edit_mode = true,
                _ if arg == "--no-force-edit-mode" => force_edit_mode = false,
                _ => {}
            }
            index += 1;
        }

        if !driver_requested {
            return None;
        }

        if suites.is_empty() {
            suites.push(CliDriverSuite::Connect);
        }

        Some(Ok(Self {
            project_path,
            suites,
            open_unity,
            install_plugin,
            force_edit_mode,
            connect_timeout,
            suite_timeout,
            poll_interval,
            no_progress_timeout,
        }))
    }
}

fn split_arg(arg: &str) -> Option<(&str, &str)> {
    let (name, value) = arg.split_once('=').unwrap_or((arg, ""));
    match name {
        "--locus-driver"
        | "--locus-cli"
        | "--project"
        | "--suite"
        | "--timeout-ms"
        | "--suite-timeout-ms"
        | "--connect-timeout-ms"
        | "--poll-ms"
        | "--no-progress-timeout-ms" => Some((name, value)),
        _ => None,
    }
}

fn arg_name(arg: &str) -> &str {
    arg.split_once('=').map(|(name, _)| name).unwrap_or(arg)
}

fn read_option_value(
    name: &str,
    inline: &str,
    args: &[String],
    index: &mut usize,
) -> Result<String, String> {
    if !inline.is_empty() {
        return Ok(inline.to_string());
    }
    let Some(next) = args.get(*index + 1) else {
        return Err(format!("{name} requires a value"));
    };
    *index += 1;
    Ok(next.clone())
}

fn parse_millis(name: &str, value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{name} requires an integer millisecond value"))
        .and_then(|millis| {
            if millis == 0 {
                Err(format!("{name} must be greater than 0"))
            } else {
                Ok(millis)
            }
        })
}

fn push_suite(suites: &mut Vec<CliDriverSuite>, value: &str) -> Result<(), String> {
    let expanded = match value {
        "all" => {
            for suite in [
                CliDriverSuite::Connect,
                CliDriverSuite::StateProbe,
                CliDriverSuite::NativeBridge,
                CliDriverSuite::HotReload,
            ] {
                if !suites.contains(&suite) {
                    suites.push(suite);
                }
            }
            return Ok(());
        }
        "connect" => CliDriverSuite::Connect,
        "state-probe" | "state_probe" | "state" => CliDriverSuite::StateProbe,
        "native-bridge" | "native_bridge" | "native" => CliDriverSuite::NativeBridge,
        "hot-reload" | "hot_reload" | "hotreload" | "hot" => CliDriverSuite::HotReload,
        _ => {
            return Err(format!(
            "Unknown --suite '{}'. Use connect, state-probe, native-bridge, hot-reload, or all.",
            value
        ))
        }
    };
    if !suites.contains(&expanded) {
        suites.push(expanded);
    }
    Ok(())
}

pub fn spawn(app_handle: AppHandle, workspace: Arc<Workspace>, config: CliDriverConfig) {
    tauri::async_runtime::spawn(async move {
        let exit_code = match run_driver(app_handle.clone(), workspace, config).await {
            Ok(()) => 0,
            Err(error) => {
                emit_json("error", json!({ "message": error }));
                1
            }
        };
        app_handle.exit(exit_code);
    });
}

async fn run_driver(
    app_handle: AppHandle,
    workspace: Arc<Workspace>,
    config: CliDriverConfig,
) -> Result<(), String> {
    emit_json(
        "start",
        json!({
            "driver": DRIVER_NAME,
            "suites": config.suites.iter().map(|suite| suite.as_str()).collect::<Vec<_>>(),
            "openUnity": config.open_unity,
            "installPlugin": config.install_plugin,
            "connectTimeoutMs": config.connect_timeout.as_millis(),
            "suiteTimeoutMs": config.suite_timeout.as_millis(),
            "noProgressTimeoutMs": config.no_progress_timeout.as_millis(),
        }),
    );

    let project = resolve_project_path(config.project_path.as_deref(), &workspace).await?;
    set_workspace_for_driver(&workspace, &project).await?;
    prepare_suite_environment(&project, &config)?;
    check_or_install_plugin(&project, config.install_plugin).await?;

    let status = ensure_connected(&project, &config).await?;
    let transport = resolve_active_transport(&project).await;
    emit_json(
        "connected",
        json!({
            "project": project,
            "editorStatus": status.editor_status,
            "processId": status.editor_process_id,
            "processPath": status.editor_process_path,
            "channel": status.control_channel_state,
            "transport": transport,
        }),
    );

    for suite in &config.suites {
        match suite {
            CliDriverSuite::Connect => {
                let semantic = unity_bridge::unity_semantic_state(&project).await;
                emit_json(
                    "suite_result",
                    json!({
                        "suite": suite.as_str(),
                        "passed": 1,
                        "failed": 0,
                        "semanticPhase": semantic.phase,
                        "semanticSource": semantic.source,
                    }),
                );
            }
            CliDriverSuite::StateProbe => {
                unity_bridge::set_state_probe_enabled(true);
                let summary = run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    unity_bridge::run_state_probe_selftest(app_handle.clone(), project.clone()),
                )
                .await?;
                ensure_summary_passed(summary)?;
            }
            CliDriverSuite::NativeBridge => {
                unity_bridge::set_native_bridge_enabled(true);
                unity_bridge::sync_native_bridge_marker(&project, true)?;
                let summary = run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    unity_bridge::run_native_bridge_selftest(app_handle.clone(), project.clone()),
                )
                .await?;
                ensure_summary_passed(summary)?;

                // Confirm the channel actually resolved to the native broker;
                // the suite exists to exercise the required native transport.
                let transport = resolve_active_transport(&project).await;
                emit_json(
                    "native_transport_confirmed",
                    json!({ "suite": suite.as_str(), "transport": transport }),
                );
                if transport != "native_broker" {
                    return Err(format!(
                        "native-bridge suite ran over '{transport}', expected 'native_broker'"
                    ));
                }
            }
            CliDriverSuite::HotReload => {
                crate::csharp_compile::set_enabled(true).await;
                crate::csharp_compile::warm_up_in_background();
                crate::unity_hotreload::set_enabled(true);
                if config.force_edit_mode {
                    ensure_edit_mode(&project, config.connect_timeout, config.poll_interval)
                        .await?;
                }
                let summary = run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    crate::unity_hotreload::selftest::run(app_handle.clone(), project.clone()),
                )
                .await?;
                ensure_summary_passed(summary)?;
            }
        }
    }

    emit_json("finished", json!({ "ok": true }));
    Ok(())
}

fn prepare_suite_environment(project: &str, config: &CliDriverConfig) -> Result<(), String> {
    if config
        .suites
        .iter()
        .any(|suite| matches!(suite, CliDriverSuite::NativeBridge))
    {
        unity_bridge::set_native_bridge_enabled(true);
        unity_bridge::sync_native_bridge_marker(project, true)?;
        emit_json(
            "native_bridge",
            json!({ "action": "markerSynced", "enabled": true }),
        );
    }
    Ok(())
}

async fn resolve_project_path(
    requested: Option<&str>,
    workspace: &Arc<Workspace>,
) -> Result<String, String> {
    let raw = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value.to_string(),
        None => workspace.path.read().await.trim().to_string(),
    };
    if raw.is_empty() {
        return Err("Missing --project and no saved Unity workspace is available".to_string());
    }
    let path = canonicalize_lossy(&raw);
    if !unity_bridge::is_unity_project(&path) {
        return Err(format!("Path is not a Unity project: {path}"));
    }
    Ok(path)
}

async fn set_workspace_for_driver(workspace: &Arc<Workspace>, project: &str) -> Result<(), String> {
    let workspace_id = crate::workspace::load_or_create_workspace(project).ok();
    {
        let mut path = workspace.path.write().await;
        *path = project.to_string();
    }
    {
        let mut id = workspace.workspace_id.write().await;
        *id = workspace_id;
    }
    workspace.bump_generation();
    Ok(())
}

fn canonicalize_lossy(path: &str) -> String {
    let path = Path::new(path.trim().trim_matches('"'));
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_string()
}

async fn check_or_install_plugin(project: &str, install: bool) -> Result<(), String> {
    match unity_bridge::check_plugin_status(project)? {
        PluginStatus::UpToDate => {
            emit_json("plugin", json!({ "status": "upToDate" }));
            Ok(())
        }
        status if install => {
            emit_json(
                "plugin",
                json!({ "status": format!("{status:?}"), "action": "install" }),
            );
            let hash = unity_bridge::install_or_update_plugin(project).await?;
            emit_json("plugin", json!({ "status": "installed", "hash": hash }));
            Ok(())
        }
        status => Err(format!(
            "Unity plugin is {:?}; rerun with --install-plugin to update the project copy",
            status
        )),
    }
}

async fn ensure_connected(
    project: &str,
    config: &CliDriverConfig,
) -> Result<UnityConnectionStatus, String> {
    let started = Instant::now();
    let mut launched = false;
    let mut last_progress_at = Instant::now();
    let mut last_signature = String::new();
    let mut recent_samples: Vec<serde_json::Value> = Vec::new();
    let mut last_log = Instant::now()
        .checked_sub(Duration::from_secs(60))
        .unwrap_or_else(Instant::now);

    loop {
        let status = unity_bridge::query_unity_connection_status(project).await;
        let sample = connection_wait_sample(started, &status);
        push_recent_sample(&mut recent_samples, sample.clone());
        let signature = connection_progress_signature(&status);
        if signature != last_signature {
            last_signature = signature;
            last_progress_at = Instant::now();
            emit_json("connection_progress", sample.clone());
        }

        if status.connected {
            return Ok(status);
        }

        if !launched
            && config.open_unity
            && matches!(
                status.editor_process_state,
                UnityEditorProcessState::NotRunning
            )
        {
            let launch = unity_bridge::launch_project(project)?;
            emit_json(
                "unity_launch",
                json!({
                    "editorPath": launch.editor_path,
                    "projectPath": launch.project_path,
                    "projectVersion": launch.project_version,
                }),
            );
            launched = true;
            last_progress_at = Instant::now();
            last_signature = "unity_launch_requested".to_string();
        }

        if last_log.elapsed() >= Duration::from_secs(5) {
            emit_json(
                "waiting_connection",
                json!({
                    "elapsedMs": started.elapsed().as_millis(),
                    "connected": status.connected,
                    "editorStatus": status.editor_status,
                    "processState": status.editor_process_state,
                    "processId": status.editor_process_id,
                    "channel": status.control_channel_state,
                    "lastError": status.last_error,
                }),
            );
            last_log = Instant::now();
        }

        if last_progress_at.elapsed() >= config.no_progress_timeout {
            emit_json(
                "connection_stalled",
                json!({
                    "elapsedMs": started.elapsed().as_millis(),
                    "noProgressMs": last_progress_at.elapsed().as_millis(),
                    "recent": recent_samples,
                }),
            );
            return Err(format!(
                "Unity connection made no progress for {}ms; last channel={}, processState={:?}, processId={:?}, lastError={}",
                config.no_progress_timeout.as_millis(),
                status.control_channel_state,
                status.editor_process_state,
                status.editor_process_id,
                status.last_error.clone().unwrap_or_else(|| "none".to_string())
            ));
        }

        if started.elapsed() >= config.connect_timeout {
            emit_json(
                "connection_timeout",
                json!({
                    "elapsedMs": started.elapsed().as_millis(),
                    "recent": recent_samples,
                }),
            );
            return Err(format!(
                "Unity connection timed out after {}ms",
                config.connect_timeout.as_millis()
            ));
        }
        tokio::time::sleep(config.poll_interval).await;
    }
}

fn connection_progress_signature(status: &UnityConnectionStatus) -> String {
    format!(
        "{:?}|{:?}|{}|{}|{:?}",
        status.editor_process_state,
        status.editor_process_id,
        status.control_channel_state,
        status.editor_status,
        status.last_error
    )
}

fn connection_wait_sample(started: Instant, status: &UnityConnectionStatus) -> serde_json::Value {
    json!({
        "elapsedMs": started.elapsed().as_millis(),
        "connected": status.connected,
        "editorStatus": &status.editor_status,
        "processState": &status.editor_process_state,
        "processId": status.editor_process_id,
        "channel": &status.control_channel_state,
        "lastError": &status.last_error,
    })
}

fn push_recent_sample(samples: &mut Vec<serde_json::Value>, sample: serde_json::Value) {
    const MAX_RECENT_SAMPLES: usize = 8;
    samples.push(sample);
    if samples.len() > MAX_RECENT_SAMPLES {
        samples.remove(0);
    }
}

async fn ensure_edit_mode(
    project: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<(), String> {
    let status = unity_bridge::query_unity_connection_status(project).await;
    if status.editor_status == UNITY_EDITOR_STATUS_EDITING {
        return Ok(());
    }

    emit_json(
        "editor_mode",
        json!({ "action": "set", "desiredStatus": UNITY_EDITOR_STATUS_EDITING }),
    );
    unity_bridge::set_editor_status(project, UNITY_EDITOR_STATUS_EDITING).await?;

    let started = Instant::now();
    loop {
        let status = unity_bridge::query_unity_connection_status(project).await;
        if status.connected && status.editor_status == UNITY_EDITOR_STATUS_EDITING {
            emit_json(
                "editor_mode",
                json!({ "status": UNITY_EDITOR_STATUS_EDITING }),
            );
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "Unity did not reach edit mode within {}ms",
                timeout.as_millis()
            ));
        }
        tokio::time::sleep(poll_interval).await;
    }
}

async fn run_event_selftest<Fut>(
    app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    timeout: Duration,
    start: Fut,
) -> Result<SelfTestSummary, String>
where
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    let Some(event_name) = suite.event_name() else {
        return Err(format!("Suite {} has no self-test event", suite.as_str()));
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SelfTestEvent>();
    let listener = app_handle.listen_any(event_name, move |event| {
        match serde_json::from_str::<SelfTestEvent>(event.payload()) {
            Ok(payload) => {
                let _ = tx.send(payload);
            }
            Err(error) => {
                eprintln!(
                    "[locus-driver] failed to parse self-test event '{}': {}",
                    event.payload(),
                    error
                );
            }
        }
    });

    emit_json(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
            "timeoutMs": timeout.as_millis(),
        }),
    );

    let mut start_task = tokio::spawn(start);
    let timeout_sleep = tokio::time::sleep(timeout);
    tokio::pin!(timeout_sleep);
    let mut start_done = false;

    loop {
        tokio::select! {
            _ = &mut timeout_sleep => {
                if !start_done {
                    start_task.abort();
                }
                app_handle.unlisten(listener);
                return Err(format!("Suite {} timed out after {}ms", suite.as_str(), timeout.as_millis()));
            }
            result = &mut start_task, if !start_done => {
                start_done = true;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        app_handle.unlisten(listener);
                        return Err(format!("Suite {} failed to start: {}", suite.as_str(), error));
                    }
                    Err(error) => {
                        app_handle.unlisten(listener);
                        return Err(format!("Suite {} task failed: {}", suite.as_str(), error));
                    }
                }
            }
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else {
                    app_handle.unlisten(listener);
                    return Err(format!("Suite {} event stream closed", suite.as_str()));
                };
                if let Some(line) = event.line.as_deref() {
                    println!("[locus-driver:{}] {}", suite.as_str(), line);
                }
                emit_json(
                    "suite_event",
                    json!({
                        "suite": suite.as_str(),
                        "running": event.running,
                        "finished": event.finished,
                        "line": event.line,
                        "passed": event.passed,
                        "failed": event.failed,
                    }),
                );
                if event.finished {
                    app_handle.unlisten(listener);
                    let summary = SelfTestSummary {
                        suite,
                        passed: event.passed,
                        failed: event.failed,
                    };
                    emit_json(
                        "suite_result",
                        json!({
                            "suite": suite.as_str(),
                            "passed": summary.passed,
                            "failed": summary.failed,
                        }),
                    );
                    return Ok(summary);
                }
            }
        }
    }
}

fn ensure_summary_passed(summary: SelfTestSummary) -> Result<(), String> {
    if summary.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "Suite {} finished with {} failed check(s)",
            summary.suite.as_str(),
            summary.failed
        ))
    }
}

/// Which transport the Tauri↔Unity command channel resolved to right now. With
/// the native bridge enabled (the default), the in-process broker publishes a
/// shared-memory status plane. Emitted on connect and asserted by the
/// native-bridge suite so a silent fallback is observable.
async fn resolve_active_transport(project: &str) -> &'static str {
    if unity_bridge::native_bridge_enabled() {
        if let Some(status) = unity_bridge::query_native_broker_status(project).await {
            if status.native_alive {
                return "native_broker";
            }
        }
    }
    "managed_pipe"
}

fn emit_json<T: Serialize>(event: &str, payload: T) {
    let line = serde_json::to_string(&DriverEvent { event, payload }).unwrap_or_else(|error| {
        format!(r#"{{"event":"serialization_error","message":"{}"}}"#, error)
    });
    println!("LOCUS_DRIVER_JSON {line}");
}

#[cfg(test)]
mod tests {
    use super::{CliDriverConfig, CliDriverSuite};

    fn parse(args: &[&str]) -> Option<Result<CliDriverConfig, String>> {
        CliDriverConfig::parse(args.iter().map(|arg| arg.to_string()).collect())
    }

    #[test]
    fn parse_ignores_normal_app_start() {
        assert!(parse(&["--foo"]).is_none());
    }

    #[test]
    fn parse_driver_suites_and_timeouts() {
        let parsed = parse(&[
            "--locus-driver",
            "unity-test",
            "--project",
            "F:/Game",
            "--suite",
            "connect,state-probe",
            "--suite",
            "native",
            "--timeout-ms",
            "42",
            "--connect-timeout-ms=77",
            "--no-progress-timeout-ms",
            "33",
            "--no-open-unity",
        ])
        .unwrap()
        .unwrap();

        assert_eq!(parsed.project_path.as_deref(), Some("F:/Game"));
        assert_eq!(
            parsed.suites,
            vec![
                CliDriverSuite::Connect,
                CliDriverSuite::StateProbe,
                CliDriverSuite::NativeBridge
            ]
        );
        assert_eq!(parsed.suite_timeout.as_millis(), 42);
        assert_eq!(parsed.connect_timeout.as_millis(), 77);
        assert_eq!(parsed.no_progress_timeout.as_millis(), 33);
        assert!(!parsed.open_unity);
    }

    #[test]
    fn parse_all_expands_in_stable_order() {
        let parsed = parse(&["--locus-unity-test", "--suite=all"])
            .unwrap()
            .unwrap();

        assert_eq!(
            parsed.suites,
            vec![
                CliDriverSuite::Connect,
                CliDriverSuite::StateProbe,
                CliDriverSuite::NativeBridge,
                CliDriverSuite::HotReload
            ]
        );
    }
}
