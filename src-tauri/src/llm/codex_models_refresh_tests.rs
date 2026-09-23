use super::*;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn cached_model() -> CodexRemoteModel {
    serde_json::from_value(serde_json::json!({
        "slug": "gpt-existing",
        "visibility": "list"
    }))
    .unwrap()
}

async fn models_server(status: &str, body: &str) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nETag: \"new\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut request = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                let done = line == "\r\n";
                request.push_str(&line);
                if done {
                    break;
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            request
        })
        .await
        .expect("models request timed out")
    });
    (base_url, server)
}

#[tokio::test]
async fn explicit_refresh_fetches_and_replaces_a_fresh_cache() {
    let dir = tempfile::tempdir().unwrap();
    persist_cache(dir.path(), &[cached_model()], Some("\"old\"".into())).unwrap();
    let (base_url, server) = models_server(
        "200 OK",
        r#"{"models":[{"slug":"gpt-new","visibility":"list"}]}"#,
    )
    .await;

    let cached =
        list_codex_available_models("test-token", None, Some(&base_url), dir.path(), false)
            .await
            .unwrap();
    assert_eq!(cached[0].id, "openai/gpt-existing");

    let refreshed = list_codex_available_models(
        "test-token",
        Some("test-account"),
        Some(&base_url),
        dir.path(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(refreshed[0].id, "openai/gpt-new");
    assert_eq!(
        load_fresh_cache(dir.path()).unwrap().models[0].slug,
        "gpt-new"
    );

    let request = server.await.unwrap().to_ascii_lowercase();
    assert!(request.starts_with(&format!(
        "get /models?client_version={} ",
        CODEX_CLIENT_VERSION
    )));
    assert!(request.contains("authorization: bearer test-token\r\n"));
    assert!(request.contains("chatgpt-account-id: test-account\r\n"));
    assert!(request.contains("if-none-match: \"old\"\r\n"));
}

#[tokio::test]
async fn explicit_refresh_accepts_a_not_modified_response() {
    let dir = tempfile::tempdir().unwrap();
    persist_cache(dir.path(), &[cached_model()], Some("\"old\"".into())).unwrap();
    let (base_url, server) = models_server("304 Not Modified", "").await;

    let models = list_codex_available_models("test-token", None, Some(&base_url), dir.path(), true)
        .await
        .unwrap();

    assert_eq!(models[0].id, "openai/gpt-existing");
    assert!(load_fresh_cache(dir.path()).is_some());
    assert!(server
        .await
        .unwrap()
        .to_ascii_lowercase()
        .contains("if-none-match: \"old\"\r\n"));
}

#[tokio::test]
async fn explicit_refresh_reports_errors_and_preserves_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    persist_cache(dir.path(), &[cached_model()], Some("\"old\"".into())).unwrap();
    let original = std::fs::read(cache_path(dir.path())).unwrap();
    let (base_url, server) = models_server("503 Service Unavailable", "unavailable").await;

    let result =
        list_codex_available_models("test-token", None, Some(&base_url), dir.path(), true).await;

    assert!(result.unwrap_err().contains("503"));
    assert_eq!(std::fs::read(cache_path(dir.path())).unwrap(), original);
    server.await.unwrap();
}
