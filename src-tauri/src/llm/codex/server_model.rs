//! Prefer explicit server headers, falling back to the response's model
//! declaration. Never infer a response model from the outbound request.
use serde_json::Value;

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(super) fn from_headers(headers: &http::HeaderMap) -> Option<String> {
    ["openai-model", "x-openai-model"]
        .into_iter()
        .find_map(|key| {
            headers
                .get(key)
                .and_then(|value| value.to_str().ok())
                .and_then(nonempty)
        })
}

fn from_json_headers(headers: &Value) -> Option<String> {
    headers.as_object()?.iter().find_map(|(key, value)| {
        if !key.eq_ignore_ascii_case("openai-model") && !key.eq_ignore_ascii_case("x-openai-model")
        {
            return None;
        }
        value.as_str().and_then(nonempty).or_else(|| {
            value
                .as_array()?
                .iter()
                .find_map(|value| value.as_str().and_then(nonempty))
        })
    })
}

pub(crate) fn from_event(event: &Value) -> Option<String> {
    from_json_headers(&event["response"]["headers"])
        .or_else(|| from_json_headers(&event["headers"]))
}

/// Historical metadata retained metadata events but not transport headers.
pub(crate) fn from_saved_response(response: &Value) -> Option<String> {
    if let Some(model) = response
        .get("server_model")
        .and_then(Value::as_str)
        .and_then(nonempty)
    {
        return Some(model);
    }
    let events = &response["events"];
    // Locus stored terminal response fields directly in the events object.
    if let Some(model) = from_json_headers(&events["headers"]) {
        return Some(model);
    }
    let kinds = [
        "response.completed",
        "response.incomplete",
        "response.metadata",
        "codex.response.metadata",
        "response.in_progress",
        "response.created",
    ];
    if let Some(model) = kinds.into_iter().find_map(|kind| from_event(&events[kind])) {
        return Some(model);
    }
    // Terminal response fields are flattened into events by the Codex parser.
    let mut observer = super::super::upstream_model::Observer::default();
    for kind in kinds.into_iter().rev() {
        observer.observe(&events[kind], kind);
    }
    observer.observe(events, "response.completed");
    observer.model()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn server_model_headers_handle_case_arrays_precedence_and_missing_values() {
        assert_eq!(from_event(&json!({"response":{"headers":{"OpenAI-Model":["", "actual"]}},"headers":{"x-openai-model":"fallback"}})).as_deref(), Some("actual"));
        assert_eq!(from_event(&json!({"response":{"headers":{"openai-model":" "}},"headers":{"X-OpenAI-Model":"fallback"}})).as_deref(), Some("fallback"));
        assert_eq!(
            from_event(&json!({"response":{"model":"echoed"},"model":"requested"})),
            None
        );
        let mut headers = http::HeaderMap::new();
        headers.insert("OpenAI-Model", "actual-http".parse().unwrap());
        assert_eq!(from_headers(&headers).as_deref(), Some("actual-http"));
    }

    #[test]
    fn server_model_stream_keeps_latest_report_and_ignores_echoes() {
        let mut state = super::super::CodexStreamState::new();
        state.server_model = Some("handshake".into());
        for event in [
            json!({"type":"response.created","response":{"headers":{"openai-model":"created"}}}),
            json!({"type":"response.metadata","headers":{"openai-model":"routed"}}),
            json!({"type":"response.completed","response":{"model":"echoed"}}),
        ] {
            super::super::process_sse_event_block(
                &format!("data: {event}"),
                false,
                &mut state,
                &|_| {},
                &|_| {},
                &|_, _| {},
            )
            .unwrap();
        }
        assert_eq!(state.server_model.as_deref(), Some("routed"));
        assert_eq!(state.response_model.model().as_deref(), Some("echoed"));
    }

    #[test]
    fn server_model_body_uses_terminal_then_first_declaration_without_headers() {
        for terminal in [Some("completed-model"), None] {
            let mut state = super::super::CodexStreamState::new();
            for event in [
                json!({"type":"response.created","response":{"model":"created-model"}}),
                json!({"type":"response.in_progress","response":{"model":"later-model"}}),
                json!({"type":"response.completed","response":{"model":terminal}}),
            ] {
                super::super::process_sse_event_block(
                    &format!("data: {event}"),
                    false,
                    &mut state,
                    &|_| {},
                    &|_| {},
                    &|_, _| {},
                )
                .unwrap();
            }
            assert_eq!(state.server_model, None);
            assert_eq!(
                state.response_model.model().as_deref(),
                Some(terminal.unwrap_or("created-model"))
            );
        }
    }

    #[tokio::test]
    async fn server_model_http_and_websocket_preserve_headers_and_event_overrides() {
        use super::super::*;
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        for websocket in [false, true] {
            for case in 0..3 {
                let event_override = case == 1;
                let with_header = case != 2;
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let base = format!("http://{}", listener.local_addr().unwrap());
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut completed = json!({"type":"response.completed","response":{"id":"response-test","model":"body-model","output":[]}});
                    if event_override {
                        completed["response"]["headers"] = json!({"X-OpenAI-Model":["routed"]});
                    }
                    if websocket {
                        let mut ws = tokio_tungstenite::accept_hdr_async_with_config(
                            socket,
                            move |_: &http::Request<()>, mut response: http::Response<()>| {
                                if with_header {
                                    response
                                        .headers_mut()
                                        .insert("OpenAI-Model", "handshake".parse().unwrap());
                                }
                                Ok(response)
                            },
                            Some(websocket_config()),
                        )
                        .await
                        .unwrap();
                        ws.next().await.unwrap().unwrap();
                        ws.send(Message::Text(completed.to_string().into()))
                            .await
                            .unwrap();
                    } else {
                        let mut reader = BufReader::new(&mut socket);
                        let mut length = 0;
                        loop {
                            let mut line = String::new();
                            assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                            if line == "\r\n" {
                                break;
                            }
                            if let Some(value) =
                                line.to_ascii_lowercase().strip_prefix("content-length:")
                            {
                                length = value.trim().parse::<usize>().unwrap();
                            }
                        }
                        reader.read_exact(&mut vec![0; length]).await.unwrap();
                        let body = format!("data: {completed}\n\n");
                        let header = if with_header {
                            "OpenAI-Model: handshake\r\n"
                        } else {
                            ""
                        };
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{header}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                    }
                });
                let body = json!({"model":"requested","input":[],"tools":[],"stream":true});
                let options = CodexStreamOptions::default();
                let mode = if websocket {
                    CodexTransportMode::Websocket
                } else {
                    CodexTransportMode::Http
                };
                let mut trace = diagnostics::Attempt::new(None, &options, 1, mode);
                let mut turn_state = TurnState::default();
                let timeout = Duration::from_secs(5);
                let response = if websocket {
                    match stream_chat_websocket_once(
                        "token",
                        None,
                        Some(&base),
                        "requested",
                        &[],
                        &[],
                        None,
                        None,
                        false,
                        body,
                        &mut turn_state,
                        None,
                        &|_| {},
                        &|_| {},
                        &|_, _| {},
                        timeout,
                        timeout,
                        &mut trace,
                    )
                    .await
                    .unwrap()
                    {
                        CodexTransportAttempt::Response(response) => response,
                        _ => panic!("unexpected HTTP fallback"),
                    }
                } else {
                    stream_chat_http_once(
                        "token",
                        None,
                        Some(&base),
                        "requested",
                        &[],
                        &[],
                        None,
                        false,
                        body,
                        None,
                        &mut turn_state,
                        &|_| {},
                        &|_| {},
                        &|_, _| {},
                        timeout,
                        timeout,
                        &mut trace,
                    )
                    .await
                    .unwrap()
                };
                server.await.unwrap();
                let mut saved = response.continuation_request.unwrap();
                assert_eq!(
                    saved["codex_response"]["server_model"],
                    if event_override {
                        "routed"
                    } else if with_header {
                        "handshake"
                    } else {
                        "body-model"
                    }
                );
                // Server identity is metadata, not part of request compatibility.
                assert!(request_without_input(&saved)
                    .get("codex_response")
                    .is_none());
                let signature = request_without_input(&saved);
                saved[crate::llm::upstream_model::METADATA_KEY] = json!("reported");
                assert_eq!(request_without_input(&saved), signature);
            }
        }
    }
}
