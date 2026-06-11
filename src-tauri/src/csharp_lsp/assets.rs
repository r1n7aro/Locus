//! Acquisition of the Roslyn language server binaries.
//!
//! The server (`Microsoft.CodeAnalysis.LanguageServer.<rid>` from the public
//! `vs-impl` NuGet feed, MIT) and the Microsoft.Unity.Analyzers package are
//! downloaded on demand into the persistent app config dir
//! (`%APPDATA%/locus/csharp-lsp/...`) the first time the feature is used.
//!
//! The nuget.org server build (5.0.0-1.25277.114) is pinned against a 2025
//! MSBuild and its net472 BuildHost fails to initialize against VS 2026
//! (MSBuild 18.x) installs, so a current feed build is pinned instead.
//!
//! The .NET runtime that hosts the server is acquired by the shared
//! `crate::dotnet_runtime` module (the compile-server sidecar reuses it).

use std::path::PathBuf;

use crate::dotnet_runtime::{self, download_to_file, extract_zip, is_complete, mark_complete};

pub const SERVER_VERSION: &str = "5.4.0-2.26179.14";
/// Microsoft.Unity.Analyzers (MIT) from nuget.org — Unity-specific Roslyn
/// diagnostics (UNT*) plus suppressors for general C# diagnostics that are
/// wrong in Unity code (e.g. "make serialized field readonly").
pub const UNITY_ANALYZERS_VERSION: &str = "1.26.0";

pub use crate::dotnet_runtime::{is_platform_supported, platform_rid};

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

fn server_download_url(rid: &str) -> String {
    let id = format!("microsoft.codeanalysis.languageserver.{rid}");
    format!(
        "https://pkgs.dev.azure.com/azure-public/vside/_packaging/vs-impl/nuget/v3/flat2/{id}/{version}/{id}.{version}.nupkg",
        id = id,
        version = SERVER_VERSION.to_ascii_lowercase()
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

async fn ensure_server_installed(rid: &str, progress: &ProgressFn) -> Result<PathBuf, String> {
    let dir = server_dir(rid)?;
    let dll = dir.join("Microsoft.CodeAnalysis.LanguageServer.dll");
    if is_complete(&dir) && dll.is_file() {
        return Ok(dll);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create server dir: {e}"))?;

    let nupkg = dir.join("server.nupkg");
    let component_progress = move |received, total| progress(AssetComponent::Server, received, total);
    download_to_file(&server_download_url(rid), &nupkg, &component_progress).await?;

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

/// Single-flight across workspaces for the LSP-specific assets (server +
/// analyzers). The shared runtime has its own single-flight in
/// `dotnet_runtime`.
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
    let component_progress =
        move |received, total| progress(AssetComponent::UnityAnalyzers, received, total);
    download_to_file(&unity_analyzers_download_url(), &nupkg, &component_progress).await?;

    let extract_dir = dir.clone();
    let archive = nupkg.clone();
    tokio::task::spawn_blocking(move || {
        extract_zip(&archive, &extract_dir, Some("analyzers/dotnet/cs/"))
    })
    .await
    .map_err(|e| format!("Extraction task failed: {e}"))??;
    let _ = std::fs::remove_file(&nupkg);

    if !dll.is_file() {
        return Err("Analyzer package did not contain Microsoft.Unity.Analyzers.dll".to_string());
    }
    mark_complete(&dir)?;
    Ok(dll)
}

/// Ensure server + runtime are available, downloading them when missing.
pub async fn ensure_assets(progress: &ProgressFn) -> Result<ResolvedAssets, String> {
    let rid = platform_rid()
        .ok_or_else(|| "C# code analysis is not supported on this platform yet".to_string())?;

    let server_dll = {
        let _install_guard = INSTALL_LOCK.lock().await;
        ensure_server_installed(rid, progress).await?
    };

    let dotnet_progress =
        move |received, total| progress(AssetComponent::DotnetRuntime, received, total);
    let dotnet = dotnet_runtime::ensure_dotnet(&dotnet_progress).await?;

    Ok(ResolvedAssets {
        dotnet_program: dotnet.program,
        server_dll,
        envs: dotnet.envs,
        dotnet_source: dotnet.source,
    })
}
