use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::session::models::{ImageData, ToolCallInfo};
use crate::tool::ToolResult;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Built-in Claude Code tools removed from the model's tool list so the CLI
/// acts purely as the reasoning loop while every capability flows through the
/// Locus SDK-MCP bridge (and therefore through Locus permission gating).
///
/// This is a deny list, so it must cover every builtin across CLI variants;
/// names unknown to a given build only produce a stderr warning ("matches no
/// known tool"), never an error — verified on claude.exe 2.1.170.
const DISALLOWED_CLAUDE_BUILTINS: &[&str] = &[
    // Execution / editing / search builtins replaced by Locus tools.
    "Bash",
    "BashOutput",
    "KillShell",
    "Edit",
    "MultiEdit",
    "Write",
    "Read",
    "NotebookEdit",
    "NotebookRead",
    "Glob",
    "Grep",
    "LS",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "TodoWrite",
    "SlashCommand",
    // Interactive / planning / discovery builtins that would bypass the Locus
    // UX (Locus ships its own ask_user_question and plan mode).
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "Skill",
    "ToolSearch",
    "ListMcpResources",
    "ReadMcpResource",
    // Extras observed on the desktop-bundled claude.exe 2.1.x builds.
    "CronCreate",
    "CronDelete",
    "CronList",
    "EnterWorktree",
    "ExitWorktree",
    "Monitor",
    "PushNotification",
    "RemoteTrigger",
    "ScheduleWakeup",
    "TaskCreate",
    "TaskGet",
    "TaskList",
    "TaskUpdate",
    "TaskOutput",
    "TaskStop",
    "Workflow",
];

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

/// Drop the cached Claude session id for a Locus session. Called when the
/// persisted store says there is no resume point (e.g. the history was rolled
/// back past every Claude-backed turn) so the stale cache cannot resurrect a
/// Claude session containing rolled-back content.
pub async fn clear_session_id(locus_session_id: &str) {
    claude_session_map().lock().await.remove(locus_session_id);
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
pub struct ClaudeCodeToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeCliOptions {
    pub locus_session_id: String,
    pub cwd: String,
    pub system_prompt: String,
    pub model: String,
    pub effort: Option<String>,
    pub resume_session_id: Option<String>,
    pub server_name: String,
    pub tools: Vec<ClaudeCodeToolDefinition>,
    pub debug: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeCodeAssistantMessage {
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

#[derive(Debug, Clone)]
pub struct ClaudeCodeToolResult {
    pub output: String,
    pub is_error: bool,
    pub images: Option<Vec<ImageData>>,
}

impl From<ToolResult> for ClaudeCodeToolResult {
    fn from(result: ToolResult) -> Self {
        Self {
            output: result.output,
            is_error: result.is_error,
            images: None,
        }
    }
}

pub type ClaudeCodeHostFuture<'a> = Pin<Box<dyn Future<Output = ClaudeCodeToolResult> + Send + 'a>>;

pub trait ClaudeCodeHost {
    fn on_text_delta(&mut self, delta: String);
    fn on_thinking_delta(&mut self, delta: String);
    fn on_tool_call_start(&mut self, tool_call_id: String, tool_name: String);
    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String>;
    fn execute_tool<'a>(
        &'a mut self,
        request_id: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a>;
}

pub fn claude_cli_status() -> (bool, String) {
    match find_claude_cli() {
        Some(path) => (true, path.display().to_string()),
        None => (false, String::new()),
    }
}

/// Login state of the local Claude Code CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeCliLoginState {
    LoggedIn,
    LoggedOut,
    /// Credentials could not be inspected (e.g. macOS Keychain storage);
    /// callers should not block usage on this state.
    Unknown,
}

/// Detect whether the Claude Code CLI has usable credentials without spawning
/// it (a probe run would either bill a real API call when logged in or burn a
/// subprocess startup per status refresh). Mirrors the CLI's own auth sources:
/// explicit env vars, the OAuth credentials file under the Claude config dir,
/// or an `apiKeyHelper` in settings.json.
///
/// This is only a fast heuristic — `settings.json`-based credentials are NOT
/// honored at runtime because real turns spawn the CLI with
/// `--setting-sources ""`. Use [`run_login_test`] for an authoritative check
/// that exercises the actual endpoint and credentials.
pub fn claude_cli_login_status() -> (ClaudeCliLoginState, String) {
    // `ANTHROPIC_AUTH_TOKEN` is the bearer-token variable used with custom
    // endpoints/gateways (alongside `ANTHROPIC_BASE_URL`); Locus forwards the
    // whole environment to the CLI child, so honoring it here keeps the status
    // indicator in sync with what a real turn can actually authenticate with.
    for var in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        if std::env::var(var)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
        {
            return (ClaudeCliLoginState::LoggedIn, format!("{} (env)", var));
        }
    }

    if let Some(dir) = claude_config_dir() {
        if credentials_file_has_login(&dir.join(".credentials.json")) {
            return (
                ClaudeCliLoginState::LoggedIn,
                "subscription login".to_string(),
            );
        }
        if settings_has_api_key_helper(&dir.join("settings.json")) {
            return (ClaudeCliLoginState::LoggedIn, "apiKeyHelper".to_string());
        }
    }

    // macOS stores OAuth credentials in the Keychain, which cannot be read
    // here without risking an interactive prompt — report Unknown instead of
    // a false "logged out".
    #[cfg(target_os = "macos")]
    let fallback = (ClaudeCliLoginState::Unknown, String::new());
    #[cfg(not(target_os = "macos"))]
    let fallback = (ClaudeCliLoginState::LoggedOut, String::new());
    fallback
}

fn claude_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|home| home.join(".claude"))
}

