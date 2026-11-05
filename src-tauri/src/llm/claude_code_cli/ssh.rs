//! Desktop settings and callbacks for the shared SSH transport.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use super::mod_bridge::PendingCall;
pub use locus_remote_core::claude_ssh::{SshConfig, RemoteInfo, diagnostics, startup};
pub(crate) mod unlock;

pub use locus_remote_core::claude_launch::PermissionMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSettings {
    pub version: u32,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    #[serde(flatten)]
    pub runtime: super::runtime_settings::RuntimeSettings,
    #[serde(default)]
    pub prewarm: bool,
    #[serde(default = "default_pool_capacity")]
    pub pool_capacity: u32,
    #[serde(default = "default_idle_minutes")]
    pub idle_minutes: u32,
    /// Route for new Multi-Agent conversations; existing ones keep their pin.
    #[serde(default = "default_native_multi_agent")]
    pub native_multi_agent: bool,
    #[serde(default)]
    pub native_agents: NativeAgentSettings,
    pub remote: Option<SshConfig>,
    pub authorized_target: Option<String>,
    pub info: Option<RemoteInfo>,
    /// Computed on read; never accept a frontend claim that a password exists.
    #[serde(default, skip_deserializing)]
    pub has_unlock_password: bool,
}
impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            version: 1,
            permission_mode: PermissionMode::default(),
            runtime: Default::default(),
            prewarm: false,
            pool_capacity: default_pool_capacity(),
            idle_minutes: default_idle_minutes(),
            native_multi_agent: default_native_multi_agent(),
            native_agents: NativeAgentSettings::default(),
            remote: None,
            authorized_target: None,
            info: None,
            has_unlock_password: false,
        }
    }
}
fn default_idle_minutes() -> u32 { 15 }
fn default_pool_capacity() -> u32 { 1 }
fn default_native_multi_agent() -> bool { true }

/// `providers.claudeCode.nativeAgents.*`: the Claude Code builtin Agent types
/// the native Agent tool offers, and the delegatable Locus agent types it
/// leaves out. Read when a conversation pins its route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeAgentSettings {
    #[serde(default = "default_builtin_agents")]
    pub builtins: Vec<String>,
    #[serde(default)]
    pub excluded_locus_agents: Vec<String>,
}
impl Default for NativeAgentSettings {
    fn default() -> Self { Self { builtins: default_builtin_agents(), excluded_locus_agents: Vec::new() } }
}
fn default_builtin_agents() -> Vec<String> {
    ["general-purpose", "Explore", "Plan"].into_iter().map(str::to_owned).collect()
}
/// State kept in `claude_connection.json`: which SSH target the user
/// authorized and what the last probe found. The options themselves are
/// settings under `providers.claudeCode`.
fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join("claude_connection.json"))
}
const SETTINGS_GROUP: &str = "providers.claudeCode";
const STATE_FIELDS: [&str; 2] = ["authorizedTarget", "info"];
const OPTIONAL_REMOTE_FIELDS: [&str; 3] = ["identityFile", "knownHostsFile", "cliPath"];

/// `remote.*` settings as `Option<SshConfig>` JSON: an empty target means
/// local, empty paths mean unset.
fn remote_from_settings(remote: &Value) -> Value {
    let Some(fields) = remote.as_object() else { return Value::Null };
    if fields.get("target").and_then(Value::as_str).unwrap_or_default().is_empty() {
        return Value::Null;
    }
    let mut remote = fields.clone();
    for field in OPTIONAL_REMOTE_FIELDS {
        if remote.get(field).and_then(Value::as_str).is_some_and(str::is_empty) {
            remote.insert(field.to_string(), Value::Null);
        }
    }
    Value::Object(remote)
}

fn remote_to_settings(remote: Option<&SshConfig>) -> Value {
    json!({
        "target": remote.map(|remote| remote.target.as_str()).unwrap_or_default(),
        "port": remote.and_then(|remote| remote.port),
        "identityFile": remote.and_then(|remote| remote.identity_file.as_deref()).unwrap_or_default(),
        "knownHostsFile": remote.and_then(|remote| remote.known_hosts_file.as_deref()).unwrap_or_default(),
        "cliPath": remote.and_then(|remote| remote.cli_path.as_deref()).unwrap_or_default(),
    })
}

