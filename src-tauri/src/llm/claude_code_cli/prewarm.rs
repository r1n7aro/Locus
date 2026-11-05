//! Unused processes have reserved Locus IDs, but no database rows or messages.
//! Allocation retains that ID, native UUID, remote directory and SSH transport.
//! A used process never returns to the spare pool: it stays sticky until reaped.
use super::*;
use super::super::ssh::{ConnectionSettings, PermissionMode, SshConfig};
use super::super::connection::NativeConnection;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Context {
    pub cwd: String,
    pub checkout: String,
    pub epoch: u64,
    pub model: String,
    pub remote: NativeConnection,
    pub permission: PermissionMode,
    pub runtime: super::super::runtime_settings::RuntimeSettings,
    pub native_tool_search: bool,
}
impl Context {
    pub fn options(&self, id: String, resume: Option<String>) -> ClaudeCodeCliOptions {
        let native_tool_search = self.native_tool_search && super::super::tool_catalog::native_mode_for_session(&id, resume.is_some()).unwrap_or(self.native_tool_search);
        ClaudeCodeCliOptions { locus_session_id: id, cwd: self.cwd.clone(), model: self.model.clone(), remote: self.remote.as_ssh().cloned(),
            system_prompt: String::new(), effort: None, tools: vec![], resume_session_id: resume, native_tool_search,
            // Spares never register a delegation directory. A native-agent
            // conversation does not reuse them (the run restarts the process).
            native_agents: false, agents: serde_json::Value::Null }
    }
    fn configured(&self, settings: &ConnectionSettings) -> bool {
        let selected=match &self.remote {
            NativeConnection::Ssh(config)=>settings.remote.as_ref()==Some(config),
            NativeConnection::Server{binding_id}=>{
                #[cfg(feature="server-client")]
                {crate::server_client::binding_is_active(binding_id)}
                #[cfg(not(feature="server-client"))]
                {let _=binding_id;false}
            }
        };
        selected && settings.permission_mode==self.permission && settings.runtime==self.runtime
    }
}
struct Spare { id: String, context: Context }
struct Job { generation: String, abort: tokio::task::AbortHandle, done: tokio::sync::watch::Receiver<bool>, claimed: bool }
#[derive(Default)]
struct Pool {
    desired: Option<(Context, Instant)>,
    spares: Vec<Spare>,
    jobs: HashMap<String, Job>,
    retry_after: Option<Instant>,
    maintenance: bool,
    manual: bool,
    last_error: Option<String>,
}
fn pool() -> &'static Mutex<Pool> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    POOL.get_or_init(Default::default)
}

struct IdleHost;
impl ClaudeCodeHost for IdleHost {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, _: ClaudeCodeAssistantMessage) -> Result<(), String> { Err("Unexpected output during prewarm".into()) }
    fn execute_tool<'a>(&'a mut self, _: &'a str, _: &'a str, _: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async { ClaudeCodeToolResult { output: "Tools cannot execute during prewarm".into(), is_error: true, images: None, context: Vec::new() } })
    }
}

const HISTORY_GATE_TIMEOUT: Duration = Duration::from_secs(90);

async fn startup_gate(id: &str, timeout: Duration) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
    tokio::time::timeout(timeout, history_gate(id).write_owned()).await
        .map_err(|_| format!("History lock: timed out after {}s waiting for the current turn or history operation; no SSH connection was started", timeout.as_secs()))
}

fn startup_result(id: &str, result: Result<(), String>) -> Result<(), String> {
    result.map_err(|error| super::super::turn_retry::with_context(&format!("Claude prewarm session={id}"), error))
}

// Startup owns the history-lock, SSH/environment and TUI deadlines. A second
// blanket deadline here would hide the failing stage (and cancel authorized
// environment setup before its own timeout can report useful diagnostics).
async fn wait_job(mut receive: tokio::sync::watch::Receiver<bool>) {
    while !*receive.borrow_and_update() {
        if receive.changed().await.is_err() { break; }
    }
}

