//! A real, persistent Claude terminal, with structured Mod events as the GUI
//! transport. ANSI output is bounded diagnostics only, never parsed as tool calls.
use super::*;
use portable_pty::{CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(test)]
#[path = "permission_mode_tests.rs"]
mod permission_mode_tests;
#[path = "native_agent_pump.rs"]
mod agent_pump;
pub(crate) use agent_pump::stop_native_agent;

#[path = "connection_status.rs"]
mod connection_status;
#[cfg(test)]
pub(crate) use connection_status::connection_status;
pub(crate) use connection_status::{connection_overview, ConnectionStatus};
#[path = "prewarm.rs"]
pub(crate) mod prewarm;
#[path = "startup_diagnostics.rs"]
mod startup_diagnostics;

enum SessionTransport {
    Ssh(super::ssh::Transport),
    #[cfg(feature = "server-client")]
    Server(super::server_transport::Transport),
}
macro_rules! transport_method {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            SessionTransport::Ssh(transport) => transport.$method($($arg),*),
            #[cfg(feature = "server-client")]
            SessionTransport::Server(transport) => transport.$method($($arg),*),
        }
    };
}
impl SessionTransport {
    fn stop(&self) { transport_method!(self, stop) }
    fn alive(&self) -> bool { transport_method!(self, alive) }
    fn failure(&self) -> String { transport_method!(self, failure) }
    fn latency_ms(&self) -> Option<f64> { transport_method!(self, latency_ms) }
    fn health_failure(&self) -> Option<String> { transport_method!(self, health_failure) }
    fn input(&self, text: &str) -> Result<(), String> { transport_method!(self, input, text) }
    fn send(&self, value: serde_json::Value) -> Result<(), String> { transport_method!(self, send, value) }
}

struct Session {
    #[cfg(windows)]
    job: Option<super::pty_job::PtyJob>,
    remote: Option<SessionTransport>,
    connection: Option<super::ssh::SshConfig>,
    bridge: tokio::sync::Mutex<mod_bridge::ModBridge>,
    options: Arc<std::sync::RwLock<serde_json::Value>>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    child: Option<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    _master: Option<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    ready: AtomicBool,
    prewarmed: AtomicBool,
    recovery_required: AtomicBool,
    fresh_start: bool,
    upstream_model: Mutex<Option<String>>,
    busy: AtomicBool,
    healthy: AtomicBool,
    session_id: Mutex<Option<String>>,
    cwd: String,
    model: String,
    permission_mode: super::ssh::PermissionMode,
    runtime_settings: super::runtime_settings::RuntimeSettings,
    attachment_root: String,
    controls: Arc<mod_bridge::ControlSignals>,
    accepting_inputs: AtomicBool,
    journal: super::native_journal::Journal,
    native_tool_search: bool,
    native_agents: bool,
    tool_catalog_path: PathBuf,
    tool_catalog: Mutex<super::tool_catalog::ToolCatalog>,
    last_used: Mutex<Instant>,
    /// Installed by the latest run; serves subagents between runs too.
    agent_host: agent_pump::AgentHostSlot,
    /// Subagent calls announced in this process (identity and dedup).
    agent_tools: Mutex<super::turn_tools::TurnTools>,
}
fn sessions() -> &'static Mutex<HashMap<String, Arc<Session>>> {
    static POOL: OnceLock<Mutex<HashMap<String, Arc<Session>>>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}
pub(crate) fn has_live_session(id: &str) -> bool {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|s| s.alive())
}
pub(crate) fn is_prewarmed(id: &str) -> bool {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|s| s.prewarmed.load(Ordering::SeqCst) && s.alive())
}
pub(crate) fn has_synchronized_session(id: &str) -> bool {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|s|
        s.alive() && !s.prewarmed.load(Ordering::SeqCst) && !s.recovery_required.load(Ordering::SeqCst))
}
pub(crate) fn require_recovery(id: &str) {
    if let Some(session) = sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id) {
        session.recovery_required.store(true, Ordering::SeqCst);
    }
}
pub(crate) fn is_unused_prewarm(id: &str) -> bool {
    sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|s| s.fresh_start && s.prewarmed.load(Ordering::SeqCst) && s.ready.load(Ordering::SeqCst) && s.alive())
}
/// Native JSONL of a local (non-SSH) TUI conversation. Claude files it under
/// the session profile as `projects/<sanitized cwd>/<native id>.jsonl`.
pub(crate) fn local_native_transcript(id: &str, native_id: &str) -> Option<std::path::PathBuf> {
    uuid::Uuid::parse_str(native_id).ok()?;
    let projects = super::ssh::profile_root(id).ok()?.join("profile").join("projects");
    std::fs::read_dir(projects).ok()?.flatten()
        .map(|entry| entry.path().join(format!("{native_id}.jsonl")))
        .find(|path| path.is_file())
}
pub(super) fn history_gate(id:&str)->Arc<tokio::sync::RwLock<()>> {
    static GATES:OnceLock<Mutex<HashMap<String,std::sync::Weak<tokio::sync::RwLock<()>>>>>=OnceLock::new();
    let mut gates=GATES.get_or_init(||Mutex::new(HashMap::new())).lock().unwrap_or_else(|e|e.into_inner());
    if let Some(gate)=gates.get(id).and_then(std::sync::Weak::upgrade) { return gate; }
    if gates.len()>256 { gates.retain(|_,gate|gate.strong_count()>0); }
    let gate=Arc::new(tokio::sync::RwLock::new(()));
    gates.insert(id.into(),Arc::downgrade(&gate));
    gate
}
pub(crate) fn close_session(id: &str) {
    if let Some(session) = sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id)
    {
        session.stop();
    }
}
/// After a cancelled turn: a process whose background agents still run stays,
/// as in Claude's own TUI. The turn itself was already interrupted with Esc.
pub(crate) fn close_cancelled_session(id: &str) {
    let leased = sessions().lock().unwrap_or_else(|e| e.into_inner()).get(id)
        .is_some_and(|session| session.leased() && session.alive());
    if !leased { close_session(id); }
}
pub(crate) fn close_all() {
    locus_remote_core::claude_ssh::archive_pool::shutdown();
    prewarm::shutdown();
    for (_, session) in sessions().lock().unwrap_or_else(|e| e.into_inner()).drain() {
        session.stop();
    }
}

const REMOTE_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

fn remote_idle_expired(last_used: Instant, now: Instant) -> bool {
    let timeout = super::ssh::load_settings().ok().map(|s| Duration::from_secs(u64::from(s.idle_minutes.clamp(1, 60)) * 60)).unwrap_or(REMOTE_IDLE_TIMEOUT);
    idle_expired_after(last_used, now, timeout)
}
fn idle_expired_after(last_used: Instant, now: Instant, timeout: Duration) -> bool {
    now.saturating_duration_since(last_used) >= timeout
}

fn remove_cached_session(id: &str, expected: &Arc<Session>) {
    let removed = {
        let mut pool = sessions().lock().unwrap_or_else(|e| e.into_inner());
        if pool.get(id).is_some_and(|current| Arc::ptr_eq(current, expected)) {
            pool.remove(id)
        } else { None }
    };
    if let Some(session) = removed { session.stop(); }
}

