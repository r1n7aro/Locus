#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingInvokeSnapshot {
    pub command: String,
    pub age_ms: u64,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendLifecycleEvent {
    pub event: String,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub href: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendBridgeStallSnapshot {
    pub id: String,
    pub detected_at_ms: u64,
    pub reason: String,
    pub heartbeat: Box<FrontendBridgeHeartbeat>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendBridgeHeartbeat {
    pub sequence: u64,
    pub sent_at_ms: u64,
    pub session_id: String,
    pub href: String,
    pub ready_state: String,
    pub visibility_state: String,
    pub navigation_type: Option<String>,
    pub performance_now_ms: f64,
    pub event_loop_lag_ms: f64,
    pub callback_count: Option<u64>,
    #[serde(default)]
    pub pending_invokes: Vec<PendingInvokeSnapshot>,
    #[serde(default)]
    pub lifecycle: Vec<FrontendLifecycleEvent>,
    pub recovered_stall: Option<Box<FrontendBridgeStallSnapshot>>,
}

#[cfg(target_os = "windows")]
mod imp {
    use super::FrontendBridgeHeartbeat;
    use std::collections::HashSet;
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use futures::{SinkExt, StreamExt};
    use http_body_util::Full;
    use hyper::body::{Bytes, Incoming};
    use hyper::header::{
        CONNECTION, CONTENT_TYPE, HOST, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
        SEC_WEBSOCKET_VERSION, UPGRADE,
    };
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode, Version};
    use hyper_util::rt::TokioIo;
    use serde_json::{json, Value};
    use tauri::{AppHandle, Manager};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, oneshot, watch};
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
    use tokio_tungstenite::tungstenite::protocol::{Message, Role};
    use tokio_tungstenite::WebSocketStream;

    const MAIN_WINDOW_LABEL: &str = "main";
    const MAIN_TARGET_ID: &str = "main";
    const MAIN_TARGET_SESSION_PREFIX: &str = "locus-main-session";
    const DEBUG_PORT_START: u16 = 19_222;
    const DEBUG_PORT_ATTEMPTS: u16 = 25;
    const CDP_CALL_TIMEOUT: Duration = Duration::from_secs(30);
    const BRIDGE_PROBE_TIMEOUT: Duration = Duration::from_secs(4);
    const BRIDGE_PROBE_INTERVAL: Duration = Duration::from_secs(5);
    const BRIDGE_STALLED_PROBE_BACKOFF_MS: u64 = 60_000;
    const BRIDGE_HEARTBEAT_STALE_MS: u64 = 15_000;
    const BRIDGE_STARTUP_GRACE_MS: u64 = 20_000;
    const EVENT_QUEUE_CAPACITY: usize = 2_048;
    const CDP_EVENT_NAMES: &[&str] = &[
        "Accessibility.loadComplete",
        "Accessibility.nodesUpdated",
        "Animation.animationCanceled",
        "Animation.animationCreated",
        "Animation.animationStarted",
        "Animation.animationUpdated",
        "Audits.issueAdded",
        "Autofill.addressFormFilled",
        "BackgroundService.recordingStateChanged",
        "BackgroundService.backgroundServiceEventReceived",
        "Browser.downloadWillBegin",
        "Browser.downloadProgress",
        "CSS.fontsUpdated",
        "CSS.mediaQueryResultChanged",
        "CSS.styleSheetAdded",
        "CSS.styleSheetChanged",
        "CSS.styleSheetRemoved",
        "Cast.sinksUpdated",
        "Cast.issueUpdated",
        "DOM.attributeModified",
        "DOM.attributeRemoved",
        "DOM.characterDataModified",
        "DOM.childNodeCountUpdated",
        "DOM.childNodeInserted",
        "DOM.childNodeRemoved",
        "DOM.distributedNodesUpdated",
        "DOM.documentUpdated",
        "DOM.inlineStyleInvalidated",
        "DOM.pseudoElementAdded",
        "DOM.topLayerElementsUpdated",
        "DOM.pseudoElementRemoved",
        "DOM.setChildNodes",
        "DOM.shadowRootPopped",
        "DOM.shadowRootPushed",
        "DOMStorage.domStorageItemAdded",
        "DOMStorage.domStorageItemRemoved",
        "DOMStorage.domStorageItemUpdated",
        "DOMStorage.domStorageItemsCleared",
        "Database.addDatabase",
        "Emulation.virtualTimeBudgetExpired",
        "Input.dragIntercepted",
        "Inspector.detached",
        "Inspector.targetCrashed",
        "Inspector.targetReloadedAfterCrash",
        "LayerTree.layerPainted",
        "LayerTree.layerTreeDidChange",
        "Log.entryAdded",
        "Network.dataReceived",
        "Network.eventSourceMessageReceived",
        "Network.loadingFailed",
        "Network.loadingFinished",
        "Network.requestIntercepted",
        "Network.requestServedFromCache",
        "Network.requestWillBeSent",
        "Network.resourceChangedPriority",
        "Network.signedExchangeReceived",
        "Network.responseReceived",
        "Network.webSocketClosed",
        "Network.webSocketCreated",
        "Network.webSocketFrameError",
        "Network.webSocketFrameReceived",
        "Network.webSocketFrameSent",
        "Network.webSocketHandshakeResponseReceived",
        "Network.webSocketWillSendHandshakeRequest",
        "Network.webTransportCreated",
        "Network.webTransportConnectionEstablished",
        "Network.webTransportClosed",
        "Network.requestWillBeSentExtraInfo",
        "Network.responseReceivedExtraInfo",
        "Network.responseReceivedEarlyHints",
        "Network.trustTokenOperationDone",
        "Network.subresourceWebBundleMetadataReceived",
        "Network.subresourceWebBundleMetadataError",
        "Network.subresourceWebBundleInnerResponseParsed",
        "Network.subresourceWebBundleInnerResponseError",
        "Network.reportingApiReportAdded",
        "Network.reportingApiReportUpdated",
        "Network.reportingApiEndpointsChangedForOrigin",
        "Overlay.inspectNodeRequested",
        "Overlay.nodeHighlightRequested",
        "Overlay.screenshotRequested",
        "Overlay.inspectModeCanceled",
        "Page.domContentEventFired",
        "Page.fileChooserOpened",
        "Page.frameAttached",
        "Page.frameClearedScheduledNavigation",
        "Page.frameDetached",
        "Page.frameNavigated",
        "Page.documentOpened",
        "Page.frameResized",
        "Page.frameRequestedNavigation",
        "Page.frameScheduledNavigation",
        "Page.frameStartedLoading",
        "Page.frameStoppedLoading",
        "Page.downloadWillBegin",
        "Page.downloadProgress",
        "Page.interstitialHidden",
        "Page.interstitialShown",
        "Page.javascriptDialogClosed",
        "Page.javascriptDialogOpening",
        "Page.lifecycleEvent",
        "Page.backForwardCacheNotUsed",
        "Page.loadEventFired",
        "Page.navigatedWithinDocument",
        "Page.screencastFrame",
        "Page.screencastVisibilityChanged",
        "Page.windowOpen",
        "Page.compilationCacheProduced",
        "Performance.metrics",
        "PerformanceTimeline.timelineEventAdded",
        "Security.certificateError",
        "Security.visibleSecurityStateChanged",
        "Security.securityStateChanged",
        "ServiceWorker.workerErrorReported",
        "ServiceWorker.workerRegistrationUpdated",
        "ServiceWorker.workerVersionUpdated",
        "Storage.cacheStorageContentUpdated",
        "Storage.cacheStorageListUpdated",
        "Storage.indexedDBContentUpdated",
        "Storage.indexedDBListUpdated",
        "Storage.interestGroupAccessed",
        "Storage.interestGroupAuctionEventOccurred",
        "Storage.interestGroupAuctionNetworkRequestCreated",
        "Storage.sharedStorageAccessed",
        "Storage.storageBucketCreatedOrUpdated",
        "Storage.storageBucketDeleted",
        "Storage.attributionReportingSourceRegistered",
        "Storage.attributionReportingTriggerRegistered",
        "Target.attachedToTarget",
        "Target.detachedFromTarget",
        "Target.receivedMessageFromTarget",
        "Target.targetCreated",
        "Target.targetDestroyed",
        "Target.targetCrashed",
        "Target.targetInfoChanged",
        "Tethering.accepted",
        "Tracing.bufferUsage",
        "Tracing.dataCollected",
        "Tracing.tracingComplete",
        "Fetch.requestPaused",
        "Fetch.authRequired",
        "WebAudio.contextCreated",
        "WebAudio.contextWillBeDestroyed",
        "WebAudio.contextChanged",
        "WebAudio.audioListenerCreated",
        "WebAudio.audioListenerWillBeDestroyed",
        "WebAudio.audioNodeCreated",
        "WebAudio.audioNodeWillBeDestroyed",
        "WebAudio.audioParamCreated",
        "WebAudio.audioParamWillBeDestroyed",
        "WebAudio.nodesConnected",
        "WebAudio.nodesDisconnected",
        "WebAudio.nodeParamConnected",
        "WebAudio.nodeParamDisconnected",
        "WebAuthn.credentialAdded",
        "WebAuthn.credentialAsserted",
        "Media.playerPropertiesChanged",
        "Media.playerEventsAdded",
        "Media.playerMessagesLogged",
        "Media.playerErrorsRaised",
        "Media.playersCreated",
        "DeviceAccess.deviceRequestPrompted",
        "Preload.ruleSetUpdated",
        "Preload.ruleSetRemoved",
        "Preload.preloadEnabledStateUpdated",
        "Preload.prefetchStatusUpdated",
        "Preload.prerenderStatusUpdated",
        "Preload.preloadingAttemptSourcesUpdated",
        "FedCm.dialogShown",
        "FedCm.dialogClosed",
        "Console.messageAdded",
        "Debugger.breakpointResolved",
        "Debugger.paused",
        "Debugger.resumed",
        "Debugger.scriptFailedToParse",
        "Debugger.scriptParsed",
        "HeapProfiler.addHeapSnapshotChunk",
        "HeapProfiler.heapStatsUpdate",
        "HeapProfiler.lastSeenObjectId",
        "HeapProfiler.reportHeapSnapshotProgress",
        "HeapProfiler.resetProfiles",
        "Profiler.consoleProfileFinished",
        "Profiler.consoleProfileStarted",
        "Profiler.preciseCoverageDeltaUpdate",
        "Runtime.bindingCalled",
        "Runtime.consoleAPICalled",
        "Runtime.exceptionRevoked",
        "Runtime.exceptionThrown",
        "Runtime.executionContextCreated",
        "Runtime.executionContextDestroyed",
        "Runtime.executionContextsCleared",
        "Runtime.inspectRequested",
    ];

    type HttpBody = Full<Bytes>;

    pub struct CdpDebugServerHandle {
        inner: tokio::sync::Mutex<RunningState>,
        bridge: BridgeDiagnosticState,
    }

    impl Default for CdpDebugServerHandle {
        fn default() -> Self {
            Self {
                inner: tokio::sync::Mutex::new(RunningState::default()),
                bridge: BridgeDiagnosticState::default(),
            }
        }
    }

    impl CdpDebugServerHandle {
        pub fn record_frontend_heartbeat(&self, heartbeat: FrontendBridgeHeartbeat) {
            if !self.bridge.enabled.load(Ordering::Relaxed) {
                return;
            }
            let now = unix_time_millis();
            self.bridge
                .last_frontend_heartbeat_ms
                .store(now, Ordering::Relaxed);
            if let Some(recovered) = heartbeat.recovered_stall.as_ref() {
                let is_new = self
                    .bridge
                    .last_recovered_stall_id
                    .lock()
                    .map(|mut last_id| {
                        if last_id.as_deref() == Some(recovered.id.as_str()) {
                            false
                        } else {
                            *last_id = Some(recovered.id.clone());
                            true
                        }
                    })
                    .unwrap_or(false);
                if is_new {
                    let snapshot = bounded_json(recovered.as_ref(), 24_000);
                    eprintln!(
                        "[WebViewBridge][warning] recovered frontend stall snapshot={snapshot}"
                    );
                }
            }
            if let Ok(mut slot) = self.bridge.last_frontend_snapshot.lock() {
                *slot = Some(heartbeat);
            }
        }
    }

    #[derive(Default)]
    struct RunningState {
        task: Option<JoinHandle<()>>,
        diagnostic_task: Option<JoinHandle<()>>,
        connection_tasks: Option<Arc<Mutex<Vec<JoinHandle<()>>>>>,
        shutdown: Option<watch::Sender<bool>>,
        port: Option<u16>,
        native_subscriptions: Option<NativeDiagnosticSubscriptions>,
    }

    #[derive(Default)]
    struct BridgeDiagnosticState {
        enabled: AtomicBool,
        started_at_ms: AtomicU64,
        last_frontend_heartbeat_ms: AtomicU64,
        last_frontend_snapshot: Mutex<Option<FrontendBridgeHeartbeat>>,
        last_recovered_stall_id: Mutex<Option<String>>,
    }

    #[derive(Debug, Clone, Copy)]
    struct NativeDiagnosticSubscriptions {
        navigation_starting: i64,
        navigation_completed: i64,
        process_failed: i64,
    }

    #[derive(Debug, Clone)]
    struct EventSubscription {
        name: String,
        token: i64,
    }

    #[derive(Debug, Clone)]
    struct NativeCdpEvent {
        method: String,
        params: Value,
        session_id: Option<String>,
    }

    #[derive(Debug, Default)]
    struct BrowserConnectionState {
        next_session_sequence: u64,
        sessions: Vec<String>,
        session_lookup: HashSet<String>,
        auto_attach_session_id: Option<String>,
    }

    impl BrowserConnectionState {
        fn attach(&mut self) -> String {
            self.next_session_sequence = self.next_session_sequence.saturating_add(1);
            let session_id = format!(
                "{MAIN_TARGET_SESSION_PREFIX}-{}",
                self.next_session_sequence
            );
            self.sessions.push(session_id.clone());
            self.session_lookup.insert(session_id.clone());
            session_id
        }

        fn ensure_auto_attach(&mut self) -> (String, bool) {
            if let Some(session_id) = self.auto_attach_session_id.as_ref() {
                return (session_id.clone(), false);
            }
            let session_id = self.attach();
            self.auto_attach_session_id = Some(session_id.clone());
            (session_id, true)
        }

        fn disable_auto_attach(&mut self) -> Option<String> {
            let session_id = self.auto_attach_session_id.take()?;
            self.detach(&session_id);
            Some(session_id)
        }

        fn detach(&mut self, session_id: &str) -> bool {
            if !self.session_lookup.remove(session_id) {
                return false;
            }
            self.sessions.retain(|value| value != session_id);
            if self.auto_attach_session_id.as_deref() == Some(session_id) {
                self.auto_attach_session_id = None;
            }
            true
        }

        fn contains(&self, session_id: &str) -> bool {
            self.session_lookup.contains(session_id)
        }

        fn is_attached(&self) -> bool {
            !self.sessions.is_empty()
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ConnectionMode {
        Page,
        Browser,
    }

    pub async fn reconcile(app: AppHandle, enabled: bool) -> Result<Option<u16>, String> {
        let handle = app.state::<Arc<CdpDebugServerHandle>>().inner().clone();
        let mut running = handle.inner.lock().await;

        if !enabled {
            stop_locked(&app, &handle, &mut running).await;
            return Ok(None);
        }
        if running.task.is_some() {
            return Ok(running.port);
        }

        let (listener, port) = bind_listener().await?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let native_subscriptions = subscribe_native_diagnostics(&app).await?;
        let connection_tasks = Arc::new(Mutex::new(Vec::new()));
        let server_app = app.clone();
        let server_connections = Arc::clone(&connection_tasks);
        let server_shutdown = shutdown_rx.clone();
        let task = tokio::spawn(async move {
            serve(
                listener,
                port,
                server_app,
                server_shutdown,
                server_connections,
            )
            .await;
        });
        handle.bridge.enabled.store(true, Ordering::Relaxed);
        handle
            .bridge
            .started_at_ms
            .store(unix_time_millis(), Ordering::Relaxed);
        handle
            .bridge
            .last_frontend_heartbeat_ms
            .store(0, Ordering::Relaxed);
        if let Ok(mut snapshot) = handle.bridge.last_frontend_snapshot.lock() {
            *snapshot = None;
        }
        let diagnostic_app = app.clone();
        let diagnostic_handle = Arc::clone(&handle);
        let diagnostic_task = tokio::spawn(async move {
            monitor_bridge_health(diagnostic_app, diagnostic_handle, shutdown_rx).await;
        });

        running.task = Some(task);
        running.diagnostic_task = Some(diagnostic_task);
        running.connection_tasks = Some(connection_tasks);
        running.shutdown = Some(shutdown_tx);
        running.port = Some(port);
        running.native_subscriptions = Some(native_subscriptions);
        eprintln!("[CdpDebug] listening on http://127.0.0.1:{port}");
        eprintln!("[WebViewBridge] debug diagnostics enabled");
        Ok(Some(port))
    }

    async fn stop_locked(
        app: &AppHandle,
        handle: &CdpDebugServerHandle,
        running: &mut RunningState,
    ) {
        handle.bridge.enabled.store(false, Ordering::Relaxed);
        if let Some(shutdown) = running.shutdown.take() {
            let _ = shutdown.send(true);
        }
        if let Some(tasks) = running.connection_tasks.take() {
            let mut tasks = tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for task in tasks.drain(..) {
                task.abort();
            }
        }
        if let Some(task) = running.task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = running.diagnostic_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(subscriptions) = running.native_subscriptions.take() {
            unsubscribe_native_diagnostics(app, subscriptions).await;
        }
        if let Some(port) = running.port.take() {
            eprintln!("[CdpDebug] stopped listening on 127.0.0.1:{port}");
        }
        eprintln!("[WebViewBridge] debug diagnostics disabled");
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum BridgeHealth {
        Healthy,
        RendererToHostStalled,
        HostToRendererStalled,
        BidirectionalStalled,
    }

    fn classify_bridge_health(
        frontend_heartbeat_stalled: bool,
        cdp_probe_stalled: bool,
    ) -> BridgeHealth {
        match (frontend_heartbeat_stalled, cdp_probe_stalled) {
            (false, false) => BridgeHealth::Healthy,
            (true, false) => BridgeHealth::RendererToHostStalled,
            (false, true) => BridgeHealth::HostToRendererStalled,
            (true, true) => BridgeHealth::BidirectionalStalled,
        }
    }

    async fn monitor_bridge_health(
        app: AppHandle,
        handle: Arc<CdpDebugServerHandle>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut interval = tokio::time::interval(BRIDGE_PROBE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Avoid probing during the first renderer bootstrap turn.
        interval.tick().await;
        let mut last_health = BridgeHealth::Healthy;
        let mut heartbeat_miss_streak = 0u32;
        let mut cdp_failure_streak = 0u32;
        let mut next_cdp_probe_at_ms = 0u64;
        let mut last_cdp_error: Option<String> = None;
        let mut last_cdp_snapshot: Option<Value> = None;

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                _ = interval.tick() => {
                    if !handle.bridge.enabled.load(Ordering::Relaxed) {
                        break;
                    }
                    let now = unix_time_millis();
                    let started_at = handle.bridge.started_at_ms.load(Ordering::Relaxed);
                    let last_heartbeat = handle
                        .bridge
                        .last_frontend_heartbeat_ms
                        .load(Ordering::Relaxed);
                    let startup_grace_elapsed = now.saturating_sub(started_at)
                        >= BRIDGE_STARTUP_GRACE_MS;
                    let heartbeat_age_ms = if last_heartbeat == 0 {
                        None
                    } else {
                        Some(now.saturating_sub(last_heartbeat))
                    };
                    let heartbeat_overdue = startup_grace_elapsed
                        && heartbeat_age_ms
                            .map(|age| age >= BRIDGE_HEARTBEAT_STALE_MS)
                            .unwrap_or(true);
                    heartbeat_miss_streak = if heartbeat_overdue {
                        heartbeat_miss_streak.saturating_add(1)
                    } else {
                        0
                    };

                    if now >= next_cdp_probe_at_ms {
                        match probe_renderer_state(&app).await {
                            Ok(snapshot) => {
                                cdp_failure_streak = 0;
                                last_cdp_error = None;
                                last_cdp_snapshot = Some(snapshot);
                                next_cdp_probe_at_ms = 0;
                            }
                            Err(error) => {
                                cdp_failure_streak = cdp_failure_streak.saturating_add(1);
                                last_cdp_error = Some(error);
                                if cdp_failure_streak >= 2 {
                                    next_cdp_probe_at_ms = now
                                        .saturating_add(BRIDGE_STALLED_PROBE_BACKOFF_MS);
                                }
                            }
                        }
                    }

                    let heartbeat_stalled = heartbeat_miss_streak >= 2
                        || (heartbeat_overdue && cdp_failure_streak > 0);
                    let cdp_stalled = cdp_failure_streak >= 2
                        || (heartbeat_overdue && cdp_failure_streak > 0);
                    let health = classify_bridge_health(heartbeat_stalled, cdp_stalled);
                    if health == last_health {
                        continue;
                    }

                    if health == BridgeHealth::Healthy {
                        eprintln!(
                            "[WebViewBridge] bridge recovered previous_state={last_health:?} heartbeat_age_ms={:?}",
                            heartbeat_age_ms
                        );
                    } else {
                        let frontend_snapshot = handle
                            .bridge
                            .last_frontend_snapshot
                            .lock()
                            .ok()
                            .and_then(|snapshot| snapshot.clone())
                            .map(|snapshot| bounded_json(&snapshot, 24_000))
                            .unwrap_or_else(|| "null".to_string());
                        let cdp_snapshot = last_cdp_snapshot
                            .as_ref()
                            .map(|snapshot| bounded_json(snapshot, 12_000))
                            .unwrap_or_else(|| "null".to_string());
                        let cdp_error = last_cdp_error.as_deref().unwrap_or("none");
                        eprintln!(
                            "[WebViewBridge][warning] bridge stall detected state={health:?} heartbeat_age_ms={:?} heartbeat_miss_streak={} cdp_failure_streak={} cdp_error={} frontend_snapshot={} cdp_snapshot={}",
                            heartbeat_age_ms,
                            heartbeat_miss_streak,
                            cdp_failure_streak,
                            cdp_error,
                            frontend_snapshot,
                            cdp_snapshot,
                        );
                    }
                    last_health = health;
                }
            }
        }
    }

    async fn probe_renderer_state(app: &AppHandle) -> Result<Value, String> {
        let expression = r#"(() => ({
            timestampMs: Date.now(),
            href: location.href,
            readyState: document.readyState,
            visibilityState: document.visibilityState,
            callbackCount: window.__TAURI_INTERNALS__?.callbacks?.size ?? null,
            performanceNowMs: performance.now()
        }))()"#;
        call_devtools_method_with_timeout(
            app,
            None,
            "Runtime.evaluate".to_string(),
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false,
            }),
            BRIDGE_PROBE_TIMEOUT,
        )
        .await
    }

    pub(crate) async fn evaluate_main_webview(
        app: &AppHandle,
        expression: &str,
        timeout: Duration,
    ) -> Result<Value, String> {
        let response = call_devtools_method_with_timeout(
            app,
            None,
            "Runtime.evaluate".to_string(),
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
            timeout,
        )
        .await?;
        if let Some(exception) = response.get("exceptionDetails") {
            return Err(format!("WebView evaluation failed: {exception}"));
        }
        response
            .pointer("/result/value")
            .cloned()
            .ok_or_else(|| format!("WebView evaluation returned no value: {response}"))
    }

    async fn subscribe_native_diagnostics(
        app: &AppHandle,
    ) -> Result<NativeDiagnosticSubscriptions, String> {
        use webview2_com::{
            take_pwstr,
            Microsoft::Web::WebView2::Win32::{
                ICoreWebView2, ICoreWebView2ProcessFailedEventArgs2,
                ICoreWebView2ProcessFailedEventArgs3,
            },
            NavigationCompletedEventHandler, NavigationStartingEventHandler,
            ProcessFailedEventHandler,
        };
        use windows_core::{Interface, BOOL, PWSTR};

        let window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "The main WebView2 window is unavailable".to_string())?;
        let (result_tx, result_rx) = oneshot::channel();
        window
            .with_webview(move |webview| {
                let result = (|| -> Result<NativeDiagnosticSubscriptions, String> {
                    let core: ICoreWebView2 = unsafe { webview.controller().CoreWebView2() }
                        .map_err(|error| format!("Failed to access WebView2 core: {error}"))?;

                    let navigation_starting = NavigationStartingEventHandler::create(Box::new(
                        move |sender, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut raw_uri = PWSTR::null();
                            let mut navigation_id = 0u64;
                            let mut user_initiated = BOOL(0);
                            let mut redirected = BOOL(0);
                            let _ = unsafe { args.Uri(&mut raw_uri) };
                            let _ = unsafe { args.NavigationId(&mut navigation_id) };
                            let _ = unsafe { args.IsUserInitiated(&mut user_initiated) };
                            let _ = unsafe { args.IsRedirected(&mut redirected) };
                            let uri = take_pwstr(raw_uri);
                            let mut browser_pid = 0u32;
                            if let Some(sender) = sender {
                                let _ = unsafe { sender.BrowserProcessId(&mut browser_pid) };
                            }
                            eprintln!(
                                "[WebViewBridge] navigation starting id={} user_initiated={} redirected={} browser_pid={} uri={}",
                                navigation_id,
                                user_initiated.as_bool(),
                                redirected.as_bool(),
                                browser_pid,
                                bounded_text(&uri, 2_000),
                            );
                            Ok(())
                        },
                    ));
                    let mut navigation_starting_token = 0i64;
                    unsafe {
                        core.add_NavigationStarting(
                            &navigation_starting,
                            &mut navigation_starting_token,
                        )
                    }
                    .map_err(|error| {
                        format!("Failed to subscribe NavigationStarting: {error}")
                    })?;

                    let navigation_completed = NavigationCompletedEventHandler::create(Box::new(
                        move |sender, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut navigation_id = 0u64;
                            let mut success = BOOL(0);
                            let mut web_error = Default::default();
                            let _ = unsafe { args.NavigationId(&mut navigation_id) };
                            let _ = unsafe { args.IsSuccess(&mut success) };
                            let _ = unsafe { args.WebErrorStatus(&mut web_error) };
                            let mut raw_uri = PWSTR::null();
                            if let Some(sender) = sender {
                                let _ = unsafe { sender.Source(&mut raw_uri) };
                            }
                            let uri = take_pwstr(raw_uri);
                            eprintln!(
                                "[WebViewBridge] navigation completed id={} success={} web_error={} uri={}",
                                navigation_id,
                                success.as_bool(),
                                web_error.0,
                                bounded_text(&uri, 2_000),
                            );
                            Ok(())
                        },
                    ));
                    let mut navigation_completed_token = 0i64;
                    unsafe {
                        core.add_NavigationCompleted(
                            &navigation_completed,
                            &mut navigation_completed_token,
                        )
                    }
                    .map_err(|error| {
                        format!("Failed to subscribe NavigationCompleted: {error}")
                    })?;

                    let process_failed = ProcessFailedEventHandler::create(Box::new(
                        move |sender, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut kind = Default::default();
                            let _ = unsafe { args.ProcessFailedKind(&mut kind) };
                            let mut browser_pid = 0u32;
                            if let Some(sender) = sender {
                                let _ = unsafe { sender.BrowserProcessId(&mut browser_pid) };
                            }
                            let mut reason_value = None;
                            let mut exit_code = None;
                            let mut description = String::new();
                            let mut failure_module = String::new();
                            if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                                let mut reason = Default::default();
                                let mut raw_description = PWSTR::null();
                                let mut raw_exit_code = 0i32;
                                if unsafe { args2.Reason(&mut reason) }.is_ok() {
                                    reason_value = Some(reason.0);
                                }
                                if unsafe { args2.ExitCode(&mut raw_exit_code) }.is_ok() {
                                    exit_code = Some(raw_exit_code);
                                }
                                if unsafe { args2.ProcessDescription(&mut raw_description) }.is_ok()
                                {
                                    description = take_pwstr(raw_description);
                                }
                            }
                            if let Ok(args3) = args.cast::<ICoreWebView2ProcessFailedEventArgs3>() {
                                let mut raw_module = PWSTR::null();
                                if unsafe { args3.FailureSourceModulePath(&mut raw_module) }.is_ok() {
                                    failure_module = take_pwstr(raw_module);
                                }
                            }
                            eprintln!(
                                "[WebViewBridge][warning] WebView2 process failed kind={} reason={:?} exit_code={:?} browser_pid={} description={} failure_module={}",
                                kind.0,
                                reason_value,
                                exit_code,
                                browser_pid,
                                bounded_text(&description, 4_000),
                                bounded_text(&failure_module, 2_000),
                            );
                            Ok(())
                        },
                    ));
                    let mut process_failed_token = 0i64;
                    unsafe { core.add_ProcessFailed(&process_failed, &mut process_failed_token) }
                        .map_err(|error| format!("Failed to subscribe ProcessFailed: {error}"))?;

                    Ok(NativeDiagnosticSubscriptions {
                        navigation_starting: navigation_starting_token,
                        navigation_completed: navigation_completed_token,
                        process_failed: process_failed_token,
                    })
                })();
                let _ = result_tx.send(result);
            })
            .map_err(|error| format!("Failed to access the main WebView2: {error}"))?;

        tokio::time::timeout(Duration::from_secs(3), result_rx)
            .await
            .map_err(|_| "WebView2 diagnostic subscription timed out".to_string())?
            .map_err(|_| "WebView2 diagnostic subscription channel closed".to_string())?
    }

    async fn unsubscribe_native_diagnostics(
        app: &AppHandle,
        subscriptions: NativeDiagnosticSubscriptions,
    ) {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2;

        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        let (done_tx, done_rx) = oneshot::channel();
        if window
            .with_webview(move |webview| {
                if let Ok(core) =
                    unsafe { webview.controller().CoreWebView2() }.map(|core: ICoreWebView2| core)
                {
                    let _ = unsafe {
                        core.remove_NavigationStarting(subscriptions.navigation_starting)
                    };
                    let _ = unsafe {
                        core.remove_NavigationCompleted(subscriptions.navigation_completed)
                    };
                    let _ = unsafe { core.remove_ProcessFailed(subscriptions.process_failed) };
                }
                let _ = done_tx.send(());
            })
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), done_rx).await;
        }
    }

    fn unix_time_millis() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    fn bounded_text(value: &str, max_chars: usize) -> String {
        let mut chars = value.chars();
        let bounded = chars.by_ref().take(max_chars).collect::<String>();
        if chars.next().is_some() {
            format!("{bounded} …(truncated)")
        } else {
            bounded
        }
    }

    fn bounded_json<T: serde::Serialize>(value: &T, max_chars: usize) -> String {
        let serialized = serde_json::to_string(value)
            .unwrap_or_else(|error| format!("{{\"serializationError\":\"{error}\"}}"));
        bounded_text(&serialized, max_chars)
    }

    async fn bind_listener() -> Result<(TcpListener, u16), String> {
        let mut errors = Vec::new();
        for offset in 0..DEBUG_PORT_ATTEMPTS {
            let port = DEBUG_PORT_START + offset;
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => return Ok((listener, port)),
                Err(error) => errors.push(format!("{port}: {error}")),
            }
        }
        Err(format!(
            "No CDP debug port is available in {}-{} ({})",
            DEBUG_PORT_START,
            DEBUG_PORT_START + DEBUG_PORT_ATTEMPTS - 1,
            errors.join("; ")
        ))
    }

    async fn serve(
        listener: TcpListener,
        port: u16,
        app: AppHandle,
        mut shutdown: watch::Receiver<bool>,
        connection_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    ) {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _peer)) => {
                            let app = app.clone();
                            let shutdown = shutdown.clone();
                            let connection_tasks = Arc::clone(&connection_tasks);
                            connections.spawn(async move {
                                let service = service_fn(move |request| {
                                    handle_request(
                                        request,
                                        app.clone(),
                                        port,
                                        shutdown.clone(),
                                        Arc::clone(&connection_tasks),
                                    )
                                });
                                let connection = http1::Builder::new()
                                    .serve_connection(TokioIo::new(stream), service)
                                    .with_upgrades();
                                if let Err(error) = connection.await {
                                    eprintln!("[CdpDebug] HTTP connection ended: {error}");
                                }
                            });
                            while connections.try_join_next().is_some() {}
                        }
                        Err(error) => {
                            eprintln!("[CdpDebug] accept failed: {error}");
                        }
                    }
                }
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    }

    async fn handle_request(
        mut request: Request<Incoming>,
        app: AppHandle,
        port: u16,
        shutdown: watch::Receiver<bool>,
        connection_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    ) -> Result<Response<HttpBody>, Infallible> {
        if !host_allowed(
            request
                .headers()
                .get(HOST)
                .and_then(|value| value.to_str().ok()),
        ) {
            return Ok(text_response(StatusCode::FORBIDDEN, "invalid host"));
        }
        if request.method() != Method::GET {
            return Ok(text_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "GET required",
            ));
        }

        let path = request.uri().path();
        if matches!(path, "/json" | "/json/list") {
            return Ok(json_response(
                StatusCode::OK,
                &Value::Array(vec![target_descriptor(&app, port)]),
            ));
        }
        if path == "/json/version" {
            return Ok(json_response(StatusCode::OK, &version_descriptor(port)));
        }
        if matches!(path, "/devtools/page/main" | "/devtools/browser/locus") {
            let mode = if path == "/devtools/browser/locus" {
                ConnectionMode::Browser
            } else {
                ConnectionMode::Page
            };
            return Ok(
                match websocket_upgrade_response(
                    &mut request,
                    app,
                    shutdown,
                    connection_tasks,
                    mode,
                ) {
                    Ok(response) => response,
                    Err(message) => text_response(StatusCode::BAD_REQUEST, &message),
                },
            );
        }
        Ok(text_response(StatusCode::NOT_FOUND, "not found"))
    }

    fn host_allowed(host: Option<&str>) -> bool {
        let Some(host) = host else { return false };
        let bare = host
            .rsplit_once(':')
            .map(|(value, _)| value)
            .unwrap_or(host);
        matches!(bare, "127.0.0.1" | "localhost")
    }

    fn target_descriptor(app: &AppHandle, port: u16) -> Value {
        let (title, url) = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .map(|window| {
                let title = window.title().unwrap_or_else(|_| "Locus".to_string());
                let url = window
                    .url()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| "tauri://localhost".to_string());
                (title, url)
            })
            .unwrap_or_else(|| ("Locus".to_string(), "tauri://localhost".to_string()));
        json!({
            "description": "Locus main WebView2",
            "devtoolsFrontendUrl": format!("devtools://devtools/bundled/inspector.html?ws=127.0.0.1:{port}/devtools/page/main"),
            "id": MAIN_TARGET_ID,
            "title": title,
            "type": "page",
            "url": url,
            "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/main"),
        })
    }

    fn version_descriptor(port: u16) -> Value {
        json!({
            "Browser": "Locus/WebView2",
            "Protocol-Version": "1.3",
            "User-Agent": "Locus WebView2",
            "V8-Version": "",
            "WebKit-Version": "",
            "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/browser/locus"),
        })
    }

    fn main_target_info(app: &AppHandle, attached: bool) -> Value {
        let descriptor = target_descriptor(app, 0);
        json!({
            "targetId": MAIN_TARGET_ID,
            "type": "page",
            "title": descriptor["title"],
            "url": descriptor["url"],
            "attached": attached,
            "canAccessOpener": false,
        })
    }

    fn websocket_upgrade_response(
        request: &mut Request<Incoming>,
        app: AppHandle,
        shutdown: watch::Receiver<bool>,
        connection_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
        mode: ConnectionMode,
    ) -> Result<Response<HttpBody>, String> {
        let headers = request.headers();
        let connection_upgrade = headers
            .get(CONNECTION)
            .and_then(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(|character| character == ' ' || character == ',')
                    .any(|part| part.eq_ignore_ascii_case("upgrade"))
            })
            .unwrap_or(false);
        let websocket_upgrade = headers
            .get(UPGRADE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.eq_ignore_ascii_case("websocket"))
            .unwrap_or(false);
        let websocket_version = headers
            .get(SEC_WEBSOCKET_VERSION)
            .map(|value| value == "13")
            .unwrap_or(false);
        let key = headers
            .get(SEC_WEBSOCKET_KEY)
            .ok_or_else(|| "missing Sec-WebSocket-Key".to_string())?;
        if request.version() < Version::HTTP_11
            || !connection_upgrade
            || !websocket_upgrade
            || !websocket_version
        {
            return Err("invalid WebSocket upgrade request".to_string());
        }

        let accept_key = derive_accept_key(key.as_bytes());
        let on_upgrade = hyper::upgrade::on(request);
        let task = tokio::spawn(async move {
            match on_upgrade.await {
                Ok(upgraded) => {
                    let websocket = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        Role::Server,
                        None,
                    )
                    .await;
                    handle_websocket(app, websocket, shutdown, mode).await;
                }
                Err(error) => eprintln!("[CdpDebug] WebSocket upgrade failed: {error}"),
            }
        });
        connection_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(task);

        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .version(Version::HTTP_11)
            .header(CONNECTION, "Upgrade")
            .header(UPGRADE, "websocket")
            .header(SEC_WEBSOCKET_ACCEPT, accept_key)
            .body(HttpBody::default())
            .expect("static WebSocket response builds"))
    }

    async fn handle_websocket(
        app: AppHandle,
        websocket: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
        mut shutdown: watch::Receiver<bool>,
        mode: ConnectionMode,
    ) {
        let (event_tx, mut event_rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        let _event_tx_guard = event_tx.clone();
        let subscriptions = match subscribe_to_events(&app, event_tx).await {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                eprintln!("[CdpDebug] failed to subscribe to CDP events: {error}");
                Vec::new()
            }
        };
        let mut browser_state = BrowserConnectionState::default();
        let (mut outgoing, mut incoming) = websocket.split();

        'connection: loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        let _ = outgoing.send(Message::Close(None)).await;
                        break;
                    }
                }
                event = event_rx.recv() => {
                    let Some(event) = event else { break };
                    for message in event_messages(event, mode, &browser_state) {
                        if outgoing.send(Message::text(message)).await.is_err() {
                            break 'connection;
                        }
                    }
                }
                message = incoming.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            for response in dispatch_messages(
                                &app,
                                text.as_ref(),
                                mode,
                                &mut browser_state,
                            ).await {
                                if outgoing.send(Message::text(response)).await.is_err() {
                                    break 'connection;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
            }
        }

        unsubscribe_from_events(&app, subscriptions).await;
    }

    fn event_messages(
        event: NativeCdpEvent,
        mode: ConnectionMode,
        browser_state: &BrowserConnectionState,
    ) -> Vec<String> {
        let message = |session_id: Option<&str>| {
            let mut value = json!({
                "method": event.method.clone(),
                "params": event.params.clone(),
            });
            if let Some(session_id) = session_id {
                value["sessionId"] = Value::String(session_id.to_string());
            }
            value.to_string()
        };

        match mode {
            ConnectionMode::Page => vec![message(event.session_id.as_deref())],
            ConnectionMode::Browser if event.method.starts_with("Target.") => {
                vec![message(event.session_id.as_deref())]
            }
            ConnectionMode::Browser => match event.session_id.as_deref() {
                Some(session_id) => vec![message(Some(session_id))],
                None => browser_state
                    .sessions
                    .iter()
                    .map(|session_id| message(Some(session_id)))
                    .collect(),
            },
        }
    }

    async fn dispatch_messages(
        app: &AppHandle,
        raw: &str,
        mode: ConnectionMode,
        browser_state: &mut BrowserConnectionState,
    ) -> Vec<String> {
        let request: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(error) => {
                return vec![json!({
                    "id": Value::Null,
                    "error": { "code": -32700, "message": error.to_string() }
                })
                .to_string()]
            }
        };
        let Some(id) = request.get("id").cloned() else {
            return Vec::new();
        };
        let method = match request.get("method").and_then(Value::as_str) {
            Some(method) if !method.is_empty() => method.to_string(),
            _ => {
                return vec![json!({
                    "id": id,
                    "error": { "code": -32600, "message": "CDP method is required" }
                })
                .to_string()]
            }
        };
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let session_id = request
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        if mode == ConnectionMode::Browser {
            if let Some(messages) = synthetic_browser_messages(
                app,
                &id,
                &method,
                &params,
                session_id.as_deref(),
                browser_state,
            ) {
                return messages
                    .into_iter()
                    .map(|message| message.to_string())
                    .collect();
            }
        }

        let call_session_id = match (mode, session_id.as_deref()) {
            (ConnectionMode::Browser, Some(session_id)) if browser_state.contains(session_id) => {
                None
            }
            (ConnectionMode::Browser, Some(session_id)) => {
                return vec![json!({
                    "id": id,
                    "error": {
                        "code": -32001,
                        "message": format!("Unknown Locus target session: {session_id}"),
                    }
                })
                .to_string()]
            }
            _ => session_id.clone(),
        };

        // WebView2 exposes one in-process page session. Re-arm Runtime when a
        // browser client attaches so it receives the existing execution
        // contexts just like a fresh native CDP target session would.
        if mode == ConnectionMode::Browser
            && session_id
                .as_deref()
                .is_some_and(|session_id| browser_state.contains(session_id))
            && method == "Runtime.enable"
        {
            let _ = call_devtools_method(app, None, "Runtime.disable".to_string(), json!({})).await;
        }

        let mut response = match call_devtools_method(app, call_session_id, method, params).await {
            Ok(result) => match result.get("error") {
                Some(error) => json!({ "id": id, "error": error }),
                None => json!({ "id": id, "result": result }),
            },
            Err(error) => json!({
                "id": id,
                "error": { "code": -32000, "message": error }
            }),
        };
        if mode == ConnectionMode::Browser {
            if let Some(session_id) = session_id {
                response["sessionId"] = Value::String(session_id);
            }
        }
        vec![response.to_string()]
    }

    fn synthetic_browser_messages(
        app: &AppHandle,
        id: &Value,
        method: &str,
        params: &Value,
        session_id: Option<&str>,
        browser_state: &mut BrowserConnectionState,
    ) -> Option<Vec<Value>> {
        let with_session = |mut response: Value| {
            if let Some(session_id) = session_id {
                response["sessionId"] = Value::String(session_id.to_string());
            }
            response
        };
        let response = |result: Value| with_session(json!({ "id": id, "result": result }));
        let error = |code: i64, message: String| {
            with_session(json!({
                "id": id,
                "error": { "code": code, "message": message },
            }))
        };
        let attached_message = |session_id: &str| {
            json!({
                "method": "Target.attachedToTarget",
                "params": {
                    "sessionId": session_id,
                    "targetInfo": main_target_info(app, true),
                    "waitingForDebugger": false,
                }
            })
        };
        match method {
            "Browser.getVersion" => Some(vec![response(json!({
                "protocolVersion": "1.3",
                "product": "Chrome/Locus WebView2",
                "revision": "unknown",
                "userAgent": "Locus WebView2",
                "jsVersion": "unknown",
            }))]),
            "Target.getBrowserContexts" => Some(vec![response(json!({ "browserContextIds": [] }))]),
            "Target.getTargets" => Some(vec![response(json!({
                "targetInfos": [main_target_info(app, browser_state.is_attached())]
            }))]),
            "Target.getTargetInfo" => Some(vec![response(json!({
                "targetInfo": main_target_info(app, browser_state.is_attached())
            }))]),
            "Target.setDiscoverTargets" => {
                let mut messages = Vec::new();
                if params
                    .get("discover")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    messages.push(json!({
                        "method": "Target.targetCreated",
                        "params": {
                            "targetInfo": main_target_info(app, browser_state.is_attached())
                        }
                    }));
                }
                messages.push(response(json!({})));
                Some(messages)
            }
            "Target.setAutoAttach" => {
                let mut messages = Vec::new();
                if session_id.is_none() {
                    let enabled = params
                        .get("autoAttach")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if enabled {
                        let (auto_session_id, created) = browser_state.ensure_auto_attach();
                        if created {
                            messages.push(attached_message(&auto_session_id));
                        }
                    } else if let Some(detached_session_id) = browser_state.disable_auto_attach() {
                        messages.push(json!({
                            "method": "Target.detachedFromTarget",
                            "params": {
                                "sessionId": detached_session_id,
                                "targetId": MAIN_TARGET_ID,
                            }
                        }));
                    }
                }
                messages.push(response(json!({})));
                Some(messages)
            }
            "Target.attachToTarget" => {
                let target_id = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if target_id != MAIN_TARGET_ID {
                    return Some(vec![error(
                        -32602,
                        format!("Unknown Locus target: {target_id}"),
                    )]);
                }
                let attached_session_id = browser_state.attach();
                Some(vec![
                    attached_message(&attached_session_id),
                    response(json!({ "sessionId": attached_session_id })),
                ])
            }
            "Target.detachFromTarget" => {
                let detached_session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !browser_state.detach(detached_session_id) {
                    return Some(vec![error(
                        -32602,
                        format!("Unknown Locus target session: {detached_session_id}"),
                    )]);
                }
                Some(vec![
                    json!({
                        "method": "Target.detachedFromTarget",
                        "params": {
                            "sessionId": detached_session_id,
                            "targetId": MAIN_TARGET_ID,
                        }
                    }),
                    response(json!({})),
                ])
            }
            _ => None,
        }
    }

    async fn call_devtools_method(
        app: &AppHandle,
        session_id: Option<String>,
        method: String,
        params: Value,
    ) -> Result<Value, String> {
        call_devtools_method_with_timeout(app, session_id, method, params, CDP_CALL_TIMEOUT).await
    }

    async fn call_devtools_method_with_timeout(
        app: &AppHandle,
        session_id: Option<String>,
        method: String,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        use webview2_com::{
            CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR,
            Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2_11},
        };
        use windows_core::Interface;

        let window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "The main WebView2 window is unavailable".to_string())?;
        let (response_tx, response_rx) = oneshot::channel::<Result<String, String>>();
        let response_tx = Arc::new(Mutex::new(Some(response_tx)));
        let params = params.to_string();

        window
            .with_webview(move |webview| {
                let finish = |result: Result<String, String>| {
                    if let Ok(mut sender) = response_tx.lock() {
                        if let Some(sender) = sender.take() {
                            let _ = sender.send(result);
                        }
                    }
                };
                let controller = webview.controller();
                let core: ICoreWebView2 = match unsafe { controller.CoreWebView2() } {
                    Ok(core) => core,
                    Err(error) => {
                        finish(Err(format!("Failed to access WebView2 core: {error}")));
                        return;
                    }
                };
                let method_wide = CoTaskMemPWSTR::from(method.as_str());
                let params_wide = CoTaskMemPWSTR::from(params.as_str());
                let handler_sender = Arc::clone(&response_tx);
                let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
                    move |error_code, result_json| {
                        if let Ok(mut sender) = handler_sender.lock() {
                            if let Some(sender) = sender.take() {
                                let result = error_code
                                    .map(|_| result_json)
                                    .map_err(|error| format!("WebView2 CDP call failed: {error}"));
                                let _ = sender.send(result);
                            }
                        }
                        Ok(())
                    },
                ));
                let call_result = match session_id.as_deref() {
                    Some(session_id) => match core.cast::<ICoreWebView2_11>() {
                        Ok(core) => {
                            let session_wide = CoTaskMemPWSTR::from(session_id);
                            unsafe {
                                core.CallDevToolsProtocolMethodForSession(
                                    *session_wide.as_ref().as_pcwstr(),
                                    *method_wide.as_ref().as_pcwstr(),
                                    *params_wide.as_ref().as_pcwstr(),
                                    &handler,
                                )
                            }
                        }
                        Err(error) => Err(error),
                    },
                    None => unsafe {
                        core.CallDevToolsProtocolMethod(
                            *method_wide.as_ref().as_pcwstr(),
                            *params_wide.as_ref().as_pcwstr(),
                            &handler,
                        )
                    },
                };
                if let Err(error) = call_result {
                    finish(Err(format!(
                        "Failed to dispatch WebView2 CDP call: {error}"
                    )));
                }
            })
            .map_err(|error| format!("Failed to access the main WebView2: {error}"))?;

        let raw = tokio::time::timeout(timeout, response_rx)
            .await
            .map_err(|_| "WebView2 CDP call timed out".to_string())?
            .map_err(|_| "WebView2 CDP response channel closed".to_string())??;
        serde_json::from_str(&raw)
            .map_err(|error| format!("Invalid WebView2 CDP response: {error}"))
    }

    async fn subscribe_to_events(
        app: &AppHandle,
        event_tx: mpsc::Sender<NativeCdpEvent>,
    ) -> Result<Vec<EventSubscription>, String> {
        use webview2_com::{
            take_pwstr, CoTaskMemPWSTR, DevToolsProtocolEventReceivedEventHandler,
            Microsoft::Web::WebView2::Win32::{
                ICoreWebView2, ICoreWebView2DevToolsProtocolEventReceivedEventArgs2,
            },
        };
        use windows_core::{Interface, PWSTR};

        let window = app
            .get_webview_window(MAIN_WINDOW_LABEL)
            .ok_or_else(|| "The main WebView2 window is unavailable".to_string())?;
        let (result_tx, result_rx) = oneshot::channel();

        window
            .with_webview(move |webview| {
                let result = (|| -> Result<Vec<EventSubscription>, String> {
                    let controller = webview.controller();
                    let core: ICoreWebView2 = unsafe { controller.CoreWebView2() }
                        .map_err(|error| format!("Failed to access WebView2 core: {error}"))?;
                    let mut subscriptions = Vec::new();
                    for event_name in CDP_EVENT_NAMES.iter().map(|name| (*name).to_string()) {
                        let event_wide = CoTaskMemPWSTR::from(event_name.as_str());
                        let receiver = match unsafe {
                            core.GetDevToolsProtocolEventReceiver(*event_wide.as_ref().as_pcwstr())
                        } {
                            Ok(receiver) => receiver,
                            Err(_) => continue,
                        };
                        let callback_name = event_name.clone();
                        let callback_tx = event_tx.clone();
                        let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(
                            move |_sender, args| {
                                let Some(args) = args else { return Ok(()) };
                                let mut raw_params = PWSTR::null();
                                if unsafe { args.ParameterObjectAsJson(&mut raw_params) }.is_err() {
                                    return Ok(());
                                }
                                let raw_params = take_pwstr(raw_params);
                                let params = serde_json::from_str::<Value>(&raw_params)
                                    .unwrap_or_else(|_| json!({}));
                                let session_id = args
                                    .cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()
                                    .ok()
                                    .and_then(|args| {
                                        let mut raw_session = PWSTR::null();
                                        unsafe { args.SessionId(&mut raw_session) }.ok()?;
                                        let session = take_pwstr(raw_session);
                                        (!session.is_empty()).then_some(session)
                                    });
                                let _ = callback_tx.try_send(NativeCdpEvent {
                                    method: callback_name.clone(),
                                    params,
                                    session_id,
                                });
                                Ok(())
                            },
                        ));
                        let mut token = 0i64;
                        if unsafe {
                            receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)
                        }
                        .is_ok()
                        {
                            subscriptions.push(EventSubscription {
                                name: event_name,
                                token,
                            });
                        }
                    }
                    Ok(subscriptions)
                })();
                let _ = result_tx.send(result);
            })
            .map_err(|error| format!("Failed to access the main WebView2: {error}"))?;

        result_rx
            .await
            .map_err(|_| "WebView2 CDP subscription channel closed".to_string())?
    }

    async fn unsubscribe_from_events(app: &AppHandle, subscriptions: Vec<EventSubscription>) {
        use webview2_com::{CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::ICoreWebView2};

        if subscriptions.is_empty() {
            return;
        }
        let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
            return;
        };
        let (done_tx, done_rx) = oneshot::channel();
        if window
            .with_webview(move |webview| {
                if let Ok(core) =
                    unsafe { webview.controller().CoreWebView2() }.map(|core: ICoreWebView2| core)
                {
                    for subscription in subscriptions {
                        let event_wide = CoTaskMemPWSTR::from(subscription.name.as_str());
                        if let Ok(receiver) = unsafe {
                            core.GetDevToolsProtocolEventReceiver(*event_wide.as_ref().as_pcwstr())
                        } {
                            let _ = unsafe {
                                receiver.remove_DevToolsProtocolEventReceived(subscription.token)
                            };
                        }
                    }
                }
                let _ = done_tx.send(());
            })
            .is_ok()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), done_rx).await;
        }
    }

    fn text_response(status: StatusCode, text: &str) -> Response<HttpBody> {
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .header("cache-control", "no-cache")
            .body(Full::new(Bytes::from(text.to_string())))
            .expect("static text response builds")
    }

    fn json_response(status: StatusCode, value: &Value) -> Response<HttpBody> {
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json; charset=utf-8")
            .header("access-control-allow-origin", "*")
            .header("cache-control", "no-cache")
            .body(Full::new(Bytes::from(value.to_string())))
            .expect("static JSON response builds")
    }

    #[cfg(test)]
    mod tests {
        use super::{
            classify_bridge_health, event_messages, host_allowed, version_descriptor, BridgeHealth,
            BrowserConnectionState, ConnectionMode, NativeCdpEvent, CDP_EVENT_NAMES,
        };
        use serde_json::{json, Value};
        use std::collections::HashSet;

        #[test]
        fn accepts_loopback_hosts_only() {
            assert!(host_allowed(Some("127.0.0.1:19222")));
            assert!(host_allowed(Some("localhost:19222")));
            assert!(!host_allowed(Some("example.com:19222")));
            assert!(!host_allowed(None));
        }

        #[test]
        fn includes_page_and_runtime_events_used_by_debug_clients() {
            assert!(CDP_EVENT_NAMES.contains(&"Page.loadEventFired"));
            assert!(CDP_EVENT_NAMES.contains(&"Runtime.consoleAPICalled"));
            assert!(CDP_EVENT_NAMES.contains(&"Target.targetCreated"));
        }

        #[test]
        fn exposes_a_standard_browser_websocket_endpoint() {
            let descriptor = version_descriptor(19222);
            assert_eq!(descriptor["Protocol-Version"], "1.3");
            assert_eq!(
                descriptor["webSocketDebuggerUrl"],
                "ws://127.0.0.1:19222/devtools/browser/locus"
            );
        }

        #[test]
        fn classifies_each_bridge_direction_independently() {
            assert_eq!(classify_bridge_health(false, false), BridgeHealth::Healthy);
            assert_eq!(
                classify_bridge_health(true, false),
                BridgeHealth::RendererToHostStalled
            );
            assert_eq!(
                classify_bridge_health(false, true),
                BridgeHealth::HostToRendererStalled
            );
            assert_eq!(
                classify_bridge_health(true, true),
                BridgeHealth::BidirectionalStalled
            );
        }

        #[test]
        fn creates_unique_browser_sessions_and_detaches_them_independently() {
            let mut state = BrowserConnectionState::default();
            let (auto_session, created) = state.ensure_auto_attach();
            let manual_session = state.attach();

            assert!(created);
            assert_ne!(auto_session, manual_session);
            assert!(state.contains(&auto_session));
            assert!(state.contains(&manual_session));
            assert!(state.detach(&manual_session));
            assert!(state.contains(&auto_session));
            assert!(!state.contains(&manual_session));
        }

        #[test]
        fn fans_native_page_events_out_to_every_synthetic_browser_session() {
            let mut state = BrowserConnectionState::default();
            let first = state.attach();
            let second = state.attach();
            let messages = event_messages(
                NativeCdpEvent {
                    method: "Runtime.consoleAPICalled".to_string(),
                    params: json!({ "type": "log" }),
                    session_id: None,
                },
                ConnectionMode::Browser,
                &state,
            );
            let sessions = messages
                .iter()
                .map(|message| serde_json::from_str::<Value>(message).unwrap())
                .filter_map(|message| message["sessionId"].as_str().map(str::to_string))
                .collect::<HashSet<_>>();

            assert_eq!(sessions, HashSet::from([first, second]));
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use super::FrontendBridgeHeartbeat;
    use serde_json::Value;
    use std::time::Duration;
    use tauri::AppHandle;

    #[derive(Default)]
    pub struct CdpDebugServerHandle;

    impl CdpDebugServerHandle {
        pub fn record_frontend_heartbeat(&self, _heartbeat: FrontendBridgeHeartbeat) {}
    }

    pub async fn reconcile(_app: AppHandle, _enabled: bool) -> Result<Option<u16>, String> {
        Ok(None)
    }

    pub(crate) async fn evaluate_main_webview(
        _app: &AppHandle,
        _expression: &str,
        _timeout: Duration,
    ) -> Result<Value, String> {
        Err("WebView evaluation is only available on Windows".to_string())
    }
}

pub(crate) use imp::evaluate_main_webview;
pub use imp::{reconcile, CdpDebugServerHandle};
