//! Persistent MCP connection registry.
//!
//! Owns one live client per enabled server plus a synchronous wire-tool
//! snapshot the agent request path reads without awaiting. Lifecycle follows
//! the csharp_compile manager pattern (lazy ensure + explicit reconcile), not
//! a file watcher: connections change only on startup, on settings-page
//! writes, and on the agent-facing `mcp_reload` tool (Codex-style explicit
//! refresh).
//!
//! Tool-list stability: a dead connection keeps its registry entry (config +
//! last known tools) so the wire snapshot — and therefore the model-visible
//! tool list and its prompt-cache prefix — does not flap while a server is
//! down. Calls against a dead entry attempt one lazy reconnect and otherwise
//! fail with a readable error.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

use super::client::McpClient;
use super::config::{self, McpLoadMode, McpServerConfig};
use super::types::McpToolInfo;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Ping keepalive: catches servers whose process (or HTTP endpoint) is alive
/// but wedged, which otherwise only surfaces as a tool-call timeout.
const PING_INTERVAL: Duration = Duration::from_secs(30);
const PING_TIMEOUT: Duration = Duration::from_secs(10);
const PING_FAILURES_BEFORE_DEAD: u32 = 2;

/// Auto-restart crash-loop protection: give up after this many restarts
/// inside the rolling window.
const CRASH_WINDOW: Duration = Duration::from_secs(600);
const MAX_CRASHES_IN_WINDOW: usize = 5;
const RESTART_BASE_DELAY: Duration = Duration::from_secs(2);
const RESTART_MAX_DELAY: Duration = Duration::from_secs(60);

/// Debounce for tools/list_changed bursts (several servers emit one
/// notification per tool mutation).
const LIST_CHANGED_DEBOUNCE: Duration = Duration::from_millis(300);

/// Anthropic allows 128-char tool names but OpenAI-compatible backends cap at
/// 64; Locus serves both, so the wire name budget is the smaller one.
const MAX_WIRE_TOOL_NAME: usize = 64;
pub const MCP_TOOL_PREFIX: &str = "mcp__";

struct McpServerEntry {
    config: McpServerConfig,
    client: Arc<McpClient>,
    tools: Vec<McpToolInfo>,
}

/// One tool row of the synchronous snapshot; everything the request path
/// needs without touching the async registry.
#[derive(Debug, Clone)]
pub struct McpWireTool {
    pub wire_name: String,
    pub server_id: String,
    pub server_name: String,
    pub tool_name: String,
    pub description: String,
    pub input_schema: Value,
    pub load_mode: McpLoadMode,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerReport {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub error: Option<String>,
    pub tool_names: Vec<String>,
    /// Tools the server offers but the allow/deny lists hide from agents.
    pub hidden_tool_count: usize,
}

/// Result of one tools/call, split so the agent layer can attach images as
/// native content blocks (Anthropic) or the provider-specific fallback.
#[derive(Debug, Clone)]
pub struct McpCallOutcome {
    pub text: String,
    pub images: Vec<crate::session::models::ImageData>,
}

fn registry() -> &'static AsyncMutex<HashMap<String, McpServerEntry>> {
    static REGISTRY: OnceLock<AsyncMutex<HashMap<String, McpServerEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| AsyncMutex::new(HashMap::new()))
}

/// Last connection error per server id (successful connects clear it);
/// feeds the status popover so failures survive past the reconcile call.
fn last_errors() -> &'static Mutex<HashMap<String, String>> {
    static ERRORS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    ERRORS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// mcp_servers.json mtime at the last reconcile. `ensure_fresh` compares
/// against the live file so config edits that bypass both the settings
/// commands and mcp_reload (an agent writing the file via bash) still take
/// effect on the next request assembly — the Codex-style per-turn refresh.
fn last_reconciled_mtime() -> &'static Mutex<Option<std::time::SystemTime>> {
    static MTIME: OnceLock<Mutex<Option<std::time::SystemTime>>> = OnceLock::new();
    MTIME.get_or_init(|| Mutex::new(None))
}

