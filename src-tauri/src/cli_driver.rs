use std::{
    collections::{BTreeMap, BTreeSet},
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
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::sync::watch;

use crate::unity_bridge::{
    self, PluginStatus, UnityConnectionStatus, UnityEditorProcessState,
    UnityLaunchCodeOptimization, UNITY_EDITOR_STATUS_EDITING,
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
    Workspace,
    WorkspaceSwitch,
    SessionUndo,
    Connect,
    Sidecar,
    TypeIndex,
    StateProbe,
    NativeBridge,
    HotReload,
    HotReloadRelease,
    ParallelEditRefresh,
    RecompileImport,
    Execute,
    PythonSdk,
    ModalDialog,
    SafeMode,
    YamlParity,
    UnityTest,
}

impl CliDriverSuite {
    fn as_str(self) -> &'static str {
        match self {
            CliDriverSuite::Workspace => "workspace",
            CliDriverSuite::WorkspaceSwitch => "workspace-switch",
            CliDriverSuite::SessionUndo => "session-undo",
            CliDriverSuite::Connect => "connect",
            CliDriverSuite::Sidecar => "sidecar",
            CliDriverSuite::TypeIndex => "type-index",
            CliDriverSuite::StateProbe => "state-probe",
            CliDriverSuite::NativeBridge => "native-bridge",
            CliDriverSuite::HotReload => "hot-reload",
            CliDriverSuite::HotReloadRelease => "hot-reload-release",
            CliDriverSuite::ParallelEditRefresh => "parallel-edit-refresh",
            CliDriverSuite::RecompileImport => "recompile-import",
            CliDriverSuite::Execute => "execute",
            CliDriverSuite::PythonSdk => "python-sdk",
            CliDriverSuite::ModalDialog => "modal-dialog",
            CliDriverSuite::SafeMode => "safe-mode",
            CliDriverSuite::YamlParity => "yaml-parity",
            CliDriverSuite::UnityTest => "unity-test",
        }
    }

    fn event_name(self) -> Option<&'static str> {
        match self {
            CliDriverSuite::Workspace => None,
            CliDriverSuite::WorkspaceSwitch => None,
            CliDriverSuite::SessionUndo => None,
            CliDriverSuite::Connect => None,
            CliDriverSuite::Sidecar => None,
            CliDriverSuite::TypeIndex => None,
            CliDriverSuite::StateProbe => Some("unity-state-probe-selftest"),
            CliDriverSuite::NativeBridge => Some("unity-native-bridge-selftest"),
            CliDriverSuite::HotReload => Some("unity-hotreload-selftest"),
            CliDriverSuite::HotReloadRelease => Some("unity-hotreload-selftest"),
            CliDriverSuite::ParallelEditRefresh => None,
            CliDriverSuite::RecompileImport => None,
            // Bespoke suite: emits its own suite_* events like sidecar/type-index.
            CliDriverSuite::Execute => None,
            CliDriverSuite::PythonSdk => None,
            CliDriverSuite::ModalDialog => None,
            CliDriverSuite::SafeMode => None,
            CliDriverSuite::YamlParity => None,
            CliDriverSuite::UnityTest => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CliDriverConfig {
    pub project_path: Option<String>,
    pub workspace_paths: Vec<String>,
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
    pub workspace_paths: Vec<String>,
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
            workspace_paths: self.workspace_paths,
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
    pub fn requires_frontend(&self) -> bool {
        self.suites.contains(&CliDriverSuite::SessionUndo)
    }

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
        let mut workspace_paths = Vec::new();
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
                Some(("--workspace-project", value)) => {
                    let value =
                        match read_option_value("--workspace-project", value, &args, &mut index) {
                            Ok(value) => value,
                            Err(error) => return Some(Err(error)),
                        };
                    workspace_paths.push(value);
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
            workspace_paths,
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
        | "--workspace-project"
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
                CliDriverSuite::PythonSdk,
                CliDriverSuite::ModalDialog,
                CliDriverSuite::YamlParity,
            ] {
                if !suites.contains(&suite) {
                    suites.push(suite);
                }
            }
            return Ok(());
        }
        "connect" => CliDriverSuite::Connect,
        "workspace" | "multi-workspace" | "multi_workspace" => CliDriverSuite::Workspace,
        "workspace-switch" | "workspace_switch" | "cross-project" | "cross_project" => {
            CliDriverSuite::WorkspaceSwitch
        }
        "session-undo" | "session_undo" | "undo" | "file-undo" | "file_undo" => {
            CliDriverSuite::SessionUndo
        }
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
        "recompile-import" | "recompile_import" | "compile-import" | "compile_import"
        | "asset-refresh" | "asset_refresh" => CliDriverSuite::RecompileImport,
        "execute" | "exec" | "unity-execute" | "unity_execute" | "execute-code" | "run-states"
        | "run_states" | "runstates" => CliDriverSuite::Execute,
        "python-sdk" | "python_sdk" | "sdk" | "sdk-editor" | "sdk_editor" => {
            CliDriverSuite::PythonSdk
        }
        "modal-dialog" | "modal_dialog" | "dialog" | "unity-dialog" | "unity_dialog" => {
            CliDriverSuite::ModalDialog
        }
        "safe-mode" | "safe_mode" | "safe-mode-recovery" | "editor-recovery" => {
            CliDriverSuite::SafeMode
        }
        "yaml-parity" | "yaml_parity" | "yaml-diff" | "yaml_diff" => {
            CliDriverSuite::YamlParity
        }
        "unity-test" | "unity_test" | "test-framework" | "test_framework" => {
            CliDriverSuite::UnityTest
        }
        _ => {
            return Err(format!(
            "Unknown --suite '{}'. Use workspace, workspace-switch, session-undo, connect, sidecar, type-index, state-probe, native-bridge, hot-reload, hot-reload-release, parallel-edit-refresh, recompile-import, execute, python-sdk, modal-dialog, safe-mode, yaml-parity, unity-test, or all.",
            value
        ))
        }
    };
    if !suites.contains(&expanded) {
        suites.push(expanded);
    }
    Ok(())
}

