use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Instant;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::session::models::ToolCallInfo;
use crate::tool::ToolResult;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn claude_session_map() -> &'static tokio::sync::Mutex<HashMap<String, String>> {
    static STORE: OnceLock<tokio::sync::Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

pub async fn cached_session_id(locus_session_id: &str) -> Option<String> {
    claude_session_map()
        .lock()
        .await
        .get(locus_session_id)
        .cloned()
}

pub async fn store_session_id(locus_session_id: &str, claude_session_id: &str) {
    if locus_session_id.is_empty() || claude_session_id.is_empty() {
        return;
    }
    claude_session_map()
        .lock()
        .await
        .insert(locus_session_id.to_string(), claude_session_id.to_string());
}

#[derive(Debug, Clone)]
pub struct ClaudeSdkToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeSdkOptions {
    pub locus_session_id: String,
    pub cwd: String,
    pub system_prompt: String,
    pub model: String,
    pub resume_session_id: Option<String>,
    pub server_name: String,
    pub tools: Vec<ClaudeSdkToolDefinition>,
    pub debug: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeSdkAssistantMessage {
    pub text: String,
    pub tool_calls: Vec<ToolCallInfo>,
    pub thinking_text: String,
    pub thinking_signature: String,
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeTurnResult {
    pub final_text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_write_tokens: u32,
    pub cost_usd: f64,
    pub raw_request: String,
    pub raw_response: String,
    pub claude_session_id: Option<String>,
}

pub type ClaudeSdkHostFuture<'a> = Pin<Box<dyn Future<Output = ToolResult> + Send + 'a>>;

pub trait ClaudeSdkHost {
    fn on_text_delta(&mut self, delta: String);
    fn on_thinking_delta(&mut self, delta: String);
    fn on_tool_call_start(&mut self, tool_call_id: String, tool_name: String);
    fn on_assistant_message(&mut self, message: ClaudeSdkAssistantMessage) -> Result<(), String>;
    fn execute_tool<'a>(
        &'a mut self,
        request_id: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> ClaudeSdkHostFuture<'a>;
}

pub fn claude_cli_status() -> (bool, String) {
    match find_claude_cli() {
        Some(path) => (true, path.display().to_string()),
        None => (false, String::new()),
    }
}

pub fn find_claude_cli() -> Option<PathBuf> {
    for candidate in path_candidates().into_iter().chain(extra_candidates()) {
        if candidate.is_file() && is_usable_claude_cli(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub async fn run_turn<H: ClaudeSdkHost>(
    options: ClaudeCodeSdkOptions,
    user_message: serde_json::Value,
    host: &mut H,
) -> Result<ClaudeCodeTurnResult, String> {
    let cli_path = find_claude_cli().ok_or_else(|| {
        "Claude Code CLI not found. Install `@anthropic-ai/claude-code` and ensure `claude` is available in PATH.".to_string()
    })?;

    // Decide whether we can swap the program for `bun --require <hook> <cli.js>`
    // (only viable when debug mode is on AND we can locate bun + cli.js + write the
    // hook file). Otherwise spawn the original `claude.exe` shim and rely on
    // `NODE_OPTIONS=--require` (which works for npm/node-based installs but is
    // silently ignored by bun-installed shims).
    let mut bun_preload_active: Option<(BunPreloadLayout, PathBuf)> = None;
    if options.debug {
        if let Some(layout) = try_bun_preload_layout(&cli_path) {
            if let Some(hook_path) = install_http_hook() {
                eprintln!(
                    "[Claude SDK debug] bun preload layout detected: bun={}, cli.js={}",
                    layout.bun_exe.display(),
                    layout.cli_js.display()
                );
                bun_preload_active = Some((layout, hook_path));
            }
        } else {
            eprintln!("[Claude SDK debug] bun preload layout NOT detected — falling back to NODE_OPTIONS=--require (only works for node-based claude shims)");
        }
    }

    let mut cmd = if let Some((ref layout, ref hook_path)) = bun_preload_active {
        let mut c = tokio::process::Command::new(&layout.bun_exe);
        c.arg("--require").arg(hook_path).arg(&layout.cli_js);
        c
    } else {
        tokio::process::Command::new(&cli_path)
    };
    cmd.kill_on_drop(true);
    cmd.current_dir(&options.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("--print")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--system-prompt")
        .arg(options.system_prompt.clone())
        .arg("--permission-mode")
        .arg("bypassPermissions");

    let normalized_model = normalize_claude_model(&options.model);
    if !normalized_model.is_empty() {
        cmd.arg("--model").arg(normalized_model);
    }

    if let Some(resume_session_id) = options.resume_session_id.as_deref() {
        if !resume_session_id.trim().is_empty() {
            cmd.arg("--resume").arg(resume_session_id.trim());
        }
    }

    let mut envs: HashMap<OsString, OsString> = std::env::vars_os().collect();
    envs.entry(OsString::from("CLAUDE_CODE_ENTRYPOINT"))
        .or_insert_with(|| OsString::from("sdk-rs"));
    envs.entry(OsString::from("LOCUS_SESSION_ID"))
        .or_insert_with(|| OsString::from(options.locus_session_id.clone()));
    crate::network::extend_proxy_env_map(&mut envs);

    if options.debug {
        let abs_dir = sdk_debug_dir();
        eprintln!(
            "[Claude SDK debug] LOCUS_DEBUG_DIR -> {}",
            abs_dir.display()
        );
        envs.insert(
            OsString::from("LOCUS_DEBUG_DIR"),
            OsString::from(abs_dir.to_string_lossy().to_string()),
        );

        if bun_preload_active.is_some() {
            // Hook is loaded via the explicit `bun --require <hook>` arg added when
            // building cmd above. Nothing more to do for the env.
            eprintln!("[Claude SDK debug] hook injection mode = bun --require (explicit cli flag)");
        } else {
            // Fallback for non-bun installs: try NODE_OPTIONS=--require, which works
            // for npm/node-based shims but is silently ignored by bun.
            match install_http_hook() {
                Some(hook_path) => {
                    eprintln!(
                        "[Claude SDK debug] hook script written to {}",
                        hook_path.display()
                    );
                    let hook_str = hook_path.to_string_lossy().to_string();
                    let require_arg = if hook_str.contains(char::is_whitespace) {
                        format!("--require \"{}\"", hook_str)
                    } else {
                        format!("--require {}", hook_str)
                    };
                    let combined = match envs.get(&OsString::from("NODE_OPTIONS")) {
                        Some(existing) => {
                            let existing_str = existing.to_string_lossy().to_string();
                            if existing_str.is_empty() {
                                require_arg
                            } else {
                                format!("{} {}", require_arg, existing_str)
                            }
                        }
                        None => require_arg,
                    };
                    eprintln!(
                        "[Claude SDK debug] hook injection mode = NODE_OPTIONS={}",
                        combined
                    );
                    envs.insert(OsString::from("NODE_OPTIONS"), OsString::from(combined));
                }
                None => {
                    eprintln!(
                        "[Claude SDK debug] install_http_hook returned None — hook not injected"
                    );
                }
            }
        }
    }

    cmd.envs(envs);

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    if options.debug {
        save_debug_request(&options, &cmd, &user_message);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start Claude Code CLI: {}", e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Claude Code CLI stdout unavailable".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Claude Code CLI stderr unavailable".to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Claude Code CLI stdin unavailable".to_string())?;

    let stream_stderr = options.debug;
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if stream_stderr {
            let mut reader = BufReader::new(stderr).lines();
            loop {
                match reader.next_line().await {
                    Ok(Some(line)) => {
                        eprintln!("[Claude SDK stderr] {}", line);
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("[Claude SDK stderr] read error: {}", e);
                        break;
                    }
                }
            }
        } else {
            let mut reader = BufReader::new(stderr);
            let _ = reader.read_to_string(&mut buf).await;
        }
        buf
    });

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut raw_request = String::new();
    let mut raw_response = String::new();
    let init_request_id = format!("req_init_{}", uuid::Uuid::new_v4());
    let init_request = json!({
        "subtype": "initialize",
        "sdkMcpServers": [options.server_name],
    });
    write_json_line(
        &mut stdin,
        &json!({
            "type": "control_request",
            "request_id": init_request_id,
            "request": init_request,
        }),
        &mut raw_request,
    )
    .await?;

    let mut init_done = false;
    let mut prompt_sent = false;
    let mut saw_result = false;
    let mut result = ClaudeCodeTurnResult::default();
    let mut thinking_started_at: Option<Instant> = None;

    while let Some(line) = stdout_lines
        .next_line()
        .await
        .map_err(|e| format!("Failed reading Claude Code output: {}", e))?
    {
        if line.trim().is_empty() {
            continue;
        }

        raw_response.push_str(&line);
        raw_response.push('\n');

        let message: serde_json::Value = serde_json::from_str(&line)
            .map_err(|e| format!("Invalid Claude Code stream JSON: {}\nline={}", e, line))?;

        if let Some(session_id) = message.get("session_id").and_then(|v| v.as_str()) {
            if !session_id.is_empty() {
                result.claude_session_id = Some(session_id.to_string());
            }
        }

        match message
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
        {
            "control_response" => {
                let response = message
                    .get("response")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| "Claude Code control_response missing response".to_string())?;
                let request_id = response
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let subtype = response
                    .get("subtype")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();

                if request_id == init_request_id {
                    if subtype == "success" {
                        init_done = true;
                    } else {
                        let error = response
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown initialize error");
                        return Err(format!("Claude Code initialize failed: {}", error));
                    }
                }
            }
            "control_request" => {
                handle_control_request(
                    &mut stdin,
                    &message,
                    host,
                    &options.tools,
                    &mut raw_request,
                )
                .await?;
            }
            "stream_event" => {
                handle_stream_event(&message, host, &mut thinking_started_at);
            }
            "assistant" => {
                if let Some(parsed) = parse_assistant_message(&message) {
                    host.on_assistant_message(parsed)?;
                }
            }
            "result" => {
                if thinking_started_at.is_some() && result.output_tokens == 0 {
                    let _ = thinking_started_at.take();
                }
                saw_result = true;
                parse_result_message(&message, &mut result)?;
                break;
            }
            "system" | "user" | "rate_limit_event" | "tool_progress" | "auth_status"
            | "tool_use_summary" | "keep_alive" => {}
            other => {
                if options.debug {
                    eprintln!("[Claude SDK] ignoring message type '{}': {}", other, line);
                }
            }
        }

        if init_done && !prompt_sent {
            write_json_line(
                &mut stdin,
                &json!({
                    "type": "user",
                    "session_id": "",
                    "message": user_message,
                    "parent_tool_use_id": serde_json::Value::Null,
                }),
                &mut raw_request,
            )
            .await?;
            prompt_sent = true;
        }
    }

    // Close stdin so the CLI sees EOF and exits — without this, --print + stream-json
    // input mode keeps the child alive forever waiting for more input, and child.wait()
    // below blocks indefinitely.
    drop(stdin);

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Failed waiting for Claude Code CLI exit: {}", e))?;
    let stderr_output = stderr_task.await.unwrap_or_default();

    if options.debug {
        save_debug_response(&options, &raw_response, &stderr_output, status.code());
    }

    result.raw_request = raw_request;
    result.raw_response = raw_response;

    if !saw_result {
        let detail = stderr_output.trim();
        if detail.is_empty() {
            return Err("Claude Code CLI ended without a result message".to_string());
        }
        return Err(format!(
            "Claude Code CLI ended without a result message: {}",
            detail
        ));
    }

    if !status.success() && result.final_text.is_empty() {
        let detail = stderr_output.trim();
        if detail.is_empty() {
            return Err(format!("Claude Code CLI exited with status {}", status));
        }
        return Err(format!(
            "Claude Code CLI exited with status {}: {}",
            status, detail
        ));
    }

    Ok(result)
}

fn parse_result_message(
    message: &serde_json::Value,
    out: &mut ClaudeCodeTurnResult,
) -> Result<(), String> {
    let subtype = message
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    if subtype != "success" {
        let errors = message
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Claude Code returned an error result".to_string());
        return Err(errors);
    }

    out.final_text = message
        .get("result")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    out.cost_usd = message
        .get("total_cost_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if let Some(usage) = message.get("usage").and_then(|v| v.as_object()) {
        out.input_tokens = usage_u32(usage, &["input_tokens", "inputTokens"]);
        out.output_tokens = usage_u32(usage, &["output_tokens", "outputTokens"]);
        out.cache_read_tokens =
            usage_u32(usage, &["cache_read_input_tokens", "cacheReadInputTokens"]);
        out.cache_write_tokens = usage_u32(
            usage,
            &["cache_creation_input_tokens", "cacheCreationInputTokens"],
        );
    }

    Ok(())
}

fn usage_u32(usage: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> u32 {
    for key in keys {
        if let Some(value) = usage.get(*key) {
            if let Some(n) = value.as_u64() {
                return n.min(u32::MAX as u64) as u32;
            }
            if let Some(n) = value.as_i64() {
                return n.max(0).min(u32::MAX as i64) as u32;
            }
        }
    }
    0
}

fn handle_stream_event<H: ClaudeSdkHost>(
    message: &serde_json::Value,
    host: &mut H,
    thinking_started_at: &mut Option<Instant>,
) {
    let Some(event) = message.get("event") else {
        return;
    };
    let Some(event_type) = event.get("type").and_then(|v| v.as_str()) else {
        return;
    };

    match event_type {
        "content_block_start" => {
            let Some(block) = event.get("content_block").and_then(|v| v.as_object()) else {
                return;
            };
            let block_type = block
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if block_type == "tool_use" {
                let tool_call_id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let tool_name = block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if !tool_call_id.is_empty() && !tool_name.is_empty() {
                    host.on_tool_call_start(tool_call_id, tool_name);
                }
            }
        }
        "content_block_delta" => {
            let Some(delta) = event.get("delta").and_then(|v| v.as_object()) else {
                return;
            };
            match delta
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
            {
                "text_delta" => {
                    if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                        host.on_text_delta(text.to_string());
                    }
                }
                "thinking_delta" => {
                    if let Some(text) = delta.get("thinking").and_then(|v| v.as_str()) {
                        if thinking_started_at.is_none() {
                            *thinking_started_at = Some(Instant::now());
                        }
                        host.on_thinking_delta(text.to_string());
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn parse_assistant_message(message: &serde_json::Value) -> Option<ClaudeSdkAssistantMessage> {
    let payload = message.get("message")?;
    let content = payload.get("content")?;
    let mut parsed = ClaudeSdkAssistantMessage::default();

    match content {
        serde_json::Value::String(text) => {
            parsed.text = text.clone();
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                let block_type = block
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            parsed.text.push_str(text);
                        }
                    }
                    "thinking" => {
                        if let Some(text) = block.get("thinking").and_then(|v| v.as_str()) {
                            parsed.thinking_text.push_str(text);
                        }
                        if let Some(signature) = block.get("signature").and_then(|v| v.as_str()) {
                            parsed.thinking_signature = signature.to_string();
                        }
                    }
                    "tool_use" => {
                        let id = block
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let name = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let arguments = block
                            .get("input")
                            .cloned()
                            .unwrap_or_else(|| json!({}))
                            .to_string();
                        if !id.is_empty() && !name.is_empty() {
                            parsed.tool_calls.push(ToolCallInfo {
                                id,
                                name,
                                arguments,
                                order: None,
                                server_tool: None,
                                server_tool_output: None,
                                outcome: None,
                                recorded_output: None,
                                nested_tool_calls: None,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    if parsed.text.is_empty() && parsed.thinking_text.is_empty() && parsed.tool_calls.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

async fn handle_control_request<H: ClaudeSdkHost>(
    stdin: &mut tokio::process::ChildStdin,
    message: &serde_json::Value,
    host: &mut H,
    tools: &[ClaudeSdkToolDefinition],
    raw_request: &mut String,
) -> Result<(), String> {
    let request_id = message
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Claude Code control_request missing request_id".to_string())?;
    let request = message
        .get("request")
        .and_then(|v| v.as_object())
        .ok_or_else(|| "Claude Code control_request missing request body".to_string())?;
    let subtype = request
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let response = match subtype {
        "mcp_message" => {
            let server_name = request
                .get("server_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let mcp_message = request
                .get("message")
                .cloned()
                .ok_or_else(|| "Claude Code mcp_message missing message".to_string())?;
            let mcp_response =
                handle_mcp_message(request_id, server_name, &mcp_message, host, tools).await;
            json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {
                        "mcp_response": mcp_response,
                    }
                }
            })
        }
        "can_use_tool" => json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": request_id,
                "response": {
                    "behavior": "allow",
                    "updatedInput": request.get("input").cloned().unwrap_or_else(|| json!({})),
                    "toolUseID": request.get("tool_use_id").cloned().unwrap_or(serde_json::Value::Null),
                }
            }
        }),
        other => json!({
            "type": "control_response",
            "response": {
                "subtype": "error",
                "request_id": request_id,
                "error": format!("Unsupported Claude Code control_request subtype: {}", other),
            }
        }),
    };

    write_json_line(stdin, &response, raw_request).await
}

async fn handle_mcp_message<H: ClaudeSdkHost>(
    request_id: &str,
    _server_name: &str,
    message: &serde_json::Value,
    host: &mut H,
    tools: &[ClaudeSdkToolDefinition],
) -> serde_json::Value {
    let id = message
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = message
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));

    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "locus",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }
        }),
        "notifications/initialized" => json!({
            "jsonrpc": "2.0",
            "result": {}
        }),
        "tools/list" => {
            let tool_items: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                    })
                })
                .collect();
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_items,
                }
            })
        }
        "tools/call" => {
            let tool_name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let result = host.execute_tool(request_id, tool_name, arguments).await;
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{
                        "type": "text",
                        "text": result.output,
                    }],
                    "isError": result.is_error,
                }
            })
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Unsupported MCP method '{}'", method),
            }
        }),
    }
}

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
    raw_request: &mut String,
) -> Result<(), String> {
    let line = serde_json::to_string(value)
        .map_err(|e| format!("Failed to serialize Claude SDK payload: {}", e))?;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("Failed writing to Claude Code CLI stdin: {}", e))?;
    stdin
        .write_all(b"\n")
        .await
        .map_err(|e| format!("Failed writing newline to Claude Code CLI stdin: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("Failed flushing Claude Code CLI stdin: {}", e))?;
    raw_request.push_str(&line);
    raw_request.push('\n');
    Ok(())
}

/// JS source of the http/https/fetch interception hook. Loaded into the CLI child via
/// `bun --require <abs path>` (preferred for bun-installed claude) or
/// `NODE_OPTIONS=--require <abs path>` (fallback for node-based wrappers) when debug
/// mode is on.
const CLAUDE_HTTP_HOOK_JS: &str = include_str!("claude_http_hook.cjs");

/// When the SDK debug hook needs to be injected and the user's `claude` is installed
/// via bun, we cannot rely on `NODE_OPTIONS=--require` because bun silently ignores
/// `--require` inside that env var. Instead we re-target the spawn at `bun.exe`
/// directly with `--require <hook> <cli.js>` as the leading args, bypassing the
/// `.exe` shim. This struct holds the discovered locations.
#[derive(Debug, Clone)]
struct BunPreloadLayout {
    bun_exe: PathBuf,
    cli_js: PathBuf,
}

/// Locate `bun.exe` + the `@anthropic-ai/claude-code` `cli.js` for the given claude
/// CLI path. Returns `None` if either piece can't be found, in which case we fall back
/// to the (probably broken-on-bun) `NODE_OPTIONS=--require` path. The user can force a
/// specific layout via the `LOCUS_BUN_EXE` and `LOCUS_CLAUDE_CLI_JS` env vars.
fn try_bun_preload_layout(claude_cli_path: &Path) -> Option<BunPreloadLayout> {
    let bun_exe = find_bun_exe(claude_cli_path)?;
    let cli_js = find_claude_cli_js(claude_cli_path)?;
    Some(BunPreloadLayout { bun_exe, cli_js })
}

fn find_bun_exe(claude_cli_path: &Path) -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("LOCUS_BUN_EXE") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }

    // Common case: claude.exe sits next to bun.exe in `~/.bun/bin/`.
    if let Some(parent) = claude_cli_path.parent() {
        for name in bun_binary_names() {
            let candidate = parent.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        for name in bun_binary_names() {
            let candidate = home.join(".bun").join("bin").join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn bun_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["bun.exe"]
    }
    #[cfg(not(target_os = "windows"))]
    {
        &["bun"]
    }
}

fn find_claude_cli_js(_claude_cli_path: &Path) -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("LOCUS_CLAUDE_CLI_JS") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }

    let home = dirs::home_dir()?;
    let pkg_relative = PathBuf::from("@anthropic-ai")
        .join("claude-code")
        .join("cli.js");

    let candidates: Vec<PathBuf> = vec![
        // Found in the wild on the user's machine — bun/npm install in home dir.
        home.join("node_modules").join(&pkg_relative),
        // bun install -g default location.
        home.join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join(&pkg_relative),
        // npm -g default on Windows.
        std::env::var_os("AppData")
            .map(PathBuf::from)
            .map(|p| p.join("npm").join("node_modules").join(&pkg_relative))
            .unwrap_or_default(),
        // npm -g default on Unix-y systems.
        PathBuf::from("/usr/local/lib/node_modules").join(&pkg_relative),
    ];

    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Provider tag used for the SDK's debug folder. Keep in sync with the value passed