fn ping_failures() -> &'static Mutex<HashMap<String, u32>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn crash_history() -> &'static Mutex<HashMap<String, VecDeque<Instant>>> {
    static HISTORY: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending_tool_refreshes() -> &'static Mutex<HashSet<String>> {
    static PENDING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashSet::new()))
}

static EVENT_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();

pub const MCP_STATUS_EVENT: &str = "mcp-status";

pub fn set_event_app_handle(handle: tauri::AppHandle) {
    let _ = EVENT_APP_HANDLE.set(handle);
    ensure_ping_loop();
}

/// Per-server runtime status for the chat-bar indicator: config identity
/// plus live connection facts.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerRuntimeStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub connected: bool,
    pub tools_count: usize,
    pub error: Option<String>,
}

pub async fn collect_status() -> Vec<McpServerRuntimeStatus> {
    let configs = config::load_servers();
    let entries = registry().lock().await;
    let errors = last_errors().lock().unwrap_or_else(|p| p.into_inner());
    configs
        .iter()
        .map(|server| {
            let entry = entries.get(&server.id);
            let connected = entry.map(|e| !e.client.is_dead()).unwrap_or(false);
            McpServerRuntimeStatus {
                id: server.id.clone(),
                name: server.name.clone(),
                enabled: server.enabled,
                connected: server.enabled && connected,
                tools_count: entry
                    .map(|e| {
                        e.tools
                            .iter()
                            .filter(|t| e.config.tool_exposed(&t.name))
                            .count()
                    })
                    .unwrap_or(0),
                error: if server.enabled {
                    errors.get(&server.id).cloned()
                } else {
                    None
                },
            }
        })
        .collect()
}

async fn emit_status_snapshot() {
    let Some(handle) = EVENT_APP_HANDLE.get() else {
        return;
    };
    let status = collect_status().await;
    use tauri::Emitter;
    let _ = handle.emit(MCP_STATUS_EVENT, status);
}

fn wire_snapshot() -> &'static Mutex<Vec<McpWireTool>> {
    static SNAPSHOT: OnceLock<Mutex<Vec<McpWireTool>>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| Mutex::new(Vec::new()))
}

// ── Wire tool names ─────────────────────────────────────────────────────

/// Stable FNV-1a so truncated-name hashes survive process restarts
/// (std's DefaultHasher makes no cross-version guarantee).
fn fnv1a(input: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn sanitize_tool_segment(segment: &str) -> String {
    let cleaned: String = segment
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "tool".to_string()
    } else {
        cleaned
    }
}

/// `mcp__<server>__<tool>`, sanitized to `[A-Za-z0-9_-]` and capped at 64
/// chars (over-long names keep a stable 8-hex-digit FNV suffix). Collisions
/// after truncation are disambiguated by the caller via the snapshot map.
pub fn wire_tool_name(server_id: &str, tool_name: &str) -> String {
    let full = format!(
        "{MCP_TOOL_PREFIX}{}__{}",
        sanitize_tool_segment(server_id),
        sanitize_tool_segment(tool_name)
    );
    if full.len() <= MAX_WIRE_TOOL_NAME {
        return full;
    }
    let hash = format!("{:08x}", fnv1a(&full) as u32);
    let keep = MAX_WIRE_TOOL_NAME - hash.len() - 1;
    format!("{}_{hash}", &full[..keep])
}

// ── Synchronous snapshot readers (agent request path) ───────────────────

pub fn wire_tool_names() -> Vec<String> {
    let snapshot = wire_snapshot().lock().unwrap_or_else(|p| p.into_inner());
    snapshot.iter().map(|t| t.wire_name.clone()).collect()
}

