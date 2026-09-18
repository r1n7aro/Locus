use super::*;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn message(id: &str, role: &str, content: &str) -> ChatMessage {
    serde_json::from_value(json!({"id":id,"role":role,"content":content,"createdAt":0})).unwrap()
}

async fn read_http(socket: &mut tokio::net::TcpStream) -> Value {
    let mut reader = BufReader::new(socket);
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap();
        }
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&zstd::stream::decode_all(bytes.as_slice()).unwrap()).unwrap()
}

async fn ws_read(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) -> Value {
    serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}

async fn ws_send(ws: &mut tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>, event: Value) {
    ws.send(Message::Text(event.to_string().into()))
        .await
        .unwrap();
}

#[test]
fn compaction_accepts_extra_output_but_requires_one_checkpoint_and_completion() {
    let item = json!({"type":"compaction","encrypted_content":"opaque"});
    let extras = vec![
        json!({"type":"reasoning"}),
        json!({"type":"message"}),
        item.clone(),
    ];
    assert_eq!(
        validate_remote_compaction_v2_output(&extras, true).unwrap(),
        item
    );
    assert!(validate_remote_compaction_v2_output(&extras, false).is_err());
    assert!(validate_remote_compaction_v2_output(&extras[..2], true).is_err());
    assert!(validate_remote_compaction_v2_output(&[item.clone(), item], true).is_err());
}

#[test]
fn turn_routing_keeps_first_value_until_a_new_turn() {
    let mut state = TurnState::default();
    state.store_header(Some(" "));
    state.store_header(Some("sampling"));
    state.store_header(Some("compaction"));
    state.store_header(None);
    assert_eq!(state.header_value(), Some("sampling"));
    assert_eq!(TurnState::default().header_value(), None);
}

#[test]
fn interrupted_complete_tools_are_not_misread_as_zero_complete_tools() {
    assert!(!should_retry_safe_codex_error("WebSocket ended before the response finalized (text_len=0, complete_tool_calls=1, incomplete_tool_calls=0)"));
}