/// to [`crate::llm::debug::save_request`] for the synthesized request snapshot.
const SDK_PROVIDER_TAG: &str = "anthropic_sdk";

/// Resolve the absolute per-provider debug directory for the SDK backend. Anchored
/// under the shared LLM debug root, which defaults to `<repo_root>/debug/llm/anthropic_sdk/`
/// in dev and `<install_dir>/data/debug/llm/anthropic_sdk/` in packaged runtimes
/// (or under `LOCUS_DEBUG_DIR` if set). The dev path stays outside `src-tauri/` so
/// `tauri dev` does not see new captures and trigger a rebuild loop.
///
/// On Windows, `std::fs::canonicalize` returns paths prefixed with `\\?\`
/// (the Win32 namespace extended-length prefix). Neither node nor bun accept that
/// form in `--require`, and bun silently swallows the failure — so the http hook
/// never loads. We strip the prefix here to keep the path interoperable with the
/// JS runtime that reads it back via `NODE_OPTIONS`.
fn sdk_debug_dir() -> PathBuf {
    let dir = crate::llm::debug::debug_dir_for(SDK_PROVIDER_TAG);
    let canonical = std::fs::canonicalize(&dir).unwrap_or(dir);
    strip_windows_namespace_prefix(canonical)
}

#[cfg(target_os = "windows")]
fn strip_windows_namespace_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        // \\?\UNC\server\share -> \\server\share, plain \\?\C:\... -> C:\...
        if let Some(unc) = rest.strip_prefix("UNC\\") {
            return PathBuf::from(format!(r"\\{}", unc));
        }
        return PathBuf::from(rest.to_string());
    }
    p
}

