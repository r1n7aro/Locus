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
use tokio::io::AsyncWriteExt;

use crate::session::models::{ImageData, ToolCallInfo};
use crate::tool::ToolResult;

mod mod_bridge;
#[cfg(feature = "server-client")]
mod server_transport;
mod bridge_health;
mod turn_tools;
mod diagnostics;
pub(crate) mod runtime_settings;
mod attachments;
pub(crate) mod fork;
pub(crate) mod rewind;
#[cfg(test)]
mod native_integration_tests;
#[cfg(test)]
mod native_feature_tests;
#[cfg(test)]
mod native_lazy_tests;
#[cfg(test)]
mod native_search_tests;
#[cfg(test)]
mod native_agents_tests;
pub(crate) mod native_tools;
pub(crate) mod native_scope;
pub(crate) mod native_executor;
pub(crate) mod native_grep;
pub(crate) mod native_output;
pub(crate) mod tui;
pub(crate) mod ssh;
pub(crate) mod connection;
pub(crate) mod connection_publisher;
pub(crate) mod recovery;
pub(crate) mod turn_retry;
pub(crate) mod archive;
mod native_journal;
pub(crate) mod tool_catalog;
mod pty_job;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Tools disabled only for the isolated login/connectivity probe. Interactive
/// work uses the native-tool Mod and keeps Claude's built-in tool definitions.
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

// Every TUI launch exposes only builtins whose execution the Mod actually adapts.
// Unsupported feature-flagged tools (e.g. Artifact) otherwise change schemas
// across CLI restarts and invalidate an unrelated conversation's prefix.
// Launches use `claude_launch::native_tools`, which adds the delegation tools.
#[cfg(test)]
pub(super) use locus_remote_core::claude_launch::NATIVE_BUILTIN_TOOLS;

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
    tui::close_session(locus_session_id);
    claude_session_map().lock().await.remove(locus_session_id);
}

