use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use base64::Engine;
use futures::StreamExt;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::{sleep, timeout};
use url::Url;
use walkdir::WalkDir;

use crate::agent::definition::AgentDefRegistry;
use crate::error::AppError;
use crate::plugin::{
    inspect_plugin_source_manifest_sync, install_plugin_from_path_sync,
    list_installed_plugin_summaries, normalize_plugin_id, set_plugin_enabled_sync,
    uninstall_plugin_sync, InstalledPluginSummary, LocusPluginProjectDependency,
    PluginInstallScope, PLUGIN_MANIFEST_FILE_NAME,
};
use crate::process_util::{async_command, resolve_github_cli};
use crate::workspace::Workspace;
use crate::{AgentDefRegistryState, AppAgentDir};

pub const PLUGINS_CHANGED_EVENT: &str = "plugins-changed";
const DEFAULT_PLUGIN_REGISTRY_BASE_URL: &str =
    "https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/main/public/v1";
const PLUGIN_REGISTRY_DOWNLOAD_MAX_BYTES: u64 = 512 * 1024 * 1024;
const PLUGIN_REGISTRY_JSON_MAX_BYTES: u64 = 4 * 1024 * 1024;
const PLUGIN_REGISTRY_DESCRIPTION_MAX_BYTES: u64 = 2 * 1024 * 1024;
const PLUGIN_REGISTRY_USER_AGENT: &str = concat!("Locus/", env!("CARGO_PKG_VERSION"));
const DEFAULT_PLUGIN_REGISTRY_DESCRIPTION_BRANCH: &str = "main";
const DEFAULT_PLUGIN_REGISTRY_DESCRIPTION_PATH: &str = "README.md";
const PLUGIN_REGISTRY_INDEX_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const PLUGIN_REGISTRY_DESCRIPTION_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const PLUGIN_GITHUB_AUTH_CONFIG_FILE: &str = "plugin-github-auth.json";
const PLUGIN_GITHUB_CLI_HOSTNAME: &str = "github.com";
const PLUGIN_GITHUB_CLI_SCOPES: &str = "repo,read:org";
const PLUGIN_GITHUB_CLI_VERIFICATION_URI: &str = "https://github.com/login/device";
const PLUGIN_GITHUB_CLI_LOGIN_INTERVAL: u64 = 2;
const PLUGIN_GITHUB_CLI_LOGIN_CODE_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const PLUGIN_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(5 * 60);

static MARKDOWN_LINK_DEST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?P<prefix>!?\[[^\]\n]*\]\()(?P<url><[^>\n]+>|[^)\s\n]+)(?P<suffix>(?:\s+["'][^"'\n]*["'])?\))"#)
        .expect("markdown link destination regex")
});
static GITHUB_CLI_DEVICE_CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b([A-Z0-9]{4}-[A-Z0-9]{4}|[A-Z0-9]{8})\b").expect("GitHub CLI device code regex")
});
static REGISTRY_HTTP_CLIENT: LazyLock<Result<reqwest::Client, String>> =
    LazyLock::new(|| build_registry_http_client(Duration::from_secs(12)));
static REGISTRY_DOWNLOAD_HTTP_CLIENT: LazyLock<Result<reqwest::Client, String>> =
    LazyLock::new(|| build_registry_http_client(Duration::from_secs(120)));
static PLUGIN_GITHUB_CLI_LOGIN_SESSION: LazyLock<Mutex<Option<PluginGithubCliLoginSession>>> =
    LazyLock::new(|| Mutex::new(None));

fn project_agent_dir(working_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(working_dir)
        .join("Locus")
        .join("agent")
}

pub(crate) async fn reload_agent_registry(
    registry: &AgentDefRegistryState,
    app_agent_dir: &AppAgentDir,
    working_dir: &str,
) {
    let project_agent_dir = project_agent_dir(working_dir);
    let project_agent_opt = project_agent_dir
        .is_dir()
        .then_some(project_agent_dir.as_path());
    let next = AgentDefRegistry::load_with_plugins(
        app_agent_dir.0.as_deref(),
        project_agent_opt,
        &crate::plugin::installed_agent_sources(working_dir),
    );
    *registry.0.write().await = next;
}

fn default_registry_entry_base_path() -> String {
    "plugins".to_string()
}

fn default_registry_summary_base_path() -> String {
    "shards".to_string()
}

fn default_registry_search_index_path() -> String {
    "search/summaries.json".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryManifest {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub registry_version: u32,
    #[serde(default)]
    pub bucket_strategy: String,
    #[serde(default)]
    pub bucket_count: u32,
    #[serde(default = "default_registry_entry_base_path")]
    pub entry_base_path: String,
    #[serde(default = "default_registry_summary_base_path")]
    pub summary_base_path: String,
    #[serde(default = "default_registry_search_index_path")]
    pub search_index_path: String,
    #[serde(default)]
    pub available_buckets: Vec<String>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryCompatibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_locus_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_independent: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryIcon {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub url: String,
}

impl PluginRegistryIcon {
    fn is_empty(&self) -> bool {
        self.kind.is_empty() && self.id.is_empty() && self.url.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryStat {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default, skip_serializing_if = "PluginRegistryIcon::is_empty")]
    pub icon: PluginRegistryIcon,
}

impl PluginRegistryStat {
    fn is_empty(&self) -> bool {
        self.id.is_empty() && self.label.is_empty() && self.value.is_null() && self.icon.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryDescriptionSource {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub path: String,
}

impl PluginRegistryDescriptionSource {
    fn is_empty(&self) -> bool {
        self.kind.is_empty()
            && self.url.is_empty()
            && self.repo.is_empty()
            && self.branch.is_empty()
            && self.path.is_empty()
    }
}

fn localized_text_is_empty(value: &BTreeMap<String, String>) -> bool {
    value.is_empty()
}

fn localized_description_source_is_empty(
    value: &BTreeMap<String, PluginRegistryDescriptionSource>,
) -> bool {
    value.is_empty()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryDownload {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

impl PluginRegistryDownload {
    fn is_empty(&self) -> bool {
        self.url.trim().is_empty() && self.sha256.trim().is_empty() && self.size_bytes.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginDownloadSource {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default, rename = "ref")]
    pub ref_name: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub asset: String,
    #[serde(default)]
    pub asset_pattern: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub version: String,
}

impl PluginDownloadSource {
    fn is_empty(&self) -> bool {
        self.kind.trim().is_empty()
            && self.id.trim().is_empty()
            && self.input.trim().is_empty()
            && self.url.trim().is_empty()
            && self.repo.trim().is_empty()
            && self.ref_name.trim().is_empty()
            && self.branch.trim().is_empty()
            && self.tag.trim().is_empty()
            && self.commit.trim().is_empty()
            && self.asset.trim().is_empty()
            && self.asset_pattern.trim().is_empty()
            && self.sha256.trim().is_empty()
            && self.size_bytes.is_none()
            && self.version.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistrySummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "localized_text_is_empty")]
    pub summary_i18n: BTreeMap<String, String>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub latest_version: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "PluginRegistryIcon::is_empty")]
    pub icon: PluginRegistryIcon,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<PluginRegistryStat>,
    #[serde(default)]
    pub compatibility: PluginRegistryCompatibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryShard {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub plugins: Vec<PluginRegistrySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistrySearchIndex {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub generated_at: String,
    #[serde(default)]
    pub plugins: Vec<PluginRegistrySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryEntry {
    #[serde(default)]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "localized_text_is_empty")]
    pub summary_i18n: BTreeMap<String, String>,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "localized_text_is_empty")]
    pub description_i18n: BTreeMap<String, String>,
    #[serde(
        default,
        skip_serializing_if = "PluginRegistryDescriptionSource::is_empty"
    )]
    pub description_source: PluginRegistryDescriptionSource,
    #[serde(default, skip_serializing_if = "localized_description_source_is_empty")]
    pub description_source_i18n: BTreeMap<String, PluginRegistryDescriptionSource>,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub latest_version: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "PluginRegistryIcon::is_empty")]
    pub icon: PluginRegistryIcon,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stats: Vec<PluginRegistryStat>,
    #[serde(default, skip_serializing_if = "PluginRegistryDownload::is_empty")]
    pub download: PluginRegistryDownload,
    #[serde(default, skip_serializing_if = "PluginDownloadSource::is_empty")]
    pub download_source: PluginDownloadSource,
    #[serde(default)]
    pub compatibility: PluginRegistryCompatibility,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryManifestFetchResult {
    pub base_url: String,
    pub manifest: PluginRegistryManifest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryDescriptionFetchResult {
    pub content: String,
    pub source_url: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PluginRegistryCacheMode {
    #[default]
    Default,
    CachePreferred,
    NetworkPreferred,
}

impl PluginRegistryCacheMode {
    fn prefer_any_cache(self) -> bool {
        matches!(self, Self::CachePreferred)
    }

    fn skip_fresh_cache(self) -> bool {
        matches!(self, Self::NetworkPreferred)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRegistryInstallRequest {
    pub id: String,
    #[serde(default)]
    pub latest_version: String,
    #[serde(default)]
    pub download: PluginRegistryDownload,
    #[serde(default)]
    pub download_source: PluginDownloadSource,
}

fn normalize_registry_base_url(value: Option<String>) -> Result<String, String> {
    let raw = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_PLUGIN_REGISTRY_BASE_URL);
    let parsed =
        Url::parse(raw).map_err(|error| format!("Invalid plugin registry URL: {}", error))?;
    ensure_secure_plugin_url(&parsed, "Plugin registry")?;
    Ok(raw.trim_end_matches('/').to_string())
}

fn plugin_url_is_local_http(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn ensure_secure_plugin_url(url: &Url, label: &str) -> Result<(), String> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if plugin_url_is_local_http(url) => Ok(()),
        "http" => Err(format!(
            "{} URL must use HTTPS unless it points to localhost",
            label
        )),
        _ => Err(format!("{} URL must use http or https", label)),
    }
}

fn normalize_registry_subpath(
    value: Option<String>,
    default_value: &str,
) -> Result<String, String> {
    let raw = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_value)
        .replace('\\', "/");
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty()
        || trimmed.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
        })
    {
        return Err(format!("Invalid plugin registry path: {}", raw));
    }
    Ok(trimmed.to_string())
}

fn resolve_registry_url(base_url: &str, rel_path: &str) -> Result<String, String> {
    let normalized = rel_path.trim_start_matches('/');
    if normalized.is_empty()
        || normalized.contains('\\')
        || normalized.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
        })
    {
        return Err(format!("Invalid plugin registry path: {}", rel_path));
    }
    let base = Url::parse(&format!("{}/", base_url.trim_end_matches('/')))
        .map_err(|error| format!("Invalid plugin registry URL: {}", error))?;
    let resolved = base
        .join(normalized)
        .map_err(|error| format!("Invalid plugin registry URL: {}", error))?;
    Ok(resolved.to_string())
}

fn normalize_registry_bucket(value: &str) -> Result<String, String> {
    let bucket = value.trim().to_ascii_lowercase();
    if bucket.len() != 2 || !bucket.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("Invalid plugin registry bucket".to_string());
    }
    Ok(bucket)
}

fn plugin_registry_bucket_for_id(plugin_id: &str) -> Result<String, String> {
    let plugin_id = normalize_plugin_id(plugin_id)?;
    let mut hasher = Sha256::new();
    hasher.update(plugin_id.as_bytes());
    let digest = hasher.finalize();
    Ok(format!("{:02x}", digest[0]))
}

fn normalize_sha256(value: &str) -> Result<String, String> {
    let hash = value.trim().to_ascii_lowercase();
    if hash.len() != 64 || !hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("Plugin registry entry has an invalid sha256".to_string());
    }
    Ok(hash)
}

fn normalize_optional_sha256(value: &str) -> Result<Option<String>, String> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        normalize_sha256(value).map(Some)
    }
}

fn normalize_download_source_kind(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !matches!(ch, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_git_ref(value: &str, label: &str) -> Result<String, String> {
    let git_ref = value.trim().trim_matches('/');
    if git_ref.is_empty()
        || git_ref.contains('\\')
        || git_ref.contains("..")
        || git_ref.chars().any(char::is_whitespace)
        || git_ref
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(format!("Invalid plugin download {}: {}", label, value));
    }
    Ok(git_ref.to_string())
}

fn normalize_commit_ref(value: &str) -> Result<String, String> {
    let commit = value.trim();
    if commit.len() < 7 || commit.len() > 64 || !commit.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("Invalid plugin download commit: {}", value));
    }
    Ok(commit.to_string())
}

