use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};
use uuid::Uuid;

use super::{
    exit_play_mode, is_play_mode_status, query_unity_status, send_message,
    send_message_without_timeout, transport, PipeResponse,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnityTestMode {
    All,
    Editmode,
    Playmode,
}

impl UnityTestMode {
    fn as_bridge_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Editmode => "editmode",
            Self::Playmode => "playmode",
        }
    }
}

impl Default for UnityTestMode {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestFilter {
    #[serde(default, alias = "test_mode")]
    pub test_mode: UnityTestMode,
    #[serde(
        default,
        alias = "assembly_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub assembly_name: Option<String>,
    #[serde(
        default,
        alias = "fixture_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub fixture_name: Option<String>,
    #[serde(default, alias = "test_name", skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestTarget {
    #[serde(
        default,
        alias = "assembly_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub assembly_name: Option<String>,
    #[serde(
        default,
        alias = "fixture_name",
        skip_serializing_if = "Option::is_none"
    )]
    pub fixture_name: Option<String>,
    #[serde(default, alias = "test_name", skip_serializing_if = "Option::is_none")]
    pub test_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestRunRequest {
    #[serde(flatten)]
    pub filter: UnityTestFilter,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<UnityTestTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestDiscovery {
    #[serde(default)]
    pub assemblies: Vec<UnityTestAssembly>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestAssembly {
    pub name: String,
    pub test_mode: String,
    #[serde(default)]
    pub fixtures: Vec<UnityTestFixture>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestFixture {
    pub name: String,
    #[serde(default)]
    pub tests: Vec<UnityTestMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestMethod {
    pub name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub attributes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestProgress {
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub current_test: String,
    #[serde(default)]
    pub completed: u32,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestPhaseResult {
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub test_mode: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub passed: u32,
    #[serde(default)]
    pub failed: u32,
    #[serde(default)]
    pub skipped: u32,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub results: Vec<UnityTestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestResult {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test_mode: String,
    #[serde(default)]
    pub assembly_name: String,
    #[serde(default)]
    pub fixture_name: String,
    #[serde(default)]
    pub test_name: String,
    #[serde(default)]
    pub full_name: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub stack_trace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestError {
    pub code: String,
    pub message: String,
    #[serde(skip)]
    partial_phases: Vec<UnityTestPhaseResult>,
}

impl UnityTestError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            partial_phases: Vec::new(),
        }
    }

    fn with_partial_phase(mut self, phase: UnityTestPhaseResult) -> Self {
        self.partial_phases.push(phase);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestSnapshot {
    pub run_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub terminal_status: String,
    pub preparation: UnityTestPreparation,
    pub requested_scope: UnityTestRunRequest,
    pub phase_summaries: Vec<UnityTestPhaseResult>,
    pub total_summary: UnityTestSummary,
    pub results: Vec<UnityTestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<UnityTestError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestPreparation {
    pub method: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnityTestSummary {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration: f64,
}

static ACTIVE_TEST_RUN: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn active_test_run() -> &'static Mutex<Option<String>> {
    ACTIVE_TEST_RUN.get_or_init(|| Mutex::new(None))
}

pub async fn find_tests(
    project_path: &str,
    request: UnityTestFilter,
) -> Result<UnityTestDiscovery, UnityTestError> {
    let payload = serde_json::to_string(&request).map_err(|error| {
        UnityTestError::new(
            "unknown",
            format!("Failed to serialize test find request: {error}"),
        )
    })?;
    let response = send_message(project_path, "find_tests", &payload)
        .await
        .map_err(map_transport_error)?;
    let message = ok_message(response)?;
    serde_json::from_str::<UnityTestDiscovery>(&message).map_err(|error| {
        UnityTestError::new(
            "unknown",
            format!("Invalid Unity test discovery response: {error}"),
        )
    })
}

pub async fn run_tests<F>(
    project_path: &str,
    request: UnityTestRunRequest,
    mut cancel_rx: watch::Receiver<bool>,
    mut on_progress: F,
) -> Result<UnityTestSnapshot, UnityTestError>
where
    F: FnMut(UnityTestProgress) + Send,
{
    let run_id = Uuid::new_v4().to_string();
    {
        let mut active = active_test_run().lock().await;
        if active.is_some() {
            return Err(UnityTestError::new(
                "busy",
                "A Unity test run is already active",
            ));
        }
        *active = Some(run_id.clone());
    }

    let started_at = Utc::now();
    let requested_scope = request.clone();
    let mut restore_play_mode_domain_reload = None;
    let result = match prepare_play_mode_domain_reload(project_path, &request).await {
        Ok(restore) => {
            restore_play_mode_domain_reload = restore;
            run_tests_inner(
                project_path,
                run_id.clone(),
                request,
                &mut cancel_rx,
                &mut on_progress,
            )
            .await
        }
        Err(error) => Err(error),
    };
    let finished_at = Utc::now();

    let snapshot = match result {
        Ok(mut snapshot) => {
            snapshot.finished_at = finished_at;
            snapshot
        }
        Err(error) => {
            let mut phase_summaries = error.partial_phases.clone();
            let results = flatten_phase_results(&mut phase_summaries);
            let total_summary = summarize(&phase_summaries);
            UnityTestSnapshot {
                run_id,
                started_at,
                finished_at,
                terminal_status: if error.code == "cancelled" {
                    "cancelled".to_string()
                } else if error.code == "compile_failed" {
                    "prepare_error".to_string()
                } else {
                    "runtime_error".to_string()
                },
                preparation: UnityTestPreparation {
                    method: "unknown".to_string(),
                    status: "error".to_string(),
                    message: Some(error.message.clone()),
                },
                requested_scope,
                phase_summaries,
                total_summary,
                results,
                error: Some(error),
            }
        }
    };

    let mut snapshot = snapshot;
    if let Ok(Some(previous)) = read_latest_snapshot(project_path) {
        merge_snapshot_results(&previous, &mut snapshot);
    }
    let _ = write_latest_snapshot(project_path, &snapshot);
    let _ = exit_play_mode(project_path).await;
    if let Some(domain_reload) = restore_play_mode_domain_reload {
        let _ =
            crate::unity_hotreload::coordinator::set_play_mode_reload(project_path, domain_reload)
                .await;
    }
    let mut active = active_test_run().lock().await;
    *active = None;

    if let Some(error) = snapshot.error.clone() {
        Err(error)
    } else {
        Ok(snapshot)
    }
}

async fn run_tests_inner<F>(
    project_path: &str,
    run_id: String,
    mut request: UnityTestRunRequest,
    cancel_rx: &mut watch::Receiver<bool>,
    on_progress: &mut F,
) -> Result<UnityTestSnapshot, UnityTestError>
where
    F: FnMut(UnityTestProgress) + Send,
{
    if *cancel_rx.borrow() {
        return Err(UnityTestError::new("cancelled", "Unity test run cancelled"));
    }

    let started_at = Utc::now();
    let requested_scope = request.clone();
    let modes = requested_phase_modes(&request);
    let preparation = prepare_for_run(project_path).await?;

    let mut phases = Vec::new();
    for mode in modes {
        prepare_for_phase(project_path, &mode).await?;
        request.filter.test_mode = mode.clone();
        match run_phase(project_path, &run_id, &request, cancel_rx, on_progress).await {
            Ok(mut phase) => {
                tag_phase_results(&mut phase);
                phases.push(phase);
            }
            Err(mut error) => {
                phases.append(&mut error.partial_phases);
                error.partial_phases = phases;
                return Err(error);
            }
        }
    }

    let results = flatten_phase_results(&mut phases);
    let total_summary = summarize(&phases);
    let terminal_status = if total_summary.failed > 0 {
        "completed_failed"
    } else {
        "completed"
    }
    .to_string();

    Ok(UnityTestSnapshot {
        run_id,
        started_at,
        finished_at: Utc::now(),
        terminal_status,
        preparation,
        requested_scope,
        phase_summaries: phases,
        total_summary,
        results,
        error: None,
    })
}

fn requested_phase_modes(request: &UnityTestRunRequest) -> Vec<UnityTestMode> {
    match request.filter.test_mode {
        UnityTestMode::Editmode => vec![UnityTestMode::Editmode],
        UnityTestMode::Playmode => vec![UnityTestMode::Playmode],
        UnityTestMode::All => vec![UnityTestMode::Editmode, UnityTestMode::Playmode],
    }
}

async fn prepare_play_mode_domain_reload(
    project_path: &str,
    request: &UnityTestRunRequest,
) -> Result<Option<bool>, UnityTestError> {
    if !requested_phase_modes(request)
        .iter()
        .any(|mode| *mode == UnityTestMode::Playmode)
    {
        return Ok(None);
    }

    let (_connected, _code_optimization, domain_reload_on_play) =
        crate::unity_hotreload::coordinator::detect_hot_reload_editor_settings(project_path).await;
    if domain_reload_on_play != Some(true) {
        return Ok(None);
    }

    let effective = crate::unity_hotreload::coordinator::set_play_mode_reload(project_path, false)
        .await
        .map_err(|error| {
            UnityTestError::new(
                "unknown",
                format!("Failed to disable Play Mode domain reload for Unity tests: {error}"),
            )
        })?;
    if effective {
        return Err(UnityTestError::new(
            "unknown",
            "Unity kept Play Mode domain reload enabled; PlayMode tests cannot run over the native bridge",
        ));
    }

    Ok(Some(true))
}

async fn prepare_for_run(project_path: &str) -> Result<UnityTestPreparation, UnityTestError> {
    let (connected, status, _) = query_unity_status(project_path).await;
    if !connected {
        return Err(UnityTestError::new(
            "unity_disconnected",
            "Unity Editor not connected",
        ));
    }

    let hot_reload = crate::unity_hotreload::coordinator::hot_reload(project_path, None).await;
    let mut method = "hot_reload".to_string();
    let mut message = hot_reload.as_ref().ok().cloned();
    if hot_reload.is_err() {
        method = "recompile".to_string();
        if is_play_mode_status(status) {
            exit_play_mode(project_path)
                .await
                .map_err(|error| UnityTestError::new("compile_failed", error))?;
        }
        message = Some(
            super::recompile_and_wait(project_path)
                .await
                .map_err(|error| UnityTestError::new("compile_failed", error))?,
        );
    }

    Ok(UnityTestPreparation {
        method,
        status: "ok".to_string(),
        message,
    })
}

async fn prepare_for_phase(
    project_path: &str,
    _mode: &UnityTestMode,
) -> Result<(), UnityTestError> {
    let (_, current, _) = query_unity_status(project_path).await;
    if is_play_mode_status(current) {
        exit_play_mode(project_path)
            .await
            .map_err(|error| UnityTestError::new("unknown", error))?;
    }
    Ok(())
}

async fn run_phase<F>(
    project_path: &str,
    run_id: &str,
    request: &UnityTestRunRequest,
    cancel_rx: &mut watch::Receiver<bool>,
    on_progress: &mut F,
) -> Result<UnityTestPhaseResult, UnityTestError>
where
    F: FnMut(UnityTestProgress) + Send,
{
    let effective_request = resolve_run_targets(project_path, request).await?;
    if request_has_selector(request) && effective_request.tests.is_empty() {
        return Ok(UnityTestPhaseResult {
            run_id: run_id.to_string(),
            test_mode: request.filter.test_mode.as_bridge_str().to_string(),
            status: "passed".to_string(),
            ..Default::default()
        });
    }

    let mut payload_value = serde_json::to_value(&effective_request)
        .map_err(|error| UnityTestError::new("unknown", error.to_string()))?;
    if let Some(object) = payload_value.as_object_mut() {
        object.insert(
            "runId".to_string(),
            serde_json::Value::String(run_id.to_string()),
        );
        object.insert(
            "testMode".to_string(),
            serde_json::Value::String(request.filter.test_mode.as_bridge_str().to_string()),
        );
    }
    let payload = payload_value.to_string();
    let phase = request.filter.test_mode.as_bridge_str().to_string();
    on_progress(UnityTestProgress {
        active: true,
        run_id: run_id.to_string(),
        phase: phase.clone(),
        current_test: String::new(),
        completed: 0,
        total: 0,
        failed: 0,
        revision: 0,
    });

    let send = send_message_without_timeout(project_path, "run_tests", &payload);
    tokio::pin!(send);
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let started = Instant::now();

    loop {
        tokio::select! {
            response = &mut send => {
                return parse_phase_response(response.map_err(map_transport_error)?);
            }
            _ = interval.tick() => {
                if started.elapsed() > Duration::from_millis(300) {
                    if let Ok(Some(progress)) = query_progress(project_path).await {
                        if progress.active {
                            on_progress(progress);
                        }
                    }
                }
            }
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    let _ = cancel_tests(project_path).await;
                    let partial = tokio::time::timeout(Duration::from_secs(5), &mut send)
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .and_then(|response| parse_phase_response(response).ok());
                    let error = UnityTestError::new("cancelled", "Unity test run cancelled");
                    return Err(match partial {
                        Some(phase) => error.with_partial_phase(phase),
                        None => error,
                    });
                }
            }
        }
    }
}

fn parse_phase_response(response: PipeResponse) -> Result<UnityTestPhaseResult, UnityTestError> {
    let message = ok_message(response)?;
    let phase_result = serde_json::from_str::<UnityTestPhaseResult>(&message).map_err(|error| {
        UnityTestError::new(
            "unknown",
            format!("Invalid Unity test phase response: {error}"),
        )
    })?;
    if matches!(phase_result.status.as_str(), "passed" | "failed") {
        return Ok(phase_result);
    }
    let code = phase_result.error_code.as_deref().unwrap_or("").trim();
    let code = if code.is_empty() { "unknown" } else { code };
    let error_message = phase_result.error_message.as_deref().unwrap_or("").trim();
    let message = if error_message.is_empty() {
        format!(
            "Unity test phase '{}' ended with status '{}'",
            phase_result.test_mode, phase_result.status
        )
    } else {
        error_message.to_string()
    };
    Err(UnityTestError::new(code, message))
}

async fn resolve_run_targets(
    project_path: &str,
    request: &UnityTestRunRequest,
) -> Result<UnityTestRunRequest, UnityTestError> {
    if !request_has_selector(request) {
        return Ok(request.clone());
    }

    let discovery_filter = if request.tests.is_empty() {
        request.filter.clone()
    } else {
        UnityTestFilter {
            test_mode: request.filter.test_mode.clone(),
            ..Default::default()
        }
    };
    let discovery = find_tests(project_path, discovery_filter).await?;
    let mut next = request.clone();
    next.filter.assembly_name = None;
    next.filter.search = None;
    next.filter.fixture_name = None;
    next.filter.test_name = None;
    next.tests = resolve_targets_from_discovery(&discovery, request);
    Ok(next)
}

fn resolve_targets_from_discovery(
    discovery: &UnityTestDiscovery,
    request: &UnityTestRunRequest,
) -> Vec<UnityTestTarget> {
    discovery
        .assemblies
        .iter()
        .flat_map(|assembly| {
            let assembly_name = assembly.name.clone();
            assembly.fixtures.iter().flat_map(move |fixture| {
                let assembly_name = assembly_name.clone();
                let fixture_name = fixture.name.clone();
                fixture.tests.iter().filter_map(move |test| {
                    if !filter_matches_discovered_test(
                        &request.filter,
                        &assembly_name,
                        &fixture_name,
                        test,
                    ) {
                        return None;
                    }

                    if !request.tests.is_empty()
                        && !request.tests.iter().any(|target| {
                            target_matches_discovered_test(
                                target,
                                &assembly_name,
                                &fixture_name,
                                test,
                            )
                        })
                    {
                        return None;
                    }

                    Some(UnityTestTarget {
                        assembly_name: Some(assembly_name.clone()),
                        fixture_name: Some(fixture_name.clone()),
                        test_name: Some(if test.full_name.trim().is_empty() {
                            test.name.clone()
                        } else {
                            test.full_name.clone()
                        }),
                    })
                })
            })
        })
        .collect()
}

fn filter_matches_discovered_test(
    filter: &UnityTestFilter,
    assembly_name: &str,
    fixture_name: &str,
    test: &UnityTestMethod,
) -> bool {
    matches_optional_exact(&filter.assembly_name, assembly_name)
        && matches_optional_fixture(&filter.fixture_name, fixture_name)
        && matches_optional_test_name(&filter.test_name, fixture_name, test)
        && matches_optional_search(&filter.search, assembly_name, fixture_name, test)
}

fn target_matches_discovered_test(
    target: &UnityTestTarget,
    assembly_name: &str,
    fixture_name: &str,
    test: &UnityTestMethod,
) -> bool {
    matches_optional_exact(&target.assembly_name, assembly_name)
        && matches_optional_fixture(&target.fixture_name, fixture_name)
        && matches_optional_test_name(&target.test_name, fixture_name, test)
}

fn matches_optional_exact(expected: &Option<String>, actual: &str) -> bool {
    match expected
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(expected) => actual.eq_ignore_ascii_case(expected),
        None => true,
    }
}

fn matches_optional_fixture(expected: &Option<String>, actual: &str) -> bool {
    match expected
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(expected) => {
            actual.eq_ignore_ascii_case(expected)
                || actual
                    .rsplit('.')
                    .next()
                    .is_some_and(|short| short.eq_ignore_ascii_case(expected))
        }
        None => true,
    }
}

fn matches_optional_test_name(
    expected: &Option<String>,
    fixture_name: &str,
    test: &UnityTestMethod,
) -> bool {
    match expected
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(expected) => {
            test.name.eq_ignore_ascii_case(expected)
                || test.full_name.eq_ignore_ascii_case(expected)
                || format!("{}.{}", fixture_name, test.name).eq_ignore_ascii_case(expected)
        }
        None => true,
    }
}

fn matches_optional_search(
    expected: &Option<String>,
    assembly_name: &str,
    fixture_name: &str,
    test: &UnityTestMethod,
) -> bool {
    match expected
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(expected) => {
            contains_ignore_case(assembly_name, expected)
                || contains_ignore_case(fixture_name, expected)
                || contains_ignore_case(&test.name, expected)
                || contains_ignore_case(&test.full_name, expected)
                || test
                    .source_path
                    .as_deref()
                    .is_some_and(|path| contains_ignore_case(path, expected))
        }
        None => true,
    }
}

fn contains_ignore_case(actual: &str, expected: &str) -> bool {
    actual.to_lowercase().contains(&expected.to_lowercase())
}

fn request_has_selector(request: &UnityTestRunRequest) -> bool {
    request_has_search(request)
        || !request.tests.is_empty()
        || request
            .filter
            .assembly_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || request
            .filter
            .fixture_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || request
            .filter
            .test_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn request_has_search(request: &UnityTestRunRequest) -> bool {
    request
        .filter
        .search
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
}

pub async fn query_progress(
    project_path: &str,
) -> Result<Option<UnityTestProgress>, UnityTestError> {
    let response = transport::send_message_if_writer_free(
        project_path,
        "test_run_progress",
        "",
        Duration::from_millis(450),
    )
    .await
    .map_err(map_transport_error)?;
    let Some(response) = response else {
        return Ok(None);
    };
    if !response.ok {
        return Ok(None);
    }
    let Some(message) = response.message else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<UnityTestProgress>(&message).ok())
}

pub async fn cancel_tests(project_path: &str) -> Result<(), UnityTestError> {
    let response = send_message(project_path, "cancel_tests", "")
        .await
        .map_err(map_transport_error)?;
    ok_message(response).map(|_| ())
}

fn summarize(phases: &[UnityTestPhaseResult]) -> UnityTestSummary {
    phases
        .iter()
        .fold(UnityTestSummary::default(), |mut acc, phase| {
            acc.total += phase.total;
            acc.passed += phase.passed;
            acc.failed += phase.failed;
            acc.skipped += phase.skipped;
            acc.duration += phase.duration;
            acc
        })
}

fn tag_phase_results(phase: &mut UnityTestPhaseResult) {
    for result in &mut phase.results {
        if result.test_mode.trim().is_empty() {
            result.test_mode = phase.test_mode.clone();
        }
    }
}

fn flatten_phase_results(phases: &mut [UnityTestPhaseResult]) -> Vec<UnityTestResult> {
    for phase in phases.iter_mut() {
        tag_phase_results(phase);
    }
    phases
        .iter()
        .flat_map(|phase| phase.results.clone())
        .collect()
}

fn result_identity(result: &UnityTestResult) -> String {
    let name = if result.full_name.trim().is_empty() {
        result.test_name.trim()
    } else {
        result.full_name.trim()
    };
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        result.test_mode.trim().to_lowercase(),
        result.assembly_name.trim().to_lowercase(),
        result.fixture_name.trim().to_lowercase(),
        name.to_lowercase(),
    )
}

fn snapshot_results_with_modes(snapshot: &UnityTestSnapshot) -> Vec<UnityTestResult> {
    snapshot
        .results
        .iter()
        .cloned()
        .map(|mut result| {
            if result.test_mode.trim().is_empty() {
                if let Some(phase) = snapshot.phase_summaries.iter().find(|phase| {
                    phase.results.iter().any(|candidate| {
                        candidate
                            .assembly_name
                            .eq_ignore_ascii_case(&result.assembly_name)
                            && candidate
                                .fixture_name
                                .eq_ignore_ascii_case(&result.fixture_name)
                            && candidate.full_name.eq_ignore_ascii_case(&result.full_name)
                            && candidate.test_name.eq_ignore_ascii_case(&result.test_name)
                    })
                }) {
                    result.test_mode = phase.test_mode.clone();
                } else if snapshot.requested_scope.filter.test_mode != UnityTestMode::All {
                    result.test_mode = snapshot
                        .requested_scope
                        .filter
                        .test_mode
                        .as_bridge_str()
                        .to_string();
                }
            }
            result
        })
        .collect()
}

fn merge_snapshot_results(previous: &UnityTestSnapshot, current: &mut UnityTestSnapshot) {
    let mut merged = std::collections::BTreeMap::new();
    for result in snapshot_results_with_modes(previous) {
        merged.insert(result_identity(&result), result);
    }
    for result in snapshot_results_with_modes(current) {
        merged.insert(result_identity(&result), result);
    }
    current.results = merged.into_values().collect();
}

fn ok_message(response: PipeResponse) -> Result<String, UnityTestError> {
    if response.ok {
        return Ok(response.message.unwrap_or_default());
    }
    let message = response
        .error
        .unwrap_or_else(|| "Unity test request failed".to_string());
    let code = match message.as_str() {
        "busy" => "busy",
        _ if message.starts_with("test_framework_missing") => "test_framework_missing",
        _ if message.contains("pipe") || message.contains("disconnected") => "unity_disconnected",
        _ => "unknown",
    };
    Err(UnityTestError::new(code, message))
}

fn map_transport_error(error: String) -> UnityTestError {
    let code =
        if error.contains("disconnected") || error.contains("connect") || error.contains("pipe") {
            "unity_disconnected"
        } else {
            "unknown"
        };
    UnityTestError::new(code, error)
}

fn latest_snapshot_path(project_path: &str) -> PathBuf {
    Path::new(project_path)
        .join("Locus")
        .join("test-results")
        .join("latest.json")
}

pub fn write_latest_snapshot(
    project_path: &str,
    snapshot: &UnityTestSnapshot,
) -> Result<(), UnityTestError> {
    let path = latest_snapshot_path(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            UnityTestError::new(
                "unknown",
                format!("Failed to create test-results directory: {error}"),
            )
        })?;
    }
    let json = serde_json::to_string_pretty(snapshot)
        .map_err(|error| UnityTestError::new("unknown", error.to_string()))?;
    fs::write(&path, json).map_err(|error| {
        UnityTestError::new(
            "unknown",
            format!("Failed to write latest Unity test snapshot: {error}"),
        )
    })
}