pub fn resolve_wire_tool(name: &str) -> Option<McpWireTool> {
    let snapshot = wire_snapshot().lock().unwrap_or_else(|p| p.into_inner());
    snapshot.iter().find(|t| t.wire_name == name).cloned()
}

/// Wire names currently exposed for one server (drives the settings page's
/// per-server approval bulk action).
pub fn wire_tool_names_for_server(server_id: &str) -> Vec<String> {
    let snapshot = wire_snapshot().lock().unwrap_or_else(|p| p.into_inner());
    snapshot
        .iter()
        .filter(|t| t.server_id == server_id)
        .map(|t| t.wire_name.clone())
        .collect()
}

/// OpenAI-style function JSON for one MCP tool; `None` when the name is not
/// in the snapshot. The description is prefixed with the server origin so
/// the model can tell external tools apart.
pub fn resolve_api_tool_json(name: &str) -> Option<Value> {
    let tool = resolve_wire_tool(name)?;
    Some(json!({
        "type": "function",
        "function": {
            "name": tool.wire_name,
            "description": mcp_tool_description_text(&tool),
            "parameters": tool.input_schema,
        }
    }))
}

pub fn resolve_tool_description(name: &str) -> Option<(String, Value)> {
    let tool = resolve_wire_tool(name)?;
    Some((mcp_tool_description_text(&tool), tool.input_schema))
}

fn mcp_tool_description_text(tool: &McpWireTool) -> String {
    if tool.description.is_empty() {
        format!("[MCP: {}] External MCP server tool.", tool.server_name)
    } else {
        format!("[MCP: {}] {}", tool.server_name, tool.description)
    }
}

fn rebuild_wire_snapshot(entries: &HashMap<String, McpServerEntry>) {
    let mut rows = Vec::new();
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut ids: Vec<&String> = entries.keys().collect();
    ids.sort();
    for id in ids {
        let entry = &entries[id];
        for tool in &entry.tools {
            if !entry.config.tool_exposed(&tool.name) {
                continue;
            }
            let mut wire_name = wire_tool_name(id, &tool.name);
            // Post-truncation collisions are vanishingly rare; keep them
            // deterministic by appending a counter.
            if let Some(count) = taken.get_mut(&wire_name) {
                *count += 1;
                wire_name = format!("{}_{}", &wire_name[..wire_name.len().min(60)], count);
            } else {
                taken.insert(wire_name.clone(), 1);
            }
            rows.push(McpWireTool {
                wire_name,
                server_id: id.clone(),
                server_name: entry.config.name.clone(),
                tool_name: tool.name.clone(),
                description: tool.description.clone().unwrap_or_default(),
                input_schema: tool
                    .input_schema
                    .clone()
                    .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                load_mode: entry.config.load_mode,
            });
        }
    }
    let mut snapshot = wire_snapshot().lock().unwrap_or_else(|p| p.into_inner());
    *snapshot = rows;
}

// ── Connection lifecycle ────────────────────────────────────────────────

async fn connect_entry(config: &McpServerConfig) -> Result<McpServerEntry, String> {
    let client = McpClient::connect(config).await?;
    let handshake = super::run_handshake_and_list(&client).await;
    match handshake {
        Ok((_, tools)) => Ok(McpServerEntry {
            config: config.clone(),
            client: Arc::new(client),
            tools,
        }),
        Err(e) => {
            client.shutdown().await;
            Err(e)
        }
    }
}

/// True when a config change requires dropping and re-spawning the
/// connection; display-level fields (name, timeout, allow/deny, load mode,
/// auto-restart) apply in place.
fn requires_reconnect(a: &McpServerConfig, b: &McpServerConfig) -> bool {
    a.transport != b.transport
        || a.command != b.command
        || a.args != b.args
        || a.env != b.env
        || a.cwd != b.cwd
        || a.url != b.url
        || a.headers != b.headers
}