fn source_primary_input(source: &PluginDownloadSource) -> String {
    for value in [&source.input, &source.url, &source.repo] {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    String::new()
}

fn source_repo_value(source: &PluginDownloadSource, fallback_repo: Option<&str>) -> String {
    let repo = source.repo.trim();
    if !repo.is_empty() {
        return repo.to_string();
    }
    fallback_repo
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn source_ref_value(
    source: &PluginDownloadSource,
    kind: &str,
    default_value: Option<&str>,
) -> String {
    let candidates: &[&str] = match kind {
        "branch" => &[&source.branch, &source.ref_name],
        "tag" | "release" => &[&source.tag, &source.ref_name],
        "commit" => &[&source.commit, &source.ref_name],
        _ => &[
            &source.ref_name,
            &source.branch,
            &source.tag,
            &source.commit,
        ],
    };
    for value in candidates {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    default_value.unwrap_or_default().to_string()
}

fn normalize_registry_icon(icon: &mut PluginRegistryIcon) {
    icon.kind = icon.kind.trim().to_ascii_lowercase();
    icon.id = icon.id.trim().to_string();
    icon.url = icon.url.trim().to_string();
    if icon.kind != "locus" && icon.kind != "url" {
        icon.kind = if !icon.url.is_empty() {
            "url".to_string()
        } else if !icon.id.is_empty() {
            "locus".to_string()
        } else {
            String::new()
        };
    }
    if icon.kind == "locus" {
        icon.url.clear();
    } else if icon.kind == "url" {
        icon.id.clear();
    }
}

fn normalize_registry_stat(stat: &mut PluginRegistryStat) {
    stat.id = stat.id.trim().to_string();
    stat.label = stat.label.trim().to_string();
    normalize_registry_icon(&mut stat.icon);
}

fn normalize_registry_description_source(source: &mut PluginRegistryDescriptionSource) {
    source.kind = source.kind.trim().to_ascii_lowercase();
    source.url = source.url.trim().to_string();
    source.repo = source.repo.trim().trim_end_matches(".git").to_string();
    source.branch = source.branch.trim().to_string();
    source.path = source.path.trim().replace('\\', "/");
    if source.kind != "github" && source.kind != "url" {
        source.kind = if !source.url.is_empty() {
            "url".to_string()
        } else if !source.repo.is_empty() || !source.path.is_empty() {
            "github".to_string()
        } else {
            String::new()
        };
    }
}

fn normalize_registry_locale_key(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
    if normalized == "zh" || normalized.starts_with("zh-") {
        "zh".to_string()
    } else if normalized == "en" || normalized.starts_with("en-") {
        "en".to_string()
    } else {
        normalized
    }
}

fn normalize_registry_localized_text(values: &mut BTreeMap<String, String>) {
    let mut normalized = BTreeMap::new();
    for (locale, value) in std::mem::take(values) {
        let locale = normalize_registry_locale_key(&locale);
        let value = value.trim().to_string();
        if locale.is_empty() || value.is_empty() {
            continue;
        }
        normalized.insert(locale, value);
    }
    *values = normalized;
}

fn normalize_registry_localized_description_sources(
    values: &mut BTreeMap<String, PluginRegistryDescriptionSource>,
) {
    let mut normalized = BTreeMap::new();
    for (locale, mut source) in std::mem::take(values) {
        let locale = normalize_registry_locale_key(&locale);
        normalize_registry_description_source(&mut source);
        if locale.is_empty() || source.is_empty() {
            continue;
        }
        normalized.insert(locale, source);
    }
    *values = normalized;
}

fn is_github_repo_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn parse_github_repo(value: &str) -> Result<(String, String), String> {
    let raw = value.trim().trim_end_matches(".git");
    if raw.is_empty() {
        return Err("Plugin registry description source repo is required".to_string());
    }
    let repo_path = if raw.starts_with("http://") || raw.starts_with("https://") {
        let parsed = Url::parse(raw)
            .map_err(|error| format!("Invalid plugin description source repo: {}", error))?;
        if parsed.host_str() != Some("github.com") {
            return Err("Plugin description source repo must use github.com".to_string());
        }
        parsed
            .path()
            .trim_matches('/')
            .trim_end_matches(".git")
            .to_string()
    } else {
        raw.trim_matches('/').to_string()
    };
    let mut parts = repo_path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if parts.next().is_some() || !is_github_repo_part(owner) || !is_github_repo_part(repo) {
        return Err("Plugin description source repo must be owner/repo".to_string());
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn parse_github_repo_for_download(value: &str) -> Result<(String, String), String> {
    let raw = value.trim().trim_end_matches(".git");
    if raw.is_empty() {
        return Err("Plugin download source repo is required".to_string());
    }
    let parts: Vec<String> = if raw.starts_with("http://") || raw.starts_with("https://") {
        let parsed =
            Url::parse(raw).map_err(|error| format!("Invalid plugin download repo: {}", error))?;
        if parsed.host_str() != Some("github.com") {
            return Err("Plugin download source repo must use github.com".to_string());
        }
        parsed
            .path_segments()
            .map(|segments| {
                segments
                    .filter(|part| !part.is_empty())
                    .map(|part| part.trim_end_matches(".git").to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        raw.trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .map(|part| part.trim_end_matches(".git").to_string())
            .collect()
    };
    let owner = parts.first().map(String::as_str).unwrap_or_default();
    let repo = parts.get(1).map(String::as_str).unwrap_or_default();
    if !is_github_repo_part(owner) || !is_github_repo_part(repo) {
        return Err("Plugin download source repo must be owner/repo".to_string());
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn infer_github_download_source(input: &str) -> Option<PluginDownloadSource> {
    let parsed = Url::parse(input.trim()).ok()?;
    if parsed.host_str() != Some("github.com") {
        return None;
    }
    let parts = parsed
        .path_segments()?
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 || !is_github_repo_part(parts[0]) {
        return None;
    }
    let repo_name = parts[1].trim_end_matches(".git");
    if !is_github_repo_part(repo_name) {
        return None;
    }
    let repo = format!("{}/{}", parts[0], repo_name);
    let mut source = PluginDownloadSource {
        input: input.trim().to_string(),
        repo,
        ..PluginDownloadSource::default()
    };
    match parts.get(2).copied() {
        Some("releases") if parts.get(3) == Some(&"latest") => {
            source.kind = "latestRelease".to_string();
        }
        Some("releases") if parts.get(3) == Some(&"tag") && parts.len() > 4 => {
            source.kind = "release".to_string();
            source.tag = parts[4..].join("/");
        }
        Some("tree") if parts.len() > 3 => {
            source.kind = "branch".to_string();
            source.branch = parts[3..].join("/");
        }
        Some("commit") if parts.len() > 3 => {
            source.kind = "commit".to_string();
            source.commit = parts[3].to_string();
        }
        _ => {
            source.kind = "repo".to_string();
        }
    }
    Some(source)
}

fn normalize_github_branch(value: &str) -> Result<String, String> {
    let branch = value.trim().trim_matches('/');
    let branch = if branch.is_empty() {
        DEFAULT_PLUGIN_REGISTRY_DESCRIPTION_BRANCH
    } else {
        branch
    };
    if branch.contains('\\')
        || branch.chars().any(char::is_whitespace)
        || branch.split('/').any(|segment| {
            segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
        })
    {
        return Err(format!(
            "Invalid plugin description source branch: {}",
            value
        ));
    }
    Ok(branch.to_string())
}

fn normalize_registry_markdown_path(value: &str) -> Result<String, String> {
    let raw = value.trim().replace('\\', "/");
    let path = raw.trim_matches('/');
    let path = if path.is_empty() {
        DEFAULT_PLUGIN_REGISTRY_DESCRIPTION_PATH
    } else {
        path
    };
    if path.split('/').any(|segment| {
        segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
    }) {
        return Err(format!("Invalid plugin description source path: {}", value));
    }
    let lower = path.to_ascii_lowercase();
    if !lower.ends_with(".md") && !lower.ends_with(".markdown") {
        return Err("Plugin description source path must be a Markdown file".to_string());
    }
    Ok(path.to_string())
}

fn normalize_registry_markdown_url(value: &str) -> Result<String, String> {
    let raw = value.trim();
    let parsed =
        Url::parse(raw).map_err(|error| format!("Invalid plugin description URL: {}", error))?;
    ensure_secure_plugin_url(&parsed, "Plugin description")?;
    let path = parsed.path().to_ascii_lowercase();
    if !path.ends_with(".md") && !path.ends_with(".markdown") {
        return Err("Plugin description URL must point to a Markdown file".to_string());
    }
    Ok(parsed.to_string())
}

fn github_raw_markdown_url(repo: &str, branch: &str, path: &str) -> Result<String, String> {
    let (owner, repo_name) = parse_github_repo(repo)?;
    let branch = normalize_github_branch(branch)?;
    let path = normalize_registry_markdown_path(path)?;
    let mut url = Url::parse("https://raw.githubusercontent.com/")
        .map_err(|error| format!("Invalid plugin description URL: {}", error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Invalid plugin description URL".to_string())?;
        segments.push(&owner);
        segments.push(&repo_name);
        for segment in branch.split('/') {
            segments.push(segment);
        }
        for segment in path.split('/') {
            segments.push(segment);
        }
    }
    Ok(url.to_string())
}

fn resolve_registry_description_source_url(
    entry_repo: Option<&str>,
    source: Option<&PluginRegistryDescriptionSource>,
) -> Result<Option<String>, String> {
    let Some(source) = source else {
        return Ok(None);
    };
    let mut source = source.clone();
    normalize_registry_description_source(&mut source);
    if source.is_empty() {
        return Ok(None);
    }
    if !source.url.is_empty() {
        return normalize_registry_markdown_url(&source.url).map(Some);
    }
    if source.kind == "url" {
        return Err("Plugin description URL is required".to_string());
    }
    let repo = source
        .repo
        .trim()
        .is_empty()
        .then(|| entry_repo.unwrap_or_default())
        .unwrap_or(source.repo.as_str());
    github_raw_markdown_url(repo, &source.branch, &source.path).map(Some)
}

fn markdown_destination_needs_rewrite(value: &str) -> bool {
    let destination = value.trim();
    if destination.is_empty() || destination.starts_with('#') || destination.starts_with("//") {
        return false;
    }
    if destination.starts_with("data:") || destination.starts_with("blob:") {
        return false;
    }
    Url::parse(destination).is_err()
}

fn rewrite_markdown_relative_urls(markdown: &str, source_url: &str) -> String {
    let Ok(base_url) = Url::parse(source_url) else {
        return markdown.to_string();
    };
    MARKDOWN_LINK_DEST_RE
        .replace_all(markdown, |captures: &Captures<'_>| {
            let Some(url_match) = captures.name("url") else {
                return captures[0].to_string();
            };
            let raw_url = url_match.as_str();
            let (destination, wrapped) =
                if raw_url.starts_with('<') && raw_url.ends_with('>') && raw_url.len() > 2 {
                    (&raw_url[1..raw_url.len() - 1], true)
                } else {
                    (raw_url, false)
                };
            if !markdown_destination_needs_rewrite(destination) {
                return captures[0].to_string();
            }
            let Ok(resolved) = base_url.join(destination) else {
                return captures[0].to_string();
            };
            let replacement = if wrapped {
                format!("<{}>", resolved)
            } else {
                resolved.to_string()
            };
            format!(
                "{}{}{}",
                captures
                    .name("prefix")
                    .map(|value| value.as_str())
                    .unwrap_or(""),
                replacement,
                captures
                    .name("suffix")
                    .map(|value| value.as_str())
                    .unwrap_or(""),
            )
        })
        .into_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{:02x}", byte));
    }
    output
}

fn build_registry_http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(8))
            .timeout(timeout)
            .gzip(true)
            .deflate(true)
            .user_agent(PLUGIN_REGISTRY_USER_AGENT),
    )
}

fn registry_http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    let source = if timeout > Duration::from_secs(30) {
        &REGISTRY_DOWNLOAD_HTTP_CLIENT
    } else {
        &REGISTRY_HTTP_CLIENT
    };
    source
        .as_ref()
        .map(|client| client.clone())
        .map_err(|error| error.clone())
}

fn plugin_registry_cache_root() -> Result<PathBuf, String> {
    let root = super::persistent_config_dir()?
        .join("plugin-registry-cache")
        .join("v1");
    fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create plugin registry cache: {}", error))?;
    Ok(root)
}

fn plugin_registry_cache_path(url: &str, extension: &str) -> Result<PathBuf, String> {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = sha256_hex(&hasher.finalize());
    let extension = extension
        .trim()
        .trim_start_matches('.')
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let extension = if extension.is_empty() {
        "cache".to_string()
    } else {
        extension
    };
    Ok(plugin_registry_cache_root()?.join(format!("{}.{}", hash, extension)))
}

fn plugin_registry_cache_is_fresh(metadata: &fs::Metadata, ttl: Duration) -> bool {
    metadata
        .modified()
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age <= ttl)
        .unwrap_or(false)
}

async fn read_plugin_registry_cache(
    url: &str,
    extension: &str,
    ttl: Option<Duration>,
) -> Option<Vec<u8>> {
    let path = plugin_registry_cache_path(url, extension).ok()?;
    let metadata = tokio::fs::metadata(&path).await.ok()?;
    if let Some(ttl) = ttl {
        if !plugin_registry_cache_is_fresh(&metadata, ttl) {
            return None;
        }
    }
    tokio::fs::read(path).await.ok()
}

async fn write_plugin_registry_cache(
    url: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let path = plugin_registry_cache_path(url, extension)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Plugin registry cache path has no parent".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Failed to create plugin registry cache: {}", error))?;
    let temp_path = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp_path, bytes)
        .await
        .map_err(|error| format!("Failed to write plugin registry cache: {}", error))?;
    let _ = tokio::fs::remove_file(&path).await;
    if let Err(error) = tokio::fs::rename(&temp_path, &path).await {
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(format!("Failed to commit plugin registry cache: {}", error));
    }
    Ok(())
}

async fn fetch_registry_http_bytes_optional(
    url: &str,
    max_bytes: u64,
    error_label: &str,
) -> Result<Option<Vec<u8>>, String> {
    let parsed_url =
        Url::parse(url).map_err(|error| format!("Invalid {} URL: {}", error_label, error))?;
    let client = registry_http_client(Duration::from_secs(12))?;
    let request = client.get(parsed_url.clone());
    let request = with_github_auth_if_available(request, &parsed_url).await;
    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to fetch {}: {}", error_label, error))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(http_error_message(response, error_label).await);
    }
    read_limited_response_bytes(response, max_bytes, error_label)
        .await
        .map(Some)
}