fn watch_remote_session(id: String, session: &Arc<Session>) {
    let weak = Arc::downgrade(session);
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let Some(session) = weak.upgrade() else { break; };
            // The same exclusive gate covers startup, turns and history edits,
            // so reaping cannot race a new turn or interrupt a pending tool.
            let Ok(_guard) = history_gate(&id).try_write_owned() else { continue; };
            let idle = remote_idle_expired(*session.last_used.lock().unwrap_or_else(|e| e.into_inner()), Instant::now())
                && !session.leased();
            let alive = session.alive();
            if !alive {
                if let Some(remote) = &session.remote { let _ = remote.failure(); }
            }
            if !alive || idle {
                remove_cached_session(&id, &session);
                break;
            }
        }
    });
}

/// Configuration changes retire only idle SSH sessions for other targets.
/// Running turns remain pinned to their recorded server and finish normally.
pub(crate) fn retire_idle_remote_connections(keep: Option<&super::ssh::SshConfig>) {
    locus_remote_core::claude_ssh::archive_pool::retire_except(keep);
    let candidates: Vec<_> = sessions().lock().unwrap_or_else(|e| e.into_inner()).iter()
        .filter(|(_, s)| s.connection.is_some() && s.connection.as_ref() != keep && !s.leased())
        .map(|(id, s)| (id.clone(), s.clone())).collect();
    for (id, session) in candidates {
        if let Ok(_guard) = history_gate(&id).try_write_owned() {
            remove_cached_session(&id, &session);
        }
    }
}
/// Retire idle processes now; a running turn applies new preferences when its lease ends.
pub(crate) fn retire_outdated_runtime_settings(settings: super::runtime_settings::RuntimeSettings) {
    prewarm::cancel_all();
    let candidates: Vec<_> = sessions().lock().unwrap_or_else(|e| e.into_inner()).iter()
        .filter(|(_, session)| session.runtime_settings != settings && !session.leased())
        .map(|(id, session)| (id.clone(), session.clone())).collect();
    for (id, session) in candidates {
        if let Ok(_guard) = history_gate(&id).try_write_owned() {
            remove_cached_session(&id, &session);
        }
    }
}

