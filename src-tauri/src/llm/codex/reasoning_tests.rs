use super::*;

fn message(id: &str, role: &str, text: &str) -> ChatMessage {
    serde_json::from_value(json!({"id":id,"role":role,"content":text,"createdAt":0})).unwrap()
}

fn body(level: &str, history: &[ChatMessage], metadata: &HashMap<String, Value>) -> Value {
    build_request_body_with_tool_search(
        "gpt-6-astra",
        "Stable instructions",
        history,
        &[],
        None,
        Some(level),
        Some("reasoning-test"),
        Some(metadata),
        CodexStreamOptions::default(),
    )
}

fn complete(
    body: &Value,
    history: &mut Vec<ChatMessage>,
    metadata: &mut HashMap<String, Value>,
    id: &str,
) -> LastWebsocketResponse {
    let text = format!("answer {id}");
    let output = vec![
        json!({"type":"reasoning","id":format!("rs_{id}"),"summary":[],"encrypted_content":"opaque"}),
        json!({"type":"message","id":format!("msg_{id}"),"role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":text}]}),
    ];
    metadata.insert(
        id.to_string(),
        protocol::response_metadata(continuation_metadata(body), &output, &text, &[], &json!({})),
    );
    let mut assistant = message(id, "assistant", &text);
    assistant.response_id = Some(format!("resp_{id}"));
    history.push(assistant);
    LastWebsocketResponse {
        request_signature: websocket_request_signature(body),
        input: body["input"].as_array().unwrap().clone(),
        response_id: format!("resp_{id}"),
        items_added: output,
    }
}

fn updates(body: &Value) -> Vec<&Value> {
    body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "configuration_update")
        .collect()
}

#[test]
fn effort_switches_preserve_wire_prefix_signature_and_websocket_delta() {
    let mut history = vec![message("u1", "user", "start")];
    let mut metadata = HashMap::new();
    let first = body("low", &history, &metadata);
    assert!(updates(&first).is_empty());
    let mut last = complete(&first, &mut history, &mut metadata, "a1");
    let mut expected_updates = 0;
    let mut effective = "low";
    for (index, level) in ["high", "high", "max", "low"].into_iter().enumerate() {
        history.push(message(&format!("u{}", index + 2), "user", "follow up"));
        let next = body(level, &history, &metadata);
        assert_eq!(next["reasoning"]["effort"], "low");
        assert_eq!(
            websocket_request_signature(&next),
            websocket_request_signature(&first)
        );
        let mut prefix = last.input.clone();
        prefix.extend(last.items_added.clone());
        assert!(next["input"].as_array().unwrap().starts_with(&prefix));
        let delta = build_websocket_transport_request(&next, Some(&last), true);
        assert_eq!(delta["previous_response_id"], last.response_id);
        assert!(delta.get(METADATA_KEY).is_none());
        assert_eq!(
            delta["input"].as_array().unwrap().len(),
            if level == effective { 1 } else { 2 }
        );
        if level != effective {
            expected_updates += 1;
            assert_eq!(delta["input"][0], update(level));
            assert_eq!(delta["input"][1]["role"], "user");
        }
        assert_eq!(updates(&next).len(), expected_updates);
        let replay =
            build_history_transport_request(&next, &history, Some(&metadata), false, false);
        assert_eq!(replay["input"], next["input"]);
        assert!(replay.get("previous_response_id").is_none());
        assert!(replay.get(METADATA_KEY).is_none());
        assert_eq!(
            next,
            body(level, &history, &metadata),
            "retry is deterministic"
        );
        last = complete(
            &next,
            &mut history,
            &mut metadata,
            &format!("a{}", index + 2),
        );
        effective = level;
    }
}

#[test]
fn replay_survives_serialization_and_forked_message_ids() {
    let mut history = vec![message("u1", "user", "start")];
    let mut metadata = HashMap::new();
    let first = body("low", &history, &metadata);
    complete(&first, &mut history, &mut metadata, "a1");
    history.push(message("u2", "user", "harder"));
    let next = body("max", &history, &metadata);
    complete(&next, &mut history, &mut metadata, "a2");
    history.push(message("u3", "user", "continue"));
    let expected = body("max", &history, &metadata);
    let metadata: HashMap<String, Value> =
        serde_json::from_str(&serde_json::to_string(&metadata).unwrap()).unwrap();
    let mut fork_metadata = HashMap::new();
    for message in &mut history {
        let id = format!("fork_{}", message.id);
        if let Some(value) = metadata.get(&message.id) {
            fork_metadata.insert(id.clone(), value.clone());
        }
        message.id = id;
    }
    assert_eq!(expected, body("max", &history, &fork_metadata));
    assert_eq!(updates(&expected).len(), 1);
}