pub fn spawn(app_handle: AppHandle, config: CliDriverConfig) {
    // The headless CLI driver is not interruptible; hand `run_driver` a receiver
    // whose sender stays alive for the whole run so its cancel selects never fire.
    let (cancel_tx, cancel_rx) = watch::channel(false);
    tauri::async_runtime::spawn(async move {
        let _cancel_guard = cancel_tx;
        let sink = DriverEventSink::cli();
        let exit_code = match run_driver(app_handle.clone(), config, sink.clone(), cancel_rx).await
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
        let result = run_driver(app_handle, config, sink.clone(), cancel_rx).await;
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
    config: CliDriverConfig,
    sink: DriverEventSink,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<(), String> {
    app_handle
        .state::<Arc<crate::config::AppConfig>>()
        .set_debug_enabled(true)?;
    sink.emit(
        "start",
        json!({
            "driver": DRIVER_NAME,
            "suites": config.suites.iter().map(|suite| suite.as_str()).collect::<Vec<_>>(),
            "workspaceProjects": &config.workspace_paths,
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

    if config.suites.contains(&CliDriverSuite::Workspace)
        || config.suites.contains(&CliDriverSuite::WorkspaceSwitch)
    {
        if config.suites.len() != 1 {
            return Err(
                "Multi-workspace suites must run alone because they own multiple Unity projects"
                    .to_string(),
            );
        }
        if config.suites.contains(&CliDriverSuite::WorkspaceSwitch) {
            run_workspace_switch_suite(&app_handle, &config, &sink, &mut cancel_rx).await?;
        } else {
            run_workspace_suite(&app_handle, &config, &sink, &mut cancel_rx).await?;
        }
        sink.emit("finished", json!({ "ok": true }));
        return Ok(());
    }

    if config.suites.contains(&CliDriverSuite::SessionUndo) {
        if config.suites.len() != 1 {
            return Err(
                "The session-undo suite must run alone because it owns its fixture and undo stack"
                    .to_string(),
            );
        }
        let project = resolve_project_path(config.project_path.as_deref(), &app_handle).await?;
        run_session_undo_suite(&app_handle, &project, &config, &sink).await?;
        sink.emit("finished", json!({ "ok": true }));
        return Ok(());
    }

    let project = resolve_project_path(config.project_path.as_deref(), &app_handle).await?;
    set_workspace_for_driver(&app_handle, &project).await?;
    prepare_suite_environment(&project, &config, &sink)?;
    let plugin_outcome = check_or_install_plugin(&project, config.install_plugin, &sink).await?;

    let python_sdk_ran = if config.suites.contains(&CliDriverSuite::PythonSdk) {
        run_python_sdk_editor_suite(&app_handle, &project, &config, &sink).await?;
        true
    } else {
        false
    };

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
            CliDriverSuite::Workspace => Err(
                "The workspace suite must be dispatched before single-project suites".to_string(),
            ),
            CliDriverSuite::WorkspaceSwitch => Err(
                "The workspace-switch suite must be dispatched before single-project suites"
                    .to_string(),
            ),
            CliDriverSuite::SessionUndo => Err(
                "The session-undo suite must be dispatched before Unity connection setup"
                    .to_string(),
            ),
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
            CliDriverSuite::RecompileImport => {
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
                        run_recompile_import_suite(&project, *suite, &config, &sink, &mut cancel_rx)
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
                        run_execute_suite(
                            &app_handle,
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
            CliDriverSuite::PythonSdk if python_sdk_ran => Ok(()),
            CliDriverSuite::PythonSdk => Err(
                "The python-sdk suite did not run before the shared connection preflight"
                    .to_string(),
            ),
            CliDriverSuite::ModalDialog => {
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
                        run_modal_dialog_suite(&app_handle, &project, *suite, &config, &sink).await
                    }
                    Err(error) => Err(error),
                }
            }
            CliDriverSuite::SafeMode => {
                run_safe_mode_recovery_suite(
                    &app_handle,
                    &project,
                    *suite,
                    &config,
                    &sink,
                    &mut cancel_rx,
                )
                .await
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

#[derive(Clone)]
struct WorkspaceSuiteTarget {
    index: usize,
    project: String,
    runtime: Arc<crate::workspace_service::WorkspaceRuntime>,
    session_id: String,
    plugin_outcome: PluginPrepareOutcome,
}

async fn run_workspace_suite(
    app_handle: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let projects = resolve_workspace_project_paths(app_handle, config).await?;
    sink.emit(
        "suite_start",
        json!({
            "suite": CliDriverSuite::Workspace.as_str(),
            "projects": projects,
        }),
    );

    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let resource_policy = app_handle
        .state::<Arc<crate::resource_policy::ResourcePolicyStore>>()
        .inner()
        .clone();
    let policy_before = resource_policy.snapshot();
    let mut workspace_limits = policy_before.limits.clone();
    workspace_limits.max_running_workspace_services = workspace_limits
        .max_running_workspace_services
        .max(projects.len());
    workspace_limits.max_lsp_processes = workspace_limits.max_lsp_processes.max(projects.len());
    let policy_after = if workspace_limits == policy_before.limits {
        policy_before.clone()
    } else {
        resource_policy
            .update(workspace_limits)
            .map_err(|error| format!("Failed to prepare workspace test resource policy: {error}"))?
    };
    registry.notify_policy_changed();
    registry.converge_resource_policy().await;
    sink.emit(
        "workspace_policy",
        json!({
            "before": policy_before,
            "after": policy_after,
            "targetWorkspaceCount": projects.len(),
        }),
    );

    crate::csharp_lsp::set_enabled(true, None).await;
    crate::csharp_compile::set_enabled(true).await;
    let mut scoped_events = registry.event_router().subscribe();

    let mut plugin_outcomes = Vec::with_capacity(projects.len());
    for (index, project) in projects.iter().enumerate() {
        if run_cancelled(cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        sink.emit(
            "workspace_target",
            json!({ "index": index, "project": project, "phase": "plugin" }),
        );
        plugin_outcomes.push(check_or_install_plugin(project, config.install_plugin, sink).await?);
    }

    let mut targets = Vec::with_capacity(projects.len());
    for (index, (project, plugin_outcome)) in
        projects.iter().zip(plugin_outcomes.into_iter()).enumerate()
    {
        if run_cancelled(cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        open_and_focus_workspace_for_driver(app_handle, project).await?;
        let runtime = registry
            .runtime_for_root(Path::new(project))
            .ok_or_else(|| format!("Workspace runtime was not registered for {project}"))?;
        let session_id =
            create_workspace_driver_session(app_handle, index, runtime.as_ref()).await?;
        sink.emit(
            "workspace_registered",
            json!({
                "index": index,
                "project": project,
                "projectId": runtime.project_id(),
                "checkoutId": runtime.checkout_id(),
                "workspaceGeneration": runtime.generation(),
                "sessionId": session_id,
            }),
        );
        targets.push(WorkspaceSuiteTarget {
            index,
            project: project.clone(),
            runtime,
            session_id,
            plugin_outcome,
        });
    }

    validate_workspace_identities(&targets)?;

    let connection_futures = targets.iter().cloned().map(|target| {
        let config = config.clone();
        let sink = sink.clone();
        let mut target_cancel = cancel_rx.clone();
        async move {
            let status = ensure_connected(
                &target.project,
                &config,
                target.plugin_outcome,
                &sink,
                &mut target_cancel,
            )
            .await?;
            Ok::<_, String>((target, status))
        }
    });
    let connected = futures::future::join_all(connection_futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let process_ids = connected
        .iter()
        .filter_map(|(_, status)| status.editor_process_id)
        .collect::<BTreeSet<_>>();
    if process_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct Unity Editor processes, observed {:?}",
            targets.len(),
            process_ids
        ));
    }
    sink.emit(
        "workspace_connections",
        json!({
            "connected": connected.iter().map(|(target, status)| json!({
                "index": target.index,
                "project": target.project,
                "checkoutId": target.runtime.checkout_id(),
                "processId": status.editor_process_id,
                "channel": status.control_channel_state,
            })).collect::<Vec<_>>()
        }),
    );

    let probe_contexts = create_workspace_probe_contexts(&registry, &targets).await?;
    validate_workspace_tool_routing(app_handle, &targets, &probe_contexts).await?;
    validate_workspace_event_routing(
        app_handle,
        &registry,
        &targets,
        &mut scoped_events,
        config.suite_timeout,
    )
    .await?;

    let compile_futures = targets.iter().map(|target| compile_workspace_probe(target));
    let compile_results = futures::future::join_all(compile_futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let compile_checkout_ids = compile_results
        .iter()
        .filter_map(|value| value.get("checkoutId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    let compile_session_ids = compile_results
        .iter()
        .filter_map(|value| value.get("editorSessionId").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if compile_checkout_ids.len() != targets.len() || compile_session_ids.len() != targets.len() {
        return Err(format!(
            "Compile scopes were not isolated: checkoutIds={}, editorSessionIds={}, targets={}",
            compile_checkout_ids.len(),
            compile_session_ids.len(),
            targets.len()
        ));
    }
    sink.emit(
        "workspace_compile_scopes",
        json!({ "scopes": compile_results }),
    );

    let launches = futures::future::join_all(
        targets
            .iter()
            .map(|target| launch_workspace_mock_chat(app_handle, target)),
    )
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let agent_results =
        futures::future::join_all(launches.iter().zip(targets.iter()).map(|(launch, target)| {
            wait_for_workspace_mock_chat(app_handle, launch, target, config.suite_timeout)
        }))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    sink.emit(
        "workspace_mock_agents",
        json!({ "model": "mock/tool", "runs": agent_results }),
    );

    futures::future::join_all(targets.iter().map(|target| async move {
        crate::csharp_lsp::warm_up_workspace(&target.project)
            .await
            .map_err(|error| format!("LSP warm-up failed for {}: {error}", target.project))
    }))
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let lsp_metrics = wait_for_workspace_lsp_entries(&targets, config.suite_timeout).await?;
    let workspace_metrics = registry.metrics().await;
    if workspace_metrics.running_workspace_services < targets.len() {
        return Err(format!(
            "Expected at least {} running workspace services, observed {}",
            targets.len(),
            workspace_metrics.running_workspace_services
        ));
    }
    sink.emit(
        "workspace_metrics",
        json!({
            "workspaces": workspace_metrics,
            "lsp": lsp_metrics,
            "compile": crate::csharp_compile::scheduler::metrics(),
        }),
    );

    sink.emit(
        "suite_result",
        json!({
            "suite": CliDriverSuite::Workspace.as_str(),
            "passed": 8,
            "failed": 0,
            "workspaceCount": targets.len(),
            "projectId": targets[0].runtime.project_id(),
            "checkoutIds": targets.iter().map(|target| target.runtime.checkout_id()).collect::<Vec<_>>(),
            "unityProcessIds": process_ids,
            "mockModel": "mock/tool",
        }),
    );
    drop(probe_contexts);
    Ok(())
}

const SESSION_UNDO_FIXTURE_RELATIVE_PATH: &str = ".locus-session-undo-driver-probe.txt";
const SESSION_UNDO_FIXTURE_CONTENT: &str = "LOCUS_SESSION_UNDO_DRIVER_PROBE\n";

async fn probe_session_undo_frontend(
    app_handle: &AppHandle,
    project_id: &str,
    checkout_id: &str,
    workspace_generation: u64,
    session_id: &str,
    phase: &str,
) -> Result<Value, String> {
    let config = json!({
        "projectId": project_id,
        "checkoutId": checkout_id,
        "workspaceGeneration": workspace_generation,
        "sessionId": session_id,
        "fixture": SESSION_UNDO_FIXTURE_RELATIVE_PATH,
        "phase": phase,
    });
    let expression = r#"(() => {
      const config = __LOCUS_SESSION_UNDO_CONFIG__;
      const visible = (element) => {
        if (!(element instanceof HTMLElement)) return false;
        const style = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return style.display !== 'none'
          && style.visibility !== 'hidden'
          && rect.width > 0
          && rect.height > 0;
      };
      const pending = (stage, detail = null) => ({
        ready: false,
        phase: config.phase,
        stage,
        detail,
      });
      const app = document.querySelector('#app')?.__vue_app__;
      if (!app) return pending('vue-app');
      const pinia = app.config?.globalProperties?.$pinia
        ?? Reflect.ownKeys(app._context.provides)
          .map((key) => app._context.provides[key])
          .find((value) => value?._s instanceof Map && value._s.has('ui'));
      if (!pinia) return pending('pinia');
      const ui = pinia._s.get('ui');
      const workbench = pinia._s.get('workbench');

      {
        const workspaceContext = pinia._s.get('workspaceContext');
        if (!ui || !workspaceContext || !workbench) {
          return pending('workbench-stores', [...pinia._s.keys()]);
        }
        ui.setPage('development');
        if (!workspaceContext.checkoutsById?.[config.checkoutId]) {
          return pending('workspace-checkout', Object.keys(workspaceContext.checkoutsById ?? {}));
        }
        workbench.switchWorkspaceScope('main', config.checkoutId);
        const editorId = 'cli-session-undo-' + config.sessionId;
        workbench.openEditor('main', {
          editorId,
          resource: {
            kind: 'session',
            projectId: config.projectId,
            sessionId: config.sessionId,
          },
          title: 'Workspace driver target 0',
          icon: null,
          preview: false,
          pinned: true,
          dirty: false,
          capabilities: { split: true, detach: true, duplicate: true },
          checkoutBinding: {
            checkoutId: config.checkoutId,
            expectedGeneration: config.workspaceGeneration,
          },
          sourcePath: null,
          availability: 'available',
          unavailableReason: null,
        }, {
          preview: false,
          pinned: true,
          replacePreview: true,
        });
      }

      const shell = [...document.querySelectorAll('.workbench-session-shell')].find((candidate) => (
        candidate.dataset.sessionId === config.sessionId && visible(candidate)
      ));
      if (!shell) return pending('session-editor', {
        activePage: ui?.activePage ?? null,
        workbenchScope: workbench?.workspaceScope?.('main') ?? null,
        windowIds: Object.keys(workbench?.windows ?? {}),
        activeEditor: workbench?.activeEditor?.('main') ?? null,
        domEditorIds: [...document.querySelectorAll('[data-editor-id]')]
          .map((element) => element.dataset.editorId),
        sessionShellIds: [...document.querySelectorAll('.workbench-session-shell')]
          .map((element) => element.dataset.sessionId ?? null),
      });
      const toggle = [...shell.querySelectorAll('.changes-toggle-btn')].find(visible);
      if (!toggle) return pending('changes-button');
      if (toggle.disabled) throw new Error('Changes button is disabled after the session became idle.');

      let panel = [...shell.querySelectorAll('.chat-sidebar-panel')].find(visible) ?? null;
      if (!panel) {
        toggle.click();
        return pending('opening-changes-panel');
      }

      const fixtureSelector = '.changes-file-main[title="' + config.fixture + '"]';
      let fixture = panel.querySelector(fixtureSelector);
      if (config.phase === 'empty' && fixture) {
        throw new Error('The isolated fixture was already present before the mock write.');
      }
      if (config.phase === 'written') {
        if (!fixture) return pending('written-fixture');
      }
      if (config.phase === 'undo' && fixture) {
        const confirm = [...panel.querySelectorAll('.confirm-ok')].find(visible);
        if (confirm) {
          if (!confirm.disabled) confirm.click();
          return pending('undo-performing');
        }
        const undoButton = [...panel.querySelectorAll('.undo-btn')].find(visible);
        if (!undoButton) return pending('undo-button');
        if (!undoButton.disabled) undoButton.click();
        return pending('undo-preflight');
      }

      return {
        ready: true,
        phase: config.phase,
        sessionId: config.sessionId,
        editorId: 'cli-session-undo-' + config.sessionId,
        buttonVisible: visible(toggle),
        buttonEnabled: !toggle.disabled,
        panelVisible: visible(panel),
        fixtureVisible: Boolean(fixture && visible(fixture)),
        emptyHintVisible: Boolean([...panel.querySelectorAll('.empty-hint')].find(visible)),
      };
    })()"#
        .replace("__LOCUS_SESSION_UNDO_CONFIG__", &config.to_string());

    let probe_started = Instant::now();
    let mut last_report = Value::Null;
    let report = loop {
        match crate::cdp_debug::evaluate_main_webview(
            app_handle,
            &expression,
            Duration::from_secs(5),
        )
        .await
        {
            Ok(report) if report.get("ready").and_then(Value::as_bool) == Some(true) => {
                break report;
            }
            Ok(report) if probe_started.elapsed() < Duration::from_secs(60) => {
                last_report = report;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(report) => {
                return Err(format!(
                    "Session undo frontend {phase} probe timed out: {report}"
                ));
            }
            Err(error)
                if (error.contains("main WebView2 window is unavailable")
                    || error.contains("0x80070057")
                    || error.contains("CDP call timed out"))
                    && probe_started.elapsed() < Duration::from_secs(60) =>
            {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => {
                return Err(format!(
                    "Session undo frontend {phase} probe failed: {error}; last state: {last_report}"
                ));
            }
        }
    };
    if report.get("buttonVisible").and_then(Value::as_bool) != Some(true)
        || report.get("buttonEnabled").and_then(Value::as_bool) != Some(true)
        || report.get("panelVisible").and_then(Value::as_bool) != Some(true)
    {
        return Err(format!(
            "Session undo frontend {phase} probe returned an invalid control state: {report}"
        ));
    }
    Ok(report)
}

async fn run_session_undo_suite(
    app_handle: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({
            "suite": CliDriverSuite::SessionUndo.as_str(),
            "project": project,
            "fixture": SESSION_UNDO_FIXTURE_RELATIVE_PATH,
        }),
    );

    let fixture_path = Path::new(project).join(SESSION_UNDO_FIXTURE_RELATIVE_PATH);
    if fixture_path.exists() {
        return Err(format!(
            "Session undo fixture already exists and was left untouched: {}",
            fixture_path.display()
        ));
    }

    let result =
        run_session_undo_suite_inner(app_handle, project, config, sink, &fixture_path).await;
    if fixture_path.exists() {
        if let Err(cleanup_error) = std::fs::remove_file(&fixture_path) {
            let primary_error = result
                .err()
                .unwrap_or_else(|| "Session undo left its fixture on disk".to_string());
            return Err(format!(
                "{primary_error}; fixture cleanup also failed for {}: {cleanup_error}",
                fixture_path.display()
            ));
        }
    }
    result
}

async fn run_session_undo_suite_inner(
    app_handle: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    fixture_path: &Path,
) -> Result<(), String> {
    let app_config = app_handle.state::<Arc<crate::config::AppConfig>>();
    app_config.set_session_undo_enabled(true)?;
    if !app_config.session_undo_enabled() {
        return Err(
            "Session undo remained disabled after enabling the isolated config".to_string(),
        );
    }

    open_and_focus_workspace_for_driver(app_handle, project).await?;
    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let runtime = registry
        .runtime_for_root(Path::new(project))
        .ok_or_else(|| format!("Workspace runtime was not registered for {project}"))?;
    let mut scoped_events = registry.event_router().subscribe();
    let session_id = create_workspace_driver_session(app_handle, 0, runtime.as_ref()).await?;
    let target = WorkspaceSuiteTarget {
        index: 0,
        project: project.to_string(),
        runtime,
        session_id,
        plugin_outcome: PluginPrepareOutcome::UpToDate,
    };
    let project_id = target.runtime.project_id().to_string();
    let undo_manager = app_handle
        .state::<crate::UndoManagerHandle>()
        .inner()
        .clone();
    let initial_entries = undo_manager.list_entries(&target.session_id).await;
    if !initial_entries.is_empty() {
        return Err(format!(
            "Expected an empty undo stack before the mock write, observed {} entries",
            initial_entries.len()
        ));
    }
    let checkout_id = target.runtime.checkout_id().to_string();
    let workspace_generation = target.runtime.generation();
    let frontend_empty = probe_session_undo_frontend(
        app_handle,
        &project_id,
        &checkout_id,
        workspace_generation,
        &target.session_id,
        "empty",
    )
    .await?;
    let launch = launch_workspace_mock_chat_with_prompt(
        app_handle,
        &target,
        format!(
            "{} Write the isolated session undo fixture.",
            crate::agent::instance::MOCK_SESSION_UNDO_FILE_SCENARIO
        ),
    )
    .await?;
    let run_result =
        wait_for_workspace_mock_chat(app_handle, &launch, &target, config.suite_timeout).await?;

    let written = std::fs::read_to_string(fixture_path)
        .map_err(|error| format!("Mock write did not create the fixture: {error}"))?;
    if written != SESSION_UNDO_FIXTURE_CONTENT {
        return Err(format!(
            "Mock write fixture content mismatch: expected {:?}, observed {:?}",
            SESSION_UNDO_FIXTURE_CONTENT, written
        ));
    }

    let entries = undo_manager.list_entries(&target.session_id).await;
    if entries.len() != 1 {
        return Err(format!(
            "Expected one undo entry after the mock write, observed {}",
            entries.len()
        ));
    }
    let entry = entries[0].clone();
    if !entry
        .changed_files
        .iter()
        .any(|file| file.path.replace('\\', "/") == SESSION_UNDO_FIXTURE_RELATIVE_PATH)
    {
        return Err(format!(
            "Undo entry did not contain fixture {}: {:?}",
            SESSION_UNDO_FIXTURE_RELATIVE_PATH, entry.changed_files
        ));
    }

    let mut saw_undo_available = false;
    while let Ok(event) = scoped_events.try_recv() {
        if event.event_name != "stream-event" {
            continue;
        }
        let Ok(envelope) =
            serde_json::from_value::<crate::commands::StreamEventEnvelope>(event.envelope.payload)
        else {
            continue;
        };
        if envelope.run_id == launch.run_id
            && matches!(
                envelope.event,
                crate::commands::StreamEvent::UndoAvailable {
                    ref session_id,
                    ref assistant_message_id,
                } if session_id == &target.session_id
                    && assistant_message_id == &entry.assistant_message_id
            )
        {
            saw_undo_available = true;
            break;
        }
    }
    if !saw_undo_available {
        return Err(
            "The mock write recorded undo state without a scoped UndoAvailable event".to_string(),
        );
    }

    let frontend_written = probe_session_undo_frontend(
        app_handle,
        &project_id,
        &checkout_id,
        workspace_generation,
        &target.session_id,
        "written",
    )
    .await?;

    let frontend_reverted = probe_session_undo_frontend(
        app_handle,
        &project_id,
        &checkout_id,
        workspace_generation,
        &target.session_id,
        "undo",
    )
    .await?;
    if fixture_path.exists() {
        return Err(format!(
            "Session undo left the fixture on disk: {}",
            fixture_path.display()
        ));
    }
    let remaining_entries = undo_manager.list_entries(&target.session_id).await;
    if !remaining_entries.is_empty() {
        return Err(format!(
            "Session undo left {} active undo entries",
            remaining_entries.len()
        ));
    }
    sink.emit(
        "suite_event",
        json!({
            "suite": CliDriverSuite::SessionUndo.as_str(),
            "line": format!(
                "PASS  session-undo: mock write recorded {}, emitted UndoAvailable, and restored the fixture",
                SESSION_UNDO_FIXTURE_RELATIVE_PATH
            ),
            "passed": 7,
            "failed": 0,
        }),
    );
    sink.emit(
        "suite_result",
        json!({
            "suite": CliDriverSuite::SessionUndo.as_str(),
            "passed": 7,
            "failed": 0,
            "sessionUndoEnabled": true,
            "fixtureRestored": true,
            "undoAvailable": true,
            "sessionId": target.session_id,
            "runId": launch.run_id,
            "assistantMessageId": entry.assistant_message_id,
            "frontend": {
                "empty": frontend_empty,
                "written": frontend_written,
                "reverted": frontend_reverted,
            },
            "run": run_result,
        }),
    );
    Ok(())
}

async fn run_workspace_switch_suite(
    app_handle: &AppHandle,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    let projects = resolve_workspace_project_paths(app_handle, config).await?;
    if projects.len() != 2 {
        return Err(format!(
            "The workspace-switch suite requires exactly two Unity projects, observed {}",
            projects.len()
        ));
    }
    sink.emit(
        "suite_start",
        json!({
            "suite": CliDriverSuite::WorkspaceSwitch.as_str(),
            "projects": projects,
        }),
    );

    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let resource_policy = app_handle
        .state::<Arc<crate::resource_policy::ResourcePolicyStore>>()
        .inner()
        .clone();
    let policy_before = resource_policy.snapshot();
    let mut workspace_limits = policy_before.limits.clone();
    workspace_limits.max_running_workspace_services = workspace_limits
        .max_running_workspace_services
        .max(projects.len());
    let policy_after = if workspace_limits == policy_before.limits {
        policy_before.clone()
    } else {
        resource_policy
            .update(workspace_limits)
            .map_err(|error| format!("Failed to prepare workspace test resource policy: {error}"))?
    };
    registry.notify_policy_changed();
    registry.converge_resource_policy().await;
    sink.emit(
        "workspace_policy",
        json!({
            "before": policy_before,
            "after": policy_after,
            "targetWorkspaceCount": projects.len(),
        }),
    );

    crate::csharp_compile::set_enabled(true).await;
    let mut scoped_events = registry.event_router().subscribe();
    let mut plugin_outcomes = Vec::with_capacity(projects.len());
    for (index, project) in projects.iter().enumerate() {
        if run_cancelled(cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        sink.emit(
            "workspace_target",
            json!({ "index": index, "project": project, "phase": "plugin" }),
        );
        plugin_outcomes.push(check_or_install_plugin(project, config.install_plugin, sink).await?);
    }

    let mut targets = Vec::with_capacity(projects.len());
    for (index, (project, plugin_outcome)) in
        projects.iter().zip(plugin_outcomes.into_iter()).enumerate()
    {
        open_and_focus_workspace_for_driver(app_handle, project).await?;
        let runtime = registry
            .runtime_for_root(Path::new(project))
            .ok_or_else(|| format!("Workspace runtime was not registered for {project}"))?;
        let session_id =
            create_workspace_driver_session(app_handle, index, runtime.as_ref()).await?;
        sink.emit(
            "workspace_registered",
            json!({
                "index": index,
                "project": project,
                "projectId": runtime.project_id(),
                "checkoutId": runtime.checkout_id(),
                "workspaceGeneration": runtime.generation(),
                "sessionId": session_id,
            }),
        );
        targets.push(WorkspaceSuiteTarget {
            index,
            project: project.clone(),
            runtime,
            session_id,
            plugin_outcome,
        });
    }
    validate_distinct_workspace_identities(&targets)?;

    let connection_futures = targets.iter().cloned().map(|target| {
        let config = config.clone();
        let sink = sink.clone();
        let mut target_cancel = cancel_rx.clone();
        async move {
            let status = ensure_connected(
                &target.project,
                &config,
                target.plugin_outcome,
                &sink,
                &mut target_cancel,
            )
            .await?;
            Ok::<_, String>((target, status))
        }
    });
    let connected = futures::future::join_all(connection_futures)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let process_ids = connected
        .iter()
        .filter_map(|(_, status)| status.editor_process_id)
        .collect::<BTreeSet<_>>();
    if process_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct Unity Editor processes, observed {:?}",
            targets.len(),
            process_ids
        ));
    }
    sink.emit(
        "workspace_connections",
        json!({
            "connected": connected.iter().map(|(target, status)| json!({
                "index": target.index,
                "project": target.project,
                "projectId": target.runtime.project_id(),
                "checkoutId": target.runtime.checkout_id(),
                "processId": status.editor_process_id,
                "channel": status.control_channel_state,
            })).collect::<Vec<_>>()
        }),
    );

    let probe_contexts = create_workspace_probe_contexts(&registry, &targets).await?;
    validate_workspace_tool_routing(app_handle, &targets, &probe_contexts).await?;
    validate_workspace_event_routing(
        app_handle,
        &registry,
        &targets,
        &mut scoped_events,
        config.suite_timeout,
    )
    .await?;

    let first = &targets[0];
    let second = &targets[1];
    open_and_focus_workspace_for_driver(app_handle, &first.project).await?;
    let first_launch = launch_workspace_mock_chat_with_prompt(
        app_handle,
        first,
        format!(
            "{} Keep this local model run active while the focused project changes.",
            crate::agent::instance::MOCK_WORKSPACE_SWITCH_HOLD_SCENARIO
        ),
    )
    .await?;
    wait_for_workspace_runs_running(
        app_handle,
        std::slice::from_ref(&first_launch),
        config.suite_timeout,
    )
    .await?;
    let first_scope_before = workspace_run_scope(app_handle, &first_launch, first)?;

    open_and_focus_workspace_for_driver(app_handle, &second.project).await?;
    if !main_pane_focuses_runtime(app_handle, &second.runtime)? {
        return Err("The frontend-equivalent workspace switch did not focus project B".to_string());
    }
    let first_status_after_switch = workspace_run_status(app_handle, &first_launch.run_id)?;
    if first_status_after_switch != "running" {
        return Err(format!(
            "Project A run ended during workspace switch with status {first_status_after_switch}"
        ));
    }
    let first_scope_after = workspace_run_scope(app_handle, &first_launch, first)?;
    if first_scope_after != first_scope_before {
        return Err("Project A run scope changed while focusing project B".to_string());
    }

    let second_launch = launch_workspace_mock_chat(app_handle, second).await?;
    wait_for_workspace_runs_running(
        app_handle,
        &[first_launch.clone(), second_launch.clone()],
        config.suite_timeout,
    )
    .await?;
    sink.emit(
        "workspace_switch_parallel_active",
        json!({
            "focusedProjectId": second.runtime.project_id(),
            "focusedCheckoutId": second.runtime.checkout_id(),
            "backgroundRun": {
                "runId": first_launch.run_id,
                "projectId": first.runtime.project_id(),
                "checkoutId": first.runtime.checkout_id(),
                "status": workspace_run_status(app_handle, &first_launch.run_id)?,
            },
            "focusedRun": {
                "runId": second_launch.run_id,
                "projectId": second.runtime.project_id(),
                "checkoutId": second.runtime.checkout_id(),
                "status": workspace_run_status(app_handle, &second_launch.run_id)?,
            },
        }),
    );

    let agent_results = futures::future::join_all([
        wait_for_workspace_mock_chat(app_handle, &first_launch, first, config.suite_timeout),
        wait_for_workspace_mock_chat(app_handle, &second_launch, second, config.suite_timeout),
    ])
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    if !main_pane_focuses_runtime(app_handle, &second.runtime)? {
        return Err("Project B focus changed while the background run completed".to_string());
    }
    let workspace_metrics = registry.metrics().await;
    if workspace_metrics.running_workspace_services < targets.len() {
        return Err(format!(
            "Expected at least {} running workspace services, observed {}",
            targets.len(),
            workspace_metrics.running_workspace_services
        ));
    }
    sink.emit(
        "workspace_switch_completed",
        json!({
            "focusedProjectId": second.runtime.project_id(),
            "focusedCheckoutId": second.runtime.checkout_id(),
            "runs": agent_results,
            "workspaces": workspace_metrics,
        }),
    );
    sink.emit(
        "suite_result",
        json!({
            "suite": CliDriverSuite::WorkspaceSwitch.as_str(),
            "passed": 6,
            "failed": 0,
            "projectIds": targets.iter().map(|target| target.runtime.project_id()).collect::<Vec<_>>(),
            "checkoutIds": targets.iter().map(|target| target.runtime.checkout_id()).collect::<Vec<_>>(),
            "unityProcessIds": process_ids,
            "backgroundRunPreserved": true,
            "mockModel": "mock/tool",
        }),
    );
    drop(probe_contexts);
    Ok(())
}

async fn resolve_workspace_project_paths(
    app_handle: &AppHandle,
    config: &CliDriverConfig,
) -> Result<Vec<String>, String> {
    let first = resolve_project_path(config.project_path.as_deref(), app_handle).await?;
    let mut projects = vec![first];
    for requested in &config.workspace_paths {
        let project = canonicalize_lossy(requested);
        if !unity_bridge::is_unity_project(&project) {
            return Err(format!("Path is not a Unity project: {project}"));
        }
        projects.push(project);
    }
    let mut unique = BTreeSet::new();
    projects.retain(|project| unique.insert(project.replace('\\', "/").to_ascii_lowercase()));
    if projects.len() < 2 {
        return Err(
            "The workspace suite requires --project plus at least one --workspace-project"
                .to_string(),
        );
    }
    Ok(projects)
}

async fn open_and_focus_workspace_for_driver(
    app_handle: &AppHandle,
    project: &str,
) -> Result<(), String> {
    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let runtime = registry.open_workspace(project)?;

    let contexts = app_handle.state::<Arc<crate::workspace_service::WindowContextRegistry>>();
    let persistence = app_handle.state::<Arc<crate::commands::WindowContextPersistence>>();
    let mutation = persistence
        .mutation
        .lock()
        .map_err(|error| format!("workspace focus persistence lock poisoned: {error}"))?;
    let intent_epoch = contexts
        .next_pane_intent_epoch("main", "main")
        .map_err(|error| error.to_string())?;
    contexts
        .focus("main", "main", Arc::clone(&runtime), intent_epoch)
        .map_err(|error| error.to_string())?;
    crate::commands::persist_window_context_recovery(app_handle, &contexts)?;
    drop(mutation);

    if crate::unity_bridge::is_unity_project(project) {
        registry
            .execution_context(
                runtime.checkout_id(),
                &[crate::workspace_service::service::ServiceKind::Unity],
            )
            .await?;
    }
    registry.converge_resource_policy().await;
    if let Ok(data_dir) = crate::commands::resolve_runtime_storage_dir(app_handle) {
        crate::commands::save_recent_dir_pub(&data_dir, &runtime.root().to_string_lossy());
    }
    Ok(())
}

fn main_pane_focuses_runtime(
    app_handle: &AppHandle,
    runtime: &crate::workspace_service::WorkspaceRuntime,
) -> Result<bool, String> {
    let contexts = app_handle.state::<Arc<crate::workspace_service::WindowContextRegistry>>();
    Ok(contexts
        .pane("main", "main")
        .map_err(|error| error.to_string())?
        .is_some_and(|context| {
            context.focused_checkout_id == *runtime.checkout_id()
                && context.workspace_generation == runtime.generation()
        }))
}

async fn create_workspace_driver_session(
    app_handle: &AppHandle,
    index: usize,
    runtime: &crate::workspace_service::WorkspaceRuntime,
) -> Result<String, String> {
    crate::commands::create_session(
        format!("Workspace driver target {index}"),
        None,
        Some("chat".to_string()),
        Some(crate::agent::definition::DEFAULT_AGENT_ID.to_string()),
        crate::workspace_service::WorkspaceRef::for_runtime(runtime),
        app_handle.state(),
        app_handle.state(),
    )
    .await
    .map_err(|error| format!("Failed to create workspace driver session: {error}"))
}

fn validate_workspace_identities(targets: &[WorkspaceSuiteTarget]) -> Result<(), String> {
    let project_ids = targets
        .iter()
        .map(|target| target.runtime.project_id().to_string())
        .collect::<BTreeSet<_>>();
    let checkout_ids = targets
        .iter()
        .map(|target| target.runtime.checkout_id().to_string())
        .collect::<BTreeSet<_>>();
    if project_ids.len() != 1 {
        return Err(format!(
            "Copied project and worktree did not share one projectId: {:?}",
            project_ids
        ));
    }
    if checkout_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct checkoutIds, observed {:?}",
            targets.len(),
            checkout_ids
        ));
    }
    Ok(())
}

fn validate_distinct_workspace_identities(targets: &[WorkspaceSuiteTarget]) -> Result<(), String> {
    let project_ids = targets
        .iter()
        .map(|target| target.runtime.project_id().to_string())
        .collect::<BTreeSet<_>>();
    let checkout_ids = targets
        .iter()
        .map(|target| target.runtime.checkout_id().to_string())
        .collect::<BTreeSet<_>>();
    if project_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct projectIds, observed {:?}",
            targets.len(),
            project_ids
        ));
    }
    if checkout_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct checkoutIds, observed {:?}",
            targets.len(),
            checkout_ids
        ));
    }
    Ok(())
}

