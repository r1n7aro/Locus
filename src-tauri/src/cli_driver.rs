use std::{
    future::Future,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Listener};
use tokio::sync::watch;

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
pub const UNITY_INTEGRATION_TEST_EVENT: &str = "unity-integration-test";

/// Sentinel error returned through `run_driver` when the active UI run is
/// cancelled, so `spawn_ui` can emit a `cancelled` event instead of `error`.
pub const UNITY_INTEGRATION_TEST_CANCELLED: &str = "__locus_unity_integration_test_cancelled__";

static UI_RUN_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Cooperative cancel signal for the single in-flight UI run. Set by
/// `unity_integration_test_cancel`, observed by `run_driver` between suites and
/// inside the long connection / self-test waits so an interrupt takes effect
/// without waiting out the remaining timeouts.
static UI_RUN_CANCEL: Mutex<Option<watch::Sender<bool>>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliDriverSuite {
    Connect,
    Sidecar,
    TypeIndex,
    StateProbe,
    NativeBridge,
    HotReload,
    Execute,
}

impl CliDriverSuite {
    fn as_str(self) -> &'static str {
        match self {
            CliDriverSuite::Connect => "connect",
            CliDriverSuite::Sidecar => "sidecar",
            CliDriverSuite::TypeIndex => "type-index",
            CliDriverSuite::StateProbe => "state-probe",
            CliDriverSuite::NativeBridge => "native-bridge",
            CliDriverSuite::HotReload => "hot-reload",
            CliDriverSuite::Execute => "execute",
        }
    }

    fn event_name(self) -> Option<&'static str> {
        match self {
            CliDriverSuite::Connect => None,
            CliDriverSuite::Sidecar => None,
            CliDriverSuite::TypeIndex => None,
            CliDriverSuite::StateProbe => Some("unity-state-probe-selftest"),
            CliDriverSuite::NativeBridge => Some("unity-native-bridge-selftest"),
            CliDriverSuite::HotReload => Some("unity-hotreload-selftest"),
            // Bespoke suite: emits its own suite_* events like sidecar/type-index.
            CliDriverSuite::Execute => None,
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
    pub type_index_sample_mode: crate::unity_type_index_selftest::TypeIndexSampleMode,
    pub connect_timeout: Duration,
    pub suite_timeout: Duration,
    pub poll_interval: Duration,
    pub no_progress_timeout: Duration,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityIntegrationTestRunRequest {
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub suites: Vec<String>,
    #[serde(default)]
    pub open_unity: Option<bool>,
    #[serde(default)]
    pub install_plugin: Option<bool>,
    #[serde(default)]
    pub force_edit_mode: Option<bool>,
    #[serde(default)]
    pub type_index_sample_mode: Option<String>,
    #[serde(default)]
    pub connect_timeout_ms: Option<u64>,
    #[serde(default)]
    pub suite_timeout_ms: Option<u64>,
    #[serde(default)]
    pub poll_ms: Option<u64>,
    #[serde(default)]
    pub no_progress_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityIntegrationTestRunStarted {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverEvent<'a, T: Serialize> {
    event: &'a str,
    payload: T,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DriverUiEvent {
    run_id: String,
    event: String,
    payload: Value,
}

#[derive(Clone)]
struct DriverEventSink {
    app_handle: Option<AppHandle>,
    run_id: Option<String>,
    print_stdout: bool,
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

impl DriverEventSink {
    fn cli() -> Self {
        Self {
            app_handle: None,
            run_id: None,
            print_stdout: true,
        }
    }

    fn ui(app_handle: AppHandle, run_id: String) -> Self {
        Self {
            app_handle: Some(app_handle),
            run_id: Some(run_id),
            print_stdout: false,
        }
    }

    fn emit<T: Serialize>(&self, event: &str, payload: T) {
        if self.print_stdout {
            emit_json(event, &payload);
        }
        if let (Some(app_handle), Some(run_id)) = (&self.app_handle, &self.run_id) {
            let payload = serde_json::to_value(&payload).unwrap_or_else(|error| {
                json!({ "message": format!("event payload serialization failed: {error}") })
            });
            let envelope = DriverUiEvent {
                run_id: run_id.clone(),
                event: event.to_string(),
                payload,
            };
            if let Err(error) = app_handle.emit(UNITY_INTEGRATION_TEST_EVENT, envelope) {
                eprintln!("[locus-driver] failed to emit UI event '{event}': {error}");
            }
        }
    }
}

impl UnityIntegrationTestRunRequest {
    fn into_config(self) -> Result<CliDriverConfig, String> {
        let mut suites = Vec::new();
        if self.suites.is_empty() {
            push_suite(&mut suites, "all")?;
        } else {
            for suite in self.suites {
                push_suite(&mut suites, suite.trim())?;
            }
        }
        Ok(CliDriverConfig {
            project_path: self.project_path,
            suites,
            open_unity: self.open_unity.unwrap_or(true),
            install_plugin: self.install_plugin.unwrap_or(false),
            force_edit_mode: self.force_edit_mode.unwrap_or(true),
            type_index_sample_mode: self
                .type_index_sample_mode
                .as_deref()
                .map(crate::unity_type_index_selftest::TypeIndexSampleMode::parse)
                .transpose()?
                .unwrap_or_default(),
            connect_timeout: Duration::from_millis(
                self.connect_timeout_ms
                    .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS),
            ),
            suite_timeout: Duration::from_millis(
                self.suite_timeout_ms.unwrap_or(DEFAULT_SUITE_TIMEOUT_MS),
            ),
            poll_interval: Duration::from_millis(self.poll_ms.unwrap_or(DEFAULT_POLL_MS)),
            no_progress_timeout: Duration::from_millis(
                self.no_progress_timeout_ms
                    .unwrap_or(DEFAULT_NO_PROGRESS_TIMEOUT_MS),
            ),
        })
    }
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
        let mut type_index_sample_mode =
            crate::unity_type_index_selftest::TypeIndexSampleMode::default();
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
                Some(("--type-index-sample", value)) => {
                    let value =
                        match read_option_value("--type-index-sample", value, &args, &mut index) {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                    type_index_sample_mode =
                        match crate::unity_type_index_selftest::TypeIndexSampleMode::parse(&value) {
                            Ok(value) => value,
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
                _ if arg == "--type-index-full" => {
                    type_index_sample_mode =
                        crate::unity_type_index_selftest::TypeIndexSampleMode::All;
                }
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
            type_index_sample_mode,
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
        | "--no-progress-timeout-ms"
        | "--type-index-sample" => Some((name, value)),
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
                CliDriverSuite::Sidecar,
                CliDriverSuite::TypeIndex,
                CliDriverSuite::StateProbe,
                CliDriverSuite::NativeBridge,
                CliDriverSuite::HotReload,
                CliDriverSuite::Execute,
            ] {
                if !suites.contains(&suite) {
                    suites.push(suite);
                }
            }
            return Ok(());
        }
        "connect" => CliDriverSuite::Connect,
        "sidecar" | "compile-server" | "compile_server" => CliDriverSuite::Sidecar,
        "type-index" | "type_index" | "typeindex" | "schema" | "serialized-schema"
        | "serialized_schema" => CliDriverSuite::TypeIndex,
        "state-probe" | "state_probe" | "state" => CliDriverSuite::StateProbe,
        "native-bridge" | "native_bridge" | "native" => CliDriverSuite::NativeBridge,
        "hot-reload" | "hot_reload" | "hotreload" | "hot" => CliDriverSuite::HotReload,
        "execute" | "exec" | "unity-execute" | "unity_execute" | "execute-code" | "run-states"
        | "run_states" | "runstates" => CliDriverSuite::Execute,
        _ => {
            return Err(format!(
            "Unknown --suite '{}'. Use connect, sidecar, type-index, state-probe, native-bridge, hot-reload, execute, or all.",
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
    // The headless CLI driver is not interruptible; hand `run_driver` a receiver
    // whose sender stays alive for the whole run so its cancel selects never fire.
    let (cancel_tx, cancel_rx) = watch::channel(false);
    tauri::async_runtime::spawn(async move {
        let _cancel_guard = cancel_tx;
        let sink = DriverEventSink::cli();
        let exit_code =
            match run_driver(app_handle.clone(), workspace, config, sink.clone(), cancel_rx).await
        {
            Ok(()) => 0,
            Err(error) => {
                sink.emit("error", json!({ "message": error }));
                1
            }
        };
        app_handle.exit(exit_code);
    });
}

pub fn spawn_ui(
    app_handle: AppHandle,
    workspace: Arc<Workspace>,
    request: UnityIntegrationTestRunRequest,
) -> Result<UnityIntegrationTestRunStarted, String> {
    if UI_RUN_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Unity integration tests are already running".to_string());
    }

    let config = match request.into_config() {
        Ok(config) => config,
        Err(error) => {
            UI_RUN_ACTIVE.store(false, Ordering::SeqCst);
            return Err(error);
        }
    };
    let run_id = uuid::Uuid::new_v4().to_string();
    let sink = DriverEventSink::ui(app_handle.clone(), run_id.clone());
    let (cancel_tx, cancel_rx) = watch::channel(false);
    if let Ok(mut guard) = UI_RUN_CANCEL.lock() {
        *guard = Some(cancel_tx);
    }
    tauri::async_runtime::spawn(async move {
        let result = run_driver(app_handle, workspace, config, sink.clone(), cancel_rx).await;
        match result {
            Ok(()) => {}
            Err(error) if error == UNITY_INTEGRATION_TEST_CANCELLED => {
                sink.emit("cancelled", json!({}));
                sink.emit("finished", json!({ "ok": false, "cancelled": true }));
            }
            Err(error) => {
                sink.emit("error", json!({ "message": error }));
                sink.emit("finished", json!({ "ok": false }));
            }
        }
        if let Ok(mut guard) = UI_RUN_CANCEL.lock() {
            *guard = None;
        }
        UI_RUN_ACTIVE.store(false, Ordering::SeqCst);
    });

    Ok(UnityIntegrationTestRunStarted { run_id })
}

/// Signal the in-flight UI integration-test run (if any) to stop at the next
/// cancellation checkpoint. A no-op when nothing is running.
pub fn cancel_ui() {
    if let Ok(guard) = UI_RUN_CANCEL.lock() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(true);
        }
    }
}

fn run_cancelled(cancel_rx: &watch::Receiver<bool>) -> bool {
    *cancel_rx.borrow()
}

async fn run_driver(
    app_handle: AppHandle,
    workspace: Arc<Workspace>,
    config: CliDriverConfig,
    sink: DriverEventSink,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    sink.emit(
        "start",
        json!({
            "driver": DRIVER_NAME,
            "suites": config.suites.iter().map(|suite| suite.as_str()).collect::<Vec<_>>(),
            "openUnity": config.open_unity,
            "installPlugin": config.install_plugin,
            "typeIndexSampleMode": config.type_index_sample_mode.as_str(),
            "connectTimeoutMs": config.connect_timeout.as_millis(),
            "suiteTimeoutMs": config.suite_timeout.as_millis(),
            "noProgressTimeoutMs": config.no_progress_timeout.as_millis(),
        }),
    );

    let project = resolve_project_path(config.project_path.as_deref(), &workspace).await?;
    set_workspace_for_driver(&workspace, &project).await?;
    prepare_suite_environment(&project, &config, &sink)?;
    check_or_install_plugin(&project, config.install_plugin, &sink).await?;

    let status = ensure_connected(&project, &config, &sink, &mut cancel_rx).await?;
    let transport = resolve_active_transport(&project).await;
    sink.emit(
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
        if run_cancelled(&cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        match suite {
            CliDriverSuite::Connect => {
                sink.emit(
                    "suite_start",
                    json!({ "suite": suite.as_str(), "project": project }),
                );
                let semantic = unity_bridge::unity_semantic_state(&project).await;
                sink.emit(
                    "suite_event",
                    json!({
                        "suite": suite.as_str(),
                        "line": format!(
                            "PASS  connect: semantic phase '{}' (source {})",
                            semantic.phase, semantic.source
                        ),
                        "passed": 1,
                        "failed": 0,
                    }),
                );
                sink.emit(
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
            CliDriverSuite::Sidecar => {
                run_sidecar_suite(&project, *suite, &sink).await?;
            }
            CliDriverSuite::TypeIndex => {
                run_type_index_suite(&project, *suite, config.type_index_sample_mode, &sink).await?;
            }
            CliDriverSuite::StateProbe => {
                unity_bridge::set_state_probe_enabled(true);
                let summary = run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    &sink,
                    &mut cancel_rx,
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
                    &sink,
                    &mut cancel_rx,
                    unity_bridge::run_native_bridge_selftest(app_handle.clone(), project.clone()),
                )
                .await?;
                ensure_summary_passed(summary)?;

                // Confirm the channel actually resolved to the native broker;
                // the suite exists to exercise the required native transport.
                let transport = resolve_active_transport(&project).await;
                sink.emit(
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
                    ensure_edit_mode(
                        &project,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await?;
                }
                let summary = run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    &sink,
                    &mut cancel_rx,
                    crate::unity_hotreload::selftest::run(app_handle.clone(), project.clone()),
                )
                .await?;
                ensure_summary_passed(summary)?;
            }
            CliDriverSuite::Execute => {
                // The execute suite drives the real unity_execute / unity_run_states
                // code paths, so it needs the sidecar compiler warm and (by default)
                // a deterministic edit-mode editor.
                crate::csharp_compile::set_enabled(true).await;
                crate::csharp_compile::warm_up_in_background();
                if config.force_edit_mode {
                    ensure_edit_mode(
                        &project,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await?;
                }
                run_execute_suite(&project, *suite, &config, &sink, &cancel_rx).await?;
            }
        }
    }

    sink.emit("finished", json!({ "ok": true }));
    Ok(())
}

fn prepare_suite_environment(
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    if config
        .suites
        .iter()
        .any(|suite| matches!(suite, CliDriverSuite::NativeBridge))
    {
        unity_bridge::set_native_bridge_enabled(true);
        unity_bridge::sync_native_bridge_marker(project, true)?;
        sink.emit(
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

async fn check_or_install_plugin(
    project: &str,
    install: bool,
    sink: &DriverEventSink,
) -> Result<(), String> {
    match unity_bridge::check_plugin_status(project)? {
        PluginStatus::UpToDate => {
            sink.emit("plugin", json!({ "status": "upToDate" }));
            Ok(())
        }
        status if install => {
            sink.emit(
                "plugin",
                json!({ "status": format!("{status:?}"), "action": "install" }),
            );
            let hash = unity_bridge::install_or_update_plugin(project).await?;
            sink.emit("plugin", json!({ "status": "installed", "hash": hash }));
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
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
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
        if *cancel_rx.borrow() {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        let status = unity_bridge::query_unity_connection_status(project).await;
        let sample = connection_wait_sample(started, &status);
        push_recent_sample(&mut recent_samples, sample.clone());
        let signature = connection_progress_signature(&status);
        if signature != last_signature {
            last_signature = signature;
            last_progress_at = Instant::now();
            sink.emit("connection_progress", sample.clone());
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
            let launch = unity_bridge::launch_project(project).await?;
            sink.emit(
                "unity_launch",
                json!({
                    "editorPath": launch.editor_path,
                    "projectPath": launch.project_path,
                    "projectVersion": launch.project_version,
                    "processId": launch.process_id,
                }),
            );
            launched = true;
            last_progress_at = Instant::now();
            last_signature = "unity_launch_requested".to_string();
        }

        if last_log.elapsed() >= Duration::from_secs(5) {
            sink.emit(
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
            sink.emit(
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
            sink.emit(
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
        tokio::select! {
            _ = tokio::time::sleep(config.poll_interval) => {}
            _ = cancel_rx.changed() => {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
        }
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

async fn run_sidecar_suite(
    project: &str,
    suite: CliDriverSuite,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
        }),
    );

    crate::csharp_compile::set_enabled(true).await;
    let status = crate::csharp_compile::refresh_status().await;
    if !status.platform_supported {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": "sidecar suite requires a supported .NET platform",
                "passed": 0,
                "failed": 1,
            }),
        );
        sink.emit(
            "suite_result",
            json!({ "suite": suite.as_str(), "passed": 0, "failed": 1 }),
        );
        return Err("sidecar suite requires a supported .NET platform".to_string());
    }
    if !status.server_available {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": "sidecar suite requires the bundled LocusCompileServer.dll",
                "passed": 0,
                "failed": 1,
            }),
        );
        sink.emit(
            "suite_result",
            json!({ "suite": suite.as_str(), "passed": 0, "failed": 1 }),
        );
        return Err("sidecar suite requires the bundled LocusCompileServer.dll".to_string());
    }

    let params = crate::csharp_compile::params::get_params(project).await?;

    let outcome = crate::csharp_compile::compile_raw(json!({
        "assemblyName": "__LocusSidecarIntegrationSelfTest",
        "sources": [{
            "path": "SidecarIntegrationSelfTest.cs",
            "text": "public static class SidecarIntegrationSelfTest { public static int Value() { return 42; } }",
        }],
        "params": params,
        "returnAssemblyPath": false,
        "emitDebugSymbols": false,
    }))
    .await?;

    match outcome {
        Ok(compiled) => {
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": format!(
                        "PASS  sidecar compile: assembly '{}' built via compile/raw",
                        compiled.assembly_name
                    ),
                    "passed": 2,
                    "failed": 0,
                }),
            );
            sink.emit(
                "suite_result",
                json!({
                    "suite": suite.as_str(),
                    "passed": 2,
                    "failed": 0,
                    "assemblyName": compiled.assembly_name,
                    "running": crate::csharp_compile::status().await.running,
                }),
            );
            Ok(())
        }
        Err(failure) => {
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": format!("sidecar compile failed at {}: {}", failure.stage, failure.message),
                    "passed": 1,
                    "failed": 1,
                }),
            );
            sink.emit(
                "suite_result",
                json!({
                    "suite": suite.as_str(),
                    "passed": 1,
                    "failed": 1,
                    "stage": failure.stage,
                    "message": failure.message,
                }),
            );
            Err(format!(
                "sidecar compile failed at {}: {}",
                failure.stage, failure.message
            ))
        }
    }
}

async fn run_type_index_suite(
    project: &str,
    suite: CliDriverSuite,
    sample_mode: crate::unity_type_index_selftest::TypeIndexSampleMode,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
        }),
    );

    crate::csharp_compile::set_enabled(true).await;
    let index = match unity_bridge::refresh_unity_type_index(project).await {
        Ok(index) => index,
        Err(error) => {
            emit_suite_failure(sink, suite, &error);
            return Err(error);
        }
    };

    let mut on_progress = |progress: crate::unity_type_index_selftest::TypeIndexProgress| {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": format!(
                    "type-index: {}/{} targets ({}%) · {} properties checked",
                    progress.processed_targets,
                    progress.total_targets,
                    progress.percent,
                    progress.checked_properties
                ),
                "processedTargets": progress.processed_targets,
                "totalTargets": progress.total_targets,
                "percent": progress.percent,
            }),
        );
    };
    let summary = match crate::unity_type_index_selftest::run(project, sample_mode, &mut on_progress)
        .await
    {
        Ok(summary) => summary,
        Err(error) => {
            emit_suite_failure(sink, suite, &error);
            return Err(error);
        }
    };
    if summary.failed > 0 {
        for line in &summary.lines {
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": line,
                    "passed": summary.passed,
                    "failed": summary.failed,
                }),
            );
        }
        for diff in &summary.diffs {
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": diff,
                    "passed": summary.passed,
                    "failed": summary.failed,
                }),
            );
        }
    } else {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": format!(
                    "PASS  type-index: {} checks · {} targets · {} properties matched full schema",
                    summary.passed + 1,
                    summary.checked_targets,
                    summary.checked_properties
                ),
                "passed": summary.passed + 1,
                "failed": 0,
            }),
        );
    }
    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": summary.passed + 1,
            "failed": summary.failed,
            "typeIndexEntryCount": index.entry_count(),
            "typeIndexFingerprint": index.fingerprint,
            "sampleMode": sample_mode.as_str(),
            "checkedTargets": summary.checked_targets,
            "checkedProperties": summary.checked_properties,
            "checkedDiscoverFilters": summary.checked_discover_filters,
            "skippedTargets": summary.skipped_targets,
            "diffs": summary.diffs,
        }),
    );

    if summary.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "type-index suite found {} dynamic/full schema diff(s)",
            summary.failed
        ))
    }
}