pub fn load_settings() -> Result<ConnectionSettings, String> {
    let state = crate::config::read_state_file(&settings_path()?);
    let mut object = match crate::settings::group::read(
        crate::settings::global::global(),
        SETTINGS_GROUP,
    ) {
        Value::Object(object) => object,
        _ => Default::default(),
    };
    let remote = object.remove("remote").unwrap_or(Value::Null);
    object.insert("remote".into(), remote_from_settings(&remote));
    object.insert("version".into(), json!(1));
    for field in STATE_FIELDS {
        object.insert(field.into(), state.get(field).cloned().unwrap_or(Value::Null));
    }
    let value: ConnectionSettings =
        serde_json::from_value(Value::Object(object)).map_err(|e| e.to_string())?;
    value.runtime.validate()?;
    if let Some(remote) = &value.remote {
        remote.validate()?;
        if value.authorized_target.as_deref() != Some(remote.fingerprint().as_str()) {
            return Err("Authorize environment setup for this SSH target in Settings".into());
        }
    }
    Ok(value)
}
pub fn save_settings(value: &ConnectionSettings) -> Result<(), String> {
    value.runtime.validate()?;
    let mut options = serde_json::to_value(value).map_err(|e| e.to_string())?;
    options["remote"] = remote_to_settings(value.remote.as_ref());
    crate::settings::global::global()
        .set_values(&crate::settings::group::changes(SETTINGS_GROUP, &options))
        .map_err(|error| error.message)?;
    crate::config::update_state_file(&settings_path()?, |state| {
        state.insert("version".into(), json!(1));
        state.insert("authorizedTarget".into(), json!(value.authorized_target));
        state.insert("info".into(), json!(value.info));
    })
}

/// Applies `providers.claudeCode.*` changes written through settings
/// (agent `config_set`) the way the connection commands do.
pub(crate) fn apply_settings_change(changed_keys: &[&str]) {
    let settings = load_settings();
    let touched = |prefix: &str| changed_keys.iter().any(|key| key.starts_with(prefix));
    if touched("providers.claudeCode.idleCompaction")
        || touched("providers.claudeCode.telemetry")
        || touched("providers.claudeCode.autoCompactWindow")
    {
        if let Ok(settings) = &settings {
            super::tui::retire_outdated_runtime_settings(settings.runtime);
        }
    }
    let prewarm_off = settings.as_ref().map(|settings| !settings.prewarm).unwrap_or(true);
    if touched("providers.claudeCode.remote.") || (touched("providers.claudeCode.prewarm") && prewarm_off) {
        super::tui::prewarm::cancel_all();
    }
    if touched("providers.claudeCode.remote.") {
        let keep = settings.ok().and_then(|settings| settings.remote);
        super::tui::retire_idle_remote_connections(keep.as_ref());
    }
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("Missing parent directory")?)
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    // Windows rename replaces existing regular files as well.
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}
pub fn profile_root(id: &str) -> Result<PathBuf, String> {
    Ok(std::env::var_os("LOCUS_CLAUDE_TUI_PROFILE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(crate::commands::persistent_config_dir()?.join("claude-tui"))
        .join(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, id.as_bytes()).to_string()))
}
/// Only called with an explicit, persisted restore binding from SessionStore.
pub(crate) fn bind_restored_connection(id:&str,remote:&SshConfig)->Result<(),String> {
    remote.validate()?;
    let path=profile_root(id)?.join("connection.json");
    let value=json!({"version":1,"remote":remote});
    if std::fs::read(&path).ok().and_then(|bytes|serde_json::from_slice::<Value>(&bytes).ok()).as_ref()!=Some(&value) {
        write_json(&path,&value)?;
    }
    Ok(())
}
/// Explicit, repeatable v1 migration: historical local sessions remain local.
/// A saved binding wins over subsequently changed global connection settings.
pub fn session_connection(
    id: &str,
    legacy_local: bool,
    recorded: Option<Option<SshConfig>>,
) -> Result<Option<SshConfig>, String> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = profile_root(id)?.join("connection.json");
    if let Some(recorded) = recorded {
        // The database survives profile-directory loss and participates in
        // normal session backup/export. Never reinterpret a remote resume ID
        // as local merely because its configuration sidecar is missing.
        if let Some(config) = &recorded {
            config.validate()?;
        }
        let bound = resolve_binding(&path, false, || Ok(recorded.clone()))?;
        if bound != recorded {
            return Err("Claude session connection differs from its saved history".into());
        }
        Ok(bound)
    } else {
        resolve_binding(&path, legacy_local, || load_settings().map(|s| s.remote))
    }
}
fn resolve_binding(
    path: &Path,
    legacy_local: bool,
    current: impl FnOnce() -> Result<Option<SshConfig>, String>,
) -> Result<Option<SshConfig>, String> {
    #[derive(Serialize, Deserialize)]
    struct Binding {
        version: u32,
        remote: Option<SshConfig>,
    }
    match std::fs::read(path) {
        Ok(bytes) => {
            let binding: Binding = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Invalid Claude session connection: {e}"))?;
            if binding.version != 1 {
                return Err("Unsupported Claude session connection version".into());
            }
            if let Some(remote) = &binding.remote {
                remote.validate()?;
            }
            Ok(binding.remote)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let remote = if legacy_local { None } else { current()? };
            write_json(
                path,
                &Binding {
                    version: 1,
                    remote: remote.clone(),
                },
            )?;
            Ok(remote)
        }
        Err(e) => Err(e.to_string()),
    }
}
pub fn session_key(id: &str, remote: &SshConfig) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("{id}:{}", remote.fingerprint()).as_bytes(),
    )
    .to_string()
}


