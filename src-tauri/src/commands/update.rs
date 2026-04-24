use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

const PUBLIC_UPDATE_BASE_URL: &str = "https://unity.farlocus.com";
const UPDATE_MANIFEST_PATH: &str = "/data/update.json";
const PUBLIC_UPDATE_MANIFEST_URL: &str = "https://unity.farlocus.com/data/update.json";
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
    pub locales: HashMap<String, AppUpdateLocaleEntry>,
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
    base_url: String,
    manifest_url: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}

fn build_update_sources() -> Vec<AppUpdateSource> {
    let mut sources = Vec::new();

    if cfg!(debug_assertions) {
        for port in LOCAL_UPDATE_PORT_RANGE {
            let base_url = format!("http://localhost:{}", port);
            sources.push(AppUpdateSource {
                kind: AppUpdateSourceKind::Local,
                manifest_url: format!("{}{}", base_url, UPDATE_MANIFEST_PATH),
                base_url,
                connect_timeout: Duration::from_millis(180),
                request_timeout: Duration::from_millis(450),
            });
        }
    }

    sources.push(AppUpdateSource {
        kind: AppUpdateSourceKind::Remote,
        base_url: PUBLIC_UPDATE_BASE_URL.to_string(),
        manifest_url: PUBLIC_UPDATE_MANIFEST_URL.to_string(),
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(8),
    });

    sources
}

fn validate_manifest(manifest: &AppUpdateManifest) -> Result<(), AppError> {
    if manifest.version.trim().is_empty() {
        return Err("Update manifest version is empty".into());
    }

    if manifest.locales.is_empty() {
        return Err("Update manifest locales are empty".into());
    }

    Ok(())
}

async fn fetch_manifest_from_source(
    source: &AppUpdateSource,
) -> Result<AppUpdateManifest, AppError> {
    let client = reqwest::Client::builder()
        .connect_timeout(source.connect_timeout)
        .timeout(source.request_timeout)
        .gzip(true)
        .deflate(true)
        .user_agent(concat!("Locus/", env!("CARGO_PKG_VERSION")))
        .build()
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

    validate_manifest(&manifest)?;
    Ok(manifest)
}

#[tauri::command]
pub async fn fetch_app_update_manifest() -> Result<AppUpdateManifestFetchResult, AppError> {
    let mut last_error: Option<AppError> = None;

    for source in build_update_sources() {
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