fn report_for_entry(entry: &McpServerEntry) -> McpServerReport {
    let exposed: Vec<String> = entry
        .tools
        .iter()
        .filter(|t| entry.config.tool_exposed(&t.name))
        .map(|t| t.name.clone())
        .collect();
    McpServerReport {
        id: entry.config.id.clone(),
        name: entry.config.name.clone(),
        connected: true,
        error: None,
        hidden_tool_count: entry.tools.len() - exposed.len(),
        tool_names: exposed,
    }
}

/// Reconciles live connections against `mcp_servers.json`: connects new or
/// changed enabled servers, drops removed or disabled ones, keeps healthy
/// unchanged ones, and reports the outcome per enabled server.
pub async fn reconcile() -> Vec<McpServerReport> {
    // Snapshot the mtime before reading: if the file changes mid-reconcile,
    // the next ensure_fresh still sees a difference and re-runs.
    let mtime = config::config_file_mtime();
    let desired = config::load_servers();
    let mut reports = Vec::new();
    let mut entries = registry().lock().await;
    *last_reconciled_mtime()
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = mtime;

    let desired_ids: Vec<&str> = desired
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.id.as_str())
        .collect();
    let stale: Vec<String> = entries
        .keys()
        .filter(|id| !desired_ids.contains(&id.as_str()))
        .cloned()
        .collect();
    for id in stale {
        if let Some(entry) = entries.remove(&id) {
            entry.client.shutdown().await;
        }
    }

    for server in desired.iter().filter(|s| s.enabled) {
        let existing = entries.get(&server.id);
        let reusable = existing
            .map(|entry| !requires_reconnect(&entry.config, server) && !entry.client.is_dead())
            .unwrap_or(false);
        if reusable {
            // Refresh display-level config so timeouts and allow/deny lists
            // apply immediately.
            let entry = entries.get_mut(&server.id).expect("checked above");
            entry.config = server.clone();
            last_errors()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&server.id);
            reports.push(report_for_entry(entry));
            continue;
        }
        if let Some(old) = entries.remove(&server.id) {
            old.client.shutdown().await;
        }
        match connect_entry(server).await {
            Ok(entry) => {
                arm_auto_restart(&entry);
                reports.push(report_for_entry(&entry));
                entries.insert(server.id.clone(), entry);
                last_errors()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&server.id);
            }
            Err(e) => {
                last_errors()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(server.id.clone(), e.clone());
                reports.push(McpServerReport {
                    id: server.id.clone(),
                    name: server.name.clone(),
                    connected: false,
                    error: Some(e),
                    tool_names: Vec::new(),
                    hidden_tool_count: 0,
                });
            }
        }
    }

    rebuild_wire_snapshot(&entries);
    drop(entries);
    emit_status_snapshot().await;
    reports
}

/// Cheap staleness backstop on the request-assembly path: reconciles only
/// when mcp_servers.json changed since the last reconcile. Normal requests
/// pay one fs metadata stat; a detected change blocks assembly until the
/// connections match the file — exactly what an agent that just wrote the
/// config expects.
pub async fn ensure_fresh() {
    let current = config::config_file_mtime();
    {
        let last = last_reconciled_mtime()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if *last == current {
            return;
        }
    }
    let _ = reconcile().await;
}