#[test]
fn tool_results_keep_the_updated_effort_without_moving_or_duplicating_updates() {
    let mut history = vec![message("u1", "user", "start")];
    let mut metadata = HashMap::new();
    let first = body("low", &history, &metadata);
    complete(&first, &mut history, &mut metadata, "a1");
    history.push(message("u2", "user", "use tools"));
    let next = body("high", &history, &metadata);
    let mut assistant = message("a2", "assistant", "");
    assistant.tool_calls = Some(vec![serde_json::from_value(
        json!({"id":"call_one","name":"probe","arguments":"{}"}),
    )
    .unwrap()]);
    let output =
        vec![json!({"type":"function_call","call_id":"call_one","name":"probe","arguments":"{}"})];
    metadata.insert(
        "a2".to_string(),
        protocol::response_metadata(
            continuation_metadata(&next),
            &output,
            "",
            assistant.tool_calls.as_ref().unwrap(),
            &json!({}),
        ),
    );
    history.push(assistant);
    let mut result = message("t1", "tool", "42");
    result.tool_call_id = Some("call_one".to_string());
    history.push(result);
    let continuation = body("high", &history, &metadata);
    assert_eq!(continuation["reasoning"]["effort"], "low");
    assert!(continuation["input"]
        .as_array()
        .unwrap()
        .starts_with(next["input"].as_array().unwrap()));
    assert_eq!(updates(&continuation).len(), 1);
    assert_eq!(
        continuation["input"].as_array().unwrap().last().unwrap()["type"],
        "function_call_output"
    );
}

#[test]
fn compaction_retains_baseline_and_reasserts_effort_once_after_the_new_window() {
    let mut history = vec![message("u1", "user", "start")];
    let mut metadata = HashMap::new();
    let first = body("low", &history, &metadata);
    complete(&first, &mut history, &mut metadata, "a1");
    history.push(message("u2", "user", "harder"));
    let next = body("high", &history, &metadata);
    complete(&next, &mut history, &mut metadata, "a2");
    let compact = build_request_body(
        "gpt-6-astra",
        "Stable instructions",
        &history,
        &[],
        Some("high"),
        Some("reasoning-test"),
        Some(&metadata),
        CodexStreamOptions::remote_compaction_v2(),
    );
    assert_eq!(compact["reasoning"]["effort"], "low");
    assert_eq!(updates(&compact).len(), 1);
    assert_eq!(
        compact["input"].as_array().unwrap().last().unwrap()["type"],
        "compaction_trigger"
    );
    let wire = build_websocket_transport_request(&compact, None, true);
    let outcome = CodexRemoteCompactOutcome {
        output: retained_remote_compaction_v2_window(
            &history,
            Some(&metadata),
            json!({"type":"compaction","encrypted_content":"compact"}),
        ),
        encrypted_content: Some("compact".to_string()),
        raw_request: wire.to_string(),
        raw_response: String::new(),
    };
    assert!(!outcome
        .output
        .iter()
        .any(|item| item["type"] == "configuration_update"));
    let mut history = vec![
        message("handoff", "user", "local fallback"),
        message("u3", "user", "continue"),
    ];
    let mut metadata = HashMap::from([("handoff".to_string(), compaction_metadata(&outcome))]);
    let next = body("high", &history, &metadata);
    assert_eq!(next["reasoning"]["effort"], "low");
    let input = next["input"].as_array().unwrap();
    let index = input
        .iter()
        .position(|item| item["type"] == "compaction")
        .unwrap();
    assert_eq!(input[index + 1], update("high"));
    assert_eq!(input[index + 2]["role"], "user");
    complete(&next, &mut history, &mut metadata, "a3");
    history.push(message("u4", "user", "continue again"));
    let replay = body("high", &history, &metadata);
    assert_eq!(updates(&replay).len(), 1);
    assert!(replay["input"].as_array().unwrap().starts_with(input));
}