/// First-send preparation can retain a verified empty prewarm. Explicit
/// history edits still use clear_session_id and always retire the process.
pub async fn clear_stale_session_id_for_turn(locus_session_id: &str) {
    if !tui::is_unused_prewarm(locus_session_id) { tui::close_session(locus_session_id); }
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

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClaudeCodeToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub defer_loading: bool,
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeCliOptions {
    pub remote: Option<ssh::SshConfig>,
    pub locus_session_id: String,
    pub cwd: String,
    pub system_prompt: String,
    pub model: String,
    pub effort: Option<String>,
    pub resume_session_id: Option<String>,
    pub tools: Vec<ClaudeCodeToolDefinition>,
    pub native_tool_search: bool,
    /// Launch with the native Agent/SendMessage tools (pinned route, Multi-Agent on).
    pub native_agents: bool,
    /// Pinned delegation directory for the Mod (`/options` "agents"); null when off.
    pub agents: serde_json::Value,
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
    pub upstream_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClaudeCodeToolResult {
    pub output: String,
    pub is_error: bool,
    pub images: Option<Vec<ImageData>>,
    /// Supplemental model context is independent of display/protocol output.
    pub context: Vec<String>,
}

impl ClaudeCodeToolResult {
    pub(crate) fn text_with_context(&self) -> String {
        let mut output = self.output.clone();
        for context in &self.context {
            if !context.is_empty() && !output.contains(context) {
                if !output.is_empty() { output.push_str("\n\n"); }
                output.push_str(context);
            }
        }
        output
    }
}

impl From<ToolResult> for ClaudeCodeToolResult {
    fn from(result: ToolResult) -> Self {
        Self {
            output: result.output,
            is_error: result.is_error,
            images: None,
            context: Vec::new(),
        }
    }
}

pub type ClaudeCodeHostFuture<'a> = Pin<Box<dyn Future<Output = ClaudeCodeToolResult> + Send + 'a>>;
/// A native Claude subagent of the current conversation (`agent.spawn`).
pub use mod_bridge::NativeAgent as ClaudeCodeNativeAgent;

/// Claude's `<task-notification>` for a background subagent run. The Mod
/// drops it before Claude's model sees it; Locus delivers the outcome as a
/// task result (and wakes an idle conversation) instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCodeAgentNotice {
    pub agent_id: String,
    /// The call that started this run: the Agent call or a resuming SendMessage.
    pub tool_use_id: String,
    /// `completed`, `failed` or `killed`.
    pub status: String,
    pub summary: String,
    pub result: Option<String>,
    /// An earlier run's notice, seen after Claude already continued the
    /// agent: its task belongs to the newer run.
    pub superseded: bool,
}

pub(crate) type ClaudeCodeUnitFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Executes and records a conversation's native Claude subagents. A Locus run
/// installs it; it keeps serving background agents after that run ends, so
/// none of its calls borrow the run's host. Calls may run concurrently.
pub trait ClaudeNativeAgentHost: Send + Sync {
    /// A run started: an `Agent` call, or the `SendMessage` that resumed it.
    fn registered<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent) -> ClaudeCodeUnitFuture<'a>;
    /// A subagent announced a tool call (Locus name and arguments).
    fn tool_started(&self, agent: &ClaudeCodeNativeAgent, tool_call_id: &str, tool_name: &str, arguments: &serde_json::Value);
    /// Display-only text of a subagent's message; never parent content.
    fn text(&self, agent: &ClaudeCodeNativeAgent, text: &str);
    /// A subagent's model message (thinking and text), for its Locus record.
    fn message(&self, _agent: &ClaudeCodeNativeAgent, _thinking: &str, _text: &str) {}
    /// Executes a bridged call under the parent's authority. `SendMessage`
    /// is a message to the main agent; its output is the native result JSON.
    fn execute<'a>(
        &'a self,
        agent: &'a ClaudeCodeNativeAgent,
        tool_call_id: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a>;
    /// A call that Claude ran itself or that was refused before Locus ran it.
    /// Idempotent with `execute`.
    fn tool_finished<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent, tool_call_id: &'a str, result: ClaudeCodeToolResult) -> ClaudeCodeUnitFuture<'a>;
    /// A background run ended (completed, failed, or stopped).
    fn notification<'a>(&'a self, notice: &'a ClaudeCodeAgentNotice) -> ClaudeCodeUnitFuture<'a>;
    /// Usage of one subagent model step.
    fn usage(&self, _agent: &ClaudeCodeNativeAgent, _usage: &serde_json::Value) {}
    /// The model a subagent runs on changed (first seen, normally once).
    fn model_resolved(&self, _agent: &ClaudeCodeNativeAgent, _model: &str) {}
    /// The Claude process is gone; its running background agents ended.
    fn process_exited<'a>(&'a self, agents: Vec<ClaudeCodeNativeAgent>) -> ClaudeCodeUnitFuture<'a>;
}

pub struct ClaudeCodeInput {
    pub id: String,
    pub message: serde_json::Value,
}
pub trait ClaudeCodeInputSource: Send + Sync {
    fn claim(&self) -> Result<Option<ClaudeCodeInput>,String>;
    fn accepted(&self,id:&str,checkpoint:&serde_json::Value) -> Result<(),String>;
    fn refresh_tools(&self) -> Pin<Box<dyn Future<Output=Result<Option<Vec<ClaudeCodeToolDefinition>>,String>> + Send + '_>> {
        Box::pin(async {Ok(None)})
    }
}

