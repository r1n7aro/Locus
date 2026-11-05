use crate::llm::claude_code_cli::ssh::{
    self, ConnectionSettings, PermissionMode, RemoteInfo, SshConfig,
};
use crate::session::store::SessionStore;
use std::sync::Arc;
use tauri::{AppHandle,State};
use crate::workspace_service::ProjectRegistry;

#[tauri::command]
pub fn get_claude_native_archive(session_id:String,store:State<'_,Arc<SessionStore>>)->Result<Option<crate::llm::claude_code_cli::archive::ArchiveInfo>,String> {
    store.claude_archive_info(&session_id)
}

/// The conversation's pinned Agent types: backend and model of each, for
/// the Agent cards. `None` when the conversation has no native route.
#[tauri::command]
pub fn get_claude_agent_routes(session_id:String,store:State<'_,Arc<SessionStore>>)->Result<Option<serde_json::Value>,String> {
    crate::agent::instance::claude_agent_routes(store.inner(),&session_id)
}

/// Agent card id → the model its native Claude subagent ran on and its
/// read-only record (live changes arrive as `claude-agent-card`).
#[tauri::command]
pub fn get_claude_agent_cards(session_id:String,store:State<'_,Arc<SessionStore>>)->Result<std::collections::HashMap<String,crate::agent::instance::ClaudeAgentCard>,String> {
    crate::agent::instance::claude_agent_cards(store.inner(),&session_id)
}

#[tauri::command]
pub async fn backup_claude_native_session(session_id:String,store:State<'_,Arc<SessionStore>>)->Result<Option<crate::llm::claude_code_cli::archive::ArchiveInfo>,String> {
    crate::llm::claude_code_cli::archive::backup(store.inner(),&session_id).await
}

#[tauri::command]
pub fn check_claude_session_transfer(session_id:String,store:State<'_,Arc<SessionStore>>)->Result<Option<crate::llm::claude_code_cli::archive::TransferCheck>,String> {
    crate::llm::claude_code_cli::archive::transfer_check(store.inner(),&session_id)
}

#[tauri::command]
pub async fn restore_claude_native_session(session_id:String,expected_target:String,expected_native:String,expected_archive:String,store:State<'_,Arc<SessionStore>>,app_handle:AppHandle,workspace_registry:State<'_,Arc<ProjectRegistry>>)->Result<crate::llm::claude_code_cli::archive::RestoreReport,String> {
    let report=crate::llm::claude_code_cli::archive::restore(store.inner(),&session_id,&expected_target,&expected_native,&expected_archive).await?;
    if let Ok(Some(scope))=super::session::resolve_optional_session_workspace_scope(store.inner(),workspace_registry.inner(),&session_id,"restore_claude_native_session") {
        super::emit_session_content_changed(&app_handle,
            Some(&crate::workspace_service::event::WorkspaceEventScope::for_runtime(scope.runtime())),
            &scope.runtime().root().to_string_lossy(),&session_id,"claude_native_restore");
    }
    Ok(report)
}

#[tauri::command]
pub fn get_claude_session_connection(session_id: Option<String>, working_dir: Option<String>, model: Option<String>) -> Result<Option<crate::llm::claude_code_cli::tui::ConnectionStatus>, String> {
    let settings = ssh::load_settings()?;
    Ok(crate::llm::claude_code_cli::tui::connection_overview(session_id.as_deref(), working_dir.as_deref(), model.as_deref(), &settings))
}

#[tauri::command]
pub async fn connect_claude_session(
    model: String, workspace_ref: crate::workspace_service::WorkspaceRef,
    workspace_registry: State<'_, Arc<ProjectRegistry>>,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), String> {
    use crate::llm::claude_code_cli::tui::prewarm;
    let settings = ssh::load_settings()?;
    let remote = prewarm::target(&model,&settings)?;
    let scope = super::session::resolve_workspace_scope(workspace_registry.inner(), &workspace_ref, "connectClaude")
        .map_err(|e| e.to_string())?;
    let runtime = scope.runtime();
    prewarm::connect(prewarm::Context { cwd: runtime.root().display().to_string(), checkout: runtime.checkout_id().to_string(),
        epoch: runtime.materialization_epoch(), model, remote, permission: settings.permission_mode, runtime: settings.runtime,
        native_tool_search: config.dynamic_tool_loading_mode() == crate::config::DynamicToolLoadingMode::Native }).await
}