async fn read_limited_response_bytes(
    response: reqwest::Response,
    max_bytes: u64,
    error_label: &str,
) -> Result<Vec<u8>, String> {
    if response
        .content_length()
        .map(|length| length > max_bytes)
        .unwrap_or(false)
    {
        return Err(format!("{} is too large", error_label));
    }
    let mut bytes = Vec::new();
    let mut downloaded = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("Failed to read {}: {}", error_label, error))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > max_bytes {
            return Err(format!("{} is too large", error_label));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn remove_plugin_registry_cache(url: &str, extension: &str) {
    if let Ok(path) = plugin_registry_cache_path(url, extension) {
        let _ = tokio::fs::remove_file(&path).await;
    }
}

async fn fetch_registry_cached_bytes(
    url: &str,
    extension: &str,
    cache_mode: PluginRegistryCacheMode,
    ttl: Duration,
    max_bytes: u64,
    error_label: &str,
) -> Result<Vec<u8>, String> {
    if cache_mode.prefer_any_cache() {
        if let Some(bytes) = read_plugin_registry_cache(url, extension, None).await {
            return Ok(bytes);
        }
    }
    if !cache_mode.skip_fresh_cache() {
        if let Some(bytes) = read_plugin_registry_cache(url, extension, Some(ttl)).await {
            return Ok(bytes);
        }
    }
    match fetch_registry_http_bytes_optional(url, max_bytes, error_label).await {
        Ok(Some(bytes)) => {
            let _ = write_plugin_registry_cache(url, extension, &bytes).await;
            Ok(bytes)
        }
        // A definitive 404 means the resource is gone upstream: drop any stale
        // cache instead of serving it forever.
        Ok(None) => {
            remove_plugin_registry_cache(url, extension).await;
            Err(format!("Failed to fetch {}: HTTP 404 Not Found", error_label))
        }
        Err(error) => {
            if let Some(bytes) = read_plugin_registry_cache(url, extension, None).await {
                Ok(bytes)
            } else {
                Err(error)
            }
        }
    }
}

async fn fetch_registry_cached_bytes_optional(
    url: &str,
    extension: &str,
    cache_mode: PluginRegistryCacheMode,
    ttl: Duration,
    max_bytes: u64,
    error_label: &str,
) -> Result<Option<Vec<u8>>, String> {
    if cache_mode.prefer_any_cache() {
        if let Some(bytes) = read_plugin_registry_cache(url, extension, None).await {
            return Ok(Some(bytes));
        }
    }
    if !cache_mode.skip_fresh_cache() {
        if let Some(bytes) = read_plugin_registry_cache(url, extension, Some(ttl)).await {
            return Ok(Some(bytes));
        }
    }
    match fetch_registry_http_bytes_optional(url, max_bytes, error_label).await {
        Ok(Some(bytes)) => {
            let _ = write_plugin_registry_cache(url, extension, &bytes).await;
            Ok(Some(bytes))
        }
        Ok(None) => {
            remove_plugin_registry_cache(url, extension).await;
            Ok(None)
        }
        Err(error) => {
            if let Some(bytes) = read_plugin_registry_cache(url, extension, None).await {
                Ok(Some(bytes))
            } else {
                Err(error)
            }
        }
    }
}

async fn fetch_registry_json<T>(url: &str, cache_mode: PluginRegistryCacheMode) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let bytes = fetch_registry_cached_bytes(
        url,
        "json",
        cache_mode,
        PLUGIN_REGISTRY_INDEX_CACHE_TTL,
        PLUGIN_REGISTRY_JSON_MAX_BYTES,
        "plugin registry",
    )
    .await?;
    serde_json::from_slice::<T>(&bytes)
        .map_err(|error| format!("Failed to parse plugin registry: {}", error))
}

async fn fetch_registry_json_optional<T>(
    url: &str,
    cache_mode: PluginRegistryCacheMode,
) -> Result<Option<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let Some(bytes) = fetch_registry_cached_bytes_optional(
        url,
        "json",
        cache_mode,
        PLUGIN_REGISTRY_INDEX_CACHE_TTL,
        PLUGIN_REGISTRY_JSON_MAX_BYTES,
        "plugin registry",
    )
    .await?
    else {
        return Ok(None);
    };
    serde_json::from_slice::<T>(&bytes)
        .map(Some)
        .map_err(|error| format!("Failed to parse plugin registry: {}", error))
}

async fn fetch_registry_text(
    url: &str,
    cache_mode: PluginRegistryCacheMode,
) -> Result<String, String> {
    let bytes = fetch_registry_cached_bytes(
        url,
        "md",
        cache_mode,
        PLUGIN_REGISTRY_DESCRIPTION_CACHE_TTL,
        PLUGIN_REGISTRY_DESCRIPTION_MAX_BYTES,
        "plugin registry description",
    )
    .await?;
    String::from_utf8(bytes.to_vec())
        .map_err(|error| format!("Plugin registry description is not UTF-8: {}", error))
}

struct ResolvedPluginInstallSource {
    source_path: PathBuf,
    cleanup_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    zipball_url: Option<String>,
    #[serde(default)]
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PluginGithubAuthConfig {
    #[serde(default)]
    account: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginGithubAuthStatus {
    pub authenticated: bool,
    pub account: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginGithubRepoStarStatus {
    pub repo: String,
    pub starred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stargazers_count: Option<u64>,
}

struct PluginGithubCliLoginSession {
    id: String,
    handle: tokio::task::JoinHandle<Result<PluginGithubAuthStatus, String>>,
    progress: PluginGithubCliLoginProgressHandle,
}

type PluginGithubCliLoginProgressHandle = Arc<Mutex<PluginGithubCliLoginProgress>>;

#[derive(Debug, Default)]
struct PluginGithubCliLoginProgress {
    user_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginGithubOAuthStartResult {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<PluginGithubAuthStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginGithubOAuthPollResult {
    pub status: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<PluginGithubAuthStatus>,
}

#[derive(Debug, Deserialize)]
struct GithubUser {
    login: String,
}

fn plugin_download_temp_root() -> Result<PathBuf, String> {
    let root = super::app_temp_dir()?.join("plugin-source-downloads");
    fs::create_dir_all(&root)
        .map_err(|error| format!("Failed to create plugin download directory: {}", error))?;
    Ok(root)
}

fn cleanup_resolved_plugin_source(source: &ResolvedPluginInstallSource) {
    if let Some(path) = &source.cleanup_path {
        if path.is_dir() {
            let _ = fs::remove_dir_all(path);
        } else if path.is_file() {
            let _ = fs::remove_file(path);
        }
    }
}

fn plugin_github_auth_config_path() -> Result<PathBuf, String> {
    Ok(super::persistent_config_dir()?.join(PLUGIN_GITHUB_AUTH_CONFIG_FILE))
}

fn normalize_plugin_github_token(value: &str) -> Result<String, String> {
    let token = value.trim();
    if token.is_empty() {
        return Err("GitHub token is required".to_string());
    }
    if token.chars().any(char::is_whitespace) {
        return Err("GitHub token cannot contain whitespace".to_string());
    }
    Ok(token.to_string())
}

async fn read_plugin_github_auth_config() -> PluginGithubAuthConfig {
    let Ok(path) = plugin_github_auth_config_path() else {
        return PluginGithubAuthConfig::default();
    };
    let Ok(bytes) = tokio::fs::read(path).await else {
        return PluginGithubAuthConfig::default();
    };
    serde_json::from_slice::<PluginGithubAuthConfig>(&bytes).unwrap_or_default()
}

async fn write_plugin_github_auth_config(config: &PluginGithubAuthConfig) -> Result<(), String> {
    let path = plugin_github_auth_config_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| "GitHub auth config path has no parent".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("Failed to create GitHub auth config directory: {}", error))?;
    let json = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("Failed to serialize GitHub auth config: {}", error))?;
    tokio::fs::write(path, json)
        .await
        .map_err(|error| format!("Failed to save GitHub auth config: {}", error))
}

async fn clear_plugin_github_auth_config() -> Result<(), String> {
    let path = plugin_github_auth_config_path()?;
    let _ = crate::keychain::delete_secret(crate::keychain::KEY_PLUGIN_GITHUB_TOKEN);
    match tokio::fs::remove_file(path).await {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to clear GitHub auth config: {}", error)),
    }
}

async fn plugin_github_auth_token() -> Option<String> {
    crate::keychain::get_secret(crate::keychain::KEY_PLUGIN_GITHUB_TOKEN)
        .ok()
        .flatten()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

async fn plugin_github_required_token() -> Result<String, String> {
    plugin_github_auth_token()
        .await
        .ok_or_else(|| "GitHub login is required".to_string())
}

fn plugin_github_auth_status_from_config(config: PluginGithubAuthConfig) -> PluginGithubAuthStatus {
    let authenticated = crate::keychain::get_secret(crate::keychain::KEY_PLUGIN_GITHUB_TOKEN)
        .ok()
        .flatten()
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    PluginGithubAuthStatus {
        authenticated,
        account: config.account.trim().to_string(),
    }
}

async fn save_plugin_github_token_auth(token: &str) -> Result<PluginGithubAuthStatus, String> {
    let token = normalize_plugin_github_token(token)?;
    let user = fetch_github_user_with_token(&token).await?;
    crate::keychain::set_secret(crate::keychain::KEY_PLUGIN_GITHUB_TOKEN, &token)?;
    let config = PluginGithubAuthConfig {
        account: user.login.trim().to_string(),
    };
    write_plugin_github_auth_config(&config).await?;
    Ok(plugin_github_auth_status_from_config(config))
}

fn github_cli_unavailable_message() -> String {
    "GitHub CLI is unavailable. Bundle it with `bun run github-cli:bundle` for development builds."
        .to_string()
}

fn apply_github_cli_env(command: &mut tokio::process::Command) {
    command
        .env("GH_TELEMETRY", "false")
        .env("DO_NOT_TRACK", "true")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1");
}

async fn run_github_cli_capture(args: &[&str], label: &str) -> Result<String, String> {
    let gh = resolve_github_cli().ok_or_else(github_cli_unavailable_message)?;
    let program = gh.path.to_string_lossy().to_string();
    let mut command = async_command(&program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_github_cli_env(&mut command);

    let output = command
        .output()
        .await
        .map_err(|error| format!("Failed to run {}: {}", label, error))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        if detail.is_empty() {
            return Err(format!("{} failed", label));
        }
        return Err(format!("{} failed: {}", label, detail));
    }
    Ok(stdout)
}

async fn import_github_cli_auth() -> Result<PluginGithubAuthStatus, String> {
    let token = run_github_cli_capture(
        &["auth", "token", "--hostname", PLUGIN_GITHUB_CLI_HOSTNAME],
        "GitHub CLI token",
    )
    .await?;
    save_plugin_github_token_auth(&token).await
}

fn extract_github_cli_user_code(text: &str) -> Option<String> {
    GITHUB_CLI_DEVICE_CODE_RE
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|code| code.as_str().to_ascii_uppercase())
}

fn update_github_cli_login_user_code(progress: &PluginGithubCliLoginProgressHandle, text: &str) {
    let Some(user_code) = extract_github_cli_user_code(text) else {
        return;
    };

    let mut guard = progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.user_code.is_none() {
        guard.user_code = Some(user_code);
    }
}

fn github_cli_login_user_code(progress: &PluginGithubCliLoginProgressHandle) -> String {
    progress
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .user_code
        .clone()
        .unwrap_or_default()
}

async fn wait_for_github_cli_user_code(progress: PluginGithubCliLoginProgressHandle) -> String {
    let _ = timeout(PLUGIN_GITHUB_CLI_LOGIN_CODE_WAIT_TIMEOUT, async {
        loop {
            if !github_cli_login_user_code(&progress).is_empty() {
                break;
            }
            sleep(Duration::from_millis(80)).await;
        }
    })
    .await;

    github_cli_login_user_code(&progress)
}

async fn read_github_cli_login_pipe<R>(
    reader: R,
    progress: PluginGithubCliLoginProgressHandle,
) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    let mut output = String::new();

    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                update_github_cli_login_user_code(&progress, &line);
                output.push_str(&line);
                output.push('\n');
            }
            Ok(None) => break,
            Err(error) => {
                if !output.is_empty() {
                    output.push('\n');
                }
                output.push_str(&format!("Failed to read GitHub CLI login output: {error}"));
                break;
            }
        }
    }

    output.trim().to_string()
}

async fn run_github_cli_login(progress: PluginGithubCliLoginProgressHandle) -> Result<(), String> {
    let gh = resolve_github_cli().ok_or_else(github_cli_unavailable_message)?;
    let program = gh.path.to_string_lossy().to_string();
    let mut command = async_command(&program);
    command
        .args([
            "auth",
            "login",
            "--web",
            "--clipboard",
            "--hostname",
            PLUGIN_GITHUB_CLI_HOSTNAME,
            "--git-protocol",
            "https",
            "--scopes",
            PLUGIN_GITHUB_CLI_SCOPES,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_github_cli_env(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Failed to run GitHub CLI login: {error}"))?;

    let stdout_task = child
        .stdout
        .take()
        .map(|stdout| tokio::spawn(read_github_cli_login_pipe(stdout, progress.clone())));
    let stderr_task = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(read_github_cli_login_pipe(stderr, progress)));

    let status = child
        .wait()
        .await
        .map_err(|error| format!("Failed to wait for GitHub CLI login: {error}"))?;

    let stdout = match stdout_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    let stderr = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };

    if !status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        if detail.is_empty() {
            return Err("GitHub CLI login failed".to_string());
        }
        return Err(format!("GitHub CLI login failed: {detail}"));
    }

    Ok(())
}

fn github_cli_oauth_start_result(
    device_code: String,
    user_code: String,
    message: Option<String>,
    auth: Option<PluginGithubAuthStatus>,
) -> PluginGithubOAuthStartResult {
    PluginGithubOAuthStartResult {
        user_code,
        verification_uri: PLUGIN_GITHUB_CLI_VERIFICATION_URI.to_string(),
        device_code,
        interval: PLUGIN_GITHUB_CLI_LOGIN_INTERVAL,
        expires_in: 0,
        message,
        auth,
    }
}

fn github_cli_oauth_poll_result(
    status: &str,
    user_code: String,
    message: Option<String>,
    auth: Option<PluginGithubAuthStatus>,
) -> PluginGithubOAuthPollResult {
    PluginGithubOAuthPollResult {
        status: status.to_string(),
        user_code,
        verification_uri: PLUGIN_GITHUB_CLI_VERIFICATION_URI.to_string(),
        message,
        auth,
    }
}

fn github_cli_login_session_user_code(device_code: &str) -> Option<String> {
    let guard = PLUGIN_GITHUB_CLI_LOGIN_SESSION
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.as_ref().and_then(|session| {
        (session.id == device_code).then(|| github_cli_login_user_code(&session.progress))
    })
}

async fn collect_finished_github_cli_login_session(
    device_code: Option<&str>,
) -> Option<Result<PluginGithubAuthStatus, String>> {
    let session = {
        let mut guard = PLUGIN_GITHUB_CLI_LOGIN_SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(session) = guard.as_ref() else {
            return None;
        };
        if device_code
            .map(|expected| session.id != expected)
            .unwrap_or(false)
        {
            return None;
        }
        if !session.handle.is_finished() {
            return None;
        }
        guard.take()
    };

    match session.expect("checked session").handle.await {
        Ok(result) => Some(result),
        Err(error) => Some(Err(format!("GitHub CLI login task failed: {}", error))),
    }
}

fn is_github_request_url(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("api.github.com")
            | Some("github.com")
            | Some("raw.githubusercontent.com")
            | Some("codeload.github.com")
    )
}

async fn with_github_auth_if_available(
    request: reqwest::RequestBuilder,
    url: &Url,
) -> reqwest::RequestBuilder {
    if !is_github_request_url(url) {
        return request;
    }
    if let Some(token) = plugin_github_auth_token().await {
        request.bearer_auth(token)
    } else {
        request
    }
}

