//! Native broker bridge self-test.
//!
//! The test is intentionally live: it verifies the configured marker, talks to
//! the broker over the native shared-memory status plane, checks the combined native/managed
//! capability surface, then requests a real Unity script reload and confirms
//! the native pipe survives until the managed executor reports ready again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{Emitter, Listener};

use super::NativeBrokerStatus;

static RUNNING: AtomicBool = AtomicBool::new(false);

const EVENT_NAME: &str = "unity-native-bridge-selftest";
const STATUS_WAIT_TIMEOUT: Duration = Duration::from_secs(45);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(120);
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(20);
const EDITOR_STATUS_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const EDITOR_STATUS_WAIT_TIMEOUT: Duration = Duration::from_secs(45);
const EDITOR_UPDATE_WAIT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTestEvent {
    running: bool,
    finished: bool,
    line: Option<String>,
    passed: u32,
    failed: u32,
}

struct RunningGuard;

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

struct SelfTest {
    app: tauri::AppHandle,
    project: String,
    passed: u32,
    failed: u32,
}

impl SelfTest {
    fn emit(&self, line: Option<String>, finished: bool) {
        let _ = self.app.emit(
            EVENT_NAME,
            SelfTestEvent {
                running: !finished,
                finished,
                line,
                passed: self.passed,
                failed: self.failed,
            },
        );
    }

    fn log(&self, line: impl Into<String>) {
        let line = line.into();
        if line.starts_with("FAIL  ") {
            tracing::error!(log_module = "NativeBridge SelfTest", "{line}");
        } else {
            tracing::info!(log_module = "NativeBridge SelfTest", "{line}");
        }
        self.emit(Some(line), false);
    }

    fn pass(&mut self, name: &str, detail: impl Into<String>) {
        self.passed += 1;
        self.log(format!("PASS  {name}: {}", detail.into()));
    }

    fn fail(&mut self, name: &str, detail: impl Into<String>) {
        self.failed += 1;
        self.log(format!("FAIL  {name}: {}", detail.into()));
    }

    fn check_feature_enabled(&mut self) -> bool {
        if super::native_bridge_enabled() {
            self.pass("N1 native bridge flag", "enabled in runtime config");
            true
        } else {
            self.fail(
                "N1 native bridge flag",
                "enable Native Plugin Bridge in Settings > Testing before running this test",
            );
            false
        }
    }

    fn check_installed_plugin_files(&mut self) -> bool {
        match super::check_plugin_status(&self.project) {
            Ok(super::PluginStatus::UpToDate) => {
                self.pass(
                    "N2 Unity plugin files",
                    "installed plugin matches this Locus build",
                );
                true
            }
            Ok(status) => {
                self.fail(
                    "N2 Unity plugin files",
                    format!(
                        "installed plugin is {:?}; update the Locus Unity plugin and wait for Unity to compile it",
                        status
                    ),
                );
                false
            }
            Err(error) => {
                self.fail(
                    "N2 Unity plugin files",
                    format!("could not verify installed plugin files: {error}"),
                );
                false
            }
        }
    }

    fn check_marker(&mut self) -> bool {
        if let Err(error) = super::sync_native_bridge_marker(&self.project, true) {
            self.fail("N3 native marker", error);
            return false;
        }

        let marker = std::path::Path::new(super::strip_extended_path_prefix(&self.project))
            .join("Library")
            .join("Locus")
            .join("NativeBridge.enabled");
        let expected = super::get_native_pipe_name(&self.project);
        let actual = std::fs::read_to_string(&marker)
            .map(|value| value.trim().to_string())
            .unwrap_or_default();
        if actual == expected {
            self.pass("N3 native marker", format!("pipe={actual}"));
            true
        } else {
            self.fail(
                "N3 native marker",
                format!("expected '{expected}', got '{actual}'"),
            );
            false
        }
    }

