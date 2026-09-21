//! Independent macOS host acquisition. The Windows implementation is unchanged.
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[path = "macos_dotnet_archive.rs"]
mod archive;
use archive::extract_runtime;

// Share the existing download and NuGet extraction helpers, not its platform
// selection or runtime installation. Public types keep sidecar callers stable.
#[allow(dead_code)]
#[path = "dotnet_runtime.rs"]
mod shared;
pub use shared::{
    download_to_file, extract_zip, is_complete, mark_complete, ProgressFn, ResolvedDotnet,
    DOTNET_RUNTIME_VERSION,
};

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn platform_rid() -> Option<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Some("osx-arm64"),
        "x86_64" => Some("osx-x64"),
        _ => None,
    }
}

pub fn is_platform_supported() -> bool {
    platform_rid().is_some()
}

fn runtime_dir(rid: &str) -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?
        .join("csharp-lsp")
        .join("dotnet")
        .join(DOTNET_RUNTIME_VERSION)
        .join(rid))
}

fn resolved(program: PathBuf, source: &'static str) -> ResolvedDotnet {
    let mut envs = vec![
        ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
        ("DOTNET_CLI_UI_LANGUAGE".into(), "en".into()),
    ];
    if source == "managed" {
        if let Some(root) = program.parent() {
            envs.push(("DOTNET_ROOT".into(), root.to_string_lossy().into_owned()));
        }
    }
    ResolvedDotnet {
        program,
        source,
        envs,
    }
}

async fn system_dotnet() -> Option<PathBuf> {
    // Explicit paths also work when launched through Finder's restricted PATH.
    let mut candidates = vec![PathBuf::from("dotnet")];
    if cfg!(target_arch = "x86_64") {
        candidates.push(PathBuf::from("/usr/local/share/dotnet/x64/dotnet"));
    }
    candidates.extend([
        PathBuf::from("/usr/local/share/dotnet/dotnet"),
        PathBuf::from("/usr/local/bin/dotnet"),
        PathBuf::from("/opt/homebrew/bin/dotnet"),
    ]);
    for candidate in candidates {
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            crate::process_util::async_command(&candidate.to_string_lossy())
                .arg("--list-runtimes")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = output {
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
                    line.trim()
                        .strip_prefix("Microsoft.NETCore.App ")
                        .and_then(|rest| rest.split_whitespace().next())
                        .is_some_and(|version| version.starts_with("10.") && !version.contains('-'))
                })
            {
                return Some(candidate);
            }
        }
    }
    None
}

fn cached(rid: &str) -> Option<ResolvedDotnet> {
    let dir = runtime_dir(rid).ok()?;
    let host = dir.join("dotnet");
    if is_complete(&dir)
        && host.is_file()
        && std::fs::metadata(&host).ok()?.permissions().mode() & 0o111 != 0
    {
        Some(resolved(host, "managed"))
    } else {
        None
    }
}

pub async fn try_resolve_cached_dotnet() -> Option<ResolvedDotnet> {
    let rid = platform_rid()?;
    if let Some(host) = system_dotnet().await {
        return Some(resolved(host, "system"));
    }
    cached(rid)
}

pub async fn ensure_dotnet(progress: &ProgressFn<'_>) -> Result<ResolvedDotnet, String> {
    let rid = platform_rid().ok_or("Managed sidecars are not supported on this architecture")?;
    let _guard = INSTALL_LOCK.lock().await;
    if let Some(host) = system_dotnet().await {
        return Ok(resolved(host, "system"));
    }
    if let Some(host) = cached(rid) {
        return Ok(host);
    }
    let dir = runtime_dir(rid)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join("dotnet-runtime.tar.gz");
    let url = format!(
        "https://builds.dotnet.microsoft.com/dotnet/Runtime/{0}/dotnet-runtime-{0}-{rid}.tar.gz",
        DOTNET_RUNTIME_VERSION
    );
    download_to_file(&url, &archive, progress).await?;
    let archive_path = archive.clone();
    let destination = dir.clone();
    tokio::task::spawn_blocking(move || extract_runtime(&archive_path, &destination))
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;
    let _ = std::fs::remove_file(archive);
    mark_complete(&dir)?;
    Ok(resolved(dir.join("dotnet"), "managed"))
}
