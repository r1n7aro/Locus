//! Shared acquisition of the .NET runtime that hosts Locus' framework-dependent
//! managed sidecars (the Roslyn language server in `csharp_lsp` and the C#
//! compile server in `csharp_compile`).
//!
//! A system `dotnet` with a 10.x runtime is preferred; otherwise the official
//! dotnet-runtime archive is downloaded once into the persistent app config dir
//! and reused by every sidecar.
//!
//! The runtime is cached under `%APPDATA%/locus/csharp-lsp/dotnet/{ver}/{rid}`.
//! That `csharp-lsp` path segment is legacy: it predates the runtime being
//! shared, and is preserved verbatim so existing users do not re-download the
//! ~30 MB archive after this module was split out of `csharp_lsp::assets`.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

/// The runtime line all managed sidecars target. Both the LSP server and the
/// compile server are framework-dependent against this.
pub const DOTNET_RUNTIME_VERSION: &str = "10.0.9";
const DOTNET_RUNTIME_MAJOR: &str = "10.";
const COMPLETE_MARKER: &str = ".locus-complete";

/// Progress callback: (received bytes, total bytes if known). Carries a
/// lifetime parameter so adapter closures may capture borrowed callbacks
/// (a bare `dyn` alias would default the object bound to `'static`).
pub type ProgressFn<'a> = dyn Fn(u64, Option<u64>) + Send + Sync + 'a;

/// Host executable + environment needed to launch a framework-dependent
/// managed DLL on the resolved runtime.
#[derive(Debug, Clone)]
pub struct ResolvedDotnet {
    /// Host executable to spawn (`dotnet` / managed `dotnet.exe`).
    pub program: PathBuf,
    /// Extra environment for the child process.
    pub envs: Vec<(String, String)>,
    /// "system" or "managed" — surfaced in status UIs.
    pub source: &'static str,
}

pub fn platform_rid() -> Option<&'static str> {
    // The download/extract pipeline below assumes zip archives; macOS/Linux
    // runtime archives are tar.gz, so only Windows is wired up for now.
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("win-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("win-arm64")
    } else {
        None
    }
}

pub fn is_platform_supported() -> bool {
    platform_rid().is_some()
}

/// Root of the shared runtime cache (see module docs for the legacy path).
fn root_dir() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join("csharp-lsp"))
}

fn dotnet_dir(rid: &str) -> Result<PathBuf, String> {
    Ok(root_dir()?
        .join("dotnet")
        .join(DOTNET_RUNTIME_VERSION)
        .join(rid))
}

fn dotnet_download_url(rid: &str) -> String {
    format!(
        "https://builds.dotnet.microsoft.com/dotnet/Runtime/{v}/dotnet-runtime-{v}-{rid}.zip",
        v = DOTNET_RUNTIME_VERSION
    )
}

pub fn is_complete(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).is_file()
}

pub fn mark_complete(dir: &Path) -> Result<(), String> {
    std::fs::write(dir.join(COMPLETE_MARKER), DOTNET_RUNTIME_VERSION)
        .map_err(|e| format!("Failed to write completion marker: {e}"))
}

/// Accept only stable runtimes of the required major version: a `10.x.y` line
/// qualifies, `10.0.0-preview`/`-rc` builds do not (a sidecar may rely on APIs
/// that changed before GA, and there is no fallback once the system dotnet is
/// chosen).
fn runtime_line_supports_sidecars(version: &str) -> bool {
    version.starts_with(DOTNET_RUNTIME_MAJOR) && !version.contains('-')
}

/// Probe the system `dotnet` for a runtime able to host the sidecars.
async fn system_dotnet_supports_sidecars() -> bool {
    let mut cmd = crate::process_util::async_command("dotnet");
    cmd.arg("--list-runtimes")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = match tokio::time::timeout(std::time::Duration::from_secs(4), cmd.output()).await {
        Ok(Ok(output)) => output,
        _ => return false,
    };
    if !output.status.success() {
        return false;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(|line| {
        line.trim()
            .strip_prefix("Microsoft.NETCore.App ")
            .and_then(|rest| rest.split_whitespace().next())
            .map(runtime_line_supports_sidecars)
            .unwrap_or(false)
    })
}

/// Download `url` to `target`, reporting progress.
pub async fn download_to_file(
    url: &str,
    target: &Path,
    progress: &ProgressFn<'_>,
) -> Result<(), String> {
    let client = crate::network::default_reqwest_client()
        .map_err(|e| format!("HTTP client unavailable: {e}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed ({url}): {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Download failed ({url}): HTTP {}",
            response.status()
        ));
    }
    let total = response.content_length();
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    let partial = target.with_extension("partial");
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| format!("Failed to create download file: {e}"))?;
    let mut received: u64 = 0;
    let mut stream = response;
    loop {
        let chunk = stream
            .chunk()
            .await
            .map_err(|e| format!("Download interrupted: {e}"))?;
        let Some(chunk) = chunk else { break };
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write download: {e}"))?;
        received += chunk.len() as u64;
        progress(received, total);
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush download: {e}"))?;
    drop(file);
    tokio::fs::rename(&partial, target)
        .await
        .map_err(|e| format!("Failed to finalize download: {e}"))?;
    Ok(())
}

