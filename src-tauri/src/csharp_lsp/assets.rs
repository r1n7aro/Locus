//! Acquisition of the Roslyn language server binaries and the .NET runtime
//! that hosts them.
//!
//! Both assets are downloaded on demand into the persistent app config dir
//! (`%APPDATA%/locus/csharp-lsp/...`) the first time the feature is used:
//!
//! - Server: `Microsoft.CodeAnalysis.LanguageServer.<rid>` from the public
//!   `vs-impl` NuGet feed (MIT). The nuget.org build (5.0.0-1.25277.114) is
//!   pinned against a 2025 MSBuild and its net472 BuildHost fails to
//!   initialize against VS 2026 (MSBuild 18.x) installs, so we pin a current
//!   feed build instead.
//! - Runtime: the server targets net10.0 and is framework-dependent. A system
//!   `dotnet` with a 10.x runtime is preferred; otherwise the official
//!   dotnet-runtime archive is downloaded next to the server.

use std::path::{Path, PathBuf};

use tokio::io::AsyncWriteExt;

pub const SERVER_VERSION: &str = "5.4.0-2.26179.14";
const DOTNET_RUNTIME_VERSION: &str = "10.0.9";
const DOTNET_RUNTIME_MAJOR: &str = "10.";
/// Microsoft.Unity.Analyzers (MIT) from nuget.org — Unity-specific Roslyn
/// diagnostics (UNT*) plus suppressors for general C# diagnostics that are
/// wrong in Unity code (e.g. "make serialized field readonly").
pub const UNITY_ANALYZERS_VERSION: &str = "1.26.0";
const COMPLETE_MARKER: &str = ".locus-complete";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetComponent {
    Server,
    DotnetRuntime,
    UnityAnalyzers,
}

impl AssetComponent {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetComponent::Server => "server",
            AssetComponent::DotnetRuntime => "dotnet",
            AssetComponent::UnityAnalyzers => "analyzers",
        }
    }
}

/// Progress callback: (component, received bytes, total bytes if known).
pub type ProgressFn = dyn Fn(AssetComponent, u64, Option<u64>) + Send + Sync;

#[derive(Debug, Clone)]
pub struct ResolvedAssets {
    /// Host executable to spawn (`dotnet` / managed `dotnet.exe`).
    pub dotnet_program: PathBuf,
    /// `Microsoft.CodeAnalysis.LanguageServer.dll` path, passed as first arg.
    pub server_dll: PathBuf,
    /// Extra environment for the child process.
    pub envs: Vec<(String, String)>,
    /// "system" or "managed" — surfaced in the status UI.
    pub dotnet_source: &'static str,
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

fn root_dir() -> Result<PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join("csharp-lsp"))
}

pub fn logs_dir() -> Result<PathBuf, String> {
    let dir = root_dir()?.join("logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log dir: {e}"))?;
    Ok(dir)
}

fn server_dir(rid: &str) -> Result<PathBuf, String> {
    Ok(root_dir()?.join("server").join(SERVER_VERSION).join(rid))
}

fn dotnet_dir(rid: &str) -> Result<PathBuf, String> {
    Ok(root_dir()?
        .join("dotnet")
        .join(DOTNET_RUNTIME_VERSION)
        .join(rid))
}

fn server_download_url(rid: &str) -> String {
    let id = format!("microsoft.codeanalysis.languageserver.{rid}");
    format!(
        "https://pkgs.dev.azure.com/azure-public/vside/_packaging/vs-impl/nuget/v3/flat2/{id}/{version}/{id}.{version}.nupkg",
        id = id,
        version = SERVER_VERSION.to_ascii_lowercase()
    )
}

fn dotnet_download_url(rid: &str) -> String {
    format!(
        "https://builds.dotnet.microsoft.com/dotnet/Runtime/{v}/dotnet-runtime-{v}-{rid}.zip",
        v = DOTNET_RUNTIME_VERSION
    )
}

fn unity_analyzers_dir() -> Result<PathBuf, String> {
    Ok(root_dir()?
        .join("unity-analyzers")
        .join(UNITY_ANALYZERS_VERSION))
}

fn unity_analyzers_download_url() -> String {
    format!(
        "https://api.nuget.org/v3-flatcontainer/microsoft.unity.analyzers/{v}/microsoft.unity.analyzers.{v}.nupkg",
        v = UNITY_ANALYZERS_VERSION
    )
}

fn is_complete(dir: &Path) -> bool {
    dir.join(COMPLETE_MARKER).is_file()
}

fn mark_complete(dir: &Path) -> Result<(), String> {
    std::fs::write(dir.join(COMPLETE_MARKER), SERVER_VERSION)
        .map_err(|e| format!("Failed to write completion marker: {e}"))
}

/// Accept only stable runtimes of the required major version: a `10.x.y`
/// line qualifies, `10.0.0-preview`/`-rc` builds do not (the server may rely
/// on APIs that changed before GA, and there is no fallback once the system
/// dotnet is chosen).
fn runtime_line_supports_server(version: &str) -> bool {
    version.starts_with(DOTNET_RUNTIME_MAJOR) && !version.contains('-')
}

