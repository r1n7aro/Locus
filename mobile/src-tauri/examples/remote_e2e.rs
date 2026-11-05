//! Phone-side end-to-end check against a running Locus desktop.
//!
//! Usage: remote_e2e <pairing-link>
//! The desktop must have remote access enabled with "require approval" off
//! (the driver script `scripts/locus-remote-access-e2e.ts` sets that up).
//! Prints `STEP <name> ok|skip|FAIL <detail>`; after `READY_FOR_REVOKE` it waits
//! for a line on stdin, then expects the desktop to have revoked the device.

use std::time::Duration;

use locus_remote_lib::client::{ClientError, HttpClient};
use locus_remote_lib::connection::{Connection, LinkState};
use locus_remote_protocol::{self as protocol, PairRequest, PairingLink};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

struct Report {
    failures: usize,
}

impl Report {
    fn ok(&self, step: &str, detail: impl std::fmt::Display) {
        println!("STEP {step} ok {detail}");
    }
    fn skip(&self, step: &str, detail: impl std::fmt::Display) {
        println!("STEP {step} skip {detail}");
    }
    fn fail(&mut self, step: &str, detail: impl std::fmt::Display) {
        self.failures += 1;
        println!("STEP {step} FAIL {detail}");
    }
    fn check(&mut self, step: &str, condition: bool, detail: impl std::fmt::Display) {
        if condition {
            self.ok(step, detail)
        } else {
            self.fail(step, detail)
        }
    }
}

