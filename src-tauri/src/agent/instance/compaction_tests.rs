use super::*;
use crate::commands::CodexTransportMode;
use http_body_util::{BodyExt, Full};
use serde_json::{json, Value};

#[tokio::test]
async fn compaction_uses_the_native_agent_tool_snapshot_and_existing_http_turn() {
    let temp = tempfile::tempdir().unwrap();
    let mut instance = super::super::tests::native_plan_test_instance(&temp);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    instance.backend = LlmBackend::Custom {
        api_key: "test-token".into(),
        api_model: "gpt-6-astra".into(),
        endpoint: base.clone(),
        api_format: crate::commands::ApiFormat::OpenaiResponses,
        context_length: 500_000,
        remote_compaction_mode: crate::commands::RemoteCompactionMode::CodexV2,
        supports_tool_lazy_loading: true,
        supported_reasoning_efforts: vec![],
        reasoning_param_format:
            crate::commands::CustomReasoningParamFormat::OpenaiResponsesReasoningEffort,
        replay_reasoning_content: false,
        reasoning_replay_field: None,
        server_tools: Default::default(),
        supports_vision: true,
    };
    instance.effective_model = "gpt-6-astra".into();
    let selected = HashSet::from(["skill_list".to_string()]);
    let prepared = instance
        .prepare_request_tools(
            LazyToolRenderer::CodexNative,
            crate::config::DynamicToolLoadingMode::Native,
            &selected,
        )
        .await;
    assert!(prepared.tool_search_description.is_some());
    assert!(!prepared
        .api_tools
        .iter()
        .any(|tool| tool["function"]["name"] == "tool_call"));
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = records.clone();
    let server = tokio::spawn(async move {
        for compact in [false, true] {
            let (socket, _) = listener.accept().await.unwrap();
            let received = received.clone();
            let service = hyper::service::service_fn(
                move |request: hyper::Request<hyper::body::Incoming>| {
                    let received = received.clone();
                    async move {
                        let sticky = request
                            .headers()
                            .get("x-codex-turn-state")
                            .map(|h| h.to_str().unwrap().to_string());
                        let bytes = request.into_body().collect().await.unwrap().to_bytes();
                        let body: Value = serde_json::from_slice(
                            &zstd::stream::decode_all(bytes.as_ref()).unwrap(),
                        )
                        .unwrap();
                        received.lock().unwrap().push((body, sticky));
                        let output = if compact {
                            json!([{"type":"compaction","encrypted_content":"opaque"}])
                        } else {
                            json!([{"type":"message","id":"msg_first","role":"assistant","content":[{"type":"output_text","text":"Ready"}]}])
                        };
                        Ok::<_, std::convert::Infallible>(hyper::Response::builder().header("content-type","text/event-stream")
                        .header("x-codex-turn-state", if compact { "later-state" } else { "sampling-state" })
                        .body(Full::new(hyper::body::Bytes::from(format!("data: {}\n\n", json!({"type":"response.completed","response":{"id":if compact {"compact"} else {"first"},"output":output}}))))).unwrap())
                    }
                },
            );
            hyper::server::conn::http1::Builder::new()
                .keep_alive(false)
                .serve_connection(hyper_util::rt::TokioIo::new(socket), service)
                .await
                .unwrap();
        }
    });
    let mut turn = codex::TurnState::default();
    let user: ChatMessage = serde_json::from_value(
        json!({"id":"u","role":"user","content":"Run the selected skill","createdAt":0}),
    )
    .unwrap();
    let mut history = vec![user];
    let response = codex::stream_chat(
        "test-token",
        None,
        CodexTransportMode::Http,
        Some(&base),
        "gpt-6-astra",
        "Instructions",
        &history,
        &prepared.api_tools,
        prepared.tool_search_description.as_deref(),
        None,
        false,
        false,
        Some(&instance.session_id),
        None,
        &mut turn,
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .await
    .unwrap();
    let assistant: ChatMessage = serde_json::from_value(json!({"id":"a","role":"assistant","content":response.text,"responseId":response.response_id,"createdAt":0})).unwrap();
    history.push(assistant);
    let metadata = HashMap::from([("a".to_string(), response.continuation_request.unwrap())]);
    let compacted = instance
        .request_codex_compaction(
            "Instructions",
            &history,
            &prepared,
            &mut turn,
            &metadata,
            "run",
            2,
        )
        .await
        .unwrap();
    assert_eq!(compacted.encrypted_content.as_deref(), Some("opaque"));
    server.await.unwrap();
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].0["input"][0], records[1].0["input"][0]);
    assert_eq!(records[1].1.as_deref(), Some("sampling-state"));
    assert!(records[0].1.is_none());
    assert_eq!(
        records[1].0["input"].as_array().unwrap().last().unwrap()["type"],
        "compaction_trigger"
    );
}