/// Existing callers keep their API; transport behavior is shared with Locus Server.
pub struct Transport(locus_remote_core::claude_ssh::Transport);
impl std::ops::Deref for Transport {
    type Target = locus_remote_core::claude_ssh::Transport;
    fn deref(&self) -> &Self::Target { &self.0 }
}
impl Transport {
    pub async fn start(
        config: &SshConfig,
        request: Value,
        calls: Option<(mpsc::Sender<PendingCall>, Arc<super::mod_bridge::ControlSignals>, Arc<std::sync::RwLock<Value>>)>,
    ) -> Result<(Self, RemoteInfo), String> {
        Self::start_with_unlock(config, request, calls, unlock::UnlockPassword::Saved).await
    }
    pub async fn start_with_unlock(
        config: &SshConfig,
        mut request: Value,
        calls: Option<(mpsc::Sender<PendingCall>, Arc<super::mod_bridge::ControlSignals>, Arc<std::sync::RwLock<Value>>)>,
        password: unlock::UnlockPassword,
    ) -> Result<(Self, RemoteInfo), String> {
        config.validate()?;
        let key = unlock::key(config)?;
        if request.get("runtimeSettings").is_none() {
            request["runtimeSettings"] = json!(load_settings()?.runtime);
        }
        let calls = calls.map(|(sender, controls, options)| locus_remote_core::claude_ssh::BridgeCalls {
            sender, route: Arc::new(move |payload| controls.route(payload, &options)),
        });
        let unlock = Arc::new(move || password.resolve(&key));
        let (transport, info) = locus_remote_core::claude_ssh::Transport::start(config, request, calls, unlock).await?;
        Ok((Self(transport), info))
    }
}
pub async fn probe(config: &SshConfig, authorized: bool) -> Result<RemoteInfo, String> {
    probe_with_unlock(config, authorized, unlock::UnlockPassword::Saved).await
}
pub async fn probe_with_unlock(config: &SshConfig, authorized: bool, password: unlock::UnlockPassword) -> Result<RemoteInfo, String> {
    // Setup with native agents enabled also installs the Claude Code they need.
    let native_multi_agent = authorized && load_settings().is_ok_and(|settings| settings.native_multi_agent);
    let (_transport, info) = Transport::start_with_unlock(
        config,
        json!({"mode":if authorized {"prepare"} else {"inspect"},"authorizedSetup":authorized,"nativeMultiAgent":native_multi_agent}),
        None,
        password,
    )
    .await?;
    Ok(info)
}

/// Saves the CLI version a session launch found, which can differ from the
/// last connection test (the launch installs Claude Code when it must, and
/// Claude updates itself). New conversations choose their route from it.
pub fn record_launched_cli(config: &SshConfig, launched: &RemoteInfo) {
    let Some(settings) = load_settings().ok().and_then(|settings| with_launched_cli(settings, config, launched)) else { return };
    if let Err(error) = save_settings(&settings) {
        eprintln!("[Claude SSH] could not save the remote CLI version: {error}");
    }
}

