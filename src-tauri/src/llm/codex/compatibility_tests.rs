use super::*;
use serde_json::{json, Value};

fn message(id: &str, role: &str, text: &str) -> ChatMessage {
    serde_json::from_value(json!({"id":id,"role":role,"content":text,"createdAt":0})).unwrap()
}
fn tools() -> Vec<Value> {
    vec![
        json!({"type":"function","function":{"name":"probe","description":"Return a test value",
        "parameters":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}}}),
    ]
}
fn output() -> Vec<Value> {
    vec![
        json!({"id":"rs_one","type":"reasoning","summary":[],"encrypted_content":"opaque-reasoning"}),
        json!({"id":"fc_one","type":"function_call","call_id":"call_one","namespace":"functions",
            "name":"probe","arguments":"{\"value\":42}","status":"completed"}),
    ]
}
fn parse(event: Value, state: &mut CodexStreamState) -> Result<bool, String> {
    process_sse_event_block(
        &format!("data:{}", event),
        false,
        state,
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
}
fn astra_body(history: &[ChatMessage], metadata: Option<&HashMap<String, Value>>) -> Value {
    build_request_body_with_tool_search(
        "gpt-6-astra",
        "Test instructions",
        history,
        &tools(),
        Some("Load tools by wire name"),
        Some("low"),
        Some("test-session"),
        metadata,
        CodexStreamOptions::default(),
    )
}

#[test]
fn lite_tools_instructions_and_identity_match_release_contract() {
    let body = astra_body(&[message("u", "user", "hello")], None);
    assert!(body.get("tools").is_none());
    assert!(body.get("instructions").is_none());
    assert_eq!(body["input"][0]["type"], "additional_tools");
    assert_eq!(body["input"][0]["tools"][0]["name"], "functions");
    assert_eq!(body["input"][0]["tools"][0]["tools"][0]["name"], "probe");
    assert_eq!(body["input"][0]["tools"][2]["type"], "tool_search");
    assert_eq!(body["input"][1]["role"], "developer");
    assert_eq!(body["reasoning"]["context"], "all_turns");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert!(protocol::is_lite_request(&body));
    let retry = astra_body(&[message("u", "user", "different user input")], None);
    assert_eq!(body["input"][0]["id"], retry["input"][0]["id"]);
    assert_eq!(body["input"][1]["id"], retry["input"][1]["id"]);
    let compact = build_compact_request_body(
        "gpt-6-astra",
        "Test instructions",
        &[],
        &tools(),
        Some("low"),
        false,
        Some("test-session"),
        None,
    );
    assert!(protocol::is_lite_request(&compact));
    assert!(compact.get("tools").is_none());
    assert!(compact.get("stream").is_none());
}

#[test]
fn lite_tool_discovery_output_and_images_keep_wire_shapes() {
    let mut body = json!({"model":"gpt-6-astra", "tools":[],"input":[
        {"type":"tool_search_output","call_id":"s1","tools":[{"type":"function","name":"probe","parameters":{}}]},
        {"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,test","detail":"original"}]},
        {"type":"function_call_output","call_id":"c1","output":[{"type":"input_image","image_url":"test","detail":"high"}]}]});
    protocol::apply_lite(&mut body, Some("session"));
    assert_eq!(body["input"][1]["tools"][0]["type"], "namespace");
    assert_eq!(body["input"][1]["tools"][0]["tools"][0]["name"], "probe");
    assert!(body["input"][2]["content"][0].get("detail").is_none());
    assert!(body["input"][3]["output"][0].get("detail").is_none());
    assert_eq!(
        protocol::local_function_name(&json!({"namespace":"functions","name":"probe"})),
        "probe"
    );
    assert_eq!(
        protocol::local_function_name(&json!({"namespace":"other","name":"probe"})),
        "other.probe"
    );
}