impl Session {
    fn stop(&self) {
        self.healthy.store(false, Ordering::SeqCst);
        #[cfg(windows)]
        if let Some(job) = &self.job { job.terminate(); }
        if let Some(child) = &self.child { let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill(); }
        if let Some(remote) = &self.remote { remote.stop(); }
    }
    fn write(&self, text: &str) -> Result<(), String> {
        if let Some(remote) = &self.remote { return remote.input(text); }
        let mut writer = self.writer.as_ref().ok_or("Claude terminal unavailable")?.lock().unwrap_or_else(|e| e.into_inner());
        writer
            .write_all(text.as_bytes())
            .and_then(|_| writer.flush())
            .map_err(|e| e.to_string())
    }
    fn alive(&self) -> bool {
        self.healthy.load(Ordering::SeqCst) && if let Some(remote) = &self.remote {
            remote.alive()
        } else {
            self.child.as_ref().is_some_and(|child| matches!(child.lock().unwrap_or_else(|e| e.into_inner()).try_wait(), Ok(None)))
        }
    }
    fn merge_tools(&self, incoming: &[ClaudeCodeToolDefinition]) -> Result<(Vec<ClaudeCodeToolDefinition>, Vec<ClaudeCodeToolDefinition>), String> {
        let mut catalog = self.tool_catalog.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = catalog.clone();
        let delta = next.merge(incoming)?;
        if next != *catalog {
            next.keep_pinned_route(&self.tool_catalog_path);
            next.save(&self.tool_catalog_path)?;
        }
        let snapshot = next.active_tools();
        *catalog = next;
        Ok((snapshot, delta))
    }
    fn configure(&self, mut options: serde_json::Value) -> Result<(), String> {
        self.with_agent_control(&mut options);
        *self.options.write().unwrap_or_else(|e| e.into_inner()) = options.clone();
        if let Some(remote) = &self.remote { remote.send(json!({"type":"configure","options":options}))?; }
        Ok(())
    }
}
impl Session {
    /// Subagent stop commands ride the Mod's input poll. Every configuration
    /// carries the current list, so a new turn never drops a pending stop.
    fn with_agent_control(&self, options: &mut serde_json::Value) {
        if self.native_agents && options.is_object() {
            options["agentControl"] = self.controls.agent_control();
        }
    }
    /// Running background agents keep this process: between turns, across a
    /// cancelled turn, and against idle reaping.
    fn leased(&self) -> bool {
        self.native_agents && self.controls.has_running_background_agents()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
struct TurnLease(Arc<Session>, bool);
impl Drop for TurnLease {
    fn drop(&mut self) {
        let mut options = self.0.options.read().unwrap_or_else(|e| e.into_inner()).clone();
        options["active"] = json!(false);
        let _ = self.0.configure(options);
        self.0.busy.store(false, Ordering::SeqCst);
        self.0.accepting_inputs.store(false, Ordering::SeqCst);
        *self.0.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        if !self.1 {
            // Interrupt first, then retire this process. A fresh resume never
            // consumes stale callbacks from an interrupted Locus run. Running
            // background agents keep it, as in Claude's own TUI: Esc cancels
            // the main turn and its foreground agents only. Stale callbacks of
            // the cancelled turn are refused by run identity.
            let _ = self.0.write("\x1b");
            let keep = self.0.leased() && self.0.alive()
                && !self.0.recovery_required.load(Ordering::SeqCst);
            if !keep { self.0.stop(); }
        } else if super::ssh::load_settings().is_ok_and(|settings| settings.runtime != self.0.runtime_settings) {
            self.0.stop();
        }
    }
}

fn configure_profile(profile: &Path, cwd: &Path, gateway_auth: bool) -> Result<(), String> {
    std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    if gateway_auth {
        let cached = profile.join(".credentials.json");
        if cached.is_file() {
            std::fs::remove_file(cached).map_err(|e| e.to_string())?;
        }
    } else if let Some(source) = claude_config_dir().map(|root| root.join(".credentials.json")) {
        if source.is_file() {
            std::fs::copy(source, profile.join(".credentials.json")).map_err(|e| e.to_string())?;
        }
    }
    let path = profile.join(".claude.json");
    let mut value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .filter(ValueExt::is_object_value)
        .unwrap_or(json!({}));
    value["hasCompletedOnboarding"] = json!(true);
    value["theme"] = json!("dark");
    value["lastOnboardingVersion"] = json!("2.1.299");
    let normalized = cwd.to_string_lossy().replace('\\', "/");
    if !value["projects"].is_object() {
        value["projects"] = json!({});
    }
    value["projects"][normalized] =
        json!({"hasTrustDialogAccepted":true,"hasClaudeMdExternalIncludesApproved":false});
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        let suffix: String = key
            .chars()
            .rev()
            .take(20)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        value["customApiKeyResponses"] = json!({"approved":[suffix],"rejected":[]});
    }
    std::fs::write(path, value.to_string()).map_err(|e| e.to_string())
}
// A function item avoids inference ambiguity in Option::filter.
struct ValueExt;
impl ValueExt {
    fn is_object_value(value: &serde_json::Value) -> bool {
        value.is_object()
    }
}

async fn start(options: &ClaudeCodeCliOptions, permission_mode: super::ssh::PermissionMode, _prewarm:bool) -> Result<Arc<Session>, String> {
    let runtime_settings = super::ssh::load_settings()?.runtime;
    #[cfg(feature = "server-client")]
    if crate::server_client::binding_for_model(&options.model)?.is_some() {
        return start_server(options, permission_mode, runtime_settings, _prewarm).await;
    }
    if let Some(remote) = &options.remote { return start_remote(options, remote, permission_mode, runtime_settings).await; }
    let cli = find_claude_cli().ok_or("Claude Code CLI was not found")?;
    let root = super::ssh::profile_root(&options.locus_session_id)?;
    let tool_catalog_path = super::tool_catalog::path(&options.locus_session_id)?;
    let tool_catalog = super::tool_catalog::ToolCatalog::load(&tool_catalog_path, options.native_tool_search)?;
    let cwd = root.join("terminal");
    let profile = root.join("profile");
    let plugin = root.join("mod");
    for dir in [&cwd, &plugin.join(".claude-plugin"), &plugin.join("hooks")] {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut child_env = base_child_env();
    runtime_settings.apply_env(&mut child_env);
    configure_profile(
        &profile,
        &cwd,
        child_env
            .get(std::ffi::OsStr::new("ANTHROPIC_AUTH_TOKEN"))
            .is_some_and(|value| !value.is_empty()),
    )?;
    std::fs::write(plugin.join(".claude-plugin/plugin.json"),r#"{"name":"locus","version":"0.2.0","description":"Locus GUI and local tool bridge"}"#).map_err(|e|e.to_string())?;
    std::fs::write(
        plugin.join("hooks/hooks.json"),
        r#"{"modules":["./register.js"]}"#,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(plugin.join("hooks/register.js"), native_mod_source())
        .map_err(|e| e.to_string())?;
    startup_diagnostics::validate_plugin(&cli, &plugin, &child_env).await
        .map_err(|error| super::turn_retry::with_context("Environment preparation", error))?;
    let bridge = mod_bridge::ModBridge::start().await?;
    *bridge.options.write().unwrap_or_else(|e| e.into_inner()) =
        json!({"transport":"tui","active":false,"systemPrompt":options.system_prompt,"tools":tool_catalog.active_tools(),"agents":options.agents});
    let mut command = CommandBuilder::new(cli);
    command.cwd(&cwd);
    command.env_clear();
    for (key, value) in child_env {
        command.env(key, value);
    }
    command.env("CLAUDE_CONFIG_DIR", &profile);
    command.env("LOCUS_CLAUDE_BRIDGE_URL", &bridge.url);
    command.env("LOCUS_CLAUDE_BRIDGE_TOKEN", &bridge.token);
    command.env("LOCUS_CLAUDE_TUI", "1");
    command.env("LOCUS_CLAUDE_NATIVE_TOOL_SEARCH", if options.native_tool_search { "1" } else { "0" });
    if options.native_tool_search { command.env("ENABLE_TOOL_SEARCH", "true"); }
    command.env("TERM", "xterm-256color");
    command.env("CLAUDE_CODE_ENABLE_PROMPT_SUGGESTION", "false");
    command.env_remove("CLAUDECODE");
    command.env_remove("CLAUDE_CODE_ENTRYPOINT");
    command.env_remove("CLAUDE_CODE_TASK_LIST_ID");
    if options.native_agents {
        for key in locus_remote_core::claude_launch::NATIVE_AGENT_REMOVED_ENV { command.env_remove(key); }
        for (key, value) in locus_remote_core::claude_launch::NATIVE_AGENT_ENV { command.env(key, value); }
    }
    command.env("NO_PROXY", "127.0.0.1,localhost");
    command.env("no_proxy", "127.0.0.1,localhost");
    command.args(["--plugin-dir"]);
    command.arg(&plugin);
    // Native-agent conversations add only the delegation tools; every other
    // launch keeps exactly the historical arguments (ssh_host.py mirrors this).
    let mut allowed_tools = vec!["TaskCreate", "TaskGet", "TaskList", "TaskUpdate", "WebSearch", "WebFetch", "ToolSearch"];
    if options.native_agents { allowed_tools.extend(["Agent", "SendMessage"]); }
    command.args(["--setting-sources", "", "--strict-mcp-config", "--allowedTools"]);
    command.args(&allowed_tools);
    command.args([
        "--disallowedTools", "mcp__locus__web_fetch",
        "--permission-mode",
        permission_mode.as_str(),
        "--system-prompt-snapshot",
        "off",
    ]);
    // Every loading mode pins the adapted builtins. Without the list Claude
    // also offers NotebookEdit and other tools the Mod always denies.
    command.args(["--tools", &locus_remote_core::claude_launch::native_tools(options.native_agents)]);
    // Explicit CLI settings remain active with --setting-sources "".
    command.args(["--settings", &runtime_settings.cli_settings(
        permission_mode == super::ssh::PermissionMode::BypassPermissions,
    ).to_string()]);
    let model = normalize_claude_model(&options.model);
    if !model.is_empty() {
        command.args(["--model", &model]);
    }
    command.args(super::fork::resume_args(&root,options.resume_session_id.as_deref())?);
    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows: 40,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| format!("TUI initialization: Failed to start Claude TUI: {e}"))?;
    #[cfg(windows)]
    let job = match child
        .process_id()
        .ok_or("Claude process ID is unavailable".to_string())
        .and_then(super::pty_job::PtyJob::attach)
    {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            return Err(format!("Failed to own Claude process tree: {error}"));
        }
    };
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().map_err(|e| e.to_string())?,
    ));
    let diagnostic = Arc::new(Mutex::new(Vec::<u8>::new()));
    let buffer = diagnostic.clone();
    let input = writer.clone();
    std::thread::spawn(move || {
        let mut bytes = [0u8; 8192];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 {
                break;
            }
            let chunk = &bytes[..count];
            if chunk.windows(4).any(|part| part == b"\x1b[6n") {
                let _ = input
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .write_all(b"\x1b[1;1R");
            }
            let mut buffer = buffer.lock().unwrap_or_else(|e| e.into_inner());
            buffer.extend_from_slice(chunk);
            if buffer.len() > 65536 {
                let excess = buffer.len() - 65536;
                buffer.drain(..excess);
            }
        }
    });
    Ok(agent_pump::with_agent_pump(Arc::new(Session {
        #[cfg(windows)]
        job: Some(job),
        remote: None,
        connection: None,
        options: bridge.options.clone(),
        controls: bridge.controls.clone(),
        accepting_inputs: AtomicBool::new(false),
        bridge: tokio::sync::Mutex::new(bridge),
        writer: Some(writer),
        child: Some(Mutex::new(child)),
        _master: Some(Mutex::new(pair.master)),
        diagnostic,
        ready: AtomicBool::new(false),
        prewarmed: AtomicBool::new(false), recovery_required: AtomicBool::new(false), upstream_model: Mutex::new(None),
        fresh_start: options.resume_session_id.is_none(),
        busy: AtomicBool::new(false),
        healthy: AtomicBool::new(true),
        session_id: Mutex::new(None),
        cwd: options.cwd.clone(),
        model: options.model.clone(),
        permission_mode,
        runtime_settings,
        attachment_root: root.to_string_lossy().into_owned(),
        journal: super::native_journal::Journal::new(root.join("calls")),
        native_tool_search: options.native_tool_search,
        native_agents: options.native_agents,
        tool_catalog_path, tool_catalog: Mutex::new(tool_catalog),
        last_used: Mutex::new(Instant::now()),
        agent_host: Default::default(), agent_tools: Default::default(),
    }), &options.locus_session_id))
}