async fn create_workspace_probe_contexts(
    registry: &Arc<crate::workspace_service::ProjectRegistry>,
    targets: &[WorkspaceSuiteTarget],
) -> Result<Vec<Arc<crate::workspace_service::AgentExecutionContext>>, String> {
    let mut contexts = Vec::with_capacity(targets.len());
    let mut service_ids = BTreeSet::new();
    for target in targets {
        let execution = registry
            .execution_context(
                target.runtime.checkout_id(),
                &[crate::workspace_service::service::ServiceKind::Unity],
            )
            .await?;
        let binding = execution
            .binding(crate::workspace_service::service::ServiceKind::Unity)
            .ok_or_else(|| format!("Unity binding missing for {}", target.project))?;
        service_ids.insert(binding.service_instance_id.to_string());
        contexts.push(execution);
    }
    if service_ids.len() != targets.len() {
        return Err(format!(
            "Expected {} distinct Unity service instances, observed {:?}",
            targets.len(),
            service_ids
        ));
    }
    Ok(contexts)
}

async fn validate_workspace_tool_routing(
    app_handle: &AppHandle,
    targets: &[WorkspaceSuiteTarget],
    contexts: &[Arc<crate::workspace_service::AgentExecutionContext>],
) -> Result<(), String> {
    let tool_registry = app_handle
        .state::<Arc<crate::tool::ToolRegistry>>()
        .inner()
        .clone();
    for (target, execution) in targets.iter().zip(contexts.iter()) {
        let marker = format!(
            "workspace-driver:{}:{}",
            target.index,
            target.runtime.checkout_id()
        );
        let marker_path = target
            .runtime
            .root()
            .join("Library")
            .join("Locus")
            .join("workspace-driver-marker.txt");
        if let Some(parent) = marker_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Failed to create workspace marker directory: {error}"))?;
        }
        std::fs::write(&marker_path, &marker)
            .map_err(|error| format!("Failed to write workspace marker: {error}"))?;
        let result = tool_registry
            .execute_with_context(
                "read",
                &json!({ "filePath": "Library/Locus/workspace-driver-marker.txt" }),
                crate::tool::ToolExecutionContext {
                    app_handle: Some(app_handle.clone()),
                    execution: Some(execution.clone()),
                    ..Default::default()
                },
            )
            .await;
        let _ = std::fs::remove_file(&marker_path);
        if result.is_error || !result.output.contains(&marker) {
            return Err(format!(
                "Checkout-scoped read was misrouted for {}: {}",
                target.project, result.output
            ));
        }
    }
    Ok(())
}