#[test]
fn terminal_output_recovers_tools_and_preserves_opaque_items() {
    let mut state = CodexStreamState::new();
    parse(
        json!({"type":"response.created","response":{"id":"resp_one"}}),
        &mut state,
    )
    .unwrap();
    parse(
        json!({"type":"response.metadata","headers":{"openai-model":"gpt-6-astra"}}),
        &mut state,
    )
    .unwrap();
    parse(
        json!({"type":"response.completed","response":{"id":"resp_one","output":output(),
        "usage":{"input_tokens":20,"output_tokens":3,"input_tokens_details":{"cached_tokens":5}}}}),
        &mut state,
    )
    .unwrap();
    let (calls, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
    assert_eq!(incomplete, 0);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_call.name, "probe");
    assert_eq!(state.items_added, output());
    assert_eq!(state.input_tokens, 15);
    assert_eq!(
        state.metadata_events["response.metadata"]["headers"]["openai-model"],
        "gpt-6-astra"
    );
}

#[test]
fn failures_and_incomplete_responses_terminate_before_tool_execution() {
    for event in [
        json!({"type":"response.failed","response":{"error":{"code":"context_length_exceeded","message":"too much context"}}}),
        json!({"type":"response.incomplete","response":{"incomplete_details":{"reason":"max_output_tokens"}}}),
        json!({"type":"response.cancelled"}),
        json!({"type":"error","error":{"code":"invalid_prompt","message":"invalid input"}}),
    ] {
        let mut state = CodexStreamState::new();
        parse(
            json!({"type":"response.output_item.done","item":output()[1]}),
            &mut state,
        )
        .unwrap();
        let error = parse(event.clone(), &mut state).unwrap_err();
        assert!(!state.got_completed_event);
        assert!(error.contains("OpenAI Codex"));
        if event["type"] == "response.failed" {
            assert!(error.contains("context_length_exceeded"));
        }
    }
}

#[test]
fn text_done_recovery_does_not_repeat_deltas() {
    let mut state = CodexStreamState::new();
    parse(
        json!({"type":"response.output_text.delta","item_id":"msg_one","delta":"hel"}),
        &mut state,
    )
    .unwrap();
    parse(
        json!({"type":"response.output_text.done","item_id":"msg_one","text":"hello"}),
        &mut state,
    )
    .unwrap();
    parse(
        json!({"type":"response.output_text.done","item_id":"msg_one","text":"hello"}),
        &mut state,
    )
    .unwrap();
    assert_eq!(state.full_text, "hello");
}

#[test]
fn persisted_response_items_survive_reopen_and_edited_messages_invalidate_replay() {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::session::store::SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Test", None, None, "chat", None)
        .unwrap();
    let mut state = CodexStreamState::new();
    parse(
        json!({"type":"response.completed","response":{"id":"resp_one","output":output()}}),
        &mut state,
    )
    .unwrap();
    let calls: Vec<_> = collect_complete_tool_calls(&state.tool_calls_map)
        .0
        .into_iter()
        .map(|call| call.tool_call)
        .collect();
    let metadata = protocol::response_metadata(
        request_without_input(&astra_body(&[], None)),
        &output(),
        "",
        &calls,
        &json!({}),
    );
    let id = store
        .add_assistant_with_tool_calls_and_render_parts(
            &session,
            "",
            &calls,
            None,
            None,
            None,
            Some("resp_one"),
            Some(&metadata),
            None,
            None,
            &[],
        )
        .unwrap();
    drop(store);
    let store = crate::session::store::SessionStore::new(dir.path()).unwrap();
    let map = store.get_response_request_metadata(&session).unwrap();
    let history = store.load_session(&session).unwrap().messages;
    let input = build_input_with_metadata(&history, Some(&map));
    assert_eq!(&input[..2], output().as_slice());
    assert_eq!(input[2]["type"], "function_call_output"); // interrupted tool round is normalized
    let mut edited = history[0].clone();
    edited.content = "edited".to_string();
    assert!(protocol::replay_output(&map[&id], &edited).is_none());
    let export = dir.path().join("context.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, &session, "", None, None, &export,
    )
    .unwrap();
    let value: Value = serde_yaml::from_str(&std::fs::read_to_string(export).unwrap()).unwrap();
    assert_eq!(
        value["sessions"][0]["messages"][0]["codexResponse"]["output"],
        json!(output())
    );
}

