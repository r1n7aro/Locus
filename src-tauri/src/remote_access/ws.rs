//! One authenticated WebSocket session: RPC requests in, responses and
//! workspace events out.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use serde_json::Value;
use tauri::Manager;
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use locus_remote_protocol::{self as protocol, error_codes, methods, ClientFrame, DeviceInfo, ServerFrame};
use crate::workspace_service::ProjectRegistry;

use super::connections::Connection;
use super::devices::DeviceRecord;
use super::server::ConnContext;
use super::{file_watch, invoke, policy, Shared};
use crate::workspace_service::event::WindowEventInterests;

/// Clients ping every 25 s; three missed pings end the session.
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_IN_FLIGHT: usize = 8;
const RESPONSE_QUEUE: usize = 64;

/// Error code, message, and the command's own error value if it failed.
type DispatchError = (&'static str, String, Option<Value>);
/// Successful results are JSON text, passed through without re-encoding.
type DispatchResult = Result<String, DispatchError>;

fn encode(frame: &ServerFrame) -> String {
    serde_json::to_string(frame).unwrap_or_default()
}

/// `ServerFrame::Response` with `ok: true`, built around already-serialized
/// JSON so large session payloads are not parsed and re-encoded here.
fn response_frame(id: u64, result_json: &str) -> String {
    format!(r#"{{"type":"response","id":{id},"ok":true,"result":{result_json}}}"#)
}

/// The client drops the connection on a frame above the limit, so an
/// oversized result becomes an error for that one request.
fn bounded_frame(id: u64, frame: String) -> String {
    if frame.len() <= protocol::MAX_FRAME_BYTES {
        return frame;
    }
    let megabytes = frame.len().div_ceil(1024 * 1024);
    encode(&ServerFrame::err(
        id,
        error_codes::TOO_LARGE,
        format!("The result ({megabytes} MB) is too large to send to this device"),
    ))
}

pub async fn run(shared: Arc<Shared>, io: TokioIo<Upgraded>, device: DeviceRecord, context: ConnContext) {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(protocol::MAX_FRAME_BYTES);
    config.max_frame_size = Some(protocol::MAX_FRAME_BYTES);
    let socket = WebSocketStream::from_raw_socket(io, Role::Server, Some(config)).await;

    let (connection, mut events, mut close) = Connection::new(device.id.clone(), context.local);
    shared.connections.insert(&shared.app, connection.clone());
    shared.notify_changed();

    let (mut sink, mut stream) = socket.split();
    let (responses_tx, mut responses) = mpsc::channel::<String>(RESPONSE_QUEUE);
    let in_flight = Arc::new(Semaphore::new(MAX_IN_FLIGHT));

    let reader = async {
        loop {
            let message = match tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(message))) => message,
                _ => break,
            };
            let text = match message {
                Message::Text(text) => text,
                Message::Ping(_) | Message::Pong(_) => continue,
                // Binary frames are not part of the protocol.
                Message::Close(_) | Message::Binary(_) | Message::Frame(_) => break,
            };
            let Ok(frame) = serde_json::from_str::<ClientFrame>(text.as_str()) else {
                break;
            };
            match frame {
                ClientFrame::Ping { nonce } => {
                    let _ = responses_tx.try_send(encode(&ServerFrame::Pong { nonce }));
                }
                ClientFrame::Request { id, method, params } => {
                    let Ok(permit) = in_flight.clone().try_acquire_owned() else {
                        let busy = ServerFrame::err(id, error_codes::BUSY, "Too many concurrent requests");
                        let _ = responses_tx.try_send(encode(&busy));
                        continue;
                    };
                    let shared = shared.clone();
                    let connection = connection.clone();
                    let responses_tx = responses_tx.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let frame = match dispatch(&shared, &connection, &method, params).await {
                            Ok(json) => response_frame(id, &json),
                            Err((code, message, data)) => encode(&ServerFrame::err_with_data(id, code, message, data)),
                        };
                        let _ = responses_tx.send(bounded_frame(id, frame)).await;
                    });
                }
            }
        }
    };

    let writer = async {
        loop {
            tokio::select! {
                biased;
                changed = close.changed() => {
                    let reason = changed
                        .ok()
                        .and_then(|_| close.borrow().clone())
                        .unwrap_or_else(|| "closing".into());
                    let notice = encode(&ServerFrame::Closing { reason: reason.clone() });
                    let _ = sink.send(Message::Text(notice.into())).await;
                    let _ = sink
                        .send(Message::Close(Some(CloseFrame { code: CloseCode::Normal, reason: reason.into() })))
                        .await;
                    break;
                }
                Some(text) = responses.recv() => {
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                _ = connection.overflowed() => {
                    if connection.take_overflow() {
                        while let Ok(frame) = events.try_recv() {
                            connection.event_sent(&frame);
                        }
                        let resync = encode(&ServerFrame::Resync { reason: "overflow".into() });
                        if sink.send(Message::Text(resync.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Some(frame) = events.recv() => {
                    connection.event_sent(&frame);
                    if sink.send(Message::Text(frame)).await.is_err() {
                        break;
                    }
                }
            }
        }
    };

    tokio::select! {
        _ = reader => {}
        _ = writer => {}
    }
    file_watch::release_all(&shared.app, &connection);
    shared.connections.remove(&connection.id);
    shared.notify_changed();
}

async fn dispatch(
    shared: &Arc<Shared>,
    connection: &Connection,
    method: &str,
    params: Value,
) -> DispatchResult {
    if !methods::is_valid_name(method) {
        return Err((error_codes::UNKNOWN_METHOD, format!("Unknown method '{method}'"), None));
    }
    // Permissions can change and devices can be revoked while connected.
    let Some(device) = shared.devices.get(&connection.device_id) else {
        connection.request_close("revoked");
        return Err((error_codes::FORBIDDEN, "This device is no longer paired".into(), None));
    };
    match method {
        methods::SET_INTERESTS => {
            let interests: WindowEventInterests = serde_json::from_value(params)
                .map_err(|error| (error_codes::INVALID_PARAMS, error.to_string(), None))?;
            connection
                .set_interests(interests)
                .map_err(|error| (error_codes::INVALID_PARAMS, error, None))?;
            return Ok("null".into());
        }
        methods::SET_FILE_WATCHES => {
            #[derive(serde::Deserialize)]
            struct Params {
                watches: Vec<protocol::FileWatch>,
            }
            let Params { watches } = serde_json::from_value(params)
                .map_err(|error| (error_codes::INVALID_PARAMS, error.to_string(), None))?;
            connection
                .set_file_watches(watches)
                .map_err(|error| (error_codes::INVALID_PARAMS, error, None))?;
            // Watching is on demand: keep the watchers of these workspaces running.
            file_watch::sync(&shared.app, connection);
            return Ok("null".into());
        }
        methods::GET_DEVICE_INFO => {
            let info = DeviceInfo {
                device_id: device.id,
                device_name: device.name,
                full_control: device.full_control,
                desktop_id: shared.settings_snapshot().desktop_id,
                desktop_name: super::config::desktop_name(),
                build_id: shared.build_id(),
                methods: policy::allowed_methods(device.full_control).to_vec(),
            };
            return serde_json::to_string(&info).map_err(|error| (error_codes::FAILED, error.to_string(), None));
        }
        _ => {}
    }
    let Some(rule) = policy::rule_for(method) else {
        return Err((error_codes::UNKNOWN_METHOD, format!("'{method}' is only available on the desktop"), None));
    };
    if !policy::allows(rule, device.full_control) {
        return Err((
            error_codes::FORBIDDEN,
            "This needs full control; allow it for this device in Locus settings".into(),
            None,
        ));
    }
    if !device.full_control && !rule.paths.is_empty() {
        let roots = policy::workspace_roots(shared.app.state::<Arc<ProjectRegistry>>().inner());
        if let Some(path) = policy::path_violation(rule, &params, &roots) {
            return Err((error_codes::FORBIDDEN, format!("'{path}' is outside the open workspaces"), None));
        }
    }
    match method {
        methods::ANSWER_QUESTION => {
            let question_id = params
                .get("questionId")
                .and_then(Value::as_str)
                .ok_or((error_codes::INVALID_PARAMS, "questionId is required".to_string(), None))?;
            // Tool confirmations are registered without a form; ask_user
            // questions always carry one (see `user_question::take_answer`).
            // Unknown kinds therefore fail closed. The lock is released before
            // forwarding because `answer_question` takes it again.
            let is_tool_confirmation = {
                let store = shared.app.state::<crate::QuestionStore>();
                let pending = store.lock().await;
                pending.get(question_id).map(|question| question.form.is_none())
            };
            match is_tool_confirmation {
                None => {
                    return Err((error_codes::FAILED, "Question not found or already answered".into(), None))
                }
                Some(true) if !device.full_control => {
                    return Err((
                        error_codes::FORBIDDEN,
                        "This device may not approve tool calls; allow full control in Locus settings".into(),
                        None,
                    ))
                }
                _ => {}
            }
        }
        _ => {}
    }
    invoke::call(&shared.app, method, params)
        .await
        .map_err(|failure| (error_codes::FAILED, failure.message, failure.data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_response_frames_parse_as_protocol_frames() {
        let frame: ServerFrame = serde_json::from_str(&response_frame(7, r#"{"a":[1,2]}"#)).unwrap();
        assert_eq!(frame, ServerFrame::ok(7, serde_json::json!({"a": [1, 2]})));
        let frame: ServerFrame = serde_json::from_str(&response_frame(8, "null")).unwrap();
        assert_eq!(frame, ServerFrame::Response { id: 8, ok: true, result: None, error: None });
    }

    #[test]
    fn oversized_results_become_errors() {
        let small = response_frame(3, "[1]");
        assert_eq!(bounded_frame(3, small.clone()), small);
        let big = response_frame(4, &format!("\"{}\"", "x".repeat(protocol::MAX_FRAME_BYTES)));
        let frame: ServerFrame = serde_json::from_str(&bounded_frame(4, big)).unwrap();
        match frame {
            ServerFrame::Response { id, ok, error: Some(error), .. } => {
                assert_eq!((id, ok), (4, false));
                assert_eq!(error.code, error_codes::TOO_LARGE);
            }
            other => panic!("unexpected frame {other:?}"),
        }
    }
}