pub trait ClaudeCodeHost {
    /// Called after a bridged operation, before Claude can start its next step.
    fn refresh_tools(&self) -> Pin<Box<dyn Future<Output = Result<Option<Vec<ClaudeCodeToolDefinition>>, String>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }
    fn on_interrupted_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> { self.on_assistant_message(message) }
    fn on_checkpoint(&mut self, _event: &serde_json::Value) -> Result<(), String> { Ok(()) }
    fn on_compaction(&mut self, _event: &serde_json::Value) -> Result<(), String> { Ok(()) }
    /// Called before Claude summarizes. The reply carries summarizer
    /// instructions, the local transcript copy and the carry-forward block;
    /// an empty reply leaves Claude's native compaction unchanged.
    fn prepare_compaction<'a>(&'a mut self, _event: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = serde_json::Value> + Send + 'a>> {
        Box::pin(async { serde_json::json!({"ok":true}) })
    }
    fn on_session_started(&mut self, _session_id: &str) -> Result<(), String> { Ok(()) }
    fn on_model_resolved(&mut self, _model: &str) -> Result<(), String> { Ok(()) }
    fn on_step_started(&mut self, _step_id: u64) -> Result<(), String> { Ok(()) }
    fn on_step_usage(&mut self, _usage: &serde_json::Value, _context: &serde_json::Value, _timing: &serde_json::Value, _step_id: u64) -> Result<(), String> { Ok(()) }
    fn on_phase(&mut self, _phase: crate::session::models::ClaudeCodePhase) {}
    fn on_diagnostic(&mut self, _notice: crate::error::AppError) {}
    fn on_text_delta(&mut self, delta: String);
    fn on_thinking_delta(&mut self, delta: String);
    fn on_tool_call_start(&mut self, tool_call_id: String, tool_name: String);
    /// Display-only retirement of an incomplete streamed preview. This is not
    /// a tool result and must never be persisted or sent back to the model.
    fn on_tool_call_abandoned(&mut self, _tool_call_id: &str) {}
    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String>;
    fn on_external_tool_result<'a>(
        &'a mut self,
        _id: &'a str,
        _result: ClaudeCodeToolResult,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
    fn execute_tool<'a>(
        &'a mut self,
        request_id: &'a str,
        tool_name: &'a str,
        arguments: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a>;
    /// This conversation's subagent host. The session's agent pump keeps it
    /// after the run ends, for background agents.
    fn native_agent_host(&self) -> Option<std::sync::Arc<dyn ClaudeNativeAgentHost>> { None }
    /// Claude starts the native `Agent` call announced in this run. Its
    /// subagent takes its own workspace locks, so the run releases its batch.
    fn before_native_agent<'a>(&'a mut self, _tool_call_id: &'a str) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

pub fn claude_cli_status() -> (bool, String) {
    match find_claude_cli() {
        Some(path) => (true, path.display().to_string()),
        None => (false, String::new()),
    }
}

