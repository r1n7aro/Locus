use std::{
    collections::BTreeMap,
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
        UnityLaunchCodeOptimization, UNITY_EDITOR_STATUS_EDITING,
    },
    workspace::Workspace,
};

const DRIVER_NAME: &str = "unity-test";
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_SUITE_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_POLL_MS: u64 = 500;
const DEFAULT_NO_PROGRESS_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_YAML_PARITY_SAMPLE_COUNT: u32 = 5;
const POST_PLUGIN_INSTALL_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
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
    HotReloadRelease,
    ParallelEditRefresh,
    Execute,
    YamlParity,
    UnityTest,
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
            CliDriverSuite::HotReloadRelease => "hot-reload-release",
            CliDriverSuite::ParallelEditRefresh => "parallel-edit-refresh",
            CliDriverSuite::Execute => "execute",
            CliDriverSuite::YamlParity => "yaml-parity",
            CliDriverSuite::UnityTest => "unity-test",
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
            CliDriverSuite::HotReloadRelease => Some("unity-hotreload-selftest"),
            CliDriverSuite::ParallelEditRefresh => None,
            // Bespoke suite: emits its own suite_* events like sidecar/type-index.
            CliDriverSuite::Execute => None,
            CliDriverSuite::YamlParity => None,
            CliDriverSuite::UnityTest => None,
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
    pub yaml_parity_sample_count: u32,
    pub yaml_parity_seed: i32,
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
    pub yaml_parity_sample_count: Option<u32>,
    #[serde(default)]
    pub yaml_parity_seed: Option<i32>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginPrepareOutcome {
    UpToDate,
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticReadyRequirement {
    UnityApi,
    AssetModification,
}

impl SemanticReadyRequirement {
    fn as_str(self) -> &'static str {
        match self {
            SemanticReadyRequirement::UnityApi => "unityApi",
            SemanticReadyRequirement::AssetModification => "assetModification",
        }
    }
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
            yaml_parity_sample_count: self
                .yaml_parity_sample_count
                .unwrap_or(DEFAULT_YAML_PARITY_SAMPLE_COUNT)
                .clamp(1, 50),
            yaml_parity_seed: self.yaml_parity_seed.unwrap_or(0),
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

    fn launch_code_optimization(&self) -> Option<UnityLaunchCodeOptimization> {
        if self
            .suites
            .iter()
            .any(|suite| matches!(suite, CliDriverSuite::HotReloadRelease))
        {
            Some(UnityLaunchCodeOptimization::Release)
        } else {
            None
        }
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
        let mut yaml_parity_sample_count = DEFAULT_YAML_PARITY_SAMPLE_COUNT;
        let mut yaml_parity_seed = 0i32;
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
                Some(("--yaml-parity-samples", value)) => {
                    let value = match read_option_value(
                        "--yaml-parity-samples",
                        value,
                        &args,
                        &mut index,
                    ) {
                        Ok(value) => value,
                        Err(error) => return Some(Err(error)),
                    };
                    yaml_parity_sample_count = match value.parse::<u32>() {
                        Ok(value) if (1..=50).contains(&value) => value,
                        _ => {
                            return Some(Err(
                                "--yaml-parity-samples requires an integer from 1 to 50"
                                    .to_string(),
                            ))
                        }
                    };
                }
                Some(("--yaml-parity-seed", value)) => {
                    let value =
                        match read_option_value("--yaml-parity-seed", value, &args, &mut index) {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                    yaml_parity_seed = match value.parse::<i32>() {
                        Ok(value) => value,
                        Err(_) => {
                            return Some(Err(
                                "--yaml-parity-seed requires a signed 32-bit integer".to_string()
                            ))
                        }
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
            yaml_parity_sample_count,
            yaml_parity_seed,
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
        | "--type-index-sample"
        | "--yaml-parity-samples"
        | "--yaml-parity-seed" => Some((name, value)),
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
                CliDriverSuite::HotReloadRelease,
                CliDriverSuite::ParallelEditRefresh,
                CliDriverSuite::Execute,
                CliDriverSuite::YamlParity,
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
        "hot-reload-release" | "hot_reload_release" | "hotrelease" | "hot-release"
        | "hot_release" | "release-hot-reload" | "release_hot_reload" => {
            CliDriverSuite::HotReloadRelease
        }
        "parallel-edit-refresh" | "parallel_edit_refresh" | "parallel-refresh"
        | "parallel_refresh" | "edit-refresh" | "edit_refresh" => {
            CliDriverSuite::ParallelEditRefresh
        }
        "execute" | "exec" | "unity-execute" | "unity_execute" | "execute-code" | "run-states"
        | "run_states" | "runstates" => CliDriverSuite::Execute,
        "yaml-parity" | "yaml_parity" | "yaml-diff" | "yaml_diff" => {
            CliDriverSuite::YamlParity
        }
        "unity-test" | "unity_test" | "test-framework" | "test_framework" => {
            CliDriverSuite::UnityTest
        }
        _ => {
            return Err(format!(
            "Unknown --suite '{}'. Use connect, sidecar, type-index, state-probe, native-bridge, hot-reload, hot-reload-release, parallel-edit-refresh, execute, yaml-parity, unity-test, or all.",
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
        let exit_code = match run_driver(
            app_handle.clone(),
            workspace,
            config,
            sink.clone(),
            cancel_rx,
        )
        .await
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
            "yamlParitySampleCount": config.yaml_parity_sample_count,
            "yamlParitySeed": config.yaml_parity_seed,
            "connectTimeoutMs": config.connect_timeout.as_millis(),
            "suiteTimeoutMs": config.suite_timeout.as_millis(),
            "noProgressTimeoutMs": config.no_progress_timeout.as_millis(),
        }),
    );

    let project = resolve_project_path(config.project_path.as_deref(), &workspace).await?;
    set_workspace_for_driver(&workspace, &project).await?;
    prepare_suite_environment(&project, &config, &sink)?;
    let plugin_outcome = check_or_install_plugin(&project, config.install_plugin, &sink).await?;

    let status = ensure_connected(&project, &config, plugin_outcome, &sink, &mut cancel_rx).await?;
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

    let mut suite_failures = Vec::new();

    for suite in &config.suites {
        if run_cancelled(&cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        let suite_result = match suite {
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
                Ok(())
            }
            CliDriverSuite::Sidecar => run_sidecar_suite(&project, *suite, &sink).await,
            CliDriverSuite::TypeIndex => {
                run_type_index_suite(&project, *suite, config.type_index_sample_mode, &sink).await
            }
            CliDriverSuite::StateProbe => {
                unity_bridge::set_state_probe_enabled(true);
                match run_event_selftest(
                    &app_handle,
                    &project,
                    *suite,
                    config.suite_timeout,
                    config.no_progress_timeout,
                    &sink,
                    &mut cancel_rx,
                    unity_bridge::run_state_probe_selftest(app_handle.clone(), project.clone()),
                )
                .await
                {
                    Ok(summary) => ensure_summary_passed(summary),
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::NativeBridge => {
                unity_bridge::set_native_bridge_enabled(true);
                match unity_bridge::sync_native_bridge_marker(&project, true) {
                    Ok(()) => {
                        match run_event_selftest(
                            &app_handle,
                            &project,
                            *suite,
                            config.suite_timeout,
                            config.no_progress_timeout,
                            &sink,
                            &mut cancel_rx,
                            unity_bridge::run_native_bridge_selftest(
                                app_handle.clone(),
                                project.clone(),
                            ),
                        )
                        .await
                        {
                            Ok(summary) => {
                                let result = ensure_summary_passed(summary);

                                // Confirm the channel actually resolved to the native broker;
                                // the suite exists to exercise the required native transport.
                                let transport = resolve_active_transport(&project).await;
                                sink.emit(
                                    "native_transport_confirmed",
                                    json!({ "suite": suite.as_str(), "transport": transport }),
                                );
                                if transport != "native_broker" {
                                    Err(format!(
                                        "native-bridge suite ran over '{transport}', expected 'native_broker'"
                                    ))
                                } else {
                                    result
                                }
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::HotReload | CliDriverSuite::HotReloadRelease => {
                run_hot_reload_suite(
                    &app_handle,
                    &project,
                    *suite,
                    &config,
                    plugin_outcome,
                    &sink,
                    &mut cancel_rx,
                    matches!(*suite, CliDriverSuite::HotReloadRelease),
                )
                .await
            }
            CliDriverSuite::ParallelEditRefresh => {
                let edit_mode_result = if config.force_edit_mode {
                    ensure_edit_mode(
                        &project,
                        *suite,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await
                } else {
                    Ok(())
                };
                match edit_mode_result {
                    Ok(()) => {
                        run_parallel_edit_refresh_suite(
                            &project,
                            *suite,
                            &config,
                            &sink,
                            &mut cancel_rx,
                        )
                        .await
                    }
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::Execute => {
                // The execute suite drives the real unity_execute / unity_run_states
                // code paths, so it needs the sidecar compiler warm and (by default)
                // a deterministic edit-mode editor.
                crate::csharp_compile::set_enabled(true).await;
                crate::csharp_compile::warm_up_in_background();
                let edit_mode_result = if config.force_edit_mode {
                    ensure_edit_mode(
                        &project,
                        *suite,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await
                } else {
                    Ok(())
                };
                match edit_mode_result {
                    Ok(()) => {
                        run_execute_suite(&project, *suite, &config, &sink, &mut cancel_rx).await
                    }
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::YamlParity => {
                let edit_mode_result = if config.force_edit_mode {
                    ensure_edit_mode(
                        &project,
                        *suite,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await
                } else {
                    Ok(())
                };
                match edit_mode_result {
                    Ok(()) => run_yaml_parity_suite(&project, *suite, &config, &sink).await,
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::UnityTest => {
                let edit_mode_result = if config.force_edit_mode {
                    ensure_edit_mode(
                        &project,
                        *suite,
                        config.connect_timeout,
                        config.poll_interval,
                        &sink,
                        &mut cancel_rx,
                    )
                    .await
                } else {
                    Ok(())
                };
                match edit_mode_result {
                    Ok(()) => run_unity_test_suite(&project, *suite, &config, &sink).await,
                    Err(error) => Err(error),
                }
            }
        };

        if let Err(error) = suite_result {
            if error == UNITY_INTEGRATION_TEST_CANCELLED {
                return Err(error);
            }
            let message = error;
            let stop_run = should_stop_after_suite_error(&message);
            sink.emit(
                "suite_error",
                json!({
                    "suite": suite.as_str(),
                    "message": message.clone(),
                }),
            );
            suite_failures.push(format!("{}: {message}", suite.as_str()));
            if stop_run {
                return Err(format_suite_failures(&suite_failures));
            }
        }
    }

    if !suite_failures.is_empty() {
        return Err(format_suite_failures(&suite_failures));
    }

    sink.emit("finished", json!({ "ok": true }));
    Ok(())
}

async fn run_yaml_parity_suite(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
            "sampleCount": config.yaml_parity_sample_count,
            "seed": config.yaml_parity_seed,
        }),
    );

    let request = json!({
        "sample_count": config.yaml_parity_sample_count,
        "seed": config.yaml_parity_seed,
    });
    let text = unity_bridge::yaml_preview_cache_selftest(project, &request).await?;
    let report: Value = serde_json::from_str(&text)
        .map_err(|error| format!("YAML parity self-test returned invalid JSON: {error}"))?;
    let passed = report
        .get("passed")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let failed = report
        .get("failed")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let skipped = report
        .get("skipped")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    if let Some(cases) = report.get("cases").and_then(Value::as_array) {
        for case in cases {
            let status = case
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("failed");
            let scene = case
                .get("scene_path")
                .and_then(Value::as_str)
                .unwrap_or("<unknown>");
            let message = case.get("message").and_then(Value::as_str).unwrap_or("");
            let marker = match status {
                "passed" => "PASS ",
                "skipped" => "SKIP ",
                _ => "FAIL ",
            };
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": format!("{marker} yaml-parity: {scene} {message}"),
                    "passed": passed,
                    "failed": failed,
                }),
            );
        }
    }

    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
            "previewSupported": report.get("preview_supported"),
            "unityVersion": report.get("unity_version"),
            "mode": report.get("mode"),
            "seed": report.get("seed"),
            "sampleCount": report.get("sample_count"),
            "candidateCount": report.get("candidate_count"),
        }),
    );

    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "YAML parity suite finished with {failed} failed scene check(s)"
        ))
    }
}

async fn run_unity_test_suite(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({ "suite": suite.as_str(), "project": project }),
    );

    let workspace_status = crate::workspace::unity_test_tools_workspace_status(project);
    if !workspace_status.enabled {
        return Err(
            "Unity Test tools are disabled in this workspace's Locus/config.json".to_string(),
        );
    }
    if !workspace_status.package_installed {
        return Err("com.unity.test-framework is not installed in this project".to_string());
    }

    let recompile = unity_bridge::recompile_and_wait(project).await?;
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!("PASS  unity-test convergence: {recompile}"),
            "passed": 1,
            "failed": 0,
        }),
    );

    let list_request = json!({ "max_results": 50 });
    let list_text = unity_bridge::unity_test_list(project, &list_request).await?;
    let list: Value = serde_json::from_str(&list_text)
        .map_err(|error| format!("Unity Test list returned invalid JSON: {error}"))?;
    let list_mode = list.get("mode").and_then(Value::as_str).unwrap_or_default();
    if list_mode != "edit|play" {
        return Err(format!(
            "Unity Test list defaulted to unexpected mode '{list_mode}'"
        ));
    }
    let matched = list
        .get("matched")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if matched == 0 {
        return Err("Unity Test Framework discovered no tests".to_string());
    }
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!("PASS  unity-test list: discovered {matched} Edit/Play Mode test(s)"),
            "passed": 2,
            "failed": 0,
        }),
    );

    let run_request = json!({ "mode": "edit|play", "result_detail": "failures" });
    let result = unity_bridge::unity_test_run(project, &run_request, config.suite_timeout).await?;
    if result.mode != "edit|play" {
        return Err(format!(
            "Unity Test run used unexpected mode '{}'",
            result.mode
        ));
    }
    let failed = u64::from(result.status != "passed");
    let passed_checks = if failed == 0 { 3 } else { 2 };
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!(
                "{} unity-test run: {} passed, {} failed, {} skipped, {} inconclusive",
                if failed == 0 { "PASS " } else { "FAIL " },
                result.passed,
                result.failed,
                result.skipped,
                result.inconclusive,
            ),
            "passed": passed_checks,
            "failed": failed,
        }),
    );
    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": passed_checks,
            "failed": failed,
            "tests": {
                "total": result.total,
                "passed": result.passed,
                "failed": result.failed,
                "skipped": result.skipped,
                "inconclusive": result.inconclusive,
            },
            "failures": result.failures,
        }),
    );

    if failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "Unity Test Framework run failed ({} failed test(s))",
            result.failed
        ))
    }
}