#[tauri::command]
pub async fn prewarm_claude_session(
    session_id: Option<String>, model: String, workspace_ref: crate::workspace_service::WorkspaceRef, warm_existing: bool,
    store: State<'_, Arc<SessionStore>>, workspace_registry: State<'_, Arc<ProjectRegistry>>,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<(), String> {
    use crate::llm::claude_code_cli::{recovery, tui::prewarm};
    // Background warm-up never starts CLI processes while the experimental
    // provider is off.
    if !config.claude_code_cli_enabled() { return Ok(()); }
    let settings = ssh::load_settings()?;
    if !settings.prewarm { return Ok(()); }
    let Ok(remote) = prewarm::target(&model,&settings) else { return Ok(()); };
    let scope = if let Some(id) = session_id.as_deref() {
        super::session::resolve_session_workspace_scope(store.inner(), workspace_registry.inner(), id, Some(&workspace_ref), "prewarmClaude")
    } else {
        super::session::resolve_workspace_scope(workspace_registry.inner(), &workspace_ref, "prewarmClaude")
    }.map_err(|e| e.to_string())?;
    let runtime = scope.runtime();
    let context = prewarm::Context { cwd: runtime.root().display().to_string(), checkout: runtime.checkout_id().to_string(),
        epoch: runtime.materialization_epoch(), model, remote: remote.clone(), permission: settings.permission_mode, runtime: settings.runtime,
        native_tool_search: config.dynamic_tool_loading_mode() == crate::config::DynamicToolLoadingMode::Native };
    // A pending/unsynchronized conversation must not prevent the server pool
    // from connecting. Only resuming that particular conversation needs history.
    prewarm::maintain(context.clone(), None, warm_existing);
    // Interest heartbeats never re-read the full transcript or resume a process.
    // Only explicit selection validates history and can warm a bound session.
    let existing = if let Some(id) = session_id.filter(|_| warm_existing) {
        if store.active_run_for_session(&id)?.is_some() { return Ok(()); }
        let native=recovery::native_session_id(store.inner(),&id)?;
        if native.is_none() {
            let messages = store.get_messages(&id)?;
            let metadata = store.get_response_request_metadata(&id)?;
            let pending = store.claude_recovery_pending_messages(&id, &messages, &metadata, None)?;
            store.verify_claude_history_origin(&id, &messages, &metadata, &pending)?;
            if !pending.is_empty() { return Ok(()); }
        }
        if crate::llm::claude_code_cli::archive::transfer_check(store.inner(), &id)?.is_some() { return Ok(()); }
        if let Some(bound) = recovery::recorded_connection(store.inner(), &id)? {
            if bound.as_ref() != Some(&remote) { return Ok(()); }
        }
        Some((id.clone(), native))
    } else { None };
    if existing.is_some() { prewarm::maintain(context, existing, warm_existing); }
    Ok(())
}

#[tauri::command]
pub fn get_claude_connection() -> Result<ConnectionSettings, String> {
    let mut settings = ssh::load_settings()?;
    if let Some(config) = &settings.remote {
        settings.has_unlock_password = ssh::unlock::has_saved(config)?;
    }
    Ok(settings)
}

#[tauri::command]
pub fn set_claude_permission_mode(mode: PermissionMode) -> Result<ConnectionSettings, String> {
    let mut settings = ssh::load_settings()?;
    settings.permission_mode = mode;
    ssh::save_settings(&settings)?;
    get_claude_connection()
}

#[tauri::command]
pub fn set_claude_runtime_settings(idle_compaction: bool, telemetry: bool, auto_compact_window: Option<u32>) -> Result<ConnectionSettings, String> {
    let mut settings = ssh::load_settings()?;
    settings.runtime = crate::llm::claude_code_cli::runtime_settings::RuntimeSettings { idle_compaction, telemetry, auto_compact_window };
    ssh::save_settings(&settings)?;
    crate::llm::claude_code_cli::tui::retire_outdated_runtime_settings(settings.runtime);
    get_claude_connection()
}

/// Applies to conversations that have not pinned their route yet. Running
/// processes are unaffected: existing conversations keep their own pin.
#[tauri::command]
pub fn set_claude_native_agents(
    enabled: bool,
    builtins: Vec<String>,
    excluded_locus_agents: Option<Vec<String>>,
) -> Result<ConnectionSettings, String> {
    let mut settings = ssh::load_settings()?;
    settings.native_multi_agent = enabled;
    settings.native_agents.builtins = builtins;
    // Omitted by older clients: keep the saved exclusions.
    if let Some(excluded) = excluded_locus_agents {
        settings.native_agents.excluded_locus_agents = excluded;
    }
    ssh::save_settings(&settings)?;
    get_claude_connection()
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeNativeAgentsSupport {
    pub cli_version: Option<String>,
    pub supported: bool,
    /// The saved SSH connection, not the local CLI.
    pub remote: bool,
    /// "Test and save" installs the required version: an SSH connection
    /// without a custom Claude path.
    pub installable: bool,
}

/// Whether new conversations on the configured connection can use native
/// agents: the version check a new conversation runs when it pins its route.
#[tauri::command]
pub async fn get_claude_native_agents_support() -> Result<ClaudeNativeAgentsSupport, String> {
    tokio::task::spawn_blocking(|| {
        let settings = ssh::load_settings()?;
        let cli_version = crate::llm::claude_code_cli::native_agents_cli_version(&settings);
        let supported = cli_version.as_deref().is_some_and(|version| {
            crate::llm::claude_code_cli::version_at_least(
                version,
                crate::llm::claude_code_cli::NATIVE_AGENTS_MIN_VERSION,
            )
        });
        let remote = settings.remote.is_some();
        let installable = settings
            .remote
            .as_ref()
            .is_some_and(|remote| remote.cli_path.is_none());
        Ok(ClaudeNativeAgentsSupport { cli_version, supported, remote, installable })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn set_claude_prewarm(enabled: bool, capacity: u32, idle_minutes: u32) -> Result<ConnectionSettings, String> {
    if ![5, 15, 30, 60].contains(&idle_minutes) { return Err("Invalid idle timeout".into()); }
    if !(1..=4).contains(&capacity) { return Err("Pool capacity must be between 1 and 4".into()); }
    let mut settings = ssh::load_settings()?;
    settings.prewarm = enabled;
    settings.pool_capacity = capacity;
    settings.idle_minutes = idle_minutes;
    ssh::save_settings(&settings)?;
    if !enabled { crate::llm::claude_code_cli::tui::prewarm::cancel_all(); }
    get_claude_connection()
}

/// Inspection may unlock the macOS keychain using the saved password and run
/// CLI auth status. It never installs software or changes the Claude login.
#[tauri::command]
pub async fn inspect_claude_ssh(config: SshConfig) -> Result<RemoteInfo, String> {
    let info = ssh::probe(&config, false).await?;
    if let Ok(mut settings) = ssh::load_settings() {
        if settings.remote.as_ref() == Some(&config) {
            settings.info = Some(info.clone());
            ssh::save_settings(&settings)?;
        }
    }
    Ok(info)
}

/// Called only by the explicit "Test and save" action in Settings.
/// Authorization is recorded only after setup for this exact target succeeds.
#[tauri::command]
pub async fn authorize_claude_ssh(
    config: SshConfig,
    unlock_password: Option<String>,
) -> Result<ConnectionSettings, String> {
    let password = match unlock_password {
        Some(value) => ssh::unlock::UnlockPassword::supplied(value)?,
        None => ssh::unlock::UnlockPassword::Saved,
    };
    let info = ssh::probe_with_unlock(
        &config,
        true,
        match &password {
            ssh::unlock::UnlockPassword::Supplied(value) => {
                ssh::unlock::UnlockPassword::Supplied(value.clone())
            }
            ssh::unlock::UnlockPassword::Saved => ssh::unlock::UnlockPassword::Saved,
        },
    )
    .await?;
    validate_connection_test(
        &info,
        matches!(password, ssh::unlock::UnlockPassword::Supplied(_)),
    )?;
    let previous_settings = ssh::load_settings()?;
    let settings = ConnectionSettings {
        version: 1,
        authorized_target: Some(config.fingerprint()),
        remote: Some(config),
        info: Some(info),
        permission_mode: previous_settings.permission_mode,
        runtime: previous_settings.runtime,
        prewarm: previous_settings.prewarm,
        pool_capacity: previous_settings.pool_capacity,
        idle_minutes: previous_settings.idle_minutes,
        native_multi_agent: previous_settings.native_multi_agent,
        native_agents: previous_settings.native_agents,
        has_unlock_password: false,
    };
    // Testing never replaces a working secret. Commit only after CLI auth succeeds;
    // restore the previous secret if writing the connection configuration fails.
    if let ssh::unlock::UnlockPassword::Supplied(value) = password {
        let key = ssh::unlock::key(settings.remote.as_ref().unwrap())?;
        let previous = ssh::unlock::UnlockPassword::Saved.resolve(&key)?;
        crate::keychain::set_secret(&key, &value)?;
        if let Err(error) = ssh::save_settings(&settings) {
            let rollback = match previous {
                Some(previous) => crate::keychain::set_secret(&key, &previous),
                None => crate::keychain::delete_secret(&key),
            };
            return Err(if rollback.is_ok() {
                error
            } else {
                "Connection save failed and the previous unlock password could not be restored; clear or replace the saved password".into()
            });
        }
    } else {
        ssh::save_settings(&settings)?;
    }
    crate::llm::claude_code_cli::tui::retire_idle_remote_connections(settings.remote.as_ref());
    get_claude_connection()
}

fn validate_connection_test(info: &RemoteInfo, supplied_password: bool) -> Result<(), String> {
    if !info.cli_supported || !info.authenticated {
        return Err(if info.keychain_unlocked == Some(false) {
            "Remote macOS keychain is locked; enter its unlock password and test again"
        } else {
            "Remote Claude authentication check failed; the connection was not saved"
        }
        .into());
    }
    if supplied_password
        && (info.platform.as_deref() != Some("darwin") || !info.keychain_unlock_tested)
    {
        return Err(
            "Unlock passwords can only be saved after a successful macOS keychain test".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn clear_claude_ssh_unlock_password(config: SshConfig) -> Result<(), String> {
    crate::keychain::delete_secret(&ssh::unlock::key(&config)?)
}

#[tauri::command]
pub fn use_local_claude() -> Result<ConnectionSettings, String> {
    let previous = ssh::load_settings()?;
    let settings = ConnectionSettings {
        permission_mode: previous.permission_mode,
        runtime: previous.runtime,
        prewarm: previous.prewarm,
        pool_capacity: previous.pool_capacity,
        idle_minutes: previous.idle_minutes,
        native_multi_agent: previous.native_multi_agent,
        native_agents: previous.native_agents,
        ..ConnectionSettings::default()
    };
    ssh::save_settings(&settings)?;
    crate::llm::claude_code_cli::tui::prewarm::cancel_all();
    crate::llm::claude_code_cli::tui::retire_idle_remote_connections(None);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_or_non_mac_unlock_tests_cannot_replace_saved_passwords() {
        let mut info = RemoteInfo {
            cli_supported: true,
            authenticated: true,
            platform: Some("linux".into()),
            ..RemoteInfo::default()
        };
        assert!(validate_connection_test(&info, false).is_ok());
        assert!(validate_connection_test(&info, true).is_err());
        info.platform = Some("darwin".into());
        assert!(validate_connection_test(&info, true).is_err());
        info.keychain_unlock_tested = true;
        assert!(validate_connection_test(&info, true).is_ok());
        info.authenticated = false;
        assert!(validate_connection_test(&info, true).is_err());
    }
}
