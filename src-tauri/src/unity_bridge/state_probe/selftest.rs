//! Built-in state-probe self-test: connects to the running Unity Editor,
//! drives real state changes (a forced domain reload, play / pause / resume /
//! exit-play transitions), and verifies the out-of-process native probe + the
//! fused semantic state actually observe them — including through the
//! pipe-dead reload window and play-mode pause windows where
//! `EditorApplication.update` may stop pumping queued main-thread work.
//!
//! Mirrors the hot-reload self-test: progress streams to the UI via the
//! `unity-state-probe-selftest` event, triggered from Settings > Code Analysis.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use super::{sample_blocking, semantic_state_for_project, status};

static RUNNING: AtomicBool = AtomicBool::new(false);
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECTION_STATUS_BUDGET: Duration = Duration::from_millis(2_500);
const PLAY_TRANSITION_TIMEOUT: Duration = Duration::from_secs(90);
const PAUSE_TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);
const BRIDGE_CAPABILITY_TIMEOUT: Duration = Duration::from_millis(800);
const DIAGNOSTIC_PIPE_TIMEOUT: Duration = Duration::from_millis(800);
const DIAGNOSTIC_PROCESS_TIMEOUT: Duration = Duration::from_millis(1_000);
const UPDATE_PUMP_REQUEST_BUDGET: Duration = Duration::from_millis(1_200);
const UPDATE_PUMP_STATUS_BUDGET: Duration = Duration::from_millis(2_500);
const SEMANTIC_WAIT_INTERVAL: Duration = Duration::from_millis(300);
const BRIDGE_CAPABILITY_RELOAD_TIMEOUT: Duration = Duration::from_secs(45);
const SEMANTIC_ACTIONABLE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTestEvent {
    running: bool,
    finished: bool,
    line: Option<String>,
    passed: u32,
    failed: u32,
}

struct SelfTest {
    app: tauri::AppHandle,
    project: String,
    passed: u32,
    failed: u32,
}

enum BridgeCapabilityProbe {
    Ready(String),
    Stale(String),
    Failed(String),
}

