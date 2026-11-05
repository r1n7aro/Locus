//! Add-on install/update/uninstall across installations and hot reload of
//! running hosts. Shared by Settings, the status popover and the SDK.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::addon::{self, AddonStatus};
use super::hub::{self, ConnectionState};
use super::installations::{self, Installation};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReport {
    pub installation: String,
    pub version: Option<String>,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Running instances of this scripts directory asked to reload.
    pub reloaded: Vec<u32>,
    /// Instances that could not reload now (busy or failed): restart them.
    pub restart_needed: Vec<u32>,
}

fn user_scripts(installation: &Installation) -> Result<&str, String> {
    installation
        .probe
        .as_ref()
        .map(|probe| probe.user_scripts.as_str())
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| format!("{} has not been probed yet", installation.path))
}

/// Install or update the host for one installation, then ask running
/// instances that loaded an older host from the same directory to reload.
pub async fn install(installation: &Installation) -> InstallReport {
    let mut report = InstallReport {
        installation: installation.id.clone(),
        version: installation.display_version.clone(),
        ok: false,
        error: None,
        reloaded: Vec::new(),
        restart_needed: Vec::new(),
    };
    let scripts = match user_scripts(installation) {
        Ok(scripts) => scripts.to_string(),
        Err(error) => {
            report.error = Some(error);
            return report;
        }
    };
    let result = tokio::task::spawn_blocking({
        let scripts = scripts.clone();
        move || addon::install(Path::new(&scripts))
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    match result {
        Ok(hash) => {
            report.ok = true;
            let installed_dir = addon::installed_dir(Path::new(&scripts));
            let (reloaded, failed) = reload_outdated(Some(&installed_dir), &hash).await;
            report.reloaded = reloaded;
            report.restart_needed = failed;
        }
        Err(error) => report.error = Some(error),
    }
    notify_changed();
    report
}

pub async fn uninstall(installation: &Installation) -> Result<(), String> {
    let scripts = user_scripts(installation)?.to_string();
    let result = tokio::task::spawn_blocking(move || addon::uninstall(Path::new(&scripts)))
        .await
        .map_err(|error| error.to_string())?;
    notify_changed();
    result
}

/// Install for the given installation id, the default, or every supported
/// installation (`all`).
pub async fn install_selected(installation: Option<&str>, all: bool) -> Result<Vec<InstallReport>, String> {
    let list = installations::list_probed().await;
    let supported = |item: &&Installation| {
        !item.hidden
            && matches!(
                item.support,
                installations::SupportStatus::Supported | installations::SupportStatus::Unverified
            )
    };
    let targets: Vec<&Installation> = if all {
        list.iter().filter(supported).collect()
    } else {
        let chosen = installations::choose(&list, installation, None)
            .ok_or_else(|| "No supported Blender installation (4.2 or newer) is available.".to_string())?;
        vec![chosen]
    };
    let mut reports = Vec::new();
    for item in targets {
        reports.push(install(item).await);
    }
    Ok(reports)
}

/// Ask connected instances whose host hash differs from `expected` (and,
/// when given, that loaded the host from `host_dir`) to reload. Returns
/// (reloaded pids, pids that need a restart).
pub async fn reload_outdated(host_dir: Option<&Path>, expected: &str) -> (Vec<u32>, Vec<u32>) {
    let Some(hub) = hub::current() else {
        return (Vec::new(), Vec::new());
    };
    let wanted_dir = host_dir.map(installations::normalize_path);
    let mut reloaded = Vec::new();
    let mut failed = Vec::new();
    for instance in hub.instances() {
        let registration = instance.registration();
        if registration.host_hash == expected || registration.host_hash == "source" {
            continue;
        }
        if let Some(wanted) = wanted_dir.as_deref() {
            if installations::normalize_path(Path::new(&registration.host_dir)) != *wanted {
                continue;
            }
        }
        if instance.connection() != ConnectionState::Connected {
            failed.push(registration.pid);
            continue;
        }
        match instance.request("reload_host", "", Duration::from_secs(10)).await {
            Ok(envelope) if envelope.ok == Some(true) => reloaded.push(registration.pid),
            _ => failed.push(registration.pid),
        }
    }
    (reloaded, failed)
}

/// `locus.blender.reload_hosts()`: every connected instance running a host
/// that differs from the bundled one reloads when idle.
pub async fn reload_hosts() -> Result<(Vec<u32>, Vec<u32>), String> {
    let expected = addon::expected_hash().ok_or_else(|| "Blender host sources are missing".to_string())?;
    Ok(reload_outdated(None, &expected).await)
}

fn notify_changed() {
    if let Some(app) = hub::app_handle() {
        use tauri::Emitter;
        let _ = app.emit("blender-installations-changed", ());
    }
}

/// Status of the add-on for the installation a workspace uses by default:
/// drives the one notification Locus shows (other installations are
/// handled in the popover).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultAddonStatus {
    pub installation: Option<Installation>,
    pub status: Option<AddonStatus>,
    /// Running instances with an older host than the bundled one.
    pub outdated_instances: Vec<u32>,
}

pub async fn default_addon_status(scope: &super::target::Scope) -> DefaultAddonStatus {
    let installation = super::target::choose_installation(scope, None, None, None).await.ok();
    let status = installation.as_ref().and_then(|item| item.addon.as_ref()).map(|addon| addon.status);
    let expected = addon::expected_hash();
    let outdated_instances = hub::current()
        .map(|hub| {
            hub.instances()
                .iter()
                .map(|instance| instance.registration())
                .filter(|registration| {
                    expected.as_deref().is_some_and(|expected| {
                        registration.host_hash != expected && registration.host_hash != "source"
                    })
                })
                .map(|registration| registration.pid)
                .collect()
        })
        .unwrap_or_default();
    DefaultAddonStatus { installation, status, outdated_instances }
}