/// The saved connection updated with a launch's CLI, or `None` when it is
/// another connection or nothing changed.
fn with_launched_cli(mut settings: ConnectionSettings, config: &SshConfig, launched: &RemoteInfo) -> Option<ConnectionSettings> {
    if settings.remote.as_ref() != Some(config) { return None; }
    let saved = settings.info.as_mut()?;
    if launched.cli_version.is_none() || saved.cli_version == launched.cli_version { return None; }
    saved.cli_version = launched.cli_version.clone();
    saved.cli_supported = launched.cli_supported;
    Some(settings)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_launch_records_the_remote_cli_version_it_found() {
        let config = SshConfig { target: "user@host".into(), port: None, identity_file: None, known_hosts_file: None, cli_path: None };
        let info = |version: Option<&str>| RemoteInfo { cli_version: version.map(str::to_owned), cli_supported: true, ..Default::default() };
        let saved = ConnectionSettings { remote: Some(config.clone()), info: Some(info(Some("2.1.300 (Claude Code)"))), ..Default::default() };
        let updated = with_launched_cli(saved.clone(), &config, &info(Some("2.1.312 (Claude Code)"))).unwrap();
        assert_eq!(updated.info.unwrap().cli_version.as_deref(), Some("2.1.312 (Claude Code)"));
        assert!(with_launched_cli(saved.clone(), &config, &info(Some("2.1.300 (Claude Code)"))).is_none());
        assert!(with_launched_cli(saved.clone(), &config, &info(None)).is_none());
        let other = SshConfig { target: "user@other".into(), ..config };
        assert!(with_launched_cli(saved, &other, &info(Some("2.1.312 (Claude Code)"))).is_none());
    }
    #[test]
    fn permission_mode_defaults_to_auto_and_roundtrips_explicit_bypass() {
        let old = json!({"version":1,"remote":null,"authorizedTarget":null,"info":null});
        let mut settings: ConnectionSettings = serde_json::from_value(old).unwrap();
        assert_eq!(settings.permission_mode, PermissionMode::Auto);
        assert!(!settings.prewarm);
        assert_eq!(settings.runtime, super::super::runtime_settings::RuntimeSettings::default());
        settings.runtime.idle_compaction = true;
        settings.runtime.telemetry = true;
        settings.runtime.auto_compact_window = Some(350_000);
        assert_eq!((settings.pool_capacity, settings.idle_minutes), (1, 15));
        assert_eq!(ConnectionSettings::default().permission_mode.as_str(), "auto");
        settings.permission_mode = PermissionMode::BypassPermissions;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("connection.json");
        write_json(&path, &settings).unwrap();
        let saved: ConnectionSettings = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved.permission_mode.as_str(), "bypassPermissions");
        assert_eq!(saved.runtime, settings.runtime);
        assert_eq!(serde_json::to_value(&saved).unwrap()["idleCompaction"], true);
        assert_eq!(serde_json::to_value(&saved).unwrap()["telemetry"], true);
        assert_eq!(serde_json::to_value(&saved).unwrap()["autoCompactWindow"], 350_000);
        assert!(serde_json::from_value::<PermissionMode>(json!("unknown")).is_err());
    }
    fn config() -> SshConfig {
        SshConfig {
            target: "user@host".into(),
            port: None,
            identity_file: None,
            known_hosts_file: None,
            cli_path: None,
        }
    }
    #[test]
    fn ssh_targets_cannot_inject_options_or_shell() {
        for target in [
            "-oProxyCommand=evil",
            "user@host;id",
            "$(id)",
            "host\n",
            "host path",
        ] {
            assert!(SshConfig {
                target: target.into(),
                ..config()
            }
            .validate()
            .is_err());
        }
        assert!(config().validate().is_ok());
        assert_ne!(
            config().fingerprint(),
            SshConfig {
                port: Some(2222),
                ..config()
            }
            .fingerprint()
        );
    }
    #[test]
    fn connection_binding_is_versioned_pinned_and_migrates_local() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join("old.json");
        assert_eq!(
            resolve_binding(&old, true, || Ok(Some(config()))).unwrap(),
            None
        );
        assert_eq!(
            resolve_binding(&old, false, || panic!("must stay local")).unwrap(),
            None
        );
        let new = root.path().join("new.json");
        assert_eq!(
            resolve_binding(&new, false, || Ok(Some(config()))).unwrap(),
            Some(config())
        );
        assert_eq!(
            resolve_binding(&new, true, || panic!("must stay remote")).unwrap(),
            Some(config())
        );
        std::fs::write(&new, r#"{"version":2,"remote":null}"#).unwrap();
        assert!(resolve_binding(&new, false, || Ok(None)).is_err());
    }
}

/// Versioned server identities never pass through the direct SSH sidecar.
pub fn native_session_connection(id:&str,legacy_local:bool,recorded:Option<Option<super::connection::NativeConnection>>)->Result<Option<super::connection::NativeConnection>,String>{
    use super::connection::NativeConnection;
    match recorded {
        Some(Some(connection @ NativeConnection::Server{..})) => {connection.validate()?;Ok(Some(connection))}
        other => {
            let recorded=other.map(|value|value.and_then(|value|value.as_ssh().cloned()));
            session_connection(id,legacy_local,recorded).map(|value|value.map(NativeConnection::Ssh))
        }
    }
}