#[cfg(not(target_os = "windows"))]
fn strip_windows_namespace_prefix(p: PathBuf) -> PathBuf {
    p
}

/// Write the embedded JS hook to disk inside the SDK debug folder and return its
/// absolute path. Returns `None` (and logs to stderr) if the file system rejects the
/// write — debug instrumentation must never abort the real run.
fn install_http_hook() -> Option<PathBuf> {
    let dir = sdk_debug_dir();
    let hook_path = dir.join("_locus_claude_http_hook.cjs");
    if let Err(e) = std::fs::write(&hook_path, CLAUDE_HTTP_HOOK_JS) {
        eprintln!(
            "[debug] failed to write claude http hook to {:?}: {}",
            hook_path, e
        );
        return None;
    }
    Some(hook_path)
}

/// Persist the synthesized SDK invocation as a `.http`-style file under the debug folder.
/// Mirrors `crate::llm::debug::save_request` so SDK runs land next to the OpenRouter /
/// Anthropic OAuth captures and can be diffed visually.
fn save_debug_request(
    options: &ClaudeCodeSdkOptions,
    cmd: &tokio::process::Command,
    user_message: &serde_json::Value,
) {
    let std_cmd = cmd.as_std();
    let program = std_cmd.get_program().to_string_lossy().to_string();
    let args: Vec<String> = std_cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();

    let tools_summary: Vec<serde_json::Value> = options
        .tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            })
        })
        .collect();

    let body = json!({
        "model": normalize_claude_model(&options.model),
        "system": options.system_prompt,
        "messages": [user_message],
        "tools": tools_summary,
        "cli": {
            "program": program,
            "args": args,
            "cwd": options.cwd,
            "resume_session_id": options.resume_session_id,
        },
    });
    let body_str = serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    let url = format!("claude-cli://{}?stream=stream-json", options.server_name);
    let headers: [(&str, &str); 2] = [
        ("x-locus-session-id", options.locus_session_id.as_str()),
        ("x-claude-model", options.model.as_str()),
    ];
    crate::llm::debug::save_request("anthropic_sdk", &url, &headers, &body_str);
}