/// Probe the system `dotnet` for a runtime able to host the server.
async fn system_dotnet_supports_server() -> bool {
    let mut cmd = crate::process_util::async_command("dotnet");
    cmd.arg("--list-runtimes")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let output = match tokio::time::timeout(std::time::Duration::from_secs(4), cmd.output()).await
    {
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
            .map(runtime_line_supports_server)
            .unwrap_or(false)
    })
}

async fn download_to_file(
    url: &str,
    target: &Path,
    component: AssetComponent,
    progress: &ProgressFn,
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
        progress(component, received, total);
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
fn extract_zip(archive: &Path, target_dir: &Path, strip_prefix: Option<&str>) -> Result<(), String> {
    let file =
        std::fs::File::open(archive).map_err(|e| format!("Failed to open archive: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read archive: {e}"))?;
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
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create dir: {e}"))?;
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

async fn ensure_server_installed(rid: &str, progress: &ProgressFn) -> Result<PathBuf, String> {
    let dir = server_dir(rid)?;
    let dll = dir.join("Microsoft.CodeAnalysis.LanguageServer.dll");
    if is_complete(&dir) && dll.is_file() {
        return Ok(dll);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create server dir: {e}"))?;

    let nupkg = dir.join("server.nupkg");
    download_to_file(
        &server_download_url(rid),
        &nupkg,
        AssetComponent::Server,
        progress,
    )
    .await?;

    let extract_dir = dir.clone();
    let prefix = format!("content/LanguageServer/{rid}/");
    let archive = nupkg.clone();
    tokio::task::spawn_blocking(move || extract_zip(&archive, &extract_dir, Some(&prefix)))
        .await
        .map_err(|e| format!("Extraction task failed: {e}"))??;
    let _ = std::fs::remove_file(&nupkg);

    if !dll.is_file() {
        return Err("Server archive did not contain Microsoft.CodeAnalysis.LanguageServer.dll".to_string());
    }
    mark_complete(&dir)?;
    Ok(dll)
}

async fn ensure_dotnet_installed(rid: &str, progress: &ProgressFn) -> Result<PathBuf, String> {
    let dir = dotnet_dir(rid)?;
    let exe = dir.join(if cfg!(windows) { "dotnet.exe" } else { "dotnet" });
    if is_complete(&dir) && exe.is_file() {
        return Ok(exe);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create runtime dir: {e}"))?;

    let archive = dir.join("dotnet-runtime.zip");
    download_to_file(
        &dotnet_download_url(rid),
        &archive,
        AssetComponent::DotnetRuntime,
        progress,
    )
    .await?;

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

/// Single-flight across workspaces: concurrent first-time setups would
/// otherwise race on remove_dir_all + extract in the shared install dirs.
static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Ensure the Microsoft.Unity.Analyzers assembly is available, downloading
/// the NuGet package when missing. Returns the analyzer DLL path.
pub async fn ensure_unity_analyzers(progress: &ProgressFn) -> Result<PathBuf, String> {
    let _install_guard = INSTALL_LOCK.lock().await;

    let dir = unity_analyzers_dir()?;
    let dll = dir.join("Microsoft.Unity.Analyzers.dll");
    if is_complete(&dir) && dll.is_file() {
        return Ok(dll);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create analyzers dir: {e}"))?;

    let nupkg = dir.join("analyzers.nupkg");
    download_to_file(
        &unity_analyzers_download_url(),
        &nupkg,
        AssetComponent::UnityAnalyzers,
        progress,
    )
    .await?;

    let extract_dir = dir.clone();
    let archive = nupkg.clone();
    tokio::task::spawn_blocking(move || {
        extract_zip(&archive, &extract_dir, Some("analyzers/dotnet/cs/"))
    })
    .await
    .map_err(|e| format!("Extraction task failed: {e}"))??;
    let _ = std::fs::remove_file(&nupkg);

    if !dll.is_file() {
        return Err(
            "Analyzer package did not contain Microsoft.Unity.Analyzers.dll".to_string(),
        );
    }
    mark_complete(&dir)?;
    Ok(dll)
}

/// Ensure server + runtime are available, downloading them when missing.
pub async fn ensure_assets(progress: &ProgressFn) -> Result<ResolvedAssets, String> {
    let rid = platform_rid()
        .ok_or_else(|| "C# code analysis is not supported on this platform yet".to_string())?;

    let _install_guard = INSTALL_LOCK.lock().await;

    let server_dll = ensure_server_installed(rid, progress).await?;

    if system_dotnet_supports_server().await {
        return Ok(ResolvedAssets {
            dotnet_program: PathBuf::from("dotnet"),
            server_dll,
            envs: vec![
                ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
                // Belt and braces with the LSP `initialize` locale: keep
                // .NET resource strings English on localized systems.
                ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
            ],
            dotnet_source: "system",
        });
    }

    let dotnet = ensure_dotnet_installed(rid, progress).await?;
    let dotnet_root = dotnet
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(ResolvedAssets {
        dotnet_program: dotnet,
        server_dll,
        envs: vec![
            ("DOTNET_ROOT".to_string(), dotnet_root),
            ("DOTNET_CLI_TELEMETRY_OPTOUT".to_string(), "1".to_string()),
            ("DOTNET_CLI_UI_LANGUAGE".to_string(), "en".to_string()),
        ],
        dotnet_source: "managed",
    })
}