#[test]
fn legacy_effort_is_the_baseline_and_other_models_do_not_receive_updates() {
    let history = vec![
        message("u1", "user", "start"),
        message("a1", "assistant", "answer"),
        message("u2", "user", "next"),
    ];
    let metadata = HashMap::from([(
        "a1".to_string(),
        json!({"model":"gpt-6-astra", "reasoning":{"effort":"high"},
        "codex_reasoning":null, "codex_response":null}),
    )]);
    let next = body("low", &history, &metadata);
    assert_eq!(next["reasoning"]["effort"], "high");
    assert_eq!(updates(&next), vec![&update("low")]);
    let other = build_request_body(
        "gpt-5.6-sol",
        "Stable instructions",
        &history,
        &[],
        Some("max"),
        Some("reasoning-test"),
        Some(&metadata),
        CodexStreamOptions::default(),
    );
    assert_eq!(other["reasoning"]["effort"], "max");
    assert!(updates(&other).is_empty());
    assert!(other.get(METADATA_KEY).is_none());
}

#[test]
fn empty_responses_do_not_create_adjacent_configuration_updates() {
    let history = vec![message("u", "user", "start"), message("a", "assistant", "")];
    let metadata = HashMap::from([(
        "a".to_string(),
        json!({"model":"gpt-6-astra","reasoning":{"effort":"low"},
        METADATA_KEY:{"version":1,"effort":"high","update_trailing_items":0}}),
    )]);
    let next = body("max", &history, &metadata);
    assert_eq!(updates(&next), vec![&update("max")]);
    let mut history = history;
    let mut metadata = metadata;
    metadata.insert("b".to_string(), continuation_metadata(&next));
    history.push(message("b", "assistant", ""));
    assert_eq!(
        updates(&body("max", &history, &metadata)),
        vec![&update("max")]
    );
    assert!(!supports_reasoning_updates("gpt-6-astra-pro"));
    assert!(!supports_reasoning_updates("custom/openai/gpt-6-astra"));
}

#[test]
fn automatic_successor_inherits_effort_without_replaying_the_parents_update() {
    let mut history = vec![message("u1", "user", "start")];
    let mut metadata = HashMap::new();
    let first = body("low", &history, &metadata);
    complete(&first, &mut history, &mut metadata, "a1");
    history.push(message("u2", "user", "harder"));
    let next = body("high", &history, &metadata);
    complete(&next, &mut history, &mut metadata, "a2");
    let inherited = inherited_metadata(metadata["a2"].clone());
    assert_eq!(inherited[METADATA_KEY]["effort"], "high");
    history.push(message("u3", "user", "steered input"));
    history.push(message("a3", "assistant", "successor"));
    metadata.insert("a3".to_string(), inherited);
    history.push(message("u4", "user", "next turn"));
    let replay = body("high", &history, &metadata);
    assert_eq!(updates(&replay).len(), 1);
    assert_eq!(replay["reasoning"]["effort"], "low");
}

fn server_output(text: &str) -> Vec<Value> {
    vec![
        json!({"type":"message","id":format!("msg_{text}"),"role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":text}]}),
    ]
}

fn completed_event(text: &str) -> Value {
    json!({"type":"response.completed","response":{"id":format!("resp_{text}"),"output":server_output(text)}})
}