/// Only model identifiers cross IPC. Never expose the CLI's provider env or credentials.
pub fn configured_model() -> Option<String> {
    let env = base_child_env();
    let configured = env.get(std::ffi::OsStr::new("ANTHROPIC_MODEL"))
        .and_then(|value| value.to_str()).map(str::to_owned)
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let settings: serde_json::Value = serde_json::from_slice(
                &std::fs::read(claude_config_dir()?.join("settings.json")).ok()?
            ).ok()?;
            settings["model"].as_str().map(str::to_owned)
        })?;
    let model = normalize_claude_model(&configured);
    (!model.is_empty()).then_some(model)
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
/// Uses the same provider environment as real turns, including settings.json's
/// env block. Use [`run_login_test`] to exercise the endpoint and credentials.
pub fn claude_cli_login_status() -> (ClaudeCliLoginState, String) {
    // `ANTHROPIC_AUTH_TOKEN` is the bearer-token variable used with custom
    // endpoints/gateways (alongside `ANTHROPIC_BASE_URL`); Locus forwards the
    // whole environment to the CLI child, so honoring it here keeps the status
    // indicator in sync with what a real turn can actually authenticate with.
    let env = base_child_env();
    for var in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
    ] {
        if env.get(std::ffi::OsStr::new(var))
            .and_then(|value| value.to_str())
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

pub(crate) fn claude_config_dir() -> Option<PathBuf> {
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

/// How long a resolved (or unresolved) Claude Code CLI location stays cached.
/// Short enough that a freshly installed/removed CLI is picked up promptly,
/// long enough to spare `get_providers` (settings refresh, key save/delete,
/// connectivity test, import) from re-spawning `claude --version` every call.
const CLI_DISCOVERY_TTL: Duration = Duration::from_secs(30);

struct CliDiscoveryCache {
    path: Option<PathBuf>,
    resolved_at: Instant,
}

fn cli_discovery_cache() -> &'static std::sync::Mutex<Option<CliDiscoveryCache>> {
    static CACHE: OnceLock<std::sync::Mutex<Option<CliDiscoveryCache>>> = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Locate the Claude Code CLI, memoizing the result for [`CLI_DISCOVERY_TTL`].
///
/// Discovery probes candidate paths by spawning `claude --version`
/// ([`is_usable_claude_cli`]), which is costly on Windows. Caching keeps the hot
/// `get_providers` path from re-spawning it on every settings interaction.
pub fn find_claude_cli() -> Option<PathBuf> {
    if let Ok(guard) = cli_discovery_cache().lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.resolved_at.elapsed() < CLI_DISCOVERY_TTL {
                return cache.path.clone();
            }
        }
    }

    let resolved = discover_claude_cli();

    if let Ok(mut guard) = cli_discovery_cache().lock() {
        *guard = Some(CliDiscoveryCache {
            path: resolved.clone(),
            resolved_at: Instant::now(),
        });
    }

    resolved
}

/// Native multi-agent requires the CLI version its protocol was verified on.
pub(crate) const NATIVE_AGENTS_MIN_VERSION: (u64, u64, u64) = (2, 1, 312);

/// CLI version a new conversation's native-agent check uses: for SSH the
/// last connection test or session launch recorded it, otherwise the local
/// CLI's. Blocking: the local check runs `claude --version`.
pub(crate) fn native_agents_cli_version(settings: &ssh::ConnectionSettings) -> Option<String> {
    match &settings.remote {
        Some(_) => settings.info.as_ref().and_then(|info| info.cli_version.clone()),
        None => local_cli_version(),
    }
}

/// `claude --version` of the local CLI, cached like discovery. Called when a
/// new conversation chooses its multi-agent route and by Settings.
pub(crate) fn local_cli_version() -> Option<String> {
    static CACHE: OnceLock<std::sync::Mutex<Option<(PathBuf, Option<String>, Instant)>>> = OnceLock::new();
    let path = find_claude_cli()?;
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Some((cached, version, at)) = cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if *cached == path && at.elapsed() < CLI_DISCOVERY_TTL { return version.clone(); }
    }
    let mut command = std::process::Command::new(&path);
    command.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let version = command.output().ok().filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.split_whitespace().next().map(str::to_owned));
    *cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, version.clone(), Instant::now()));
    version
}

/// Compares the numeric `major.minor.patch` prefix ("2.1.312 (Claude Code)").
pub(crate) fn version_at_least(version: &str, minimum: (u64, u64, u64)) -> bool {
    let mut parts = version.trim().split(|c: char| !c.is_ascii_digit()).filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().ok());
    match (parts.next().flatten(), parts.next().flatten(), parts.next().flatten()) {
        (Some(major), Some(minor), Some(patch)) => (major, minor, patch) >= minimum,
        _ => false,
    }
}

fn discover_claude_cli() -> Option<PathBuf> {
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
    tui::run(options, user_message, host).await
}

pub async fn run_turn_with_inputs<H: ClaudeCodeHost>(
    options: ClaudeCodeCliOptions, user_message: serde_json::Value, host:&mut H, source:&dyn ClaudeCodeInputSource,
) -> Result<ClaudeCodeTurnResult,String> {
    tui::run_with_inputs(options,user_message,host,Some(source)).await
}