fn launch(state: &mut Pool, options: ClaudeCodeCliOptions, permission: PermissionMode) {
    let id = options.locus_session_id.clone();
    if state.jobs.contains_key(&id) { return; }
    let (done, receive) = tokio::sync::watch::channel(false);
    let job_id = id.clone();
    let context = state.desired.as_ref().map(|(context, _)| context.clone());
    let generation = uuid::Uuid::new_v4().to_string();
    let job_generation = generation.clone();
    let task = tokio::spawn(async move {
        eprintln!("[Claude] prewarm started: session={job_id}");
        let result = async {
            let _gate = startup_gate(&job_id, HISTORY_GATE_TIMEOUT).await?;
            let existing = sessions().lock().unwrap_or_else(|e| e.into_inner()).get(&job_id).cloned();
            if let Some(existing) = existing {
                if existing.alive() && existing.cwd == options.cwd && existing.connection == options.remote
                    && normalize_claude_model(&existing.model) == normalize_claude_model(&options.model)
                    && existing.permission_mode == permission
                    && existing.runtime_settings == super::super::ssh::load_settings()?.runtime
                    && existing.native_tool_search == options.native_tool_search
                    && options.resume_session_id.as_ref().is_none_or(|id| existing.session_id.lock().unwrap_or_else(|e| e.into_inner()).as_ref() == Some(id)) {
                    return Ok(());
                }
                close_session(&job_id);
            }
            let session = connection_status::start_cached_with_mode(&options, permission,true).await?;
            session.prewarmed.store(true, Ordering::SeqCst);
            session.busy.store(true, Ordering::SeqCst);
            let mut lease = TurnLease(session.clone(), false);
            let mut bridge = session.bridge.lock().await;
            await_ready(&session, &options, &mut IdleHost, &mut bridge).await?;
            // No prompt, Enter, slash command, tool or inference is submitted.
            if session.options.read().unwrap_or_else(|e| e.into_inner())["active"] != false { return Err("Prewarm unexpectedly activated input".to_string()); }
            if !pool().lock().unwrap_or_else(|e| e.into_inner()).jobs.get(&job_id).is_some_and(|job| job.generation == job_generation) {
                return Err("Prewarm cancelled".into());
            }
            lease.1 = true;
            Ok::<_, String>(())
        }.await;
        let result = startup_result(&job_id, result);
        if result.is_ok() { eprintln!("[Claude] prewarm ready: session={job_id}"); }
        let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
        if state.jobs.get(&job_id).is_some_and(|job| job.generation == job_generation) {
            record_result(&mut state, context.as_ref(), result);
            state.jobs.remove(&job_id);
        }
        let _ = done.send(true);
    });
    state.jobs.insert(id, Job { generation, abort: task.abort_handle(), done: receive, claimed: false });
}

fn record_result(state: &mut Pool, context: Option<&Context>, result: Result<(), String>) {
    if context.is_none() || context != state.desired.as_ref().map(|(context, _)| context) { return; }
    match result {
        Ok(()) => { state.last_error = None; state.retry_after = None; }
        Err(error) => {
            eprintln!("[Claude] connection failed: {error}");
            state.last_error = Some(error);
            state.retry_after = Some(Instant::now() + Duration::from_secs(60));
        }
    }
}