async fn start_remote(options: &ClaudeCodeCliOptions, config: &super::ssh::SshConfig, permission_mode: super::ssh::PermissionMode, runtime_settings: super::runtime_settings::RuntimeSettings) -> Result<Arc<Session>, String> {
    let (bridge, send) = mod_bridge::ModBridge::channel();
    let tool_catalog_path = super::tool_catalog::path(&options.locus_session_id)?;
    let tool_catalog = super::tool_catalog::ToolCatalog::load(&tool_catalog_path, options.native_tool_search)?;
    let initial = json!({"transport":"tui","active":false,"systemPrompt":options.system_prompt,"tools":tool_catalog.active_tools(),"agents":options.agents});
    *bridge.options.write().unwrap_or_else(|e| e.into_inner()) = initial.clone();
    let mut request=json!({
        "mode":"session", "authorizedSetup":true, "sessionKey":super::ssh::session_key(&options.locus_session_id, config),
        "modSource":native_mod_source(), "token":bridge.token, "options":initial,
        "model":normalize_claude_model(&options.model), "resume":options.resume_session_id,
    });
    locus_remote_core::claude_launch::LaunchOptions{permission_mode,runtime_settings,native_tool_search:options.native_tool_search,native_agents:options.native_agents}.apply(&mut request)?;
    let (remote,info)=super::ssh::Transport::start(config,request,Some((send,bridge.controls.clone(),bridge.options.clone()))).await
        .map_err(|error| {
            // Bound terminal details without truncating the stage or retry tag.
            let (summary, detail) = error.split_once('\n').unwrap_or((&error, ""));
            startup_diagnostics::message(summary, detail.as_bytes())
        })?;
    super::ssh::record_launched_cli(config, &info);
    let diagnostic = remote.diagnostic.clone();
    Ok(agent_pump::with_agent_pump(Arc::new(Session {
        #[cfg(windows)] job: None,
        remote: Some(SessionTransport::Ssh(remote)), connection: Some(config.clone()), options: bridge.options.clone(),
        controls: bridge.controls.clone(),
        accepting_inputs: AtomicBool::new(false),
        bridge: tokio::sync::Mutex::new(bridge), writer: None, child: None, _master: None, diagnostic,
        ready: AtomicBool::new(false), prewarmed: AtomicBool::new(false), recovery_required: AtomicBool::new(false), upstream_model: Mutex::new(None), busy: AtomicBool::new(false), healthy: AtomicBool::new(true),
        fresh_start: options.resume_session_id.is_none(),
        session_id: Mutex::new(None), cwd: options.cwd.clone(), model: options.model.clone(),
        permission_mode,
        runtime_settings,
        attachment_root: info.session_root.ok_or("SSH host did not provide its attachment directory")?,
        journal: super::native_journal::Journal::new(super::ssh::profile_root(&options.locus_session_id)?.join("calls")),
        native_tool_search: options.native_tool_search,
        native_agents: options.native_agents,
        tool_catalog_path, tool_catalog: Mutex::new(tool_catalog),
        last_used: Mutex::new(Instant::now()),
        agent_host: Default::default(), agent_tools: Default::default(),
    }), &options.locus_session_id))
}

#[cfg(feature = "server-client")]
async fn start_server(options: &ClaudeCodeCliOptions, permission_mode: super::ssh::PermissionMode, runtime_settings: super::runtime_settings::RuntimeSettings, prewarm:bool) -> Result<Arc<Session>, String> {
    let (bridge, send) = mod_bridge::ModBridge::channel();
    let tool_catalog_path = super::tool_catalog::path(&options.locus_session_id)?;
    let tool_catalog = super::tool_catalog::ToolCatalog::load(&tool_catalog_path, options.native_tool_search)?;
    let initial = json!({"transport":"tui","active":false,"systemPrompt":options.system_prompt,"tools":tool_catalog.active_tools(),"agents":options.agents});
    *bridge.options.write().unwrap_or_else(|e| e.into_inner()) = initial.clone();
    let (remote, info) = super::server_transport::Transport::start(&options.model, &options.locus_session_id, options.resume_session_id.as_deref(), initial, locus_remote_core::claude_launch::LaunchOptions{permission_mode,runtime_settings,native_tool_search:options.native_tool_search,native_agents:options.native_agents},prewarm,send, bridge.controls.clone(), bridge.options.clone()).await?;
    let diagnostic = remote.diagnostic.clone();
    Ok(agent_pump::with_agent_pump(Arc::new(Session {
        #[cfg(windows)] job: None,
        remote: Some(SessionTransport::Server(remote)), connection: None, options: bridge.options.clone(),
        controls: bridge.controls.clone(), accepting_inputs: AtomicBool::new(false),
        bridge: tokio::sync::Mutex::new(bridge), writer: None, child: None, _master: None, diagnostic,
        ready: AtomicBool::new(false), prewarmed: AtomicBool::new(false), recovery_required: AtomicBool::new(false), upstream_model: Mutex::new(None), busy: AtomicBool::new(false), healthy: AtomicBool::new(true),
        fresh_start: options.resume_session_id.is_none(), session_id: Mutex::new(None), cwd: options.cwd.clone(), model: options.model.clone(),
        permission_mode, runtime_settings, attachment_root: info.session_root.unwrap_or_else(|| "locus-server".into()),
        journal: super::native_journal::Journal::new(super::ssh::profile_root(&options.locus_session_id)?.join("calls")),
        native_tool_search: options.native_tool_search, native_agents: options.native_agents, tool_catalog_path, tool_catalog: Mutex::new(tool_catalog), last_used: Mutex::new(Instant::now()),
        agent_host: Default::default(), agent_tools: Default::default(),
    }), &options.locus_session_id))
}

pub(super) async fn run<H: ClaudeCodeHost>(
    options: ClaudeCodeCliOptions,
    user: serde_json::Value,
    host: &mut H,
) -> Result<ClaudeCodeTurnResult, String> {
    run_with_inputs(options,user,host,None).await
}

