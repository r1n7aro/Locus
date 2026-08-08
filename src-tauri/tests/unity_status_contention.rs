#![cfg(target_os = "windows")]

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ServerOptions;

fn test_project_and_pipe() -> (tempfile::TempDir, String, String) {
    let project = tempfile::tempdir().expect("temp Unity project");
    let project_path = project.path().to_string_lossy().to_string();
    std::fs::create_dir_all(project.path().join("Assets")).expect("Assets");
    std::fs::create_dir_all(project.path().join("ProjectSettings")).expect("ProjectSettings");

    locus_lib::unity_bridge::initialize_native_bridge(true);
    locus_lib::unity_bridge::sync_native_bridge_marker(&project_path, true)
        .expect("native bridge marker");
    let pipe_name = std::fs::read_to_string(
        project
            .path()
            .join("Library")
            .join("Locus")
            .join("NativeBridge.enabled"),
    )
    .expect("read native bridge marker")
    .trim()
    .to_string();
    (project, project_path, pipe_name)
}

async fn read_request(
    reader: &mut BufReader<tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeServer>>,
) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read request");
    assert!(!line.is_empty(), "client closed before sending a request");
    serde_json::from_str(line.trim()).expect("request JSON")
}

async fn write_response(
    writer: &mut tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeServer>,
    request: &Value,
    message: &str,
) {
    let request_id = request
        .get("id")
        .and_then(Value::as_str)
        .expect("request id");
    let mut response = serde_json::to_vec(&serde_json::json!({
        "reply_to": request_id,
        "type": "response",
        "ok": true,
        "message": message,
        "processId": std::process::id(),
    }))
    .expect("response JSON");
    response.push(b'\n');
    writer.write_all(&response).await.expect("write response");
    writer.flush().await.expect("flush response");
}

#[tokio::test]
async fn concurrent_authoritative_status_requests_share_the_first_connection() {
    let (_project, project_path, pipe_name) = test_project_and_pipe();
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .expect("create status pipe");
    let server_task = tokio::spawn(async move {
        server.connect().await.expect("accept client");
        let (read_half, mut write_half) = tokio::io::split(server);
        let mut reader = BufReader::new(read_half);
        for _ in 0..2 {
            let request = read_request(&mut reader).await;
            assert_eq!(request.get("type").and_then(Value::as_str), Some("status"));
            write_response(
                &mut write_half,
                &request,
                "editing|Assets/Scenes/Main.unity",
            )
            .await;
        }
    });

    let (first, second) = tokio::join!(
        locus_lib::unity_bridge::query_unity_status(&project_path),
        locus_lib::unity_bridge::query_unity_status(&project_path),
    );
    for result in [first, second] {
        assert!(result.0, "status request must remain connected");
        assert_eq!(
            result.1,
            locus_lib::unity_bridge::UNITY_EDITOR_STATUS_EDITING
        );
        assert_eq!(result.2.as_deref(), Some("Assets/Scenes/Main.unity"));
    }
    server_task.await.expect("status server");
}

#[tokio::test]
async fn observer_reports_connected_while_the_pipe_writer_is_busy() {
    let (_project, project_path, pipe_name) = test_project_and_pipe();
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .expect("create status pipe");
    let server_task = tokio::spawn(async move {
        server.connect().await.expect("accept client");
        let (read_half, mut write_half) = tokio::io::split(server);
        let mut reader = BufReader::new(read_half);
        let request = read_request(&mut reader).await;
        write_response(
            &mut write_half,
            &request,
            "editing|Assets/Scenes/Main.unity",
        )
        .await;

        // Stop consuming input so a large client request remains inside
        // write_all while holding the shared writer lock.
        tokio::time::sleep(Duration::from_secs(10)).await;
    });

    let ready = locus_lib::unity_bridge::query_unity_connection_status(&project_path).await;
    assert!(ready.connected);
    assert_eq!(ready.control_channel_state, "ready");

    let blocked_project = project_path.clone();
    let blocker = tokio::spawn(async move {
        locus_lib::unity_bridge::send_message(
            &blocked_project,
            "block_writer",
            &"x".repeat(32 * 1024 * 1024),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let busy = locus_lib::unity_bridge::query_unity_connection_status(&project_path).await;
    assert!(
        busy.connected,
        "writer contention must preserve connection liveness"
    );
    assert_eq!(busy.control_channel_state, "busy");
    assert_eq!(
        busy.editor_status,
        locus_lib::unity_bridge::UNITY_EDITOR_STATUS_EDITING
    );

    blocker.abort();
    server_task.abort();
}