/// Calls one MCP tool by wire name. A dead connection gets one lazy restart
/// attempt (ensure semantics) before the call fails; the error text reaches
/// the model, which can retry or route around it. `cancel` aborts the wait,
/// sends `notifications/cancelled`, and yields the MCP_CALL_CANCELLED
/// marker error.
pub async fn call_tool(
    wire_name: &str,
    arguments: Value,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<McpCallOutcome, String> {
    let target = resolve_wire_tool(wire_name)
        .ok_or_else(|| format!("Unknown MCP tool '{wire_name}'. Run mcp_reload to refresh the tool list."))?;

    let mut restarted = false;
    let acquire = {
        let mut entries = registry().lock().await;
        let entry = entries.get(&target.server_id).ok_or_else(|| {
            format!(
                "MCP server '{}' is not connected. Run mcp_reload to reconnect.",
                target.server_name
            )
        })?;
        if entry.client.is_dead() {
            restarted = true;
            let config = entry.config.clone();
            eprintln!(
                "[Mcp:{}] connection is down; attempting lazy restart",
                config.id
            );
            entry.client.kill_for_exit();
            match connect_entry(&config).await {
                Ok(new_entry) => {
                    arm_auto_restart(&new_entry);
                    entries.insert(config.id.clone(), new_entry);
                    rebuild_wire_snapshot(&entries);
                    last_errors()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&config.id);
                }
                Err(e) => {
                    // Keep the dead entry: the cached tool list stays in the
                    // model-visible snapshot (no flapping) and the next call
                    // retries the restart.
                    last_errors()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(config.id.clone(), e.clone());
                    drop(entries);
                    emit_status_snapshot().await;
                    return Err(format!(
                        "MCP server '{}' is disconnected and could not be restarted: {e}",
                        target.server_name
                    ));
                }
            }
        }
        let entry = entries.get(&target.server_id).expect("just ensured");
        (
            entry.client.clone(),
            Duration::from_millis(entry.config.call_timeout_ms),
            target.tool_name.clone(),
        )
    };
    if restarted {
        emit_status_snapshot().await;
    }
    let (client, timeout, tool_name) = acquire;

    let result = client
        .request_cancellable(
            "tools/call",
            Some(json!({ "name": tool_name, "arguments": arguments })),
            timeout,
            cancel,
        )
        .await?;
    render_tool_call_result(&result)
}

/// Flattens an MCP tools/call result into agent-facing text plus image
/// attachments. Text content concatenates; images ride the session's native
/// image channel (Anthropic tool_result blocks / OpenAI follow-up content);
/// `structuredContent` is appended as JSON when present.
fn render_tool_call_result(result: &Value) -> Result<McpCallOutcome, String> {
    let mut parts: Vec<String> = Vec::new();
    let mut images: Vec<crate::session::models::ImageData> = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
                Some("image") => {
                    let data = item.get("data").and_then(Value::as_str).unwrap_or_default();
                    if data.is_empty() {
                        parts.push("[MCP tool returned an empty image]".to_string());
                    } else {
                        let mime_type = item
                            .get("mimeType")
                            .and_then(Value::as_str)
                            .filter(|m| !m.trim().is_empty())
                            .unwrap_or("image/png")
                            .to_string();
                        images.push(crate::session::models::ImageData {
                            data: data.to_string(),
                            mime_type,
                        });
                        parts.push(format!("[image {} attached]", images.len()));
                    }
                }
                Some("audio") => parts.push("[audio returned by MCP tool; not audible to you]".to_string()),
                Some(other) => parts.push(format!("[{other} content returned by MCP tool]")),
                None => {}
            }
        }
    }
    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            parts.push(format!(
                "structuredContent: {}",
                serde_json::to_string_pretty(structured).unwrap_or_default()
            ));
        }
    }
    let text = parts.join("\n");
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        Err(if text.is_empty() {
            "MCP tool reported an error without details".to_string()
        } else {
            text
        })
    } else {
        Ok(McpCallOutcome { text, images })
    }
}

/// Replaces image attachments with the explicit not-visible placeholder for
/// backends that cannot see images (vision-less OpenAI-compatible models).
pub fn describe_images_as_placeholder(outcome: &McpCallOutcome) -> String {
    let mut text = outcome.text.clone();
    for (index, _) in outcome.images.iter().enumerate() {
        let marker = format!("[image {} attached]", index + 1);
        let placeholder = "[The MCP tool returned an image, but it is NOT visible to you on this channel. Verify through a text-returning tool instead of describing the image.]";
        if text.contains(&marker) {
            text = text.replace(&marker, placeholder);
        } else {
            text.push_str("\n");
            text.push_str(placeholder);
        }
    }
    text
}

