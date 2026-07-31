//! End-to-end dogfood tests: Locus's own MCP client (mcp::client::McpClient
//! over Streamable HTTP) drives this server with a mock dispatcher, so the
//! full HTTP + protocol + session surface is exercised without a tauri app.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::http::{self, ServerContext, ToolCallOutcome};
use super::protocol::ToolListing;
use crate::mcp::client::McpClient;
use crate::mcp::config::{McpLoadMode, McpServerConfig, McpTransport};

const TOKEN: &str = "tok-123";
const TIMEOUT: Duration = Duration::from_secs(10);

fn test_context(workspace: Arc<Mutex<String>>) -> Arc<ServerContext> {
    let dispatcher: http::ToolDispatcher = {
        let workspace = workspace.clone();
        Arc::new(
            move |name: String, args: Value| -> BoxFuture<'static, ToolCallOutcome> {
                let workspace = workspace
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                Box::pin(async move {
                    ToolCallOutcome {
                        output: format!("echo {name}: {args}"),
                        is_error: false,
                        images: if name == "img" {
                            vec![("QUJD".to_string(), "image/png".to_string())]
                        } else {
                            Vec::new()
                        },
                        workspace_path: Some(workspace),
                    }
                })
            },
        )
    };
    let list_tools: http::ToolListProvider = Arc::new(|| {
        vec![
            ToolListing {
                name: "unity_project_info".to_string(),
                description: "d".to_string(),
                input_schema: json!({"type":"object"}),
            },
            ToolListing {
                name: "img".to_string(),
                description: "d".to_string(),
                input_schema: json!({"type":"object"}),
            },
        ]
    });
    let instructions: http::InstructionsProvider =
        Arc::new(|| Box::pin(async { "Active project: C:/proj".to_string() }));
    Arc::new(ServerContext::new(
        TOKEN.to_string(),
        dispatcher,
        list_tools,
        instructions,
    ))
}

async fn start_test_server(
    workspace: Arc<Mutex<String>>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let (addr, task) = http::start(0, test_context(workspace))
        .await
        .expect("test server binds");
    (addr.port(), task)
}

fn client_config(port: u16, token: &str) -> McpServerConfig {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
    McpServerConfig {
        id: "locus-selftest".to_string(),
        name: "locus-selftest".to_string(),
        transport: McpTransport::Http,
        command: String::new(),
        args: Vec::new(),
        env: BTreeMap::new(),
        cwd: String::new(),
        url: format!("http://127.0.0.1:{port}/mcp"),
        headers,
        enabled: true,
        call_timeout_ms: 10_000,
        auto_restart: false,
        load_mode: McpLoadMode::Direct,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_handshake_list_and_call() {
    let workspace = Arc::new(Mutex::new("C:/proj".to_string()));
    let (port, server) = start_test_server(workspace).await;

    let client = McpClient::connect(&client_config(port, TOKEN))
        .await
        .expect("client connects");
    let (init, tools) = crate::mcp::run_handshake_and_list(&client)
        .await
        .expect("handshake succeeds");
    assert_eq!(
        init.server_info.as_ref().and_then(|s| s.name.clone()),
        Some("locus".to_string())
    );
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "unity_project_info");

    let result = client
        .request(
            "tools/call",
            Some(json!({"name": "unity_project_info", "arguments": {"a": 1}})),
            TIMEOUT,
        )
        .await
        .expect("tools/call succeeds");
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("echo unity_project_info"), "got: {text}");
    assert_eq!(result["isError"], false);

    client.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_maps_images_into_content() {
    let workspace = Arc::new(Mutex::new("C:/proj".to_string()));
    let (port, server) = start_test_server(workspace).await;

    let client = McpClient::connect(&client_config(port, TOKEN))
        .await
        .expect("client connects");
    crate::mcp::run_handshake_and_list(&client)
        .await
        .expect("handshake succeeds");
    let result = client
        .request(
            "tools/call",
            Some(json!({"name": "img", "arguments": {}})),
            TIMEOUT,
        )
        .await
        .expect("tools/call succeeds");
    let content = result["content"].as_array().expect("content array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["data"], "QUJD");
    assert_eq!(content[1]["mimeType"], "image/png");

    client.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_workspace_change_prepends_notice() {
    let workspace = Arc::new(Mutex::new("C:/proj-a".to_string()));
    let (port, server) = start_test_server(workspace.clone()).await;

    let client = McpClient::connect(&client_config(port, TOKEN))
        .await
        .expect("client connects");
    crate::mcp::run_handshake_and_list(&client)
        .await
        .expect("handshake succeeds");

    let first = client
        .request(
            "tools/call",
            Some(json!({"name": "unity_project_info", "arguments": {}})),
            TIMEOUT,
        )
        .await
        .expect("first call succeeds");
    let first_text = first["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        !first_text.contains("[notice]"),
        "first call has no notice: {first_text}"
    );

    *workspace.lock().unwrap() = "C:/proj-b".to_string();

    let second = client
        .request(
            "tools/call",
            Some(json!({"name": "unity_project_info", "arguments": {}})),
            TIMEOUT,
        )
        .await
        .expect("second call succeeds");
    let second_text = second["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        second_text.starts_with("[notice] The active Locus workspace changed"),
        "got: {second_text}"
    );
    assert!(second_text.contains("C:/proj-b"));

    client.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_unknown_method_and_tool_error() {
    let workspace = Arc::new(Mutex::new("C:/proj".to_string()));
    let (port, server) = start_test_server(workspace).await;

    let client = McpClient::connect(&client_config(port, TOKEN))
        .await
        .expect("client connects");
    crate::mcp::run_handshake_and_list(&client)
        .await
        .expect("handshake succeeds");

    let method_error = client
        .request("resources/list", Some(json!({})), TIMEOUT)
        .await
        .expect_err("unknown method errors");
    assert!(method_error.contains("-32601"), "got: {method_error}");

    let tool_error = client
        .request(
            "tools/call",
            Some(json!({"name": "nope", "arguments": {}})),
            TIMEOUT,
        )
        .await
        .expect_err("unknown tool errors");
    assert!(
        tool_error.contains("Unknown or disabled tool"),
        "got: {tool_error}"
    );

    client.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn rejects_bad_auth_origin_and_host() {
    let workspace = Arc::new(Mutex::new("C:/proj".to_string()));
    let (port, server) = start_test_server(workspace).await;
    let url = format!("http://127.0.0.1:{port}/mcp");
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"}).to_string();
    let http = reqwest::Client::new();

    let no_auth = http
        .post(&url)
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("request sends");
    assert_eq!(no_auth.status().as_u16(), 401);

    let wrong_token = http
        .post(&url)
        .header("authorization", "Bearer wrong")
        .body(body.clone())
        .send()
        .await
        .expect("request sends");
    assert_eq!(wrong_token.status().as_u16(), 401);

    let with_origin = http
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("origin", "http://evil.example")
        .body(body.clone())
        .send()
        .await
        .expect("request sends");
    assert_eq!(with_origin.status().as_u16(), 403);

    let bad_host = http
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("host", "evil.example")
        .body(body.clone())
        .send()
        .await
        .expect("request sends");
    assert_eq!(bad_host.status().as_u16(), 403);

    let good = http
        .post(&url)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(body)
        .send()
        .await
        .expect("request sends");
    assert_eq!(good.status().as_u16(), 200);

    server.abort();
}
