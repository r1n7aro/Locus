//! Read-only snapshots of this application's per-conversation SSH/TUI lifetime.
//! Inspecting status must never open SSH, unlock a keychain or query a provider.
use super::*;
use super::super::ssh::{SshConfig, ConnectionSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ConnectionPhase {
    Disconnected,
    Connecting,
    Starting,
    Ready,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionStatus {
    pub phase: ConnectionPhase,
    pub target: String,
    pub port: Option<u16>,
    pub latency_ms: Option<f64>,
    pub last_error: Option<String>,
    pub pool: PoolStatus,
    pub archive_pool: Option<locus_remote_core::claude_ssh::archive_pool::Snapshot>,
    pub diagnostics: super::super::ssh::diagnostics::Overview,
}

#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolStatus {
    pub state: &'static str,
    pub capacity: u32,
    pub ready: usize,
    pub starting: usize,
    pub active: usize,
    pub retained: usize,
}

fn starting() -> &'static Mutex<HashMap<String, SshConfig>> {
    static STARTING: OnceLock<Mutex<HashMap<String, SshConfig>>> = OnceLock::new();
    STARTING.get_or_init(|| Mutex::new(HashMap::new()))
}

// Also clears a pending startup if its future fails or is cancelled.
struct StartupStatus(String);
#[cfg(feature="server-client")]
fn server_starting()->&'static Mutex<HashMap<String,String>> {
    static STARTING:OnceLock<Mutex<HashMap<String,String>>>=OnceLock::new();STARTING.get_or_init(Default::default)
}
#[cfg(feature="server-client")]
struct ServerStartupStatus(String);
#[cfg(feature="server-client")]
impl Drop for ServerStartupStatus {fn drop(&mut self){server_starting().lock().unwrap_or_else(|e|e.into_inner()).remove(&self.0);}}
impl StartupStatus {
    fn new(id: &str, config: &SshConfig) -> Self {
        starting().lock().unwrap_or_else(|e| e.into_inner()).insert(id.into(), config.clone());
        Self(id.into())
    }
}
impl Drop for StartupStatus {
    fn drop(&mut self) {
        starting().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

fn snapshot(config: &SshConfig, phase: ConnectionPhase) -> ConnectionStatus {
    ConnectionStatus { phase, target: config.target.clone(), port: config.port, latency_ms: None, last_error: None,
        pool: PoolStatus { state: "disabled", ..Default::default() }, archive_pool: None, diagnostics: Default::default() }
}

#[cfg(test)]
pub(crate) fn connection_status(id: Option<&str>, configured: Option<&SshConfig>) -> Option<ConnectionStatus> {
    connection_overview(id, None, Some("claude_code/test"), &ConnectionSettings { remote: configured.cloned(), ..Default::default() })
}

struct ProcessStatus {
    id: String,
    phase: ConnectionPhase,
    busy: bool,
    latency_ms: Option<f64>,
}

fn pool_status(processes: &[ProcessStatus], pool: &prewarm::PoolSnapshot, enabled: bool, capacity: u32) -> PoolStatus {
    let mut result = PoolStatus { state: "disabled", capacity, ..Default::default() };
    for process in processes {
        match process.phase {
            ConnectionPhase::Connecting | ConnectionPhase::Starting => result.starting += 1,
            ConnectionPhase::Ready if pool.spares.contains(&process.id) => result.ready += 1,
            ConnectionPhase::Ready if process.busy => result.active += 1,
            ConnectionPhase::Ready => result.retained += 1,
            ConnectionPhase::Disconnected => {},
        }
    }
    result.state = if !enabled { "disabled" }
        else if result.ready >= capacity as usize { "ready" }
        else if result.starting > 0 { "warming" }
        else if pool.retrying { "retrying" }
        else if pool.demanded { "waiting" }
        else { "idle" };
    result
}

/// Read existing processes only. An API-selected composer sees the configured
/// server, while a Claude conversation continues to report its bound server.
pub(crate) fn connection_overview(id: Option<&str>, cwd: Option<&str>, model: Option<&str>, settings: &ConnectionSettings) -> Option<ConnectionStatus> {
    #[cfg(feature="server-client")]
    let selected_server=if model.is_none(){prewarm::desired_server_model().or_else(|| {
        if settings.remote.is_some(){return None;}
        sessions().lock().unwrap_or_else(|e|e.into_inner()).values().filter(|session|session.model.starts_with("locus-server/"))
            .max_by_key(|session|*session.last_used.lock().unwrap_or_else(|e|e.into_inner())).map(|session|session.model.clone())
    })}else{None};
    #[cfg(feature="server-client")]
    let model=model.or(selected_server.as_deref());
    if model.is_some_and(|model|model.starts_with("locus-server/")) {
        #[cfg(feature="server-client")]
        {
            let (binding,_) = crate::server_client::binding_for_model(model?).ok()??;
            if binding.kind!="claude" {return None;}
            let pool=prewarm::status_connection(&super::super::connection::NativeConnection::server(&binding.id).ok()?,settings);
            let cached=sessions().lock().unwrap_or_else(|e|e.into_inner()).clone();
            let mut processes=Vec::new();
            for (id,session) in &cached {
                if Some(session.model.as_str())!=model {continue;}
                processes.push(ProcessStatus{id:id.clone(),phase:if !session.alive(){ConnectionPhase::Disconnected}else if session.ready.load(Ordering::SeqCst){ConnectionPhase::Ready}else{ConnectionPhase::Starting},busy:session.busy.load(Ordering::SeqCst),latency_ms:session.remote.as_ref().and_then(|remote|remote.latency_ms())});
            }
            for (id,pending_model) in server_starting().lock().unwrap_or_else(|e|e.into_inner()).iter() {
                if Some(pending_model.as_str())==model {processes.retain(|process|&process.id!=id);processes.push(ProcessStatus{id:id.clone(),phase:ConnectionPhase::Connecting,busy:false,latency_ms:None});}
            }
            for pending in &pool.pending {if !processes.iter().any(|process|&process.id==pending){processes.push(ProcessStatus{id:pending.clone(),phase:ConnectionPhase::Connecting,busy:false,latency_ms:None});}}
            let spare=if id.is_none(){cwd.zip(model).and_then(|(cwd,model)|prewarm::spare_id(cwd,model,settings))}else{None};
            let chosen=id.or(spare.as_deref()).and_then(|id|processes.iter().find(|process|process.id==id)).or_else(||processes.iter().find(|process|process.phase==ConnectionPhase::Ready)).or_else(||processes.first());
            return Some(ConnectionStatus {phase:chosen.map_or(ConnectionPhase::Disconnected,|process|process.phase),target:binding.name,port:None,latency_ms:chosen.and_then(|process|process.latency_ms),
                last_error:pool.last_error.clone(),pool:pool_status(&processes,&pool,settings.prewarm,settings.pool_capacity.clamp(1,4)),archive_pool:None,diagnostics:Default::default()});
        }
        #[cfg(not(feature="server-client"))]
        return None;
    }
    let spare = if id.is_none() { cwd.zip(model).and_then(|(cwd, model)| prewarm::spare_id(cwd, model, settings)) } else { None };
    let id = id.or(spare.as_deref());
    let cached = sessions().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let pending = starting().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let config = id.and_then(|id| pending.get(id).or_else(|| cached.get(id).and_then(|s| s.connection.as_ref())))
        .or(settings.remote.as_ref())?;
    let pool = prewarm::status(config, settings);
    let mut processes = Vec::new();
    for (id, session) in &cached {
        if session.connection.as_ref() != Some(config) { continue; }
        let alive = session.alive();
        processes.push(ProcessStatus { id: id.clone(),
            phase: if !alive { ConnectionPhase::Disconnected } else if session.ready.load(Ordering::SeqCst) { ConnectionPhase::Ready } else { ConnectionPhase::Starting },
            busy: session.busy.load(Ordering::SeqCst),
            latency_ms: if alive { session.remote.as_ref().and_then(|remote| remote.latency_ms()) } else { None },
        });
    }
    for (id, target) in &pending {
        if target == config {
            // Restarting a cached, dead process must show the new connection.
            processes.retain(|p| &p.id != id);
            processes.push(ProcessStatus { id: id.clone(), phase: ConnectionPhase::Connecting, busy: false, latency_ms: None });
        }
    }
    for id in &pool.pending {
        if !processes.iter().any(|p| &p.id == id) {
            processes.push(ProcessStatus { id: id.clone(), phase: ConnectionPhase::Connecting, busy: false, latency_ms: None });
        }
    }
    let selected = if let Some(id) = id { processes.iter().find(|p| p.id == id) }
        else if model.is_none() { processes.iter().min_by_key(|p| match p.phase {
            ConnectionPhase::Ready => if p.latency_ms.is_some() { 0 } else { 1 },
            ConnectionPhase::Starting => 2, ConnectionPhase::Connecting => 3, ConnectionPhase::Disconnected => 4,
        }) } else { None };
    let mut result = snapshot(config, selected.map_or(ConnectionPhase::Disconnected, |p| p.phase));
    result.latency_ms = selected.and_then(|p| p.latency_ms);
    result.pool = pool_status(&processes, &pool, settings.prewarm && settings.remote.as_ref() == Some(config), settings.pool_capacity.clamp(1, 4));
    result.last_error = pool.last_error;
    result.archive_pool = locus_remote_core::claude_ssh::archive_pool::snapshot(config);
    result.diagnostics = super::super::ssh::diagnostics::overview(config);
    Some(result)
}

pub(super) async fn start_cached(options: &ClaudeCodeCliOptions, permission_mode: super::super::ssh::PermissionMode) -> Result<Arc<Session>, String> {
    start_cached_with_mode(options,permission_mode,false).await
}
pub(super) async fn start_cached_with_mode(options:&ClaudeCodeCliOptions,permission_mode:super::super::ssh::PermissionMode,prewarm:bool)->Result<Arc<Session>,String> {
    if let Some(remote) = &options.remote { super::super::archive::prewarm(remote); }
    let _startup = options.remote.as_ref().map(|config| StartupStatus::new(&options.locus_session_id, config));
    #[cfg(feature="server-client")]
    let _server_startup=if options.model.starts_with("locus-server/"){server_starting().lock().unwrap_or_else(|e|e.into_inner()).insert(options.locus_session_id.clone(),options.model.clone());Some(ServerStartupStatus(options.locus_session_id.clone()))}else{None};
    let session = start(options, permission_mode,prewarm).await?;
    sessions().lock().unwrap_or_else(|e| e.into_inner()).insert(options.locus_session_id.clone(), session.clone());
    if session.remote.is_some() {
        watch_remote_session(options.locus_session_id.clone(), &session);
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> SshConfig {
        SshConfig { target: "unreachable.invalid".into(), port: Some(2222), identity_file: None, known_hosts_file: None, cli_path: None }
    }

    #[test]
    fn pool_counts_spares_busy_sticky_and_pending_without_counting_dead_processes() {
        let mut pool = prewarm::PoolSnapshot { spares: ["spare".into(), "booting".into()].into(), pending: vec![], demanded: true, retrying: false, last_error: None };
        let mut processes: Vec<_> = [("spare", ConnectionPhase::Ready, false), ("booting", ConnectionPhase::Starting, true),
            ("busy", ConnectionPhase::Ready, true), ("sticky", ConnectionPhase::Ready, false), ("dead", ConnectionPhase::Disconnected, false)]
            .into_iter().map(|(id, phase, busy)| ProcessStatus { id: id.into(), phase, busy, latency_ms: None }).collect();
        let status = pool_status(&processes, &pool, true, 2);
        assert_eq!((status.ready, status.starting, status.active, status.retained), (1, 1, 1, 1));
        assert_eq!(status.state, "warming");
        processes[1].phase = ConnectionPhase::Ready;
        assert_eq!(pool_status(&processes, &pool, true, 2).state, "ready");
        assert_eq!(pool_status(&processes, &pool, false, 2).state, "disabled");
        processes.clear();
        pool.retrying = true;
        assert_eq!(pool_status(&processes, &pool, true, 2).state, "retrying");
        pool.retrying = false;
        assert_eq!(pool_status(&processes, &pool, true, 2).state, "waiting");
        pool.demanded = false;
        assert_eq!(pool_status(&processes, &pool, true, 2).state, "idle");
    }

    #[test]
    fn overview_of_api_composer_uses_server_but_bound_conversation_stays_pinned() {
        let mut target = config();
        target.target = uuid::Uuid::new_v4().to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let _startup = StartupStatus::new(&id, &target);
        let mut settings = ConnectionSettings { remote: Some(target.clone()), prewarm: true, pool_capacity: 2, ..Default::default() };
        let overview = connection_overview(None, None, None, &settings).unwrap();
        assert_eq!(overview.phase, ConnectionPhase::Connecting);
        assert_eq!(overview.pool.starting, 1);
        assert_eq!(overview.latency_ms, None);
        // A new Claude conversation must not borrow another conversation's TUI.
        assert_eq!(connection_overview(None, None, Some("claude_code/opus"), &settings).unwrap().phase, ConnectionPhase::Disconnected);
        settings.remote = Some(SshConfig { target: uuid::Uuid::new_v4().to_string(), ..config() });
        let bound = connection_overview(Some(&id), None, Some("claude_code/opus"), &settings).unwrap();
        assert_eq!(bound.target, target.target);
        assert_eq!(bound.pool.state, "disabled");
        assert_eq!(connection_overview(None, None, None, &settings).unwrap().pool.starting, 0);
    }

    #[test]
    fn configured_is_not_connected_and_other_sessions_are_independent() {
        let config = config();
        let id = uuid::Uuid::new_v4().to_string();
        assert!(connection_status(None, None).is_none());
        assert_eq!(connection_status(None, Some(&config)).unwrap().phase, ConnectionPhase::Disconnected);
        let startup = StartupStatus::new(&id, &config);
        assert_eq!(connection_status(Some(&id), None).unwrap().phase, ConnectionPhase::Connecting);
        assert_eq!(connection_status(Some("another-conversation"), Some(&config)).unwrap().phase, ConnectionPhase::Disconnected);
        drop(startup);
        assert_eq!(connection_status(Some(&id), Some(&config)).unwrap().phase, ConnectionPhase::Disconnected);
    }

    #[tokio::test]
    async fn cancelled_startup_does_not_leave_connecting_status() {
        let id = uuid::Uuid::new_v4().to_string();
        let task_id = id.clone();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _startup = StartupStatus::new(&task_id, &config());
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        wait.await.unwrap();
        assert_eq!(connection_status(Some(&id), None).unwrap().phase, ConnectionPhase::Connecting);
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(connection_status(Some(&id), None).is_none());
    }
}