async fn http_error_message(response: reqwest::Response, error_label: &str) -> String {
    let status = response.status();
    let headers = response.headers().clone();
    let rate_remaining = headers
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let rate_reset = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response.text().await.unwrap_or_default();
    let detail = body.trim().chars().take(400).collect::<String>();
    let mut message = format!("Failed to fetch {}: HTTP {}", error_label, status.as_u16());
    if !detail.is_empty() {
        message.push_str(": ");
        message.push_str(&detail);
    }
    if let Some(rate_remaining) = rate_remaining {
        message.push_str(&format!(" (GitHub rate remaining: {}", rate_remaining));
        if let Some(rate_reset) = rate_reset {
            message.push_str(&format!(", reset: {}", rate_reset));
        }
        message.push(')');
    }
    message
}

async fn download_plugin_archive_to_temp(
    url: &str,
    id_hint: &str,
    expected_hash: Option<&str>,
    size_bytes: Option<u64>,
) -> Result<ResolvedPluginInstallSource, String> {
    let parsed_url = Url::parse(url.trim())
        .map_err(|error| format!("Invalid plugin download URL: {}", error))?;
    ensure_secure_plugin_url(&parsed_url, "Plugin download")?;
    let expected_hash = match expected_hash {
        Some(hash) if !hash.trim().is_empty() => Some(normalize_sha256(hash)?),
        _ => None,
    };
    if let Some(size_bytes) = size_bytes {
        if size_bytes > PLUGIN_REGISTRY_DOWNLOAD_MAX_BYTES {
            return Err("Plugin download is too large".to_string());
        }
    }

    let client = registry_http_client(Duration::from_secs(120))?;
    let request = client.get(parsed_url.clone());
    let request = with_github_auth_if_available(request, &parsed_url).await;
    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to download plugin: {}", error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(http_error_message(response, "plugin download").await);
    }
    if response
        .content_length()
        .map(|length| length > PLUGIN_REGISTRY_DOWNLOAD_MAX_BYTES)
        .unwrap_or(false)
    {
        return Err("Plugin download is too large".to_string());
    }

    let safe_hint = normalize_plugin_id(id_hint).unwrap_or_else(|_| "plugin".to_string());
    let file_path =
        plugin_download_temp_root()?.join(format!("{}-{}.zip", safe_hint, uuid::Uuid::new_v4()));
    let mut file = tokio::fs::File::create(&file_path)
        .await
        .map_err(|error| format!("Failed to create plugin download: {}", error))?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = fs::remove_file(&file_path);
                return Err(format!("Failed to read plugin download: {}", error));
            }
        };
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > PLUGIN_REGISTRY_DOWNLOAD_MAX_BYTES {
            let _ = fs::remove_file(&file_path);
            return Err("Plugin download is too large".to_string());
        }
        hasher.update(&chunk);
        if let Err(error) = file.write_all(&chunk).await {
            let _ = fs::remove_file(&file_path);
            return Err(format!("Failed to write plugin download: {}", error));
        }
    }

    file.flush()
        .await
        .map_err(|error| format!("Failed to finish plugin download: {}", error))?;
    if let Some(size_bytes) = size_bytes {
        if size_bytes != downloaded {
            let _ = fs::remove_file(&file_path);
            return Err(format!(
                "Plugin download size mismatch: expected {}, got {}",
                size_bytes, downloaded
            ));
        }
    }

    if let Some(expected_hash) = expected_hash {
        let actual_hash = sha256_hex(&hasher.finalize());
        if actual_hash != expected_hash {
            let _ = fs::remove_file(&file_path);
            return Err("Plugin download sha256 mismatch".to_string());
        }
    }

    Ok(ResolvedPluginInstallSource {
        source_path: file_path.clone(),
        cleanup_path: Some(file_path),
    })
}

fn github_archive_zip_url(owner: &str, repo: &str, reference: &str) -> Result<String, String> {
    let reference = normalize_git_ref(reference, "ref")?;
    let mut url = Url::parse("https://codeload.github.com/")
        .map_err(|error| format!("Invalid GitHub archive URL: {}", error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Invalid GitHub archive URL".to_string())?;
        segments.push(owner);
        segments.push(repo);
        segments.push("zip");
        for segment in reference.split('/') {
            segments.push(segment);
        }
    }
    Ok(url.to_string())
}

fn github_release_api_url(owner: &str, repo: &str, tag: Option<&str>) -> Result<String, String> {
    let mut url = Url::parse("https://api.github.com/")
        .map_err(|error| format!("Invalid GitHub release URL: {}", error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Invalid GitHub release URL".to_string())?;
        segments.push("repos");
        segments.push(owner);
        segments.push(repo);
        segments.push("releases");
        if let Some(tag) = tag {
            segments.push("tags");
            for segment in normalize_git_ref(tag, "release tag")?.split('/') {
                segments.push(segment);
            }
        } else {
            segments.push("latest");
        }
    }
    Ok(url.to_string())
}

fn normalize_plugin_github_repo_for_api(value: &str) -> Result<(String, String, String), String> {
    let (owner, repo) = parse_github_repo_for_download(value)
        .map_err(|_| "Plugin GitHub repo must be owner/repo or a github.com URL".to_string())?;
    let normalized = format!("{}/{}", owner, repo);
    Ok((owner, repo, normalized))
}

fn github_user_star_api_url(owner: &str, repo: &str) -> Result<String, String> {
    let mut url = Url::parse("https://api.github.com/")
        .map_err(|error| format!("Invalid GitHub star URL: {}", error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Invalid GitHub star URL".to_string())?;
        segments.push("user");
        segments.push("starred");
        segments.push(owner);
        segments.push(repo);
    }
    Ok(url.to_string())
}

fn github_release_asset_download_url(
    owner: &str,
    repo: &str,
    tag: Option<&str>,
    asset: &str,
) -> Result<String, String> {
    let asset = asset.trim();
    if asset.is_empty() {
        return Err("Plugin release asset is required".to_string());
    }
    let mut url = Url::parse("https://github.com/")
        .map_err(|error| format!("Invalid GitHub release asset URL: {}", error))?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "Invalid GitHub release asset URL".to_string())?;
        segments.push(owner);
        segments.push(repo);
        segments.push("releases");
        if let Some(tag) = tag {
            segments.push("download");
            for segment in normalize_git_ref(tag, "release tag")?.split('/') {
                segments.push(segment);
            }
        } else {
            segments.push("latest");
            segments.push("download");
        }
        segments.push(asset);
    }
    Ok(url.to_string())
}

fn release_asset_pattern_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let mut star_index: Option<usize> = None;
    let mut star_value_index = 0usize;
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

async fn fetch_github_api_bytes(
    api_url: &str,
    max_bytes: u64,
    error_label: &str,
) -> Result<Vec<u8>, String> {
    let parsed_url =
        Url::parse(api_url).map_err(|error| format!("Invalid GitHub API URL: {}", error))?;
    let client = registry_http_client(Duration::from_secs(12))?;
    let request = client
        .get(parsed_url.clone())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    let request = with_github_auth_if_available(request, &parsed_url).await;
    let response = request
        .send()
        .await
        .map_err(|error| format!("Failed to fetch {}: {}", error_label, error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(http_error_message(response, error_label).await);
    }
    if response
        .content_length()
        .map(|length| length > max_bytes)
        .unwrap_or(false)
    {
        return Err(format!("{} is too large", error_label));
    }
    read_limited_response_bytes(response, max_bytes, error_label).await
}

async fn fetch_github_user_with_token(token: &str) -> Result<GithubUser, String> {
    let client = registry_http_client(Duration::from_secs(12))?;
    let url = Url::parse("https://api.github.com/user")
        .map_err(|error| format!("Invalid GitHub user URL: {}", error))?;
    let response = client
        .get(url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| format!("Failed to verify GitHub token: {}", error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(http_error_message(response, "GitHub user").await);
    }
    response
        .json::<GithubUser>()
        .await
        .map_err(|error| format!("Failed to parse GitHub user: {}", error))
}

async fn send_github_repo_star_request(
    method: reqwest::Method,
    api_url: &str,
    token: &str,
    error_label: &str,
) -> Result<reqwest::Response, String> {
    let parsed_url =
        Url::parse(api_url).map_err(|error| format!("Invalid GitHub API URL: {}", error))?;
    let client = registry_http_client(Duration::from_secs(12))?;
    client
        .request(method, parsed_url)
        .bearer_auth(token)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|error| format!("Failed to fetch {}: {}", error_label, error))
}

async fn fetch_github_repo_star_status_with_token(
    repo: &str,
    token: &str,
) -> Result<PluginGithubRepoStarStatus, String> {
    let (owner, repo_name, normalized_repo) = normalize_plugin_github_repo_for_api(repo)?;
    let api_url = github_user_star_api_url(&owner, &repo_name)?;
    let response =
        send_github_repo_star_request(reqwest::Method::GET, &api_url, token, "GitHub star status")
            .await?;
    let status = response.status();
    let starred = if status == reqwest::StatusCode::NOT_FOUND {
        false
    } else if status.is_success() {
        true
    } else {
        return Err(http_error_message(response, "GitHub star status").await);
    };
    Ok(PluginGithubRepoStarStatus {
        repo: normalized_repo,
        starred,
        stargazers_count: None,
    })
}

async fn set_github_repo_star_status_with_token(
    repo: &str,
    starred: bool,
    token: &str,
) -> Result<PluginGithubRepoStarStatus, String> {
    let (owner, repo_name, normalized_repo) = normalize_plugin_github_repo_for_api(repo)?;
    let api_url = github_user_star_api_url(&owner, &repo_name)?;
    let method = if starred {
        reqwest::Method::PUT
    } else {
        reqwest::Method::DELETE
    };
    let response =
        send_github_repo_star_request(method, &api_url, token, "GitHub star update").await?;
    if !response.status().is_success() {
        return Err(http_error_message(response, "GitHub star update").await);
    }
    Ok(PluginGithubRepoStarStatus {
        repo: normalized_repo,
        starred,
        stargazers_count: None,
    })
}

async fn fetch_github_release(
    owner: &str,
    repo: &str,
    tag: Option<&str>,
) -> Result<GithubRelease, String> {
    let api_url = github_release_api_url(owner, repo, tag)?;
    let bytes = fetch_github_api_bytes(
        &api_url,
        PLUGIN_REGISTRY_JSON_MAX_BYTES,
        "plugin release metadata",
    )
    .await?;
    serde_json::from_slice::<GithubRelease>(&bytes)
        .map_err(|error| format!("Failed to parse plugin release metadata: {}", error))
}

fn select_github_release_download(
    release: &GithubRelease,
    asset_name: &str,
    asset_pattern: &str,
) -> Result<(String, Option<u64>), String> {
    let asset_name = asset_name.trim();
    if !asset_name.is_empty() {
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == asset_name)
            .ok_or_else(|| format!("Plugin release asset not found: {}", asset_name))?;
        return Ok((asset.browser_download_url.clone(), asset.size));
    }
    let asset_pattern = asset_pattern.trim();
    if !asset_pattern.is_empty() {
        let asset = release
            .assets
            .iter()
            .find(|asset| release_asset_pattern_matches(asset_pattern, &asset.name))
            .ok_or_else(|| format!("Plugin release asset not found: {}", asset_pattern))?;
        return Ok((asset.browser_download_url.clone(), asset.size));
    }
    if let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name.to_ascii_lowercase().ends_with(".zip"))
        .or_else(|| release.assets.first())
    {
        return Ok((asset.browser_download_url.clone(), asset.size));
    }
    release
        .zipball_url
        .clone()
        .map(|url| (url, None))
        .ok_or_else(|| "Plugin release has no downloadable asset".to_string())
}

async fn resolve_github_release_source(
    source: &PluginDownloadSource,
    tag: Option<&str>,
    fallback_repo: Option<&str>,
    id_hint: &str,
) -> Result<ResolvedPluginInstallSource, String> {
    let repo = source_repo_value(source, fallback_repo);
    let (owner, repo_name) = parse_github_repo_for_download(&repo)?;
    if !source.asset.trim().is_empty() {
        let direct_url = github_release_asset_download_url(&owner, &repo_name, tag, &source.asset)?;
        let expected_hash = normalize_optional_sha256(&source.sha256)?;
        match download_plugin_archive_to_temp(
            &direct_url,
            id_hint,
            expected_hash.as_deref(),
            source.size_bytes,
        )
        .await
        {
            Ok(source) => return Ok(source),
            Err(direct_error) => {
                let release = fetch_github_release(&owner, &repo_name, tag)
                    .await
                    .map_err(|api_error| format!("{}; {}", direct_error, api_error))?;
                let (url, release_size) =
                    select_github_release_download(&release, &source.asset, &source.asset_pattern)?;
                return download_plugin_archive_to_temp(
                    &url,
                    id_hint,
                    expected_hash.as_deref(),
                    source.size_bytes.or(release_size),
                )
                .await;
            }
        }
    }
    let release = fetch_github_release(&owner, &repo_name, tag).await?;
    let (url, release_size) =
        select_github_release_download(&release, &source.asset, &source.asset_pattern)?;
    let expected_hash = normalize_optional_sha256(&source.sha256)?;
    download_plugin_archive_to_temp(
        &url,
        id_hint,
        expected_hash.as_deref(),
        source.size_bytes.or(release_size),
    )
    .await
}

async fn run_plugin_git(args: &[String], cwd: Option<&Path>, label: &str) -> Result<(), String> {
    let mut command = async_command("git");
    let command_args = plugin_git_command_args(args);
    command
        .args(&command_args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.kill_on_drop(true);
    if let Some(token) = plugin_github_auth_token().await {
        apply_plugin_git_auth_env(&mut command, &token);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = timeout(PLUGIN_GIT_COMMAND_TIMEOUT, command.output())
        .await
        .map_err(|_| {
            format!(
                "{} timed out after {} seconds",
                label,
                PLUGIN_GIT_COMMAND_TIMEOUT.as_secs()
            )
        })?
        .map_err(|error| format!("{} failed: {}", label, error))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("{} failed", label))
    } else {
        Err(format!("{} failed: {}", label, stderr))
    }
}

fn plugin_git_command_args(args: &[String]) -> Vec<String> {
    let mut command_args =
        Vec::with_capacity(args.len() + if cfg!(target_os = "windows") { 2 } else { 0 });
    #[cfg(target_os = "windows")]
    {
        command_args.push("-c".to_string());
        command_args.push("http.sslBackend=schannel".to_string());
    }
    command_args.extend(args.iter().cloned());
    command_args
}