impl SelfTest {
    fn emit(&self, line: Option<String>, finished: bool) {
        let _ = self.app.emit(
            "unity-state-probe-selftest",
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
            tracing::error!(log_module = "StateProbe SelfTest", "{line}");
        } else {
            tracing::info!(log_module = "StateProbe SelfTest", "{line}");
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

    async fn execute(&self, code: &str) -> Result<String, String> {
        match tokio::time::timeout(
            EXECUTE_TIMEOUT,
            crate::unity_bridge::unity_execute_code(&self.project, code),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(format!(
                "unity_execute_code timed out after {}s",
                EXECUTE_TIMEOUT.as_secs()
            )),
        }
    }

    fn check_installed_plugin_files(&mut self) -> bool {
        match crate::unity_bridge::check_plugin_status(&self.project) {
            Ok(crate::unity_bridge::PluginStatus::UpToDate) => {
                self.pass(
                    "precondition Unity plugin files",
                    "installed plugin matches this Locus build",
                );
                true
            }
            Ok(status) => {
                self.fail(
                    "precondition Unity plugin files",
                    format!(
                        "installed plugin is {:?}; update the Locus Unity plugin and wait for Unity to compile it before rerunning",
                        status
                    ),
                );
                false
            }
            Err(error) => {
                self.fail(
                    "precondition Unity plugin files",
                    format!("could not verify installed plugin files: {error}"),
                );
                false
            }
        }
    }

    async fn probe_bridge_capabilities_once(&self) -> BridgeCapabilityProbe {
        let started = Instant::now();
        match crate::unity_bridge::send_message_with_timeout(
            &self.project,
            "bridge_capabilities",
            "",
            BRIDGE_CAPABILITY_TIMEOUT,
        )
        .await
        {
            Ok(resp) if resp.ok => {
                let message = resp.message.unwrap_or_default();
                let has_status = message.contains("status_cached");
                let has_set_status = message.contains("set_editor_status_async");
                if has_status && has_set_status {
                    BridgeCapabilityProbe::Ready(format!(
                        "{} elapsed={}ms",
                        message,
                        started.elapsed().as_millis()
                    ))
                } else {
                    BridgeCapabilityProbe::Stale(format!(
                        "missing required capabilities in '{}' after {}ms",
                        message,
                        started.elapsed().as_millis()
                    ))
                }
            }
            Ok(resp) => {
                let error = resp.error.unwrap_or_else(|| "unknown error".to_string());
                let detail = format!(
                    "bridge rejected capability probe after {}ms: {}",
                    started.elapsed().as_millis(),
                    error
                );
                if error.contains("unknown message type: bridge_capabilities") {
                    BridgeCapabilityProbe::Stale(detail)
                } else {
                    BridgeCapabilityProbe::Failed(detail)
                }
            }
            Err(error) => BridgeCapabilityProbe::Failed(format!(
                "capability probe failed after {}ms: {}",
                started.elapsed().as_millis(),
                error
            )),
        }
    }

    async fn request_bridge_runtime_reload(&self) -> Result<(), String> {
        self.execute("UnityEditor.EditorUtility.RequestScriptReload(); return \"requested\";")
            .await
            .map(|_| ())
    }

    async fn wait_for_bridge_capabilities_after_reload(&self) -> Result<String, String> {
        let start = Instant::now();
        let mut last_detail: Option<String> = None;
        loop {
            if start.elapsed() > BRIDGE_CAPABILITY_RELOAD_TIMEOUT {
                let detail =
                    last_detail.unwrap_or_else(|| "no capability probe sampled".to_string());
                return Err(format!(
                    "runtime capabilities did not appear within {}s after script reload; last probe: {}",
                    BRIDGE_CAPABILITY_RELOAD_TIMEOUT.as_secs(),
                    detail
                ));
            }

            match self.probe_bridge_capabilities_once().await {
                BridgeCapabilityProbe::Ready(detail) => return Ok(detail),
                BridgeCapabilityProbe::Stale(detail) | BridgeCapabilityProbe::Failed(detail) => {
                    last_detail = Some(detail);
                }
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn check_bridge_capabilities(&mut self) -> bool {
        match self.probe_bridge_capabilities_once().await {
            BridgeCapabilityProbe::Ready(detail) => {
                self.pass("precondition bridge runtime capabilities", detail);
                true
            }
            BridgeCapabilityProbe::Stale(detail) => {
                self.log(format!(
                    "precondition bridge runtime capabilities: runtime is stale ({detail}); requesting script reload"
                ));
                match self.request_bridge_runtime_reload().await {
                    Ok(()) => {
                        self.log(
                            "Requested RequestScriptReload(); waiting for the updated bridge runtime…",
                        );
                    }
                    Err(error) => {
                        self.fail(
                            "precondition bridge runtime capabilities",
                            format!(
                                "could not request script reload for stale bridge runtime: {error}"
                            ),
                        );
                        return false;
                    }
                }

                match self.wait_for_bridge_capabilities_after_reload().await {
                    Ok(detail) => {
                        self.pass(
                            "precondition bridge runtime capabilities",
                            format!("updated runtime loaded after script reload; {detail}"),
                        );
                        true
                    }
                    Err(error) => {
                        self.fail("precondition bridge runtime capabilities", error);
                        false
                    }
                }
            }
            BridgeCapabilityProbe::Failed(detail) => {
                self.fail(
                    "precondition bridge runtime capabilities",
                    format!("{detail}; update/recompile the Locus Unity plugin"),
                );
                false
            }
        }
    }

    /// Resolve the editor PID + engine-module hint once, for direct native
    /// sampling (which needs no pipe and so stays responsive during reload).
    async fn editor_process(&self) -> Result<(u32, String), String> {
        let info = crate::unity_bridge::query_current_project_editor_process(&self.project).await;
        let pid = info
            .process_id
            .ok_or_else(|| "Unity editor process not found".to_string())?;
        Ok((pid, info.executable_path.unwrap_or_default()))
    }

    async fn native_sample(
        &self,
        pid: u32,
        hint: String,
        allow_suspend: bool,
    ) -> Result<Option<super::NativeSample>, String> {
        tauri::async_runtime::spawn_blocking(move || sample_blocking(pid, &hint, allow_suspend))
            .await
            .map_err(|e| format!("native sample task failed: {e}"))?
    }

    async fn wait_for_edit_mode(&self, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            let (connected, status, _) =
                crate::unity_bridge::query_unity_status(&self.project).await;
            if connected && status == crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err("editor did not return to edit mode in time".to_string());
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    async fn wait_for_play_state(&self, playing: bool, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        let mut last_detail: Option<String> = None;
        loop {
            if start.elapsed() > timeout {
                let detail = last_detail.unwrap_or_else(|| "no status sampled".to_string());
                return Err(format!(
                    "editor did not reach {} in time; {detail}",
                    if playing { "play mode" } else { "edit mode" }
                ));
            }

            let (connected, status, _) =
                crate::unity_bridge::query_unity_status(&self.project).await;
            if connected && crate::unity_bridge::is_play_mode_status(status) == playing {
                return Ok(());
            }
            last_detail = Some(format!("last status={status}, connected={connected}"));
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    }

    async fn wait_for_semantic_phase(
        &self,
        expected: &[&str],
        timeout: Duration,
    ) -> Result<super::SemanticState, String> {
        let start = Instant::now();
        let mut last_detail: Option<String> = None;
        loop {
            if start.elapsed() > timeout {
                let detail = last_detail.unwrap_or_else(|| "no semantic state sampled".to_string());
                return Err(format!(
                    "semantic state did not reach one of [{}] in time; {detail}",
                    expected.join(", ")
                ));
            }

            let state = semantic_state_for_project(&self.project).await;
            if expected.iter().any(|phase| state.phase == *phase) {
                return Ok(state);
            }
            last_detail = Some(format!(
                "last phase={} source={} confidence={}",
                state.phase, state.source, state.confidence
            ));
            tokio::time::sleep(SEMANTIC_WAIT_INTERVAL).await;
        }
    }

    async fn wait_for_semantic_actionable(
        &self,
        timeout: Duration,
    ) -> Result<super::SemanticState, String> {
        let start = Instant::now();
        let mut last_detail: Option<String> = None;
        loop {
            if start.elapsed() > timeout {
                let detail = last_detail.unwrap_or_else(|| "no semantic state sampled".to_string());
                return Err(format!(
                    "semantic state did not become actionable within {}s; {detail}",
                    timeout.as_secs()
                ));
            }

            let state = semantic_state_for_project(&self.project).await;
            if !state.transient
                && state.safety.recommended_action == "proceed"
                && state.channel.control_pipe == "ready"
            {
                return Ok(state);
            }
            last_detail = Some(Self::format_semantic_state(&state));
            tokio::time::sleep(SEMANTIC_WAIT_INTERVAL).await;
        }
    }

    async fn set_editor_status_step(
        &mut self,
        name: &str,
        desired_status: &'static str,
        timeout: Duration,
        pid: u32,
        hint: &str,
    ) -> bool {
        self.log(format!(
            "STEP  {name}: set_editor_status({desired_status}), timeout={}s",
            timeout.as_secs()
        ));
        self.log_diagnostic_snapshot(&format!("{name} before request"), pid, hint, false)
            .await;

        match self
            .wait_for_semantic_actionable(SEMANTIC_ACTIONABLE_TIMEOUT)
            .await
        {
            Ok(state) => self.log(format!(
                "STEP  {name}: semantic ready before request: {}",
                Self::format_semantic_state(&state)
            )),
            Err(error) => {
                self.fail(name, error);
                self.log_diagnostic_snapshot(
                    &format!("{name} before request blocked"),
                    pid,
                    hint,
                    true,
                )
                .await;
                return false;
            }
        }

        let mut attempt = 1;
        loop {
            let start = Instant::now();
            let result = tokio::time::timeout(
                timeout,
                crate::unity_bridge::set_editor_status(&self.project, desired_status),
            )
            .await;
            let elapsed = start.elapsed();

            match result {
                Ok(Ok(())) => {
                    self.pass(
                        name,
                        format!(
                            "set_editor_status({desired_status}) completed in {}ms",
                            elapsed.as_millis()
                        ),
                    );
                    self.log_diagnostic_snapshot(
                        &format!("{name} after success"),
                        pid,
                        hint,
                        false,
                    )
                    .await;
                    return true;
                }
                Ok(Err(error)) if attempt == 1 && Self::is_reload_boundary_error(&error) => {
                    self.log(format!(
                        "STEP  {name}: set_editor_status({desired_status}) hit reload boundary after {}ms; waiting for semantic recovery",
                        elapsed.as_millis()
                    ));
                    match self
                        .wait_for_semantic_actionable(SEMANTIC_ACTIONABLE_TIMEOUT)
                        .await
                    {
                        Ok(state) => self.log(format!(
                            "STEP  {name}: semantic recovered before retry: {}",
                            Self::format_semantic_state(&state)
                        )),
                        Err(wait_error) => {
                            self.fail(name, wait_error);
                            self.log_diagnostic_snapshot(
                                &format!("{name} after reload-boundary wait failure"),
                                pid,
                                hint,
                                true,
                            )
                            .await;
                            return false;
                        }
                    }
                    attempt += 1;
                }
                Ok(Err(error)) => {
                    self.fail(
                        name,
                        format!(
                            "set_editor_status({desired_status}) failed after {}ms: {error}",
                            elapsed.as_millis()
                        ),
                    );
                    self.log_diagnostic_snapshot(&format!("{name} after failure"), pid, hint, true)
                        .await;
                    return false;
                }
                Err(_) => {
                    self.fail(
                        name,
                        format!(
                            "set_editor_status({desired_status}) timed out after {}s; Unity main-thread action or status confirmation did not complete",
                            timeout.as_secs()
                        ),
                    );
                    self.log_diagnostic_snapshot(&format!("{name} after timeout"), pid, hint, true)
                        .await;
                    return false;
                }
            }
        }
    }

    async fn probe_connection_status(
        &mut self,
        name: &str,
        expected_statuses: &[&str],
        require_connected: bool,
    ) {
        let start = Instant::now();
        let status = crate::unity_bridge::query_unity_connection_status(&self.project).await;
        let elapsed = start.elapsed();
        self.record_connection_status_result(
            name,
            expected_statuses,
            require_connected,
            status,
            elapsed,
        );
    }

    async fn probe_paused_update_pump_resilience(&mut self, pid: u32, hint: &str) {
        self.log(
            "Update-pump pause probe: sending one queued main-thread request with a 1200ms budget",
        );

        let started = Instant::now();
        let queued_result = crate::unity_bridge::send_message_with_timeout(
            &self.project,
            "get_console_text",
            "",
            UPDATE_PUMP_REQUEST_BUDGET,
        )
        .await;
        let elapsed = started.elapsed();

        match queued_result {
            Ok(resp) if resp.ok => self.pass(
                "UP1 paused main-thread queue probe",
                format!(
                    "get_console_text completed in {}ms; this Editor still pumps update while paused",
                    elapsed.as_millis()
                ),
            ),
            Ok(resp) => self.fail(
                "UP1 paused main-thread queue probe",
                format!(
                    "get_console_text returned ok=false in {}ms: {}",
                    elapsed.as_millis(),
                    resp.error.as_deref().unwrap_or("unknown error")
                ),
            ),
            Err(error) if elapsed <= UPDATE_PUMP_REQUEST_BUDGET + Duration::from_millis(350) => {
                self.pass(
                    "UP1 paused main-thread queue probe",
                    format!(
                        "get_console_text released the caller in {}ms with error={}; update pump is likely paused",
                        elapsed.as_millis(),
                        error
                    ),
                );
            }
            Err(error) => self.fail(
                "UP1 paused main-thread queue probe",
                format!(
                    "get_console_text exceeded the {}ms budget: elapsed={}ms error={}",
                    UPDATE_PUMP_REQUEST_BUDGET.as_millis(),
                    elapsed.as_millis(),
                    error
                ),
            ),
        }

        let started = Instant::now();
        let (connected, status, _) = crate::unity_bridge::query_unity_status_with_timeout(
            &self.project,
            UPDATE_PUMP_STATUS_BUDGET,
        )
        .await;
        let elapsed = started.elapsed();
        if connected && status == crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED {
            self.pass(
                "UP2 paused direct status optional",
                format!("status={} elapsed={}ms", status, elapsed.as_millis()),
            );
        } else {
            self.pass(
                "UP2 paused direct status optional",
                format!(
                    "direct status unavailable within {}ms; observer state remains authoritative (connected={} status={} elapsed={}ms)",
                    UPDATE_PUMP_STATUS_BUDGET.as_millis(),
                    connected,
                    status,
                    elapsed.as_millis()
                ),
            );
            self.log_diagnostic_snapshot("UP2 paused status unavailable", pid, hint, true)
                .await;
        }

        self.probe_connection_status(
            "UP3 connection status after paused queue probe",
            &[crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED],
            false,
        )
        .await;

        match semantic_state_for_project(&self.project).await {
            state if state.editor_mode.value == "paused" && state.process.state == "running" => {
                self.pass(
                    "UP3b observer state while paused",
                    format!(
                        "editor_mode={} source={} channel={} process={}",
                        state.editor_mode.value,
                        state.editor_mode.source,
                        state.channel.control_pipe,
                        state.process.state
                    ),
                );
            }
            state => self.fail(
                "UP3b observer state while paused",
                format!(
                    "expected running paused observer state; got {}",
                    Self::format_semantic_state(&state)
                ),
            ),
        }

        let started = Instant::now();
        match self.native_sample(pid, hint.to_string(), false).await {
            Ok(Some(sample)) => self.pass(
                "UP4 native passive sample while paused",
                format!(
                    "{} elapsed={}ms",
                    Self::format_native_sample(&sample),
                    started.elapsed().as_millis()
                ),
            ),
            Ok(None) => self.log("UP4 note: native passive sample unavailable while paused"),
            Err(error) => self.fail("UP4 native passive sample while paused", error),
        }
    }

    async fn restore_edit_mode(&mut self) {
        let (connected, status, _) = crate::unity_bridge::query_unity_status(&self.project).await;
        if !connected || !crate::unity_bridge::is_play_mode_status(status) {
            return;
        }

        self.log("Cleanup: editor is still in play mode; requesting exit.");
        match tokio::time::timeout(
            Duration::from_secs(30),
            crate::unity_bridge::set_editor_status(
                &self.project,
                crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING,
            ),
        )
        .await
        {
            Ok(Ok(())) => match self
                .wait_for_play_state(false, Duration::from_secs(30))
                .await
            {
                Ok(()) => self.pass("cleanup exit play mode", "edit mode restored"),
                Err(error) => self.fail("cleanup exit play mode", error),
            },
            Ok(Err(error)) => self.fail(
                "cleanup exit play mode",
                format!("set_editor_status(editing) failed: {error}"),
            ),
            Err(error) => self.fail(
                "cleanup exit play mode",
                format!("set_editor_status(editing) timed out: {error}"),
            ),
        }
    }

    async fn log_diagnostic_snapshot(
        &self,
        label: &str,
        pid: u32,
        hint: &str,
        include_stack: bool,
    ) {
        self.log(format!("DIAG  {label}: snapshot begin"));

        let started = Instant::now();
        let (pipe_connected, pipe_status, scene_path) =
            crate::unity_bridge::query_unity_status_with_timeout(
                &self.project,
                DIAGNOSTIC_PIPE_TIMEOUT,
            )
            .await;
        self.log(format!(
            "DIAG  {label}: pipe connected={} status={} scene={} elapsed={}ms",
            pipe_connected,
            pipe_status,
            scene_path.as_deref().unwrap_or("none"),
            started.elapsed().as_millis()
        ));

        let started = Instant::now();
        let connection = crate::unity_bridge::query_unity_connection_status(&self.project).await;
        self.log(format!(
            "DIAG  {label}: connection {} elapsed={}ms",
            Self::format_connection_status(&connection),
            started.elapsed().as_millis()
        ));

        let started = Instant::now();
        let semantic = semantic_state_for_project(&self.project).await;
        self.log(format!(
            "DIAG  {label}: semantic {} elapsed={}ms",
            Self::format_semantic_state(&semantic),
            started.elapsed().as_millis()
        ));

        let started = Instant::now();
        let process_result = tokio::time::timeout(
            DIAGNOSTIC_PROCESS_TIMEOUT,
            crate::unity_bridge::query_current_project_editor_process(&self.project),
        )
        .await;
        match process_result {
            Ok(process) => self.log(format!(
                "DIAG  {label}: process state={:?} pid={:?} path={} project={} checked_at={} error={} elapsed={}ms",
                process.state,
                process.process_id,
                process.executable_path.as_deref().unwrap_or("none"),
                process.project_path.as_deref().unwrap_or("none"),
                process.checked_at_ms,
                process.last_error.as_deref().unwrap_or("none"),
                started.elapsed().as_millis()
            )),
            Err(_) => self.log(format!(
                "DIAG  {label}: process timed out after {}ms",
                DIAGNOSTIC_PROCESS_TIMEOUT.as_millis()
            )),
        }

        let started = Instant::now();
        match self.native_sample(pid, hint.to_string(), false).await {
            Ok(Some(sample)) => self.log(format!(
                "DIAG  {label}: native passive {} elapsed={}ms",
                Self::format_native_sample(&sample),
                started.elapsed().as_millis()
            )),
            Ok(None) => self.log(format!("DIAG  {label}: native passive unavailable")),
            Err(error) => self.log(format!("DIAG  {label}: native passive error={error}")),
        }

        if include_stack {
            let started = Instant::now();
            match self.native_sample(pid, hint.to_string(), true).await {
                Ok(Some(sample)) => self.log(format!(
                    "DIAG  {label}: native stack {} elapsed={}ms",
                    Self::format_native_sample(&sample),
                    started.elapsed().as_millis()
                )),
                Ok(None) => self.log(format!("DIAG  {label}: native stack unavailable")),
                Err(error) => self.log(format!("DIAG  {label}: native stack error={error}")),
            }
        }

        let probe = status();
        self.log(format!(
            "DIAG  {label}: probe enabled={} supported={} tier={:?} pid={:?} last_phase={} error={}",
            probe.enabled,
            probe.supported,
            probe.tier,
            probe.process_id,
            probe.last_phase.as_deref().unwrap_or("none"),
            probe.error.as_deref().unwrap_or("none")
        ));
    }

    fn format_semantic_state(state: &super::SemanticState) -> String {
        format!(
            "phase={} source={} confidence={} transient={} needs_user={} reload_phase={} editor_mode={}/{} channel={} process={} main_thread={} safety={} state_plane={}/{}/{} history={} detail={}",
            state.phase,
            state.source,
            state.confidence,
            state.transient,
            state.needs_user,
            state.reload_phase.as_deref().unwrap_or("none"),
            state.editor_mode.value,
            state.editor_mode.source,
            state.channel.control_pipe,
            state.process.state,
            state.main_thread.state,
            state.safety.recommended_action,
            state.state_plane.observer,
            state.state_plane.native_broker,
            state.state_plane.native_hook,
            state.state_plane.history_samples,
            state.detail.as_deref().unwrap_or("none")
        )
    }

    fn is_reload_boundary_error(error: &str) -> bool {
        error.contains("domain_reload_interrupted")
            || error.contains("managed_reloading")
            || error.contains("managed executor unavailable")
    }

    fn format_native_sample(sample: &super::NativeSample) -> String {
        format!(
            "reload={:?} rip_in_unity={} cpu_active={} quiescent_for_ms={} suspended={} suspend_window_us={}",
            sample.reloading,
            sample.rip_in_unity,
            sample.cpu_active,
            sample.quiescent_for_ms,
            sample.suspended,
            sample.suspend_window_us
        )
    }

    fn format_connection_status(status: &crate::unity_bridge::UnityConnectionStatus) -> String {
        format!(
            "connected={} control_channel={} editor_status={} scene={} process={:?} pid={:?} process_path={} process_project={} latency_ms={:?} reconnect_attempts={} last_error={} process_error={} checked_at={}",
            status.connected,
            status.control_channel_state,
            status.editor_status,
            status.scene_path.as_deref().unwrap_or("none"),
            status.editor_process_state,
            status.editor_process_id,
            status.editor_process_path.as_deref().unwrap_or("none"),
            status.editor_project_path.as_deref().unwrap_or("none"),
            status.latency_ms,
            status.reconnect_attempts,
            status.last_error.as_deref().unwrap_or("none"),
            status.process_last_error.as_deref().unwrap_or("none"),
            status.checked_at_ms
        )
    }

    fn record_connection_status_result(
        &mut self,
        name: &str,
        expected_statuses: &[&str],
        require_connected: bool,
        status: crate::unity_bridge::UnityConnectionStatus,
        elapsed: Duration,
    ) {
        let status_matches = expected_statuses
            .iter()
            .any(|expected| status.editor_status == *expected);
        if elapsed <= CONNECTION_STATUS_BUDGET
            && status_matches
            && (!require_connected || status.connected)
        {
            self.pass(
                name,
                format!(
                    "connected={} control_channel={} editor_status={} process={:?} elapsed={}ms",
                    status.connected,
                    status.control_channel_state,
                    status.editor_status,
                    status.editor_process_state,
                    elapsed.as_millis()
                ),
            );
            return;
        }

        self.fail(
            name,
            format!(
                "expected connected={} status in [{}] within {}ms; got connected={} control_channel={} status={} process={:?} elapsed={}ms error={}",
                require_connected,
                expected_statuses.join(", "),
                CONNECTION_STATUS_BUDGET.as_millis(),
                status.connected,
                status.control_channel_state,
                status.editor_status,
                status.editor_process_state,
                elapsed.as_millis(),
                status.last_error.as_deref().unwrap_or("none")
            ),
        );
    }

    // ── phases ───────────────────────────────────────────────────────

    async fn run_baseline(&mut self, pid: u32, hint: &str) {
        self.log("Phase 1/4 — baseline (edit mode)");

        // Passive tier: no suspension at all — this is the default used in
        // normal operation.
        match self.native_sample(pid, hint.to_string(), false).await {
            Ok(Some(sample)) => {
                if !sample.suspended {
                    self.pass(
                        "B1 passive sample (no suspension)",
                        format!(
                            "pid {pid}: cpu_active={}, never suspended the main thread",
                            sample.cpu_active
                        ),
                    );
                } else {
                    self.fail(
                        "B1 passive sample (no suspension)",
                        "passive tier suspended the thread",
                    );
                }
            }
            Ok(None) => self.log("B1 note: probe disabled or unavailable"),
            Err(error) => self.fail("B1 passive sample (no suspension)", error),
        }

        // Stack tier: opt-in suspend for one bulk read. In idle edit mode it
        // must report NOT reloading.
        let tier = format!("{:?}", status().tier);
        match self.native_sample(pid, hint.to_string(), true).await {
            Ok(Some(sample)) if sample.suspended => {
                if sample.reloading.is_none() {
                    self.pass(
                        "B2 stack sample (edit mode)",
                        format!(
                            "tier {tier}: not reloading, suspend window {}µs",
                            sample.suspend_window_us
                        ),
                    );
                } else {
                    self.fail(
                        "B2 stack sample (edit mode)",
                        "reported a reload while idle",
                    );
                }
            }
            Ok(_) => self.log(format!(
                "B2 note: stack tier inactive (tier {tier}; no PDB/symbols) — reload detection \
                 will fall back to passive inference"
            )),
            Err(error) => self.fail("B2 stack sample (edit mode)", error),
        }

        let state = semantic_state_for_project(&self.project).await;
        if state.phase == "editing" {
            self.pass(
                "B3 fused baseline state",
                format!("phase=editing source={}", state.source),
            );
        } else {
            self.fail(
                "B3 fused baseline state",
                format!(
                    "expected phase=editing, got phase={} source={}",
                    state.phase, state.source
                ),
            );
        }

        self.probe_connection_status(
            "B4 connection status in edit mode",
            &[crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING],
            true,
        )
        .await;
    }

    /// Performance + risk characterization, run in edit mode so timings are
    /// not skewed by a reload. Reports passive vs. stack cost and the real
    /// main-thread freeze window, and states the risk posture explicitly.
    async fn run_perf_risk(&mut self, pid: u32, hint: &str) {
        self.log("Phase 2/4 — performance & risk analysis (edit mode)");
        const ITERS: u32 = 20;

        // Passive tier latency (no suspension).
        let mut passive_total_us = 0u64;
        let mut passive_ok = 0u32;
        for _ in 0..ITERS {
            let start = Instant::now();
            if let Ok(Some(_)) = self.native_sample(pid, hint.to_string(), false).await {
                passive_total_us += start.elapsed().as_micros() as u64;
                passive_ok += 1;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Stack tier latency + measured suspend window.
        let mut stack_total_us = 0u64;
        let mut window_total_us = 0u64;
        let mut window_max_us = 0u64;
        let mut stack_ok = 0u32;
        for _ in 0..ITERS {
            let start = Instant::now();
            if let Ok(Some(sample)) = self.native_sample(pid, hint.to_string(), true).await {
                if sample.suspended {
                    stack_total_us += start.elapsed().as_micros() as u64;
                    window_total_us += sample.suspend_window_us;
                    window_max_us = window_max_us.max(sample.suspend_window_us);
                    stack_ok += 1;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        if passive_ok > 0 {
            self.pass(
                "PR1 passive tier cost",
                format!(
                    "avg {}µs/sample over {passive_ok} samples — main thread NEVER suspended",
                    passive_total_us / passive_ok as u64
                ),
            );
        } else {
            self.log("PR1 note: passive samples unavailable");
        }

        if stack_ok > 0 {
            let avg_window = window_total_us / stack_ok as u64;
            self.pass(
                "PR2 stack tier freeze window",
                format!(
                    "main thread suspended ~{avg_window}µs avg / {window_max_us}µs max (total \
                     sample {}µs avg) over {stack_ok} samples",
                    stack_total_us / stack_ok as u64
                ),
            );
            if window_max_us > 5_000 {
                self.fail(
                    "PR3 freeze window bound",
                    format!("worst suspend window {window_max_us}µs exceeds the 5ms budget"),
                );
            } else {
                self.pass(
                    "PR3 freeze window bound",
                    format!("worst freeze {window_max_us}µs is within the 5ms budget"),
                );
            }
        } else {
            self.log(
                "PR2 note: stack tier inactive (no symbols) — only the passive, no-suspend tier \
                 runs; nothing suspends the editor",
            );
        }

        // Risk posture (stated, not measured).
        self.log("PR risk posture:");
        self.log(
            "  • Default operation is PASSIVE (CPU/liveness via GetThreadTimes) — no suspension.",
        );
        self.log(
            "  • Stack tier only READS memory (ReadProcessMemory); it never runs editor code or \
             takes an editor lock, so it cannot deadlock Unity.",
        );
        self.log(
            "  • Each suspend is RAII-guarded — a probe panic/early-return still resumes the \
             thread. Residual risk: a hard-kill of Locus inside the µs window leaves the thread \
             suspended (bounded by the window above).",
        );
    }

    /// The headline test: force a domain reload and confirm the native probe
    /// sees `reloading` while the managed executor is re-registering.
    async fn run_reload(&mut self, pid: u32, hint: &str) {
        self.log("Phase 3/4 — forced domain reload (pipe-dead window)");

        if let Err(error) = self
            .execute("UnityEditor.EditorUtility.RequestScriptReload(); return \"requested\";")
            .await
        {
            self.fail(
                "R1 trigger domain reload",
                format!("RequestScriptReload failed: {error}"),
            );
            return;
        }
        self.log("Requested RequestScriptReload(); sampling the native probe…");

        let mut observed_phase: Option<super::ReloadPhase> = None;
        let mut saw_pipe_drop = false;
        let mut checked_status_during_pipe_drop = false;
        let mut pipe_drop_status_probe: Option<
            tokio::task::JoinHandle<(crate::unity_bridge::UnityConnectionStatus, Duration)>,
        > = None;
        let start = Instant::now();
        // Poll the NATIVE tier directly (no pipe) so we stay responsive while
        // the managed channel is torn down.
        while start.elapsed() < Duration::from_secs(30) {
            // Stack tier (suspend) here: catching the reload sub-phase is the
            // whole point, and during a reload the main thread is busy in
            // native code where a sub-ms suspend is harmless.
            if let Ok(Some(sample)) = self.native_sample(pid, hint.to_string(), true).await {
                if let Some(phase) = sample.reloading {
                    observed_phase = Some(phase);
                    self.log(format!("  observed reload sub-phase: {}", phase.as_str()));
                    break;
                }
            }
            // A cheap pipe check records that the channel really went down. Use
            // a SHORT timeout so a half-open pipe during the reload can't stall
            // the 60ms sampling cadence on the default 35s.
            let (connected, _, _) = crate::unity_bridge::query_unity_status_with_timeout(
                &self.project,
                Duration::from_millis(300),
            )
            .await;
            if !connected {
                saw_pipe_drop = true;
                if !checked_status_during_pipe_drop {
                    let project = self.project.clone();
                    pipe_drop_status_probe = Some(tokio::spawn(async move {
                        let started = Instant::now();
                        let status =
                            crate::unity_bridge::query_unity_connection_status(&project).await;
                        (status, started.elapsed())
                    }));
                    checked_status_during_pipe_drop = true;
                }
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }

        if let Some(probe) = pipe_drop_status_probe {
            match probe.await {
                Ok((status, elapsed)) => self.record_connection_status_result(
                    "R1 connection status during pipe-dead reload",
                    &[
                        crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING,
                        crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED,
                    ],
                    false,
                    status,
                    elapsed,
                ),
                Err(error) => self.fail(
                    "R1 connection status during pipe-dead reload",
                    format!("background status probe failed: {error}"),
                ),
            }
        } else {
            self.log("R1 note: pipe drop was not observed during this reload sample window");
        }

        match observed_phase {
            Some(phase) => self.pass(
                "R2 native probe observed the reload",
                format!(
                    "caught domain reload natively (sub-phase {}) while the pipe was down",
                    phase.as_str()
                ),
            ),
            None => {
                let tier = format!("{:?}", status().tier);
                if tier == "Stack" {
                    self.fail(
                        "R2 native probe observed the reload",
                        "native tier is Stack but no reload frame was caught in 30s (reload may \
                         have been too brief between 60ms samples, or symbols mismatched)",
                    );
                } else {
                    self.log(format!(
                        "R2 note: native stack tier inactive (tier {tier}); reload not classifiable \
                         natively. Pipe drop seen: {saw_pipe_drop}"
                    ));
                }
            }
        }

        match self.wait_for_edit_mode(Duration::from_secs(90)).await {
            Ok(()) => self.pass("R3 editor recovered", "edit mode restored after the reload"),
            Err(error) => self.fail("R3 editor recovered", error),
        }

        self.probe_connection_status(
            "R4 connection status after reload",
            &[crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING],
            true,
        )
        .await;
    }

    async fn run_play(&mut self, pid: u32, hint: &str) {
        self.log("Phase 4/4 — play-mode / pause / resume matrix");

        if !self
            .set_editor_status_step(
                "P1 enter play mode",
                crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING,
                PLAY_TRANSITION_TIMEOUT,
                pid,
                hint,
            )
            .await
        {
            return;
        }

        tokio::time::sleep(Duration::from_millis(800)).await;

        match self
            .wait_for_semantic_phase(&["playing", "paused"], Duration::from_secs(15))
            .await
        {
            Ok(state) => self.pass(
                "P2 fused play state",
                format!("phase={} source={}", state.phase, state.source),
            ),
            Err(error) => self.fail("P2 fused play state", error),
        }

        self.probe_connection_status(
            "P3 connection status in play mode",
            &[
                crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING,
                crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED,
            ],
            true,
        )
        .await;

        let pause_requested = self
            .set_editor_status_step(
                "P4 pause play mode",
                crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED,
                PAUSE_TRANSITION_TIMEOUT,
                pid,
                hint,
            )
            .await;

        match self
            .wait_for_semantic_phase(&["paused"], Duration::from_secs(15))
            .await
        {
            Ok(state) => self.pass(
                "P5 fused paused state",
                format!("phase={} source={}", state.phase, state.source),
            ),
            Err(error) => self.fail("P5 fused paused state", error),
        }

        if pause_requested {
            self.probe_connection_status(
                "P6 connection status while paused",
                &[crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED],
                true,
            )
            .await;
        } else {
            self.probe_connection_status(
                "P6 connection status after pause request failure",
                &[
                    crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING,
                    crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING_PAUSED,
                    crate::unity_bridge::UNITY_EDITOR_STATUS_DISCONNECTED,
                ],
                false,
            )
            .await;
        }

        self.probe_paused_update_pump_resilience(pid, hint).await;

        self.set_editor_status_step(
            "P7 resume play mode",
            crate::unity_bridge::UNITY_EDITOR_STATUS_PLAYING,
            PAUSE_TRANSITION_TIMEOUT,
            pid,
            hint,
        )
        .await;

        match self
            .wait_for_semantic_phase(&["playing"], Duration::from_secs(15))
            .await
        {
            Ok(state) => self.pass(
                "P8 fused resumed state",
                format!("phase={} source={}", state.phase, state.source),
            ),
            Err(error) => self.fail("P8 fused resumed state", error),
        }

        self.set_editor_status_step(
            "P9 exit play mode",
            crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING,
            PLAY_TRANSITION_TIMEOUT,
            pid,
            hint,
        )
        .await;

        match self
            .wait_for_semantic_phase(&["editing"], Duration::from_secs(15))
            .await
        {
            Ok(state) => self.pass(
                "P10 fused post-play edit state",
                format!("phase={} source={}", state.phase, state.source),
            ),
            Err(error) => self.fail("P10 fused post-play edit state", error),
        }

        self.probe_connection_status(
            "P11 connection status after play mode",
            &[crate::unity_bridge::UNITY_EDITOR_STATUS_EDITING],
            true,
        )
        .await;
    }

    async fn run(&mut self) {
        // Gate: editor must be connected and in edit mode.
        let (connected, status_str, _) =
            crate::unity_bridge::query_unity_status(&self.project).await;
        if !connected {
            self.fail(
                "precondition",
                "Unity Editor is not connected. Open the project in Unity and wait for the \
                 Locus bridge to connect, then retry.",
            );
            return;
        }
        if crate::unity_bridge::is_play_mode_status(status_str) {
            self.fail(
                "precondition",
                "Unity Editor is in play mode. Exit play mode before running the state-probe \
                 self-test.",
            );
            return;
        }

        if !self.check_installed_plugin_files() {
            return;
        }
        if !self.check_bridge_capabilities().await {
            return;
        }

        let (pid, hint) = match self.editor_process().await {
            Ok(value) => value,
            Err(error) => {
                self.fail("precondition", error);
                return;
            }
        };

        self.run_baseline(pid, &hint).await;
        self.run_perf_risk(pid, &hint).await;
        self.run_reload(pid, &hint).await;
        self.run_play(pid, &hint).await;
        self.restore_edit_mode().await;
    }
}

/// Entry point invoked by the `unity_state_probe_selftest_run` command.
pub async fn run(app: tauri::AppHandle, project: String) -> Result<(), String> {
    if project.trim().is_empty() {
        return Err("No workspace selected".to_string());
    }
    if !crate::unity_bridge::is_unity_project(&project) {
        return Err("Current workspace is not a Unity project".to_string());
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err("A state-probe self-test is already running".to_string());
    }

    let mut test = SelfTest {
        app,
        project,
        passed: 0,
        failed: 0,
    };
    test.log("Unity state-probe self-test starting…");
    test.run().await;
    let summary = format!("Finished: {} passed, {} failed", test.passed, test.failed);
    test.log(summary);
    test.emit(None, true);

    RUNNING.store(false, Ordering::SeqCst);
    Ok(())
}