#[tokio::test]
async fn websocket_compaction_reuses_tools_and_turn_and_replays_checkpoint() {
    for model in ["gpt-6-astra", "gpt-5.6-sol"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let checkpoint = json!({"id":"cmp_one","type":"compaction","encrypted_content":"opaque"});
        let sent_checkpoint = checkpoint.clone();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
                .await
                .unwrap();
            let warm = ws_read(&mut ws).await;
            assert_eq!(warm["generate"], false);
            ws_send(&mut ws, json!({"type":"response.completed","headers":{"x-codex-turn-state":"sampling"},"response":{"id":"warm","output":[]}})).await;
            let first = ws_read(&mut ws).await;
            assert_eq!(first["previous_response_id"], "warm");
            ws_send(&mut ws, json!({"type":"response.completed","response":{"id":"first","output":[
                {"id":"msg_first","type":"message","role":"assistant","content":[{"type":"output_text","text":"Ready"}]}]}})).await;
            let compact = ws_read(&mut ws).await;
            assert_eq!(compact["previous_response_id"], "first");
            assert_eq!(compact["input"], json!([{"type":"compaction_trigger"}]));
            assert_eq!(compact["client_metadata"]["x-codex-turn-state"], "sampling");
            ws_send(&mut ws, json!({"type":"response.completed","headers":{"x-codex-turn-state":"do-not-replace"},"response":{"id":"compact","output":[
                {"type":"reasoning","id":"rs_extra","summary":[]},
                {"type":"function_call","id":"fc_extra","call_id":"extra","name":"probe","arguments":"{"},
                sent_checkpoint]}})).await;
            let continuation = ws_read(&mut ws).await;
            assert!(continuation.get("previous_response_id").is_none());
            assert_eq!(
                continuation["client_metadata"]["x-codex-turn-state"],
                "sampling"
            );
            assert!(continuation["input"]
                .as_array()
                .unwrap()
                .contains(&checkpoint));
            // Both the native search tool and any activated tools keep their original prefix.
            if model == "gpt-6-astra" {
                assert_eq!(continuation["input"][0], warm["input"][0]);
            } else {
                assert_eq!(continuation["tools"], warm["tools"]);
            }
            ws_send(
                &mut ws,
                json!({"type":"response.completed","response":{"id":"after","output":[]}}),
            )
            .await;
        });
        let sid = uuid::Uuid::new_v4().to_string();
        let tools = vec![
            json!({"type":"function","function":{"name":"probe","description":"Activated skill tool","parameters":{"type":"object"}}}),
        ];
        let mut turn = TurnState::default();
        let long_context = "Existing task detail and constraints.\n".repeat(16_000);
        let mut history = vec![message("u", "user", &long_context)];
        let first = stream_chat(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            model,
            "Instructions",
            &history,
            &tools,
            Some("Load tools"),
            Some("low"),
            false,
            false,
            Some(&sid),
            None,
            &mut turn,
            &|_| {},
            &|_| {},
            &|_, _| {},
        )
        .await
        .unwrap();
        let mut assistant = message("a", "assistant", &first.text);
        assistant.response_id = first.response_id;
        history.push(assistant);
        let metadata = HashMap::from([("a".to_string(), first.continuation_request.unwrap())]);
        let result = compact_conversation_history_v2(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            model,
            "Instructions",
            &history,
            CodexCompactionContext {
                tools: &tools,
                tool_search_description: Some("Load tools"),
                turn_state: &mut turn,
                run_id: "run",
                iteration: 2,
                cancel_rx: None,
                capture: CodexCompactionCapture::default(),
            },
            Some("low"),
            false,
            Some(&sid),
            Some(&metadata),
            false,
        )
        .await
        .unwrap();
        let evidence: Value = serde_json::from_str(&result.raw_response).unwrap();
        assert_eq!(evidence["codex_stream_attempts"][0]["run_id"], "run");
        assert!(
            evidence["codex_stream_attempts"][0]["request_bytes"]
                .as_u64()
                .unwrap()
                < 8_192
        );
        let metadata = HashMap::from([("handoff".to_string(), compaction_metadata(&result))]);
        stream_chat(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            model,
            "Instructions",
            &[message("handoff", "user", "## Context Handoff")],
            &tools,
            Some("Load tools"),
            Some("low"),
            false,
            false,
            Some(&sid),
            Some(&metadata),
            &mut turn,
            &|_| {},
            &|_| {},
            &|_, _| {},
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        invalidate_cached_session(&sid);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HttpFailure {
    Headers,
    Idle,
    BodyDisconnect,
}

async fn stalled_http(failure: HttpFailure) {
    let sse = failure != HttpFailure::Headers;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let body = read_http(&mut socket).await;
            assert_eq!(
                body["input"].as_array().unwrap().last().unwrap()["type"],
                "compaction_trigger"
            );
            if sse {
                let event =
                    "data: {\"type\":\"response.created\",\"response\":{\"id\":\"slow\"}}\n\n";
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n", event.len(), event).as_bytes()).await.unwrap();
            }
            if failure == HttpFailure::BodyDisconnect {
                continue;
            }
            let mut byte = [0];
            let _ = socket.read(&mut byte).await;
        }
    });
    let capture = diagnostics::Capture::default();
    let mut options = CodexStreamOptions::remote_compaction_v2();
    options.capture = Some(capture.clone());
    options.idle_timeout = Duration::from_millis(80);
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        stream_chat_with_options(
            "token",
            None,
            CodexTransportMode::Http,
            Some(&base),
            if failure == HttpFailure::Idle {
                "gpt-5.6-sol"
            } else {
                "gpt-6-astra"
            },
            "Test",
            &[message("u", "user", "hello")],
            &[],
            None,
            None,
            false,
            None,
            None,
            &mut TurnState::default(),
            options,
            &|_| {},
            &|_| {},
            &|_, _| {},
        ),
    )
    .await
    .unwrap();
    assert!(result.is_err());
    let (raw_request, evidence) = capture.evidence();
    assert!(raw_request.contains("compaction_trigger"));
    let evidence: Value = serde_json::from_str(&evidence).unwrap();
    let records = evidence["codex_stream_attempts"].as_array().unwrap();
    assert_eq!(records.len(), 3);
    for record in records {
        assert_eq!(
            record["stage"],
            if sse { "read" } else { "response_headers" }
        );
        if sse {
            assert_eq!(record["last_event"], "response.created");
            assert_eq!(record["response_id"], "slow");
            assert!(record["raw_response"]
                .as_str()
                .unwrap()
                .contains("response.created"));
            if failure == HttpFailure::BodyDisconnect {
                assert!(record["error"]
                    .as_str()
                    .unwrap()
                    .contains("Stream read error:"));
            }
        }
    }
    server.await.unwrap();
}

#[tokio::test]
async fn compaction_http_header_stall_has_bounded_retries_and_evidence() {
    stalled_http(HttpFailure::Headers).await;
}

#[tokio::test]
async fn compaction_http_sse_stall_has_bounded_retries_and_evidence() {
    stalled_http(HttpFailure::Idle).await;
}