pub(super) async fn run_with_inputs<H: ClaudeCodeHost>(
    options:ClaudeCodeCliOptions,user:serde_json::Value,host:&mut H,source:Option<&dyn ClaudeCodeInputSource>,
) -> Result<ClaudeCodeTurnResult,String> {
    host.on_phase(crate::session::models::ClaudeCodePhase::Starting);
    let _history_guard = prewarm::turn_gate(&options.locus_session_id).await?;
    let settings = super::ssh::load_settings()?;
    let permission_mode = settings.permission_mode.for_launch(options.native_agents);
    let existing = sessions()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&options.locus_session_id)
        .cloned();
    let session = if let Some(session) = existing.filter(|session| {
        session.cwd == options.cwd
            && session.connection == options.remote
            && normalize_claude_model(&session.model) == normalize_claude_model(&options.model)
            && session.permission_mode == permission_mode
            && session.runtime_settings == settings.runtime
            && session.native_tool_search == options.native_tool_search
            && session.native_agents == options.native_agents
            && session.alive()
            && options.resume_session_id.as_ref().is_none_or(|expected| {
                session
                    .session_id
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    == Some(expected)
            })
    }) {
        session
    } else {
        close_session(&options.locus_session_id);
        connection_status::start_cached(&options, permission_mode).await?
    };
    if session.busy.swap(true, Ordering::SeqCst) {
        return Err("Claude TUI is already processing this conversation".into());
    }
    session.prewarmed.store(false, Ordering::SeqCst);
    let mut lease = TurnLease(session.clone(), false);
    let result = tokio::select! {
        result = run_session(&session, options, user, host, source) => result,
        error = drive_inputs(&session,source) => Err(error),
        error = watch_bridge_health(&session) => Err(error),
        _ = async {
            while session.alive() { tokio::time::sleep(Duration::from_millis(100)).await; }
        } => Err(if session.ready.load(Ordering::SeqCst) {
            disconnected_error(&session, "Claude TUI exited during the turn")
        } else { startup_error(&session, "Claude exited before initialization.") }),
    };
    lease.1 = result.is_ok();
    result
}

#[cfg(test)]
mod connection_lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn native_image_submission_waits_for_each_image_and_the_final_text_receipt() {
        let controls = mod_bridge::ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let submit = submit_native_input(&controls, |text| sent.send(text.to_string()).map_err(|e| e.to_string()), "run", &["C:/image one.png", "/tmp/image-two.jpg"]);
        let receipts = async {
            assert!(received.try_recv().is_err());
            controls.route(&json!({"type":"input_images","runId":"run","id":"run","count":0}), &options);
            assert_eq!(received.recv().await.unwrap(), super::super::attachments::image_paste("C:/image one.png").unwrap());
            assert!(received.try_recv().is_err());
            controls.route(&json!({"type":"input_images","runId":"run","id":"run","count":1}), &options);
            assert_eq!(received.recv().await.unwrap(), super::super::attachments::image_paste("/tmp/image-two.jpg").unwrap());
            assert!(received.try_recv().is_err());
            controls.route(&json!({"type":"input_images","runId":"run","id":"run","count":2}), &options);
            assert!(received.try_recv().is_err());
            controls.route(&json!({"type":"input_ready","runId":"run","id":"run"}), &options);
            assert_eq!(received.recv().await.unwrap(), "\r");
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async { tokio::join!(submit, receipts) }).await.unwrap();
        result.unwrap();
    }

    #[tokio::test]
    async fn native_image_staging_failure_never_sends_enter() {
        let controls = mod_bridge::ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        controls.route(&json!({"type":"input_images","runId":"run","id":"run","count":0}), &options);
        let submit = submit_native_input(&controls, |text| sent.send(text.to_string()).map_err(|e| e.to_string()), "run", &["/tmp/image.png"]);
        let fail = async {
            assert!(received.recv().await.unwrap().starts_with("\x1b[200~"));
            controls.route(&json!({"type":"input_staging_error","runId":"run","id":"run","error":"Image disappeared"}), &options);
        };
        let (result, ()) = tokio::join!(submit, fail);
        assert_eq!(result, Err("Image disappeared".into()));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn claude_code_phase_ignores_stale_steps_duplicate_sequences_and_unknown_modes() {
        use crate::session::models::ClaudeCodePhase;
        let mut sequence = 0;
        let event = json!({"stepId":2,"sequence":1,"phase":"thinking"});
        assert_eq!(accept_phase(&event, None, &mut sequence), None);
        assert_eq!(accept_phase(&event, Some(3), &mut sequence), None);
        assert_eq!(accept_phase(&event, Some(2), &mut sequence), Some(ClaudeCodePhase::Thinking));
        assert_eq!(sequence, 1);
        assert_eq!(accept_phase(&event, Some(2), &mut sequence), None);
        assert_eq!(accept_phase(&json!({"stepId":2,"sequence":2,"phase":"unknown"}), Some(2), &mut sequence), None);
        assert_eq!(sequence, 1);
        assert_eq!(accept_phase(&json!({"stepId":2,"sequence":2,"phase":"responding"}), Some(2), &mut sequence), Some(ClaudeCodePhase::Responding));
    }

    #[test]
    fn idle_expiration_is_bounded_and_resets_after_use() {
        let used = Instant::now();
        // Unit tests must not inherit the signed-in user's idle setting.
        for timeout in [Duration::from_secs(60), REMOTE_IDLE_TIMEOUT, Duration::from_secs(3600)] {
            assert!(!idle_expired_after(used, used + timeout - Duration::from_secs(1), timeout));
            assert!(idle_expired_after(used, used + timeout, timeout));
            assert!(!idle_expired_after(used + timeout, used + timeout, timeout));
        }
    }

    #[tokio::test]
    async fn startup_and_running_turns_exclude_reaping_and_duplicate_starts() {
        let id = uuid::Uuid::new_v4().to_string();
        let guard = history_gate(&id).try_write_owned().unwrap();
        assert!(history_gate(&id).try_write_owned().is_err());
        assert!(history_gate(&uuid::Uuid::new_v4().to_string()).try_write_owned().is_ok());
        drop(guard);
        assert!(history_gate(&id).try_write_owned().is_ok());
    }
}

fn startup_error(session: &Session, summary: &str) -> String {
    let error = startup_diagnostics::message(summary, &session.diagnostic.lock().unwrap_or_else(|e| e.into_inner()));
    let error = if !super::turn_retry::startup_rejected(&error) && session.remote.as_ref().is_some_and(|remote|
        !remote.alive() && super::turn_retry::is_disconnected(&remote.failure())) {
        super::turn_retry::disconnected(error)
    } else { super::turn_retry::ssh_startup_error(error) };
    super::turn_retry::with_context("TUI initialization", error)
}

fn disconnected_error(session: &Session, summary: &str) -> String {
    let reason = session.remote.as_ref().map(|remote| remote.failure())
        .unwrap_or_else(|| super::turn_retry::disconnected(summary));
    startup_diagnostics::message(&format!("{reason} ({summary})"), &session.diagnostic.lock().unwrap_or_else(|e| e.into_inner()))
}

