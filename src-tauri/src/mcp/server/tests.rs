//! End-to-end dogfood tests: Locus's own MCP client (mcp::client::McpClient
//! over Streamable HTTP) drives this server with a mock dispatcher, so the
//! full HTTP + protocol + session surface is exercised without a tauri app.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::http::{self, ServerContext, ToolCallOutcome};
use super::protocol::ToolListing;
use crate::mcp::client::McpClient;
use crate::mcp::config::{McpLoadMode, McpServerConfig, McpTransport};

const TOKEN: &str = "tok-123";
const TIMEOUT: Duration = Duration::from_secs(10);

fn test_context() -> Arc<ServerContext> {
    let resolve_checkout: http::CheckoutResolver = Arc::new(|request| {
        let current_generation = match request.checkout_id.as_str() {
            "checkout-a" => 11,
            "checkout-b" => 22,
            other => return Err(format!("unknown checkout '{other}'")),
        };
        if request
            .expected_generation
            .is_some_and(|expected| expected != current_generation)
        {
            return Err(format!(
                "stale generation for {}: current is {current_generation}",
                request.checkout_id
            ));
        }
        Ok(http::CheckoutBinding {
            checkout_id: request.checkout_id,
            workspace_generation: current_generation,
        })
    });
    let dispatcher: http::ToolDispatcher = Arc::new(
        move |binding, name: String, args: Value| -> BoxFuture<'static, ToolCallOutcome> {
            Box::pin(async move {
                if let Some(delay_ms) = args.get("delayMs").and_then(Value::as_u64) {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                ToolCallOutcome {
                    output: format!(
                        "echo {name} on {}@{}: {args}",
                        binding.checkout_id, binding.workspace_generation
                    ),
                    is_error: false,
                    images: if name == "img" {
                        vec![("QUJD".to_string(), "image/png".to_string())]
                    } else {
                        Vec::new()
                    },
                    workspace_path: Some(format!("C:/{}", binding.checkout_id)),
                }
            })
        },
    );
    let list_tools: http::ToolListProvider = Arc::new(|_binding| {
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
    let instructions: http::InstructionsProvider = Arc::new(|binding| {
        Box::pin(async move {
            format!(
                "Bound checkout: {}@{}",
                binding.checkout_id, binding.workspace_generation
            )
        })
    });
    Arc::new(ServerContext::new(
        TOKEN.to_string(),
        resolve_checkout,
        dispatcher,
        list_tools,
        instructions,
    ))
}

async fn start_test_server() -> (u16, tokio::task::JoinHandle<()>) {
    let (addr, task) = http::start(0, test_context())
        .await
        .expect("test server binds");
    (addr.port(), task)
}

fn client_config(
    port: u16,
    token: &str,
    checkout_id: Option<&str>,
    workspace_generation: Option<u64>,
) -> McpServerConfig {
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
        url: checkout_id
            .map(|checkout_id| {
                let mut url = format!("http://127.0.0.1:{port}/mcp?checkoutId={checkout_id}");
                if let Some(generation) = workspace_generation {
                    url.push_str(&format!("&workspaceGeneration={generation}"));
                }
                url
            })
            .unwrap_or_else(|| format!("http://127.0.0.1:{port}/mcp")),
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
    let (port, server) = start_test_server().await;

    let client = McpClient::connect(&client_config(port, TOKEN, Some("checkout-a"), None))
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
    assert!(text.contains("checkout-a@11"), "got: {text}");
    assert_eq!(result["isError"], false);

    client.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_maps_images_into_content() {
    let (port, server) = start_test_server().await;

    let client = McpClient::connect(&client_config(port, TOKEN, Some("checkout-a"), Some(11)))
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
async fn parallel_sessions_keep_a_b_bindings_across_slow_fast_reversal() {
    let (port, server) = start_test_server().await;
    let client_a = McpClient::connect(&client_config(port, TOKEN, Some("checkout-a"), None))
        .await
        .expect("A client connects");
    let client_b = McpClient::connect(&client_config(port, TOKEN, Some("checkout-b"), None))
        .await
        .expect("B client connects");
    crate::mcp::run_handshake_and_list(&client_a)
        .await
        .expect("A handshake succeeds");
    crate::mcp::run_handshake_and_list(&client_b)
        .await
        .expect("B handshake succeeds");

    async fn call(client: &McpClient, delay_ms: u64) -> String {
        let result = client
            .request(
                "tools/call",
                Some(json!({
                    "name": "unity_project_info",
                    "arguments": {"delayMs": delay_ms},
                })),
                TIMEOUT,
            )
            .await
            .expect("scoped call succeeds");
        result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    let a_slow = call(&client_a, 120);
    let b_fast = call(&client_b, 5);
    tokio::pin!(a_slow, b_fast);
    let b_fast = tokio::select! {
        result = &mut b_fast => result,
        result = &mut a_slow => panic!("A slow call completed before B fast call: {result}"),
    };
    let a_slow = a_slow.await;
    assert!(a_slow.contains("checkout-a@11"), "got: {a_slow}");
    assert!(b_fast.contains("checkout-b@22"), "got: {b_fast}");

    let a_fast = call(&client_a, 5);
    let b_slow = call(&client_b, 120);
    tokio::pin!(a_fast, b_slow);
    let a_fast = tokio::select! {
        result = &mut a_fast => result,
        result = &mut b_slow => panic!("B slow call completed before A fast call: {result}"),
    };
    let b_slow = b_slow.await;
    assert!(a_fast.contains("checkout-a@11"), "got: {a_fast}");
    assert!(b_slow.contains("checkout-b@22"), "got: {b_slow}");

    client_a.shutdown().await;
    client_b.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn initialize_requires_explicit_checkout_and_rejects_stale_generation() {
    let (port, server) = start_test_server().await;
    let unbound = McpClient::connect(&client_config(port, TOKEN, None, None))
        .await
        .expect("transport connects");
    let error = crate::mcp::run_handshake_and_list(&unbound)
        .await
        .expect_err("unbound initialize fails");
    assert!(
        error.contains("checkout binding is required"),
        "got: {error}"
    );

    let stale = McpClient::connect(&client_config(port, TOKEN, Some("checkout-a"), Some(10)))
        .await
        .expect("transport connects");
    let error = crate::mcp::run_handshake_and_list(&stale)
        .await
        .expect_err("stale initialize fails");
    assert!(error.contains("stale generation"), "got: {error}");

    unbound.shutdown().await;
    stale.shutdown().await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn session_binding_rejects_endpoint_checkout_switch() {
    let (port, server) = start_test_server().await;
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("test HTTP client builds");
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-06-18"},
    });
    let response = http
        .post(format!("http://127.0.0.1:{port}/mcp?checkoutId=checkout-a"))
        .bearer_auth(TOKEN)
        .json(&initialize)
        .send()
        .await
        .expect("initialize sends");
    assert_eq!(response.status().as_u16(), 200);
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("initialize returns session id")
        .to_string();

    let switched = http
        .post(format!("http://127.0.0.1:{port}/mcp?checkoutId=checkout-b"))
        .bearer_auth(TOKEN)
        .header("mcp-session-id", session_id)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .expect("switched request sends");
    assert_eq!(switched.status().as_u16(), 409);
    assert!(switched
        .text()
        .await
        .expect("conflict body reads")
        .contains("does not match"));

    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn dogfood_unknown_method_and_tool_error() {
    let (port, server) = start_test_server().await;

    let client = McpClient::connect(&client_config(port, TOKEN, Some("checkout-a"), None))
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
    let (port, server) = start_test_server().await;
    let url = format!("http://127.0.0.1:{port}/mcp?checkoutId=checkout-a");
    let body = json!({
        "jsonrpc":"2.0",
        "id":1,
        "method":"initialize",
        "params":{"protocolVersion":"2025-06-18"}
    })
    .to_string();
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("test HTTP client builds");

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
