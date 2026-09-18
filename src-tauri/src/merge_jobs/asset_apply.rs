//! A narrow bridge adapter for merge plans containing existing text assets only.
//! The caller persists the merge journal before entering this function and runs
//! on spawn_blocking; the Editor never calls back into the merge filesystem lock.
use super::{io, FileState, MergeJob};
use base64::Engine;
use serde_json::{json, Value};
use sha2::Digest;
use std::path::Path;

fn project_asset_path(root: &Path, project: &Path, path: &str) -> Option<String> {
    let absolute = root.join(path);
    let relative = absolute
        .strip_prefix(project)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    if !relative.starts_with("Assets/") {
        return None;
    }
    let extension = Path::new(&relative)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    [
        "asset",
        "prefab",
        "unity",
        "mat",
        "anim",
        "controller",
        "overridecontroller",
        "playable",
        "mask",
    ]
    .contains(&extension.as_str())
    .then_some(relative)
}

fn request(root: &Path, dir: &Path, job: &MergeJob) -> Result<Option<Value>, String> {
    if job.applied_files.is_empty() || job.applied_files.len() > 256 {
        return Ok(None);
    }
    let mut entries = Vec::new();
    for (path, after) in &job.applied_files {
        let (Some(before), Some(after)) = (
            job.applied_before.get(path).and_then(Option::as_ref),
            after.as_ref(),
        ) else {
            return Ok(None);
        };
        if before.mode != after.mode {
            return Ok(None);
        }
        let Some(asset_path) = project_asset_path(root, Path::new(&job.project_root), path) else {
            return Ok(None);
        };
        let before_bytes = io::read_blob(dir, before)?;
        let after_bytes = io::read_blob(dir, after)?;
        let text_asset =
            |bytes: &[u8]| bytes.starts_with(b"%YAML") || bytes.starts_with(b"--- !u!");
        if !text_asset(&before_bytes)
            || !text_asset(&after_bytes)
            || after_bytes.len() > 128 * 1024 * 1024
        {
            return Ok(None);
        }
        entries.push(json!({"path":asset_path,
            "expected_sha256":sha2::Sha256::digest(&before_bytes).iter().map(|byte|format!("{byte:02x}")).collect::<String>(),
            "bytes_base64":base64::engine::general_purpose::STANDARD.encode(&after_bytes)}));
    }
    Ok(Some(
        json!({"action":"disk_apply","transaction_id":job.id,"entries":entries}),
    ))
}

/// Returns false only when this plan is outside the adapter or the Editor is
/// proven offline. A connected Editor rejection never falls back to raw writes.
pub(super) fn try_apply(root: &Path, dir: &Path, job: &MergeJob) -> Result<bool, String> {
    if !crate::unity_bridge::is_unity_project(&job.project_root) {
        return Ok(false);
    }
    let Some(request) = request(root, dir, job)? else {
        return Ok(false);
    };
    let runtime = match tokio::runtime::Handle::try_current() {
        Ok(runtime) => runtime,
        Err(_) => {
            let process = crate::unity_bridge::query_current_project_editor_process_uncached(
                job.project_root.clone(),
            );
            return match process.state {
                crate::unity_bridge::UnityEditorProcessState::NotRunning => Ok(false),
                _ => Err("merge.editor_coordination_required: apply this Unity asset plan through the async SDK/IPC host".into()),
            };
        }
    };
    if !runtime.block_on(crate::unity_bridge::is_unity_connected(&job.project_root)) {
        runtime.block_on(crate::unity_assets::require_closed_editor(Path::new(
            &job.project_root,
        )))?;
        return Ok(false);
    }
    let response = runtime.block_on(crate::unity_bridge::asset_api(&job.project_root, &request))?;
    if response["applied"].as_bool() != Some(true) || response["persisted"].as_bool() != Some(true)
    {
        return Err(
            "assets.outcome_unknown: Editor did not confirm the merge asset transaction".into(),
        );
    }
    Ok(true)
}

pub(super) fn files_match(
    root: &Path,
    dir: &Path,
    states: &std::collections::BTreeMap<String, Option<FileState>>,
) -> Result<(), String> {
    for (path, expected) in states {
        if io::capture_like(root, dir, path, expected.as_ref())? != *expected {
            return Err(format!("Merge transaction bytes differ at {path}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapter_only_selects_project_assets_with_supported_extensions() {
        let root = Path::new("/repo");
        let project = Path::new("/repo/Unity");
        assert_eq!(
            project_asset_path(root, project, "Unity/Assets/Config.asset").as_deref(),
            Some("Assets/Config.asset")
        );
        assert!(project_asset_path(root, project, "Unity/Assets/Config.asset.meta").is_none());
        assert!(project_asset_path(root, project, "Unity/Assets/Script.cs").is_none());
        assert!(project_asset_path(root, project, "Unity/Packages/Local/Config.asset").is_none());
        assert!(project_asset_path(root, project, "Other/Assets/Config.asset").is_none());
    }
}