fn credentials_file_has_login(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let Some(oauth) = value.get("claudeAiOauth") else {
        return false;
    };
    ["accessToken", "refreshToken"].iter().any(|key| {
        oauth
            .get(*key)
            .and_then(|v| v.as_str())
            .map(|token| !token.trim().is_empty())
            .unwrap_or(false)
    })
}

fn settings_has_api_key_helper(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|value| {
            value
                .get("apiKeyHelper")
                .and_then(|v| v.as_str())
                .map(|helper| !helper.trim().is_empty())
        })
        .unwrap_or(false)
}

pub fn find_claude_cli() -> Option<PathBuf> {
    for candidate in path_candidates().into_iter().chain(extra_candidates()) {
        if candidate.is_file() && is_usable_claude_cli(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub async fn run_turn<H: ClaudeCodeHost>(
    options: ClaudeCodeCliOptions,
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
                    "[Claude Code CLI debug] bun preload layout detected: bun={}, cli.js={}",
                    layout.bun_exe.display(),
                    layout.cli_js.display()
                );
                bun_preload_active = Some((layout, hook_path));
            }
        } else {
            eprintln!("[Claude Code CLI debug] bun preload layout NOT detected — falling back to NODE_OPTIONS=--require (only works for node-based claude shims)");
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
        .arg("--exclude-dynamic-system-prompt-sections")
        .arg("--system-prompt")
        .arg(options.system_prompt.clone())
        .arg("--permission-mode")
        .arg("bypassPermissions")
        // Hermetic run: tool permissioning happens on the Locus side, so the
        // child must not pick up user/project Claude Code config — external
        // MCP servers, hooks, skills, and plugins would otherwise auto-run
        // under bypassPermissions outside the Locus permission UI.
        // `--strict-mcp-config` does not affect the sdkMcpServers
        // control-protocol server registered below (verified on 2.1.170).
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("");

    let normalized_model = normalize_claude_model(&options.model);
    if !normalized_model.is_empty() {
        cmd.arg("--model").arg(normalized_model);
    }

    if let Some(resume_session_id) = options.resume_session_id.as_deref() {
        if !resume_session_id.trim().is_empty() {
            cmd.arg("--resume").arg(resume_session_id.trim());
        }
    }

    if let Some(effort) = options
        .effort
        .as_deref()
        .map(str::trim)
        .filter(|effort| !effort.is_empty() && *effort != "none")
    {
        cmd.arg("--effort").arg(effort);
    }

    cmd.arg("--disallowed-tools");
    for tool in DISALLOWED_CLAUDE_BUILTINS {
        cmd.arg(tool);
    }

    let mut envs = base_child_env();
    envs.entry(OsString::from("LOCUS_SESSION_ID"))
        .or_insert_with(|| OsString::from(options.locus_session_id.clone()));

    if options.debug {
        let abs_dir = claude_code_debug_dir();
        eprintln!(
            "[Claude Code CLI debug] LOCUS_DEBUG_DIR -> {}",
            abs_dir.display()
        );
        envs.insert(
            OsString::from("LOCUS_DEBUG_DIR"),
            OsString::from(abs_dir.to_string_lossy().to_string()),
        );

        if bun_preload_active.is_some() {
            // Hook is loaded via the explicit `bun --require <hook>` arg added when
            // building cmd above. Nothing more to do for the env.
            eprintln!(
                "[Claude Code CLI debug] hook injection mode = bun --require (explicit cli flag)"
            );
        } else {
            // Fallback for non-bun installs: try NODE_OPTIONS=--require, which works
            // for npm/node-based shims but is silently ignored by bun.
            match install_http_hook() {
                Some(hook_path) => {
                    eprintln!(
                        "[Claude Code CLI debug] hook script written to {}",
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
                        "[Claude Code CLI debug] hook injection mode = NODE_OPTIONS={}",
                        combined
                    );
                    envs.insert(OsString::from("NODE_OPTIONS"), OsString::from(combined));
                }
                None => {
                    eprintln!(
                        "[Claude Code CLI debug] install_http_hook returned None — hook not injected"
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
                        eprintln!("[Claude Code CLI stderr] {}", line);
                        buf.push_str(&line);
                        buf.push('\n');
                    }
                    Ok(None) => break,
                    Err(e) => {
                        eprintln!("[Claude Code CLI stderr] read error: {}", e);
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
                handle_stream_event(
                    &message,
                    host,
                    &options.server_name,
                    &mut thinking_started_at,
                );
            }
            "assistant" => {
                if let Some(parsed) = parse_assistant_message(&message, &options.server_name) {
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
                    eprintln!(
                        "[Claude Code CLI] ignoring message type '{}': {}",
                        other, line
                    );
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

/// Environment handed to every Claude Code CLI child: the full Locus process
/// environment (so user-configured `ANTHROPIC_*` variables — including
/// `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` for a custom default endpoint —
/// flow straight through), plus the Locus entrypoint tag and resolved proxy
/// settings. Callers add per-run extras (e.g. `LOCUS_SESSION_ID`) on top.
fn base_child_env() -> HashMap<OsString, OsString> {
    let mut envs: HashMap<OsString, OsString> = std::env::vars_os().collect();
    envs.entry(OsString::from("CLAUDE_CODE_ENTRYPOINT"))
        .or_insert_with(|| OsString::from("locus-rs"));
    crate::network::extend_proxy_env_map(&mut envs);
    envs
}

/// Trivial prompt used by the connectivity test. Kept tiny to minimize the real
/// (billed) round-trip the CLI makes against the active endpoint.
const LOGIN_TEST_PROMPT: &str = "Respond with the single word: OK";

/// How long to wait for the connectivity test turn before giving up.
const LOGIN_TEST_TIMEOUT: Duration = Duration::from_secs(45);

/// Authoritative connectivity / auth check: spawn the Claude Code CLI under the
/// same hermetic flags and environment a real Locus turn uses, send a trivial
/// prompt, and report whether the turn completed. Unlike
/// [`claude_cli_login_status`] this exercises the genuine endpoint resolution
/// (`ANTHROPIC_BASE_URL`), credentials, and proxy — so a setup that only works
/// via `settings.json` (which `--setting-sources ""` disables) correctly fails
/// here, matching real turns. Returns the model reply on success, or a
/// human-readable error.
pub async fn run_login_test() -> Result<String, String> {
    let cli_path = find_claude_cli().ok_or_else(|| {
        "Claude Code CLI not found. Install `@anthropic-ai/claude-code` and ensure `claude` is available in PATH.".to_string()
    })?;

    let mut cmd = tokio::process::Command::new(&cli_path);
    cmd.kill_on_drop(true);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("--print")
        .arg("--output-format")
        .arg("json")
        .arg("--permission-mode")
        .arg("bypassPermissions")
        // Mirror run_turn's hermetic flags so the test reflects exactly what a
        // real Locus turn sees (settings.json is NOT consulted).
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("");
    // Deny every builtin: with bypassPermissions in effect the model must not be
    // able to touch the machine just because we sent it a prompt. The prompt is
    // delivered via stdin (below), never as a positional arg, so this variadic
    // flag can sit last without swallowing it.
    cmd.arg("--disallowed-tools");
    for tool in DISALLOWED_CLAUDE_BUILTINS {
        cmd.arg(tool);
    }
    cmd.envs(base_child_env());

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start Claude Code CLI: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        // `--print` with no positional prompt reads the prompt from stdin.
        let _ = stdin.write_all(LOGIN_TEST_PROMPT.as_bytes()).await;
        let _ = stdin.write_all(b"\n").await;
        let _ = stdin.flush().await;
        // Dropped here → EOF, so the CLI stops waiting for more input.
    }

    let output = match tokio::time::timeout(LOGIN_TEST_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(format!("Failed waiting for Claude Code CLI: {}", e)),
        Err(_) => {
            // Future dropped here → kill_on_drop terminates the child.
            return Err(format!(
                "Claude Code CLI did not respond within {}s",
                LOGIN_TEST_TIMEOUT.as_secs()
            ));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_login_test_output(
        &stdout,
        &stderr,
        output.status.success(),
        output.status.code(),
    )
}

/// Interpret the `--output-format json` result of the connectivity probe.
fn parse_login_test_output(
    stdout: &str,
    stderr: &str,
    success: bool,
    code: Option<i32>,
) -> Result<String, String> {
    // `--output-format json` emits a single result object; tolerate either a
    // whole-buffer object or (defensively) the last non-empty line.
    let value = serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .or_else(|| {
            stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .last()
                .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        });

    if let Some(value) = value {
        // Some builds wrap the result in a single-element array.
        let result_value = if value.is_array() {
            value
                .as_array()
                .and_then(|items| items.last())
                .cloned()
                .unwrap_or(value)
        } else {
            value
        };
        let mut parsed = ClaudeCodeTurnResult::default();
        parse_result_message(&result_value, &mut parsed)?;
        let reply = parsed.final_text.trim();
        let endpoint = active_endpoint_label();
        return Ok(if reply.is_empty() {
            format!("Connected to {endpoint}.")
        } else {
            format!(
                "Connected to {endpoint}. Reply: {}",
                truncate_for_display(reply, 120)
            )
        });
    }

    let detail = stderr.trim();
    if !detail.is_empty() {
        return Err(format!("Claude Code CLI failed: {}", detail));
    }
    if !success {
        return Err(match code {
            Some(code) => format!("Claude Code CLI exited with status {code}"),
            None => "Claude Code CLI exited abnormally".to_string(),
        });
    }
    Err("Claude Code CLI produced no result".to_string())
}

/// Label for the endpoint the CLI will talk to, shown in the test result —
/// reflects `ANTHROPIC_BASE_URL` when a custom default endpoint is configured.
fn active_endpoint_label() -> String {
    match std::env::var("ANTHROPIC_BASE_URL") {
        Ok(url) if !url.trim().is_empty() => url.trim().to_string(),
        _ => "api.anthropic.com (default)".to_string(),
    }
}

fn truncate_for_display(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max_chars).collect();
    format!("{prefix}…")
}

fn parse_result_message(
    message: &serde_json::Value,
    out: &mut ClaudeCodeTurnResult,
) -> Result<(), String> {
    let subtype = message
        .get("subtype")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    // The CLI reports some failures (e.g. "Not logged in") with
    // `subtype: "success"` but `is_error: true` and the error text in
    // `result`; honor both signals so auth failures never masquerade as a
    // normal assistant reply.
    let is_error = message
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if subtype != "success" || is_error {
        let result_text = message
            .get("result")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .unwrap_or_default();
        let errors = message
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|s| !s.is_empty());
        let mut detail = if !result_text.is_empty() {
            result_text.to_string()
        } else if let Some(errors) = errors {
            errors
        } else {
            "Claude Code returned an error result".to_string()
        };
        if detail.contains("Not logged in") {
            detail = format!(
                "{} — the Claude Code CLI has no usable login. Run `claude` in a terminal and complete /login (or set ANTHROPIC_API_KEY), then retry.",
                detail
            );
        }
        return Err(detail);
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

/// The CLI advertises SDK-MCP server tools to the model under the namespaced
/// form `mcp__<server>__<tool>`, while `tools/call` requests deliver the bare
/// tool name. Normalize model-facing names back to the bare Locus tool names
/// so round bookkeeping, undo recording, persisted history, and the UI all
/// refer to the same tool identity.
fn strip_mcp_tool_prefix(name: &str, server_name: &str) -> String {
    let prefix = format!("mcp__{}__", server_name);
    name.strip_prefix(&prefix).unwrap_or(name).to_string()
}

fn handle_stream_event<H: ClaudeCodeHost>(
    message: &serde_json::Value,
    host: &mut H,
    server_name: &str,
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
                    .map(|name| strip_mcp_tool_prefix(name, server_name))
                    .unwrap_or_default();
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

fn parse_assistant_message(
    message: &serde_json::Value,
    server_name: &str,
) -> Option<ClaudeCodeAssistantMessage> {
    let payload = message.get("message")?;
    let content = payload.get("content")?;
    let mut parsed = ClaudeCodeAssistantMessage::default();

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
                            .map(|name| strip_mcp_tool_prefix(name, server_name))
                            .unwrap_or_default();
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

async fn handle_control_request<H: ClaudeCodeHost>(
    stdin: &mut tokio::process::ChildStdin,
    message: &serde_json::Value,
    host: &mut H,
    tools: &[ClaudeCodeToolDefinition],
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

async fn handle_mcp_message<H: ClaudeCodeHost>(
    request_id: &str,
    _server_name: &str,
    message: &serde_json::Value,
    host: &mut H,
    tools: &[ClaudeCodeToolDefinition],
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
            let content = mcp_tool_result_content(&result);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": content,
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

fn mcp_tool_result_content(result: &ClaudeCodeToolResult) -> Vec<serde_json::Value> {
    let mut content = vec![json!({
        "type": "text",
        "text": result.output,
    })];

    if let Some(images) = result.images.as_ref() {
        for image in images {
            if image.data.trim().is_empty() {
                continue;
            }
            content.push(json!({
                "type": "image",
                "data": image.data,
                "mimeType": image.mime_type,
            }));
        }
    }

    content
}

async fn write_json_line(
    stdin: &mut tokio::process::ChildStdin,
    value: &serde_json::Value,
    raw_request: &mut String,
) -> Result<(), String> {
    let line = serde_json::to_string(value)
        .map_err(|e| format!("Failed to serialize Claude Code CLI payload: {}", e))?;
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

/// When the Claude Code CLI debug hook needs to be injected and the user's `claude` is installed
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

/// Provider tag used for the Claude Code CLI debug folder. Keep in sync with the value passed
/// to [`crate::llm::debug::save_request`] for the synthesized request snapshot.
const CLAUDE_CODE_PROVIDER_TAG: &str = "claude_code";

/// Resolve the absolute per-provider debug directory for the Claude Code CLI backend. Anchored
/// under the shared LLM debug root, which defaults to `<repo_root>/debug/llm/claude_code/`
/// in dev and `<install_dir>/data/debug/llm/claude_code/` in packaged runtimes
/// (or under `LOCUS_DEBUG_DIR` if set). The dev path stays outside `src-tauri/` so
/// `tauri dev` does not see new captures and trigger a rebuild loop.
///
/// On Windows, `std::fs::canonicalize` returns paths prefixed with `\\?\`
/// (the Win32 namespace extended-length prefix). Neither node nor bun accept that
/// form in `--require`, and bun silently swallows the failure — so the http hook
/// never loads. We strip the prefix here to keep the path interoperable with the
/// JS runtime that reads it back via `NODE_OPTIONS`.
fn claude_code_debug_dir() -> PathBuf {
    let dir = crate::llm::debug::debug_dir_for(CLAUDE_CODE_PROVIDER_TAG);
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

/// Write the embedded JS hook to disk inside the Claude Code CLI debug folder and return its
/// absolute path. Returns `None` (and logs to stderr) if the file system rejects the
/// write — debug instrumentation must never abort the real run.
fn install_http_hook() -> Option<PathBuf> {
    let dir = claude_code_debug_dir();
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

/// Persist the synthesized Claude Code CLI invocation as a `.http`-style file under the debug folder.
/// Mirrors `crate::llm::debug::save_request` so Claude Code CLI runs land next to the OpenRouter /
/// Anthropic OAuth captures and can be diffed visually.
fn save_debug_request(
    options: &ClaudeCodeCliOptions,
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
            "effort": options.effort,
            "resume_session_id": options.resume_session_id,
        },
    });
    let body_str = serde_json::to_string_pretty(&body).unwrap_or_else(|_| format!("{:?}", body));

    let url = format!("claude-cli://{}?stream=stream-json", options.server_name);
    let headers: [(&str, &str); 2] = [
        ("x-locus-session-id", options.locus_session_id.as_str()),
        ("x-claude-model", options.model.as_str()),
    ];
    crate::llm::debug::save_request("claude_code", &url, &headers, &body_str);
}

/// Persist the raw stream-json output from the Claude CLI subprocess so debug runs include
/// the response side too. Lives next to the request file under the same debug folder.
fn save_debug_response(
    options: &ClaudeCodeCliOptions,
    raw_response: &str,
    stderr_output: &str,
    exit_code: Option<i32>,
) {
    let dir = claude_code_debug_dir();

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
    let short = trimmed.strip_prefix("claude_code/").unwrap_or(trimmed);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sdk_mcp_prefix_from_model_tool_names() {
        assert_eq!(
            strip_mcp_tool_prefix("mcp__locus__unity_execute", "locus"),
            "unity_execute"
        );
        assert_eq!(
            strip_mcp_tool_prefix("unity_execute", "locus"),
            "unity_execute"
        );
        assert_eq!(
            strip_mcp_tool_prefix("mcp__other__tool", "locus"),
            "mcp__other__tool"
        );
    }

    #[test]
    fn parse_assistant_message_normalizes_namespaced_tool_names() {
        let message = json!({
            "type": "assistant",
            "message": {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "mcp__locus__read",
                        "input": {"path": "a.txt"}
                    }
                ]
            }
        });

        let parsed = parse_assistant_message(&message, "locus").expect("parsed message");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "toolu_1");
        assert_eq!(parsed.tool_calls[0].name, "read");
    }

    #[test]
    fn result_with_is_error_is_reported_as_failure() {
        let message = json!({
            "type": "result",
            "subtype": "success",
            "is_error": true,
            "result": "Not logged in · Please run /login",
        });

        let mut out = ClaudeCodeTurnResult::default();
        let error = parse_result_message(&message, &mut out).expect_err("must fail");
        assert!(error.contains("Not logged in"));
        assert!(error.contains("/login"));
    }

    #[test]
    fn successful_result_parses_text_and_usage() {
        let message = json!({
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "result": "done",
            "total_cost_usd": 0.5,
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let mut out = ClaudeCodeTurnResult::default();
        parse_result_message(&message, &mut out).expect("success result");
        assert_eq!(out.final_text, "done");
        assert_eq!(out.input_tokens, 10);
        assert_eq!(out.output_tokens, 5);
        assert!((out.cost_usd - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn mcp_tool_result_content_includes_images() {
        let result = ClaudeCodeToolResult {
            output: "captured".to_string(),
            is_error: false,
            images: Some(vec![ImageData {
                data: "aW1hZ2U=".to_string(),
                mime_type: "image/png".to_string(),
            }]),
        };

        let content = mcp_tool_result_content(&result);
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "captured");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["data"], "aW1hZ2U=");
        assert_eq!(content[1]["mimeType"], "image/png");
    }
}