/// Persist the raw stream-json output from the Claude CLI subprocess so debug runs include
/// the response side too. Lives next to the request file under the same debug folder.
fn save_debug_response(
    options: &ClaudeCodeSdkOptions,
    raw_response: &str,
    stderr_output: &str,
    exit_code: Option<i32>,
) {
    let dir = sdk_debug_dir();

    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S%.3f");
    let filename = format!("{}_response.ndjson", ts);
    let path = dir.join(filename);

    let mut content = String::new();
    content.push_str(&format!(
        "# locus_session_id: {}\n# model: {}\n# exit_code: {:?}\n",
        options.locus_session_id, options.model, exit_code
    ));
    if !stderr_output.trim().is_empty() {
        content.push_str("# --- stderr ---\n");
        for line in stderr_output.lines() {
            content.push_str(&format!("# {}\n", line));
        }
    }
    content.push_str("# --- stdout (stream-json) ---\n");
    content.push_str(raw_response);

    if let Err(e) = std::fs::write(&path, content) {
        eprintln!("[debug] failed to write {:?}: {}", path, e);
    }
}

fn normalize_claude_model(model: &str) -> String {
    let trimmed = model.trim();
    let short = trimmed.strip_prefix("anthropic_sdk/").unwrap_or(trimmed);
    match short {
        "claude-sonnet-4.6" => "claude-sonnet-4-6".to_string(),
        "claude-opus-4.6" => "claude-opus-4-6".to_string(),
        other => other.to_string(),
    }
}