fn raw_client(fingerprint: &str) -> reqwest::Client {
    let tls = locus_remote_lib::tls::client_config(Some(fingerprint)).expect("tls config");
    reqwest::Client::builder()
        .use_preconfigured_tls((*tls).clone())
        .http1_only()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

async fn wait_state(connection: &Connection, wanted: LinkState, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if connection.status().state == wanted {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A blob needs the token, keeps its bytes (`i % 251` pattern from the
/// driver) and never renders as a page.
#[allow(clippy::too_many_arguments)]
async fn check_blob(
    report: &mut Report,
    raw: &reqwest::Client,
    client: &HttpClient,
    address: &str,
    token: &str,
    step: &str,
    id: &str,
    size: Option<u64>,
) {
    let path = format!("{}{id}", protocol::BLOB_PREFIX);
    let anonymous = raw.get(format!("{address}{path}")).send().await;
    if !anonymous.as_ref().is_ok_and(|response| response.status() == 401) {
        report.fail(step, format!("anonymous fetch: {:?}", anonymous.map(|response| response.status())));
        return;
    }
    match client.get_asset(&path, token).await {
        Ok(asset) => {
            let header = |name: &str| asset.headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or("").to_string();
            let pattern = asset.body.iter().enumerate().all(|(index, byte)| *byte == (index % 251) as u8);
            let sized = size.is_none_or(|size| asset.body.len() as u64 == size);
            report.check(
                step,
                asset.status == 200 && pattern && sized && header("content-type") == "image/png"
                    && header("content-security-policy").starts_with("sandbox"),
                format!("status {} bytes {} type {}", asset.status, asset.body.len(), header("content-type")),
            );
        }
        Err(error) => report.fail(step, error),
    }
}

#[tokio::main]
async fn main() {
    let link = std::env::args().nth(1).expect("usage: remote_e2e <pairing-link>");
    let link = PairingLink::parse(&link).expect("valid pairing link");
    let mut report = Report { failures: 0 };
    let address = link
        .addresses
        .iter()
        .find(|address| address.pinned)
        .expect("pairing link with a direct (pinned) address")
        .url
        .clone();

    // 1. A wrong pin is a hard failure.
    let wrong = HttpClient::new(&address, Some(&"0".repeat(64))).expect("client");
    report.check(
        "pin_mismatch_rejected",
        matches!(wrong.hello().await, Err(ClientError::PinMismatch)),
        "wrong fingerprint refused",
    );

    // 2. Hello over the pinned certificate.
    let client = HttpClient::new(&address, Some(&link.fingerprint)).expect("client");
    match client.hello().await {
        Ok(hello) => report.check(
            "hello",
            hello.protocol == protocol::PROTOCOL_VERSION && hello.desktop_id == link.desktop_id && hello.pairing,
            format!("{hello:?}"),
        ),
        Err(error) => report.fail("hello", error),
    }

    // 3. Browser-style and misrouted requests are refused.
    let raw = raw_client(&link.fingerprint);
    let hello_url = format!("{address}{}", protocol::HELLO_PATH);
    let origin = raw.get(&hello_url).header("Origin", "https://evil.example").send().await;
    report.check("origin_rejected", origin.as_ref().is_ok_and(|r| r.status() == 403), format!("{:?}", origin.map(|r| r.status())));
    let host = raw.get(&hello_url).header("Host", "evil.example").send().await;
    report.check("host_rejected", host.as_ref().is_ok_and(|r| r.status() == 421), format!("{:?}", host.map(|r| r.status())));

    // 4. Wrong secret uses one attempt and is refused.
    let wrong_secret = PairRequest {
        secret: Some(protocol::encode_pairing_secret(&[9u8; protocol::PAIRING_SECRET_BYTES])),
        code: None,
        device_name: "e2e".into(),
        platform: "test".into(),
    };
    report.check(
        "wrong_secret_rejected",
        matches!(client.pair(&wrong_secret).await, Err(ClientError::Rejected { ref code, .. }) if code == "invalid_credentials"),
        "",
    );

    // 5. Pair.
    let paired = client
        .pair(&PairRequest {
            secret: Some(link.secret.clone()),
            code: None,
            device_name: "E2E Phone".into(),
            platform: "test".into(),
        })
        .await;
    let paired = match paired {
        Ok(paired) => {
            report.ok("pair", format!("device {}", paired.device_id));
            paired
        }
        Err(error) => {
            report.fail("pair", error);
            std::process::exit(1);
        }
    };
    let token = Zeroizing::new(paired.token.clone());

    // 6. Page assets need the token.
    let anonymous = raw.get(format!("{address}/m/")).send().await;
    report.check("asset_requires_token", anonymous.as_ref().is_ok_and(|r| r.status() == 401), "");
    match client.get_asset("/m/", &token).await {
        Ok(asset) => {
            let status = asset.status;
            let csp = asset.headers.get("content-security-policy").is_some();
            let body = String::from_utf8_lossy(&asset.body);
            let page = body.contains("/src/mobile/boot.ts") || body.contains("<div id=\"app\">");
            report.check("asset_page", status == 200 && page, format!("status {status} csp {csp}"));
        }
        Err(error) => report.fail("asset_page", error),
    }

    // 7. WebSocket session.
    let (events_tx, mut events) = mpsc::unbounded_channel::<Value>();
    let connection = Connection::new(
        paired.desktop_id.clone(),
        paired.desktop_name.clone(),
        "E2E Phone".into(),
        link.addresses.clone(),
        link.fingerprint.clone(),
        token.clone(),
        |_| {},
    );
    connection.subscribe(move |message| {
        if let Ok(value) = serde_json::from_str::<Value>(&message) {
            let _ = events_tx.send(value);
        }
        true
    });
    connection.start();
    report.check("connect", wait_state(&connection, LinkState::Connected, Duration::from_secs(15)).await, format!("{:?}", connection.status()));

    let call = |method: &'static str, params: Value| {
        let connection = connection.clone();
        async move { connection.call(method, params).await.map(|json| serde_json::from_str::<Value>(&json).unwrap_or(Value::Null)) }
    };
    let has_method = |info: &Value, name: &str| {
        info["methods"].as_array().is_some_and(|methods| methods.iter().any(|method| method == name))
    };
    match call("get_device_info", json!({})).await {
        Ok(info) => report.check(
            "device_info",
            info["fullControl"] == json!(false)
                && has_method(&info, "list_dir_entries")
                && has_method(&info, "knowledge_read_scoped")
                && !has_method(&info, "git_commit"),
            format!("fullControl {} methods {}", info["fullControl"], info["methods"].as_array().map(Vec::len).unwrap_or(0)),
        ),
        Err(error) => report.fail("device_info", format!("{error:?}")),
    }
    // Changes need full control; desktop-only commands are never available.
    match connection.call("delete_session", json!({"sessionId": "x"})).await {
        Err(error) => report.check("view_tier_refuses_changes", error.code == "forbidden", error.code),
        Ok(_) => report.fail("view_tier_refuses_changes", "delete_session succeeded without full control"),
    }
    match connection.call("remote_access_get_state", json!({})).await {
        Err(error) => report.check("desktop_only_refused", error.code == "unknown_method", error.code),
        Ok(_) => report.fail("desktop_only_refused", "remote access administration reached remotely"),
    }

    // 8. A real run with the mock model, observed through forwarded events.
    let projects = call("list_project_contexts", json!({})).await.unwrap_or(Value::Null);
    let workspace = projects.as_array().into_iter().flatten().find_map(|project| {
        project["checkouts"].as_array()?.iter().find_map(|checkout| {
            let runtime = checkout.get("runtime").filter(|runtime| runtime.is_object())?;
            Some(json!({
                "checkoutId": checkout["checkoutId"],
                "expectedGeneration": runtime["workspaceGeneration"],
                "expectedMaterializationEpoch": runtime.get("materializationEpoch").cloned().unwrap_or(Value::Null),
            }))
        })
    });
    let outside = if cfg!(windows) { "C:\\Windows\\win.ini" } else { "/etc/hostname" };
    if let Some(workspace) = workspace.as_ref() {
        match call("list_dir_entries", json!({"subPath": "", "workspaceRef": workspace})).await {
            Ok(entries) => report.check("view_tier_lists_files", entries.is_array(), entries.as_array().map(Vec::len).unwrap_or(0)),
            Err(error) => report.fail("view_tier_lists_files", format!("{error:?}")),
        }
        for (step, path) in [("path_guard_absolute", outside), ("path_guard_parent", "../outside.txt")] {
            match connection.call("workspace_file_preview", json!({"filePath": path, "workspaceRef": workspace})).await {
                Err(error) => report.check(step, error.code == "forbidden", format!("{} {}", error.code, error.message)),
                Ok(_) => report.fail(step, format!("{path} was readable without full control")),
            }
        }
        // A command's own failure keeps its error value for the client.
        match connection.call("workspace_file_preview", json!({"filePath": "missing-e2e-file.txt", "workspaceRef": workspace})).await {
            Err(error) => report.check("command_error_data", error.code == "failed" && error.data.is_some(), format!("{} {:?}", error.code, error.data)),
            Ok(_) => report.fail("command_error_data", "a missing file was previewed"),
        }
        // Media previews leave the WebSocket: the result carries a blob URL
        // the app fetches over HTTPS with the device token.
        match call("workspace_file_preview", json!({"filePath": "e2e-media.png", "workspaceRef": workspace})).await {
            Ok(preview) => {
                let url = preview["dataUrl"].as_str().unwrap_or_default().to_string();
                let size = preview["size"].as_u64().unwrap_or(0);
                match url.strip_prefix(protocol::BLOB_URL_PREFIX) {
                    Some(id) => {
                        report.ok("media_offloaded", format!("{size} bytes"));
                        check_blob(&mut report, &raw, &client, &address, &token, "media_blob", id, Some(size)).await;
                    }
                    None => report.fail("media_offloaded", url.chars().take(48).collect::<String>()),
                }
            }
            Err(error) => report.fail("media_offloaded", format!("{error:?}")),
        }
        match call("resolve_markdown_image", json!({"workspaceRef": workspace, "source": "e2e-media.png"})).await {
            Ok(image) => match image["url"].as_str().and_then(|url| url.strip_prefix(protocol::BLOB_URL_PREFIX)) {
                Some(id) => check_blob(&mut report, &raw, &client, &address, &token, "markdown_image_blob", id, None).await,
                None => report.fail("markdown_image_blob", &image["url"]),
            },
            Err(error) => report.fail("markdown_image_blob", format!("{error:?}")),
        }
        let bad = client.get_asset(&format!("{}..%2Fws", protocol::BLOB_PREFIX), &token).await;
        report.check("blob_id_validated", bad.as_ref().is_ok_and(|asset| asset.status == 404), "");
    } else {
        report.skip("view_tier_lists_files", "no open workspace on the desktop");
    }

    match workspace.clone() {
        None => report.skip("chat_events", "no open workspace on the desktop"),
        Some(workspace) => {
            let _ = call("set_interests", json!({"checkouts": [workspace], "streamCheckouts": [workspace], "sessions": []})).await;
            match call("chat", json!({"workspaceRef": workspace, "text": "hello from the phone", "model": "mock/stream"})).await {
                Err(error) => report.fail("chat_launch", format!("{error:?}")),
                Ok(launch) => {
                    report.ok("chat_launch", &launch);
                    let session_id = launch["sessionId"].as_str().unwrap_or_default().to_string();
                    let _ = call("set_interests", json!({"checkouts": [workspace], "streamCheckouts": [workspace], "sessions": [session_id]})).await;
                    let mut saw_delta = false;
                    let mut saw_done = false;
                    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
                    while !saw_done {
                        let Ok(Some(message)) = tokio::time::timeout_at(deadline, events.recv()).await else {
                            break;
                        };
                        if message["kind"] != "event" {
                            continue;
                        }
                        let event = &message["event"];
                        if event["eventName"] != "stream-event" || event["payload"]["sessionId"] != json!(session_id) {
                            continue;
                        }
                        match event["payload"]["type"].as_str() {
                            Some("textDelta") => saw_delta = true,
                            Some("done") => saw_done = true,
                            Some("error") => {
                                report.fail("chat_events", &event["payload"]);
                                break;
                            }
                            _ => {}
                        }
                    }
                    report.check("chat_events", saw_delta && saw_done, format!("delta {saw_delta} done {saw_done}"));
                    match call("load_session_view", json!({"sessionId": session_id})).await {
                        Ok(view) => {
                            let count = view["session"]["messages"].as_array().map(Vec::len).unwrap_or(0);
                            report.check("load_session_view", count >= 2, format!("{count} messages"));
                        }
                        Err(error) => report.fail("load_session_view", format!("{error:?}")),
                    }
                }
            }
        }
    }

    // 9. File changes are forwarded for watched paths only; watching also
    // keeps the desktop's (on-demand) file watcher running.
    if let Some(workspace) = workspace.as_ref() {
        let _ = call("set_interests", json!({"checkouts": [workspace], "streamCheckouts": [], "sessions": []})).await;
        let watches = json!({"watches": [{"checkoutId": workspace["checkoutId"], "path": "e2e-watched.md"}]});
        match call("set_file_watches", watches).await {
            Ok(_) => {
                while events.try_recv().is_ok() {}
                println!("READY_FOR_FILE_CHANGE");
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
                let (mut watched, mut unwatched) = (false, false);
                let mut deadline = tokio::time::Instant::now() + Duration::from_secs(20);
                while let Ok(Some(message)) = tokio::time::timeout_at(deadline, events.recv()).await {
                    let event = &message["event"];
                    if message["kind"] != "event" || event["eventName"] != "workspace-file-changed" {
                        continue;
                    }
                    match event["payload"]["path"].as_str() {
                        Some("e2e-watched.md") if !watched => {
                            watched = true;
                            // An unwatched change written just before would arrive by now.
                            deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
                        }
                        Some("e2e-unwatched.md") => unwatched = true,
                        _ => {}
                    }
                }
                report.check("file_watch_forwarded", watched, "e2e-watched.md");
                report.check("file_watch_filters", !unwatched, "e2e-unwatched.md not forwarded");
            }
            Err(error) => report.fail("file_watch_forwarded", format!("{error:?}")),
        }
        let _ = call("set_file_watches", json!({"watches": []})).await;
    } else {
        report.skip("file_watch_forwarded", "no open workspace on the desktop");
    }

    // 10. Full control, granted on the desktop while connected.
    println!("READY_FOR_FULL_CONTROL");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    match call("get_device_info", json!({})).await {
        Ok(info) => report.check(
            "full_control_granted",
            info["fullControl"] == json!(true) && has_method(&info, "git_commit"),
            format!("fullControl {}", info["fullControl"]),
        ),
        Err(error) => report.fail("full_control_granted", format!("{error:?}")),
    }
    match connection.call("delete_session", json!({"sessionId": "missing-e2e-session"})).await {
        Err(error) => report.check("full_control_allows_changes", error.code != "forbidden", error.code),
        Ok(_) => report.ok("full_control_allows_changes", "deleted"),
    }
    if let Some(workspace) = workspace.as_ref() {
        match connection.call("workspace_file_preview", json!({"filePath": outside, "workspaceRef": workspace})).await {
            Err(error) => report.check("full_control_skips_path_guard", error.code != "forbidden", error.code),
            Ok(_) => report.ok("full_control_skips_path_guard", outside),
        }
    }

    // 11. Revocation closes the session for good.
    println!("READY_FOR_REVOKE");
    line.clear();
    let _ = std::io::stdin().read_line(&mut line);
    report.check(
        "revoked",
        wait_state(&connection, LinkState::Revoked, Duration::from_secs(10)).await,
        format!("{:?}", connection.status()),
    );
    let after = client.get_asset("/m/", &token).await;
    report.check("token_dead_after_revoke", after.as_ref().is_ok_and(|asset| asset.status == 401), "");
    connection.stop();

    println!("RESULT {}", if report.failures == 0 { "PASS" } else { "FAIL" });
    std::process::exit(if report.failures == 0 { 0 } else { 1 });
}