// ── Server-initiated notifications ──────────────────────────────────────

/// Entry point for notifications observed by any transport's read loop.
/// tools/list_changed schedules a debounced re-list; everything else is
/// logged and dropped (the tools-only capability set never subscribes to
/// resources/prompts).
pub(crate) fn handle_server_notification(server_id: &str, method: &str, _params: Option<&Value>) {
    if method == "notifications/tools/list_changed" {
        eprintln!("[Mcp:{server_id}] tools/list_changed received; scheduling refresh");
        schedule_tools_refresh(server_id.to_string());
    } else {
        eprintln!("[Mcp:{server_id}] notification: {method}");
    }
}

fn schedule_tools_refresh(server_id: String) {
    {
        let mut pending = pending_tool_refreshes()
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !pending.insert(server_id.clone()) {
            return;
        }
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LIST_CHANGED_DEBOUNCE).await;
        pending_tool_refreshes()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&server_id);
        refresh_server_tools(&server_id).await;
    });
}

/// Re-lists one server's tools and swaps them into the registry when the
/// connection is still the same one the notification came from.
async fn refresh_server_tools(server_id: &str) {
    let client = {
        let entries = registry().lock().await;
        let Some(entry) = entries.get(server_id) else {
            return;
        };
        if entry.client.is_dead() {
            return;
        }
        entry.client.clone()
    };
    match super::list_all_tools(&client).await {
        Ok(tools) => {
            let mut entries = registry().lock().await;
            let Some(entry) = entries.get_mut(server_id) else {
                return;
            };
            if !Arc::ptr_eq(&entry.client, &client) {
                // Reconnected while we were listing; the reconnect already
                // brought a fresh list.
                return;
            }
            eprintln!(
                "[Mcp:{server_id}] tools/list_changed reconciled: {} tools",
                tools.len()
            );
            entry.tools = tools;
            rebuild_wire_snapshot(&entries);
            drop(entries);
            emit_status_snapshot().await;
        }
        Err(e) => {
            eprintln!("[Mcp:{server_id}] tools/list after list_changed failed: {e}");
        }
    }
}

// ── Ping keepalive ──────────────────────────────────────────────────────

fn ensure_ping_loop() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(PING_INTERVAL).await;
            ping_round().await;
        }
    });
}

async fn ping_round() {
    let targets: Vec<(String, Arc<McpClient>)> = {
        let entries = registry().lock().await;
        entries
            .iter()
            .filter(|(_, e)| !e.client.is_dead())
            .map(|(id, e)| (id.clone(), e.client.clone()))
            .collect()
    };
    for (id, client) in targets {
        match client.request("ping", None, PING_TIMEOUT).await {
            Ok(_) => {
                ping_failures()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
            }
            Err(e) => {
                if client.is_dead() {
                    // Process exit already surfaced through its own path.
                    continue;
                }
                let strikes = {
                    let mut failures = ping_failures()
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    let entry = failures.entry(id.clone()).or_insert(0);
                    *entry += 1;
                    *entry
                };
                eprintln!("[Mcp:{id}] ping failed ({strikes}/{PING_FAILURES_BEFORE_DEAD}): {e}");
                if strikes < PING_FAILURES_BEFORE_DEAD {
                    continue;
                }
                ping_failures()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                // Tear the wedged connection down (auto-restart watchers see
                // this as a crash); the entry and its cached tools stay for
                // the lazy per-call restart.
                let entries = registry().lock().await;
                if let Some(entry) = entries.get(&id) {
                    if Arc::ptr_eq(&entry.client, &client) {
                        entry.client.abandon_unresponsive();
                        last_errors()
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(
                                id.clone(),
                                format!(
                                    "Server stopped answering pings ({PING_FAILURES_BEFORE_DEAD} timeouts); it will be restarted on the next tool call"
                                ),
                            );
                    }
                }
                drop(entries);
                emit_status_snapshot().await;
            }
        }
    }
}

