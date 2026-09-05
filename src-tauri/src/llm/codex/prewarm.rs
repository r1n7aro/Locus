use super::*;

/// A setup request never emits UI deltas, executes tools, or contributes usage.
/// Its response ID becomes the baseline for the real request on this socket.
pub(super) async fn run(
    socket: &mut CodexWebsocketStream,
    body: &serde_json::Value,
    turn_state: &mut TurnState,
) -> Result<LastWebsocketResponse, String> {
    let mut request = build_websocket_transport_request(body, None, true);
    request["generate"] = serde_json::json!(false);
    protocol::add_turn_state(&mut request, turn_state.header_value());
    let operation = async {
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| format!("Codex prewarm send failed: {error}"))?;
        loop {
            let message = socket
                .next()
                .await
                .ok_or("Codex prewarm socket closed")?
                .map_err(|error| format!("Codex prewarm read failed: {error}"))?;
            let Message::Text(text) = message else {
                return Err("Codex prewarm received an unexpected websocket message".to_string());
            };
            let event: serde_json::Value = serde_json::from_str(&text)
                .map_err(|error| format!("Invalid Codex prewarm event: {error}"))?;
            if let Some(error) = protocol::event_error(&event) {
                return Err(error);
            }
            if let Some(state) = protocol::event_turn_state(&event) {
                turn_state.store_header(Some(state));
            }
            if event["type"] == "response.completed" {
                let id = event["response"]["id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or("Codex prewarm completed without response ID")?;
                let output = event["response"]["output"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if !output.is_empty() {
                    return Err("Codex prewarm unexpectedly generated output".to_string());
                }
                return Ok(LastWebsocketResponse {
                    request_signature: websocket_request_signature(body),
                    input: body["input"].as_array().cloned().unwrap_or_default(),
                    response_id: id.to_string(),
                    items_added: output,
                });
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(15), operation)
        .await
        .map_err(|_| "Codex prewarm timed out".to_string())?
}