#[tokio::test]
async fn websocket_prewarm_tool_round_and_reconnect_replay_reasoning() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
            .await
            .unwrap();
        let warm: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(warm["generate"], false);
        ws.send(Message::Text(
            json!({"type":"response.completed","response":{"id":"resp_warm","output":[]},
            "headers":{"x-codex-turn-state":"sticky-one"}})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let request: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["previous_response_id"], "resp_warm");
        assert_eq!(request["input"], json!([]));
        assert_eq!(
            request["client_metadata"]["x-codex-turn-state"],
            "sticky-one"
        );
        ws.send(Message::Text(
            json!({"type":"response.completed","response":{"id":"resp_one","output":output()}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let request: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(request["previous_response_id"], "resp_one");
        assert_eq!(request["input"].as_array().unwrap().len(), 1);
        assert_eq!(request["input"][0]["type"], "function_call_output");
        ws.send(Message::Text(json!({"type":"error","error":{"code":"previous_response_not_found","message":"missing"}}).to_string().into())).await.unwrap();
        drop(ws);
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
            .await
            .unwrap();
        let replay: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(replay.get("generate").is_none());
        assert!(replay.get("previous_response_id").is_none());
        assert!(replay["input"].as_array().unwrap().contains(&output()[0]));
        assert!(replay["input"].as_array().unwrap().contains(&output()[1]));
        ws.send(Message::Text(json!({"type":"response.completed","response":{"id":"resp_two","output":[
            {"type":"message","id":"msg_two","role":"assistant","content":[{"type":"output_text","text":"42"}]}]}}).to_string().into())).await.unwrap();
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let mut turn = TurnState::default();
    let mut history = vec![message("u", "user", "Call probe with value 42")];
    let first = stream_chat(
        "token",
        None,
        CodexTransportMode::Websocket,
        Some(&base),
        "gpt-6-astra",
        "Test instructions",
        &history,
        &tools(),
        None,
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
    assert_eq!(first.tool_calls[0].name, "probe");
    let mut assistant = message("a", "assistant", &first.text);
    assistant.tool_calls = Some(first.tool_calls.clone());
    assistant.response_id = first.response_id;
    history.push(assistant);
    // Exercise the parsed call's real arguments and carry the actual result.
    let args: Value = serde_json::from_str(&first.tool_calls[0].arguments).unwrap();
    let mut result = message("t", "tool", &args["value"].to_string());
    result.tool_call_id = Some(first.tool_calls[0].id.clone());
    history.push(result);
    let metadata = HashMap::from([("a".to_string(), first.continuation_request.unwrap())]);
    let second = stream_chat(
        "token",
        None,
        CodexTransportMode::Websocket,
        Some(&base),
        "gpt-6-astra",
        "Test instructions",
        &history,
        &tools(),
        None,
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
    assert_eq!(second.text, "42");
    server.await.unwrap();
    invalidate_cached_session(&sid);
}

#[tokio::test]
async fn http_fallback_sends_zstd_lite_and_reports_failed_event() {
    use http_body_util::{BodyExt, Full};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for websocket in [true, false] {
            let (tcp, _) = listener.accept().await.unwrap();
            let service = hyper::service::service_fn(
                move |request: hyper::Request<hyper::body::Incoming>| async move {
                    let response = if websocket {
                        assert!(request.headers().contains_key("upgrade"));
                        hyper::Response::builder()
                            .status(426)
                            .body(Full::new(hyper::body::Bytes::new()))
                            .unwrap()
                    } else {
                        assert_eq!(request.headers()["content-encoding"], "zstd");
                        assert_eq!(request.headers()[protocol::LITE_HEADER], "true");
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let plain = zstd::stream::decode_all(bytes.as_ref()).unwrap();
                        let body: Value = serde_json::from_slice(&plain).unwrap();
                        assert!(protocol::is_lite_request(&body));
                        assert!(body.get("previous_response_id").is_none());
                        hyper::Response::builder().header("content-type","text/event-stream")
                        .body(Full::new(hyper::body::Bytes::from("data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"invalid_prompt\",\"message\":\"test rejection\"}}}\n\n"))).unwrap()
                    };
                    Ok::<_, std::convert::Infallible>(response)
                },
            );
            hyper::server::conn::http1::Builder::new()
                .keep_alive(false)
                .serve_connection(hyper_util::rt::TokioIo::new(tcp), service)
                .await
                .unwrap();
        }
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let result = stream_chat(
        "token",
        None,
        CodexTransportMode::Websocket,
        Some(&base),
        "gpt-6-astra",
        "Test",
        &[message("u", "user", "test")],
        &[],
        None,
        Some("low"),
        false,
        false,
        Some(&sid),
        None,
        &mut TurnState::default(),
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .await;
    let error = result.unwrap_err();
    assert!(
        error.contains("invalid_prompt"),
        "unexpected error: {error}"
    );
    assert!(cached_websocket_http_fallback_enabled(Some(&sid), Some(&base), None).await);
    server.await.unwrap();
    invalidate_cached_session(&sid);
}

#[test]
fn compaction_budget_truncates_boundary_and_charges_images() {
    let item = |text: &str| json!({"type":"message","role":"user","content":[{"type":"input_text","text":text}]});
    let retained = retention::retain(vec![item("old"), item(&"中".repeat(200)), item("tail")], 20);
    assert_eq!(retained.len(), 2);
    assert!(retained[0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("tokens truncated"));
    assert_eq!(retained[1], item("tail"));
    let image = json!({"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,invalid"}]});
    assert!(retention::retain(vec![item("old"), image.clone()], 1843).is_empty());
    assert_eq!(retention::retain(vec![image.clone()], 1844), vec![image]);
    let notice = json!({"role":"developer","content":[{"type":"input_text","text":"<image_resize_notice>resized</image_resize_notice>"}]});
    assert!(retention::retain(vec![item(&"x".repeat(400)), notice], 1).is_empty());
}

#[tokio::test]
async fn prewarm_failure_reconnects_and_runs_one_inference() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for warmup in [true, false] {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async_with_config(tcp, Some(websocket_config()))
                .await
                .unwrap();
            let request: Value =
                serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
            if warmup {
                assert_eq!(request["generate"], false);
                ws.send(Message::Text(
                    json!({"type":"response.failed","response":{"error":{
                    "code":"prewarm_unavailable","message":"test prewarm failure"}}})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            } else {
                assert!(request.get("generate").is_none());
                assert!(request.get("previous_response_id").is_none());
                assert!(!request["input"].as_array().unwrap().is_empty());
                ws.send(Message::Text(json!({"type":"response.completed","response":{"id":"resp_ok","output":[
                    {"id":"msg_ok","type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}}).to_string().into())).await.unwrap();
            }
        }
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let response = tokio::time::timeout(
        Duration::from_secs(25),
        stream_chat(
            "token",
            None,
            CodexTransportMode::Websocket,
            Some(&base),
            "gpt-6-astra",
            "Test",
            &[message("u", "user", "test")],
            &[],
            None,
            Some("low"),
            false,
            false,
            Some(&sid),
            None,
            &mut TurnState::default(),
            &|_| {},
            &|_| {},
            &|_, _| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.text, "ok");
    assert!(response.raw_request.contains("response.create"));
    assert!(!response.raw_request.contains("generate"));
    server.await.unwrap();
    invalidate_cached_session(&sid);
}