// ── stdio auto-restart ──────────────────────────────────────────────────

/// Registers a crash and reports how many landed inside the rolling window.
fn register_crash(server_id: &str) -> usize {
    let mut history = crash_history().lock().unwrap_or_else(|p| p.into_inner());
    let entries = history.entry(server_id.to_string()).or_default();
    let now = Instant::now();
    entries.push_back(now);
    while let Some(front) = entries.front() {
        if now.duration_since(*front) > CRASH_WINDOW {
            entries.pop_front();
        } else {
            break;
        }
    }
    entries.len()
}

/// Watches one stdio connection for an unexpected exit and drives the
/// opt-in backoff restart. No-op for HTTP servers and when autoRestart is
/// off (the default: the lazy per-call restart plus manual reconnect).
fn arm_auto_restart(entry: &McpServerEntry) {
    if !entry.config.auto_restart {
        return;
    }
    if entry.client.as_stdio().is_none() {
        return;
    }
    let server_id = entry.config.id.clone();
    let client = entry.client.clone();
    tauri::async_runtime::spawn(async move {
        {
            let Some(stdio) = client.as_stdio() else {
                return;
            };
            stdio.wait_exited().await;
            if stdio.exit_was_expected() {
                return;
            }
        }
        auto_restart_loop(server_id, client).await;
    });
}

async fn auto_restart_loop(server_id: String, dead_client: Arc<McpClient>) {
    loop {
        let crashes = register_crash(&server_id);
        if crashes > MAX_CRASHES_IN_WINDOW {
            eprintln!(
                "[Mcp:{server_id}] crash loop: {crashes} restarts in {}s; giving up on auto-restart",
                CRASH_WINDOW.as_secs()
            );
            last_errors()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(
                    server_id.clone(),
                    format!(
                        "Server keeps crashing ({} restarts in {} minutes); auto-restart paused. Fix the server and reconnect manually.",
                        crashes,
                        CRASH_WINDOW.as_secs() / 60
                    ),
                );
            emit_status_snapshot().await;
            return;
        }
        let exponent = crashes.saturating_sub(1).min(31) as u32;
        let delay = RESTART_BASE_DELAY
            .saturating_mul(2u32.saturating_pow(exponent))
            .min(RESTART_MAX_DELAY);
        eprintln!(
            "[Mcp:{server_id}] server exited unexpectedly; auto-restarting in {}s (attempt {crashes}/{MAX_CRASHES_IN_WINDOW})",
            delay.as_secs()
        );
        tokio::time::sleep(delay).await;

        let mut entries = registry().lock().await;
        let Some(entry) = entries.get(&server_id) else {
            return; // Removed or disabled meanwhile.
        };
        if !Arc::ptr_eq(&entry.client, &dead_client) {
            return; // Someone else (lazy restart, reconcile) already replaced it.
        }
        if !entry.config.auto_restart || !entry.config.enabled {
            return;
        }
        let config = entry.config.clone();
        match connect_entry(&config).await {
            Ok(new_entry) => {
                arm_auto_restart(&new_entry);
                entries.insert(server_id.clone(), new_entry);
                rebuild_wire_snapshot(&entries);
                last_errors()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&server_id);
                drop(entries);
                eprintln!("[Mcp:{server_id}] auto-restart succeeded");
                emit_status_snapshot().await;
                return;
            }
            Err(e) => {
                eprintln!("[Mcp:{server_id}] auto-restart failed: {e}");
                last_errors()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(server_id.clone(), format!("Auto-restart failed: {e}"));
                drop(entries);
                emit_status_snapshot().await;
                // Loop into the next backoff attempt.
            }
        }
    }
}