fn is_usable_claude_cli(path: &Path) -> bool {
    let mut cmd = std::process::Command::new(path);
    cmd.arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.status().map(|status| status.success()).unwrap_or(false)
}

fn path_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(path_var) = std::env::var_os("PATH") else {
        return candidates;
    };

    for dir in std::env::split_paths(&path_var) {
        for name in cli_binary_names() {
            candidates.push(dir.join(name));
        }
    }

    candidates
}

fn extra_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(home) = dirs::home_dir() {
        #[cfg(target_os = "windows")]
        {
            candidates.push(home.join(".npm-global").join("bin").join("claude.cmd"));
            candidates.push(home.join(".npm-global").join("bin").join("claude.exe"));
            candidates.push(home.join("node_modules").join(".bin").join("claude.cmd"));
            candidates.push(home.join(".claude").join("local").join("claude.exe"));
            candidates.push(home.join(".claude").join("local").join("claude.cmd"));
        }

        #[cfg(not(target_os = "windows"))]
        {
            candidates.push(home.join(".npm-global").join("bin").join("claude"));
            candidates.push(home.join(".local").join("bin").join("claude"));
            candidates.push(home.join("node_modules").join(".bin").join("claude"));
            candidates.push(home.join(".yarn").join("bin").join("claude"));
            candidates.push(home.join(".claude").join("local").join("claude"));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = std::env::var_os("AppData") {
            let app_data = PathBuf::from(app_data);
            candidates.push(app_data.join("npm").join("claude.cmd"));
            candidates.push(app_data.join("npm").join("claude.exe"));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        candidates.push(PathBuf::from("/usr/local/bin/claude"));
    }

    candidates
}

fn cli_binary_names() -> &'static [&'static str] {
    #[cfg(target_os = "windows")]
    {
        &["claude.exe", "claude.cmd", "claude.bat"]
    }

    #[cfg(not(target_os = "windows"))]
    {
        &["claude"]
    }
}