#[tokio::test]
async fn compaction_http_body_disconnect_preserves_transport_error() {
    stalled_http(HttpFailure::BodyDisconnect).await;
}

#[tokio::test]
async fn astra_compaction_waits_past_request_timeout_on_both_transports() {
    for websocket in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let checkpoint = json!({"type":"compaction","encrypted_content":"finished"});
        let server_checkpoint = checkpoint.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let created = json!({"type":"response.created","response":{"id":"slow-astra"}});
            let added = json!({"type":"response.output_item.added","item":{"type":"compaction","encrypted_content":"pending"}});
            let completed = json!({"type":"response.completed","response":{"id":"slow-astra","output":[server_checkpoint]}});
            if websocket {
                let mut ws =
                    tokio_tungstenite::accept_async_with_config(socket, Some(websocket_config()))
                        .await
                        .unwrap();
                ws_read(&mut ws).await;
                ws_send(&mut ws, created).await;
                ws_send(&mut ws, added).await;
                tokio::time::sleep(Duration::from_millis(600)).await;
                ws_send(&mut ws, completed).await;
            } else {
                read_http(&mut socket).await;
                let initial = format!("data: {created}\n\ndata: {added}\n\n");
                let final_event = format!("data: {completed}\n\n");
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{initial}", initial.len() + final_event.len()).as_bytes()).await.unwrap();
                tokio::time::sleep(Duration::from_millis(600)).await;
                socket.write_all(final_event.as_bytes()).await.unwrap();
            }
        });
        let capture = diagnostics::Capture::default();
        let mut options = CodexStreamOptions::remote_compaction_v2();
        options.capture = Some(capture.clone());
        options.idle_timeout = Duration::from_millis(200);
        let sid = uuid::Uuid::new_v4().to_string();
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            stream_chat_with_options(
                "token",
                None,
                if websocket {
                    CodexTransportMode::Websocket
                } else {
                    CodexTransportMode::Http
                },
                Some(&base),
                "gpt-6-astra",
                "Test",
                &[message("u", "user", "compact")],
                &[],
                None,
                None,
                false,
                Some(&sid),
                None,
                &mut TurnState::default(),
                options,
                &|_| {},
                &|_| {},
                &|_, _| {},
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.response_completed);
        assert_eq!(response.response_items, vec![checkpoint]);
        let evidence: Value = serde_json::from_str(&capture.evidence().1).unwrap();
        assert_eq!(
            evidence["codex_stream_attempts"].as_array().unwrap().len(),
            1
        );
        server.await.unwrap();
        invalidate_cached_session(&sid);
    }
}