/// Short-lived jobs use the same real TUI and local tool bridge as Chat.
/// Close their session on completion, error, or cancellation.
pub async fn run_ephemeral_turn<H: ClaudeCodeHost>(
    options: ClaudeCodeCliOptions,
    user_message: serde_json::Value,
    host: &mut H,
) -> Result<ClaudeCodeTurnResult, String> {
    struct CloseSession(String);
    impl Drop for CloseSession {
        fn drop(&mut self) {
            tui::close_session(&self.0);
        }
    }
    let _session = CloseSession(options.locus_session_id.clone());
    tui::run(options, user_message, host).await
}

/// Environment handed to every Claude Code CLI child: the full Locus process
/// environment (so user-configured `ANTHROPIC_*` variables — including
/// `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` for a custom default endpoint —
/// flow straight through), plus the Locus entrypoint tag and resolved proxy
/// settings. Callers add per-run extras (e.g. `LOCUS_SESSION_ID`) on top.
fn base_child_env() -> HashMap<OsString, OsString> {
    let mut envs: HashMap<OsString, OsString> = std::env::vars_os().collect();
    // Hermetic plugin/settings loading must still honor the user's configured
    // Claude gateway credentials and model aliases. Import only provider env,
    // never permissions, hooks, plugins or arbitrary executable settings.
    if let Some(settings)=claude_config_dir().and_then(|root|std::fs::read_to_string(root.join("settings.json")).ok())
        .and_then(|text|serde_json::from_str::<serde_json::Value>(&text).ok()) {
        if let Some(values)=settings["env"].as_object(){
            for (key,value) in values {
                if (key.starts_with("ANTHROPIC_") || matches!(key.as_str(),"CLAUDE_CODE_OAUTH_TOKEN"|"CLAUDE_CODE_SUBAGENT_MODEL"|"CLAUDE_CODE_USE_BEDROCK"|"CLAUDE_CODE_USE_VERTEX"|"CLAUDE_CODE_USE_FOUNDRY")) && value.is_string() {
                    envs.insert(OsString::from(key),OsString::from(value.as_str().unwrap()));
                }
            }
        }
    }
    #[cfg(debug_assertions)]
    if let Some(url)=std::env::var_os("LOCUS_CLAUDE_TEST_BASE_URL") {envs.insert("ANTHROPIC_BASE_URL".into(),url);}
    envs.entry(OsString::from("CLAUDE_CODE_ENTRYPOINT"))
        .or_insert_with(|| OsString::from("locus-rs"));
    crate::network::extend_proxy_env_map(&mut envs);
    // Test-only CONNECT tunnel: TLS and all provider HTTP requests remain
    // owned by Claude. The tunnel can cut sockets, never recreate requests.
    #[cfg(debug_assertions)]
    if let Some(proxy) = std::env::var_os("LOCUS_CLAUDE_TEST_PROXY") {
        envs.insert("HTTPS_PROXY".into(), proxy.clone());
        envs.insert("HTTP_PROXY".into(), proxy);
        envs.remove(std::ffi::OsStr::new("ALL_PROXY"));
        envs.insert("NO_PROXY".into(), "127.0.0.1,localhost".into());
    }
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

    let settings = ssh::load_settings()?;
    let mut cmd = tokio::process::Command::new(&cli_path);
    cmd.kill_on_drop(true);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("--print")
        .arg("--output-format")
        .arg("json")
        .arg("--permission-mode")
        .arg(settings.permission_mode.as_str())
        .arg("--settings")
        .arg(settings.runtime.cli_settings(settings.permission_mode == ssh::PermissionMode::BypassPermissions).to_string())
        // Mirror run_turn's hermetic flags so the test reflects exactly what a
        // real Locus turn sees (settings.json is NOT consulted).
        .arg("--strict-mcp-config")
        .arg("--setting-sources")
        .arg("");
    // Deny every builtin: even with bypassPermissions the model must not be
    // able to touch the machine just because we sent it a prompt. The prompt is
    // delivered via stdin (below), never as a positional arg, so this variadic
    // flag can sit last without swallowing it.
    cmd.arg("--disallowed-tools");
    for tool in DISALLOWED_CLAUDE_BUILTINS {
        cmd.arg(tool);
    }
    let mut env = base_child_env();
    settings.runtime.apply_env(&mut env);
    cmd.env_clear().envs(env);

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

fn parse_assistant_message(message: &serde_json::Value) -> Option<ClaudeCodeAssistantMessage> {
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
                            .map(tui::tool_name)
                            .unwrap_or_default();
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                        let (name, arguments) = match native_tools::adapt(&id, &name, &input) {
                            Ok(call) => (call.locus_name.to_string(), call.locus_input.to_string()),
                            Err(_) => (name, input.to_string()),
                        };
                        if !id.is_empty() && !name.is_empty() {
                            let server_tool = (name == "WebSearch").then_some(crate::session::models::ServerToolKind::WebSearch);
                            parsed.tool_calls.push(ToolCallInfo {
                                id,
                                name,
                                arguments,
                                order: None,
                                server_tool,
                                server_tool_output: None,
                                server_tool_result: None,
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

fn mcp_tool_result_content(result: &ClaudeCodeToolResult) -> Vec<serde_json::Value> {
    let mut content = vec![json!({
        "type": "text",
        "text": result.text_with_context(),
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

pub(crate) fn normalize_claude_model(model: &str) -> String {
    let trimmed = model.trim();
    let short = trimmed.strip_prefix("claude_code/").unwrap_or(trimmed);
    let (base, suffix) = short
        .strip_suffix("[1m]")
        .map(|base| (base, "[1m]"))
        .unwrap_or((short, ""));
    let normalized = match base {
        "claude-opus-4.8" => "claude-opus-4-8",
        "claude-opus-4.7" => "claude-opus-4-7",
        "claude-sonnet-4.6" => "claude-sonnet-4-6",
        "claude-opus-4.6" => "claude-opus-4-6",
        "claude-haiku-4.5" => "claude-haiku-4-5",
        other => other,
    };
    format!("{}{}", normalized, suffix)
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
    fn native_agent_version_gate_reads_cli_version_output() {
        assert!(version_at_least("2.1.312 (Claude Code)", NATIVE_AGENTS_MIN_VERSION));
        assert!(version_at_least("2.2.0", NATIVE_AGENTS_MIN_VERSION));
        assert!(version_at_least("3.0.1", NATIVE_AGENTS_MIN_VERSION));
        assert!(!version_at_least("2.1.311 (Claude Code)", NATIVE_AGENTS_MIN_VERSION));
        assert!(!version_at_least("2.0.999", NATIVE_AGENTS_MIN_VERSION));
        assert!(!version_at_least("", NATIVE_AGENTS_MIN_VERSION));
        assert!(!version_at_least("Claude Code", NATIVE_AGENTS_MIN_VERSION));
    }

    #[test]
    fn normalize_claude_model_maps_current_aliases_and_ids() {
        assert_eq!(normalize_claude_model("sonnet"), "sonnet");
        assert_eq!(normalize_claude_model("fable"), "fable");
        assert_eq!(normalize_claude_model("claude_code/opus"), "opus");
        assert_eq!(normalize_claude_model("claude_code/claude-opus-5-5"), "claude-opus-5-5");
        assert_eq!(
            normalize_claude_model("claude_code/claude-opus-4.8[1m]"),
            "claude-opus-4-8[1m]"
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

        let parsed = parse_assistant_message(&message).expect("parsed message");
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
            context: vec!["Background task completed".into()],
            is_error: false,
            images: Some(vec![ImageData {
                capture: None,
                data: "aW1hZ2U=".to_string(),
                mime_type: "image/png".to_string(),
            }]),
        };

        let content = mcp_tool_result_content(&result);
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "captured\n\nBackground task completed");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["data"], "aW1hZ2U=");
        assert_eq!(content[1]["mimeType"], "image/png");
    }
}