async fn await_ready<H: ClaudeCodeHost>(
    session: &Session,
    options: &ClaudeCodeCliOptions,
    host: &mut H,
    bridge: &mut mod_bridge::ModBridge,
) -> Result<(), String> {
    if session.ready.load(Ordering::SeqCst) {
        let id = session.session_id.lock().unwrap_or_else(|e| e.into_inner()).clone().ok_or("Claude native identity unavailable")?;
        host.on_session_started(&id)?;
        if let Some(model) = session.upstream_model.lock().unwrap_or_else(|e| e.into_inner()).as_deref() { host.on_model_resolved(model)?; }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !session.ready.load(Ordering::SeqCst) {
        let callback = tokio::select! {
            result = tokio::time::timeout_at(deadline, bridge.calls.recv()) => result
                .map_err(|_| startup_error(session, "Claude TUI did not initialize. Check Claude login, workspace trust and plugin support."))?
                .ok_or_else(|| startup_error(session, "Claude bridge ended before initialization."))?,
            _ = async {
                while session.alive() { tokio::time::sleep(Duration::from_millis(100)).await; }
            } => return Err(startup_error(session, "Claude exited before initialization.")),
        };
        if callback.payload["ready"] == true {
            let id = callback.payload["sessionId"].as_str().ok_or("Claude TUI did not identify its native conversation")?;
            if options.resume_session_id.as_deref().is_some_and(|expected| expected != id) {
                return Err("Claude resumed a different native conversation; no input was submitted".into());
            }
            // Persist identity before sending input, including a first turn
            // whose SSH link drops before turn.start reaches Locus.
            host.on_session_started(id)?;
            if let Some(model) = callback.payload["model"].as_str().filter(|value| !value.trim().is_empty()) {
                *session.upstream_model.lock().unwrap_or_else(|e| e.into_inner()) = Some(model.to_string());
                host.on_model_resolved(model)?;
            }
            *session.session_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.to_string());
            session.ready.store(true, Ordering::SeqCst);
        }
        let _ = callback.reply.send(json!({"ok":true}));
    }
    Ok(())
}

fn accept_phase(
    message: &serde_json::Value,
    step: Option<u64>,
    sequence: &mut u64,
) -> Option<crate::session::models::ClaudeCodePhase> {
    if step.is_none() || message["stepId"].as_u64() != step { return None; }
    let next = message["sequence"].as_u64().filter(|next| *next > *sequence)?;
    let phase = serde_json::from_value(message["phase"].clone()).ok()?;
    *sequence = next;
    Some(phase)
}