#[tokio::test]
async fn cancelled_compaction_preserves_partial_response_without_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
            .await
            .unwrap();
        ws_read(&mut ws).await;
        ws_send(
            &mut ws,
            json!({"type":"response.created","response":{"id":"cancel-me"}}),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_tx.send(true).unwrap();
        let _ = ws.next().await;
    });
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        compact_conversation_history_v2(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            "gpt-6-astra",
            "Test",
            &[message("u", "user", "hello")],
            CodexCompactionContext {
                tools: &[],
                tool_search_description: None,
                turn_state: &mut TurnState::default(),
                run_id: "cancel-run",
                iteration: 3,
                cancel_rx: Some(cancel_rx),
                capture: CodexCompactionCapture::default(),
            },
            None,
            false,
            None,
            None,
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(result.message.contains("cancelled"));
    let evidence: Value = serde_json::from_str(&result.raw_response).unwrap();
    assert_eq!(
        evidence["codex_stream_attempts"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        evidence["codex_stream_attempts"][0]["response_id"],
        "cancel-me"
    );
    assert!(result.raw_request.contains("compaction_trigger"));
    server.await.unwrap();
}

#[tokio::test]
async fn interrupted_compaction_preserves_each_attempt_through_http_fallback() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for attempt in 0..3 {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
                .await
                .unwrap();
            let body = ws_read(&mut ws).await;
            assert!(body.get("previous_response_id").is_none());
            ws_send(&mut ws, json!({"type":"response.created","response":{"id":format!("failed-{attempt}")},"headers":{"x-codex-turn-state":"first-route"}})).await;
            ws.send(Message::Close(None)).await.unwrap();
        }
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_http(&mut socket).await;
        assert_eq!(
            request["input"].as_array().unwrap().last().unwrap()["type"],
            "compaction_trigger"
        );
        let event = format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{"id":"http-success","output":[{"type":"compaction","encrypted_content":"opaque"}]}})
        );
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",event.len(),event).as_bytes()).await.unwrap();
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        compact_conversation_history_v2(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            "gpt-6-astra",
            "Test",
            &[message("u", "user", "hello")],
            CodexCompactionContext {
                tools: &[],
                tool_search_description: None,
                turn_state: &mut TurnState::default(),
                run_id: "retry-run",
                iteration: 3,
                cancel_rx: None,
                capture: CodexCompactionCapture::default(),
            },
            None,
            false,
            Some(&sid),
            None,
            false,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let evidence: Value = serde_json::from_str(&result.raw_response).unwrap();
    let attempts = evidence["codex_stream_attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 4);
    for (index, record) in attempts[..3].iter().enumerate() {
        assert_eq!(record["response_id"], format!("failed-{index}"));
        assert_eq!(record["transport"], "websocket");
        assert!(record["error"]
            .as_str()
            .unwrap()
            .contains("compaction stream ended"));
        assert_eq!(record["last_event"], "response.created");
        assert!(record["raw_request"]
            .as_str()
            .unwrap()
            .contains("compaction_trigger"));
    }
    assert_eq!(attempts[3]["transport"], "http");
    assert_eq!(attempts[3]["response_id"], "http-success");
    assert_eq!(attempts[3]["error"], Value::Null);
    assert!(cached_websocket_http_fallback_enabled(Some(&sid), Some(&base), None).await);
    server.await.unwrap();
    invalidate_cached_session(&sid);
}

#[tokio::test]
async fn websocket_request_send_is_bounded_when_the_peer_stops_reading() {
    let sid = uuid::Uuid::new_v4().to_string();
    let base = "http://127.0.0.1:1";
    let (client, _peer) = tokio::io::duplex(1);
    let socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
        Box::new(client) as BoxedCodexIo,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;
    let shared = cached_websocket_session(&sid);
    {
        let mut cache = shared.lock().await;
        cache.connection_key = Some(websocket_connection_key(Some(base), None));
        cache.connection = Some(CodexWebsocketStream::new(socket));
    }
    let options = CodexStreamOptions::remote_compaction_v2();
    let mut trace =
        diagnostics::Attempt::new(Some(&sid), &options, 1, CodexTransportMode::Websocket);
    let history = vec![message("u", "user", "compact")];
    let body = build_request_body(
        "gpt-6-astra",
        "Test",
        &history,
        &[],
        None,
        Some(&sid),
        None,
        options,
    );
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        stream_chat_websocket_once(
            "token",
            None,
            Some(base),
            "gpt-6-astra",
            &history,
            &[],
            Some(&sid),
            Some(&sid),
            false,
            body,
            &mut TurnState::default(),
            None,
            &|_| {},
            &|_| {},
            &|_, _| {},
            Duration::from_millis(40),
            ASTRA_COMPACTION_IDLE_TIMEOUT,
            &mut trace,
        ),
    )
    .await
    .unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("blocked send must time out"),
    };
    assert!(error.contains("WebSocket send timed out"), "{error}");
    let cache = shared.lock().await;
    assert!(cache.connection.is_none());
    assert!(cache.last_response.is_none());
    drop(cache);
    invalidate_cached_session(&sid);
}

#[tokio::test]
async fn compaction_capture_survives_an_auth_retry() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for unauthorized in [true, false] {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_http(&mut socket).await;
            let (status, response) = if unauthorized {
                ("401 Unauthorized", "expired token".to_string())
            } else {
                (
                    "200 OK",
                    format!(
                        "data: {}\n\n",
                        json!({"type":"response.completed","response":{"id":"after-auth","output":[{"type":"compaction","encrypted_content":"opaque"}]}})
                    ),
                )
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        }
    });
    let capture = CodexCompactionCapture::default();
    let mut turn = TurnState::default();
    for retry in [false, true] {
        let result = compact_conversation_history_v2(
            "test-token",
            None,
            CodexTransportMode::Http,
            Some(&base),
            "gpt-6-astra",
            "Test",
            &[message("u", "user", "compact")],
            CodexCompactionContext {
                tools: &[],
                tool_search_description: None,
                turn_state: &mut turn,
                run_id: "auth-run",
                iteration: 1,
                cancel_rx: None,
                capture: capture.clone(),
            },
            None,
            false,
            None,
            None,
            false,
        )
        .await;
        if retry {
            let response = result.unwrap();
            let evidence: Value = serde_json::from_str(&response.raw_response).unwrap();
            let records = evidence["codex_stream_attempts"].as_array().unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(records[0]["raw_response"], "expired token");
            assert_eq!(records[1]["attempt"], 2);
        } else {
            assert!(result.unwrap_err().message.contains("401"));
        }
    }
    server.await.unwrap();
}