/// Formats reconcile reports for the mcp_reload tool output.
pub fn format_reports(reports: &[McpServerReport]) -> String {
    if reports.is_empty() {
        return "No enabled MCP servers configured. Add one in Settings → MCP Servers or by editing mcp_servers.json.".to_string();
    }
    let mut lines = Vec::new();
    let mut any_connected = false;
    for report in reports {
        if report.connected {
            any_connected = true;
            lines.push(format!(
                "✓ {} ({}): connected, {} tools",
                report.name,
                report.id,
                report.tool_names.len()
            ));
            for tool in &report.tool_names {
                lines.push(format!("    {}", wire_tool_name(&report.id, tool)));
            }
            if report.hidden_tool_count > 0 {
                lines.push(format!(
                    "    ({} more tools hidden by this server's allow/deny lists)",
                    report.hidden_tool_count
                ));
            }
        } else {
            lines.push(format!(
                "✗ {} ({}): {}",
                report.name,
                report.id,
                report.error.as_deref().unwrap_or("connection failed")
            ));
        }
    }
    if any_connected {
        lines.push(
            "MCP tools are lazy-loaded: call one directly if it is already visible to you; otherwise load it first via tool_load / tool_search using the exact wire name above.".to_string(),
        );
    }
    lines.join("\n")
}

/// Best-effort synchronous teardown for app exit (mirrors
/// csharp_compile::kill_active_server_for_exit).
pub fn kill_all_for_exit() {
    if let Ok(mut entries) = registry().try_lock() {
        for (_, entry) in entries.drain() {
            entry.client.kill_for_exit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_tool_name_prefixes_and_sanitizes() {
        assert_eq!(wire_tool_name("blender", "get_scene_info"), "mcp__blender__get_scene_info");
        assert_eq!(wire_tool_name("a b", "x.y"), "mcp__a_b__x_y");
    }

    #[test]
    fn wire_tool_name_caps_at_openai_limit_with_stable_hash() {
        let long = "a".repeat(80);
        let name = wire_tool_name("server", &long);
        assert_eq!(name.len(), 64);
        assert!(name.starts_with("mcp__server__"));
        // Stable across calls (FNV, not DefaultHasher).
        assert_eq!(name, wire_tool_name("server", &long));
    }

    #[test]
    fn render_tool_call_result_maps_content_and_errors() {
        let ok = serde_json::json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image", "data": "aGk=", "mimeType": "image/png"}
            ]
        });
        let rendered = render_tool_call_result(&ok).unwrap();
        assert!(rendered.text.starts_with("hello\n[image 1 attached]"));
        assert_eq!(rendered.images.len(), 1);
        assert_eq!(rendered.images[0].mime_type, "image/png");
        assert_eq!(rendered.images[0].data, "aGk=");

        let err = serde_json::json!({
            "isError": true,
            "content": [{"type": "text", "text": "Could not connect to Blender"}]
        });
        assert_eq!(
            render_tool_call_result(&err).unwrap_err(),
            "Could not connect to Blender"
        );

        let structured = serde_json::json!({
            "content": [],
            "structuredContent": {"count": 3}
        });
        assert!(render_tool_call_result(&structured)
            .unwrap()
            .text
            .contains("\"count\": 3"));
    }

    #[test]
    fn image_placeholder_swaps_markers_for_visionless_backends() {
        let outcome = McpCallOutcome {
            text: "before\n[image 1 attached]".to_string(),
            images: vec![crate::session::models::ImageData {
                data: "aGk=".to_string(),
                mime_type: "image/png".to_string(),
            }],
        };
        let text = describe_images_as_placeholder(&outcome);
        assert!(!text.contains("[image 1 attached]"));
        assert!(text.contains("NOT visible"));
    }

    #[test]
    fn crash_window_counts_recent_restarts() {
        assert_eq!(register_crash("crash-test-server"), 1);
        assert_eq!(register_crash("crash-test-server"), 2);
    }

}