async fn run_session<H: ClaudeCodeHost>(
    session: &Session,
    options: ClaudeCodeCliOptions,
    user: serde_json::Value,
    host: &mut H,
    source: Option<&dyn ClaudeCodeInputSource>,
) -> Result<ClaudeCodeTurnResult, String> {
    let mut bridge = session.bridge.lock().await;
    await_ready(session, &options, host, &mut bridge).await?;
    // This run's authority serves the conversation's subagents, also after
    // the run ends (background agents) and until the next run replaces it.
    if session.native_agents {
        if let Some(agent_host) = host.native_agent_host() {
            *session.agent_host.lock().unwrap_or_else(|e| e.into_inner()) = Some(agent_host);
        }
    }
    let prepared = super::attachments::prepare(&user, &options.cwd)?;
    let input_id = format!("LOCUS_INPUT_{}", uuid::Uuid::new_v4().simple());
    let paths = prepared.paths(&session.attachment_root, &input_id);
    let prompt = prepared.prompt(&paths);
    let (tools, _) = session.merge_tools(&options.tools)?;
    session.controls.begin_turn(&input_id);
    let configuration = json!({"transport":"tui","active":true,"inputId":input_id,"prompt":prompt,
        "systemPrompt":options.system_prompt,"model":normalize_claude_model(&options.model),"effort":options.effort.as_deref().filter(|e|*e!="none"),"tools":tools,"agents":options.agents,
        "control":user["locusControl"],"messageId":user["locusMessageId"],"pendingUsers":user["locusPendingUsers"],
        "terminalText":if user["locusControl"]=="compact" {"/compact".to_string()} else {prepared.terminal_text(&input_id,&paths)},"imageCount":prepared.image_count(),"expectedImageCount":prepared.image_count(),"attachmentMappings":prepared.mappings(&paths)});
    let mut configuration = configuration;
    session.with_agent_control(&mut configuration);
    if let Some(remote) = &session.remote {
        *session.options.write().unwrap_or_else(|e|e.into_inner()) = configuration.clone();
        remote.send(json!({"type":"configure","options":configuration,"uploads":prepared.files}))?;
    } else {
        prepared.write(Path::new(&session.attachment_root), &input_id)?;
        session.configure(configuration.clone())?;
    }
    submit_native_input(&session.controls, |text| session.write(text), &input_id, &prepared.image_paths(&paths)).await?;
    let mut result = ClaudeCodeTurnResult::default();
    result.raw_request =
        json!({"transport":if options.remote.is_some() {"ssh-tui"} else {"tui"},"connection":options.remote,"system":options.system_prompt,"user":user}).to_string();
    result.claude_session_id = session
        .session_id
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let mut tools = super::turn_tools::TurnTools::default();
    let mut started = false;
    let mut activity_step = None;
    let mut activity_sequence = 0;
    let mut diagnostics = super::diagnostics::TurnDiagnostics::default();
    let startup = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let callback = tokio::select! {
            callback=bridge.calls.recv()=>callback.ok_or_else(|| disconnected_error(session, "Claude bridge closed"))?,
            _=tokio::time::sleep(Duration::from_secs(1))=>{
                if !session.alive(){return Err(disconnected_error(session, "Claude TUI exited during the turn"));}
                if !started && tokio::time::Instant::now()>startup{return Err("Claude TUI did not accept the Locus input".into());}
                continue;
            }
        };
        let message = &callback.payload;
        if message["runId"] != input_id {
            let _ = callback
                .reply
                .send(json!({"deny":"Stale Locus turn","callId":message["callId"]}));
            continue;
        }
        result.raw_response.push_str(&message.to_string());
        result.raw_response.push('\n');
        let mut reply = json!({"ok":true});
        let mut done = false;
        match message["type"].as_str().unwrap_or("call") {
            "checkpoint" => {
                if let (Some(source),Some(id))=(source,message["submissionId"].as_str()){
                    source.accepted(id,&json!({"version":message["version"],"uuid":message["uuid"],"sessionId":message["sessionId"]}))?;
                }
                host.on_checkpoint(message)?;
            },
            "compact_start" | "compact_done" => host.on_compaction(message)?,
            "compact_prepare" => reply = host.prepare_compaction(message).await,
            "diagnostic" => {
                if let Some(notice) = diagnostics.record(message) { host.on_diagnostic(notice); }
            },
            "input_error" => return Err(message["error"].as_str().unwrap_or("Claude input failed").into()),
            "input_ready" => {},
            "step_start" => {
                diagnostics.start_step();
                activity_step = message["stepId"].as_u64();
                activity_sequence = 0;
                for id in tools.start_step(activity_step.unwrap_or(0)) {
                    host.on_tool_call_abandoned(&id);
                }
                host.on_step_started(activity_step.unwrap_or(0))?;
                host.on_phase(crate::session::models::ClaudeCodePhase::Requesting);
                if let Some(model) = message["model"].as_str().filter(|value| !value.trim().is_empty()) {
                    host.on_model_resolved(model)?;
                }
            }
            "phase" => {
                // A render callback may finish after the next model request starts.
                if let Some(phase) = accept_phase(message, activity_step, &mut activity_sequence) {
                    host.on_phase(phase);
                }
            }
            "usage" => {
                result.upstream_model = message["usage"]["model"].as_str().filter(|value| !value.trim().is_empty()).map(str::to_owned);
                host.on_step_usage(&message["usage"], &message["context"], &message["timing"], message["stepId"].as_u64().unwrap_or(0))?;
            }
            "turn_start" | "command_start" => {
                started = true;
                if message["type"]=="turn_start" {session.accepting_inputs.store(true,Ordering::SeqCst);}
                if let Some(id) = result.claude_session_id.as_deref() {
                    host.on_session_started(id)?;
                }
            }
            "text" => host.on_text_delta(message["text"].as_str().unwrap_or_default().into()),
            "thinking" => {
                host.on_thinking_delta(message["text"].as_str().unwrap_or_default().into())
            }
            "tool_start" => {
                let id = message["id"].as_str().unwrap_or_default();
                tools.start(id);
                host.on_tool_call_start(id.into(),
                    native_tools::locus_name(&tool_name(message["name"].as_str().unwrap_or_default())).into());
            }
            // Subagent callbacks belong to the session's agent pump. Without
            // one (native agents off) nothing runs on a subagent's behalf.
            "assistant" | "interrupted_assistant" | "tool_result" if message["agentId"].as_str().is_some_and(|id| !id.is_empty()) => {
                reply = json!({"deny":"Native subagent execution is not enabled"});
            }
            "assistant" | "interrupted_assistant" => {
                let mut projected = message.clone();
                let mut agent_calls = Vec::new();
                if let Some(blocks) = projected["message"]["content"].as_array_mut() {
                    let mut had_tools = false;
                    let mut has_new_tool = false;
                    for block in std::mem::take(blocks) {
                        if block["type"] == "tool_use" {
                            had_tools = true;
                            if !tools.announce(&block)? { continue; }
                            has_new_tool = true;
                            if block["name"] == "Agent" {
                                agent_calls.push(block["id"].as_str().unwrap_or_default().to_string());
                            }
                        }
                        blocks.push(block);
                    }
                    // A lost receipt can make a Mod catch re-observe the same
                    // envelope. Persist neither its tool row nor its text twice.
                    if had_tools && !has_new_tool {
                        let _ = callback.reply.send(json!({"ok":true}));
                        continue;
                    }
                }
                if let Some(parsed) = parse_assistant_message(&projected) {
                    if message["type"]=="interrupted_assistant" {host.on_interrupted_message(parsed)?;}
                    else {
                        result.final_text = parsed.text.clone();
                        host.on_assistant_message(parsed)?;
                    }
                }
                // The Mod announces a call just before Claude starts it. Its
                // subagent takes its own workspace locks, so the parent batch's
                // completed writes end now, before the receipt lets it start.
                for id in agent_calls {
                    host.before_native_agent(&id).await;
                }
            }
            "complete" => {
                if let Some(error) = diagnostics.completion_error(message, tools.has_pending()) {
                    return Err(error);
                }
                // The final text comes only from structured model messages.
                // A completion/status envelope is never conversation content.
                result.input_tokens = message["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
                result.output_tokens =
                    message["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
                result.cache_read_tokens = message["usage"]["cache_read_input_tokens"]
                    .as_u64()
                    .unwrap_or(0) as u32;
                result.cache_write_tokens = message["usage"]["cache_creation_input_tokens"]
                    .as_u64()
                    .unwrap_or(0) as u32;
                done = true;
            }
            "command" => {
                if user["locusControl"] != "compact" {
                    return Err("Unexpected Claude command outside a GUI control action".into());
                }
                done = true;
            }
            "tool_result" => {
                let id = message["id"].as_str().unwrap_or_default();
                if tools.complete(id)? {
                    host.on_external_tool_result(
                        id,
                        crate::tool::ToolResult {
                            output: message["output"].as_str().unwrap_or_default().into(),
                            is_error: message["isError"] == true,
                        }.into(),
                    )
                    .await?;
                }
            }
            "call" => {
                if user["locusControl"] == "compact" {
                    return Err("Claude attempted tool execution during compaction".into());
                }
                let id = message["callId"].as_str().unwrap_or_default();
                let name = message["tool"].as_str().unwrap_or_default();
                if callback.reply.is_closed() {
                    continue;
                }
                if message.get("agentId").is_some_and(|value| !value.is_null()) {
                    reply = json!({"callId":id,"deny":"Native subagent execution is not enabled"});
                } else if !tools.calls.get(id).is_some_and(|call| {
                    call["name"] == name
                        && native_tools::same_operation(id, name, &call["input"], &message["input"])
                }) {
                    reply = json!({"callId":id,"deny":"Call does not match the current assistant message"});
                } else if !tools.complete(id)? {
                    reply = json!({"callId":id,"deny":"Duplicate tool call was not executed"});
                } else {
                    match session.journal.begin(message) {
                        Err(error) => reply = json!({"callId":id,"deny":error}),
                        Ok(Some(previous)) => {
                            reply = previous;
                            if reply.get("result").is_some() {
                                host.on_external_tool_result(id, super::native_output::replay(name, &reply)).await?;
                            }
                        }
                        Ok(None) => {
                            if let Some(name) = name.strip_prefix("mcp__locus__") {
                                let output =
                                    tokio::select! {
                                        biased;
                                        _=session.controls.wait("turn_interrupted",message["executionId"].as_str().ok_or("Missing Claude tool execution scope")?,None)=>interrupted_tool(),
                                        output=host.execute_tool(id,name,message["input"].clone())=>output,
                                    };
                                reply = if output.is_error || output.output == crate::session::history::INTERRUPTED_TOOL_RESULT {
                                    json!({"callId":id,"deny":output.text_with_context()})
                                } else {
                                    json!({"callId":id,"result":mcp_tool_result_content(&output)})
                                };
                                reply["locusOutput"] = json!(output.text_with_context());
                            } else {
                                match native_tools::adapt(id, &tool_name(name), &message["input"]) {
                                    Ok(native) => {
                                        let scope = native_scope::NativeInvocation::new(
                                            native.clone(),
                                            options.locus_session_id.clone(),
                                        );
                                        let output = tokio::select! {
                                            biased;
                                            _=session.controls.wait("turn_interrupted",message["executionId"].as_str().ok_or("Missing Claude tool execution scope")?,None)=>interrupted_tool(),
                                            output=scope.run(host.execute_tool(id,native.locus_name,native.locus_input.clone()))=>output,
                                        };
                                        reply = native_tools::finish_result(
                                            &native,
                                            &output,
                                            scope.result(),
                                        );
                                        reply["locusOutput"] = json!(output.text_with_context());
                                    }
                                    Err(error) => reply = json!({"callId":id,"deny":error}),
                                }
                            }
                            if let Err(error) = session.journal.complete(id, &reply) {
                                return Err(format!("Failed to persist local tool outcome: {error}. The operation will not be automatically repeated."));
                            }
                        }
                    }
                }
                // Journal the operation's outcome first. Catalog failures must
                // stop inference, never turn a completed write into a retry.
                if reply.get("result").is_some() {
                    if let Some(tools) = host.refresh_tools().await? {
                        let (_, delta) = session.merge_tools(&tools)?;
                        if !delta.is_empty() { reply["toolCatalogDelta"] = json!(delta); }
                    }
                }
                if let Some(deny) = reply["deny"].as_str() {
                    if message.get("agentId").is_none_or(serde_json::Value::is_null) {
                        let error: ClaudeCodeToolResult = crate::tool::ToolResult { output: deny.to_string(), is_error: true }.into();
                        host.on_external_tool_result(id, error).await?;
                    }
                }
            }
            _ => {}
        }
        let _ = callback.reply.send(reply);
        if done {
            return Ok(result);
        }
    }
}

async fn submit_native_input(
    controls: &mod_bridge::ControlSignals,
    write: impl Fn(&str) -> Result<(), String>,
    input_id: &str,
    image_paths: &[&str],
) -> Result<(), String> {
    // Each acknowledged marker means the native paste handler finished loading
    // that image. Never paste the next path or press Enter while it is decoding.
    if !image_paths.is_empty() {
        tokio::time::timeout(Duration::from_secs(30), controls.wait_input("input_images", input_id, Some(0))).await
            .map_err(|_| "Claude could not prepare the image input box")??;
        for (index, path) in image_paths.iter().enumerate() {
            write(&super::attachments::image_paste(path)?)?;
            tokio::time::timeout(Duration::from_secs(30), controls.wait_input("input_images", input_id, Some(index as u64 + 1))).await
                .map_err(|_| format!("Claude did not finish loading image {} of {}; no input was submitted", index + 1, image_paths.len()))??;
        }
    }
    // The Mod appends text after the image markers; ConPTY and SSH both wait
    // for its acknowledgement before Enter rather than relying on a delay.
    tokio::time::timeout(Duration::from_secs(30), controls.wait_input("input_ready", input_id, None)).await
        .map_err(|_| "Claude could not prepare the native input box")??;
    write("\r")
}

async fn watch_bridge_health(session: &Session) -> String {
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let id = {
            let options = session.options.read().unwrap_or_else(|e| e.into_inner());
            (options["active"] == true).then(|| options["inputId"].as_str().map(str::to_owned)).flatten()
        };
        let Some(id) = id else { continue; };
        let transport_failure = session.remote.as_ref().and_then(|remote| remote.health_failure());
        if let Some(error) = session.controls.health_error_with_transport(&id, transport_failure.as_deref()) {
            session.recovery_required.store(true, Ordering::SeqCst);
            // TurnLease retires this process. Proven transport loss uses the
            // bounded retry loop, which reconciles native history and persisted
            // tool outcomes before any input; isolated hook faults still stop.
            let transport = match session.remote.as_ref() {
                Some(remote) => match remote.latency_ms() {
                    Some(ms) => format!("SSH transport heartbeat is responsive (last RTT {ms:.0} ms)."),
                    None if transport_failure.is_some() => "SSH transport failure was detected.".into(),
                    None => "No recent SSH transport heartbeat is available; hook failure and transport loss cannot be distinguished.".into(),
                },
                None => "Local Claude bridge.".into(),
            };
            eprintln!("[Claude] bridge health failed: {error} {transport}");
            return format!("{error} {transport}");
        }
    }
}