/// Called on selection, and by a lightweight foreground interest heartbeat.
pub(crate) fn maintain(context: Context, existing: Option<(String, Option<String>)>, warm_existing: bool) {
    let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
    // Background windows may keep sending interest heartbeats. Only an explicit
    // selection can replace another context; heartbeats must not churn the pool.
    if !warm_existing && state.desired.as_ref().is_none_or(|(desired, _)| desired != &context) { return; }
    if state.desired.as_ref().is_none_or(|(desired, _)| desired != &context) {
        state.retry_after = None;
        state.last_error = None;
    }
    state.desired = Some((context.clone(), Instant::now()));
    if let Some(remote) = context.remote.as_ssh() { super::super::archive::prewarm(remote); }
    if warm_existing {
        if let Some((id, resume)) = existing {
            launch(&mut state, context.options(id, resume), context.permission);
        }
    }
    if !state.maintenance {
        state.maintenance = true;
        tokio::spawn(async {
            loop {
                reconcile();
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
}

/// Explicit connection uses the same idle TUI as prewarm, including when
/// automatic prewarm is disabled. It never changes the saved preference.
pub(crate) async fn connect(context: Context) -> Result<(), String> {
    {
        let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
        state.manual = true;
        state.retry_after = None;
    }
    maintain(context.clone(), None, true);
    reconcile();
    let (id, receive) = {
        let state = pool().lock().unwrap_or_else(|e| e.into_inner());
        let id = state.spares.iter().find(|spare| spare.context == context)
            .map(|spare| spare.id.clone()).ok_or("Claude connection is waiting for the current operation")?;
        let receive = state.jobs.get(&id).map(|job| job.done.clone());
        (id, receive)
    };
    if let Some(receive) = receive { wait_job(receive).await; }
    if sessions().lock().unwrap_or_else(|e| e.into_inner()).get(&id)
        .is_some_and(|session| session.alive() && session.ready.load(Ordering::SeqCst)) {
        Ok(())
    } else {
        Err(pool().lock().unwrap_or_else(|e| e.into_inner()).last_error.clone()
            .unwrap_or_else(|| "Claude connection was cancelled".to_string()))
    }
}

// Used conversations have independent lifetimes. Only pending startups count
// against replenishment concurrency; a long-running turn must never starve the
// spare pool. Bound-session warmups also consume startup slots while pending.
fn replenishment_count(state: &Pool, capacity: usize, now: Instant) -> usize {
    locus_remote_core::claude_launch::spare_start_count(capacity,state.spares.len(),state.jobs.len(),state.retry_after.is_some_and(|at|now<at))
}

fn reconcile() {
    let Ok(settings) = super::super::ssh::load_settings() else { return; };
    let snapshot = sessions().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
    let desired = state.desired.as_ref().filter(|(ctx, touched)| (settings.prewarm || state.manual) && ctx.configured(&settings)
        && touched.elapsed() < Duration::from_secs(u64::from(settings.idle_minutes.clamp(1, 60)) * 60)).map(|(ctx, _)| ctx.clone());
    let mut remove = Vec::new();
    let jobs: std::collections::HashSet<_> = state.jobs.keys().cloned().collect();
    state.spares.retain(|spare| {
        let keep = desired.as_ref() == Some(&spare.context) && (jobs.contains(&spare.id) || snapshot.get(&spare.id).is_some_and(|s| s.alive()));
        if !keep { remove.push(spare.id.clone()); }
        keep
    });
    let capacity = if settings.prewarm { settings.pool_capacity.clamp(1, 4) as usize } else { 1 };
    while state.spares.len() > capacity {
        remove.push(state.spares.pop().unwrap().id);
    }
    for id in &remove {
        if let Some(job) = state.jobs.remove(id) { job.abort.abort(); }
    }
    // Foreground demand keeps unused spares warm; bound conversations retain
    // their independent idle deadline, measured from their last actual turn.
    for spare in &state.spares {
        if let Some(session) = snapshot.get(&spare.id) { *session.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now(); }
    }
    if let Some(context) = desired {
        for _ in 0..replenishment_count(&state, capacity, Instant::now()) {
            let id = uuid::Uuid::new_v4().to_string();
            state.spares.push(Spare { id: id.clone(), context: context.clone() });
            launch(&mut state, context.options(id, None), context.permission);
        }
    }
    drop(state);
    for id in remove { close_session(&id); }
}

/// Only a genuinely new conversation can consume a spare. The database is
/// created with this reserved ID, so archive/resume paths never need rebinding.
pub(crate) fn claim(cwd: &str, checkout: &str, epoch: u64, model: &str) -> Option<String> {
    let settings = super::super::ssh::load_settings().ok()?;
    let snapshot = sessions().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
    if !settings.prewarm && !state.manual { return None; }
    allocate(&mut state, &snapshot, &settings, cwd, checkout, epoch, model)
}
fn allocate(state: &mut Pool, snapshot: &HashMap<String, Arc<Session>>, settings: &ConnectionSettings, cwd: &str, checkout: &str, epoch: u64, model: &str) -> Option<String> {
    let index = state.spares.iter().enumerate().filter(|(_, s)| s.context.configured(&settings) && s.context.cwd == cwd
        && s.context.checkout == checkout && s.context.epoch == epoch && normalize_claude_model(&s.context.model) == normalize_claude_model(model))
        .min_by_key(|(_, s)| !snapshot.get(&s.id).is_some_and(|process| process.ready.load(Ordering::SeqCst) && process.alive())).map(|(i, _)| i)?;
    let spare = state.spares.remove(index);
    if let Some(job) = state.jobs.get_mut(&spare.id) { job.claimed = true; }
    Some(spare.id)
}

pub(crate) async fn wait_for(id: &str) -> Result<(), String> {
    let receive = {
        let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
        state.jobs.get_mut(id).map(|job| { job.claimed = true; job.done.clone() })
    };
    if let Some(receive) = receive { wait_job(receive).await; }
    Ok(())
}
pub(super) async fn turn_gate(id: &str) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>, String> {
    wait_for(id).await?;
    history_gate(id).try_write_owned().map_err(|_| "Claude conversation is already running or its history is being updated".into())
}

pub(crate) fn spare_id(cwd: &str, model: &str, settings: &ConnectionSettings) -> Option<String> {
    let snapshot = sessions().lock().unwrap_or_else(|e| e.into_inner()).clone();
    pool().lock().unwrap_or_else(|e| e.into_inner()).spares.iter()
        .filter(|s| s.context.configured(settings) && s.context.cwd == cwd && normalize_claude_model(&s.context.model) == normalize_claude_model(model))
        .min_by_key(|s| !snapshot.get(&s.id).is_some_and(|s| s.ready.load(Ordering::SeqCst) && s.alive())).map(|s| s.id.clone())
}

pub(super) struct PoolSnapshot {
    pub spares: std::collections::HashSet<String>,
    pub pending: Vec<String>,
    pub demanded: bool,
    pub retrying: bool,
    pub last_error: Option<String>,
}
pub(super) fn status(config: &SshConfig, settings: &ConnectionSettings) -> PoolSnapshot {
    let state = pool().lock().unwrap_or_else(|e| e.into_inner());
    pool_snapshot(&state, &NativeConnection::from(config), settings)
}
pub(super) fn status_connection(config:&NativeConnection,settings:&ConnectionSettings)->PoolSnapshot {
    pool_snapshot(&pool().lock().unwrap_or_else(|e|e.into_inner()),config,settings)
}
pub(super) fn desired_server_model()->Option<String> {
    pool().lock().unwrap_or_else(|e|e.into_inner()).desired.as_ref()
        .filter(|(context,_)|matches!(context.remote,NativeConnection::Server{..})).map(|(context,_)|context.model.clone())
}
pub(crate) fn target(model:&str,settings:&ConnectionSettings)->Result<NativeConnection,String> {
    #[cfg(feature="server-client")]
    if let Some((binding,_))=crate::server_client::binding_for_model(model)? {
        if binding.kind!="claude" {return Err("Select a Claude Code model to connect".into());}
        return NativeConnection::server(binding.id);
    }
    if !model.starts_with("claude_code/"){return Err("Select a Claude Code model to connect".into());}
    settings.remote.clone().map(NativeConnection::Ssh).ok_or_else(||"No Claude SSH server is configured".into())
}
fn pool_snapshot(state: &Pool, config: &NativeConnection, settings: &ConnectionSettings) -> PoolSnapshot {
    let spares: std::collections::HashSet<_> = state.spares.iter().filter(|s| &s.context.remote == config).map(|s| s.id.clone()).collect();
    let demanded = state.desired.as_ref().is_some_and(|(context, touched)| &context.remote == config
        && (settings.prewarm || state.manual) && context.configured(settings) && touched.elapsed() < Duration::from_secs(u64::from(settings.idle_minutes.clamp(1, 60)) * 60));
    PoolSnapshot {
        pending: spares.iter().filter(|id| state.jobs.contains_key(*id)).cloned().collect(),
        spares, demanded,
        retrying: demanded && state.retry_after.is_some_and(|at| Instant::now() < at),
        last_error: state.desired.as_ref().filter(|(context, _)| &context.remote == config && context.configured(settings))
            .and(state.last_error.clone()),
    }
}
pub(crate) fn cancel_all() {
    let ids = {
        let mut state = pool().lock().unwrap_or_else(|e| e.into_inner());
        state.desired = None;
        state.manual = false;
        state.last_error = None;
        state.retry_after = None;
        let ids: Vec<_> = state.spares.drain(..).map(|s| s.id).collect();
        state.jobs.retain(|_, job| { if !job.claimed { job.abort.abort(); } job.claimed });
        ids
    };
    for id in ids { close_session(&id); }
}
pub(crate) fn discard_unassigned(id: &str) {
    if let Some(job) = pool().lock().unwrap_or_else(|e| e.into_inner()).jobs.remove(id) { job.abort.abort(); }
    close_session(id);
}
pub(crate) fn shutdown() {
    cancel_all();
    for (_, job) in pool().lock().unwrap_or_else(|e| e.into_inner()).jobs.drain() { job.abort.abort(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> Context {
        Context { cwd: "C:/workspace".into(), checkout: "checkout".into(), epoch: 3, model: "claude_code/claude-opus-5-5".into(),
            remote: SshConfig { target: "unused.invalid".into(), port: None, identity_file: None, known_hosts_file: None, cli_path: None }.into(), permission: PermissionMode::Auto, runtime: Default::default(), native_tool_search: true }
    }
    #[test]
    fn connection_failures_remain_visible_until_recovery_and_do_not_cross_contexts() {
        let context = context();
        let settings = ConnectionSettings { prewarm: true, remote: context.remote.as_ssh().cloned(), ..Default::default() };
        let mut state = Pool { desired: Some((context.clone(), Instant::now())), ..Default::default() };
        let error = "Claude plugin validation failed.\nregister.js:116: invalid helper";
        record_result(&mut state, Some(&context), Err(error.into()));
        let snapshot = pool_snapshot(&state, &context.remote, &settings);
        assert!(snapshot.retrying);
        assert_eq!(snapshot.last_error.as_deref(), Some(error));
        state.retry_after = None; // Starting another attempt keeps its previous cause visible.
        assert_eq!(pool_snapshot(&state, &context.remote, &settings).last_error.as_deref(), Some(error));
        let other = Context { model: "claude_code/sonnet".into(), ..context.clone() };
        record_result(&mut state, Some(&other), Ok(()));
        assert_eq!(state.last_error.as_deref(), Some(error));
        record_result(&mut state, Some(&other), Err("stale failure".into()));
        assert_eq!(state.last_error.as_deref(), Some(error));
        let another_server = SshConfig { target: "other.invalid".into(), ..context.remote.as_ssh().unwrap().clone() };
        assert!(pool_snapshot(&state, &NativeConnection::from(&another_server), &settings).last_error.is_none());
        let changed = ConnectionSettings { permission_mode: PermissionMode::BypassPermissions, ..settings.clone() };
        assert!(pool_snapshot(&state, &context.remote, &changed).last_error.is_none());
        record_result(&mut state, Some(&context), Ok(()));
        assert!(state.last_error.is_none());
        assert!(state.retry_after.is_none());
        assert!(pool_snapshot(&state, &context.remote, &settings).last_error.is_none());
    }
    #[tokio::test]
    async fn replenishment_is_bounded_by_startup_jobs_not_conversation_lifetimes() {
        let context = context();
        let now = Instant::now();
        let mut state = Pool::default();
        assert_eq!(replenishment_count(&state, 4, now), 2);
        assert_eq!(replenishment_count(&state, 1, now), 1);
        let task = tokio::spawn(std::future::pending::<()>());
        let (_send, done) = tokio::sync::watch::channel(false);
        for id in ["claimed-by-running-turn", "bound-session-startup"] {
            state.jobs.insert(id.into(), Job { generation: id.into(), abort: task.abort_handle(), done: done.clone(), claimed: true });
        }
        assert_eq!(replenishment_count(&state, 4, now), 0);
        // Once startup finishes, a claimed process may stay busy for hours.
        // It no longer occupies a startup slot or blocks replacement spares.
        state.jobs.remove("claimed-by-running-turn");
        assert_eq!(replenishment_count(&state, 4, now), 1);
        state.jobs.clear();
        for id in ["one", "two", "three"] {
            state.spares.push(Spare { id: id.into(), context: context.clone() });
        }
        assert_eq!(replenishment_count(&state, 4, now), 1);
        assert_eq!(replenishment_count(&state, 2, now), 0);
        state.retry_after = Some(now + Duration::from_secs(60));
        assert_eq!(replenishment_count(&state, 4, now), 0);
        assert_eq!(replenishment_count(&state, 4, now + Duration::from_secs(60)), 1);
        task.abort();
    }

    #[tokio::test]
    async fn lock_timeout_names_the_session_and_stage_without_claiming_ssh_failed() {
        let id = uuid::Uuid::new_v4().to_string();
        let guard = history_gate(&id).write_owned().await;
        let result = startup_gate(&id, Duration::from_millis(1)).await.map(|_| ());
        let error = startup_result(&id, result).unwrap_err();
        assert!(error.contains(&id) && error.contains("History lock"));
        assert!(error.contains("no SSH connection was started"));
        assert!(!super::super::super::turn_retry::is_disconnected(&error));
        drop(guard);
        assert!(startup_gate(&id, Duration::from_secs(1)).await.is_ok());
    }

    #[test]
    fn prewarm_diagnostics_preserve_startup_stage_and_transport_classification() {
        use super::super::super::turn_retry;
        for stage in ["SSH connection", "Environment preparation", "TUI initialization"] {
            let error = startup_result("session-id", Err(format!("{stage}: timed out"))).unwrap_err();
            assert!(error.contains("session-id") && error.contains(stage));
            assert!(!turn_retry::is_disconnected(&error));
        }
        let error = startup_result("session-id", Err(turn_retry::disconnected("SSH connection: timed out"))).unwrap_err();
        assert!(turn_retry::is_disconnected(&error));
        assert!(error.contains("session-id") && error.contains("SSH connection"));
    }

    #[tokio::test]
    async fn job_waiters_follow_the_startup_result_and_do_not_hang_on_cancellation() {
        let (done, receive) = tokio::sync::watch::channel(false);
        let mut waiting = Box::pin(wait_job(receive));
        assert!(tokio::time::timeout(Duration::from_millis(1), &mut waiting).await.is_err());
        done.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap();
        let (done, receive) = tokio::sync::watch::channel(false);
        drop(done);
        tokio::time::timeout(Duration::from_secs(1), wait_job(receive)).await.unwrap();
    }

    #[test]
    fn allocation_is_exclusive_and_never_crosses_server_model_or_checkout() {
        let context = context();
        let settings = ConnectionSettings { prewarm: true, remote: context.remote.as_ssh().cloned(), ..Default::default() };
        let mut state = Pool::default();
        for id in ["one", "two"] { state.spares.push(Spare { id: id.into(), context: context.clone() }); }
        let snapshot = HashMap::new();
        assert!(allocate(&mut state, &snapshot, &settings, &context.cwd, "other", 3, &context.model).is_none());
        assert!(allocate(&mut state, &snapshot, &settings, &context.cwd, "checkout", 4, &context.model).is_none());
        assert!(allocate(&mut state, &snapshot, &settings, &context.cwd, "checkout", 3, "claude-sonnet-5-5").is_none());
        let changed_runtime = ConnectionSettings {
            runtime: super::super::super::runtime_settings::RuntimeSettings { telemetry: true, ..Default::default() },
            ..settings.clone()
        };
        assert!(allocate(&mut state, &snapshot, &changed_runtime, &context.cwd, "checkout", 3, &context.model).is_none());
        let changed = ConnectionSettings { remote: None, ..settings.clone() };
        assert!(allocate(&mut state, &snapshot, &changed, &context.cwd, "checkout", 3, &context.model).is_none());
        assert_eq!(allocate(&mut state, &snapshot, &settings, &context.cwd, "checkout", 3, "claude-opus-5-5"), Some("one".into()));
        assert_eq!(allocate(&mut state, &snapshot, &settings, &context.cwd, "checkout", 3, &context.model), Some("two".into()));
        assert!(allocate(&mut state, &snapshot, &settings, &context.cwd, "checkout", 3, &context.model).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires explicitly authorized isolated SSH config; starts idle remote Claude only, no model messages"]
    async fn real_ssh_pool_allocates_sticky_processes_and_reaps_without_inference() {
        use crate::llm::claude_code_cli::native_integration_tests::EnvGuard;
        let config_dir = std::env::var("LOCUS_CLAUDE_SSH_TEST_CONFIG_DIR").expect("select an isolated test configuration");
        let model = std::env::var("LOCUS_CLAUDE_SSH_TEST_MODEL").expect("select the permitted model");
        let _env = EnvGuard::set(&[("LOCUS_RUNTIME_CONFIG_DIR", Some(config_dir.clone()))]);
        let path = Path::new(&config_dir).join("claude_connection.json");
        struct Restore(std::path::PathBuf, Vec<u8>);
        impl Drop for Restore { fn drop(&mut self) { cancel_all(); close_all(); let _ = std::fs::write(&self.0, &self.1); } }
        let _restore = Restore(path.clone(), std::fs::read(&path).unwrap());
        // Connection options are settings now; copy the isolated ones into the
        // process store that ssh::load_settings reads.
        let isolated = crate::settings::global::open_store(Path::new(&config_dir), Path::new(&config_dir));
        let options = crate::settings::group::read(&isolated, "providers.claudeCode");
        crate::settings::global::global()
            .set_values(&crate::settings::group::changes("providers.claudeCode", &options))
            .unwrap();
        let mut settings = super::super::super::ssh::load_settings().unwrap();
        settings.prewarm = true; settings.pool_capacity = 2; settings.idle_minutes = 5;
        super::super::super::ssh::save_settings(&settings).unwrap();
        let root = tempfile::tempdir().unwrap();
        let context = Context { cwd: root.path().display().to_string(), checkout: "pool-test".into(), epoch: 0,
            model, remote: settings.remote.clone().unwrap().into(), permission: settings.permission_mode, runtime: settings.runtime, native_tool_search: true };
        maintain(context.clone(), None, true);
        let deadline = Instant::now() + Duration::from_secs(90);
        let ids = loop {
            let ids: Vec<_> = pool().lock().unwrap().spares.iter().map(|s| s.id.clone()).collect();
            if ids.len() == 2 && ids.iter().all(|id| sessions().lock().unwrap().get(id).is_some_and(|s| s.ready.load(Ordering::SeqCst) && s.alive())) { break ids; }
            assert!(Instant::now() < deadline, "pool failed to initialize");
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        std::fs::create_dir_all(root.path().join("db")).unwrap();
        let store = crate::session::store::SessionStore::new(&root.path().join("db")).unwrap();
        assert!(store.list_sessions(None).unwrap().is_empty());
        let claimed = claim(&context.cwd, &context.checkout, context.epoch, &context.model).unwrap();
        let session = sessions().lock().unwrap().get(&claimed).unwrap().clone();
        session.busy.store(true, Ordering::SeqCst);
        let native = session.session_id.lock().unwrap().clone();
        super::super::super::clear_stale_session_id_for_turn(&claimed).await;
        assert!(Arc::ptr_eq(&session, sessions().lock().unwrap().get(&claimed).unwrap()), "first-send preparation must retain the unused process");
        assert_eq!(session.options.read().unwrap()["active"], false);
        store.create_session_scoped_with_id(&claimed, "Pool allocation", None, None, None, "chat", None).unwrap();
        assert!(store.get_messages(&claimed).unwrap().is_empty());
        maintain(context.clone(), Some((claimed.clone(), native.clone())), true);
        wait_for(&claimed).await.unwrap();
        assert!(Arc::ptr_eq(&session, sessions().lock().unwrap().get(&claimed).unwrap()));
        assert_eq!(*session.session_id.lock().unwrap(), native);
        session.busy.store(true, Ordering::SeqCst);
        reconcile();
        assert_eq!(pool().lock().unwrap().spares.len(), 1, "do not replenish during an active turn");
        session.busy.store(false, Ordering::SeqCst);
        cancel_all();
        assert!(session.alive(), "turning off spare prewarm must preserve a bound conversation");
        *session.last_used.lock().unwrap() = Instant::now() - Duration::from_secs(6 * 60);
        tokio::time::timeout(Duration::from_secs(15), async { while has_live_session(&claimed) { tokio::time::sleep(Duration::from_millis(100)).await; } }).await.unwrap();
        std::println!("LOCUS_POOL_CHECK {}", json!({"spares":ids.len(),"nativeIdPreserved":true,"transportReused":true,"emptyDatabaseBeforeAllocation":true,"idleReaped":true,"modelPromptsSubmitted":0}));
    }
}