async fn validate_workspace_event_routing(
    app_handle: &AppHandle,
    registry: &Arc<crate::workspace_service::ProjectRegistry>,
    targets: &[WorkspaceSuiteTarget],
    receiver: &mut tokio::sync::mpsc::UnboundedReceiver<
        crate::workspace_service::event::RoutedWorkspaceEvent,
    >,
    timeout: Duration,
) -> Result<(), String> {
    for target in targets {
        let outcome = registry.event_router().publish_for_scope(
            app_handle,
            &crate::workspace_service::event::WorkspaceEventScope::for_runtime(&target.runtime),
            "workspace-driver-probe",
            json!({ "index": target.index }),
        );
        if outcome != crate::workspace_service::event::WorkspaceEventPublishOutcome::PublishedScoped
        {
            return Err(format!(
                "Workspace probe was not published with its scoped envelope: {}",
                target.project
            ));
        }
    }
    let expected = targets
        .iter()
        .map(|target| target.runtime.checkout_id().to_string())
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    tokio::time::timeout(timeout, async {
        while observed.len() < expected.len() {
            let event = receiver
                .recv()
                .await
                .ok_or_else(|| "Workspace event subscriber closed".to_string())?;
            if event.event_name == "workspace-driver-probe" {
                observed.insert(event.envelope.checkout_id.to_string());
            }
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "Timed out waiting for scoped workspace probe events".to_string())??;
    if observed != expected {
        return Err(format!(
            "Scoped event checkoutIds differ: expected {:?}, observed {:?}",
            expected, observed
        ));
    }
    Ok(())
}

async fn compile_workspace_probe(target: &WorkspaceSuiteTarget) -> Result<Value, String> {
    let params = crate::csharp_compile::params::get_params(&target.project).await?;
    let scope = params
        .scope_id
        .as_ref()
        .ok_or_else(|| format!("Compile scope missing for {}", target.project))?;
    let code = format!(
        "var workspaceDriverTarget = {}; System.Console.WriteLine(workspaceDriverTarget);",
        target.index
    );
    let compiled = crate::csharp_compile::compile_snippet(&params, &code, false, false).await?;
    let assembly = compiled.map_err(|failure| {
        format!(
            "Compile probe failed for {} at {}: {}",
            target.project, failure.stage, failure.message
        )
    })?;
    Ok(json!({
        "index": target.index,
        "checkoutId": scope.checkout_id,
        "workspaceGeneration": scope.workspace_generation,
        "serviceGeneration": scope.service_generation,
        "editorSessionId": scope.unity_editor_session_id,
        "assemblyName": assembly.assembly_name,
    }))
}

async fn launch_workspace_mock_chat(
    app_handle: &AppHandle,
    target: &WorkspaceSuiteTarget,
) -> Result<crate::commands::ChatLaunch, String> {
    launch_workspace_mock_chat_with_prompt(
        app_handle,
        target,
        format!(
            "Run the local workspace routing probe for checkout {}.",
            target.runtime.checkout_id()
        ),
    )
    .await
}

async fn launch_workspace_mock_chat_with_prompt(
    app_handle: &AppHandle,
    target: &WorkspaceSuiteTarget,
    prompt: String,
) -> Result<crate::commands::ChatLaunch, String> {
    crate::commands::chat(
        Some(target.session_id.clone()),
        Some(crate::workspace_service::WorkspaceRef::for_runtime(
            target.runtime.as_ref(),
        )),
        prompt,
        Some(false),
        None,
        Some(crate::agent::definition::DEFAULT_AGENT_ID.to_string()),
        None,
        Some("mock/tool".to_string()),
        None,
        Some(false),
        Some(false),
        None,
        None,
        None,
        Some("build".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        app_handle.clone(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
        app_handle.state(),
    )
    .await
    .map_err(|error| format!("Failed to launch local mock model: {error}"))
}

fn workspace_run_status(app_handle: &AppHandle, run_id: &str) -> Result<String, String> {
    app_handle
        .state::<Arc<crate::session::store::SessionStore>>()
        .run_by_id(run_id)?
        .map(|run| run.status)
        .ok_or_else(|| format!("Mock run disappeared: {run_id}"))
}

fn workspace_run_scope(
    app_handle: &AppHandle,
    launch: &crate::commands::ChatLaunch,
    target: &WorkspaceSuiteTarget,
) -> Result<Value, String> {
    let scope = app_handle
        .state::<Arc<crate::session::store::SessionStore>>()
        .get_run_scope(&launch.run_id)?
        .ok_or_else(|| format!("Mock run scope missing: {}", launch.run_id))?;
    if scope.project_id != target.runtime.project_id().as_str()
        || scope.checkout_id != target.runtime.checkout_id().as_str()
        || !scope
            .service_bindings
            .iter()
            .any(|binding| binding.service_kind == "unity")
    {
        return Err(format!(
            "Mock run scope mismatch for {}: {:?}",
            target.project, scope
        ));
    }
    Ok(json!(scope))
}

async fn wait_for_workspace_runs_running(
    app_handle: &AppHandle,
    launches: &[crate::commands::ChatLaunch],
    timeout: Duration,
) -> Result<(), String> {
    tokio::time::timeout(timeout, async {
        loop {
            let statuses = launches
                .iter()
                .map(|launch| {
                    workspace_run_status(app_handle, &launch.run_id)
                        .map(|status| (launch.run_id.as_str(), status))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if statuses.iter().all(|(_, status)| status == "running") {
                return Ok(());
            }
            if let Some((run_id, status)) = statuses
                .iter()
                .find(|(_, status)| matches!(status.as_str(), "done" | "error" | "cancelled"))
            {
                return Err(format!(
                    "Mock run {run_id} reached terminal status {status} before the parallel checkpoint"
                ));
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| "Timed out waiting for workspace mock runs to become active".to_string())?
}

async fn wait_for_workspace_mock_chat(
    app_handle: &AppHandle,
    launch: &crate::commands::ChatLaunch,
    target: &WorkspaceSuiteTarget,
    timeout: Duration,
) -> Result<Value, String> {
    let store = app_handle
        .state::<Arc<crate::session::store::SessionStore>>()
        .inner()
        .clone();
    let run = tokio::time::timeout(timeout, async {
        loop {
            let run = store
                .run_by_id(&launch.run_id)?
                .ok_or_else(|| format!("Mock run disappeared: {}", launch.run_id))?;
            if matches!(run.status.as_str(), "done" | "error" | "cancelled") {
                return Ok::<_, String>(run);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .map_err(|_| format!("Mock run timed out: {}", launch.run_id))??;
    if run.status != "done" {
        return Err(format!(
            "Mock run {} ended as {}: {}",
            launch.run_id,
            run.status,
            run.error_message.unwrap_or_default()
        ));
    }
    let scope = store
        .get_run_scope(&launch.run_id)?
        .ok_or_else(|| format!("Mock run scope missing: {}", launch.run_id))?;
    if scope.project_id != target.runtime.project_id().as_str()
        || scope.checkout_id != target.runtime.checkout_id().as_str()
        || !scope
            .service_bindings
            .iter()
            .any(|binding| binding.service_kind == "unity")
    {
        return Err(format!(
            "Mock run scope mismatch for {}: {:?}",
            target.project, scope
        ));
    }
    let messages = store.get_messages(&launch.session_id)?;
    let tool_messages = messages
        .iter()
        .filter(|message| message.role == crate::session::models::MessageRole::Tool)
        .count();
    let assistant_messages = messages
        .iter()
        .filter(|message| message.role == crate::session::models::MessageRole::Assistant)
        .count();
    if tool_messages == 0 || assistant_messages == 0 {
        return Err(format!(
            "Mock tool pipeline did not complete for {}: tool={}, assistant={}",
            target.project, tool_messages, assistant_messages
        ));
    }
    Ok(json!({
        "index": target.index,
        "sessionId": launch.session_id,
        "runId": launch.run_id,
        "projectId": scope.project_id,
        "checkoutId": scope.checkout_id,
        "workspaceGeneration": scope.workspace_generation,
        "serviceBindings": scope.service_bindings,
        "toolMessages": tool_messages,
        "assistantMessages": assistant_messages,
    }))
}

async fn wait_for_workspace_lsp_entries(
    targets: &[WorkspaceSuiteTarget],
    timeout: Duration,
) -> Result<crate::csharp_lsp::LspProcessPoolMetrics, String> {
    let expected = targets
        .iter()
        .map(|target| target.runtime.checkout_id().to_string())
        .collect::<BTreeSet<_>>();
    tokio::time::timeout(timeout, async {
        loop {
            let metrics = crate::csharp_lsp::pool_metrics().await;
            let observed = metrics
                .entries
                .iter()
                .map(|entry| entry.checkout_id.clone())
                .collect::<BTreeSet<_>>();
            if expected.is_subset(&observed) {
                return metrics;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| format!("Timed out waiting for LSP checkout entries: {:?}", expected))
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
    if !workspace_status.package_supported {
        return Err(format!(
            "Unity Test suite requires com.unity.test-framework {} or newer (found {})",
            crate::workspace::UNITY_TEST_FRAMEWORK_MIN_VERSION,
            workspace_status
                .package_version
                .as_deref()
                .unwrap_or("unknown version")
        ));
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
            CliDriverSuite::NativeBridge
                | CliDriverSuite::ParallelEditRefresh
                | CliDriverSuite::RecompileImport
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
    app_handle: &AppHandle,
) -> Result<String, String> {
    let raw = match requested.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value.to_string(),
        None => {
            let contexts =
                app_handle.state::<Arc<crate::workspace_service::WindowContextRegistry>>();
            let registry = app_handle.state::<Arc<crate::workspace_service::ProjectRegistry>>();
            contexts
                .pane("main", "main")
                .map_err(|error| error.to_string())?
                .and_then(|context| registry.runtime(&context.focused_checkout_id))
                .map(|runtime| runtime.root().display().to_string())
                .unwrap_or_default()
        }
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

async fn set_workspace_for_driver(app_handle: &AppHandle, project: &str) -> Result<(), String> {
    open_and_focus_workspace_for_driver(app_handle, project).await
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecompileImportObservation {
    case_name: String,
    converged_without_import: bool,
    elapsed_ms: u128,
    request_result: String,
    last_probe: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecompilePipelineObservation {
    baseline_elapsed_ms: u128,
    baseline_result: String,
    queued_paths: usize,
    elapsed_ms: u128,
    result: String,
    probe: String,
    asmdef_elapsed_ms: u128,
    asmdef_result: String,
    delete_elapsed_ms: u128,
    delete_result: String,
    no_op_elapsed_ms: u128,
    no_op_result: String,
}

fn recompile_import_source(type_name: &str, answer: i32) -> String {
    format!(
        "public static class {type_name}\n{{\n    public static int Answer() => {answer};\n}}\n"
    )
}

fn recompile_import_asmdef(name: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "name": name,
        "rootNamespace": "",
        "references": [],
        "includePlatforms": ["Editor"],
        "excludePlatforms": [],
        "allowUnsafeCode": false,
        "overrideReferences": false,
        "precompiledReferences": [],
        "autoReferenced": false,
        "defineConstraints": [],
        "versionDefines": [],
        "noEngineReferences": false,
    }))
    .unwrap_or_default()
        + "\n"
}

fn recompile_import_type_probe(type_name: &str, marker: &str, predicate: &str) -> String {
    format!(
        r#"var t = System.AppDomain.CurrentDomain.GetAssemblies().Select(a => a.GetType("{type_name}")).FirstOrDefault(x => x != null); bool matched = {predicate}; print("{marker}:" + (matched ? "yes" : "no") + ":" + (t == null ? "missing" : t.Assembly.GetName().Name));"#
    )
}

async fn observe_recompile_import_marker(
    project: &str,
    code: &str,
    expected_marker: &str,
    timeout: Duration,
    poll_interval: Duration,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(bool, u128, String), String> {
    let started = Instant::now();
    loop {
        if run_cancelled(cancel_rx) {
            return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
        }
        let last_probe = match execute_capture(project, code).await {
            Ok(output) if output.contains(expected_marker) => {
                return Ok((true, started.elapsed().as_millis(), clip(&output, 240)))
            }
            Ok(output) => clip(&output, 240),
            Err(error) => clip(&error, 240),
        };
        if started.elapsed() >= timeout {
            return Ok((false, started.elapsed().as_millis(), last_probe));
        }
        tokio::select! {
            _ = tokio::time::sleep(poll_interval.min(Duration::from_secs(1))) => {}
            _ = cancel_rx.changed() => {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
        }
    }
}

async fn request_script_compilation_only(project: &str) -> String {
    match execute_capture(
        project,
        r#"UnityEditor.Compilation.CompilationPipeline.RequestScriptCompilation(); print("RIR_REQUEST:accepted");"#,
    )
    .await
    {
        Ok(output) => format!("ok:{}", clip(&output, 160)),
        Err(error) => format!("error:{}", clip(&error, 160)),
    }
}

async fn refresh_recompile_import_fixture(project: &str) -> String {
    match execute_capture(
        project,
        r#"AssetDatabase.Refresh(); print("RIR_REFRESH:requested");"#,
    )
    .await
    {
        Ok(output) => format!("ok:{}", clip(&output, 160)),
        Err(error) => format!("error:{}", clip(&error, 160)),
    }
}

async fn cleanup_recompile_import_fixture(
    project: &str,
    fixture_asset_dir: &str,
    fixture_dir: &Path,
) -> Result<(), String> {
    let cleanup_code = format!(
        r#"AssetDatabase.DeleteAsset("{fixture_asset_dir}"); AssetDatabase.Refresh(); print("RIR_CLEANUP:requested");"#
    );
    let _ = execute_capture(project, &cleanup_code).await;

    let project_assets = Path::new(project).join("Assets");
    let safe_name = fixture_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("LocusRecompileImportSelfTest_"));
    if !fixture_dir.starts_with(&project_assets) || !safe_name {
        return Err(format!(
            "refused unsafe recompile-import cleanup path: {}",
            fixture_dir.display()
        ));
    }

    match tokio::fs::remove_dir_all(fixture_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("fixture directory cleanup failed: {error}")),
    }
    let fixture_meta = fixture_dir.with_extension("meta");
    match tokio::fs::remove_file(&fixture_meta).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("fixture meta cleanup failed: {error}")),
    }
    let _ = refresh_recompile_import_fixture(project).await;
    Ok(())
}

/// Observe which externally-written script changes reach Unity's compilation
/// graph through RequestScriptCompilation alone while auto refresh is held.
/// This is intentionally a fact-finding suite: each case reports its observed
/// convergence result, while infrastructure and cleanup failures still fail the
/// suite. The result is used to keep Locus's import policy evidence-based across
/// Unity versions.
async fn run_recompile_import_suite(
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({ "suite": suite.as_str(), "project": project }),
    );
    crate::csharp_compile::set_enabled(true).await;
    crate::csharp_compile::warm_up_in_background();

    let token = uuid::Uuid::new_v4().simple().to_string();
    let short = &token[..8];
    let fixture_asset_dir = format!("Assets/LocusRecompileImportSelfTest_{token}");
    let fixture_dir = Path::new(project)
        .join("Assets")
        .join(format!("LocusRecompileImportSelfTest_{token}"));
    let asmdef_path = fixture_dir.join("Locus.RecompileImport.asmdef");
    let asm_a = format!("Locus.RecompileImport.A.{short}");
    let asm_b = format!("Locus.RecompileImport.B.{short}");
    let asmdef_a = recompile_import_asmdef(&asm_a);
    let asmdef_b = recompile_import_asmdef(&asm_b);
    let existing_type = format!("LocusRirExisting_{short}");
    let new_type = format!("LocusRirNew_{short}");
    let deleted_type = format!("LocusRirDeleted_{short}");
    let assembly_type = format!("LocusRirAssembly_{short}");
    let existing_path = fixture_dir.join("Existing.cs");
    let new_path = fixture_dir.join("New.cs");
    let deleted_path = fixture_dir.join("Deleted.cs");
    let deleted_meta_path = deleted_path.with_extension("cs.meta");
    let assembly_path = fixture_dir.join("Assembly.cs");
    let existing_source_a = recompile_import_source(&existing_type, 1);
    let existing_source_b = recompile_import_source(&existing_type, 2);
    let deleted_source = recompile_import_source(&deleted_type, 1);
    let assembly_source = recompile_import_source(&assembly_type, 1);

    tokio::fs::create_dir_all(&fixture_dir)
        .await
        .map_err(|error| format!("failed to create recompile-import fixture: {error}"))?;
    for (path, content) in [
        (&asmdef_path, asmdef_a.as_str()),
        (&existing_path, existing_source_a.as_str()),
        (&deleted_path, deleted_source.as_str()),
        (&assembly_path, assembly_source.as_str()),
    ] {
        tokio::fs::write(path, content)
            .await
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }

    let cleanup_on_error = |error: String| async {
        let _ = cleanup_recompile_import_fixture(project, &fixture_asset_dir, &fixture_dir).await;
        error
    };
    let _ = refresh_recompile_import_fixture(project).await;
    let baseline_probe = format!(
        r#"var names = new[] {{ "{existing_type}", "{deleted_type}", "{assembly_type}" }}; bool present = names.All(name => System.AppDomain.CurrentDomain.GetAssemblies().Any(a => a.GetType(name) != null)); print("RIR_BASELINE:" + (present ? "yes" : "no"));"#
    );
    let (baseline_ready, _, baseline_last) = observe_recompile_import_marker(
        project,
        &baseline_probe,
        "RIR_BASELINE:yes",
        recompile_wait(config),
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !baseline_ready {
        return Err(cleanup_on_error(format!(
            "recompile-import baseline did not load: {baseline_last}"
        ))
        .await);
    }
    let deleted_meta = tokio::fs::read(&deleted_meta_path)
        .await
        .map_err(|error| format!("failed to read generated delete fixture meta: {error}"))?;

    let environment = execute_capture(
        project,
        r#"print("RIR_ENV:directoryMonitoring=" + AssetDatabase.IsDirectoryMonitoringEnabled());"#,
    )
    .await
    .unwrap_or_else(|error| format!("error:{error}"));
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!("INFO  recompile-import: {}", clip(&environment, 200)),
            "passed": 0,
            "failed": 0,
        }),
    );

    let case_timeout = recompile_wait(config).min(Duration::from_secs(45));
    let mut observations = Vec::new();

    // Existing .cs content modification.
    let owner = format!("rir-existing-{token}");
    unity_bridge::begin_edit_session(project, &owner).await?;
    tokio::fs::write(&existing_path, &existing_source_b)
        .await
        .map_err(|error| format!("failed to modify existing fixture: {error}"))?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let request_result = request_script_compilation_only(project).await;
    let existing_probe = recompile_import_type_probe(
        &existing_type,
        "RIR_EXISTING",
        "t != null && (int)t.GetMethod(\"Answer\").Invoke(null, null) == 2",
    );
    let (converged, elapsed_ms, last_probe) = observe_recompile_import_marker(
        project,
        &existing_probe,
        "RIR_EXISTING:yes",
        case_timeout,
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !converged {
        tokio::fs::write(&existing_path, &existing_source_a)
            .await
            .map_err(|error| format!("failed to restore existing fixture: {error}"))?;
    }
    let _ = end_edit_session_for_cleanup(project, &owner).await;
    if !converged {
        let _ = refresh_recompile_import_fixture(project).await;
    }
    observations.push(RecompileImportObservation {
        case_name: "existing_cs".to_string(),
        converged_without_import: converged,
        elapsed_ms,
        request_result,
        last_probe,
    });

    // Brand-new .cs file with no .meta.
    let owner = format!("rir-new-{token}");
    unity_bridge::begin_edit_session(project, &owner).await?;
    tokio::fs::write(&new_path, recompile_import_source(&new_type, 1))
        .await
        .map_err(|error| format!("failed to create new fixture: {error}"))?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let request_result = request_script_compilation_only(project).await;
    let new_probe = recompile_import_type_probe(&new_type, "RIR_NEW", "t != null");
    let (converged, elapsed_ms, last_probe) = observe_recompile_import_marker(
        project,
        &new_probe,
        "RIR_NEW:yes",
        case_timeout,
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !converged {
        let _ = tokio::fs::remove_file(&new_path).await;
    }
    let _ = end_edit_session_for_cleanup(project, &owner).await;
    if !converged {
        let _ = refresh_recompile_import_fixture(project).await;
    }
    observations.push(RecompileImportObservation {
        case_name: "new_cs".to_string(),
        converged_without_import: converged,
        elapsed_ms,
        request_result,
        last_probe,
    });

    // Externally deleted .cs and its .meta.
    let owner = format!("rir-delete-{token}");
    unity_bridge::begin_edit_session(project, &owner).await?;
    tokio::fs::remove_file(&deleted_path)
        .await
        .map_err(|error| format!("failed to delete fixture source: {error}"))?;
    tokio::fs::remove_file(&deleted_meta_path)
        .await
        .map_err(|error| format!("failed to delete fixture meta: {error}"))?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let request_result = request_script_compilation_only(project).await;
    let deleted_probe = recompile_import_type_probe(&deleted_type, "RIR_DELETE", "t == null");
    let (converged, elapsed_ms, last_probe) = observe_recompile_import_marker(
        project,
        &deleted_probe,
        "RIR_DELETE:yes",
        case_timeout,
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !converged {
        tokio::fs::write(&deleted_path, &deleted_source)
            .await
            .map_err(|error| format!("failed to restore deleted fixture source: {error}"))?;
        tokio::fs::write(&deleted_meta_path, &deleted_meta)
            .await
            .map_err(|error| format!("failed to restore deleted fixture meta: {error}"))?;
    }
    let _ = end_edit_session_for_cleanup(project, &owner).await;
    if !converged {
        let _ = refresh_recompile_import_fixture(project).await;
    }
    observations.push(RecompileImportObservation {
        case_name: "deleted_cs".to_string(),
        converged_without_import: converged,
        elapsed_ms,
        request_result,
        last_probe,
    });

    // Assembly definition content modification.
    let owner = format!("rir-asmdef-{token}");
    unity_bridge::begin_edit_session(project, &owner).await?;
    tokio::fs::write(&asmdef_path, &asmdef_b)
        .await
        .map_err(|error| format!("failed to modify asmdef fixture: {error}"))?;
    tokio::time::sleep(Duration::from_millis(750)).await;
    let request_result = request_script_compilation_only(project).await;
    let assembly_probe = recompile_import_type_probe(
        &assembly_type,
        "RIR_ASMDEF",
        &format!("t != null && t.Assembly.GetName().Name == \"{asm_b}\""),
    );
    let (converged, elapsed_ms, last_probe) = observe_recompile_import_marker(
        project,
        &assembly_probe,
        "RIR_ASMDEF:yes",
        case_timeout,
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !converged {
        tokio::fs::write(&asmdef_path, &asmdef_a)
            .await
            .map_err(|error| format!("failed to restore asmdef fixture: {error}"))?;
    }
    let _ = end_edit_session_for_cleanup(project, &owner).await;
    if !converged {
        let _ = refresh_recompile_import_fixture(project).await;
    }
    observations.push(RecompileImportObservation {
        case_name: "asmdef".to_string(),
        converged_without_import: converged,
        elapsed_ms,
        request_result,
        last_probe,
    });

    // Establish the process-local journal baseline. A fresh Locus process is
    // intentionally Unverified, so its first recompile must use one full
    // refresh before later known paths qualify for targeted import.
    let baseline_started = Instant::now();
    let journal_baseline_result = unity_bridge::recompile_and_wait(project).await;
    let baseline_elapsed_ms = baseline_started.elapsed().as_millis();
    if let Err(error) = journal_baseline_result.as_ref() {
        return Err(cleanup_on_error(format!(
            "journal baseline recompile failed after {baseline_elapsed_ms}ms: {error}"
        ))
        .await);
    }
    if !journal_baseline_result
        .as_ref()
        .is_ok_and(|result| result.contains("- asset_sync: full"))
    {
        return Err(cleanup_on_error(format!(
            "fresh journal baseline did not use full sync: {}",
            describe_result(&journal_baseline_result, "- asset_sync: full")
        ))
        .await);
    }

    // Exercise Locus's real edit-session -> queued paths -> recompile path with
    // enough files to expose accidental per-path Asset Pipeline refreshes.
    let batch_owner = format!("rir-pipeline-{token}");
    let batch_count = 64usize;
    let mut batch_asset_paths = Vec::with_capacity(batch_count);
    unity_bridge::begin_edit_session(project, &batch_owner).await?;
    for index in 0..batch_count {
        let type_name = format!("LocusRirPipeline_{short}_{index}");
        let file_name = format!("Pipeline{index}.cs");
        tokio::fs::write(
            fixture_dir.join(&file_name),
            recompile_import_source(&type_name, index as i32),
        )
        .await
        .map_err(|error| format!("failed to write pipeline fixture {file_name}: {error}"))?;
        batch_asset_paths.push(format!("{fixture_asset_dir}/{file_name}"));
    }
    unity_bridge::import_assets(project, &batch_asset_paths)
        .await
        .map_err(|error| format!("failed to queue pipeline fixture assets: {error}"))?;

    let pipeline_started = Instant::now();
    let pipeline_result = unity_bridge::recompile_and_wait(project).await;
    let pipeline_elapsed_ms = pipeline_started.elapsed().as_millis();
    if let Err(error) = pipeline_result.as_ref() {
        let _ = end_edit_session_for_cleanup(project, &batch_owner).await;
        return Err(cleanup_on_error(format!(
            "real Locus recompile failed after {pipeline_elapsed_ms}ms: {error}"
        ))
        .await);
    }
    if !pipeline_result
        .as_ref()
        .is_ok_and(|result| result.contains("- asset_sync: targeted"))
    {
        return Err(cleanup_on_error(format!(
            "known pipeline paths did not use targeted sync: {}",
            describe_result(&pipeline_result, "- asset_sync: targeted")
        ))
        .await);
    }
    let pipeline_probe_code = format!(
        r#"var first = System.AppDomain.CurrentDomain.GetAssemblies().Any(a => a.GetType("LocusRirPipeline_{short}_0") != null); var last = System.AppDomain.CurrentDomain.GetAssemblies().Any(a => a.GetType("LocusRirPipeline_{short}_63") != null); print("RIR_PIPELINE:" + (first && last ? "yes" : "no"));"#
    );
    let (pipeline_converged, _, pipeline_probe) = observe_recompile_import_marker(
        project,
        &pipeline_probe_code,
        "RIR_PIPELINE:yes",
        recompile_wait(config),
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !pipeline_converged {
        return Err(cleanup_on_error(format!(
            "real Locus recompile returned without loading all queued files: {pipeline_probe}"
        ))
        .await);
    }

    // Assembly graph edits remain eligible for the targeted path when the
    // journal knows the exact existing asmdef that changed.
    let asm_c = format!("Locus.RecompileImport.C.{short}");
    let asmdef_c = recompile_import_asmdef(&asm_c);
    let asmdef_owner = format!("rir-targeted-asmdef-{token}");
    unity_bridge::begin_edit_session(project, &asmdef_owner).await?;
    tokio::fs::write(&asmdef_path, &asmdef_c)
        .await
        .map_err(|error| format!("failed to write targeted asmdef fixture: {error}"))?;
    let asmdef_asset_path = format!("{fixture_asset_dir}/Locus.RecompileImport.asmdef");
    unity_bridge::import_assets(project, std::slice::from_ref(&asmdef_asset_path))
        .await
        .map_err(|error| format!("failed to queue targeted asmdef fixture: {error}"))?;
    let asmdef_started = Instant::now();
    let asmdef_result = unity_bridge::recompile_and_wait(project).await;
    let asmdef_elapsed_ms = asmdef_started.elapsed().as_millis();
    if let Err(error) = asmdef_result.as_ref() {
        let _ = end_edit_session_for_cleanup(project, &asmdef_owner).await;
        return Err(cleanup_on_error(format!(
            "targeted asmdef recompile failed after {asmdef_elapsed_ms}ms: {error}"
        ))
        .await);
    }
    if !asmdef_result
        .as_ref()
        .is_ok_and(|result| result.contains("- asset_sync: targeted"))
    {
        return Err(cleanup_on_error(format!(
            "known asmdef path did not use targeted sync: {}",
            describe_result(&asmdef_result, "- asset_sync: targeted")
        ))
        .await);
    }
    let asmdef_probe = recompile_import_type_probe(
        &assembly_type,
        "RIR_TARGETED_ASMDEF",
        &format!("t != null && t.Assembly.GetName().Name == \"{asm_c}\""),
    );
    let (asmdef_converged, _, asmdef_last_probe) = observe_recompile_import_marker(
        project,
        &asmdef_probe,
        "RIR_TARGETED_ASMDEF:yes",
        recompile_wait(config),
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !asmdef_converged {
        return Err(cleanup_on_error(format!(
            "targeted asmdef recompile returned before the assembly graph converged: {asmdef_last_probe}"
        ))
        .await);
    }

    // A known deletion fails closed to one full refresh, which removes the
    // stale AssetDatabase row before Unity evaluates its incremental graph.
    let delete_owner = format!("rir-delete-full-{token}");
    unity_bridge::begin_edit_session(project, &delete_owner).await?;
    tokio::fs::remove_file(&assembly_path)
        .await
        .map_err(|error| format!("failed to remove full-sync fixture source: {error}"))?;
    match tokio::fs::remove_file(assembly_path.with_extension("cs.meta")).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(cleanup_on_error(format!(
                "failed to remove full-sync fixture meta: {error}"
            ))
            .await)
        }
    }
    let assembly_asset_path = format!("{fixture_asset_dir}/Assembly.cs");
    unity_bridge::import_assets(project, std::slice::from_ref(&assembly_asset_path))
        .await
        .map_err(|error| format!("failed to queue deleted fixture path: {error}"))?;
    let delete_started = Instant::now();
    let delete_result = unity_bridge::recompile_and_wait(project).await;
    let delete_elapsed_ms = delete_started.elapsed().as_millis();
    if let Err(error) = delete_result.as_ref() {
        let _ = end_edit_session_for_cleanup(project, &delete_owner).await;
        return Err(cleanup_on_error(format!(
            "delete full-sync recompile failed after {delete_elapsed_ms}ms: {error}"
        ))
        .await);
    }
    if !delete_result
        .as_ref()
        .is_ok_and(|result| result.contains("- asset_sync: full"))
    {
        return Err(cleanup_on_error(format!(
            "deleted compile input did not fail closed to full sync: {}",
            describe_result(&delete_result, "- asset_sync: full")
        ))
        .await);
    }
    let deleted_assembly_probe =
        recompile_import_type_probe(&assembly_type, "RIR_DELETE_FULL", "t == null");
    let (delete_converged, _, delete_last_probe) = observe_recompile_import_marker(
        project,
        &deleted_assembly_probe,
        "RIR_DELETE_FULL:yes",
        recompile_wait(config),
        config.poll_interval,
        cancel_rx,
    )
    .await?;
    if !delete_converged {
        return Err(cleanup_on_error(format!(
            "delete full-sync recompile retained the removed type: {delete_last_probe}"
        ))
        .await);
    }

    let no_op_started = Instant::now();
    let no_op_result = unity_bridge::recompile_and_wait(project).await;
    let no_op_elapsed_ms = no_op_started.elapsed().as_millis();
    if let Err(error) = no_op_result.as_ref() {
        return Err(cleanup_on_error(format!(
            "no-op Locus recompile failed after {no_op_elapsed_ms}ms: {error}"
        ))
        .await);
    }
    if !no_op_result
        .as_ref()
        .is_ok_and(|result| result.contains("- asset_sync: none"))
    {
        return Err(cleanup_on_error(format!(
            "healthy no-op recompile did not skip asset synchronization: {}",
            describe_result(&no_op_result, "- asset_sync: none")
        ))
        .await);
    }
    let pipeline_observation = RecompilePipelineObservation {
        baseline_elapsed_ms,
        baseline_result: journal_baseline_result.unwrap_or_default(),
        queued_paths: batch_asset_paths.len(),
        elapsed_ms: pipeline_elapsed_ms,
        result: pipeline_result.unwrap_or_default(),
        probe: pipeline_probe,
        asmdef_elapsed_ms,
        asmdef_result: asmdef_result.unwrap_or_default(),
        delete_elapsed_ms,
        delete_result: delete_result.unwrap_or_default(),
        no_op_elapsed_ms,
        no_op_result: no_op_result.unwrap_or_default(),
    };

    for observation in &observations {
        sink.emit(
            "suite_event",
            json!({
                "suite": suite.as_str(),
                "line": format!(
                    "OBSERVE recompile-import: {} converged_without_import={} elapsed={}ms request={} probe={}",
                    observation.case_name,
                    observation.converged_without_import,
                    observation.elapsed_ms,
                    clip(&observation.request_result, 100),
                    clip(&observation.last_probe, 100),
                ),
                "passed": 0,
                "failed": 0,
            }),
        );
    }
    sink.emit(
        "suite_event",
        json!({
            "suite": suite.as_str(),
            "line": format!(
                "PASS  recompile-import: locus_pipeline baseline={}ms queued={} targeted={}ms asmdef={}ms delete_full={}ms no_op={}ms probe={}",
                pipeline_observation.baseline_elapsed_ms,
                pipeline_observation.queued_paths,
                pipeline_observation.elapsed_ms,
                pipeline_observation.asmdef_elapsed_ms,
                pipeline_observation.delete_elapsed_ms,
                pipeline_observation.no_op_elapsed_ms,
                clip(&pipeline_observation.probe, 100),
            ),
            "passed": 1,
            "failed": 0,
        }),
    );

    cleanup_recompile_import_fixture(project, &fixture_asset_dir, &fixture_dir).await?;
    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": observations.len() + 1,
            "failed": 0,
            "environment": environment,
            "observations": observations,
            "pipelineObservation": pipeline_observation,
            "fixtureCleaned": true,
        }),
    );
    Ok(())
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

async fn wait_for_modal_dialog(
    project: &str,
    present: bool,
    timeout: Duration,
) -> Result<Option<crate::unity_bridge::dialog::UnityModalDialog>, String> {
    let started = Instant::now();
    loop {
        let dialog = crate::unity_bridge::dialog::current_dialog(project);
        if dialog.is_some() == present {
            return Ok(dialog);
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "Unity modal dialog did not become {} within {}ms",
                if present { "visible" } else { "closed" },
                timeout.as_millis()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn run_python_sdk_script(
    app_handle: &AppHandle,
    project: &str,
    script: &str,
    extra_args: &[String],
    timeout: Duration,
    operation: &str,
) -> Result<String, String> {
    let runtime = crate::python_runtime::resolve_effective_python(Some(app_handle))
        .ok_or_else(|| format!("No Python runtime is available for {operation}"))?;
    crate::python_runtime::ensure_runtime_package_environment(&runtime)?;
    let sdk_env = crate::python_runtime::locus_sdk_invocation_env();
    if sdk_env.is_empty() {
        return Err("Locus SDK bridge connection is not ready".to_string());
    }

    let mut command = tokio::process::Command::new(&runtime.path);
    command
        .arg("-c")
        .arg(script)
        .arg(project)
        .args(extra_args)
        .current_dir(project)
        .kill_on_drop(true);
    for (name, value) in sdk_env {
        command.env(name, value);
    }

    let repository_python = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join("python");
    let python_path = crate::process_util::prepend_paths(
        crate::python_runtime::managed_python_path_env(std::env::var_os("PYTHONPATH"), &runtime),
        vec![repository_python],
    );
    if let Some(python_path) = python_path {
        command.env("PYTHONPATH", python_path);
    }

    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("{operation} timed out after {}s", timeout.as_secs()))?
        .map_err(|error| format!("Could not launch {operation}: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "{operation} exited with {}: {}",
            output.status,
            clip(&stderr, 4_000)
        ));
    }
    Ok(stdout)
}

async fn resolve_modal_dialog_through_python_sdk(
    app_handle: &AppHandle,
    project: &str,
    execution_id: &str,
) -> Result<String, String> {
    let script = r#"import asyncio, json, locus, sys
async def main():
    status = await locus.get_unity_editor_status(project=sys.argv[1])
    if status.ready or not status.main_thread_blocked:
        raise RuntimeError(f"Unity status did not expose the blocked main thread: {status}")
    if status.blocking_reason != "modal_dialog":
        raise RuntimeError(f"Unexpected blocking reason: {status.blocking_reason}")
    if not status.blocking_dialog_recoverable:
        raise RuntimeError("Unity dialog was not marked Agent-recoverable")
    dialog = status.blocking_dialog
    if dialog is None:
        dialog = await locus.get_unity_dialog(project=sys.argv[1])
    if dialog is None:
        raise RuntimeError("Locus SDK returned no Unity dialog")
    choice = next((item for item in dialog.choices if item.label == "Cancel Probe"), None)
    if choice is None:
        raise RuntimeError("Cancel Probe choice was not exposed")
    result = await locus.choose_unity_dialog(
        project=dialog.project,
        dialog_id=dialog.dialog_id,
        choice_id=choice.id,
    )
    execution_output = await locus.wait_unity_execution(
        project=dialog.project,
        execution_id=sys.argv[2],
    )
    print(json.dumps({
        "title": dialog.title,
        "message": dialog.message,
        "choice": choice.label,
        "invoked": result.invoked,
        "executionOutput": execution_output.strip(),
        "ready": status.ready,
        "mainThreadBlocked": status.main_thread_blocked,
        "recoverable": status.blocking_dialog_recoverable,
        "blockingReason": status.blocking_reason,
    }, ensure_ascii=False))
asyncio.run(main())"#;
    run_python_sdk_script(
        app_handle,
        project,
        script,
        &[execution_id.to_string()],
        Duration::from_secs(15),
        "Python SDK dialog resolver",
    )
    .await
}

/// Exercises the public Python SDK before the driver's shared Unity connection
/// preflight. An unopened editor covers launch; an existing editor covers
/// idempotent reuse. Both paths must reach the managed bridge and complete one
/// deterministic local-model Agent run through the first-class Python tool.
async fn run_python_sdk_editor_suite(
    app_handle: &AppHandle,
    project: &str,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    let suite = CliDriverSuite::PythonSdk;
    sink.emit(
        "suite_start",
        json!({ "suite": suite.as_str(), "project": project }),
    );
    let mut run = ExecuteSuiteRun::new(suite, sink);
    let ensure_timeout = config
        .connect_timeout
        .max(Duration::from_secs(30))
        .min(Duration::from_secs(1_800));
    let script_timeout = ensure_timeout.saturating_add(Duration::from_secs(60));
    let script = r#"import asyncio, json, locus, sys
async def main():
    project = sys.argv[1]
    timeout = float(sys.argv[2])
    before = await locus.get_unity_editor_status(project=project)
    ensured = await locus.ensure_unity_editor(
        project=project,
        wait_until="ready",
        timeout=timeout,
    )
    after = await locus.get_unity_editor_status(project=project)

    python_tool = await locus.call_tool(
        "python",
        {
            "action": "run",
            "code": "status = await locus.get_unity_editor_status(project=project)\nprint(f'LOCUS_PYTHON_TOOL_DIRECT:{status.process_state}:{status.ready}')",
            "description": "Verify first-class Python tool SDK injection",
            "readonly": True,
            "timeout": 30000,
        },
        timeout=35,
        workspace_ref=after.workspace_ref,
    )
    python_tool.raise_for_error()

    agent = locus.Agent(
        name="Unity SDK Local Probe",
        id="unity-sdk-local-probe",
        system_prompt="Use the Python tool to inspect the current Unity lifecycle, then report completion.",
        tools=["python"],
    )
    try:
        model_run = await agent.prompt(
            "[[mock:python-tool]] Run the first-class Python tool lifecycle probe.",
            model="mock/tool",
            workspace_ref=after.workspace_ref,
        )
        model_result = await model_run.wait()
        model_result.raise_for_error()
        model_events = await model_run.events()
        model_python_event = next((
            event for event in model_events
            if event.get("eventType") == "toolCallDone"
            and event.get("payload", {}).get("toolName") == "python"
        ), None)
    finally:
        agent.close()

    headless = await locus.restart_unity_editor(
        project=project,
        mode="headless",
        wait_until="ready",
        timeout=timeout,
        force=True,
    )
    headless_status = await locus.get_unity_editor_status(project=project)

    print(json.dumps({
        "before": {
            "processState": before.process_state,
            "semanticPhase": before.semantic_phase,
        },
        "ensure": {
            "launched": ensured.launched,
            "waitUntil": ensured.wait_until,
            "processId": ensured.status.process_id,
            "ready": ensured.status.ready,
        },
        "after": {
            "processState": after.process_state,
            "semanticPhase": after.semantic_phase,
            "connected": after.connected,
            "ready": after.ready,
        },
        "pythonTool": {
            "name": python_tool.name,
            "isError": python_tool.is_error,
            "output": python_tool.output,
        },
        "model": {
            "id": "mock/tool",
            "status": model_result.status,
            "text": model_result.text,
            "pythonToolOutput": (
                model_python_event.get("payload", {}).get("output", "")
                if model_python_event else ""
            ),
        },
        "headless": {
            "launchMode": headless.launch.mode,
            "statusMode": headless_status.launch_mode,
            "headless": headless_status.headless,
            "processId": headless_status.process_id,
            "ready": headless_status.ready,
        },
    }, ensure_ascii=False))
asyncio.run(main())"#;
    let output = run_python_sdk_script(
        app_handle,
        project,
        script,
        &[ensure_timeout.as_secs_f64().to_string()],
        script_timeout,
        "Python SDK Unity lifecycle probe",
    )
    .await?;
    let payload: Value = serde_json::from_str(&output).map_err(|error| {
        format!(
            "Python SDK Unity lifecycle probe returned invalid JSON: {error}; output={}",
            clip(&output, 1_000)
        )
    })?;

    let before_process = payload["before"]["processState"].as_str().unwrap_or("");
    if matches!(before_process, "not_running" | "running") {
        run.pass(
            "P1 initial status",
            format!("public SDK observed processState={before_process}"),
        );
    } else {
        run.fail(
            "P1 initial status",
            format!("expected not_running or running, observed '{before_process}'"),
        );
    }

    let launched = payload["ensure"]["launched"].as_bool().unwrap_or(false);
    let ensured_ready = payload["ensure"]["ready"].as_bool().unwrap_or(false);
    let process_id = payload["ensure"]["processId"].as_u64();
    let lifecycle_action_ok = match before_process {
        "not_running" => launched,
        "running" => !launched,
        _ => false,
    };
    if lifecycle_action_ok && ensured_ready && process_id.is_some() {
        run.pass(
            "P2 SDK ensure launch",
            format!(
                "{} PID {} and reached ready",
                if launched { "launched" } else { "reused" },
                process_id.unwrap_or_default(),
            ),
        );
    } else {
        run.fail(
            "P2 SDK ensure launch",
            format!("launched={launched}, ready={ensured_ready}, processId={process_id:?}"),
        );
    }

    let after_running = payload["after"]["processState"] == "running";
    let after_connected = payload["after"]["connected"].as_bool().unwrap_or(false);
    let after_ready = payload["after"]["ready"].as_bool().unwrap_or(false);
    if after_running && after_connected && after_ready {
        run.pass(
            "P3 final status",
            format!(
                "phase={}, connected=true, ready=true",
                payload["after"]["semanticPhase"]
                    .as_str()
                    .unwrap_or("unknown")
            ),
        );
    } else {
        run.fail(
            "P3 final status",
            format!(
                "processState={}, connected={after_connected}, ready={after_ready}",
                payload["after"]["processState"]
                    .as_str()
                    .unwrap_or("unknown")
            ),
        );
    }

    let python_tool_ok = payload["pythonTool"]["name"] == "python"
        && !payload["pythonTool"]["isError"].as_bool().unwrap_or(true)
        && payload["pythonTool"]["output"]
            .as_str()
            .unwrap_or("")
            .contains("LOCUS_PYTHON_TOOL_DIRECT:running:True");
    if python_tool_ok {
        run.pass(
            "P4 first-class Python tool",
            "checkout injection and nested lifecycle SDK call succeeded",
        );
    } else {
        run.fail(
            "P4 first-class Python tool",
            clip(payload["pythonTool"]["output"].as_str().unwrap_or(""), 500),
        );
    }

    let model_id = payload["model"]["id"].as_str().unwrap_or("");
    let model_status = payload["model"]["status"].as_str().unwrap_or("");
    let model_text = payload["model"]["text"].as_str().unwrap_or("");
    let model_python_output = payload["model"]["pythonToolOutput"].as_str().unwrap_or("");
    if model_id == "mock/tool"
        && model_status == "done"
        && model_text.contains("simulated tool call completed")
        && model_python_output.contains("LOCUS_PYTHON_TOOL_OK:running:True")
    {
        run.pass(
            "P5 local model Python call",
            format!(
                "mock/tool completed the Python tool round: {}",
                clip(model_text, 160)
            ),
        );
    } else {
        run.fail(
            "P5 local model Python call",
            format!(
                "model={model_id}, status={model_status}, text={}, pythonOutput={}",
                clip(model_text, 160),
                clip(model_python_output, 240)
            ),
        );
    }

    let headless_pid = payload["headless"]["processId"].as_u64();
    let headless_ready = payload["headless"]["ready"].as_bool().unwrap_or(false);
    let headless_mode_ok = payload["headless"]["launchMode"] == "headless"
        && payload["headless"]["statusMode"] == "headless"
        && payload["headless"]["headless"].as_bool().unwrap_or(false);
    if headless_mode_ok && headless_ready && headless_pid.is_some() {
        run.pass(
            "P6 headless lifecycle",
            format!(
                "restarted PID {} in headless mode and reached ready",
                headless_pid.unwrap_or_default()
            ),
        );
    } else {
        run.fail(
            "P6 headless lifecycle",
            format!(
                "launchMode={}, statusMode={}, headless={}, ready={headless_ready}, processId={headless_pid:?}",
                payload["headless"]["launchMode"].as_str().unwrap_or(""),
                payload["headless"]["statusMode"].as_str().unwrap_or(""),
                payload["headless"]["headless"].as_bool().unwrap_or(false),
            ),
        );
    }

    let close_result = crate::unity_bridge::force_close_current_project_unity_processes(
        project,
        Duration::from_secs(30),
    )
    .await;
    let closed_headless = match close_result {
        Ok(result) => headless_pid.is_some_and(|process_id| {
            result
                .process_ids
                .iter()
                .any(|closed| u64::from(*closed) == process_id)
        }),
        Err(_) => false,
    };
    if closed_headless {
        run.pass(
            "P7 headless close",
            "the headless editor process was explicitly closed",
        );
    } else {
        run.fail(
            "P7 headless close",
            "the headless editor process was not reported by the close operation",
        );
    }

    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": run.passed,
            "failed": run.failed,
            "processId": process_id,
            "launched": launched,
            "semanticPhase": payload["after"]["semanticPhase"],
            "mockModel": model_id,
            "headlessProcessId": headless_pid,
        }),
    );
    if run.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "python-sdk suite finished with {} failed check(s)",
            run.failed
        ))
    }
}

/// Opens a real Unity Editor modal dialog from an active `unity_execute`,
/// proves that a second main-thread request fails fast, and resolves the dialog
/// through the public Python SDK while Unity's managed main thread is blocked.
async fn run_safe_mode_recovery_suite(
    _app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
    cancel_rx: &mut watch::Receiver<bool>,
) -> Result<(), String> {
    const FIXTURE_ASSET: &str = "Assets/LocusSafeModeDriverProbe.cs";
    const FIXTURE_META: &str = "Assets/LocusSafeModeDriverProbe.cs.meta";
    const BAD_SOURCE: &str = "internal static class LocusSafeModeDriverProbe { private static LocusSafeModeDriverMissingType Value; }\n";
    const GOOD_SOURCE: &str =
        "internal static class LocusSafeModeDriverProbe { internal const int Value = 42; }\n";

    sink.emit(
        "suite_start",
        json!({ "suite": suite.as_str(), "project": project }),
    );
    let fixture = Path::new(project).join(FIXTURE_ASSET.replace('/', "\\"));
    let fixture_meta = Path::new(project).join(FIXTURE_META.replace('/', "\\"));
    if fixture.exists() || fixture_meta.exists() {
        return Err(format!(
            "safe-mode suite fixture already exists: {}",
            fixture.display()
        ));
    }

    let mut run = ExecuteSuiteRun::new(suite, sink);
    let outcome: Result<(), String> = async {
        std::fs::write(&fixture, BAD_SOURCE)
            .map_err(|error| format!("failed to write Safe Mode fixture: {error}"))?;
        // Auto Refresh can be disabled in the test project's editor preferences.
        // Queue the script explicitly, but treat a response timeout as expected:
        // ImportAsset can synchronously enter compilation before the managed
        // request posts its response. Editor.log is the authoritative result.
        let _ = unity_bridge::import_assets(project, &[FIXTURE_ASSET.to_string()]).await;
        let compile_started = Instant::now();
        loop {
            let process = unity_bridge::query_current_project_editor_process(project).await;
            let observed = unity_bridge::read_editor_log_console_entries(
                project,
                process.process_id,
                &["error".to_string()],
                100,
            )
            .map(|read| {
                read.entries.iter().any(|entry| {
                    entry.message.contains("LocusSafeModeDriverMissingType")
                        || entry.message.contains("CS0246")
                })
            })
            .unwrap_or(false);
            if observed {
                break;
            }
            if compile_started.elapsed() >= config.connect_timeout {
                return Err(format!(
                    "fixture compiler error did not appear in Editor.log within {}ms",
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
        run.pass(
            "S0 compiler fixture",
            "observed the deterministic compiler error out of process before restart",
        );

        unity_bridge::close_current_project_unity_processes(project, Duration::from_secs(45))
            .await?;
        // Unity can require a short interval after a forced close to release
        // its project lock and crash-recovery handles. Launching immediately
        // may create a short-lived editor process that exits before showing
        // the Safe Mode recovery prompt.
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        unity_bridge::launch_project_with_mode(project, unity_bridge::UnityLaunchMode::Interactive)
            .await?;

        let safe_started = Instant::now();
        let mut safe_state = None;
        let mut selected_dialog = false;
        while safe_started.elapsed() < config.suite_timeout {
            if run_cancelled(cancel_rx) {
                return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
            }
            if !selected_dialog {
                if let Some(dialog) = unity_bridge::dialog::current_dialog(project) {
                    let dialog_text =
                        format!("{} {}", dialog.title, dialog.message).to_ascii_lowercase();
                    if dialog_text.contains("safe mode") || dialog_text.contains("安全模式") {
                        if let Some(choice) = dialog
                            .choices
                            .iter()
                            .find(|choice| {
                                let label = choice.label.to_ascii_lowercase();
                                label.contains("safe mode") || label.contains("安全模式")
                            })
                            .or_else(|| dialog.choices.first())
                        {
                            unity_bridge::dialog::choose_dialog(
                                project,
                                &dialog.dialog_id,
                                &choice.id,
                            )
                            .await?;
                            selected_dialog = true;
                        }
                    }
                }
            }
            let semantic = unity_bridge::unity_semantic_state(project).await;
            let safe_mode_prompt_open = semantic
                .process
                .pid
                .and_then(unity_bridge::dialog::main_window_title)
                .map(|title| {
                    let normalized = title.to_ascii_lowercase();
                    normalized.contains("enter safe mode") || normalized.contains("进入安全模式")
                })
                .unwrap_or(false);
            // The recovery prompt itself contains "Safe Mode" in its title.
            // Keep driving the modal choice until Unity replaces that prompt
            // with the real project window carrying the SAFE MODE marker.
            if semantic.phase == "safe_mode" && !safe_mode_prompt_open {
                safe_state = Some(semantic);
                break;
            }
            if semantic.process.state == "not_running"
                && safe_started.elapsed() >= Duration::from_secs(30)
            {
                return Err("Unity exited before entering Safe Mode".to_string());
            }
            tokio::select! {
                _ = tokio::time::sleep(config.poll_interval) => {}
                _ = cancel_rx.changed() => {
                    return Err(UNITY_INTEGRATION_TEST_CANCELLED.to_string());
                }
            }
        }
        let safe_state = safe_state.ok_or_else(|| {
            format!(
                "Unity did not enter Safe Mode within {}ms",
                config.suite_timeout.as_millis()
            )
        })?;
        if safe_state.safety.can_call_unity_api
            || safe_state.safety.recommended_action != "fix_compile_errors"
        {
            return Err(format!(
                "Safe Mode capability contract is incorrect: canCallUnityApi={} action={}",
                safe_state.safety.can_call_unity_api, safe_state.safety.recommended_action
            ));
        }
        run.pass(
            "S1 external state probe",
            format!(
                "phase={} source={} editorLog={}",
                safe_state.phase,
                safe_state.source,
                safe_state
                    .editor_log
                    .path
                    .as_deref()
                    .unwrap_or("unavailable")
            ),
        );

        let log_read = unity_bridge::read_editor_log_console_entries(
            project,
            safe_state.process.pid,
            &["error".to_string()],
            50,
        )?;
        if !log_read.entries.iter().any(|entry| {
            entry.message.contains("LocusSafeModeDriverMissingType")
                || entry.message.contains("CS0246")
        }) {
            return Err(format!(
                "Editor log fallback did not expose the fixture compiler error: {}",
                log_read.path
            ));
        }
        run.pass(
            "S2 out-of-process log",
            format!("read compiler diagnostics from {}", log_read.path),
        );

        std::fs::write(&fixture, GOOD_SOURCE)
            .map_err(|error| format!("failed to repair Safe Mode fixture: {error}"))?;
        let first_recovery = wait_for_semantic_ready(
            project,
            suite,
            "safe_mode_repair",
            SemanticReadyRequirement::UnityApi,
            Duration::from_secs(30),
            config.poll_interval,
            sink,
            cancel_rx,
        )
        .await;
        let (recovered, recovery_strategy) = match first_recovery {
            Ok(recovered) => (recovered, "automatic_refresh"),
            Err(error) => {
                if error == UNITY_INTEGRATION_TEST_CANCELLED {
                    return Err(error);
                }
                let state = unity_bridge::unity_semantic_state(project).await;
                if state.phase != "safe_mode" {
                    return Err(format!(
                        "Safe Mode repair did not recover and the editor left the expected state: {error}; phase={}",
                        state.phase
                    ));
                }
                // External file changes are not auto-refreshed while an older
                // Unity Safe Mode window remains in the background. Restarting
                // is the deterministic out-of-process recovery path: startup
                // recompiles the repaired source before the managed bridge is
                // required.
                unity_bridge::close_current_project_unity_processes(
                    project,
                    Duration::from_secs(45),
                )
                .await?;
                tokio::time::sleep(Duration::from_millis(2_500)).await;
                unity_bridge::launch_project_with_mode(
                    project,
                    unity_bridge::UnityLaunchMode::Interactive,
                )
                .await?;
                let recovered = wait_for_semantic_ready(
                    project,
                    suite,
                    "safe_mode_repair_restart",
                    SemanticReadyRequirement::UnityApi,
                    config.suite_timeout,
                    config.poll_interval,
                    sink,
                    cancel_rx,
                )
                .await?;
                (recovered, "editor_restart")
            }
        };
        if recovered.phase == "safe_mode" {
            return Err("Unity remained in Safe Mode after compiler repair".to_string());
        }
        run.pass(
            "S3 Safe Mode repair recovery",
            format!(
                "recovered to phase={} after file repair via {}",
                recovered.phase, recovery_strategy
            ),
        );

        std::fs::remove_file(&fixture)
            .map_err(|error| format!("failed to remove Safe Mode fixture: {error}"))?;
        if fixture_meta.exists() {
            std::fs::remove_file(&fixture_meta)
                .map_err(|error| format!("failed to remove Safe Mode fixture meta: {error}"))?;
        }
        let _ = unity_bridge::import_assets(
            project,
            &[FIXTURE_ASSET.to_string(), FIXTURE_META.to_string()],
        )
        .await;
        wait_for_semantic_ready(
            project,
            suite,
            "safe_mode_fixture_cleanup",
            SemanticReadyRequirement::UnityApi,
            config.suite_timeout,
            config.poll_interval,
            sink,
            cancel_rx,
        )
        .await?;

        let crash_project = project.to_string();
        let execution = tokio::spawn(async move {
            unity_bridge::unity_execute_code_with_non_public_access(
                &crash_project,
                r#"await ctx.WaitSeconds(120); print("SAFE_MODE_DRIVER_SHOULD_NOT_FINISH");"#,
                false,
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(1_500)).await;
        unity_bridge::force_close_current_project_unity_processes(project, Duration::from_secs(20))
            .await?;
        let execution_result = tokio::time::timeout(Duration::from_secs(30), execution)
            .await
            .map_err(|_| {
                "Unity execution did not terminate after the editor was killed".to_string()
            })?
            .map_err(|error| format!("Unity execution task join failed: {error}"))?;
        let execution_error = match execution_result {
            Ok(output) => {
                return Err(format!(
                    "Unity execution unexpectedly succeeded after the editor exited: {}",
                    clip(&output, 240)
                ));
            }
            Err(error) => error,
        };

        let crash_started = Instant::now();
        let crash_state = loop {
            let state = unity_bridge::unity_semantic_state(project).await;
            if state.phase == "crashed" {
                break state;
            }
            if crash_started.elapsed() >= Duration::from_secs(20) {
                return Err(format!(
                    "state probe did not report crash after forced exit: phase={} process={}",
                    state.phase, state.process.state
                ));
            }
            tokio::time::sleep(config.poll_interval).await;
        };
        let enriched = unity_bridge::enrich_unity_tool_error(project, &execution_error).await;
        let expected_log = crash_state.editor_log.path.as_deref().unwrap_or("");
        if !enriched.contains("state: crashed")
            || !enriched.contains("editor_log:")
            || (!expected_log.is_empty() && !enriched.contains(expected_log))
            || !enriched.contains("Inspect the Editor log")
        {
            return Err(format!(
                "crash diagnostic was incomplete: {}",
                clip(&enriched, 600)
            ));
        }
        run.pass(
            "S4 tool-time crash diagnostic",
            format!(
                "failed execution reported crash and Editor log {}",
                expected_log
            ),
        );

        unity_bridge::launch_project_with_mode(project, unity_bridge::UnityLaunchMode::Interactive)
            .await?;
        wait_for_semantic_ready(
            project,
            suite,
            "post_crash_restart",
            SemanticReadyRequirement::UnityApi,
            config.suite_timeout,
            config.poll_interval,
            sink,
            cancel_rx,
        )
        .await?;
        run.pass(
            "S5 post-crash recovery",
            "Unity restarted and the bridge became ready",
        );
        Ok(())
    }
    .await;

    if let Err(error) = &outcome {
        run.fail("safe-mode recovery", error);
        let _ = unity_bridge::force_close_current_project_unity_processes(
            project,
            Duration::from_secs(20),
        )
        .await;
        let _ = std::fs::remove_file(&fixture);
        let _ = std::fs::remove_file(&fixture_meta);
        if unity_bridge::launch_project_with_mode(
            project,
            unity_bridge::UnityLaunchMode::Interactive,
        )
        .await
        .is_ok()
        {
            let _ = wait_for_semantic_ready(
                project,
                suite,
                "failure_cleanup",
                SemanticReadyRequirement::UnityApi,
                config.suite_timeout,
                config.poll_interval,
                sink,
                cancel_rx,
            )
            .await;
        }
    }

    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": run.passed,
            "failed": run.failed,
        }),
    );
    outcome
}

async fn run_modal_dialog_suite(
    app_handle: &AppHandle,
    project: &str,
    suite: CliDriverSuite,
    config: &CliDriverConfig,
    sink: &DriverEventSink,
) -> Result<(), String> {
    sink.emit(
        "suite_start",
        json!({ "suite": suite.as_str(), "project": project }),
    );
    let mut run = ExecuteSuiteRun::new(suite, sink);

    crate::unity_bridge::dialog::ensure_project_observed(project).await?;
    if crate::unity_bridge::dialog::current_dialog(project).is_some() {
        return Err("A Unity modal dialog was already open before the probe".to_string());
    }

    let marker_name = format!(
        "locus-modal-dialog-probe-{}.txt",
        uuid::Uuid::new_v4().simple()
    );
    let marker_path = Path::new(project).join("Temp").join(&marker_name);
    let _ = std::fs::remove_file(&marker_path);
    let code = format!(
        r#"bool accepted = EditorUtility.DisplayDialog(
    "Locus Native Dialog Probe",
    "This dialog belongs to the isolated Unity probe project. The event hook must target this dialog only.",
    "Continue Probe",
    "Cancel Probe");
var markerPath = System.IO.Path.Combine(
    System.IO.Directory.GetParent(Application.dataPath).FullName,
    "Temp",
    "{marker_name}");
System.IO.File.WriteAllText(markerPath, accepted ? "continued" : "cancelled");
print("MODAL_PROBE:" + (accepted ? "continued" : "cancelled"));"#
    );
    let execute_project = project.to_string();
    let execute_task = tauri::async_runtime::spawn(async move {
        unity_bridge::unity_execute_code_with_progress(&execute_project, &code, |_| {}).await
    });

    let dialog = wait_for_modal_dialog(
        project,
        true,
        config.suite_timeout.min(Duration::from_secs(20)),
    )
    .await
    .map_err(|_| "WinEventHook did not publish the Unity modal dialog within 20s".to_string())?
    .ok_or_else(|| "Unity modal dialog snapshot was unexpectedly empty".to_string())?;

    let labels = dialog
        .choices
        .iter()
        .map(|choice| choice.label.as_str())
        .collect::<Vec<_>>();
    if dialog.title == "Locus Native Dialog Probe"
        && dialog.message.contains("isolated Unity probe project")
        && labels.contains(&"Continue Probe")
        && labels.contains(&"Cancel Probe")
        && dialog.main_thread_blocked
    {
        run.pass(
            "D1 native event snapshot",
            format!(
                "title/body/{} choices captured for dialog_id={}",
                dialog.choices.len(),
                dialog.dialog_id
            ),
        );
    } else {
        run.fail(
            "D1 native event snapshot",
            format!(
                "unexpected snapshot title='{}' message='{}' choices={labels:?}",
                dialog.title,
                clip(&dialog.message, 180)
            ),
        );
    }

    let preflight_started = Instant::now();
    let preflight = tokio::time::timeout(
        Duration::from_secs(2),
        unity_bridge::set_editor_status(project, UNITY_EDITOR_STATUS_EDITING),
    )
    .await;
    match preflight {
        Ok(Err(error))
            if crate::unity_bridge::dialog::is_unity_modal_dialog_blocked_error(&error)
                && error.contains("request_state=not_sent") =>
        {
            run.pass(
                "D2 main-thread preflight",
                format!(
                    "failed fast in {}ms",
                    preflight_started.elapsed().as_millis()
                ),
            );
        }
        Ok(Err(error)) => run.fail(
            "D2 main-thread preflight",
            format!("unexpected error: {}", clip(&error, 240)),
        ),
        Ok(Ok(())) => run.fail(
            "D2 main-thread preflight",
            "request unexpectedly reached the blocked Unity main thread",
        ),
        Err(_) => run.fail(
            "D2 main-thread preflight",
            "request did not return within 2s",
        ),
    }

    let execute_result = tokio::time::timeout(Duration::from_secs(5), execute_task)
        .await
        .map_err(|_| "The active unity_execute did not detach after dialog detection".to_string())?
        .map_err(|error| format!("unity_execute task join failed: {error}"))?;
    let detached_execution_id = execute_result.as_ref().err().and_then(|error| {
        error
            .lines()
            .find_map(|line| line.strip_prefix("request_id="))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    });
    match execute_result {
        Err(error)
            if crate::unity_bridge::dialog::is_unity_modal_dialog_blocked_error(&error)
                && (error.contains("request_state=detached")
                    || error.contains("request_state=unknown"))
                && detached_execution_id.is_some() =>
        {
            run.pass(
                "D3 active request detach",
                "unity_execute returned dialog content and a resumable execution id without waiting for its normal timeout",
            );
        }
        Err(error) => run.fail(
            "D3 active request detach",
            format!("unexpected error: {}", clip(&error, 300)),
        ),
        Ok(output) => run.fail(
            "D3 active request detach",
            format!("execute unexpectedly completed: {}", clip(&output, 180)),
        ),
    }

    let python_result = match detached_execution_id.as_deref() {
        Some(execution_id) => {
            resolve_modal_dialog_through_python_sdk(app_handle, project, execution_id).await
        }
        None => Err("The detached unity_execute error did not include request_id".to_string()),
    };
    match &python_result {
        Ok(output)
            if output.contains("Locus Native Dialog Probe")
                && output.contains("Cancel Probe")
                && output.contains("\"invoked\": true")
                && output.contains("\"executionOutput\": \"MODAL_PROBE:cancelled\"")
                && output.contains("\"ready\": false")
                && output.contains("\"mainThreadBlocked\": true")
                && output.contains("\"recoverable\": true")
                && output.contains("\"blockingReason\": \"modal_dialog\"") =>
        {
            run.pass(
                "D4 Python SDK choice and wait",
                "SDK closed Cancel Probe and immediately recovered the original Unity execution",
            );
        }
        Ok(output) => run.fail(
            "D4 Python SDK choice and wait",
            format!("unexpected SDK output: {}", clip(output, 300)),
        ),
        Err(error) => {
            run.fail(
                "D4 Python SDK choice and wait",
                format!("SDK error: {}", clip(error, 300)),
            );
            if let Some(current) = crate::unity_bridge::dialog::current_dialog(project) {
                if let Some(cancel) = current
                    .choices
                    .iter()
                    .find(|choice| choice.label == "Cancel Probe")
                {
                    let _ = crate::unity_bridge::dialog::choose_dialog(
                        project,
                        &current.dialog_id,
                        &cancel.id,
                    )
                    .await;
                }
            }
        }
    }

    let marker_started = Instant::now();
    let marker = loop {
        if let Ok(value) = std::fs::read_to_string(&marker_path) {
            break Some(value);
        }
        if marker_started.elapsed() >= Duration::from_secs(10) {
            break None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    if marker.as_deref() == Some("cancelled") {
        run.pass(
            "D5 Unity main-thread resume",
            "DisplayDialog returned false and the post-dialog marker was written",
        );
    } else {
        run.fail(
            "D5 Unity main-thread resume",
            format!("marker was {:?}", marker.as_deref()),
        );
    }

    let dialog_closed = wait_for_modal_dialog(project, false, Duration::from_secs(5))
        .await
        .is_ok();
    tokio::time::sleep(Duration::from_millis(500)).await;
    match tokio::time::timeout(
        Duration::from_secs(30),
        execute_capture(project, r#"print("MODAL_RECOVERED:42");"#),
    )
    .await
    {
        Ok(Ok(output)) if dialog_closed && output.contains("MODAL_RECOVERED:42") => {
            run.pass(
                "D6 recovered request",
                "dialog registry cleared and a new Unity main-thread request completed",
            );
        }
        Ok(Ok(output)) => run.fail(
            "D6 recovered request",
            format!(
                "dialog_closed={dialog_closed}, output='{}'",
                clip(&output, 180)
            ),
        ),
        Ok(Err(error)) => run.fail(
            "D6 recovered request",
            format!("recovered execute failed: {}", clip(&error, 240)),
        ),
        Err(_) => run.fail(
            "D6 recovered request",
            "recovered execute did not complete within 30s",
        ),
    }

    let _ = std::fs::remove_file(&marker_path);
    sink.emit(
        "suite_result",
        json!({
            "suite": suite.as_str(),
            "passed": run.passed,
            "failed": run.failed,
            "dialogId": dialog.dialog_id,
            "pythonSdk": python_result.is_ok(),
        }),
    );
    if run.failed == 0 {
        Ok(())
    } else {
        Err(format!(
            "modal-dialog suite finished with {} failed check(s)",
            run.failed
        ))
    }
}

/// Drives the real `unity_execute` / `unity_run_states` code paths end to end:
/// round-trip correctness, many sequential compiled snippets, async/blocking
/// execution with progress + cancellation, op-lock serialization, the legacy
/// in-Unity compile path, a run-states transition, and a full new-type
/// recompile. Bespoke suite shaped like `run_sidecar_suite`: emits `suite_event`
/// lines per check and a final `suite_result`, returning `Err` if any failed.
async fn run_execute_suite(
    app_handle: &AppHandle,
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

    match run_agent_unity_execute_probe(app_handle, project, config.suite_timeout).await {
        Ok(output) => run.pass(
            "E0 agent pipeline",
            format!("mock Agent unity_execute returned '{}'", clip(&output, 160)),
        ),
        Err(error) => run.fail(
            "E0 agent pipeline",
            format!("mock Agent unity_execute failed: {}", clip(&error, 240)),
        ),
    }

    match run_agent_unity_yaml_read_probe(app_handle, project, config.suite_timeout).await {
        Ok(output) => run.pass(
            "E0Y agent yaml pipeline",
            format!(
                "mock Agent unity_yaml_read returned '{}'",
                clip(&output, 160)
            ),
        ),
        Err(error) => run.fail(
            "E0Y agent yaml pipeline",
            format!("mock Agent unity_yaml_read failed: {}", clip(&error, 240)),
        ),
    }

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
    run.check_marker(
        project,
        "E3J anonymous JSON",
        r#"printJson(new { Migrated = 3, MainScene = "Assets/Main.unity", EntityScene = "Assets/Entity.unity", Counts = new Dictionary<string, int> { { "Enemy", 4 } } });"#,
        r#"{"Migrated":3,"MainScene":"Assets/Main.unity","EntityScene":"Assets/Entity.unity","Counts":{"Enemy":4}}"#,
    )
    .await;
    run.check_marker(
        project,
        "E3J reference loop",
        r#"var value = new Dictionary<string, object>(); value["Count"] = 7; value["Self"] = value; printJson(value);"#,
        r#"{"$id":1,"Count":7,"Self":{"$ref":1}}"#,
    )
    .await;
    run.check_marker(
        project,
        "E3J BFS ownership",
        r#"var shared = new Dictionary<string, object> { { "Value", 9 } }; var deep = new Dictionary<string, object> { { "Child", shared } }; var value = new Dictionary<string, object> { { "Deep", deep }, { "Shallow", shared } }; printJson(value);"#,
        r#"{"Deep":{"Child":{"$ref":3}},"Shallow":{"$id":3,"Value":9}}"#,
    )
    .await;
    run.check_marker(
        project,
        "E3J deferred enumerable",
        r#"IEnumerable<int> Infinite() { while (true) yield return 1; } printJson(new { Values = Infinite() });"#,
        r#""$deferredEnumerable":"#,
    )
    .await;
    run.check_marker(
        project,
        "E3J nested Unity object",
        r#"var go = new GameObject("LocusPrintJsonProbe"); try { printJson(new { UnityObject = go }); } finally { UnityEngine.Object.DestroyImmediate(go); }"#,
        r#""UnityObject":{"$unityObject":true,"type":"UnityEngine.GameObject","instanceId":"#,
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

async fn run_agent_unity_execute_probe(
    app_handle: &AppHandle,
    project: &str,
    timeout: Duration,
) -> Result<String, String> {
    open_and_focus_workspace_for_driver(app_handle, project).await?;
    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let runtime = registry
        .runtime_for_root(Path::new(project))
        .ok_or_else(|| format!("Workspace runtime was not registered for {project}"))?;
    let session_id = create_workspace_driver_session(app_handle, 0, runtime.as_ref()).await?;
    let target = WorkspaceSuiteTarget {
        index: 0,
        project: project.to_string(),
        runtime,
        session_id,
        plugin_outcome: PluginPrepareOutcome::UpToDate,
    };
    let launch = launch_workspace_mock_chat_with_prompt(
        app_handle,
        &target,
        format!(
            "{} Reproduce the Agent unity_execute pipeline.",
            crate::agent::instance::MOCK_AGENT_UNITY_EXECUTE_SCENARIO
        ),
    )
    .await?;
    wait_for_workspace_mock_chat(app_handle, &launch, &target, timeout).await?;

    let store = app_handle
        .state::<Arc<crate::session::store::SessionStore>>()
        .inner()
        .clone();
    let messages = store.get_messages(&launch.session_id)?;
    let tool_call_id = messages
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .find(|tool_call| tool_call.name == "unity_execute")
        .map(|tool_call| tool_call.id.clone())
        .ok_or_else(|| "Mock Agent did not emit unity_execute".to_string())?;
    let output = messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some(tool_call_id.as_str()))
        .map(|message| message.content.trim().to_string())
        .ok_or_else(|| "Mock Agent unity_execute result was not persisted".to_string())?;
    if output.is_empty()
        || output.contains("failed")
        || output.contains("not connected")
        || output.contains("service binding error")
    {
        return Err(format!("unexpected tool result: {output}"));
    }
    Ok(output)
}

async fn run_agent_unity_yaml_read_probe(
    app_handle: &AppHandle,
    project: &str,
    timeout: Duration,
) -> Result<String, String> {
    open_and_focus_workspace_for_driver(app_handle, project).await?;
    let registry = app_handle
        .state::<Arc<crate::workspace_service::ProjectRegistry>>()
        .inner()
        .clone();
    let runtime = registry
        .runtime_for_root(Path::new(project))
        .ok_or_else(|| format!("Workspace runtime was not registered for {project}"))?;
    let session_id = create_workspace_driver_session(app_handle, 0, runtime.as_ref()).await?;
    let target = WorkspaceSuiteTarget {
        index: 0,
        project: project.to_string(),
        runtime,
        session_id,
        plugin_outcome: PluginPrepareOutcome::UpToDate,
    };
    let launch = launch_workspace_mock_chat_with_prompt(
        app_handle,
        &target,
        format!(
            "{} Reproduce the Agent unity_yaml_read pipeline.",
            crate::agent::instance::MOCK_AGENT_UNITY_YAML_READ_SCENARIO
        ),
    )
    .await?;
    wait_for_workspace_mock_chat(app_handle, &launch, &target, timeout).await?;

    let store = app_handle
        .state::<Arc<crate::session::store::SessionStore>>()
        .inner()
        .clone();
    let messages = store.get_messages(&launch.session_id)?;
    let tool_call_id = messages
        .iter()
        .filter_map(|message| message.tool_calls.as_ref())
        .flatten()
        .find(|tool_call| tool_call.name == "unity_yaml_read")
        .map(|tool_call| tool_call.id.clone())
        .ok_or_else(|| "Mock Agent did not emit unity_yaml_read".to_string())?;
    let output = messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some(tool_call_id.as_str()))
        .map(|message| message.content.trim().to_string())
        .ok_or_else(|| "Mock Agent unity_yaml_read result was not persisted".to_string())?;
    if output.is_empty()
        || output.contains("failed")
        || output.contains("not connected")
        || output.contains("service binding error")
    {
        return Err(format!("unexpected tool result: {output}"));
    }
    Ok(output)
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
    fn parse_python_sdk_suite() {
        let parsed = parse(&[
            "--locus-driver",
            "unity-test",
            "--project",
            "F:/Game",
            "--suite",
            "python-sdk",
        ])
        .unwrap()
        .unwrap();

        assert_eq!(parsed.suites, vec![CliDriverSuite::PythonSdk]);
        assert!(parsed.open_unity);
    }

    #[test]
    fn parse_workspace_suite_keeps_repeated_project_paths() {
        let parsed = parse(&[
            "--locus-driver",
            "unity-test",
            "--project",
            "F:/Game",
            "--workspace-project",
            "F:/GameCopy",
            "--workspace-project=F:/GameWorktree",
            "--suite",
            "workspace",
        ])
        .unwrap()
        .unwrap();

        assert_eq!(parsed.suites, vec![CliDriverSuite::Workspace]);
        assert_eq!(parsed.project_path.as_deref(), Some("F:/Game"));
        assert_eq!(
            parsed.workspace_paths,
            vec!["F:/GameCopy".to_string(), "F:/GameWorktree".to_string()]
        );
    }

    #[test]
    fn parse_workspace_switch_suite_keeps_the_second_project() {
        let parsed = parse(&[
            "--locus-driver",
            "unity-test",
            "--project",
            "F:/GameA",
            "--workspace-project",
            "F:/GameB",
            "--suite",
            "workspace-switch",
        ])
        .unwrap()
        .unwrap();

        assert_eq!(parsed.suites, vec![CliDriverSuite::WorkspaceSwitch]);
        assert_eq!(parsed.project_path.as_deref(), Some("F:/GameA"));
        assert_eq!(parsed.workspace_paths, vec!["F:/GameB".to_string()]);
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
                CliDriverSuite::PythonSdk,
                CliDriverSuite::ModalDialog,
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
    fn parse_session_undo_suite_aliases() {
        for alias in ["session-undo", "session_undo", "undo", "file-undo"] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::SessionUndo],
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
    fn parse_recompile_import_suite_aliases() {
        for alias in ["recompile-import", "compile_import", "asset-refresh"] {
            let parsed = parse(&["--locus-unity-test", "--suite", alias])
                .unwrap()
                .unwrap();
            assert_eq!(
                parsed.suites,
                vec![CliDriverSuite::RecompileImport],
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