fn plugin_github_git_auth_header(token: &str) -> String {
    let credential = format!("x-access-token:{}", token.trim());
    format!(
        "AUTHORIZATION: basic {}",
        base64::engine::general_purpose::STANDARD.encode(credential)
    )
}

fn apply_plugin_git_auth_env(command: &mut tokio::process::Command, token: &str) {
    command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
        .env("GIT_CONFIG_VALUE_0", plugin_github_git_auth_header(token));
}

fn normalize_git_repo_source(value: &str) -> Result<String, String> {
    let repo = value.trim();
    if repo.is_empty() {
        return Err("Plugin git repository is required".to_string());
    }
    if let Ok((owner, repo_name)) = parse_github_repo_for_download(repo) {
        if !repo.starts_with("http://") && !repo.starts_with("https://") {
            return Ok(format!("https://github.com/{}/{}.git", owner, repo_name));
        }
    }
    if repo.starts_with("http://") || repo.starts_with("https://") || repo.starts_with("ssh://") {
        let parsed = Url::parse(repo)
            .map_err(|error| format!("Invalid plugin git repository: {}", error))?;
        if matches!(parsed.scheme(), "http" | "https") {
            ensure_secure_plugin_url(&parsed, "Plugin git repository")?;
        }
        return Ok(repo.to_string());
    }
    if repo.contains('@') && repo.contains(':') {
        return Ok(repo.to_string());
    }
    Err(format!("Invalid plugin git repository: {}", value))
}