fn prepare_suite_environment(
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    if config.suites.iter().any(|suite| {
        matches!(
            suite,
            CliDriverSuite::NativeBridge | CliDriverSuite::ParallelEditRefresh
        )
    }) {
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
) -> Result<PluginPrepareOutcome, String> {
    match unity_bridge::check_plugin_status(project)? {
        PluginStatus::UpToDate => {
            sink.emit("plugin", json!({ "status": "upToDate" }));
            Ok(PluginPrepareOutcome::UpToDate)
        }
        status if install => {
            sink.emit(
                "plugin",
                json!({ "status": format!("{status:?}"), "action": "install" }),
            );
            let hash = unity_bridge::install_or_update_plugin(project).await?;
            sink.emit("plugin", json!({ "status": "installed", "hash": hash }));
            Ok(PluginPrepareOutcome::Installed)
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
    plugin_outcome: PluginPrepareOutcome,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<UnityConnectionStatus, String> {
    let started = Instant::now();
    let connect_timeout = connection_timeout_for_plugin_outcome(config, plugin_outcome);
    let reload_aware_wait = plugin_outcome == PluginPrepareOutcome::Installed;
    if reload_aware_wait {
        unity_bridge::set_state_probe_enabled(true);
        unity_bridge::start_unity_semantic_state_observer(project);
        sink.emit(
            "connection_wait_mode",
            json!({
                "reason": "pluginInstalled",
                "connectTimeoutMs": connect_timeout.as_millis(),
                "baseConnectTimeoutMs": config.connect_timeout.as_millis(),
                "stateProbe": true,
            }),
        );
    }
    let mut launched = false;
    let mut last_progress_at = Instant::now();
    let mut last_signature = String::new();
    let mut last_semantic_signature = String::new();
    let mut last_semantic_sample: Option<serde_json::Value> = None;
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

        let mut semantic_waiting = false;
        if reload_aware_wait {
            let semantic = unity_bridge::unity_semantic_state(project).await;
            semantic_waiting = semantic_state_is_reload_wait(&semantic);
            let semantic_sample = semantic_connection_wait_sample(&semantic);
            last_semantic_sample = Some(semantic_sample.clone());
            let semantic_signature = semantic_connection_wait_signature(&semantic);
            if semantic_signature != last_semantic_signature {
                last_semantic_signature = semantic_signature;
                if semantic_waiting {
                    last_progress_at = Instant::now();
                }
                sink.emit("connection_semantic_progress", semantic_sample);
            }
        }

        if !launched
            && config.open_unity
            && matches!(
                status.editor_process_state,
                UnityEditorProcessState::NotRunning
            )
        {
            let launch_code_optimization = config.launch_code_optimization();
            let launch =
                unity_bridge::launch_project_with_options(project, launch_code_optimization)
                    .await?;
            sink.emit(
                "unity_launch",
                json!({
                    "editorPath": launch.editor_path,
                    "projectPath": launch.project_path,
                    "projectVersion": launch.project_version,
                    "processId": launch.process_id,
                    "codeOptimization": match launch_code_optimization {
                        Some(UnityLaunchCodeOptimization::Debug) => "debug",
                        Some(UnityLaunchCodeOptimization::Release) => "release",
                        None => "default",
                    },
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
                    "semantic": last_semantic_sample,
                }),
            );
            last_log = Instant::now();
        }

        if !semantic_waiting && last_progress_at.elapsed() >= config.no_progress_timeout {
            sink.emit(
                "connection_stalled",
                json!({
                    "elapsedMs": started.elapsed().as_millis(),
                    "noProgressMs": last_progress_at.elapsed().as_millis(),
                    "recent": recent_samples,
                    "semantic": last_semantic_sample,
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

        if started.elapsed() >= connect_timeout {
            sink.emit(
                "connection_timeout",
                json!({
                    "elapsedMs": started.elapsed().as_millis(),
                    "recent": recent_samples,
                    "semantic": last_semantic_sample,
                }),
            );
            return Err(format!(
                "Unity connection timed out after {}ms",
                connect_timeout.as_millis()
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

fn connection_timeout_for_plugin_outcome(
    config: &CliDriverConfig,
    plugin_outcome: PluginPrepareOutcome,
) -> Duration {
    match plugin_outcome {
        PluginPrepareOutcome::Installed => config
            .connect_timeout
            .max(POST_PLUGIN_INSTALL_CONNECT_TIMEOUT),
        PluginPrepareOutcome::UpToDate => config.connect_timeout,
    }
}

fn semantic_state_is_reload_wait(state: &unity_bridge::SemanticState) -> bool {
    matches!(state.phase.as_str(), "starting" | "reloading")
        || state.safety.recommended_action == "wait_reload"
}

fn semantic_connection_wait_signature(state: &unity_bridge::SemanticState) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        state.phase,
        state.source,
        state.confidence,
        state.reload_phase.as_deref().unwrap_or(""),
        state.domain.phase,
        state.editor_mode.value,
        state.safety.can_call_unity_api,
        state.safety.can_modify_assets_safely,
        state.safety.recommended_action,
        state.detail.as_deref().unwrap_or("")
    )
}

fn semantic_connection_wait_sample(state: &unity_bridge::SemanticState) -> serde_json::Value {
    json!({
        "phase": &state.phase,
        "source": &state.source,
        "confidence": &state.confidence,
        "transient": state.transient,
        "needsUser": state.needs_user,
        "detail": &state.detail,
        "reloadPhase": &state.reload_phase,
        "editorMode": &state.editor_mode.value,
        "canCallUnityApi": state.safety.can_call_unity_api,
        "canModifyAssetsSafely": state.safety.can_modify_assets_safely,
        "recommendedAction": &state.safety.recommended_action,
        "process": &state.process.state,
        "processId": state.process.pid,
        "channel": &state.channel.control_pipe,
        "domain": &state.domain.phase,
        "mainThread": &state.main_thread.state,
    })
}

fn semantic_ready_requirement_satisfied(
    state: &unity_bridge::SemanticState,
    requirement: SemanticReadyRequirement,
) -> bool {
    match requirement {
        SemanticReadyRequirement::UnityApi => state.safety.can_call_unity_api,
        SemanticReadyRequirement::AssetModification => state.safety.can_modify_assets_safely,
    }
}

async fn wait_for_semantic_ready(
    project: &str,
    suite: CliDriverSuite,
    action: &'static str,
    requirement: SemanticReadyRequirement,
    timeout: Duration,
    poll_interval: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<unity_bridge::SemanticState, String> {
    unity_bridge::set_state_probe_enabled(true);
    unity_bridge::start_unity_semantic_state_observer(project);
    sink.emit(
        "semantic_wait_start",
        json!({
            "suite": suite.as_str(),
            "action": action,
            "requirement": requirement.as_str(),
            "timeoutMs": timeout.as_millis(),
        }),
    );

    let started = Instant::now();
    let mut last_signature = String::new();

    loop {
        if *cancel_rx.borrow() {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }

        let semantic = unity_bridge::unity_semantic_state(project).await;
        let ready = semantic_ready_requirement_satisfied(&semantic, requirement);
        let sample = semantic_connection_wait_sample(&semantic);
        let signature = format!(
            "{}|{}",
            ready,
            semantic_connection_wait_signature(&semantic)
        );
        if signature != last_signature || ready {
            last_signature = signature;
            sink.emit(
                "semantic_wait",
                json!({
                    "suite": suite.as_str(),
                    "action": action,
                    "requirement": requirement.as_str(),
                    "ready": ready,
                    "elapsedMs": started.elapsed().as_millis(),
                    "state": sample.clone(),
                }),
            );
        }
        if ready {
            return Ok(semantic);
        }

        if started.elapsed() >= timeout {
            sink.emit(
                "semantic_wait_timeout",
                json!({
                    "suite": suite.as_str(),
                    "action": action,
                    "requirement": requirement.as_str(),
                    "elapsedMs": started.elapsed().as_millis(),
                    "state": sample,
                }),
            );
            return Err(format!(
                "Unity semantic state was not ready for {action} within {}ms",
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

fn unity_reload_boundary_error(error: &str) -> bool {
    matches!(error, "managed_reloading" | "domain_reload_interrupted")
        || error.contains("managed_reloading")
        || error.contains("domain_reload_interrupted")
}

fn remaining_or_timeout(
    started: Instant,
    timeout: Duration,
    action: &'static str,
) -> Result<Duration, String> {
    timeout.checked_sub(started.elapsed()).ok_or_else(|| {
        format!(
            "{action} did not become ready within {}ms",
            timeout.as_millis()
        )
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
    let summary =
        match crate::unity_type_index_selftest::run(project, sample_mode, &mut on_progress).await {
            Ok(summary) => summary,
            Err(error) => {
                emit_suite_failure(sink, suite, &error);
                return Err(error);
            }
        };
    if summary.failed > 0 || !summary.warnings.is_empty() {
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
    }
    for warning in &summary.warnings {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": format!("WARN  type-index: {warning}"),
                "passed": summary.passed,
                "failed": summary.failed,
                "warning": true,
            }),
        );
    }
    if summary.failed > 0 {
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
            "warnings": summary.warnings,
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

fn should_stop_after_suite_error(error: &str) -> bool {
    error.contains(" timed out after ")
        || error.contains(" made no event progress for ")
        || error.contains(" event stream closed")
        || error.contains(" failed to start:")
        || error.contains(" task failed:")
}

fn format_suite_failures(suite_failures: &[String]) -> String {
    format!(
        "{} Unity integration test suite(s) failed: {}",
        suite_failures.len(),
        suite_failures.join("; ")
    )
}

/// One direct-IL operation from the same operation × visibility matrix used by
/// the hot-reload access probe. The target type is public, so member visibility
/// and nested-type visibility are measured without an internal container type
/// contaminating every cell.
struct NonPublicWrapperProbeCell {
    op: &'static str,
    visibility: &'static str,
    body: &'static str,
    expected: &'static str,
}

const NON_PUBLIC_WRAPPER_PROBE_TARGET: &str = "global::Locus.LocusExecuteAccessProbeTarget";

const NON_PUBLIC_WRAPPER_PROBE_CELLS: &[NonPublicWrapperProbeCell] = &[
    NonPublicWrapperProbeCell {
        op: "ldfld",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t._privInst);"#,
        expected: "7",
    },
    NonPublicWrapperProbeCell {
        op: "ldfld",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t._intInst);"#,
        expected: "11",
    },
    NonPublicWrapperProbeCell {
        op: "stfld",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); t._privInst = 42; print("__MARKER__:" + t.ReadPrivInst());"#,
        expected: "42",
    },
    NonPublicWrapperProbeCell {
        op: "stfld",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); t._intInst = 43; print("__MARKER__:" + t.ReadIntInst());"#,
        expected: "43",
    },
    NonPublicWrapperProbeCell {
        op: "ldsfld",
        visibility: "private",
        body: r#"__TARGET__.ResetStatics(); print("__MARKER__:" + __TARGET__._privStatic);"#,
        expected: "13",
    },
    NonPublicWrapperProbeCell {
        op: "ldsfld",
        visibility: "internal",
        body: r#"__TARGET__.ResetStatics(); print("__MARKER__:" + __TARGET__._intStatic);"#,
        expected: "17",
    },
    NonPublicWrapperProbeCell {
        op: "stsfld",
        visibility: "private",
        body: r#"__TARGET__.ResetStatics(); __TARGET__._privStatic = 47; print("__MARKER__:" + __TARGET__.ReadPrivStatic());"#,
        expected: "47",
    },
    NonPublicWrapperProbeCell {
        op: "stsfld",
        visibility: "internal",
        body: r#"__TARGET__.ResetStatics(); __TARGET__._intStatic = 53; print("__MARKER__:" + __TARGET__.ReadIntStatic());"#,
        expected: "53",
    },
    NonPublicWrapperProbeCell {
        op: "ldflda",
        visibility: "private",
        body: r#"int __LocusProbeLdfldaPrivate() { var t = __TARGET__.New(); ref int slot = ref t._privInst; slot = 59; return t.ReadPrivInst(); } print("__MARKER__:" + __LocusProbeLdfldaPrivate());"#,
        expected: "59",
    },
    NonPublicWrapperProbeCell {
        op: "ldflda",
        visibility: "internal",
        body: r#"int __LocusProbeLdfldaInternal() { var t = __TARGET__.New(); ref int slot = ref t._intInst; slot = 61; return t.ReadIntInst(); } print("__MARKER__:" + __LocusProbeLdfldaInternal());"#,
        expected: "61",
    },
    NonPublicWrapperProbeCell {
        op: "ldsflda",
        visibility: "private",
        body: r#"int __LocusProbeLdsfldaPrivate() { __TARGET__.ResetStatics(); ref int slot = ref __TARGET__._privStatic; slot = 67; return __TARGET__.ReadPrivStatic(); } print("__MARKER__:" + __LocusProbeLdsfldaPrivate());"#,
        expected: "67",
    },
    NonPublicWrapperProbeCell {
        op: "ldsflda",
        visibility: "internal",
        body: r#"int __LocusProbeLdsfldaInternal() { __TARGET__.ResetStatics(); ref int slot = ref __TARGET__._intStatic; slot = 71; return __TARGET__.ReadIntStatic(); } print("__MARKER__:" + __LocusProbeLdsfldaInternal());"#,
        expected: "71",
    },
    NonPublicWrapperProbeCell {
        op: "call",
        visibility: "private",
        body: r#"print("__MARKER__:" + __TARGET__.PrivStatic(3));"#,
        expected: "16",
    },
    NonPublicWrapperProbeCell {
        op: "call",
        visibility: "internal",
        body: r#"print("__MARKER__:" + __TARGET__.IntStatic(3));"#,
        expected: "22",
    },
    NonPublicWrapperProbeCell {
        op: "callvirt",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.PrivMethod(3));"#,
        expected: "7",
    },
    NonPublicWrapperProbeCell {
        op: "callvirt",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.IntMethod(3));"#,
        expected: "10",
    },
    NonPublicWrapperProbeCell {
        op: "newobj",
        visibility: "private",
        body: r#"var t = new __TARGET__(9); print("__MARKER__:" + t.ReadPrivInst());"#,
        expected: "9",
    },
    NonPublicWrapperProbeCell {
        op: "newobj",
        visibility: "internal",
        body: r#"var t = new __TARGET__(); print("__MARKER__:" + t.ReadPrivInst());"#,
        expected: "7",
    },
    NonPublicWrapperProbeCell {
        op: "ldftn",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); System.Func<int, int> f = t.PrivMethod; print("__MARKER__:" + f(5));"#,
        expected: "11",
    },
    NonPublicWrapperProbeCell {
        op: "ldftn",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); System.Func<int, int> f = t.IntMethod; print("__MARKER__:" + f(5));"#,
        expected: "16",
    },
    NonPublicWrapperProbeCell {
        op: "property_get",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.PrivProperty);"#,
        expected: "23",
    },
    NonPublicWrapperProbeCell {
        op: "property_get",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.IntProperty);"#,
        expected: "29",
    },
    NonPublicWrapperProbeCell {
        op: "property_set",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); t.PrivProperty = 31; print("__MARKER__:" + t.ReadPrivProperty());"#,
        expected: "31",
    },
    NonPublicWrapperProbeCell {
        op: "property_set",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); t.IntProperty = 37; print("__MARKER__:" + t.ReadIntProperty());"#,
        expected: "37",
    },
    NonPublicWrapperProbeCell {
        op: "event_add",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); System.Action h = delegate { }; t.PrivEvent += h; print("__MARKER__:" + t.ReadPrivEventSubscribers());"#,
        expected: "1",
    },
    NonPublicWrapperProbeCell {
        op: "event_add",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); System.Action h = delegate { }; t.IntEvent += h; print("__MARKER__:" + t.ReadIntEventSubscribers());"#,
        expected: "1",
    },
    NonPublicWrapperProbeCell {
        op: "generic_call",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.PrivGeneric<int>(41));"#,
        expected: "41",
    },
    NonPublicWrapperProbeCell {
        op: "generic_call",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); print("__MARKER__:" + t.IntGeneric<int>(43));"#,
        expected: "43",
    },
    NonPublicWrapperProbeCell {
        op: "ref_call",
        visibility: "private",
        body: r#"var t = __TARGET__.New(); int value = 7; t.PrivRef(ref value); print("__MARKER__:" + value);"#,
        expected: "12",
    },
    NonPublicWrapperProbeCell {
        op: "ref_call",
        visibility: "internal",
        body: r#"var t = __TARGET__.New(); int value = 7; t.IntRef(ref value); print("__MARKER__:" + value);"#,
        expected: "14",
    },
    NonPublicWrapperProbeCell {
        op: "castclass",
        visibility: "private",
        body: r#"object value = null; var typed = (__TARGET__.PrivNested)value; print("__MARKER__:" + (typed == null));"#,
        expected: "True",
    },
    NonPublicWrapperProbeCell {
        op: "castclass",
        visibility: "internal",
        body: r#"object value = null; var typed = (__TARGET__.IntNested)value; print("__MARKER__:" + (typed == null));"#,
        expected: "True",
    },
    NonPublicWrapperProbeCell {
        op: "ldtoken",
        visibility: "private",
        body: r#"print("__MARKER__:" + typeof(__TARGET__.PrivNested).Name);"#,
        expected: "PrivNested",
    },
    NonPublicWrapperProbeCell {
        op: "ldtoken",
        visibility: "internal",
        body: r#"print("__MARKER__:" + typeof(__TARGET__.IntNested).Name);"#,
        expected: "IntNested",
    },
];

#[derive(Default)]
struct NonPublicWrapperProbeSummary {
    direct: u32,
    blocked: u32,
    infrastructure_failed: u32,
    cells: BTreeMap<String, bool>,
}

impl NonPublicWrapperProbeSummary {
    fn complete_direct(&self, expected_cells: usize) -> bool {
        self.infrastructure_failed == 0
            && self.blocked == 0
            && self.direct as usize == expected_cells
    }
}

fn non_public_probe_key(cell: &NonPublicWrapperProbeCell) -> String {
    format!("{}_{}", cell.op, cell.visibility)
}

fn non_public_probe_code(cell: &NonPublicWrapperProbeCell, marker: &str) -> String {
    cell.body
        .replace("__TARGET__", NON_PUBLIC_WRAPPER_PROBE_TARGET)
        .replace("__MARKER__", marker)
}

fn non_public_probe_expected_marker(
    marker_prefix: &str,
    cell: &NonPublicWrapperProbeCell,
) -> String {
    format!("{marker_prefix}:{}", cell.expected)
}

fn non_public_probe_compile_control_rejected(error: &str) -> bool {
    error.contains("_privInst")
        && ["CS0122", "CS1061", "CS0117", "CS1729"]
            .iter()
            .any(|code| error.contains(code))
}

fn non_public_probe_compile_failed(error: &str) -> bool {
    error.contains("compilation failed:")
        || error.contains("CS0122")
        || error.contains("CS0050")
        || error.contains("CS0051")
        || error.contains("skip_verification")
        || error.contains("DeclSecurity")
        || error.contains("mode mismatch")
        || error.contains("requires the sidecar compiler")
        || error.contains("requires a Unity plugin with")
}

async fn query_effective_unity_inlining(project: &str) -> Result<(bool, String), String> {
    let resp = unity_bridge::send_message_with_timeout(
        project,
        "hot_reload_inlining_active",
        "",
        Duration::from_secs(15),
    )
    .await?;
    if !resp.ok {
        return Err(resp
            .error
            .unwrap_or_else(|| "inlining probe failed".to_string()));
    }
    let message = resp.message.unwrap_or_default();
    let parsed: serde_json::Value = serde_json::from_str(&message)
        .map_err(|error| format!("inlining probe response parse failed: {error}"))?;
    let active = parsed
        .get("inlining_active")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let setting = parsed
        .get("code_optimization")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let detail = parsed
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    Ok((active, format!("setting={setting} {detail}")))
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
                format!(
                    "expected '{expect}' in output, got '{}'",
                    clip(&output, 160)
                ),
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
                        format!(
                            "api progress revision regressed {}x",
                            observed.api_regressions
                        ),
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
            Err(error) => self.fail(
                "E7 progress",
                format!("execute error: {}", clip(&error, 200)),
            ),
        }
    }

    async fn check_thread_and_tick_discovery(&mut self, project: &str) {
        let code = r#"bool mainBefore = ctx.IsMainThread;
await ctx.SwitchToThreadPool();
bool pool = !ctx.IsMainThread && ctx.Thread.IsThreadPoolThread;
await ctx.SwitchToMainThread();
var ticks = ctx.ListTickSystems();
print("E7T:" + mainBefore + ":" + pool + ":" + ctx.IsMainThread + ":" + ticks.Count);"#;
        match execute_capture(project, code).await {
            Ok(output) => {
                let marker = output
                    .lines()
                    .find(|line| line.starts_with("E7T:"))
                    .unwrap_or_default();
                let count = marker
                    .rsplit(':')
                    .next()
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or_default();
                if marker.starts_with("E7T:True:True:True:") && count > 10 {
                    self.pass(
                        "E7T thread/tick-discovery",
                        format!("main -> pool -> main; discovered {count} PlayerLoop nodes"),
                    );
                } else {
                    self.fail(
                        "E7T thread/tick-discovery",
                        format!("unexpected output '{}'", clip(&output, 180)),
                    );
                }
            }
            Err(error) => self.fail(
                "E7T thread/tick-discovery",
                format!("execute error: {}", clip(&error, 200)),
            ),
        }
    }

    async fn check_pending_await_diagnostics(&mut self, project: &str) {
        let latest = Arc::new(std::sync::Mutex::new(None));
        let observer = Arc::clone(&latest);
        let code = r#"int marker = 42;
await ctx.WaitSeconds(0.8f);
print("E7W:" + marker);"#;
        let result =
            unity_bridge::unity_execute_code_with_progress(project, code, move |snapshot| {
                if snapshot.source == "await" {
                    if let Ok(mut value) = observer.lock() {
                        *value = Some(snapshot);
                    }
                }
            })
            .await;
        let observed = latest.lock().ok().and_then(|value| value.clone());
        match (result, observed) {
            (Ok(output), Some(snapshot))
                if output.contains("E7W:42")
                    && snapshot.wait_kind == "editor_time"
                    && snapshot.source_line == 2
                    && snapshot.source_text.contains("ctx.WaitSeconds(0.8f)")
                    && snapshot.wait_target.contains("seconds") =>
            {
                self.pass(
                    "E7W await-diagnostics",
                    format!(
                        "line={} waited={}ms source='{}'",
                        snapshot.source_line, snapshot.waited_ms, snapshot.source_text
                    ),
                );
            }
            (Ok(output), Some(snapshot)) => self.fail(
                "E7W await-diagnostics",
                format!(
                    "output='{}' kind={} line={} source='{}' target='{}'",
                    clip(&output, 80),
                    snapshot.wait_kind,
                    snapshot.source_line,
                    snapshot.source_text,
                    snapshot.wait_target
                ),
            ),
            (Ok(output), None) => self.fail(
                "E7W await-diagnostics",
                format!("no await snapshot; output='{}'", clip(&output, 100)),
            ),
            (Err(error), _) => self.fail(
                "E7W await-diagnostics",
                format!("execute error: {}", clip(&error, 200)),
            ),
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

    /// Two frame-spanning executes compile/bootstrap under the operation lock,
    /// then overlap while awaiting Unity and keep request-scoped output.
    async fn check_concurrency(&mut self, project: &str) {
        let code_a = r#"await ctx.WaitSeconds(1.5f); print("E9A:ok");"#;
        let code_b = r#"await ctx.WaitSeconds(1.5f); print("E9B:ok");"#;
        let started = Instant::now();
        let (ra, rb) = tokio::join!(
            execute_capture(project, code_a),
            execute_capture(project, code_b)
        );
        let elapsed = started.elapsed();
        let a_ok = matches!(&ra, Ok(output) if output.contains("E9A:ok"));
        let b_ok = matches!(&rb, Ok(output) if output.contains("E9B:ok"));
        if a_ok && b_ok && elapsed < Duration::from_millis(2800) {
            self.pass(
                "E9 concurrent-await",
                format!(
                    "two 1.5s waits completed independently in {}ms",
                    elapsed.as_millis()
                ),
            );
        } else {
            self.fail(
                "E9 concurrent-await",
                format!(
                    "elapsed={}ms, A={}, B={}",
                    elapsed.as_millis(),
                    describe_result(&ra, "E9A:ok"),
                    describe_result(&rb, "E9B:ok")
                ),
            );
        }
    }

    async fn check_player_loop_debugger(&mut self, project: &str) {
        if let Err(error) =
            unity_bridge::set_editor_status(project, unity_bridge::UNITY_EDITOR_STATUS_PLAYING)
                .await
        {
            self.fail(
                "E9D debugger",
                format!("could not enter Play Mode: {}", clip(&error, 180)),
            );
            return;
        }

        let tick_result = execute_capture(
            project,
            r#"var update = ctx.FindTickSystem(typeof(UnityEngine.PlayerLoop.Update.ScriptRunBehaviourUpdate).FullName);
var stamp = await ctx.WaitAfter(update);
print("E9D:tick:" + stamp.Boundary + ":" + stamp.FrameCount + ":" + ctx.IsMainThread);"#,
        )
        .await;
        let break_result = execute_capture(
            project,
            r#"await ctx.BreakWhen(UnityLoopPoint.AfterUpdate, () => true, label: "e9d", condition: "true");
print("E9D:unreachable");"#,
        )
        .await;
        let (_, paused_status, _) = unity_bridge::query_unity_status(project).await;
        let step_result = if paused_status == unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED {
            execute_capture(
                project,
                r#"int before = Time.frameCount; var stamp = await ctx.StepFrame(); print("E9D:step:" + before + ":" + Time.frameCount + ":" + EditorApplication.isPaused);"#,
            )
            .await
        } else {
            Err(format!("expected playing_paused, got {paused_status}"))
        };
        let resume_result = execute_capture(
            project,
            r#"await ctx.ResumeGame(); print("E9D:resume:" + EditorApplication.isPaused);"#,
        )
        .await;

        let run_states = json!({
            "request_editor_status": "playing",
            "initial_state": "tick",
            "states": [{
                "name": "tick",
                "start": "ctx.SetTickPoint(UnityLoopPoint.AfterUpdate);",
                "update": "if (ctx.TotalFrames >= 3) { print(\"E9D:run-states-tick\"); ctx.Done(); }",
            }],
        });
        let run_states_result = unity_bridge::unity_run_states(project, &run_states).await;

        let restore =
            unity_bridge::set_editor_status(project, unity_bridge::UNITY_EDITOR_STATUS_EDITING)
                .await;

        let tick_ok = matches!(&tick_result, Ok(output) if output.contains("E9D:tick:After:") && output.contains(":True"));
        let break_ok = matches!(&break_result, Ok(output) if output.contains("status: breakpoint") && output.contains("label: e9d") && !output.contains("E9D:unreachable"));
        let step_ok = matches!(&step_result, Ok(output) if output.contains("E9D:step:") && output.contains(":True"));
        let resume_ok = matches!(&resume_result, Ok(output) if output.contains("E9D:resume:False"));
        let run_states_ok = matches!(&run_states_result, Ok(output) if output.contains("E9D:run-states-tick") && output.contains("status: ok"));
        if tick_ok && break_ok && step_ok && resume_ok && run_states_ok && restore.is_ok() {
            self.pass(
                "E9D debugger",
                "dynamic tick wait, breakpoint termination, paused step, resume and run-states tick passed",
            );
        } else {
            self.fail(
                "E9D debugger",
                format!(
                    "tick={} break={} paused={} step={} resume={} run_states={} restore={}",
                    describe_result(&tick_result, "E9D:tick:"),
                    describe_result(&break_result, "status: breakpoint"),
                    paused_status,
                    describe_result(&step_result, "E9D:step:"),
                    describe_result(&resume_result, "E9D:resume:False"),
                    describe_result(&run_states_result, "E9D:run-states-tick"),
                    restore
                        .as_ref()
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|error| clip(error, 100)),
                ),
            );
        }
    }

    /// The legacy in-Unity compile path (`execute_code`) — exercised by turning
    /// the sidecar off for a single round trip — still compiles and executes.
    async fn check_legacy_compile(&mut self, project: &str) {
        let was_enabled = crate::csharp_compile::is_enabled();
        crate::csharp_compile::set_enabled(false).await;
        let result = execute_capture(
            project,
            r#"var values = new[] { 41 }; ref int value = ref values[0]; value++; print("E12:" + value);"#,
        )
        .await;
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

    fn record_non_public_wrapper_probe(
        &mut self,
        surface: &str,
        key: &str,
        marker: &str,
        result: Result<String, String>,
        summary: &mut NonPublicWrapperProbeSummary,
    ) {
        match result {
            Ok(output) if output.contains(marker) => {
                summary.direct += 1;
                summary.cells.insert(key.to_string(), true);
                self.line(format!(
                    "PROBE {surface} {key}: DIRECT ({})",
                    clip(&output, 120)
                ));
            }
            Ok(output) => {
                summary.infrastructure_failed += 1;
                summary.cells.insert(key.to_string(), false);
                self.line(format!(
                    "FAIL  {surface} {key}: wrapper completed without marker '{}' ({})",
                    marker,
                    clip(&output, 180)
                ));
            }
            Err(error) if non_public_probe_compile_failed(&error) => {
                summary.infrastructure_failed += 1;
                summary.cells.insert(key.to_string(), false);
                self.line(format!(
                    "FAIL  {surface} {key}: probe compilation/infrastructure failed ({})",
                    clip(&error, 220)
                ));
            }
            Err(error) => {
                summary.blocked += 1;
                summary.cells.insert(key.to_string(), false);
                self.line(format!(
                    "PROBE {surface} {key}: BLOCKED ({})",
                    clip(&error, 180)
                ));
            }
        }
    }

    async fn check_non_public_compile_controls(&mut self, project: &str) {
        let execute_control = format!(
            "var t = {target}.New(); print(t._privInst);",
            target = NON_PUBLIC_WRAPPER_PROBE_TARGET
        );
        match execute_capture(project, &execute_control).await {
            Err(error) if non_public_probe_compile_control_rejected(&error) => self.pass(
                "E13 execute access control",
                format!(
                    "normal unity_execute compilation rejected direct private access ({})",
                    clip(&error, 100)
                ),
            ),
            Err(error) => self.fail(
                "E13 execute access control",
                format!("unexpected rejection shape: '{}'", clip(&error, 180)),
            ),
            Ok(output) => self.fail(
                "E13 execute access control",
                format!(
                    "normal compilation unexpectedly executed: '{}'",
                    clip(&output, 140)
                ),
            ),
        }

        let run_states_control = json!({
            "request_editor_status": "editing",
            "initial_state": "probe",
            "states": [{
                "name": "probe",
                "update": format!(
                    "var t = {target}.New(); print(t._privInst); ctx.Done(\"control\");",
                    target = NON_PUBLIC_WRAPPER_PROBE_TARGET
                ),
            }],
        });
        match unity_bridge::unity_run_states(project, &run_states_control).await {
            Err(error) if non_public_probe_compile_control_rejected(&error) => self.pass(
                "E13 run-states access control",
                format!(
                    "normal unity_run_states compilation rejected direct private access ({})",
                    clip(&error, 100)
                ),
            ),
            Err(error) => self.fail(
                "E13 run-states access control",
                format!("unexpected rejection shape: '{}'", clip(&error, 180)),
            ),
            Ok(output) => self.fail(
                "E13 run-states access control",
                format!(
                    "normal compilation unexpectedly executed: '{}'",
                    clip(&output, 140)
                ),
            ),
        }
    }

    async fn report_low_level_non_public_probe(
        &mut self,
        project: &str,
        mode: crate::csharp_compile::NonPublicAccessProbeMode,
        config: &CliDriverConfig,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> Result<BTreeMap<String, bool>, String> {
        let check_name = format!("E14 low-level {}", mode.as_str());
        let mut attempt = 0u32;
        let value = loop {
            attempt += 1;
            match crate::unity_hotreload::coordinator::access_probe_run_with_mode(project, mode)
                .await
            {
                Ok(value) => break value,
                Err(error) if unity_reload_boundary_error(&error) && attempt < 4 => {
                    self.line(format!(
                        "PROBE low-level [{}] attempt {} crossed a domain reload; waiting and retrying",
                        mode.as_str(),
                        attempt
                    ));
                    wait_for_semantic_ready(
                        project,
                        self.suite,
                        "access-probe reload recovery",
                        SemanticReadyRequirement::UnityApi,
                        recompile_wait(config),
                        config.poll_interval,
                        self.sink,
                        cancel_rx,
                    )
                    .await?;
                }
                Err(error) => {
                    self.fail(
                        &check_name,
                        format!(
                            "probe failed after {attempt} attempt(s): {}",
                            clip(&error, 220)
                        ),
                    );
                    return Ok(BTreeMap::new());
                }
            }
        };

        if mode.emits_skip_verification()
            && value
                .get("skipVerificationDeclSecurity")
                .and_then(serde_json::Value::as_bool)
                != Some(true)
        {
            self.fail(
                &check_name,
                "compile server did not confirm SkipVerification DeclSecurity metadata",
            );
            return Ok(BTreeMap::new());
        }

        let cells = value
            .get("caps")
            .and_then(|caps| caps.get("cells"))
            .and_then(serde_json::Value::as_object);
        let raw_cells = value
            .get("matrix")
            .and_then(|matrix| matrix.get("cells"))
            .and_then(serde_json::Value::as_array);
        let mut measured = BTreeMap::new();
        if let Some(capability_cells) = cells {
            for (key, capability) in capability_cells {
                let direct = capability.as_bool().unwrap_or(false);
                measured.insert(key.clone(), direct);
                let raw = raw_cells.and_then(|cells| {
                    cells.iter().find(|cell| {
                        let op = cell
                            .get("op")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let visibility = cell
                            .get("visibility")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        format!("{op}_{visibility}") == key.as_str()
                    })
                });
                let detail = raw
                    .map(|cell| {
                        let expected = cell
                            .get("expected")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or_default();
                        let actual = cell
                            .get("actual")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or_default();
                        let error = cell
                            .get("error")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        if error.is_empty() {
                            format!("expected={expected} actual={actual}")
                        } else {
                            clip(error, 130)
                        }
                    })
                    .unwrap_or_else(|| "raw result missing".to_string());
                self.line(format!(
                    "PROBE low-level [{}] {key}: {} ({detail})",
                    mode.as_str(),
                    if direct { "DIRECT" } else { "BLOCKED" },
                ));
            }
        }

        let caps = value.get("caps");
        let primitive = |name: &str| {
            caps.and_then(|caps| caps.get(name))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        self.line(format!(
            "PROBE low-level [{}] fallbacks: create_delegate={} dynamic_method={} byref_dynamic_method={}",
            mode.as_str(),
            primitive("createDelegateNonPublic"),
            primitive("dynamicMethodSkipVisibility"),
            primitive("dynamicMethodByrefReturn"),
        ));

        if measured.len() == NON_PUBLIC_WRAPPER_PROBE_CELLS.len() {
            let direct = measured.values().filter(|value| **value).count();
            self.pass(
                &check_name,
                format!(
                    "executed {}/{} direct operation cells with return-value validation",
                    direct,
                    measured.len()
                ),
            );
        } else {
            self.fail(
                &check_name,
                format!(
                    "expected {} cells, received {}",
                    NON_PUBLIC_WRAPPER_PROBE_CELLS.len(),
                    measured.len()
                ),
            );
        }
        Ok(measured)
    }

    async fn probe_unity_execute_non_public(
        &mut self,
        project: &str,
        mode: crate::csharp_compile::NonPublicAccessProbeMode,
        cancel_rx: &watch::Receiver<bool>,
    ) -> Result<NonPublicWrapperProbeSummary, String> {
        let mut summary = NonPublicWrapperProbeSummary::default();
        let surface = format!("unity_execute[{}]", mode.as_str());
        for cell in NON_PUBLIC_WRAPPER_PROBE_CELLS {
            if run_cancelled(cancel_rx) {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
            let key = non_public_probe_key(cell);
            let marker_prefix = format!("NP_EXEC_{}_{key}_OK", mode.as_str());
            let expected_marker = non_public_probe_expected_marker(&marker_prefix, cell);
            let code = non_public_probe_code(cell, &marker_prefix);
            let result =
                unity_bridge::unity_execute_code_with_access_probe(project, &code, mode).await;
            self.record_non_public_wrapper_probe(
                &surface,
                &key,
                &expected_marker,
                result,
                &mut summary,
            );
        }

        let marker_prefix = format!("NP_EXEC_{}_POST_AWAIT_OK", mode.as_str());
        let expected_marker = format!("{marker_prefix}:7");
        let post_await = format!(
            "await ctx.WaitFrames(1); var t = {target}.New(); print(\"{marker_prefix}:\" + t._privInst);",
            target = NON_PUBLIC_WRAPPER_PROBE_TARGET
        );
        let result =
            unity_bridge::unity_execute_code_with_access_probe(project, &post_await, mode).await;
        self.record_non_public_wrapper_probe(
            &surface,
            "post_await_ldfld_private",
            &expected_marker,
            result,
            &mut summary,
        );

        let check_name = format!("E15 unity_execute {}", mode.as_str());
        if summary.infrastructure_failed == 0 {
            self.pass(
                &check_name,
                format!(
                    "direct={} blocked={} across {} operation and post-await cells",
                    summary.direct,
                    summary.blocked,
                    NON_PUBLIC_WRAPPER_PROBE_CELLS.len()
                ),
            );
        } else {
            self.fail(
                &check_name,
                format!(
                    "{} probe cell(s) failed before a runtime capability result",
                    summary.infrastructure_failed
                ),
            );
        }
        Ok(summary)
    }

    async fn probe_unity_run_states_non_public(
        &mut self,
        project: &str,
        mode: crate::csharp_compile::NonPublicAccessProbeMode,
        cancel_rx: &watch::Receiver<bool>,
    ) -> Result<NonPublicWrapperProbeSummary, String> {
        let mut summary = NonPublicWrapperProbeSummary::default();
        let surface = format!("unity_run_states[{}]", mode.as_str());
        for cell in NON_PUBLIC_WRAPPER_PROBE_CELLS {
            if run_cancelled(cancel_rx) {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
            let key = non_public_probe_key(cell);
            let marker_prefix = format!("NP_RUN_{}_{key}_OK", mode.as_str());
            let expected_marker = non_public_probe_expected_marker(&marker_prefix, cell);
            let update = format!(
                "{} ctx.Done(\"non-public-probe\");",
                non_public_probe_code(cell, &marker_prefix)
            );
            let request = json!({
                "request_editor_status": "editing",
                "initial_state": "probe",
                "states": [{ "name": "probe", "update": update }],
            });
            let result =
                unity_bridge::unity_run_states_with_access_probe(project, &request, mode).await;
            self.record_non_public_wrapper_probe(
                &surface,
                &key,
                &expected_marker,
                result,
                &mut summary,
            );
        }

        for visibility in ["private", "internal"] {
            let key = format!("build_ldfld_{visibility}");
            let marker_prefix = format!("NP_RUN_{}_{key}_OK", mode.as_str());
            let member = if visibility == "private" {
                "_privInst"
            } else {
                "_intInst"
            };
            let expected = if visibility == "private" { "7" } else { "11" };
            let expected_marker = format!("{marker_prefix}:{expected}");
            let variables = format!(
                "var buildTarget = {target}.New(); var buildValue = buildTarget.{member};",
                target = NON_PUBLIC_WRAPPER_PROBE_TARGET
            );
            let update =
                format!("print(\"{marker_prefix}:\" + buildValue); ctx.Done(\"build-probe\");");
            let request = json!({
                "request_editor_status": "editing",
                "initial_state": "probe",
                "states": [{
                    "name": "probe",
                    "variables": variables,
                    "update": update,
                }],
            });
            let result =
                unity_bridge::unity_run_states_with_access_probe(project, &request, mode).await;
            self.record_non_public_wrapper_probe(
                &surface,
                &key,
                &expected_marker,
                result,
                &mut summary,
            );
        }

        let check_name = format!("E16 unity_run_states {}", mode.as_str());
        if summary.infrastructure_failed == 0 {
            self.pass(
                &check_name,
                format!(
                    "direct={} blocked={} across Build and handler contexts",
                    summary.direct, summary.blocked
                ),
            );
        } else {
            self.fail(
                &check_name,
                format!(
                    "{} probe cell(s) failed before a runtime capability result",
                    summary.infrastructure_failed
                ),
            );
        }
        Ok(summary)
    }

    fn report_non_public_probe_comparison(
        &self,
        mode: crate::csharp_compile::NonPublicAccessProbeMode,
        low_level: &BTreeMap<String, bool>,
        execute: &NonPublicWrapperProbeSummary,
        run_states: &NonPublicWrapperProbeSummary,
    ) {
        for cell in NON_PUBLIC_WRAPPER_PROBE_CELLS {
            let key = non_public_probe_key(cell);
            let low = low_level.get(&key).copied();
            let execute_value = execute.cells.get(&key).copied();
            let run_states_value = run_states.cells.get(&key).copied();
            if low != execute_value || execute_value != run_states_value {
                self.line(format!(
                    "PROBE comparison [{}] {key}: low-level={low:?} unity_execute={execute_value:?} unity_run_states={run_states_value:?}",
                    mode.as_str()
                ));
            }
        }
    }

    fn report_non_public_strategy_verdict(
        &mut self,
        low_level: &BTreeMap<
            crate::csharp_compile::NonPublicAccessProbeMode,
            BTreeMap<String, bool>,
        >,
        execute: &BTreeMap<
            crate::csharp_compile::NonPublicAccessProbeMode,
            NonPublicWrapperProbeSummary,
        >,
        run_states: &BTreeMap<
            crate::csharp_compile::NonPublicAccessProbeMode,
            NonPublicWrapperProbeSummary,
        >,
    ) {
        let low_expected = NON_PUBLIC_WRAPPER_PROBE_CELLS.len();
        let execute_expected = low_expected + 1;
        let run_states_expected = low_expected + 2;
        let mut selected = None;
        let mut indeterminate = false;

        for mode in crate::csharp_compile::NonPublicAccessProbeMode::ALL {
            let low = low_level.get(&mode);
            let execute_summary = execute.get(&mode);
            let run_states_summary = run_states.get(&mode);
            let low_direct = low
                .map(|cells| cells.values().filter(|value| **value).count())
                .unwrap_or_default();
            let execute_direct = execute_summary
                .map(|summary| summary.direct as usize)
                .unwrap_or_default();
            let run_states_direct = run_states_summary
                .map(|summary| summary.direct as usize)
                .unwrap_or_default();
            let complete = low
                .map(|cells| cells.len() == low_expected && low_direct == low_expected)
                .unwrap_or(false)
                && execute_summary
                    .map(|summary| summary.complete_direct(execute_expected))
                    .unwrap_or(false)
                && run_states_summary
                    .map(|summary| summary.complete_direct(run_states_expected))
                    .unwrap_or(false);
            let mode_indeterminate = low.map(|cells| cells.len() != low_expected).unwrap_or(true)
                || execute_summary
                    .map(|summary| summary.infrastructure_failed > 0)
                    .unwrap_or(true)
                || run_states_summary
                    .map(|summary| summary.infrastructure_failed > 0)
                    .unwrap_or(true);
            indeterminate |= mode_indeterminate;
            self.line(format!(
                "PROBE strategy [{}]: low-level={low_direct}/{low_expected} execute={execute_direct}/{execute_expected} run_states={run_states_direct}/{run_states_expected} complete={complete} indeterminate={mode_indeterminate}",
                mode.as_str()
            ));
            if selected.is_none() && complete {
                selected = Some(mode);
            }
        }

        match selected {
            Some(mode) => self.pass(
                "E17 non-public strategy verdict",
                format!(
                    "selected={} for direct IL across low-level, async, Build, and handler contexts",
                    mode.as_str()
                ),
            ),
            None if indeterminate => self.fail(
                "E17 non-public strategy verdict",
                "selected=indeterminate; at least one strategy had a compile or probe-infrastructure failure",
            ),
            None => self.pass(
                "E17 non-public strategy verdict",
                "selected=native_access_check_hook; no assembly-metadata policy covered every direct-IL cell",
            ),
        }
    }

    async fn check_non_public_access_probes(
        &mut self,
        project: &str,
        config: &CliDriverConfig,
        cancel_rx: &mut watch::Receiver<bool>,
    ) -> Result<(), String> {
        let (connected, original) =
            crate::unity_hotreload::coordinator::detect_code_optimization(project).await;
        let Some(original) = original.filter(|_| connected) else {
            self.fail(
                "E13 Debug-effective precondition",
                "could not read Unity Code Optimization before running access probes",
            );
            return Ok(());
        };

        let timeout = recompile_wait(config);
        if let Err(error) = ensure_code_optimization(
            project,
            self.suite,
            "debug",
            timeout,
            config.poll_interval,
            self.sink,
            cancel_rx,
            false,
        )
        .await
        {
            if error == UNITY_INTEGRATION_TEST_CANCELLED {
                return Err(error);
            }
            self.fail(
                "E13 Debug-effective precondition",
                format!("could not switch to Debug: {}", clip(&error, 180)),
            );
            return Ok(());
        }

        wait_for_semantic_ready(
            project,
            self.suite,
            "access probes after Debug switch",
            SemanticReadyRequirement::UnityApi,
            timeout,
            config.poll_interval,
            self.sink,
            cancel_rx,
        )
        .await?;

        match query_effective_unity_inlining(project).await {
            Ok((false, detail)) => self.pass(
                "E13 Debug-effective precondition",
                format!("runtime inlining canary is inactive ({detail})"),
            ),
            Ok((true, detail)) => self.fail(
                "E13 Debug-effective precondition",
                format!("runtime still reports active inlining ({detail})"),
            ),
            Err(error) => self.fail(
                "E13 Debug-effective precondition",
                format!("inlining canary failed: {}", clip(&error, 180)),
            ),
        }

        self.check_non_public_compile_controls(project).await;
        let mut low_by_mode = BTreeMap::new();
        let mut execute_by_mode = BTreeMap::new();
        let mut run_states_by_mode = BTreeMap::new();
        for mode in crate::csharp_compile::NonPublicAccessProbeMode::ALL {
            let low_level = self
                .report_low_level_non_public_probe(project, mode, config, cancel_rx)
                .await?;
            let execute = self
                .probe_unity_execute_non_public(project, mode, cancel_rx)
                .await?;
            let run_states = self
                .probe_unity_run_states_non_public(project, mode, cancel_rx)
                .await?;
            self.report_non_public_probe_comparison(mode, &low_level, &execute, &run_states);
            low_by_mode.insert(mode, low_level);
            execute_by_mode.insert(mode, execute);
            run_states_by_mode.insert(mode, run_states);
        }
        self.report_non_public_strategy_verdict(
            &low_by_mode,
            &execute_by_mode,
            &run_states_by_mode,
        );

        if original == "release" {
            match ensure_code_optimization(
                project,
                self.suite,
                "release",
                timeout,
                config.poll_interval,
                self.sink,
                cancel_rx,
                false,
            )
            .await
            {
                Ok(_) => self.line(
                    "E13 access probe: restored Unity Code Optimization to release".to_string(),
                ),
                Err(error) if error == UNITY_INTEGRATION_TEST_CANCELLED => return Err(error),
                Err(error) => self.fail(
                    "E13 Code Optimization restore",
                    format!("restore failed: {}", clip(&error, 180)),
                ),
            }
        }
        Ok(())
    }

    /// Full recompile: add a brand-new type to the project, ask Unity to
    /// recompile, confirm a fresh execute resolves it through the domain reload,
    /// then remove the script and recompile back to the original state.
    async fn check_recompile(&mut self, project: &str, config: &CliDriverConfig) {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let type_name = format!("LocusExecuteSelfTestSubject_{}", &token[..8]);
        let rel_dir = "Assets/LocusExecuteSelfTest";
        let dir = Path::new(project)
            .join("Assets")
            .join("LocusExecuteSelfTest");
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
            self.line(
                "E10 recompile: project restored (type removed, recompiled back)".to_string(),
            );
        } else {
            self.line(
                "E10 recompile: WARNING test script removed but project may still be recompiling"
                    .to_string(),
            );
        }
    }
}

fn parse_active_edit_session_count(message: &str) -> Result<usize, String> {
    message
        .trim()
        .strip_prefix("active_edit_sessions:")
        .ok_or_else(|| format!("unexpected edit-session response: {}", clip(message, 120)))?
        .parse::<usize>()
        .map_err(|error| {
            format!(
                "invalid edit-session count '{}': {error}",
                clip(message, 120)
            )
        })
}

async fn probe_asset_guid(project: &str, asset_path: &str) -> Result<Option<String>, String> {
    let code = format!(
        r#"string guid = AssetDatabase.AssetPathToGUID("{asset_path}"); print("LPR_GUID:" + (string.IsNullOrEmpty(guid) ? "missing" : "present:" + guid));"#
    );
    let output = execute_capture(project, &code).await?;
    if output.contains("LPR_GUID:missing") {
        return Ok(None);
    }
    let Some(index) = output.find("LPR_GUID:present:") else {
        return Err(format!(
            "asset GUID probe returned no marker: {}",
            clip(&output, 180)
        ));
    };
    let guid = output[index + "LPR_GUID:present:".len()..]
        .split(|ch: char| ch.is_whitespace() || ch == ']' || ch == '<')
        .next()
        .unwrap_or_default()
        .trim_matches(['\"', '\''])
        .to_string();
    if guid.is_empty() {
        Err(format!(
            "asset GUID probe returned an empty GUID: {}",
            clip(&output, 180)
        ))
    } else {
        Ok(Some(guid))
    }
}

async fn stabilize_parallel_edit_refresh_execution(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let started = Instant::now();
    let timeout = recompile_wait(config);
    loop {
        wait_for_semantic_ready(
            project,
            suite,
            "parallel_edit_refresh_execute_preflight",
            SemanticReadyRequirement::UnityApi,
            remaining_or_timeout(started, timeout, "parallel refresh execute preflight")?,
            config.poll_interval,
            sink,
            cancel_rx,
        )
        .await?;

        match execute_capture(project, r#"print("LPR_WARMUP:ready");"#).await {
            Ok(output) if output.contains("LPR_WARMUP:ready") => return Ok(()),
            Ok(output) => {
                return Err(format!(
                    "parallel refresh execute preflight returned no marker: {}",
                    clip(&output, 160)
                ));
            }
            Err(error) if unity_reload_boundary_error(&error) && started.elapsed() < timeout => {
                sink.emit(
                    "suite_event",
                    json!({
                        "suite": suite.as_str(),
                        "line": "WAIT  parallel-edit-refresh: Unity reloaded during execute preflight; retrying after readiness",
                        "passed": 0,
                        "failed": 0,
                    }),
                );
            }
            Err(error) => return Err(error),
        }
    }
}

async fn end_edit_session_for_cleanup(project: &str, owner: &str) -> Result<String, String> {
    let started = Instant::now();
    let timeout = Duration::from_secs(20);
    loop {
        match unity_bridge::end_edit_session(project, owner).await {
            Ok(message) => return Ok(message),
            Err(error) if unity_reload_boundary_error(&error) && started.elapsed() < timeout => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn cleanup_parallel_edit_refresh_fixture(
    project: &str,
    owner_a: &str,
    owner_b: &str,
    fixture_asset_dir: &str,
    fixture_dir: &Path,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = end_edit_session_for_cleanup(project, owner_a).await {
        errors.push(format!("end owner A: {error}"));
    }
    if let Err(error) = end_edit_session_for_cleanup(project, owner_b).await {
        errors.push(format!("end owner B: {error}"));
    }

    let cleanup_code = format!(
        r#"AssetDatabase.DeleteAsset("{fixture_asset_dir}"); AssetDatabase.Refresh(); print("LPR_CLEANUP:done");"#
    );
    if let Err(error) = execute_capture(project, &cleanup_code).await {
        errors.push(format!("AssetDatabase cleanup: {}", clip(&error, 160)));
    }

    let project_assets = Path::new(project).join("Assets");
    let fixture_name_is_safe = fixture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("LocusParallelEditRefreshSelfTest_"));
    if fixture_dir.starts_with(&project_assets) && fixture_name_is_safe {
        match tokio::fs::remove_dir_all(fixture_dir).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("filesystem fixture cleanup: {error}")),
        }
        let fixture_meta = fixture_dir.with_extension("meta");
        match tokio::fs::remove_file(&fixture_meta).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => errors.push(format!("filesystem meta cleanup: {error}")),
        }
    } else {
        errors.push(format!(
            "refused unsafe fixture cleanup path: {}",
            fixture_dir.display()
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Reproduces the parallel-agent refresh boundary against a real Unity
/// AssetDatabase. Two independent edit-session owners overlap, a unique asset
/// is written to disk and queued, then owner A exits while owner B remains.
/// The completed asset must be imported at A's boundary; otherwise a long
/// sibling run can hold finished work invisible indefinitely.
async fn run_parallel_edit_refresh_suite(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": suite.as_str(),
            "project": project,
        }),
    );

    crate::csharp_compile::set_enabled(true).await;
    crate::csharp_compile::warm_up_in_background();
    stabilize_parallel_edit_refresh_execution(project, suite, config, sink, cancel_rx).await?;

    let token = uuid::Uuid::new_v4().simple().to_string();
    let owner_a = format!("parallel-refresh-a-{token}");
    let owner_b = format!("parallel-refresh-b-{token}");
    let fixture_asset_dir = format!("Assets/LocusParallelEditRefreshSelfTest_{token}");
    let fixture_asset_path = format!("{fixture_asset_dir}/probe.txt");
    let fixture_dir = Path::new(project)
        .join("Assets")
        .join(format!("LocusParallelEditRefreshSelfTest_{token}"));
    let fixture_file = fixture_dir.join("probe.txt");

    let test_result: Result<(usize, usize, usize, String), String> = async {
        tokio::fs::create_dir_all(&fixture_dir)
            .await
            .map_err(|error| format!("failed to create isolated fixture directory: {error}"))?;

        let count_a = parse_active_edit_session_count(
            &unity_bridge::begin_edit_session(project, &owner_a).await?,
        )?;
        let count_b = parse_active_edit_session_count(
            &unity_bridge::begin_edit_session(project, &owner_b).await?,
        )?;
        if count_b != count_a.saturating_add(1) {
            return Err(format!(
                "second edit-session owner did not increment the active count: {count_a} -> {count_b}"
            ));
        }

        tokio::fs::write(
            &fixture_file,
            format!("Locus parallel edit refresh integration fixture {token}\n"),
        )
        .await
        .map_err(|error| format!("failed to write isolated fixture: {error}"))?;
        unity_bridge::import_assets(project, std::slice::from_ref(&fixture_asset_path)).await?;

        match probe_asset_guid(project, &fixture_asset_path).await? {
            None => {}
            Some(guid) => {
                return Err(format!(
                    "fixture imported before either edit-session owner ended (guid={guid})"
                ))
            }
        }

        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": format!(
                    "PASS  parallel-edit-refresh: queued fixture while two owners were active ({count_a} -> {count_b})"
                ),
                "passed": 2,
                "failed": 0,
            }),
        );

        let count_after_a = parse_active_edit_session_count(
            &unity_bridge::end_edit_session(project, &owner_a).await?,
        )?;
        if count_after_a != count_a {
            return Err(format!(
                "ending owner A did not preserve the other active owner: expected {count_a}, got {count_after_a}"
            ));
        }

        let wait_budget = config.suite_timeout.min(Duration::from_secs(20));
        let started = Instant::now();
        loop {
            if run_cancelled(cancel_rx) {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
            if let Some(guid) = probe_asset_guid(project, &fixture_asset_path).await? {
                break Ok((count_a, count_b, count_after_a, guid));
            }
            if started.elapsed() >= wait_budget {
                break Err(format!(
                    "completed asset stayed outside the AssetDatabase for {}ms after owner A ended while owner B remained active",
                    wait_budget.as_millis()
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(config.poll_interval.min(Duration::from_secs(1))) => {}
                _ = cancel_rx.changed() => {
                    return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
                }
            }
        }
    }
    .await;

    let cleanup_result = cleanup_parallel_edit_refresh_fixture(
        project,
        &owner_a,
        &owner_b,
        &fixture_asset_dir,
        &fixture_dir,
    )
    .await;

    match (test_result, cleanup_result) {
        (Ok((count_a, count_b, count_after_a, guid)), Ok(())) => {
            sink.emit(
                "suite_event",
                json!({
                    "suite": suite.as_str(),
                    "line": format!(
                        "PASS  parallel-edit-refresh: owner A imported the completed asset while owner B remained active (counts {count_a} -> {count_b} -> {count_after_a}, guid={guid})"
                    ),
                    "passed": 4,
                    "failed": 0,
                }),
            );
            sink.emit(
                "suite_result",
                json!({
                    "suite": suite.as_str(),
                    "passed": 4,
                    "failed": 0,
                    "activeOwnersAfterFirstEnd": count_after_a,
                    "assetGuid": guid,
                    "fixtureCleaned": true,
                }),
            );
            Ok(())
        }
        (Err(error), Ok(())) => {
            emit_suite_failure(sink, suite, &error);
            Err(error)
        }
        (Ok(_), Err(cleanup_error)) => {
            let error = format!("parallel refresh checks passed, cleanup failed: {cleanup_error}");
            emit_suite_failure(sink, suite, &error);
            Err(error)
        }
        (Err(error), Err(cleanup_error)) => {
            let error = format!("{error}; cleanup failed: {cleanup_error}");
            emit_suite_failure(sink, suite, &error);
            Err(error)
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
    cancel_rx: &mut watch::Receiver<bool>,
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
    run.check_marker(
        project,
        "E1 round-trip",
        r#"print("E1:" + (40 + 2));"#,
        "E1:42",
    )
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
    run.check_marker(
        project,
        "E3R sync ref-local",
        r#"var values = new[] { 41 }; ref int value = ref values[0]; value++; print("E3R:" + value);"#,
        "E3R:42",
    )
    .await;
    match unity_bridge::unity_execute_code_with_non_public_access(
        project,
        r#"var values = new[] { Path.GetTempPath() }.Where(path => path.Length > 0).ToArray(); print("E3IO:" + values.Length);"#,
        true,
    )
    .await
    {
        Ok(output) if output.contains("E3IO:1") => {
            run.pass("E3IO aliases + LINQ", "common IO alias and LINQ ToArray compiled together");
        }
        Ok(output) => run.fail(
            "E3IO aliases + LINQ",
            format!("expected 'E3IO:1', got '{}'", clip(&output, 160)),
        ),
        Err(error) => run.fail(
            "E3IO aliases + LINQ",
            format!("execute error: {}", clip(&error, 200)),
        ),
    }
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
    run.check_thread_and_tick_discovery(project).await;
    run.check_pending_await_diagnostics(project).await;
    run.check_cancellation(project).await;
    run.check_concurrency(project).await;
    run.check_player_loop_debugger(project).await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Alternate compile backend and the run-states path.
    run.check_legacy_compile(project).await;
    run.check_run_states(project).await;
    if run_cancelled(cancel_rx) {
        return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
    }

    // Compiler/JIT capability experiment for private/internal access. Force a
    // Debug-effective editor first so inlining cannot hide access checks, then
    // compare the low-level cells with both real generated wrapper shapes.
    run.check_non_public_access_probes(project, config, cancel_rx)
        .await?;
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

async fn run_hot_reload_suite(
    app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    plugin_outcome: PluginPrepareOutcome,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
    force_release: bool,
) -> Result<(), String> {
    crate::csharp_compile::set_enabled(true).await;
    crate::csharp_compile::warm_up_in_background();
    crate::unity_hotreload::set_enabled(true);
    let semantic_ready_timeout = connection_timeout_for_plugin_outcome(config, plugin_outcome);

    if force_release {
        for desired in ["release", "debug"] {
            run_hot_reload_selftest_once(
                app_handle,
                project,
                suite,
                config,
                semantic_ready_timeout,
                sink,
                cancel_rx,
                Some(desired),
                true,
            )
            .await?;
        }
        Ok(())
    } else {
        run_hot_reload_selftest_once(
            app_handle,
            project,
            suite,
            config,
            semantic_ready_timeout,
            sink,
            cancel_rx,
            None,
            false,
        )
        .await
    }
}

async fn run_hot_reload_selftest_once(
    app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    semantic_ready_timeout: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
    desired_code_optimization: Option<&'static str>,
    force_set_code_optimization: bool,
) -> Result<(), String> {
    if config.force_edit_mode {
        ensure_edit_mode(
            project,
            suite,
            semantic_ready_timeout,
            config.poll_interval,
            sink,
            cancel_rx,
        )
        .await?;
    }

    if let Some(desired) = desired_code_optimization {
        sink.emit(
            "code_optimization",
            json!({
                "suite": suite.as_str(),
                "action": "phase_start",
                "desired": desired,
            }),
        );
        ensure_code_optimization(
            project,
            suite,
            desired,
            semantic_ready_timeout,
            config.poll_interval,
            sink,
            cancel_rx,
            force_set_code_optimization,
        )
        .await?;
    }

    wait_for_semantic_ready(
        project,
        suite,
        "hot_reload_preflight",
        SemanticReadyRequirement::AssetModification,
        semantic_ready_timeout,
        config.poll_interval,
        sink,
        cancel_rx,
    )
    .await?;

    let summary = run_event_selftest(
        app_handle,
        project,
        suite,
        config.suite_timeout,
        config.no_progress_timeout,
        sink,
        cancel_rx,
        crate::unity_hotreload::selftest::run(app_handle.clone(), project.to_string()),
    )
    .await?;
    ensure_summary_passed(summary)
}

async fn ensure_code_optimization(
    project: &str,
    suite: CliDriverSuite,
    desired: &'static str,
    timeout: Duration,
    poll_interval: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
    force_set: bool,
) -> Result<Option<String>, String> {
    wait_for_semantic_ready(
        project,
        suite,
        "code_optimization_probe",
        SemanticReadyRequirement::AssetModification,
        timeout,
        poll_interval,
        sink,
        cancel_rx,
    )
    .await?;

    let (connected, before) =
        crate::unity_hotreload::coordinator::detect_code_optimization(project).await;
    sink.emit(
        "code_optimization",
        json!({
            "suite": suite.as_str(),
            "action": "probe",
            "connected": connected,
            "desired": desired,
            "before": before,
        }),
    );

    if before.as_deref() == Some(desired) && !force_set {
        return Ok(before);
    }

    let started = Instant::now();
    let reported = loop {
        wait_for_semantic_ready(
            project,
            suite,
            "code_optimization_set",
            SemanticReadyRequirement::AssetModification,
            remaining_or_timeout(started, timeout, "Unity Code Optimization preflight")?,
            poll_interval,
            sink,
            cancel_rx,
        )
        .await?;

        match crate::unity_hotreload::coordinator::set_code_optimization(project, desired).await {
            Ok(reported) => break reported,
            Err(error) if unity_reload_boundary_error(&error) && started.elapsed() < timeout => {
                sink.emit(
                    "code_optimization",
                    json!({
                        "suite": suite.as_str(),
                        "action": "retry_after_reload",
                        "desired": desired,
                        "error": error,
                        "elapsedMs": started.elapsed().as_millis(),
                    }),
                );
            }
            Err(error) => return Err(error),
        }
    };
    sink.emit(
        "code_optimization",
        json!({
            "suite": suite.as_str(),
            "action": "set",
            "desired": desired,
            "reported": reported,
        }),
    );

    wait_for_code_optimization(
        project,
        suite,
        desired,
        timeout,
        poll_interval,
        sink,
        cancel_rx,
    )
    .await?;
    Ok(before)
}

async fn wait_for_code_optimization(
    project: &str,
    suite: CliDriverSuite,
    desired: &'static str,
    timeout: Duration,
    poll_interval: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        if *cancel_rx.borrow() {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }

        let status = unity_bridge::query_unity_connection_status(project).await;
        let (probe_connected, code_optimization) = if status.connected {
            crate::unity_hotreload::coordinator::detect_code_optimization(project).await
        } else {
            (false, None)
        };
        if status.connected
            && status.editor_status == UNITY_EDITOR_STATUS_EDITING
            && probe_connected
            && code_optimization.as_deref() == Some(desired)
        {
            sink.emit(
                "code_optimization",
                json!({
                    "suite": suite.as_str(),
                    "action": "ready",
                    "desired": desired,
                    "codeOptimization": code_optimization,
                    "elapsedMs": started.elapsed().as_millis(),
                }),
            );
            return Ok(());
        }

        let last_detail = format!(
            "connected={} editorStatus={} probeConnected={} codeOptimization={}",
            status.connected,
            status.editor_status,
            probe_connected,
            code_optimization.as_deref().unwrap_or("unknown")
        );
        if started.elapsed() >= timeout {
            return Err(format!(
                "Unity Code Optimization did not reach {desired} within {}ms; last {last_detail}",
                timeout.as_millis(),
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

async fn ensure_edit_mode(
    project: &str,
    suite: CliDriverSuite,
    timeout: Duration,
    poll_interval: Duration,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    wait_for_semantic_ready(
        project,
        suite,
        "editor_mode_probe",
        SemanticReadyRequirement::UnityApi,
        timeout,
        poll_interval,
        sink,
        cancel_rx,
    )
    .await?;

    let status = unity_bridge::query_unity_connection_status(project).await;
    if status.editor_status == UNITY_EDITOR_STATUS_EDITING {
        wait_for_semantic_ready(
            project,
            suite,
            "editor_mode_ready",
            SemanticReadyRequirement::AssetModification,
            timeout,
            poll_interval,
            sink,
            cancel_rx,
        )
        .await?;
        return Ok(());
    }

    let request_started = Instant::now();
    loop {
        sink.emit(
            "editor_mode",
            json!({ "action": "set", "desiredStatus": UNITY_EDITOR_STATUS_EDITING }),
        );
        match unity_bridge::set_editor_status(project, UNITY_EDITOR_STATUS_EDITING).await {
            Ok(()) => break,
            Err(error)
                if unity_reload_boundary_error(&error) && request_started.elapsed() < timeout =>
            {
                sink.emit(
                    "editor_mode",
                    json!({
                        "action": "retry_after_reload",
                        "desiredStatus": UNITY_EDITOR_STATUS_EDITING,
                        "error": error,
                        "elapsedMs": request_started.elapsed().as_millis(),
                    }),
                );
                wait_for_semantic_ready(
                    project,
                    suite,
                    "editor_mode_retry",
                    SemanticReadyRequirement::UnityApi,
                    remaining_or_timeout(request_started, timeout, "Unity edit-mode request")?,
                    poll_interval,
                    sink,
                    cancel_rx,
                )
                .await?;
            }
            Err(error) => return Err(error),
        }
    }

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
            wait_for_semantic_ready(
                project,
                suite,
                "editor_mode_ready",
                SemanticReadyRequirement::AssetModification,
                remaining_or_timeout(started, timeout, "Unity edit-mode stabilization")?,
                poll_interval,
                sink,
                cancel_rx,
            )
            .await?;
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
    no_progress_timeout: Duration,
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
            "noProgressTimeoutMs": no_progress_timeout.as_millis(),
        }),
    );

    let mut start_task = tokio::spawn(start);
    let timeout_sleep = tokio::time::sleep(timeout);
    tokio::pin!(timeout_sleep);
    let no_progress_sleep = tokio::time::sleep(no_progress_timeout);
    tokio::pin!(no_progress_sleep);
    let mut start_done = false;
    let mut last_event_line: Option<String> = None;
    let mut last_event_passed = 0u32;
    let mut last_event_failed = 0u32;

    loop {
        tokio::select! {
            _ = &mut timeout_sleep => {
                if !start_done {
                    start_task.abort();
                }
                app_handle.unlisten(listener);
                let message = format!(
                    "Suite {} timed out after {}ms",
                    suite.as_str(),
                    timeout.as_millis()
                );
                emit_suite_failure(sink, suite, &message);
                return Err(message);
            }
            _ = cancel_rx.changed() => {
                if !start_done {
                    start_task.abort();
                }
                app_handle.unlisten(listener);
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
            _ = &mut no_progress_sleep => {
                if !start_done {
                    start_task.abort();
                }
                app_handle.unlisten(listener);
                let message = format!(
                    "Suite {} made no event progress for {}ms",
                    suite.as_str(),
                    no_progress_timeout.as_millis()
                );
                sink.emit(
                    "suite_no_progress",
                    json!({
                        "suite": suite.as_str(),
                        "timeoutMs": no_progress_timeout.as_millis(),
                        "line": last_event_line,
                        "passed": last_event_passed,
                        "failed": last_event_failed,
                        "message": message.clone(),
                    }),
                );
                emit_suite_failure(sink, suite, &message);
                return Err(message);
            }
            result = &mut start_task, if !start_done => {
                start_done = true;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        app_handle.unlisten(listener);
                        let message =
                            format!("Suite {} failed to start: {}", suite.as_str(), error);
                        emit_suite_failure(sink, suite, &message);
                        return Err(message);
                    }
                    Err(error) => {
                        app_handle.unlisten(listener);
                        let message = format!("Suite {} task failed: {}", suite.as_str(), error);
                        emit_suite_failure(sink, suite, &message);
                        return Err(message);
                    }
                }
            }
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else {
                    app_handle.unlisten(listener);
                    let message = format!("Suite {} event stream closed", suite.as_str());
                    emit_suite_failure(sink, suite, &message);
                    return Err(message);
                };
                no_progress_sleep
                    .as_mut()
                    .reset(tokio::time::Instant::now() + no_progress_timeout);
                last_event_passed = event.passed;
                last_event_failed = event.failed;
                // Forward every emitted line live so the UI output console fills
                // in as the self-test runs, not only when it fails.
                if let Some(line) = event.line.clone() {
                    last_event_line = Some(line.clone());
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
    use super::{parse_active_edit_session_count, CliDriverConfig, CliDriverSuite};
    use crate::unity_bridge::UnityLaunchCodeOptimization;

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
                CliDriverSuite::HotReloadRelease,
                CliDriverSuite::ParallelEditRefresh,
                CliDriverSuite::Execute,
                CliDriverSuite::YamlParity
            ]
        );
    }

    #[test]
    fn parse_hot_reload_release_suite_aliases() {
        for alias in [
            "hot-reload-release",
            "hot_release",
            "hot-release",
            "release-hot-reload",
        ] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::HotReloadRelease],
                "alias {alias}"
            );
            assert_eq!(
                parsed.launch_code_optimization(),
                Some(UnityLaunchCodeOptimization::Release),
                "alias {alias}"
            );
        }
    }

    #[test]
    fn parse_execute_suite_aliases() {
        for alias in [
            "execute",
            "exec",
            "unity-execute",
            "execute-code",
            "run-states",
        ] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::Execute],
                "alias {alias}"
            );
        }
    }

    #[test]
    fn parse_parallel_edit_refresh_suite_aliases() {
        for alias in ["parallel-edit-refresh", "parallel_refresh", "edit-refresh"] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::ParallelEditRefresh],
                "alias {alias}"
            );
        }
    }

    #[test]
    fn parses_active_edit_session_responses() {
        assert_eq!(
            parse_active_edit_session_count("active_edit_sessions:2").unwrap(),
            2
        );
        assert!(parse_active_edit_session_count("owners:2").is_err());
    }

    #[test]
    fn parse_unity_test_suite_aliases() {
        for alias in ["unity-test", "unity_test", "test-framework"] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::UnityTest],
                "alias {alias}"
            );
        }
    }

    #[test]
    fn parse_yaml_parity_suite_and_sampling_options() {
        for alias in ["yaml-parity", "yaml_parity", "yaml-diff"] {
            let parsed = parse(&[
                "--locus-unity-test",
                "--suite",
                alias,
                "--yaml-parity-samples",
                "7",
                "--yaml-parity-seed=-42",
            ])
            .unwrap()
            .unwrap();
            assert_eq!(parsed.suites, vec![CliDriverSuite::YamlParity]);
            assert_eq!(parsed.yaml_parity_sample_count, 7);
            assert_eq!(parsed.yaml_parity_seed, -42);
        }
    }

    #[test]
    fn parse_yaml_parity_rejects_out_of_range_sample_count() {
        let error = parse(&[
            "--locus-unity-test",
            "--suite",
            "yaml-parity",
            "--yaml-parity-samples",
            "51",
        ])
        .unwrap()
        .unwrap_err();
        assert!(error.contains("1 to 50"));
    }
}
