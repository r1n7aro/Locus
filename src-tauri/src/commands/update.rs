use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

const PUBLIC_UPDATE_BASE_URL: &str = "https://unity.farlocus.com";
const STABLE_UPDATE_MANIFEST_PATH: &str = "/data/update.json";
const EXPERIMENTAL_UPDATE_MANIFEST_PATH: &str = "/data/update-experimental.json";
const LOCAL_UPDATE_PORT_RANGE: std::ops::RangeInclusive<u16> = 3000..=3005;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateChangeGroup {
    pub title: String,
    #[serde(default)]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateDownloadChannel {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInstallerDownload {
    pub id: String,
    pub label: String,
    pub url: String,
    pub platform: String,
    pub arch: String,
    pub includes_managed_python: bool,
    pub includes_managed_git: bool,
    pub requires_system_python: bool,
    pub requires_system_git: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateLocaleEntry {
    pub title: String,
    pub summary: String,
    pub changelog_url: String,
    #[serde(default)]
    pub changes: Vec<AppUpdateChangeGroup>,
    #[serde(default)]
    pub download_channels: Vec<AppUpdateDownloadChannel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateManifest {
    pub version: String,
    pub released_at: String,
    pub channel: String,
    #[serde(default)]
    pub installers: Vec<AppUpdateInstallerDownload>,
    pub locales: HashMap<String, AppUpdateLocaleEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppUpdateChannel {
    Stable,
    Experimental,
}

impl AppUpdateChannel {
    fn as_str(self) -> &'static str {
        match self {
            AppUpdateChannel::Stable => "stable",
            AppUpdateChannel::Experimental => "experimental",
        }
    }

    fn manifest_path(self) -> &'static str {
        match self {
            AppUpdateChannel::Stable => STABLE_UPDATE_MANIFEST_PATH,
            AppUpdateChannel::Experimental => EXPERIMENTAL_UPDATE_MANIFEST_PATH,
        }
    }

    fn from_manifest_channel(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(AppUpdateChannel::Stable),
            "experimental" => Some(AppUpdateChannel::Experimental),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppUpdateSourceKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateManifestFetchResult {
    pub manifest: AppUpdateManifest,
    pub source_kind: AppUpdateSourceKind,
    pub source_base_url: String,
}

#[derive(Debug, Clone)]
struct AppUpdateSource {
    kind: AppUpdateSourceKind,
    channel: AppUpdateChannel,
    base_url: String,
    manifest_url: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}

fn channel_fallbacks(channel: AppUpdateChannel) -> Vec<AppUpdateChannel> {
    match channel {
        AppUpdateChannel::Stable => vec![AppUpdateChannel::Stable],
        AppUpdateChannel::Experimental => {
            vec![AppUpdateChannel::Experimental, AppUpdateChannel::Stable]
        }
    }
}

fn build_update_sources(channel: AppUpdateChannel) -> Vec<AppUpdateSource> {
    let mut sources = Vec::new();
    let fallback_channels = channel_fallbacks(channel);

    if cfg!(debug_assertions) {
        for source_channel in fallback_channels.iter().copied() {
            for port in LOCAL_UPDATE_PORT_RANGE.clone() {
                let base_url = format!("http://localhost:{}", port);
                sources.push(AppUpdateSource {
                    kind: AppUpdateSourceKind::Local,
                    channel: source_channel,
                    manifest_url: format!("{}{}", base_url, source_channel.manifest_path()),
                    base_url,
                    connect_timeout: Duration::from_millis(180),
                    request_timeout: Duration::from_millis(450),
                });
            }
        }
    }

    for source_channel in fallback_channels {
        sources.push(AppUpdateSource {
            kind: AppUpdateSourceKind::Remote,
            channel: source_channel,
            base_url: PUBLIC_UPDATE_BASE_URL.to_string(),
            manifest_url: format!(
                "{}{}",
                PUBLIC_UPDATE_BASE_URL,
                source_channel.manifest_path()
            ),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(8),
        });
    }

    sources
}

fn validate_manifest(
    manifest: &AppUpdateManifest,
    expected_channel: AppUpdateChannel,
) -> Result<(), AppError> {
    if manifest.version.trim().is_empty() {
        return Err("Update manifest version is empty".into());
    }

    if manifest.channel.trim().is_empty() {
        return Err("Update manifest channel is empty".into());
    }

    if AppUpdateChannel::from_manifest_channel(&manifest.channel) != Some(expected_channel) {
        return Err(format!(
            "Update manifest channel mismatch: expected {}, got {}",
            expected_channel.as_str(),
            manifest.channel
        )
        .into());
    }

    if manifest.locales.is_empty() {
        return Err("Update manifest locales are empty".into());
    }

    Ok(())
}

async fn fetch_manifest_from_source(
    source: &AppUpdateSource,
) -> Result<AppUpdateManifest, AppError> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(source.connect_timeout)
            .timeout(source.request_timeout)
            .gzip(true)
            .deflate(true)
            .user_agent(concat!("Locus/", env!("CARGO_PKG_VERSION"))),
    )
    .map_err(|e| format!("Failed to create update manifest client: {}", e))?;

    let response = client
        .get(&source.manifest_url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch update manifest: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("Failed to fetch update manifest: HTTP {}", status.as_u16()).into());
    }

    let manifest = response
        .json::<AppUpdateManifest>()
        .await
        .map_err(|e| format!("Failed to parse update manifest: {}", e))?;

    validate_manifest(&manifest, source.channel)?;
    Ok(manifest)
}

#[tauri::command]
pub async fn fetch_app_update_manifest(
    channel: Option<AppUpdateChannel>,
) -> Result<AppUpdateManifestFetchResult, AppError> {
    let mut last_error: Option<AppError> = None;
    let requested_channel = channel.unwrap_or(AppUpdateChannel::Stable);

    for source in build_update_sources(requested_channel) {
        match fetch_manifest_from_source(&source).await {
            Ok(manifest) => {
                return Ok(AppUpdateManifestFetchResult {
                    manifest,
                    source_kind: source.kind,
                    source_base_url: source.base_url,
                });
            }
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "Failed to fetch update manifest".into()))
}
