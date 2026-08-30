#[cfg(target_os = "windows")]
mod imp {
    use std::convert::Infallible;
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
    const MAIN_TARGET_SESSION_ID: &str = "locus-main-session";
    const DEBUG_PORT_START: u16 = 19_222;
    const DEBUG_PORT_ATTEMPTS: u16 = 25;
    const CDP_CALL_TIMEOUT: Duration = Duration::from_secs(30);
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

    #[derive(Default)]
    pub struct CdpDebugServerHandle {
        inner: tokio::sync::Mutex<RunningState>,
    }

    #[derive(Default)]
    struct RunningState {
        task: Option<JoinHandle<()>>,
        connection_tasks: Option<Arc<Mutex<Vec<JoinHandle<()>>>>>,
        shutdown: Option<watch::Sender<bool>>,
        port: Option<u16>,
    }

    #[derive(Debug, Clone)]
    struct EventSubscription {
        name: String,
        token: i64,
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
            stop_locked(&mut running).await;
            return Ok(None);
        }
        if running.task.is_some() {
            return Ok(running.port);
        }

        let (listener, port) = bind_listener().await?;
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let connection_tasks = Arc::new(Mutex::new(Vec::new()));
        let server_app = app.clone();
        let server_connections = Arc::clone(&connection_tasks);
        let task = tokio::spawn(async move {
            serve(listener, port, server_app, shutdown_rx, server_connections).await;
        });

        running.task = Some(task);
        running.connection_tasks = Some(connection_tasks);
        running.shutdown = Some(shutdown_tx);
        running.port = Some(port);
        eprintln!("[CdpDebug] listening on http://127.0.0.1:{port}");
        Ok(Some(port))
    }

    async fn stop_locked(running: &mut RunningState) {
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
        if let Some(port) = running.port.take() {
            eprintln!("[CdpDebug] stopped listening on 127.0.0.1:{port}");
        }
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
        let default_session_id =
            (mode == ConnectionMode::Browser).then_some(MAIN_TARGET_SESSION_ID.to_string());
        let subscriptions = match subscribe_to_events(&app, event_tx, default_session_id).await {
            Ok(subscriptions) => subscriptions,
            Err(error) => {
                eprintln!("[CdpDebug] failed to subscribe to CDP events: {error}");
                Vec::new()
            }
        };
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
                    if outgoing.send(Message::text(event)).await.is_err() {
                        break;
                    }
                }
                message = incoming.next() => {
                    match message {
                        Some(Ok(Message::Text(text))) => {
                            for response in dispatch_messages(&app, text.as_ref(), mode).await {
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

    async fn dispatch_messages(app: &AppHandle, raw: &str, mode: ConnectionMode) -> Vec<String> {
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
            if let Some(messages) =
                synthetic_browser_messages(app, &id, &method, &params, session_id.as_deref())
            {
                return messages
                    .into_iter()
                    .map(|message| message.to_string())
                    .collect();
            }
        }

        let call_session_id = match (mode, session_id.as_deref()) {
            (ConnectionMode::Browser, Some(MAIN_TARGET_SESSION_ID)) => None,
            _ => session_id.clone(),
        };

        // WebView2 exposes one in-process page session. Re-arm Runtime when a
        // browser client attaches so it receives the existing execution
        // contexts just like a fresh native CDP target session would.
        if mode == ConnectionMode::Browser
            && session_id.as_deref() == Some(MAIN_TARGET_SESSION_ID)
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
    ) -> Option<Vec<Value>> {
        let with_session = |mut response: Value| {
            if let Some(session_id) = session_id {
                response["sessionId"] = Value::String(session_id.to_string());
            }
            response
        };
        let response = |result: Value| with_session(json!({ "id": id, "result": result }));
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
                "targetInfos": [main_target_info(app, true)]
            }))]),
            "Target.getTargetInfo" => Some(vec![response(json!({
                "targetInfo": main_target_info(app, true)
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
                        "params": { "targetInfo": main_target_info(app, false) }
                    }));
                }
                messages.push(response(json!({})));
                Some(messages)
            }
            "Target.setAutoAttach" => {
                let mut messages = Vec::new();
                if session_id.is_none()
                    && params
                        .get("autoAttach")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                {
                    messages.push(json!({
                        "method": "Target.attachedToTarget",
                        "params": {
                            "sessionId": MAIN_TARGET_SESSION_ID,
                            "targetInfo": main_target_info(app, true),
                            "waitingForDebugger": false,
                        }
                    }));
                }
                messages.push(response(json!({})));
                Some(messages)
            }
            "Target.attachToTarget" => Some(vec![
                json!({
                    "method": "Target.attachedToTarget",
                    "params": {
                        "sessionId": MAIN_TARGET_SESSION_ID,
                        "targetInfo": main_target_info(app, true),
                        "waitingForDebugger": false,
                    }
                }),
                response(json!({ "sessionId": MAIN_TARGET_SESSION_ID })),
            ]),
            "Target.detachFromTarget" => Some(vec![response(json!({}))]),
            _ => None,
        }
    }

    async fn call_devtools_method(
        app: &AppHandle,
        session_id: Option<String>,
        method: String,
        params: Value,
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

        let raw = tokio::time::timeout(CDP_CALL_TIMEOUT, response_rx)
            .await
            .map_err(|_| "WebView2 CDP call timed out".to_string())?
            .map_err(|_| "WebView2 CDP response channel closed".to_string())??;
        serde_json::from_str(&raw)
            .map_err(|error| format!("Invalid WebView2 CDP response: {error}"))
    }

    async fn subscribe_to_events(
        app: &AppHandle,
        event_tx: mpsc::Sender<String>,
        default_session_id: Option<String>,
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
                        let callback_default_session_id = default_session_id.clone();
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
                                let mut event = json!({
                                    "method": callback_name,
                                    "params": params,
                                });
                                if let Some(session_id) = session_id {
                                    event["sessionId"] = Value::String(session_id);
                                } else if !callback_name.starts_with("Target.") {
                                    if let Some(session_id) = callback_default_session_id.as_ref() {
                                        event["sessionId"] = Value::String(session_id.clone());
                                    }
                                }
                                let _ = callback_tx.try_send(event.to_string());
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
        use super::{host_allowed, version_descriptor, CDP_EVENT_NAMES};

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
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    use tauri::AppHandle;

    #[derive(Default)]
    pub struct CdpDebugServerHandle;

    pub async fn reconcile(_app: AppHandle, _enabled: bool) -> Result<Option<u16>, String> {
        Ok(None)
    }
}

pub use imp::{reconcile, CdpDebugServerHandle};