    async fn request_script_reload(&self) -> Result<(), String> {
        let code = "UnityEditor.EditorUtility.RequestScriptReload(); return \"requested\";";
        match tokio::time::timeout(
            EXECUTE_TIMEOUT,
            super::unity_execute_code(&self.project, code),
        )
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) if reload_request_was_accepted(&error) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(format!(
                "unity_execute_code timed out after {}s",
                EXECUTE_TIMEOUT.as_secs()
            )),
        }
    }

    async fn wait_for_native_shared_status(&self, timeout: Duration) -> Option<NativeBrokerStatus> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(status) = super::query_native_broker_status(&self.project).await {
                return Some(status);
            }
            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        }
        None
    }

    async fn ensure_broker_ready(&mut self) -> Option<NativeBrokerStatus> {
        let mut status = self
            .wait_for_native_shared_status(Duration::from_secs(3))
            .await;
        if status.is_none() {
            self.log("native broker not visible yet; requesting script reload to load marker");
            if let Err(error) = self.request_script_reload().await {
                self.fail("N4 native broker visible", error);
                return None;
            }
            status = self
                .wait_for_native_shared_status(STATUS_WAIT_TIMEOUT)
                .await;
        }

        let Some(status) = status else {
            self.fail(
                "N4 native broker visible",
                "native shared-memory status did not appear before timeout",
            );
            return None;
        };

        if status.native_alive {
            self.pass(
                "N4 native broker visible",
                format!(
                    "managedState={} generation={} protocol={}",
                    status.managed_state, status.domain_generation, status.protocol_version
                ),
            );
        } else {
            self.fail("N4 native broker visible", "nativeAlive=false");
            return None;
        }

        let start = Instant::now();
        let mut current = status;
        loop {
            if current.managed_state == "ready" {
                self.pass(
                    "N5 managed executor ready",
                    format!("editorStatus={}", current.editor_status),
                );
                return Some(current);
            }

            if start.elapsed() > STATUS_WAIT_TIMEOUT {
                self.fail(
                    "N5 managed executor ready",
                    format!("managedState={}", current.managed_state),
                );
                return None;
            }

            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
            if let Some(next) = super::query_native_broker_status(&self.project).await {
                current = next;
            }
        }
    }

    async fn check_capability_route(&mut self) {
        match super::send_message_with_timeout(
            &self.project,
            "bridge_capabilities",
            "",
            Duration::from_secs(3),
        )
        .await
        {
            Ok(resp) if resp.ok => {
                let message = resp.message.unwrap_or_default();
                if message.contains("broker_v1")
                    && message.contains("managed_executor_v1")
                    && message.contains("status_cached")
                    && message.contains("set_editor_status_async")
                {
                    self.pass("N6 capability route", message);
                } else {
                    self.fail(
                        "N6 capability route",
                        format!("missing expected capabilities in '{message}'"),
                    );
                }
            }
            Ok(resp) => self.fail(
                "N6 capability route",
                resp.error
                    .unwrap_or_else(|| "bridge_capabilities returned ok=false".to_string()),
            ),
            Err(error) => self.fail("N6 capability route", error),
        }
    }

    async fn check_semantic_state_ready(&mut self) {
        let state = super::unity_semantic_state(&self.project).await;
        if matches!(
            state.phase.as_str(),
            "editing" | "playing" | "paused" | "starting"
        ) {
            self.pass(
                "N7 semantic state ready",
                format!(
                    "phase={} source={} channel={}",
                    state.phase, state.source, state.channel.control_pipe
                ),
            );
        } else {
            self.fail(
                "N7 semantic state ready",
                format!("phase={} source={}", state.phase, state.source),
            );
        }
    }

    async fn request_editor_status_raw(&self, desired_status: &str) -> Result<(), String> {
        let resp = super::send_message_with_timeout(
            &self.project,
            "set_editor_status",
            desired_status,
            EDITOR_STATUS_REQUEST_TIMEOUT,
        )
        .await?;
        if resp.ok {
            Ok(())
        } else {
            Err(resp
                .error
                .unwrap_or_else(|| "set_editor_status returned ok=false".to_string()))
        }
    }

    async fn wait_for_native_editor_status(
        &self,
        desired_status: &str,
        timeout: Duration,
    ) -> Result<NativeBrokerStatus, String> {
        let start = Instant::now();
        let mut last_detail = "no native shared-memory status samples".to_string();
        while start.elapsed() < timeout {
            if let Some(status) = super::query_native_broker_status(&self.project).await {
                let editor_status = editor_status_kind(&status.editor_status);
                last_detail = format!(
                    "managedState={} editorStatus={} generation={}",
                    status.managed_state, status.editor_status, status.domain_generation
                );
                if editor_status == desired_status {
                    return Ok(status);
                }
            }
            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        }
        Err(format!(
            "native shared-memory status did not report '{desired_status}' within {}s; last sample: {last_detail}",
            timeout.as_secs()
        ))
    }

    async fn check_native_shared_status_tracks_editor_mode(
        &mut self,
        baseline: &NativeBrokerStatus,
    ) {
        let current = editor_status_kind(&baseline.editor_status);
        let desired = if matches!(current.as_str(), "playing" | "playing_paused") {
            "editing"
        } else {
            "playing"
        };
        self.log(format!(
            "Requesting editor status transition {current} -> {desired}"
        ));

        match self.request_editor_status_raw(desired).await {
            Ok(()) => {}
            Err(error) => {
                self.fail("N8 native shared-memory status tracks editor mode", error);
                return;
            }
        }

        match self
            .wait_for_native_editor_status(desired, EDITOR_STATUS_WAIT_TIMEOUT)
            .await
        {
            Ok(status) => self.pass(
                "N8 native shared-memory status tracks editor mode",
                format!(
                    "editorStatus={} generation={}",
                    status.editor_status, status.domain_generation
                ),
            ),
            Err(error) => self.fail("N8 native shared-memory status tracks editor mode", error),
        }

        if desired != "editing" {
            self.restore_edit_mode_after_mode_probe().await;
        }
    }

    async fn restore_edit_mode_after_mode_probe(&mut self) {
        let start = Instant::now();
        loop {
            match self.request_editor_status_raw("editing").await {
                Ok(()) => break,
                Err(error)
                    if reload_request_was_accepted(&error)
                        && start.elapsed() < STATUS_WAIT_TIMEOUT =>
                {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                Err(error) => {
                    self.log(format!("restore edit mode request failed: {error}"));
                    return;
                }
            }
        }

        match self
            .wait_for_native_editor_status("editing", STATUS_WAIT_TIMEOUT)
            .await
        {
            Ok(status) => self.log(format!(
                "edit mode restored before continuing: generation={}",
                status.domain_generation
            )),
            Err(error) => self.log(format!("edit mode restore was not confirmed: {error}")),
        }
    }

    async fn check_editor_update_event_route(&mut self) {
        super::set_event_app_handle(self.app.clone());

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let listener = self.app.listen_any("unity-editor-update", move |event| {
            let _ = tx.send(event.payload().to_string());
        });

        let result = tokio::time::timeout(EDITOR_UPDATE_WAIT_TIMEOUT, rx.recv()).await;
        self.app.unlisten(listener);

        match result {
            Ok(Some(payload)) => self.pass(
                "N9 native editor-update event route",
                format!("received {} bytes", payload.len()),
            ),
            Ok(None) => self.fail(
                "N9 native editor-update event route",
                "unity-editor-update listener closed before receiving an event",
            ),
            Err(_) => self.fail(
                "N9 native editor-update event route",
                format!(
                    "no unity-editor-update event arrived within {}s over the native transport",
                    EDITOR_UPDATE_WAIT_TIMEOUT.as_secs()
                ),
            ),
        }
    }

    async fn check_reload_persistence(&mut self, baseline: &NativeBrokerStatus) {
        let baseline_generation = baseline.domain_generation;
        self.log(format!(
            "Requesting script reload from generation {baseline_generation}"
        ));
        if let Err(error) = self.request_script_reload().await {
            self.fail("N10 request domain reload", error);
            return;
        }
        self.pass("N10 request domain reload", "RequestScriptReload accepted");

        let start = Instant::now();
        let mut saw_reloading = false;
        let mut saw_semantic_reloading = false;
        let mut checked_reload_boundary_execute = false;
        let mut answered_samples = 0u32;
        let mut missed_samples = 0u32;
        let mut last_detail = "no samples".to_string();

        while start.elapsed() < STATUS_WAIT_TIMEOUT {
            match super::query_native_broker_status(&self.project).await {
                Some(status) => {
                    answered_samples += 1;
                    last_detail = format!(
                        "managedState={} generation={}",
                        status.managed_state, status.domain_generation
                    );
                    if status.managed_state == "reloading" {
                        saw_reloading = true;
                        let semantic = super::unity_semantic_state(&self.project).await;
                        if semantic.phase == "reloading" {
                            saw_semantic_reloading = true;
                            self.log(format!(
                                "semantic reload sample: source={} state_plane={}",
                                semantic.source, semantic.state_plane.native_broker
                            ));
                        }
                        if !checked_reload_boundary_execute {
                            checked_reload_boundary_execute = true;
                            match tokio::time::timeout(
                                Duration::from_secs(35),
                                super::unity_execute_code(
                                    &self.project,
                                    "return \"native_reload_retry_ok\";",
                                ),
                            )
                            .await
                            {
                                Ok(Ok(message)) if message.contains("native_reload_retry_ok") => {
                                    self.pass("N13 reload-boundary execute retry", message);
                                }
                                Ok(Ok(message)) => self.fail(
                                    "N13 reload-boundary execute retry",
                                    format!("unexpected execute result: {message}"),
                                ),
                                Ok(Err(error)) => {
                                    self.fail("N13 reload-boundary execute retry", error)
                                }
                                Err(_) => self.fail(
                                    "N13 reload-boundary execute retry",
                                    "unity_execute_code timed out while waiting across reload",
                                ),
                            }
                        }
                    }
                    if status.domain_generation > baseline_generation
                        && status.managed_state == "ready"
                    {
                        self.pass(
                            "N11 native pipe survives reload",
                            format!(
                                "generation {} -> {}, answered={} missed={}",
                                baseline_generation,
                                status.domain_generation,
                                answered_samples,
                                missed_samples
                            ),
                        );
                        if saw_reloading {
                            self.pass(
                                "N12 managed reload state observed",
                                "native shared-memory status reported managedState=reloading",
                            );
                            if saw_semantic_reloading {
                                self.pass(
                                    "N14 semantic reload state observed",
                                    "semantic state reported reloading",
                                );
                            } else {
                                self.fail(
                                    "N14 semantic reload state observed",
                                    "native shared-memory status saw reloading but semantic state did not report phase=reloading",
                                );
                            }
                        } else {
                            self.log(
                                "N10 managed reload state observed: reload window was too short to sample",
                            );
                        }
                        return;
                    }
                }
                None => {
                    missed_samples += 1;
                }
            }
            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        }

        self.fail(
            "N11 native pipe survives reload",
            format!(
                "timed out after {}s; last sample: {}",
                STATUS_WAIT_TIMEOUT.as_secs(),
                last_detail
            ),
        );
    }

    async fn run(&mut self) {
        if !self.check_feature_enabled() {
            return;
        }
        if !self.check_installed_plugin_files() {
            return;
        }
        if !self.check_marker() {
            return;
        }

        let Some(status) = self.ensure_broker_ready().await else {
            return;
        };
        self.check_capability_route().await;
        self.check_semantic_state_ready().await;
        self.check_native_shared_status_tracks_editor_mode(&status)
            .await;
        self.check_editor_update_event_route().await;
        let reload_baseline = self
            .wait_for_native_shared_status(Duration::from_secs(5))
            .await
            .unwrap_or_else(|| status.clone());
        self.check_reload_persistence(&reload_baseline).await;
    }
}

fn editor_status_kind(status: &str) -> String {
    status.split('|').next().unwrap_or("").trim().to_string()
}

fn reload_request_was_accepted(error: &str) -> bool {
    matches!(error, "managed_reloading" | "domain_reload_interrupted")
        || error.contains("managed_reloading")
        || error.contains("domain_reload_interrupted")
}

pub async fn run(app: tauri::AppHandle, project: String) -> Result<(), String> {
    if project.trim().is_empty() {
        return Err("No workspace selected".to_string());
    }
    if !super::is_unity_project(&project) {
        return Err("Current workspace is not a Unity project".to_string());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A native bridge self-test is already running".to_string());
    }
    let _guard = RunningGuard;

    let mut test = SelfTest {
        app,
        project,
        passed: 0,
        failed: 0,
    };
    test.log("Unity native bridge self-test starting");
    test.run().await;
    test.log(format!(
        "Finished: {} passed, {} failed",
        test.passed, test.failed
    ));
    test.emit(None, true);
    Ok(())
}
