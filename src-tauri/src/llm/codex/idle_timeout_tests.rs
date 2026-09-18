use super::*;
use serde_json::{json, Value};

#[test]
fn only_astra_remote_compaction_gets_thirty_minutes_of_response_idle_time() {
    for fast_mode in [false, true] {
        let compaction = CodexStreamOptions::remote_compaction_v2().with_fast_mode(fast_mode);
        for model in [
            "gpt-6-astra",
            "openai/gpt-6-astra",
            "gpt-6-astra-2026-09-08",
        ] {
            assert_eq!(
                compaction.response_idle_timeout(model),
                Duration::from_secs(1800)
            );
            assert_eq!(compaction.idle_timeout, Duration::from_secs(300));
            assert_eq!(
                CodexStreamOptions::default()
                    .with_fast_mode(fast_mode)
                    .response_idle_timeout(model),
                Duration::from_secs(300)
            );
        }
        for model in [
            "gpt-5.6-sol",
            "gpt-5.5",
            "gpt-6-astra-pro",
            "gpt-6-astra-other",
        ] {
            assert_eq!(
                compaction.response_idle_timeout(model),
                Duration::from_secs(300)
            );
        }
    }
}

fn compact_body(session_id: &str) -> Value {
    build_request_body(
        "gpt-6-astra",
        "Test",
        &[],
        &[],
        None,
        Some(session_id),
        None,
        CodexStreamOptions::remote_compaction_v2(),
    )
}

#[tokio::test]
async fn every_websocket_data_message_renews_idle_timeout_before_event_parsing() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let checkpoint = json!({"type":"compaction","encrypted_content":"finished"});
    let server_checkpoint = checkpoint.clone();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async_with_config(socket, Some(websocket_config()))
            .await
            .unwrap();
        let request = ws.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(request.to_text().unwrap()).unwrap();
        assert_eq!(
            request["input"].as_array().unwrap().last().unwrap()["type"],
            "compaction_trigger"
        );
        let events = [
            json!({"type":"response.created","response":{"id":"slow-compact"}}).to_string(),
            json!({"type":"response.in_progress"}).to_string(),
            json!({"type":"response.output_item.added","item":{"type":"compaction","encrypted_content":"pending"}}).to_string(),
            json!({"type":"vendor.compaction.progress"}).to_string(),
            "unrecognized text frame".to_string(),
            json!({"type":"response.metadata"}).to_string(),
            json!({"type":"response.completed","response":{"id":"slow-compact","output":[server_checkpoint]}}).to_string(),
        ];
        for event in events {
            tokio::time::sleep(Duration::from_millis(100)).await;
            ws.send(Message::Text(event.into())).await.unwrap();
        }
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let options = CodexStreamOptions::remote_compaction_v2();
    let mut trace =
        diagnostics::Attempt::new(Some(&sid), &options, 1, CodexTransportMode::Websocket);
    let idle_timeout = Duration::from_millis(300);
    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        stream_chat_websocket_once(
            "test-token",
            None,
            Some(&base),
            "gpt-6-astra",
            &[],
            &[],
            Some(&sid),
            Some(&sid),
            false,
            compact_body(&sid),
            &mut TurnState::default(),
            None,
            &|_| {},
            &|_| {},
            &|_, _| {},
            idle_timeout,
            idle_timeout,
            &mut trace,
        ),
    )
    .await
    .unwrap();
    let response = match result {
        Ok(CodexTransportAttempt::Response(response)) => response,
        Err(error) => panic!("data frames must renew the deadline: {error}"),
        _ => panic!("unexpected fallback"),
    };
    assert!(started.elapsed() > idle_timeout * 2);
    assert!(response.response_completed);
    assert!(response.text.is_empty());
    assert!(response.tool_calls.is_empty());
    assert_eq!(response.response_items, vec![checkpoint]);
    server.await.unwrap();
    invalidate_cached_session(&sid);
}

#[tokio::test]
async fn websocket_ping_pong_does_not_renew_the_response_idle_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async_with_config(socket, Some(websocket_config()))
            .await
            .unwrap();
        ws.next().await.unwrap().unwrap();
        ws.send(Message::Text(
            json!({"type":"response.created","response":{"id":"silent"}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let mut pongs = 0;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(80)).await;
            if ws.send(Message::Ping(vec![1].into())).await.is_err() {
                break;
            }
            match ws.next().await {
                Some(Ok(Message::Pong(_))) => pongs += 1,
                _ => break,
            }
        }
        pongs
    });
    let sid = uuid::Uuid::new_v4().to_string();
    let options = CodexStreamOptions::remote_compaction_v2();
    let mut trace =
        diagnostics::Attempt::new(Some(&sid), &options, 1, CodexTransportMode::Websocket);
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        stream_chat_websocket_once(
            "test-token",
            None,
            Some(&base),
            "gpt-6-astra",
            &[],
            &[],
            Some(&sid),
            Some(&sid),
            false,
            compact_body(&sid),
            &mut TurnState::default(),
            None,
            &|_| {},
            &|_| {},
            &|_, _| {},
            Duration::from_millis(300),
            Duration::from_millis(300),
            &mut trace,
        ),
    )
    .await
    .expect("Ping/Pong must not keep the response alive indefinitely");
    assert!(matches!(result, Err(ref error) if error == "WebSocket read timed out"));
    assert!(server.await.unwrap() > 0);
    invalidate_cached_session(&sid);
}