async fn clone_plugin_git_source(
    source: &PluginDownloadSource,
    kind: &str,
    fallback_repo: Option<&str>,
) -> Result<ResolvedPluginInstallSource, String> {
    let repo_value = {
        let configured = source_repo_value(source, fallback_repo);
        if configured.trim().is_empty() {
            source_primary_input(source)
        } else {
            configured
        }
    };
    let repo = normalize_git_repo_source(&repo_value)?;
    let temp_root = plugin_download_temp_root()?.join(format!("git-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_root)
        .map_err(|error| format!("Failed to create plugin clone directory: {}", error))?;
    let target = temp_root.join("repo");
    let target_string = target.display().to_string();
    let clone_result = async {
        match kind {
            "branch" | "tag" => {
                let label = if kind == "branch" { "branch" } else { "tag" };
                let reference = normalize_git_ref(&source_ref_value(source, kind, None), label)?;
                run_plugin_git(
                    &[
                        "clone".to_string(),
                        "--depth".to_string(),
                        "1".to_string(),
                        "--branch".to_string(),
                        reference,
                        "--single-branch".to_string(),
                        repo.clone(),
                        target_string.clone(),
                    ],
                    None,
                    "git clone",
                )
                .await
            }
            "commit" => {
                let commit = normalize_commit_ref(&source_ref_value(source, kind, None))?;
                run_plugin_git(
                    &[
                        "clone".to_string(),
                        "--filter=blob:none".to_string(),
                        "--no-checkout".to_string(),
                        repo.clone(),
                        target_string.clone(),
                    ],
                    None,
                    "git clone",
                )
                .await?;
                run_plugin_git(
                    &[
                        "fetch".to_string(),
                        "--depth".to_string(),
                        "1".to_string(),
                        "origin".to_string(),
                        commit.clone(),
                    ],
                    Some(&target),
                    "git fetch",
                )
                .await?;
                run_plugin_git(
                    &["checkout".to_string(), "--detach".to_string(), commit],
                    Some(&target),
                    "git checkout",
                )
                .await
            }
            _ => {
                let mut args = vec!["clone".to_string(), "--depth".to_string(), "1".to_string()];
                let branch = source.branch.trim();
                if !branch.is_empty() {
                    args.extend([
                        "--branch".to_string(),
                        normalize_git_ref(branch, "branch")?,
                        "--single-branch".to_string(),
                    ]);
                }
                args.extend([repo.clone(), target_string.clone()]);
                run_plugin_git(&args, None, "git clone").await
            }
        }
    }
    .await;
    if let Err(error) = clone_result {
        let _ = fs::remove_dir_all(&temp_root);
        return Err(error);
    }
    let git_dir = target.join(".git");
    if git_dir.exists() {
        let _ = fs::remove_dir_all(git_dir);
    }
    Ok(ResolvedPluginInstallSource {
        source_path: target,
        cleanup_path: Some(temp_root),
    })
}

fn is_direct_archive_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .map(|url| {
            matches!(url.scheme(), "https" | "http")
                && url.path().to_ascii_lowercase().ends_with(".zip")
        })
        .unwrap_or(false)
}

async fn resolve_plugin_download_source(
    source: &PluginDownloadSource,
    fallback_repo: Option<&str>,
    id_hint: &str,
) -> Result<ResolvedPluginInstallSource, String> {
    let input = source_primary_input(source);
    let kind = normalize_download_source_kind(&source.kind);
    if kind == "local" || ((kind.is_empty() || kind == "auto") && Path::new(&input).exists()) {
        if input.is_empty() {
            return Err("Plugin source path is required".to_string());
        }
        return Ok(ResolvedPluginInstallSource {
            source_path: PathBuf::from(input),
            cleanup_path: None,
        });
    }

    if (kind.is_empty() || kind == "auto") && is_direct_archive_url(&input) {
        let expected_hash = normalize_optional_sha256(&source.sha256)?;
        return download_plugin_archive_to_temp(
            &input,
            id_hint,
            expected_hash.as_deref(),
            source.size_bytes,
        )
        .await;
    }

    if kind == "url" || kind == "archive" || kind == "zip" {
        let url = if !source.url.trim().is_empty() {
            source.url.trim()
        } else {
            input.as_str()
        };
        let expected_hash = normalize_optional_sha256(&source.sha256)?;
        return download_plugin_archive_to_temp(
            url,
            id_hint,
            expected_hash.as_deref(),
            source.size_bytes,
        )
        .await;
    }

    if (kind.is_empty() || kind == "auto") && !input.is_empty() {
        if let Some(inferred) = infer_github_download_source(&input) {
            return Box::pin(resolve_plugin_download_source(
                &inferred,
                fallback_repo,
                id_hint,
            ))
            .await;
        }
    }

    match kind.as_str() {
        "latestrelease" | "githublatestrelease" => {
            resolve_github_release_source(source, None, fallback_repo, id_hint).await
        }
        "release" | "releasetag" | "githubrelease" => {
            let tag = source_ref_value(source, "release", None);
            let tag = if tag.is_empty() { None } else { Some(tag) };
            resolve_github_release_source(source, tag.as_deref(), fallback_repo, id_hint).await
        }
        "branch" | "tag" | "commit" => {
            let repo = source_repo_value(source, fallback_repo);
            if let Ok((owner, repo_name)) = parse_github_repo_for_download(&repo) {
                let label = match kind.as_str() {
                    "branch" => "branch",
                    "tag" => "tag",
                    _ => "commit",
                };
                let reference = if kind == "commit" {
                    normalize_commit_ref(&source_ref_value(source, "commit", None))?
                } else {
                    normalize_git_ref(&source_ref_value(source, &kind, None), label)?
                };
                let url = github_archive_zip_url(&owner, &repo_name, &reference)?;
                let expected_hash = normalize_optional_sha256(&source.sha256)?;
                download_plugin_archive_to_temp(
                    &url,
                    id_hint,
                    expected_hash.as_deref(),
                    source.size_bytes,
                )
                .await
            } else {
                clone_plugin_git_source(source, &kind, fallback_repo).await
            }
        }
        "repo" | "git" | "" | "auto" => {
            clone_plugin_git_source(source, "repo", fallback_repo).await
        }
        _ => Err(format!(
            "Unsupported plugin download source: {}",
            source.kind
        )),
    }
}

async fn resolve_registry_install_source(
    request: &PluginRegistryInstallRequest,
    fallback_repo: Option<&str>,
) -> Result<ResolvedPluginInstallSource, String> {
    let plugin_id = normalize_plugin_id(&request.id)?;
    if registry_download_is_resolved(&request.download) {
        let expected_hash = normalize_sha256(&request.download.sha256)?;
        match download_plugin_archive_to_temp(
            &request.download.url,
            &plugin_id,
            Some(&expected_hash),
            request.download.size_bytes,
        )
        .await
        {
            Ok(source) => return Ok(source),
            Err(download_error) => {
                if plugin_download_error_blocks_registry_fallback(&download_error) {
                    return Err(download_error);
                }
                if request.download_source.is_empty() {
                    return Err(download_error);
                }
                return resolve_plugin_download_source(
                    &request.download_source,
                    fallback_repo,
                    &plugin_id,
                )
                .await
                .map_err(|source_error| {
                    format!(
                        "{}; fallback download source failed: {}",
                        download_error, source_error
                    )
                });
            }
        }
    }
    if !request.download_source.is_empty() {
        match resolve_plugin_download_source(&request.download_source, fallback_repo, &plugin_id)
            .await
        {
            Ok(source) => return Ok(source),
            Err(source_error) => {
                if request.download.url.trim().is_empty() {
                    return Err(source_error);
                }
                let expected_hash = normalize_sha256(&request.download.sha256)?;
                return download_plugin_archive_to_temp(
                    &request.download.url,
                    &plugin_id,
                    Some(&expected_hash),
                    request.download.size_bytes,
                )
                .await
                .map_err(|download_error| {
                    format!(
                        "{}; fallback registry download failed: {}",
                        source_error, download_error
                    )
                });
            }
        }
    }
    if request.download.url.trim().is_empty() {
        return Err("Plugin registry entry has no download source".to_string());
    }
    let expected_hash = normalize_sha256(&request.download.sha256)?;
    download_plugin_archive_to_temp(
        &request.download.url,
        &plugin_id,
        Some(&expected_hash),
        request.download.size_bytes,
    )
    .await
}

fn registry_download_is_resolved(download: &PluginRegistryDownload) -> bool {
    !download.url.trim().is_empty() && !download.sha256.trim().is_empty()
}

fn plugin_download_error_blocks_registry_fallback(error: &str) -> bool {
    error.contains("sha256 mismatch")
        || error.contains("size mismatch")
        || error.contains("too large")
}

fn expected_registry_install_version(request: &PluginRegistryInstallRequest) -> String {
    let source_version = request.download_source.version.trim();
    if !source_version.is_empty() {
        return source_version.to_string();
    }
    request.latest_version.trim().to_string()
}

pub(crate) fn emit_plugins_changed(app_handle: &AppHandle, working_dir: &str, source: &str) {
    if let Err(error) = app_handle.emit(PLUGINS_CHANGED_EVENT, ()) {
        eprintln!("[Locus] failed to emit plugins changed event: {}", error);
    }
    crate::view::emit_view_tree_changed(app_handle);
    super::knowledge::emit_knowledge_changed(app_handle, working_dir, source);
}

pub(crate) async fn install_plugin_from_registry_request(
    working_dir: &str,
    request: PluginRegistryInstallRequest,
    scope: PluginInstallScope,
) -> Result<InstalledPluginSummary, String> {
    let plugin_id = normalize_plugin_id(&request.id)?;
    let expected_version = expected_registry_install_version(&request);
    let resolved_source = resolve_registry_install_source(&request, None).await?;
    let source_path_string = resolved_source.source_path.display().to_string();
    let install_result = (|| -> Result<InstalledPluginSummary, String> {
        let manifest = inspect_plugin_source_manifest_sync(&source_path_string)?;
        if manifest.id != plugin_id {
            return Err(format!(
                "Plugin registry id mismatch: expected {}, got {}",
                plugin_id, manifest.id
            ));
        }
        if !expected_version.is_empty() && manifest.version != expected_version {
            return Err(format!(
                "Plugin registry version mismatch: expected {}, got {}",
                expected_version, manifest.version
            ));
        }
        install_plugin_from_path_sync(working_dir, &source_path_string, scope)
    })();
    cleanup_resolved_plugin_source(&resolved_source);
    install_result
}

pub(crate) async fn install_plugin_from_download_source(
    working_dir: &str,
    source: PluginDownloadSource,
    scope: PluginInstallScope,
) -> Result<InstalledPluginSummary, String> {
    let resolved_source = resolve_plugin_download_source(&source, None, "plugin").await?;
    let source_path_string = resolved_source.source_path.display().to_string();
    let install_result = (|| -> Result<InstalledPluginSummary, String> {
        validate_download_source_manifest(&source, &source_path_string)?;
        install_plugin_from_path_sync(working_dir, &source_path_string, scope)
    })();
    cleanup_resolved_plugin_source(&resolved_source);
    install_result
}

fn validate_download_source_manifest(
    source: &PluginDownloadSource,
    source_path: &str,
) -> Result<(), String> {
    let expected_id = source.id.trim();
    let expected_version = source.version.trim();
    if expected_id.is_empty() && expected_version.is_empty() {
        return Ok(());
    }

    let manifest = inspect_plugin_source_manifest_sync(source_path)?;
    if !expected_id.is_empty() {
        let expected_id = normalize_plugin_id(expected_id)?;
        if manifest.id != expected_id {
            return Err(format!(
                "Plugin source id mismatch: expected {}, got {}",
                expected_id, manifest.id
            ));
        }
    }
    if !expected_version.is_empty() && manifest.version != expected_version {
        return Err(format!(
            "Plugin source version mismatch: expected {}, got {}",
            expected_version, manifest.version
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportRequest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub file_path: String,
    #[serde(default)]
    pub skill_package_ids: Vec<String>,
    #[serde(default)]
    pub view_ids: Vec<String>,
    #[serde(default)]
    pub rule_files: Vec<PluginExportRuleFile>,
    #[serde(default)]
    pub project_dependencies: Vec<LocusPluginProjectDependency>,
    #[serde(default)]
    pub install_after_export: bool,
    #[serde(default)]
    pub install_scope: Option<PluginInstallScope>,
    #[serde(default)]
    pub transfer_ownership: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportRuleFile {
    pub file_name: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginExportResult {
    pub id: String,
    pub path: String,
    pub skill_count: usize,
    pub view_count: usize,
    pub rule_count: usize,
    pub file_count: usize,
    pub byte_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_plugin: Option<InstalledPluginSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transferred_components: Vec<PluginTransferredComponent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTransferredComponent {
    pub kind: String,
    pub id: String,
    pub source_root: String,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginExportComponent {
    id: String,
    path: String,
}

#[derive(Debug, Clone)]
struct PluginExportTransferSource {
    kind: &'static str,
    id: String,
    source_root: PathBuf,
}

fn normalize_export_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("Plugin export path is required".to_string());
    }
    let mut path = PathBuf::from(trimmed);
    let has_zip_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if !has_zip_extension {
        path.set_extension("zip");
    }
    Ok(path)
}

fn normalize_project_dependencies(
    dependencies: Vec<LocusPluginProjectDependency>,
) -> Vec<LocusPluginProjectDependency> {
    dependencies
        .into_iter()
        .filter_map(|mut dependency| {
            dependency.kind = dependency.kind.trim().to_string();
            if dependency.kind.is_empty() {
                dependency.kind = "custom".to_string();
            }
            dependency.name = dependency.name.trim().to_string();
            if dependency.name.is_empty() {
                return None;
            }
            dependency.version = dependency
                .version
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            dependency.notes = dependency
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            Some(dependency)
        })
        .collect()
}

fn unique_ids(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn component_dir_name(value: &str) -> Result<String, String> {
    normalize_plugin_id(value)
}

fn normalize_plugin_rule_file_name(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.starts_with('.')
    {
        return Err("Invalid plugin rule file name".to_string());
    }
    let file_name = if trimmed.ends_with(".md") {
        trimmed.to_string()
    } else {
        format!("{}.md", trimmed)
    };
    let stem = file_name.trim_end_matches(".md");
    if stem.is_empty()
        || !stem
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("Invalid plugin rule file name".to_string());
    }
    Ok(file_name)
}

fn zip_plugin_root(root: &Path, output_path: &Path) -> Result<usize, String> {
    if let Some(parent) = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    let output = fs::File::create(output_path)
        .map_err(|e| format!("Failed to create {}: {}", output_path.display(), e))?;
    let mut archive = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut paths = Vec::new();
    for entry in WalkDir::new(root).min_depth(1).follow_links(false) {
        let entry = entry.map_err(|e| format!("Failed to scan plugin export files: {}", e))?;
        paths.push(entry.path().to_path_buf());
    }
    paths.sort();

    let mut file_count = 0usize;
    for path in paths {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("Failed to inspect {}: {}", path.display(), e))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to export symlinked plugin entry: {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "Unsupported plugin export entry: {}",
                path.display()
            ));
        }
        let rel_path = path
            .strip_prefix(root)
            .map_err(|e| format!("Failed to resolve plugin export path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        archive
            .start_file(rel_path.clone(), options)
            .map_err(|e| format!("Failed to write plugin archive entry '{}': {}", rel_path, e))?;
        let mut input = fs::File::open(&path)
            .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
        std::io::copy(&mut input, &mut archive)
            .map_err(|e| format!("Failed to write archive data for {}: {}", path.display(), e))?;
        file_count += 1;
    }
    archive
        .finish()
        .map_err(|e| format!("Failed to finish plugin archive: {}", e))?;
    Ok(file_count)
}

pub fn export_plugin_archive_sync(
    working_dir: &str,
    request: PluginExportRequest,
) -> Result<PluginExportResult, String> {
    let plugin_id = normalize_plugin_id(&request.id)?;
    let plugin_name = request.name.trim();
    let plugin_name = if plugin_name.is_empty() {
        plugin_id.clone()
    } else {
        plugin_name.to_string()
    };
    let plugin_version = request.version.trim();
    let plugin_version = if plugin_version.is_empty() {
        "0.1.0".to_string()
    } else {
        plugin_version.to_string()
    };
    let output_path = normalize_export_path(&request.file_path)?;
    let skill_ids = unique_ids(request.skill_package_ids);
    let view_ids = unique_ids(request.view_ids);
    let mut seen_rule_files = BTreeSet::new();
    let rule_files = request
        .rule_files
        .into_iter()
        .map(|rule| {
            let file_name = normalize_plugin_rule_file_name(&rule.file_name)?;
            Ok((file_name, rule.content))
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .filter(|(file_name, _)| seen_rule_files.insert(file_name.clone()))
        .collect::<Vec<_>>();
    if skill_ids.is_empty() && view_ids.is_empty() && rule_files.is_empty() {
        return Err("Select at least one Skill package, View, or Rule to export.".to_string());
    }
    if request.transfer_ownership && !request.install_after_export {
        return Err("Plugin ownership transfer requires installAfterExport.".to_string());
    }
    let install_after_export = request.install_after_export;
    let install_scope = request.install_scope.unwrap_or(PluginInstallScope::App);
    let transfer_ownership = request.transfer_ownership;
    let project_dependencies = normalize_project_dependencies(request.project_dependencies);
    let allow_project_dependencies = !project_dependencies.is_empty();

    let staging_root =
        std::env::temp_dir().join(format!("locus-plugin-export-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create plugin export staging directory: {}", e))?;

    let export_result = (|| -> Result<PluginExportResult, String> {
        let mut skill_components = Vec::new();
        let mut view_components = Vec::new();
        let mut rule_components = Vec::new();
        let mut transfer_sources = Vec::new();
        let mut copied_file_count = 0usize;

        for package_id in skill_ids {
            let dir_name = component_dir_name(&package_id)?;
            let rel_path = format!("skills/{}", dir_name);
            let target_root = staging_root.join(&rel_path);
            let copied = super::skill::copy_skill_package_for_plugin_sync(
                working_dir,
                &package_id,
                &target_root,
                allow_project_dependencies,
            )?;
            copied_file_count += copied.file_count;
            skill_components.push(PluginExportComponent {
                id: copied.id.clone(),
                path: rel_path,
            });
            transfer_sources.push(PluginExportTransferSource {
                kind: "skill",
                id: copied.id,
                source_root: copied.source_root,
            });
        }

        for view_id in view_ids {
            let dir_name = component_dir_name(&view_id)?;
            let rel_path = format!("views/{}", dir_name);
            let target_root = staging_root.join(&rel_path);
            let copied = crate::view::copy_view_package_for_plugin_sync(
                working_dir,
                &view_id,
                &target_root,
            )?;
            copied_file_count += copied.file_count;
            view_components.push(PluginExportComponent {
                id: copied.id.clone(),
                path: rel_path,
            });
            transfer_sources.push(PluginExportTransferSource {
                kind: "view",
                id: copied.id,
                source_root: copied.source_root,
            });
        }

        for (file_name, content) in rule_files {
            let rel_path = format!("rules/{}", file_name);
            let target_path = staging_root.join(&rel_path);
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
            }
            fs::write(&target_path, content)
                .map_err(|e| format!("Failed to write plugin Rule {}: {}", rel_path, e))?;
            copied_file_count += 1;
            rule_components.push(PluginExportComponent {
                id: file_name.trim_end_matches(".md").to_string(),
                path: rel_path,
            });
        }

        let project_independent = project_dependencies.is_empty();
        let skill_count = skill_components.len();
        let view_count = view_components.len();
        let rule_count = rule_components.len();
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": &plugin_id,
            "name": &plugin_name,
            "version": &plugin_version,
            "compatibility": {
                "projectIndependent": project_independent
            },
            "dependencies": {
                "project": &project_dependencies
            },
            "components": {
                "agents": [],
                "rules": &rule_components,
                "skills": &skill_components,
                "views": &view_components
            }
        });
        let manifest_path = staging_root.join(PLUGIN_MANIFEST_FILE_NAME);
        let manifest_text = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Failed to serialize plugin manifest: {}", e))?;
        fs::write(&manifest_path, manifest_text)
            .map_err(|e| format!("Failed to write {}: {}", manifest_path.display(), e))?;

        let file_count = zip_plugin_root(&staging_root, &output_path)?;
        let byte_size = fs::metadata(&output_path)
            .map(|meta| meta.len())
            .unwrap_or(0);

        let mut installed_plugin = None;
        let mut transferred_components = Vec::new();
        if install_after_export {
            if transfer_ownership {
                for source in &transfer_sources {
                    match source.kind {
                        "skill" => {
                            super::skill::preflight_skill_package_transfer_to_plugin_sync(
                                working_dir,
                                &source.id,
                                &source.source_root,
                            )?;
                        }
                        "view" => {
                            crate::view::preflight_view_package_transfer_to_plugin_sync(
                                working_dir,
                                &source.id,
                                &source.source_root,
                            )?;
                        }
                        _ => {
                            return Err(format!(
                                "Unsupported plugin transfer source: {}",
                                source.kind
                            ))
                        }
                    }
                }
            }
            let output_path_string = output_path.display().to_string();
            let installed =
                install_plugin_from_path_sync(working_dir, &output_path_string, install_scope)?;
            if transfer_ownership {
                for source in transfer_sources {
                    match source.kind {
                        "skill" => {
                            super::skill::transfer_skill_package_to_plugin_sync(
                                working_dir,
                                &source.id,
                                &source.source_root,
                            )?;
                        }
                        "view" => {
                            crate::view::transfer_view_package_to_plugin_sync(
                                working_dir,
                                &source.id,
                                &source.source_root,
                            )?;
                        }
                        _ => {
                            return Err(format!(
                                "Unsupported plugin transfer source: {}",
                                source.kind
                            ))
                        }
                    }
                    transferred_components.push(PluginTransferredComponent {
                        kind: source.kind.to_string(),
                        id: source.id,
                        source_root: source.source_root.display().to_string().replace('\\', "/"),
                        plugin_id: installed.id.clone(),
                    });
                }
            }
            installed_plugin = Some(installed);
        }

        Ok(PluginExportResult {
            id: plugin_id,
            path: output_path.display().to_string().replace('\\', "/"),
            skill_count,
            view_count,
            rule_count,
            file_count: file_count.max(copied_file_count.saturating_add(1)),
            byte_size,
            installed_plugin,
            transferred_components,
        })
    })();

    if staging_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
    }
    export_result
}

#[tauri::command]
pub async fn plugin_registry_fetch_manifest(
    registry_base_url: Option<String>,
    cache_mode: Option<PluginRegistryCacheMode>,
) -> Result<PluginRegistryManifestFetchResult, AppError> {
    let base_url = normalize_registry_base_url(registry_base_url).map_err(AppError::from)?;
    let manifest_url = resolve_registry_url(&base_url, "manifest.json").map_err(AppError::from)?;
    let mut manifest = fetch_registry_json::<PluginRegistryManifest>(
        &manifest_url,
        cache_mode.unwrap_or_default(),
    )
    .await
    .map_err(AppError::from)?;
    manifest.available_buckets = manifest
        .available_buckets
        .into_iter()
        .filter_map(|bucket| normalize_registry_bucket(&bucket).ok())
        .collect();
    manifest.available_buckets.sort();
    manifest.available_buckets.dedup();
    manifest.search_index_path = normalize_registry_subpath(
        Some(manifest.search_index_path.clone()),
        "search/summaries.json",
    )
    .map_err(AppError::from)?;
    Ok(PluginRegistryManifestFetchResult { base_url, manifest })
}

fn normalize_registry_summary(summary: &mut PluginRegistrySummary) -> Result<(), String> {
    summary.id = normalize_plugin_id(&summary.id)?;
    summary.name = summary.name.trim().to_string();
    summary.summary = summary.summary.trim().to_string();
    normalize_registry_localized_text(&mut summary.summary_i18n);
    summary.author = summary.author.trim().to_string();
    summary.latest_version = summary.latest_version.trim().to_string();
    summary.updated_at = summary.updated_at.trim().to_string();
    normalize_registry_icon(&mut summary.icon);
    for stat in &mut summary.stats {
        normalize_registry_stat(stat);
    }
    summary.stats.retain(|stat| !stat.is_empty());
    summary.tags = summary
        .tags
        .iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    Ok(())
}

#[tauri::command]
pub async fn plugin_registry_fetch_shard(
    registry_base_url: Option<String>,
    summary_base_path: Option<String>,
    bucket: String,
    cache_mode: Option<PluginRegistryCacheMode>,
) -> Result<PluginRegistryShard, AppError> {
    let base_url = normalize_registry_base_url(registry_base_url).map_err(AppError::from)?;
    let summary_base_path =
        normalize_registry_subpath(summary_base_path, "shards").map_err(AppError::from)?;
    let bucket = normalize_registry_bucket(&bucket).map_err(AppError::from)?;
    let shard_url =
        resolve_registry_url(&base_url, &format!("{}/{}.json", summary_base_path, bucket))
            .map_err(AppError::from)?;
    let mut shard = fetch_registry_json_optional::<PluginRegistryShard>(
        &shard_url,
        cache_mode.unwrap_or_default(),
    )
    .await
    .map_err(AppError::from)?
    .unwrap_or_else(|| PluginRegistryShard {
        schema_version: 1,
        bucket: bucket.clone(),
        plugins: Vec::new(),
    });
    if shard.bucket.trim().is_empty() {
        shard.bucket = bucket;
    }
    for plugin in &mut shard.plugins {
        normalize_registry_summary(plugin).map_err(AppError::from)?;
    }
    shard.plugins.retain(|plugin| !plugin.name.is_empty());
    Ok(shard)
}

#[tauri::command]
pub async fn plugin_registry_fetch_search_index(
    registry_base_url: Option<String>,
    search_index_path: Option<String>,
    cache_mode: Option<PluginRegistryCacheMode>,
) -> Result<PluginRegistrySearchIndex, AppError> {
    let base_url = normalize_registry_base_url(registry_base_url).map_err(AppError::from)?;
    let search_index_path = normalize_registry_subpath(search_index_path, "search/summaries.json")
        .map_err(AppError::from)?;
    let search_index_url =
        resolve_registry_url(&base_url, &search_index_path).map_err(AppError::from)?;
    let mut index = fetch_registry_json::<PluginRegistrySearchIndex>(
        &search_index_url,
        cache_mode.unwrap_or_default(),
    )
    .await
    .map_err(AppError::from)?;
    for plugin in &mut index.plugins {
        normalize_registry_summary(plugin).map_err(AppError::from)?;
    }
    index.plugins.retain(|plugin| !plugin.name.is_empty());
    index.plugins.sort_by(|a, b| a.id.cmp(&b.id));
    index.plugins.dedup_by(|a, b| a.id == b.id);
    Ok(index)
}

#[tauri::command]
pub async fn plugin_registry_fetch_plugin(
    registry_base_url: Option<String>,
    entry_base_path: Option<String>,
    plugin_id: String,
    cache_mode: Option<PluginRegistryCacheMode>,
) -> Result<PluginRegistryEntry, AppError> {
    let base_url = normalize_registry_base_url(registry_base_url).map_err(AppError::from)?;
    let entry_base_path =
        normalize_registry_subpath(entry_base_path, "plugins").map_err(AppError::from)?;
    let plugin_id = normalize_plugin_id(&plugin_id).map_err(AppError::from)?;
    let bucket = plugin_registry_bucket_for_id(&plugin_id).map_err(AppError::from)?;
    let entry_url = resolve_registry_url(
        &base_url,
        &format!("{}/{}/{}.json", entry_base_path, bucket, plugin_id),
    )
    .map_err(AppError::from)?;
    let mut entry =
        fetch_registry_json::<PluginRegistryEntry>(&entry_url, cache_mode.unwrap_or_default())
            .await
            .map_err(AppError::from)?;
    entry.id = normalize_plugin_id(&entry.id).map_err(AppError::from)?;
    if entry.id != plugin_id {
        return Err("Plugin registry detail id mismatch".into());
    }
    entry.name = entry.name.trim().to_string();
    entry.summary = entry.summary.trim().to_string();
    normalize_registry_localized_text(&mut entry.summary_i18n);
    entry.description = entry.description.trim().to_string();
    normalize_registry_localized_text(&mut entry.description_i18n);
    normalize_registry_description_source(&mut entry.description_source);
    normalize_registry_localized_description_sources(&mut entry.description_source_i18n);
    entry.author = entry.author.trim().to_string();
    entry.repo = entry.repo.trim().to_string();
    entry.license = entry.license.trim().to_string();
    entry.latest_version = entry.latest_version.trim().to_string();
    normalize_registry_icon(&mut entry.icon);
    for stat in &mut entry.stats {
        normalize_registry_stat(stat);
    }
    entry.stats.retain(|stat| !stat.is_empty());
    entry.tags = entry
        .tags
        .iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    if entry.name.is_empty() {
        entry.name = entry.id.clone();
    }
    Ok(entry)
}

#[tauri::command]
pub async fn plugin_registry_fetch_description(
    repo: Option<String>,
    description_source: Option<PluginRegistryDescriptionSource>,
    cache_mode: Option<PluginRegistryCacheMode>,
) -> Result<PluginRegistryDescriptionFetchResult, AppError> {
    let source_url =
        resolve_registry_description_source_url(repo.as_deref(), description_source.as_ref())
            .map_err(AppError::from)?
            .ok_or_else(|| AppError::from("Plugin registry description source is missing"))?;
    let content = fetch_registry_text(&source_url, cache_mode.unwrap_or_default())
        .await
        .map_err(AppError::from)?;
    Ok(PluginRegistryDescriptionFetchResult {
        content: rewrite_markdown_relative_urls(&content, &source_url),
        source_url,
    })
}

#[tauri::command]
pub async fn plugin_list_installed(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<InstalledPluginSummary>, AppError> {
    let working_dir = workspace.path.read().await.clone();
    Ok(list_installed_plugin_summaries(&working_dir))
}

#[tauri::command]
pub async fn plugin_install_from_path(
    source_path: String,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<InstalledPluginSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let summary =
        install_plugin_from_path_sync(&working_dir, &source_path, scope).map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_install");
    Ok(summary)
}

#[tauri::command]
pub async fn plugin_install_from_registry(
    request: PluginRegistryInstallRequest,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<InstalledPluginSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let summary = install_plugin_from_registry_request(&working_dir, request, scope)
        .await
        .map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_registry_install");
    Ok(summary)
}

#[tauri::command]
pub async fn plugin_install_from_source(
    source: PluginDownloadSource,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<InstalledPluginSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let summary = install_plugin_from_download_source(&working_dir, source, scope)
        .await
        .map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_source_install");
    Ok(summary)
}

#[tauri::command]
pub async fn plugin_set_enabled(
    plugin_id: String,
    scope: PluginInstallScope,
    enabled: bool,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<InstalledPluginSummary, AppError> {
    let working_dir = workspace.path.read().await.clone();
    if scope == PluginInstallScope::Project && working_dir.trim().is_empty() {
        return Err("No working directory selected".to_string().into());
    }
    let summary = set_plugin_enabled_sync(&working_dir, &plugin_id, scope, enabled)
        .map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(
        &app_handle,
        &working_dir,
        if enabled {
            "plugin_enable"
        } else {
            "plugin_disable"
        },
    );
    Ok(summary)
}

#[tauri::command]
pub async fn plugin_github_auth_status() -> Result<PluginGithubAuthStatus, AppError> {
    let config = read_plugin_github_auth_config().await;
    Ok(plugin_github_auth_status_from_config(config))
}

#[tauri::command]
pub async fn plugin_github_repo_star_status(
    repo: String,
) -> Result<PluginGithubRepoStarStatus, AppError> {
    let token = plugin_github_required_token()
        .await
        .map_err(AppError::from)?;
    fetch_github_repo_star_status_with_token(&repo, &token)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn plugin_github_repo_set_starred(
    repo: String,
    starred: bool,
) -> Result<PluginGithubRepoStarStatus, AppError> {
    let token = plugin_github_required_token()
        .await
        .map_err(AppError::from)?;
    set_github_repo_star_status_with_token(&repo, starred, &token)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn plugin_github_auth_save_token(
    token: String,
) -> Result<PluginGithubAuthStatus, AppError> {
    save_plugin_github_token_auth(&token)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn plugin_github_oauth_start() -> Result<PluginGithubOAuthStartResult, AppError> {
    resolve_github_cli()
        .ok_or_else(github_cli_unavailable_message)
        .map_err(AppError::from)?;

    if let Ok(auth) = import_github_cli_auth().await {
        return Ok(github_cli_oauth_start_result(
            "completed".to_string(),
            String::new(),
            Some("GitHub CLI authentication imported".to_string()),
            Some(auth),
        ));
    }

    if let Some(result) = collect_finished_github_cli_login_session(None).await {
        match result {
            Ok(auth) => {
                return Ok(github_cli_oauth_start_result(
                    "completed".to_string(),
                    String::new(),
                    Some("GitHub CLI authentication imported".to_string()),
                    Some(auth),
                ));
            }
            Err(error) => return Err(AppError::from(error)),
        }
    }

    {
        let guard = PLUGIN_GITHUB_CLI_LOGIN_SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(session) = guard.as_ref() {
            if !session.handle.is_finished() {
                return Ok(github_cli_oauth_start_result(
                    session.id.clone(),
                    github_cli_login_user_code(&session.progress),
                    Some("GitHub CLI login is already in progress".to_string()),
                    None,
                ));
            }
        }
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let progress = Arc::new(Mutex::new(PluginGithubCliLoginProgress::default()));
    let login_progress = progress.clone();
    let handle = tokio::spawn(async move {
        run_github_cli_login(login_progress).await?;
        import_github_cli_auth().await
    });
    {
        let mut guard = PLUGIN_GITHUB_CLI_LOGIN_SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(PluginGithubCliLoginSession {
            id: session_id.clone(),
            handle,
            progress: progress.clone(),
        });
    }

    let user_code = wait_for_github_cli_user_code(progress).await;

    Ok(github_cli_oauth_start_result(
        session_id,
        user_code,
        Some("GitHub CLI login started".to_string()),
        None,
    ))
}

#[tauri::command]
pub async fn plugin_github_oauth_poll(
    device_code: String,
) -> Result<PluginGithubOAuthPollResult, AppError> {
    let device_code = device_code.trim().to_string();
    if device_code.is_empty() {
        return Err(AppError::from(
            "GitHub CLI login session id is required".to_string(),
        ));
    }

    let user_code = github_cli_login_session_user_code(&device_code).unwrap_or_default();

    if let Ok(auth) = import_github_cli_auth().await {
        return Ok(github_cli_oauth_poll_result(
            "success",
            user_code,
            None,
            Some(auth),
        ));
    }

    if let Some(result) = collect_finished_github_cli_login_session(Some(&device_code)).await {
        return match result {
            Ok(auth) => Ok(github_cli_oauth_poll_result(
                "success",
                user_code,
                None,
                Some(auth),
            )),
            Err(error) => Ok(github_cli_oauth_poll_result(
                "failed",
                user_code,
                Some(error),
                None,
            )),
        };
    }

    let has_matching_session = {
        let guard = PLUGIN_GITHUB_CLI_LOGIN_SESSION
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .as_ref()
            .map(|session| session.id == device_code)
            .unwrap_or(false)
    };
    if !has_matching_session {
        return Ok(github_cli_oauth_poll_result(
            "failed",
            user_code,
            Some("GitHub CLI login session expired".to_string()),
            None,
        ));
    }

    Ok(github_cli_oauth_poll_result(
        "pending", user_code, None, None,
    ))
}

#[tauri::command]
pub async fn plugin_github_auth_logout() -> Result<PluginGithubAuthStatus, AppError> {
    clear_plugin_github_auth_config()
        .await
        .map_err(AppError::from)?;
    Ok(PluginGithubAuthStatus {
        authenticated: false,
        account: String::new(),
    })
}

#[tauri::command]
pub async fn plugin_uninstall(
    plugin_id: String,
    scope: PluginInstallScope,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<String, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let removed = uninstall_plugin_sync(&working_dir, &plugin_id, scope).map_err(AppError::from)?;
    reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
    emit_plugins_changed(&app_handle, &working_dir, "plugin_uninstall");
    Ok(removed)
}

#[tauri::command]
pub async fn plugin_export(
    request: PluginExportRequest,
    workspace: State<'_, Arc<Workspace>>,
    registry: State<'_, AgentDefRegistryState>,
    app_agent_dir: State<'_, AppAgentDir>,
    app_handle: AppHandle,
) -> Result<PluginExportResult, AppError> {
    let working_dir = workspace.path.read().await.clone();
    let result = export_plugin_archive_sync(&working_dir, request).map_err(AppError::from)?;
    if result.installed_plugin.is_some() || !result.transferred_components.is_empty() {
        reload_agent_registry(&registry, &app_agent_dir, &working_dir).await;
        emit_plugins_changed(&app_handle, &working_dir, "plugin_export");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use tempfile::TempDir;

    #[test]
    fn plugin_git_command_args_use_windows_schannel_backend() {
        let args = vec![
            "clone".to_string(),
            "--depth".to_string(),
            "1".to_string(),
            "https://github.com/owner/repo.git".to_string(),
        ];

        let command_args = plugin_git_command_args(&args);

        if cfg!(target_os = "windows") {
            assert_eq!(
                &command_args[..2],
                &["-c".to_string(), "http.sslBackend=schannel".to_string()]
            );
            assert_eq!(&command_args[2..], args.as_slice());
        } else {
            assert_eq!(command_args, args);
        }
    }

    #[test]
    fn plugin_github_git_auth_header_uses_basic_token_auth() {
        let header = plugin_github_git_auth_header(" ghp_test_token ");
        let encoded = header
            .strip_prefix("AUTHORIZATION: basic ")
            .expect("basic auth prefix");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decode auth header");

        assert_eq!(
            String::from_utf8(decoded).expect("utf8 credential"),
            "x-access-token:ghp_test_token"
        );
    }

    #[test]
    fn extracts_github_cli_device_code_from_login_output() {
        assert_eq!(
            extract_github_cli_user_code("! First copy your one-time code: ABCD-EFGH"),
            Some("ABCD-EFGH".to_string())
        );
        assert_eq!(
            extract_github_cli_user_code("Enter the code displayed in the app: 12345678"),
            Some("12345678".to_string())
        );
        assert_eq!(
            extract_github_cli_user_code("Waiting for authorization..."),
            None
        );
    }

    fn write_test_plugin_source(root: &Path, id: &str, version: &str) {
        std::fs::create_dir_all(root).expect("create plugin root");
        let manifest = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "name": id,
            "version": version,
            "components": {
                "agents": [],
                "rules": [],
                "skills": [],
                "views": []
            }
        });
        std::fs::write(
            root.join(PLUGIN_MANIFEST_FILE_NAME),
            serde_json::to_string_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write plugin manifest");
    }

    #[test]
    fn download_source_manifest_validation_checks_id_and_version() {
        let source_root = TempDir::new().expect("source root");
        write_test_plugin_source(source_root.path(), "com.example.plugin", "1.2.3");
        let source_path = source_root.path().to_string_lossy();

        validate_download_source_manifest(
            &PluginDownloadSource {
                id: "com.example.plugin".to_string(),
                version: "1.2.3".to_string(),
                ..Default::default()
            },
            &source_path,
        )
        .expect("matching source");

        let id_error = validate_download_source_manifest(
            &PluginDownloadSource {
                id: "com.example.other".to_string(),
                ..Default::default()
            },
            &source_path,
        )
        .expect_err("id mismatch");
        assert!(id_error.contains("Plugin source id mismatch"));

        let version_error = validate_download_source_manifest(
            &PluginDownloadSource {
                version: "2.0.0".to_string(),
                ..Default::default()
            },
            &source_path,
        )
        .expect_err("version mismatch");
        assert!(version_error.contains("Plugin source version mismatch"));
    }

    #[test]
    fn export_plugin_archive_writes_project_dependency_metadata() {
        let workspace = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        crate::view::create_view_sync(
            &workspace.path().to_string_lossy(),
            crate::view::ViewCreateRequest {
                id: "asset-inspector".to_string(),
                package_name: None,
                name: Some("Asset Inspector".to_string()),
                template: Some("blank".to_string()),
                icon: None,
                display_path: None,
            },
        )
        .expect("create view");

        let output_path = output_dir.path().join("asset-tools.zip");
        let result = export_plugin_archive_sync(
            &workspace.path().to_string_lossy(),
            PluginExportRequest {
                id: "com.example.asset-tools".to_string(),
                name: "Asset Tools".to_string(),
                version: "0.1.0".to_string(),
                file_path: output_path.to_string_lossy().to_string(),
                skill_package_ids: Vec::new(),
                view_ids: vec!["asset-inspector".to_string()],
                rule_files: Vec::new(),
                project_dependencies: vec![LocusPluginProjectDependency {
                    kind: "unityPackage".to_string(),
                    name: "com.example.runtime".to_string(),
                    version: Some("1.2.3".to_string()),
                    notes: Some("Runtime scripts required by View property editors.".to_string()),
                }],
                install_after_export: false,
                install_scope: None,
                transfer_ownership: false,
            },
        )
        .expect("export plugin");

        assert_eq!(result.id, "com.example.asset-tools");
        assert_eq!(result.view_count, 1);
        assert!(output_path.is_file());

        let file = fs::File::open(output_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut manifest_entry = archive.by_name(PLUGIN_MANIFEST_FILE_NAME).unwrap();
        let mut manifest_text = String::new();
        manifest_entry.read_to_string(&mut manifest_text).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["compatibility"]["projectIndependent"], false);
        assert_eq!(
            manifest["dependencies"]["project"][0]["name"],
            "com.example.runtime"
        );
        assert_eq!(
            manifest["components"]["views"][0]["path"],
            "views/asset-inspector"
        );
    }

    #[test]
    fn export_plugin_archive_writes_optional_rule_files() {
        let workspace = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let output_path = output_dir.path().join("rule-tools.zip");

        let result = export_plugin_archive_sync(
            &workspace.path().to_string_lossy(),
            PluginExportRequest {
                id: "rule-tools".to_string(),
                name: "Rule Tools".to_string(),
                version: "0.1.0".to_string(),
                file_path: output_path.to_string_lossy().to_string(),
                skill_package_ids: Vec::new(),
                view_ids: Vec::new(),
                rule_files: vec![PluginExportRuleFile {
                    file_name: "risk_control".to_string(),
                    content: "# Risk Control\n\nUse extra caution.".to_string(),
                }],
                project_dependencies: Vec::new(),
                install_after_export: false,
                install_scope: None,
                transfer_ownership: false,
            },
        )
        .expect("export rule plugin");

        assert_eq!(result.rule_count, 1);
        assert!(output_path.is_file());

        let file = fs::File::open(output_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut manifest_entry = archive.by_name(PLUGIN_MANIFEST_FILE_NAME).unwrap();
        let mut manifest_text = String::new();
        manifest_entry.read_to_string(&mut manifest_text).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(
            manifest["components"]["rules"][0]["path"],
            "rules/risk_control.md"
        );

        drop(manifest_entry);
        let mut rule_entry = archive.by_name("rules/risk_control.md").unwrap();
        let mut rule_text = String::new();
        rule_entry.read_to_string(&mut rule_text).unwrap();
        assert!(rule_text.contains("# Risk Control"));
    }

    #[test]
    fn export_plugin_archive_installs_and_transfers_view_ownership() {
        let workspace = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        crate::view::create_view_sync(
            &working_dir,
            crate::view::ViewCreateRequest {
                id: "asset-board".to_string(),
                package_name: None,
                name: Some("Asset Board".to_string()),
                template: Some("blank".to_string()),
                icon: None,
                display_path: None,
            },
        )
        .expect("create view");
        let source_root =
            crate::view::resolve_view_package_root(&working_dir, "asset-board").unwrap();
        assert!(source_root.is_dir());

        let output_path = output_dir.path().join("asset-board-tools.zip");
        let result = export_plugin_archive_sync(
            &working_dir,
            PluginExportRequest {
                id: "com.example.asset-board-tools".to_string(),
                name: "Asset Board Tools".to_string(),
                version: "0.1.0".to_string(),
                file_path: output_path.to_string_lossy().to_string(),
                skill_package_ids: Vec::new(),
                view_ids: vec!["asset-board".to_string()],
                rule_files: Vec::new(),
                project_dependencies: Vec::new(),
                install_after_export: true,
                install_scope: Some(PluginInstallScope::Project),
                transfer_ownership: true,
            },
        )
        .expect("export, install, and transfer plugin");

        assert_eq!(result.view_count, 1);
        assert!(output_path.is_file());
        assert_eq!(
            result
                .installed_plugin
                .as_ref()
                .map(|plugin| (plugin.id.as_str(), plugin.scope)),
            Some(("com.example.asset-board-tools", PluginInstallScope::Project))
        );
        assert_eq!(result.transferred_components.len(), 1);
        assert_eq!(result.transferred_components[0].kind, "view");
        assert_eq!(result.transferred_components[0].id, "asset-board");
        assert!(!source_root.exists());

        let views = crate::view::list_views_sync(&working_dir).expect("list views");
        let view = views
            .iter()
            .find(|view| view.id == "asset-board")
            .expect("plugin-managed view should be visible");
        assert_eq!(view.source, "pluginProject");
        assert_eq!(
            view.plugin_id.as_deref(),
            Some("com.example.asset-board-tools")
        );
    }

    #[test]
    fn export_plugin_archive_accepts_concise_package_id() {
        let workspace = TempDir::new().unwrap();
        let output_dir = TempDir::new().unwrap();
        let working_dir = workspace.path().to_string_lossy().to_string();
        crate::view::create_view_sync(
            &working_dir,
            crate::view::ViewCreateRequest {
                id: "workspace-board".to_string(),
                package_name: None,
                name: Some("Workspace Board".to_string()),
                template: Some("blank".to_string()),
                icon: None,
                display_path: None,
            },
        )
        .expect("create view");

        let output_path = output_dir.path().join("locus-workspace.zip");
        let result = export_plugin_archive_sync(
            &working_dir,
            PluginExportRequest {
                id: "locus-workspace".to_string(),
                name: "Locus Workspace".to_string(),
                version: "0.1.0".to_string(),
                file_path: output_path.to_string_lossy().to_string(),
                skill_package_ids: Vec::new(),
                view_ids: vec!["workspace-board".to_string()],
                rule_files: Vec::new(),
                project_dependencies: Vec::new(),
                install_after_export: true,
                install_scope: Some(PluginInstallScope::Project),
                transfer_ownership: false,
            },
        )
        .expect("export and install plugin");

        assert_eq!(result.id, "locus-workspace");
        assert_eq!(
            result
                .installed_plugin
                .as_ref()
                .map(|plugin| (plugin.id.as_str(), plugin.scope)),
            Some(("locus-workspace", PluginInstallScope::Project))
        );

        let file = fs::File::open(output_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut manifest_entry = archive.by_name(PLUGIN_MANIFEST_FILE_NAME).unwrap();
        let mut manifest_text = String::new();
        manifest_entry.read_to_string(&mut manifest_text).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["id"], "locus-workspace");
    }

    #[test]
    fn registry_bucket_for_id_accepts_concise_package_id() {
        assert_eq!(
            plugin_registry_bucket_for_id("locus-workspace").expect("bucket"),
            "e1"
        );
    }

    #[test]
    fn registry_defaults_use_generated_public_index() {
        let manifest: PluginRegistryManifest = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "registryVersion": 1,
            "bucketStrategy": "sha256-id-prefix-2",
            "bucketCount": 256,
            "availableBuckets": []
        }))
        .expect("parse registry manifest");

        assert_eq!(
            DEFAULT_PLUGIN_REGISTRY_BASE_URL,
            "https://raw.githubusercontent.com/r1n7aro/locus-plugin-registry/main/public/v1"
        );
        assert_eq!(manifest.entry_base_path, "plugins");
        assert_eq!(manifest.summary_base_path, "shards");
        assert_eq!(manifest.search_index_path, "search/summaries.json");
    }

    #[test]
    fn registry_description_source_defaults_to_repo_readme() {
        let source = PluginRegistryDescriptionSource {
            path: "docs/DETAILS.md".to_string(),
            ..Default::default()
        };
        let url = resolve_registry_description_source_url(Some("owner/plugin-repo"), Some(&source))
            .expect("resolve description source")
            .expect("description url");

        assert_eq!(
            url,
            "https://raw.githubusercontent.com/owner/plugin-repo/main/docs/DETAILS.md"
        );
    }

    #[test]
    fn registry_description_source_accepts_direct_markdown_url() {
        let source = PluginRegistryDescriptionSource {
            url: "https://example.com/plugins/readme.md".to_string(),
            ..Default::default()
        };
        let url = resolve_registry_description_source_url(None, Some(&source))
            .expect("resolve description source")
            .expect("description url");

        assert_eq!(url, "https://example.com/plugins/readme.md");
    }

    #[test]
    fn github_latest_release_asset_url_uses_latest_download_route() {
        let url = github_release_asset_download_url(
            "owner",
            "plugin-repo",
            None,
            "com.example.plugin.zip",
        )
        .expect("release asset url");

        assert_eq!(
            url,
            "https://github.com/owner/plugin-repo/releases/latest/download/com.example.plugin.zip"
        );
    }

    #[test]
    fn github_tagged_release_asset_url_uses_tag_download_route() {
        let url = github_release_asset_download_url(
            "owner",
            "plugin-repo",
            Some("v1.2.3"),
            "com.example.plugin.zip",
        )
        .expect("release asset url");

        assert_eq!(
            url,
            "https://github.com/owner/plugin-repo/releases/download/v1.2.3/com.example.plugin.zip"
        );
    }

    #[test]
    fn github_release_asset_pattern_matches_versioned_zip() {
        assert!(release_asset_pattern_matches(
            "locus-workspace-*.zip",
            "locus-workspace-0.1.1.zip"
        ));
        assert!(!release_asset_pattern_matches(
            "locus-workspace-*.zip",
            "other-plugin-0.1.1.zip"
        ));
    }

    #[test]
    fn registry_download_is_resolved_requires_url_and_sha() {
        assert!(registry_download_is_resolved(&PluginRegistryDownload {
            url: "https://example.com/plugin.zip".to_string(),
            sha256: "a".repeat(64),
            size_bytes: Some(10),
        }));
        assert!(!registry_download_is_resolved(&PluginRegistryDownload {
            url: "https://example.com/plugin.zip".to_string(),
            sha256: String::new(),
            size_bytes: Some(10),
        }));
    }

    #[test]
    fn secure_plugin_urls_allow_https_and_local_http() {
        ensure_secure_plugin_url(
            &Url::parse("https://example.com/registry").unwrap(),
            "Plugin",
        )
        .expect("https should be allowed");
        ensure_secure_plugin_url(
            &Url::parse("http://localhost:14901/registry").unwrap(),
            "Plugin",
        )
        .expect("localhost http should be allowed");
        ensure_secure_plugin_url(
            &Url::parse("http://127.0.0.1:14901/registry").unwrap(),
            "Plugin",
        )
        .expect("loopback http should be allowed");
        ensure_secure_plugin_url(
            &Url::parse("http://[::1]:14901/registry").unwrap(),
            "Plugin",
        )
        .expect("ipv6 loopback http should be allowed");

        let error = ensure_secure_plugin_url(
            &Url::parse("http://example.com/registry").unwrap(),
            "Plugin registry",
        )
        .expect_err("remote http should be rejected");
        assert!(error.contains("must use HTTPS"));
        assert!(
            normalize_registry_base_url(Some("http://example.com/registry".to_string()))
                .expect_err("remote http registry should be rejected")
                .contains("must use HTTPS")
        );
        assert!(
            normalize_git_repo_source("http://example.com/owner/repo.git")
                .expect_err("remote http git repository should be rejected")
                .contains("must use HTTPS")
        );
    }

    #[test]
    fn registry_download_fallback_blocks_integrity_errors() {
        assert!(plugin_download_error_blocks_registry_fallback(
            "Plugin download sha256 mismatch: expected abc, got def"
        ));
        assert!(plugin_download_error_blocks_registry_fallback(
            "Plugin download size mismatch: expected 1 bytes, got 2"
        ));
        assert!(plugin_download_error_blocks_registry_fallback(
            "Plugin download too large: 123 bytes"
        ));
        assert!(!plugin_download_error_blocks_registry_fallback(
            "Failed to fetch plugin download: HTTP 404"
        ));
    }

    #[test]
    fn registry_localized_fields_normalize_language_aliases() {
        let mut values = BTreeMap::from([
            ("zh-CN".to_string(), " 中文摘要 ".to_string()),
            ("en-US".to_string(), " English summary ".to_string()),
            ("fr".to_string(), "   ".to_string()),
        ]);
        normalize_registry_localized_text(&mut values);

        assert_eq!(values.get("zh").map(String::as_str), Some("中文摘要"));
        assert_eq!(
            values.get("en").map(String::as_str),
            Some("English summary")
        );
        assert!(!values.contains_key("fr"));
    }

    #[test]
    fn registry_localized_description_sources_normalize_language_aliases() {
        let mut sources = BTreeMap::from([(
            "zh-CN".to_string(),
            PluginRegistryDescriptionSource {
                path: "docs/README.zh.md".to_string(),
                ..Default::default()
            },
        )]);
        normalize_registry_localized_description_sources(&mut sources);

        assert_eq!(
            sources.get("zh").map(|source| source.path.as_str()),
            Some("docs/README.zh.md")
        );
        assert_eq!(
            sources.get("zh").map(|source| source.kind.as_str()),
            Some("github")
        );
    }

    #[test]
    fn registry_install_expected_version_falls_back_to_latest_version() {
        let request = PluginRegistryInstallRequest {
            id: "com.example.plugin".to_string(),
            latest_version: "2.0.0".to_string(),
            download: PluginRegistryDownload::default(),
            download_source: PluginDownloadSource {
                kind: "release".to_string(),
                repo: "owner/repo".to_string(),
                version: String::new(),
                ..Default::default()
            },
        };

        assert_eq!(expected_registry_install_version(&request), "2.0.0");
    }

    #[test]
    fn registry_install_expected_version_prefers_download_source_version() {
        let request = PluginRegistryInstallRequest {
            id: "com.example.plugin".to_string(),
            latest_version: "2.0.0".to_string(),
            download: PluginRegistryDownload::default(),
            download_source: PluginDownloadSource {
                kind: "release".to_string(),
                repo: "owner/repo".to_string(),
                version: "2.1.0-beta.1".to_string(),
                ..Default::default()
            },
        };

        assert_eq!(expected_registry_install_version(&request), "2.1.0-beta.1");
    }

    #[test]
    fn registry_description_markdown_rewrites_relative_image_urls() {
        let markdown = "# Demo\n\n![Preview](images/preview.png)\n\n[Guide](docs/guide.md)";
        let rewritten = rewrite_markdown_relative_urls(
            markdown,
            "https://raw.githubusercontent.com/owner/repo/main/README.md",
        );

        assert!(rewritten.contains(
            "![Preview](https://raw.githubusercontent.com/owner/repo/main/images/preview.png)"
        ));
        assert!(rewritten
            .contains("[Guide](https://raw.githubusercontent.com/owner/repo/main/docs/guide.md)"));
    }
}