fn emit_suite_failure(sink: &DriverEventSink, suite: CliDriverSuite, error: &str) {
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!("ERROR {error}"),
            "passed": 0,
            "failed": 1,
        }),
    );
    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": 0,
            "failed": 1,
            "message": error,
        }),
    );
}

/// Per-check accumulator for the execute suite. Mirrors the self-test `pass`/
/// `fail`/`log` shape so failing lines are streamed as `suite_event`s (buffered
/// by the UI and surfaced only when the suite fails) and totals land in
/// `suite_result`.
struct ExecuteSuiteRun<'a> {
    suite: CliDriverSuite,
    sink: &'a DriverEventSink,
    passed: u32,
    failed: u32,
}

impl<'a> ExecuteSuiteRun<'a> {
    fn new(suite: CliDriverSuite, sink: &'a DriverEventSink) -> Self {
        Self {
            suite,
            sink,
            passed: 0,
            failed: 0,
        }
    }

    fn line(&self, line: String) {
        if self.sink.print_stdout {
            println!("[locus-driver:{}] {}", self.suite.as_str(), line);
        }
        self.sink.emit(
            "suite_event",
            json!({
                "suite": self.suite.as_str(),
                "line": line,
                "passed": self.passed,
                "failed": self.failed,
            }),
        );
    }

    fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.passed += 1;
        let detail = detail.into();
        self.line(format!("PASS  {name}: {detail}"));
    }

    fn fail(&mut self, name: &str, detail: impl Into<String>) {
        self.failed += 1;
        let detail = detail.into();
        self.line(format!("FAIL  {name}: {detail}"));
    }

    /// Run a snippet through the real execute path and require `expect` in the
    /// captured print output.
    async fn check_marker(&mut self, project: &str, name: &str, code: &str, expect: &str) {
        match execute_capture(project, code).await {
            Ok(output) if output.contains(expect) => {
                self.pass(name, format!("got '{}'", clip(&output, 80)));
            }
            Ok(output) => self.fail(
                name,
                format!("expected '{expect}' in output, got '{}'", clip(&output, 160)),
            ),
            Err(error) => self.fail(name, format!("execute error: {}", clip(&error, 200))),
        }
    }

    /// Many sequential executes, each a distinct snippet (and therefore a fresh
    /// compiled assembly). Guards against assembly-churn regressions.
    async fn check_churn(&mut self, project: &str) {
        for i in 1..=8u32 {
            let code = format!(r#"int n = {i}; print("E4:" + (n * n));"#);
            let expect = format!("E4:{}", i * i);
            match execute_capture(project, &code).await {
                Ok(output) if output.contains(&expect) => {}
                Ok(output) => {
                    return self.fail(
                        "E4 churn",
                        format!(
                            "iteration {i} expected '{expect}', got '{}'",
                            clip(&output, 120)
                        ),
                    );
                }
                Err(error) => {
                    return self.fail(
                        "E4 churn",
                        format!("iteration {i} execute error: {}", clip(&error, 160)),
                    );
                }
            }
        }
        self.pass("E4 churn", "8 sequential snippet assemblies executed");
    }

    /// The same snippet body (same host type name) loaded into distinct
    /// assemblies repeatedly must not collide in the domain.
    async fn check_same_type_reload(&mut self, project: &str) {
        for attempt in 1..=3u32 {
            match execute_capture(project, r#"print("E5:" + (6 * 7));"#).await {
                Ok(output) if output.contains("E5:42") => {}
                Ok(output) => {
                    return self.fail(
                        "E5 same-type",
                        format!("attempt {attempt} got '{}'", clip(&output, 120)),
                    );
                }
                Err(error) => {
                    return self.fail(
                        "E5 same-type",
                        format!("attempt {attempt} execute error: {}", clip(&error, 160)),
                    );
                }
            }
        }
        self.pass(
            "E5 same-type",
            "same host type reloaded 3x without collision",
        );
    }

    /// A snippet reports api progress between frame waits; assert the Rust-side
    /// poll observed at least one api snapshot with non-decreasing revisions.
    async fn check_progress(&mut self, project: &str) {
        let stats = Arc::new(std::sync::Mutex::new(ProgressStats::default()));
        let observer = Arc::clone(&stats);
        // Wall-clock waits (not frame counts) so the 250ms Rust-side progress
        // poll reliably samples the streamed api progress on a fast editor.
        let code = r#"for (int i = 0; i < 4; i++)
{
    ctx.Progress("Locus execute self-test", "step " + i, (i + 1) / 4f);
    await ctx.WaitSeconds(0.3f);
}
print("E7:done");"#;
        let result =
            unity_bridge::unity_execute_code_with_progress(project, code, move |snapshot| {
                if let Ok(mut s) = observer.lock() {
                    s.total += 1;
                    if snapshot.source == "api" {
                        s.api += 1;
                        if snapshot.revision < s.last_api_revision {
                            s.api_regressions += 1;
                        }
                        s.last_api_revision = snapshot.revision;
                    }
                }
            })
            .await;

        let observed = stats.lock().map(|s| s.clone()).unwrap_or_default();
        match result {
            Ok(output) if output.contains("E7:done") => {
                if observed.api == 0 {
                    self.fail(
                        "E7 progress",
                        "snippet finished but no api progress snapshots streamed back",
                    );
                } else if observed.api_regressions > 0 {
                    self.fail(
                        "E7 progress",
                        format!("api progress revision regressed {}x", observed.api_regressions),
                    );
                } else {
                    self.pass(
                        "E7 progress",
                        format!(
                            "{} api / {} total snapshots, revisions monotonic",
                            observed.api, observed.total
                        ),
                    );
                }
            }
            Ok(output) => self.fail(
                "E7 progress",
                format!("expected 'E7:done', got '{}'", clip(&output, 160)),
            ),
            Err(error) => self.fail("E7 progress", format!("execute error: {}", clip(&error, 200))),
        }
    }

    /// A long-running blocking execute must abort promptly when cancelled
    /// instead of running to completion.
    async fn check_cancellation(&mut self, project: &str) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        let code = r#"await ctx.WaitSeconds(120); print("E8:should-not-finish");"#;
        let started = Instant::now();
        let (result, _) = tokio::join!(
            unity_bridge::unity_execute_code_with_progress_cancellable(
                project,
                code,
                rx,
                |_snapshot| {},
            ),
            async move {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                let _ = tx.send(true);
            }
        );
        let elapsed = started.elapsed();
        match result {
            Err(error) if error == unity_bridge::UNITY_EXECUTE_CANCELLED => {
                if elapsed <= Duration::from_secs(30) {
                    self.pass(
                        "E8 cancel",
                        format!("blocking execute cancelled in {}ms", elapsed.as_millis()),
                    );
                } else {
                    self.fail(
                        "E8 cancel",
                        format!("cancelled but took {}ms (>30s)", elapsed.as_millis()),
                    );
                }
            }
            Err(error) => self.fail(
                "E8 cancel",
                format!("expected cancellation, got error: {}", clip(&error, 160)),
            ),
            Ok(output) => self.fail(
                "E8 cancel",
                format!(
                    "expected cancellation, snippet completed: '{}'",
                    clip(&output, 120)
                ),
            ),
        }
    }

    /// Two executes fired concurrently must serialize on the per-project op lock
    /// and both complete with their own, un-corrupted output.
    async fn check_concurrency(&mut self, project: &str) {
        let code_a = r#"await ctx.WaitFrames(15); print("E9A:ok");"#;
        let code_b = r#"await ctx.WaitFrames(15); print("E9B:ok");"#;
        let (ra, rb) = tokio::join!(
            execute_capture(project, code_a),
            execute_capture(project, code_b)
        );
        let a_ok = matches!(&ra, Ok(output) if output.contains("E9A:ok"));
        let b_ok = matches!(&rb, Ok(output) if output.contains("E9B:ok"));
        if a_ok && b_ok {
            self.pass(
                "E9 serialize",
                "two concurrent executes both completed correctly",
            );
        } else {
            self.fail(
                "E9 serialize",
                format!(
                    "A={}, B={}",
                    describe_result(&ra, "E9A:ok"),
                    describe_result(&rb, "E9B:ok")
                ),
            );
        }
    }

    /// The legacy in-Unity compile path (`execute_code`) — exercised by turning
    /// the sidecar off for a single round trip — still compiles and executes.
    async fn check_legacy_compile(&mut self, project: &str) {
        let was_enabled = crate::csharp_compile::is_enabled();
        crate::csharp_compile::set_enabled(false).await;
        let result = execute_capture(project, r#"print("E12:" + (21 + 21));"#).await;
        if was_enabled {
            crate::csharp_compile::set_enabled(true).await;
        }
        match result {
            Ok(output) if output.contains("E12:42") => {
                self.pass("E12 legacy-compile", "in-Unity compile path executed")
            }
            Ok(output) => self.fail(
                "E12 legacy-compile",
                format!("expected 'E12:42', got '{}'", clip(&output, 120)),
            ),
            Err(error) => self.fail(
                "E12 legacy-compile",
                format!("execute error: {}", clip(&error, 160)),
            ),
        }
    }

    /// A two-state run-states machine transitions A -> B and completes.
    async fn check_run_states(&mut self, project: &str) {
        let request = json!({
            "request_editor_status": "editing",
            "initial_state": "A",
            "states": [
                { "name": "A", "update": "print(\"E11A\"); ctx.Goto(\"B\");" },
                { "name": "B", "update": "print(\"E11B\"); ctx.Done(\"e11-complete\");" },
            ],
        });
        match unity_bridge::unity_run_states(project, &request).await {
            Ok(output) => {
                let ok = output.contains("status: ok")
                    && output.contains("final_state: B")
                    && output.contains("E11A")
                    && output.contains("E11B");
                if ok {
                    self.pass(
                        "E11 run-states",
                        "two-state machine transitioned A->B and completed",
                    );
                } else {
                    self.fail(
                        "E11 run-states",
                        format!("unexpected run-states output: '{}'", clip(&output, 200)),
                    );
                }
            }
            Err(error) => self.fail(
                "E11 run-states",
                format!("run-states error: {}", clip(&error, 200)),
            ),
        }
    }

    /// Full recompile: add a brand-new type to the project, ask Unity to
    /// recompile, confirm a fresh execute resolves it through the domain reload,
    /// then remove the script and recompile back to the original state.
    async fn check_recompile(&mut self, project: &str, config: &CliDriverConfig) {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let type_name = format!("LocusExecuteSelfTestSubject_{}", &token[..8]);
        let rel_dir = "Assets/LocusExecuteSelfTest";
        let dir = Path::new(project).join("Assets").join("LocusExecuteSelfTest");
        let file = dir.join(format!("{type_name}.cs"));
        let meta = dir.join(format!("{type_name}.cs.meta"));

        let presence_probe = format!(
            r#"bool found = System.AppDomain.CurrentDomain.GetAssemblies().Any(a => a.GetType("{type_name}") != null); print("E10:" + (found ? "present" : "absent"));"#
        );

        // 1. The new type must not already exist.
        match execute_capture(project, &presence_probe).await {
            Ok(output) if output.contains("E10:absent") => {}
            Ok(output) => {
                return self.fail(
                    "E10 recompile",
                    format!("pre-check expected absent, got '{}'", clip(&output, 120)),
                );
            }
            Err(error) => {
                return self.fail(
                    "E10 recompile",
                    format!("pre-check execute error: {}", clip(&error, 160)),
                );
            }
        }

        // 2. Write the script and ask Unity to import + recompile. The triggering
        //    execute may be torn down by the domain reload — that is expected.
        let source = format!(
            "public class {type_name}\n{{\n    public static int Answer() {{ return 1234; }}\n}}\n"
        );
        if let Err(error) = std::fs::create_dir_all(&dir) {
            return self.fail(
                "E10 recompile",
                format!("failed to create {}: {error}", dir.display()),
            );
        }
        if let Err(error) = std::fs::write(&file, source) {
            let _ = std::fs::remove_dir_all(&dir);
            return self.fail(
                "E10 recompile",
                format!("failed to write {}: {error}", file.display()),
            );
        }
        self.line(format!("E10 recompile: wrote {}", file.display()));

        let import = format!(
            r#"AssetDatabase.ImportAsset("{rel_dir}/{type_name}.cs", ImportAssetOptions.ForceUpdate); AssetDatabase.Refresh(); print("E10:refresh-requested");"#
        );
        let _ = execute_capture(project, &import).await;

        // 3. Wait through the domain reload until a fresh execute resolves the
        //    newly compiled type.
        let post_probe = format!(
            r#"var t = System.AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType("{type_name}")).FirstOrDefault(x => x != null); if (t == null) {{ print("E10:absent"); }} else {{ print("E10:answer=" + t.GetMethod("Answer").Invoke(null, null)); }}"#
        );
        let resolve_deadline = Instant::now() + recompile_wait(config);
        let mut resolved = false;
        let mut last_detail = String::from("no response");
        while Instant::now() < resolve_deadline {
            match execute_capture(project, &post_probe).await {
                Ok(output) if output.contains("E10:answer=1234") => {
                    resolved = true;
                    break;
                }
                Ok(output) => last_detail = clip(&output, 120),
                Err(error) => last_detail = clip(&error, 120),
            }
            tokio::time::sleep(config.poll_interval).await;
        }
        if resolved {
            self.pass(
                "E10 recompile",
                format!("new type '{type_name}' resolved after recompile"),
            );
        } else {
            self.fail(
                "E10 recompile",
                format!("new type did not resolve within timeout (last: {last_detail})"),
            );
        }

        // 4. Remove the script and recompile back so the project is left clean.
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(&meta);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = execute_capture(project, r#"AssetDatabase.Refresh(); print("E10:cleanup");"#).await;
        let cleanup_deadline = Instant::now() + recompile_wait(config);
        let mut cleaned = false;
        while Instant::now() < cleanup_deadline {
            if let Ok(output) = execute_capture(project, &presence_probe).await {
                if output.contains("E10:absent") {
                    cleaned = true;
                    break;
                }
            }
            tokio::time::sleep(config.poll_interval).await;
        }
        if cleaned {
            self.line("E10 recompile: project restored (type removed, recompiled back)".to_string());
        } else {
            self.line(
                "E10 recompile: WARNING test script removed but project may still be recompiling"
                    .to_string(),
            );
        }
    }
}

#[derive(Clone, Default)]
struct ProgressStats {
    total: u32,
    api: u32,
    api_regressions: u32,
    last_api_revision: u64,
}

/// Run one snippet through the real execute path and return its captured output.
async fn execute_capture(project: &str, code: &str) -> Result<String, String> {
    unity_bridge::unity_execute_code_with_progress(project, code, |_snapshot| {}).await
}

fn describe_result(result: &Result<String, String>, expect: &str) -> String {
    match result {
        Ok(output) if output.contains(expect) => "ok".to_string(),
        Ok(output) => format!("missing marker ('{}')", clip(output, 80)),
        Err(error) => format!("error ('{}')", clip(error, 80)),
    }
}

/// Per-phase wait budget for a domain reload — bounded so a wedged recompile
/// still terminates the suite within a few minutes.
fn recompile_wait(config: &CliDriverConfig) -> Duration {
    config
        .suite_timeout
        .min(Duration::from_secs(180))
        .max(Duration::from_secs(60))
}

fn clip(text: &str, max: usize) -> String {
    let collapsed = text.trim().replace(['\n', '\r'], " ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(max).collect();
    format!("{truncated}…")
}

/// Drives the real `unity_execute` / `unity_run_states` code paths end to end:
/// round-trip correctness, many sequential compiled snippets, async/blocking
/// execution with progress + cancellation, op-lock serialization, the legacy
/// in-Unity compile path, a run-states transition, and a full new-type
/// recompile. Bespoke suite shaped like `run_sidecar_suite`: emits `suite_event`
/// lines per check and a final `suite_result`, returning `Err` if any failed.
async fn run_execute_suite(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &watch::Receiver<bool>,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
        }),
    );

    let mut run = ExecuteSuiteRun::new(suite, sink);

    // Baseline correctness: compile -> load -> run -> capture output, with both
    // UnityEngine and UnityEditor references resolving on the editor main thread.
    run.check_marker(project, "E1 round-trip", r#"print("E1:" + (40 + 2));"#, "E1:42")
        .await;
    run.check_marker(
        project,
        "E2 unity-engine",
        r#"print("E2:" + Application.unityVersion);"#,
        "E2:",
    )
    .await;
    run.check_marker(
        project,
        "E3 edit-mode",
        r#"print("E3:" + EditorApplication.isPlaying);"#,
        "E3:False",
    )
    .await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Multiple executes / new compiled assemblies.
    run.check_churn(project).await;
    run.check_same_type_reload(project).await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Blocking / async execution: frame waits, streamed progress, cancellation,
    // and op-lock serialization of concurrent calls.
    run.check_marker(
        project,
        "E6 frame-wait",
        r#"await ctx.WaitFrames(20); print("E6:done");"#,
        "E6:done",
    )
    .await;
    run.check_progress(project).await;
    run.check_cancellation(project).await;
    run.check_concurrency(project).await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Alternate compile backend and the run-states path.
    run.check_legacy_compile(project).await;
    run.check_run_states(project).await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Full recompile + new-type resolution (slowest, last).
    run.check_recompile(project, config).await;

    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": run.passed,
            "failed": run.failed,
        }),
    );

    if run.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "execute suite finished with {} failed check(s)",
            run.failed
        ))
    }
}