async fn call_mock(
    base: &str,
    sid: &str,
    transport: CodexTransportMode,
    level: &str,
    history: &[ChatMessage],
    metadata: &HashMap<String, Value>,
) -> LlmResponse {
    stream_chat(
        "test-token",
        None,
        transport,
        Some(base),
        "gpt-6-astra",
        "Stable instructions",
        history,
        &[],
        None,
        Some(level),
        false,
        false,
        Some(sid),
        Some(metadata),
        &mut TurnState::default(),
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .await
    .unwrap()
}

fn save_response(
    response: LlmResponse,
    history: &mut Vec<ChatMessage>,
    metadata: &mut HashMap<String, Value>,
    id: &str,
) {
    let mut assistant = message(id, "assistant", &response.text);
    assistant.response_id = response.response_id;
    metadata.insert(id.to_string(), response.continuation_request.unwrap());
    history.push(assistant);
}

#[tokio::test]
async fn websocket_native_effort_switch_recovers_with_identical_full_history() {
    tokio::time::timeout(Duration::from_secs(25), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config())).await.unwrap();
            let warm: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(warm["generate"], false);
            assert!(warm.get(METADATA_KEY).is_none());
            let mut expected_prefix = warm["input"].as_array().unwrap().clone();
            ws.send(Message::Text(json!({"type":"response.completed","response":{"id":"resp_warm","output":[]}}).to_string().into())).await.unwrap();
            let first: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(first["reasoning"]["effort"], "low");
            ws.send(Message::Text(completed_event("one").to_string().into())).await.unwrap();
            expected_prefix.extend(server_output("one"));
            let second: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(second["previous_response_id"], "resp_one");
            assert_eq!(second["reasoning"]["effort"], "low");
            assert_eq!(second["input"][0], update("high"));
            assert_eq!(second["input"].as_array().unwrap().len(), 2);
            assert!(second.get(METADATA_KEY).is_none());
            expected_prefix.extend(second["input"].as_array().unwrap().clone());
            ws.send(Message::Text(json!({"type":"error","error":{"code":"previous_response_not_found","message":"missing"}}).to_string().into())).await.unwrap();
            drop(ws);
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config())).await.unwrap();
            let replay: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert!(replay.get("previous_response_id").is_none());
            assert_eq!(replay["input"], json!(expected_prefix));
            assert_eq!(replay["reasoning"]["effort"], "low");
            ws.send(Message::Text(completed_event("two").to_string().into())).await.unwrap();
            let third: Value = serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(third["previous_response_id"], "resp_two");
            assert_eq!(third["input"][0], update("low"));
            assert_eq!(third["input"].as_array().unwrap().len(), 2);
            ws.send(Message::Text(completed_event("three").to_string().into())).await.unwrap();
        });
        let sid = uuid::Uuid::new_v4().to_string();
        let mut history = vec![];
        let mut metadata = HashMap::new();
        for (index, level) in ["low", "high", "low"].into_iter().enumerate() {
            history.push(message(&format!("u{index}"), "user", "follow up"));
            let response = call_mock(&base, &sid, CodexTransportMode::Websocket, level, &history, &metadata).await;
            assert_eq!(response.continuation_request.as_ref().unwrap()[METADATA_KEY]["effort"], level);
            save_response(response, &mut history, &mut metadata, &format!("a{index}"));
        }
        server.await.unwrap();
        invalidate_cached_session(&sid);
    }).await.expect("WebSocket reasoning test timed out");
}

#[tokio::test]
async fn http_native_effort_switch_preserves_zstd_lite_prefix_and_replay() {
    use http_body_util::{BodyExt, Full};
    tokio::time::timeout(Duration::from_secs(15), async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let prefix = Arc::new(StdMutex::new(Vec::<Value>::new()));
            for index in 0..2 {
                let (tcp, _) = listener.accept().await.unwrap();
                let prefix = prefix.clone();
                let service = hyper::service::service_fn(
                    move |request: hyper::Request<hyper::body::Incoming>| {
                        let prefix = prefix.clone();
                        async move {
                            assert_eq!(request.headers()["content-encoding"], "zstd");
                            assert_eq!(request.headers()[protocol::LITE_HEADER], "true");
                            let bytes = request.into_body().collect().await.unwrap().to_bytes();
                            let plain = zstd::stream::decode_all(bytes.as_ref()).unwrap();
                            let body: Value = serde_json::from_slice(&plain).unwrap();
                            assert!(body.get(METADATA_KEY).is_none());
                            assert!(body.get("previous_response_id").is_none());
                            assert_eq!(body["reasoning"]["effort"], "low");
                            let mut prefix = prefix.lock().unwrap();
                            if index == 0 {
                                *prefix = body["input"].as_array().unwrap().clone();
                                prefix.extend(server_output("one"));
                            } else {
                                let input = body["input"].as_array().unwrap();
                                assert!(input.starts_with(&prefix));
                                assert_eq!(input[prefix.len()], update("max"));
                                assert_eq!(input[prefix.len() + 1]["role"], "user");
                            }
                            Ok::<_, std::convert::Infallible>(
                                hyper::Response::builder()
                                    .header("content-type", "text/event-stream")
                                    .header("connection", "close")
                                    .body(Full::new(hyper::body::Bytes::from(format!(
                                        "data: {}\n\n",
                                        completed_event(if index == 0 { "one" } else { "two" })
                                    ))))
                                    .unwrap(),
                            )
                        }
                    },
                );
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(tcp), service)
                    .await
                    .unwrap();
            }
        });
        let sid = uuid::Uuid::new_v4().to_string();
        let mut history = vec![];
        let mut metadata = HashMap::new();
        for (index, level) in ["low", "max"].into_iter().enumerate() {
            history.push(message(&format!("u{index}"), "user", "follow up"));
            let response = call_mock(
                &base,
                &sid,
                CodexTransportMode::Http,
                level,
                &history,
                &metadata,
            )
            .await;
            save_response(response, &mut history, &mut metadata, &format!("a{index}"));
        }
        server.await.unwrap();
    })
    .await
    .expect("HTTP reasoning test timed out");
}