pub fn read_latest_snapshot(
    project_path: &str,
) -> Result<Option<UnityTestSnapshot>, UnityTestError> {
    let path = latest_snapshot_path(project_path);
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(&path).map_err(|error| {
        UnityTestError::new(
            "unknown",
            format!("Failed to read latest Unity test snapshot: {error}"),
        )
    })?;
    serde_json::from_str::<UnityTestSnapshot>(&json)
        .map(Some)
        .map_err(|error| {
            UnityTestError::new(
                "unknown",
                format!("Invalid latest Unity test snapshot: {error}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_mode_runs_editmode_before_playmode() {
        let request = UnityTestRunRequest::default();
        assert_eq!(
            requested_phase_modes(&request),
            vec![UnityTestMode::Editmode, UnityTestMode::Playmode]
        );
    }

    #[test]
    fn summarizes_phase_results() {
        let summary = summarize(&[
            UnityTestPhaseResult {
                total: 2,
                passed: 1,
                failed: 1,
                skipped: 0,
                duration: 1.5,
                ..Default::default()
            },
            UnityTestPhaseResult {
                total: 1,
                passed: 0,
                failed: 0,
                skipped: 1,
                duration: 0.25,
                ..Default::default()
            },
        ]);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.duration, 1.75);
    }

    #[test]
    fn incremental_snapshot_merge_replaces_only_results_from_the_current_run() {
        let previous = test_snapshot(vec![
            test_result("editmode", "Game.Tests.A", "failed"),
            test_result("editmode", "Game.Tests.B", "passed"),
            test_result("playmode", "Game.Tests.A", "passed"),
        ]);
        let mut current = test_snapshot(vec![test_result("editmode", "Game.Tests.A", "passed")]);

        merge_snapshot_results(&previous, &mut current);

        assert_eq!(current.results.len(), 3);
        assert_eq!(
            current
                .results
                .iter()
                .find(|result| result.test_mode == "editmode" && result.full_name == "Game.Tests.A")
                .map(|result| result.outcome.as_str()),
            Some("passed")
        );
        assert!(current.results.iter().any(|result| {
            result.test_mode == "editmode"
                && result.full_name == "Game.Tests.B"
                && result.outcome == "passed"
        }));
        assert!(current.results.iter().any(|result| {
            result.test_mode == "playmode"
                && result.full_name == "Game.Tests.A"
                && result.outcome == "passed"
        }));
    }

    fn test_result(test_mode: &str, full_name: &str, outcome: &str) -> UnityTestResult {
        let fixture_name = full_name
            .rsplit_once('.')
            .map(|(fixture, _)| fixture)
            .unwrap_or("");
        UnityTestResult {
            test_mode: test_mode.to_string(),
            assembly_name: format!("Game.{test_mode}.Tests"),
            fixture_name: fixture_name.to_string(),
            test_name: full_name
                .rsplit('.')
                .next()
                .unwrap_or(full_name)
                .to_string(),
            full_name: full_name.to_string(),
            outcome: outcome.to_string(),
            ..Default::default()
        }
    }

    fn test_snapshot(results: Vec<UnityTestResult>) -> UnityTestSnapshot {
        UnityTestSnapshot {
            run_id: "run".to_string(),
            started_at: Utc::now(),
            finished_at: Utc::now(),
            terminal_status: "completed".to_string(),
            preparation: UnityTestPreparation {
                method: "none".to_string(),
                status: "ok".to_string(),
                message: None,
            },
            requested_scope: UnityTestRunRequest::default(),
            phase_summaries: Vec::new(),
            total_summary: UnityTestSummary::default(),
            results,
            error: None,
        }
    }

    #[test]
    fn cancelled_errors_carry_partial_phases_without_changing_wire_error_shape() {
        let error = UnityTestError::new("cancelled", "cancelled").with_partial_phase(
            UnityTestPhaseResult {
                test_mode: "editmode".to_string(),
                total: 2,
                passed: 1,
                failed: 1,
                ..Default::default()
            },
        );

        assert_eq!(error.partial_phases.len(), 1);
        assert_eq!(summarize(&error.partial_phases).total, 2);
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({
                "code": "cancelled",
                "message": "cancelled"
            })
        );
    }

    #[test]
    fn unity_test_arguments_accept_tool_snake_case_fields() {
        let request: UnityTestRunRequest = serde_json::from_value(json!({
            "test_mode": "editmode",
            "assembly_name": "EditMode",
            "fixture_name": "ArchiveSessionRootTests",
            "test_name": "Load_ReturnsFalse_WhenNoActiveArchive",
            "tests": [{
                "assembly_name": "EditMode",
                "fixture_name": "SolarFood.Tests.ArchiveSessionRootTests",
                "test_name": "Load_ReturnsFalse_WhenNoActiveArchive"
            }]
        }))
        .expect("parse snake_case tool arguments");

        assert_eq!(request.filter.test_mode, UnityTestMode::Editmode);
        assert_eq!(request.filter.assembly_name.as_deref(), Some("EditMode"));
        assert_eq!(
            request.filter.fixture_name.as_deref(),
            Some("ArchiveSessionRootTests")
        );
        assert_eq!(
            request.filter.test_name.as_deref(),
            Some("Load_ReturnsFalse_WhenNoActiveArchive")
        );
        assert_eq!(request.tests.len(), 1);
        assert_eq!(
            request.tests[0].fixture_name.as_deref(),
            Some("SolarFood.Tests.ArchiveSessionRootTests")
        );
    }

    #[test]
    fn unity_test_arguments_accept_bridge_camel_case_fields() {
        let request: UnityTestRunRequest = serde_json::from_value(json!({
            "testMode": "playmode",
            "assemblyName": "PlayMode",
            "fixtureName": "SolarFood.Tests.BootLoaderFreshPlayTests",
            "testName": "TryAutoResolveArchive_DoesNotLoadExistingArchive_WhenFreshPlaySkipAutoResolve"
        }))
        .expect("parse camelCase bridge arguments");

        assert_eq!(request.filter.test_mode, UnityTestMode::Playmode);
        assert_eq!(request.filter.assembly_name.as_deref(), Some("PlayMode"));
        assert_eq!(
            request.filter.fixture_name.as_deref(),
            Some("SolarFood.Tests.BootLoaderFreshPlayTests")
        );
        assert_eq!(
            request.filter.test_name.as_deref(),
            Some("TryAutoResolveArchive_DoesNotLoadExistingArchive_WhenFreshPlaySkipAutoResolve")
        );
    }

    #[test]
    fn explicit_targets_match_full_fixture_and_test_name() {
        let discovery = sample_discovery();
        let request = UnityTestRunRequest {
            filter: UnityTestFilter {
                test_mode: UnityTestMode::All,
                ..Default::default()
            },
            tests: vec![UnityTestTarget {
                assembly_name: Some("EditMode".to_string()),
                fixture_name: Some("SolarFood.Tests.ArchiveSessionRootTests".to_string()),
                test_name: Some("Load_ReturnsFalse_WhenNoActiveArchive".to_string()),
            }],
        };

        let targets = resolve_targets_from_discovery(&discovery, &request);
        assert_eq!(
            target_names(&targets),
            vec!["SolarFood.Tests.ArchiveSessionRootTests.Load_ReturnsFalse_WhenNoActiveArchive"]
        );
    }

    #[test]
    fn explicit_targets_match_short_fixture_name() {
        let discovery = sample_discovery();
        let request = UnityTestRunRequest {
            filter: UnityTestFilter {
                test_mode: UnityTestMode::All,
                ..Default::default()
            },
            tests: vec![UnityTestTarget {
                assembly_name: Some("EditMode".to_string()),
                fixture_name: Some("ArchiveSessionRootTests".to_string()),
                test_name: Some("Load_ReturnsFalse_WhenNoActiveArchive".to_string()),
            }],
        };

        let targets = resolve_targets_from_discovery(&discovery, &request);
        assert_eq!(
            target_names(&targets),
            vec!["SolarFood.Tests.ArchiveSessionRootTests.Load_ReturnsFalse_WhenNoActiveArchive"]
        );
    }

    #[test]
    fn non_matching_explicit_target_stays_empty_instead_of_broadening() {
        let discovery = sample_discovery();
        let request = UnityTestRunRequest {
            filter: UnityTestFilter {
                test_mode: UnityTestMode::All,
                ..Default::default()
            },
            tests: vec![UnityTestTarget {
                assembly_name: Some("EditMode".to_string()),
                fixture_name: Some("NonExistingFixture".to_string()),
                test_name: None,
            }],
        };

        let targets = resolve_targets_from_discovery(&discovery, &request);
        assert!(targets.is_empty());
    }

    #[test]
    fn filter_selectors_resolve_expected_targets_without_broadening() {
        let cases = vec![
            (
                "assembly editmode",
                UnityTestFilter {
                    assembly_name: Some("EditMode".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST, OTHER_TEST, CASE_TEST],
            ),
            (
                "assembly playmode",
                UnityTestFilter {
                    assembly_name: Some("PlayMode".to_string()),
                    ..Default::default()
                },
                vec![PLAYMODE_TEST],
            ),
            (
                "assembly case-insensitive with whitespace",
                UnityTestFilter {
                    assembly_name: Some(" editmode ".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST, OTHER_TEST, CASE_TEST],
            ),
            (
                "fixture short name",
                UnityTestFilter {
                    fixture_name: Some("ArchiveSessionRootTests".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST],
            ),
            (
                "fixture full name",
                UnityTestFilter {
                    fixture_name: Some("SolarFood.Tests.ArchiveSessionRootTests".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST],
            ),
            (
                "fixture case-insensitive with whitespace",
                UnityTestFilter {
                    fixture_name: Some(" archivesessionroottests ".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST],
            ),
            (
                "test short name",
                UnityTestFilter {
                    test_name: Some("Load_ReturnsFalse_WhenNoActiveArchive".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST],
            ),
            (
                "test full name",
                UnityTestFilter {
                    test_name: Some(ARCHIVE_ROOT_TEST.to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST],
            ),
            (
                "test case-insensitive with whitespace",
                UnityTestFilter {
                    test_name: Some(" load_returnsfalse_whennoactivearchive ".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST],
            ),
            (
                "search fixture",
                UnityTestFilter {
                    search: Some("ArchiveSessionRoot".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST],
            ),
            (
                "search source path",
                UnityTestFilter {
                    search: Some("BootLoaderFreshPlayTests.cs".to_string()),
                    ..Default::default()
                },
                vec![PLAYMODE_TEST],
            ),
            (
                "combined assembly fixture test",
                UnityTestFilter {
                    assembly_name: Some("EditMode".to_string()),
                    fixture_name: Some("ArchiveSessionRootTests".to_string()),
                    test_name: Some("Save_WritesArchive".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_SAVE_TEST],
            ),
            (
                "combined fixture search",
                UnityTestFilter {
                    fixture_name: Some("ArchiveSessionRootTests".to_string()),
                    search: Some("Save".to_string()),
                    ..Default::default()
                },
                vec![ARCHIVE_SAVE_TEST],
            ),
            (
                "non-existing assembly",
                UnityTestFilter {
                    assembly_name: Some("NonExistingAssembly".to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ),
            (
                "non-existing fixture",
                UnityTestFilter {
                    fixture_name: Some("NonExistingFixture".to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ),
            (
                "non-existing test",
                UnityTestFilter {
                    test_name: Some("NonExistingTest".to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ),
            (
                "non-existing combined selector",
                UnityTestFilter {
                    assembly_name: Some("PlayMode".to_string()),
                    fixture_name: Some("ArchiveSessionRootTests".to_string()),
                    ..Default::default()
                },
                Vec::new(),
            ),
        ];

        let discovery = sample_discovery();
        for (label, filter, expected) in cases {
            let request = UnityTestRunRequest {
                filter,
                tests: Vec::new(),
            };
            let targets = resolve_targets_from_discovery(&discovery, &request);
            assert_eq!(target_names(&targets), expected, "{label}");
        }
    }

    #[test]
    fn explicit_target_selectors_resolve_expected_targets_without_broadening() {
        let cases = vec![
            (
                "assembly only",
                UnityTestTarget {
                    assembly_name: Some("PlayMode".to_string()),
                    fixture_name: None,
                    test_name: None,
                },
                vec![PLAYMODE_TEST],
            ),
            (
                "fixture only short name",
                UnityTestTarget {
                    assembly_name: None,
                    fixture_name: Some("ArchiveSessionRootTests".to_string()),
                    test_name: None,
                },
                vec![ARCHIVE_ROOT_TEST, ARCHIVE_SAVE_TEST],
            ),
            (
                "test only",
                UnityTestTarget {
                    assembly_name: None,
                    fixture_name: None,
                    test_name: Some("OtherTest".to_string()),
                },
                vec![OTHER_TEST],
            ),
            (
                "test full name",
                UnityTestTarget {
                    assembly_name: None,
                    fixture_name: None,
                    test_name: Some(PLAYMODE_TEST.to_string()),
                },
                vec![PLAYMODE_TEST],
            ),
            (
                "case-insensitive full target",
                UnityTestTarget {
                    assembly_name: Some("editmode".to_string()),
                    fixture_name: Some("archivesessionroottests".to_string()),
                    test_name: Some("load_returnsfalse_whennoactivearchive".to_string()),
                },
                vec![ARCHIVE_ROOT_TEST],
            ),
            (
                "non-existing explicit target",
                UnityTestTarget {
                    assembly_name: Some("EditMode".to_string()),
                    fixture_name: Some("NonExistingFixture".to_string()),
                    test_name: None,
                },
                Vec::new(),
            ),
        ];

        let discovery = sample_discovery();
        for (label, target, expected) in cases {
            let request = UnityTestRunRequest {
                filter: UnityTestFilter {
                    test_mode: UnityTestMode::All,
                    ..Default::default()
                },
                tests: vec![target],
            };
            let targets = resolve_targets_from_discovery(&discovery, &request);
            assert_eq!(target_names(&targets), expected, "{label}");
        }
    }

    #[test]
    fn explicit_targets_are_intersected_with_simple_filters() {
        let discovery = sample_discovery();
        let request = UnityTestRunRequest {
            filter: UnityTestFilter {
                assembly_name: Some("EditMode".to_string()),
                fixture_name: Some("ArchiveSessionRootTests".to_string()),
                ..Default::default()
            },
            tests: vec![
                UnityTestTarget {
                    assembly_name: None,
                    fixture_name: None,
                    test_name: Some("Load_ReturnsFalse_WhenNoActiveArchive".to_string()),
                },
                UnityTestTarget {
                    assembly_name: None,
                    fixture_name: None,
                    test_name: Some("OtherTest".to_string()),
                },
            ],
        };

        let targets = resolve_targets_from_discovery(&discovery, &request);
        assert_eq!(target_names(&targets), vec![ARCHIVE_ROOT_TEST]);
    }

    #[test]
    fn selector_detection_covers_all_query_inputs() {
        let mut request = UnityTestRunRequest::default();
        assert!(!request_has_selector(&request));

        request.filter.assembly_name = Some("EditMode".to_string());
        assert!(request_has_selector(&request));

        request = UnityTestRunRequest::default();
        request.filter.fixture_name = Some("ArchiveSessionRootTests".to_string());
        assert!(request_has_selector(&request));

        request = UnityTestRunRequest::default();
        request.filter.test_name = Some("Load_ReturnsFalse_WhenNoActiveArchive".to_string());
        assert!(request_has_selector(&request));

        request = UnityTestRunRequest::default();
        request.filter.search = Some("Archive".to_string());
        assert!(request_has_selector(&request));

        request = UnityTestRunRequest::default();
        request.tests.push(UnityTestTarget {
            test_name: Some("OtherTest".to_string()),
            ..Default::default()
        });
        assert!(request_has_selector(&request));
    }

    const ARCHIVE_ROOT_TEST: &str =
        "SolarFood.Tests.ArchiveSessionRootTests.Load_ReturnsFalse_WhenNoActiveArchive";
    const ARCHIVE_SAVE_TEST: &str = "SolarFood.Tests.ArchiveSessionRootTests.Save_WritesArchive";
    const OTHER_TEST: &str = "SolarFood.Tests.OtherTests.OtherTest";
    const CASE_TEST: &str = "SolarFood.Tests.MixedCaseTests.CaseSensitiveName";
    const PLAYMODE_TEST: &str = "SolarFood.Tests.BootLoaderFreshPlayTests.TryAutoResolveArchive_DoesNotLoadExistingArchive_WhenFreshPlaySkipAutoResolve";

    fn sample_discovery() -> UnityTestDiscovery {
        UnityTestDiscovery {
            assemblies: vec![
                UnityTestAssembly {
                    name: "EditMode".to_string(),
                    test_mode: "editmode".to_string(),
                    fixtures: vec![
                        UnityTestFixture {
                            name: "SolarFood.Tests.ArchiveSessionRootTests".to_string(),
                            tests: vec![
                                UnityTestMethod {
                                    name: "Load_ReturnsFalse_WhenNoActiveArchive".to_string(),
                                    full_name: ARCHIVE_ROOT_TEST.to_string(),
                                    source_path: Some(
                                        "Assets/Scripts/Tests/EditMode/ArchiveSessionRootTests.cs"
                                            .to_string(),
                                    ),
                                    ..Default::default()
                                },
                                UnityTestMethod {
                                    name: "Save_WritesArchive".to_string(),
                                    full_name: ARCHIVE_SAVE_TEST.to_string(),
                                    source_path: Some(
                                        "Assets/Scripts/Tests/EditMode/ArchiveSessionRootTests.cs"
                                            .to_string(),
                                    ),
                                    ..Default::default()
                                },
                            ],
                        },
                        UnityTestFixture {
                            name: "SolarFood.Tests.OtherTests".to_string(),
                            tests: vec![UnityTestMethod {
                                name: "OtherTest".to_string(),
                                full_name: OTHER_TEST.to_string(),
                                source_path: Some(
                                    "Assets/Scripts/Tests/EditMode/OtherTests.cs".to_string(),
                                ),
                                ..Default::default()
                            }],
                        },
                        UnityTestFixture {
                            name: "SolarFood.Tests.MixedCaseTests".to_string(),
                            tests: vec![UnityTestMethod {
                                name: "CaseSensitiveName".to_string(),
                                full_name: CASE_TEST.to_string(),
                                source_path: Some(
                                    "Assets/Scripts/Tests/EditMode/MixedCaseTests.cs".to_string(),
                                ),
                                ..Default::default()
                            }],
                        },
                    ],
                },
                UnityTestAssembly {
                    name: "PlayMode".to_string(),
                    test_mode: "playmode".to_string(),
                    fixtures: vec![UnityTestFixture {
                        name: "SolarFood.Tests.BootLoaderFreshPlayTests".to_string(),
                        tests: vec![UnityTestMethod {
                            name: "TryAutoResolveArchive_DoesNotLoadExistingArchive_WhenFreshPlaySkipAutoResolve".to_string(),
                            full_name: PLAYMODE_TEST.to_string(),
                            source_path: Some(
                                "Assets/Scripts/Tests/PlayMode/BootLoaderFreshPlayTests.cs"
                                    .to_string(),
                            ),
                            ..Default::default()
                        }],
                    }],
                },
            ],
        }
    }

    fn target_names(targets: &[UnityTestTarget]) -> Vec<String> {
        targets
            .iter()
            .map(|target| {
                let fixture = target.fixture_name.as_deref().unwrap_or("");
                let test = target.test_name.as_deref().unwrap_or("");
                if test.starts_with(&format!("{fixture}.")) {
                    test.to_string()
                } else {
                    format!("{fixture}.{test}")
                }
            })
            .collect()
    }
}