async fn ensure_edit_mode(
    project: &str,
    timeout: Duration,
    poll_interval: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let status = unity_bridge::query_unity_connection_status(project).await;
    if status.editor_status == UNITY_EDITOR_STATUS_EDITING {
        return Ok(());
    }

    sink.emit(
        "editor_mode",
        json!({ "action": "set", "desiredStatus": UNITY_EDITOR_STATUS_EDITING }),
    );
    unity_bridge::set_editor_status(project, UNITY_EDITOR_STATUS_EDITING).await?;

    let started = Instant::now();
    loop {
        if *cancel_rx.borrow() {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        let status = unity_bridge::query_unity_connection_status(project).await;
        if status.connected && status.editor_status == UNITY_EDITOR_STATUS_EDITING {
            sink.emit(
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
        tokio::select! {
            _ = tokio::time::sleep(poll_interval) => {}
            _ = cancel_rx.changed() => {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
        }
    }
}

async fn run_event_selftest<Fut>(
    app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    timeout: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
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

    sink.emit(
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
            _ = cancel_rx.changed() => {
                if !start_done {
                    start_task.abort();
                }
                app_handle.unlisten(listener);
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
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
                // Forward every emitted line live so the UI output console fills
                // in as the self-test runs, not only when it fails.
                if let Some(line) = event.line.clone() {
                    if sink.print_stdout {
                        println!("[locus-driver:{}] {}", suite.as_str(), line);
                    }
                    sink.emit(
                        "suite_event",
                        json!({
                            "suite": suite.as_str(),
                            "running": event.running,
                            "finished": event.finished,
                            "line": line,
                            "passed": event.passed,
                            "failed": event.failed,
                        }),
                    );
                }
                if event.finished {
                    app_handle.unlisten(listener);
                    let summary = SelfTestSummary {
                        suite,
                        passed: event.passed,
                        failed: event.failed,
                    };
                    sink.emit(
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

fn emit_json<T: Serialize>(event: &str, payload: &T) {
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
                CliDriverSuite::Sidecar,
                CliDriverSuite::TypeIndex,
                CliDriverSuite::StateProbe,
                CliDriverSuite::NativeBridge,
                CliDriverSuite::HotReload,
                CliDriverSuite::Execute
            ]
        );
    }

    #[test]
    fn parse_execute_suite_aliases() {
        for alias in ["execute", "exec", "unity-execute", "execute-code", "run-states"] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(parsed.suites, vec![CliDriverSuite::Execute], "alias {alias}");
        }
    }
}