/// Extract `archive` into `target_dir`, keeping only entries under
/// `strip_prefix` (when provided) with that prefix removed.
pub fn extract_zip(
    archive: &Path,
    target_dir: &Path,
    strip_prefix: Option<&str>,
) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("Failed to open archive: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read archive: {e}"))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| format!("Failed to read archive entry: {e}"))?;
        let Some(raw_path) = entry.enclosed_name() else {
            continue;
        };
        let raw = raw_path.to_string_lossy().replace('\\', "/");
        let relative = match strip_prefix {
            Some(prefix) => match raw.strip_prefix(prefix) {
                Some(rest) if !rest.is_empty() => rest.to_string(),
                _ => continue,
            },
            None => raw,
        };
        let out_path = target_dir.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| format!("Failed to create dir: {e}"))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
        }
        let mut out =
            std::fs::File::create(&out_path).map_err(|e| format!("Failed to extract: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("Failed to extract: {e}"))?;
    }
    Ok(())
}

/// Single-flight across all sidecars: concurrent first-time setups would
/// otherwise race on remove_dir_all + extract in the shared install dir.
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn ensure_dotnet_installed(rid: &str, progress: &ProgressFn<'_>) -> Result<PathBuf, String> {
    let dir = dotnet_dir(rid)?;
    let exe = dir.join(if cfg!(windows) {
        "dotnet.exe"
    } else {
        "dotnet"
    });
    if is_complete(&dir) && exe.is_file() {
        return Ok(exe);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create runtime dir: {e}"))?;

    let archive = dir.join("dotnet-runtime.zip");
    download_to_file(&dotnet_download_url(rid), &archive, progress).await?;

    let extract_dir = dir.clone();
    let archive_path = archive.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive_path, &extract_dir, None))
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;
    let _ = std::fs::remove_file(&archive);

    if !exe.is_file() {
        return Err("Runtime archive did not contain the dotnet host".to_string());
    }
    mark_complete(&dir)?;
    Ok(exe)
}

/// Resolve a host without ever downloading: the system `dotnet` when
/// suitable, else an already-cached managed runtime, else `None`.
/// Used by tests and cheap availability probes.
pub async fn try_resolve_cached_dotnet() -> Option<ResolvedDotnet> {
    let rid = platform_rid()?;

    if system_dotnet_supports_sidecars().await {
        return Some(ResolvedDotnet {
            program: PathBuf::from("dotnet"),
            envs: vec![
                ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
                ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
            ],
            source: "system",
        });
    }

    let dir = dotnet_dir(rid).ok()?;
    let exe = dir.join(if cfg!(windows) {
        "dotnet.exe"
    } else {
        "dotnet"
    });
    if !is_complete(&dir) || !exe.is_file() {
        return None;
    }
    let dotnet_root = exe
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Some(ResolvedDotnet {
        program: exe,
        envs: vec![
            ("DOTNET_ROOT".to_string(), dotnet_root),
            ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
            ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
        ],
        source: "managed",
    })
}

/// Resolve a host capable of running the framework-dependent sidecars,
/// downloading the runtime when no suitable system `dotnet` is present.
pub async fn ensure_dotnet(progress: &ProgressFn<'_>) -> Result<ResolvedDotnet, String> {
    let rid = platform_rid()
        .ok_or_else(|| "Managed sidecars are not supported on this platform yet".to_string())?;

    let _install_guard = INSTALL_LOCK.lock().await;

    if system_dotnet_supports_sidecars().await {
        return Ok(ResolvedDotnet {
            program: PathBuf::from("dotnet"),
            envs: vec![
                ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
                // Keep .NET resource strings English on localized systems so
                // diagnostics/log scraping stays locale-independent.
                ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
            ],
            source: "system",
        });
    }

    let dotnet = ensure_dotnet_installed(rid, progress).await?;
    let dotnet_root = dotnet
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(ResolvedDotnet {
        program: dotnet,
        envs: vec![
            ("DOTNET_ROOT".to_string(), dotnet_root),
            ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
            ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
        ],
        source: "managed",
    })
}