async fn drive_inputs(session:&Session,source:Option<&dyn ClaudeCodeInputSource>)->String {
    let Some(source)=source else {return std::future::pending().await;};
    let result:Result<(),String>=async {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if !session.accepting_inputs.load(Ordering::SeqCst){continue;}
            let Some(input)=source.claim()? else {continue;};
            let tools=source.refresh_tools().await?;
            let prepared=super::attachments::prepare(&input.message,&session.cwd)?;
            let input_id=format!("LOCUS_INPUT_{}",uuid::Uuid::new_v4().simple());
            let paths=prepared.paths(&session.attachment_root,&input_id);
            let mut config=session.options.read().unwrap_or_else(|e|e.into_inner()).clone();
            if let Some(tools)=tools {
                let (snapshot, _) = session.merge_tools(&tools)?;
                config["tools"] = json!(snapshot);
            }
            config["insertion"]=json!({"inputId":input_id,"terminalText":prepared.terminal_text(&input_id,&paths),"submissionId":input.id,"prompt":prepared.prompt(&paths),"imageCount":prepared.image_count(),"expectedImageCount":prepared.image_count()});
            let mut mappings=config["attachmentMappings"].as_array().cloned().unwrap_or_default();
            mappings.extend(prepared.mappings(&paths).as_array().unwrap().iter().cloned());
            config["attachmentMappings"]=json!(mappings);
            if let Some(remote)=&session.remote {
                *session.options.write().unwrap_or_else(|e|e.into_inner())=config.clone();
                remote.send(json!({"type":"configure","options":config,"uploadId":input_id,"uploads":prepared.files}))?;
            } else {
                prepared.write(Path::new(&session.attachment_root),&input_id)?;
                session.configure(config.clone())?;
            }
            submit_native_input(&session.controls, |text| session.write(text), &input_id, &prepared.image_paths(&paths)).await?;
            tokio::time::timeout(Duration::from_secs(30),session.controls.wait("input_queued",&input.id,None)).await
                .map_err(|_|"Claude did not acknowledge the inserted input; delivery is unknown")?;
            session.write("\x18\x13")?;
            let checkpoint = tokio::time::timeout(Duration::from_secs(60),session.controls.input_receipt(&input.id)).await
                .map_err(|_|"Claude queued the input but did not confirm consuming it")??;
            source.accepted(&input.id,&checkpoint)?;
            let mut current=session.options.read().unwrap_or_else(|e|e.into_inner()).clone();
            current.as_object_mut().unwrap().remove("insertion");
            session.configure(current)?;
        }
    }.await;
    result.err().unwrap_or_else(||"Claude input delivery ended".into())
}

fn interrupted_tool()->ClaudeCodeToolResult {
    ClaudeCodeToolResult{output:super::native_journal::INTERRUPTED_DISPATCH_MESSAGE.into(),is_error:true,images:None,context:Vec::new()}
}

pub(super) fn tool_name(name: &str) -> String {
    name.strip_prefix("mcp__locus__")
        .unwrap_or(name)
        .to_string()
}

fn native_mod_source() -> String {
    // Opt-in CLI tests iterate on the actual checked-in Mod without requiring
    // a full desktop link for each JavaScript-only change.
    #[cfg(test)]
    if let Ok(source) = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/locus-remote-core/src/resources/native_mod.js"
    )) {
        return source;
    }
    locus_remote_core::claude_ssh::NATIVE_MOD_SOURCE.to_string()
}
