use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use http::Uri;
use hyper_util::client::proxy::matcher::Matcher;
use serde::{Deserialize, Serialize};
use url::Url;

const PROXY_CONFIG_FILE: &str = "proxy_config.json";
const LOCAL_PROXY_BYPASS: &str = "localhost,127.0.0.1,::1,*.localhost,locus-binary.localhost";
const PROXY_ENV_KEYS: [&str; 8] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

#[derive(Debug, Default)]
struct InjectedProxyEnvState {
    entries: Vec<(String, OsString)>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyMode {
    Auto,
    Manual,
    Disabled,
}

impl Default for ProxyMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManualProxyConfig {
    #[serde(default)]
    pub http_proxy: String,
    #[serde(default)]
    pub https_proxy: String,
    #[serde(default)]
    pub all_proxy: String,
    #[serde(default)]
    pub no_proxy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    #[serde(default)]
    pub mode: ProxyMode,
    #[serde(default)]
    pub manual: ManualProxyConfig,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyEnvironmentEntryKind {
    Proxy,
    Bypass,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyEnvironmentEntry {
    pub key: String,
    pub value: String,
    pub kind: ProxyEnvironmentEntryKind,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProxyRouteSource {
    System,
    Environment,
    Manual,
    Direct,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyRoute {
    pub target_label: String,
    pub target_url: String,
    pub proxy_url: Option<String>,
    pub source: ProxyRouteSource,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SystemProxyConfig {
    pub platform: String,
    pub available: bool,
    pub source: String,
    pub enabled: Option<bool>,
    pub auto_detect: Option<bool>,
    pub auto_config_url: Option<String>,
    pub proxy_server: Option<String>,
    pub proxy_override: Option<String>,
    pub http_proxy: Option<String>,
    pub https_proxy: Option<String>,
    pub socks_proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub mode: ProxyMode,
    pub config: ProxyConfig,
    pub environment: Vec<ProxyEnvironmentEntry>,
    /// Backward-compatible alias for older frontends. Prefer `environment`.
    pub manual: Vec<ProxyEnvironmentEntry>,
    pub system: SystemProxyConfig,
    pub routes: Vec<ProxyRoute>,
}

#[derive(Debug, Clone)]
pub struct ReqwestClientOptions {
    connect_timeout: Option<Duration>,
    timeout: Option<Duration>,
    tcp_keepalive: Option<Duration>,
    tcp_nodelay: Option<bool>,
    pool_idle_timeout: Option<Duration>,
    pool_max_idle_per_host: Option<usize>,
    http2_adaptive_window: Option<bool>,
    http2_keep_alive_interval: Option<Duration>,
    http2_keep_alive_timeout: Option<Duration>,
    user_agent: Option<String>,
    gzip: Option<bool>,
    deflate: Option<bool>,
}

impl Default for ReqwestClientOptions {
    fn default() -> Self {
        Self {
            connect_timeout: None,
            timeout: None,
            tcp_keepalive: None,
            tcp_nodelay: None,
            pool_idle_timeout: None,
            pool_max_idle_per_host: None,
            http2_adaptive_window: None,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: None,
            user_agent: None,
            gzip: None,
            deflate: None,
        }
    }
}

impl ReqwestClientOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connect_timeout(mut self, value: Duration) -> Self {
        self.connect_timeout = Some(value);
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        self.timeout = Some(value);
        self
    }

    pub fn tcp_keepalive(mut self, value: Duration) -> Self {
        self.tcp_keepalive = Some(value);
        self
    }

    pub fn tcp_nodelay(mut self, value: bool) -> Self {
        self.tcp_nodelay = Some(value);
        self
    }

    pub fn pool_idle_timeout(mut self, value: Duration) -> Self {
        self.pool_idle_timeout = Some(value);
        self
    }

    pub fn pool_max_idle_per_host(mut self, value: usize) -> Self {
        self.pool_max_idle_per_host = Some(value);
        self
    }

    pub fn http2_adaptive_window(mut self, value: bool) -> Self {
        self.http2_adaptive_window = Some(value);
        self
    }

    pub fn http2_keep_alive_interval(mut self, value: Duration) -> Self {
        self.http2_keep_alive_interval = Some(value);
        self
    }

    pub fn http2_keep_alive_timeout(mut self, value: Duration) -> Self {
        self.http2_keep_alive_timeout = Some(value);
        self
    }

    pub fn user_agent(mut self, value: impl Into<String>) -> Self {
        self.user_agent = Some(value.into());
        self
    }

    pub fn gzip(mut self, value: bool) -> Self {
        self.gzip = Some(value);
        self
    }

    pub fn deflate(mut self, value: bool) -> Self {
        self.deflate = Some(value);
        self
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedProxyRoute {
    pub proxy_state: String,
    pub proxy_env_key: Option<String>,
    pub proxy_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ProxyEnvScope {
    clear_user_proxy_env: bool,
    entries: Vec<(String, OsString)>,
}

fn injected_proxy_env_state() -> &'static Mutex<InjectedProxyEnvState> {
    static STATE: OnceLock<Mutex<InjectedProxyEnvState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(InjectedProxyEnvState::default()))
}

#[cfg(test)]
fn proxy_config_override() -> &'static Mutex<Option<ProxyConfig>> {
    static OVERRIDE: OnceLock<Mutex<Option<ProxyConfig>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn set_proxy_config_override(config: Option<ProxyConfig>) {
    let mut guard = proxy_config_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = config;
}

#[cfg(test)]
fn system_proxy_config_override() -> &'static Mutex<Option<SystemProxyConfig>> {
    static OVERRIDE: OnceLock<Mutex<Option<SystemProxyConfig>>> = OnceLock::new();
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
fn set_system_proxy_config_override(config: Option<SystemProxyConfig>) {
    let mut guard = system_proxy_config_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = config;
}

#[cfg(test)]
fn system_proxy_config_override_value() -> Option<SystemProxyConfig> {
    system_proxy_config_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .cloned()
}

fn proxy_config_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join(PROXY_CONFIG_FILE))
}

fn sanitize_manual_proxy_config(config: ManualProxyConfig) -> ManualProxyConfig {
    ManualProxyConfig {
        http_proxy: config.http_proxy.trim().to_string(),
        https_proxy: config.https_proxy.trim().to_string(),
        all_proxy: config.all_proxy.trim().to_string(),
        no_proxy: String::new(),
    }
}

fn sanitize_proxy_config(config: ProxyConfig) -> ProxyConfig {
    ProxyConfig {
        mode: config.mode,
        manual: sanitize_manual_proxy_config(config.manual),
    }
}

fn load_proxy_config() -> ProxyConfig {
    #[cfg(test)]
    if let Some(config) = proxy_config_override()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .cloned()
    {
        return config;
    }

    let Ok(path) = proxy_config_path() else {
        return ProxyConfig::default();
    };

    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ProxyConfig>(&raw).ok())
        .map(sanitize_proxy_config)
        .unwrap_or_default()
}

pub fn save_proxy_config(config: ProxyConfig) -> Result<ProxyStatus, String> {
    let config = sanitize_proxy_config(config);
    let path = proxy_config_path()?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize proxy config: {}", e))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to save proxy config '{}': {}", path.display(), e))?;

    let _guard = proxy_env_access_guard();
    clear_locus_injected_proxy_env_unlocked();
    Ok(get_proxy_status_unlocked())
}

fn proxy_env_access_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn proxy_env_access_guard() -> MutexGuard<'static, ()> {
    proxy_env_access_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn env_key_matches(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn env_key_matches_os(left: &OsStr, right: &str) -> bool {
    env_key_matches(&left.to_string_lossy(), right)
}

fn is_locus_injected_proxy_env(key: &str, value: &OsStr) -> bool {
    injected_proxy_env_state()
        .lock()
        .map(|state| {
            state.entries.iter().any(|(entry_key, entry_value)| {
                env_key_matches(entry_key, key) && entry_value.as_os_str() == value
            })
        })
        .unwrap_or(false)
}

fn clear_locus_injected_proxy_env_locked(state: &mut InjectedProxyEnvState) {
    for (key, value) in &state.entries {
        if std::env::var_os(key)
            .as_deref()
            .map(|current| current == value.as_os_str())
            .unwrap_or(false)
        {
            std::env::remove_var(key);
        }
    }
    state.entries.clear();
}

fn clear_locus_injected_proxy_env_unlocked() {
    if let Ok(mut state) = injected_proxy_env_state().lock() {
        clear_locus_injected_proxy_env_locked(&mut state);
    }
}

#[cfg(test)]
fn clear_locus_injected_proxy_env() {
    let _guard = proxy_env_access_guard();
    clear_locus_injected_proxy_env_unlocked();
}

#[cfg(test)]
fn replace_locus_injected_proxy_env_unlocked(entries: Vec<(String, OsString)>) {
    if let Ok(mut state) = injected_proxy_env_state().lock() {
        clear_locus_injected_proxy_env_locked(&mut state);
        state.entries = entries
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| {
                std::env::set_var(&key, &value);
                (key, value)
            })
            .collect();
    }
}

#[cfg(test)]
fn replace_locus_injected_proxy_env(entries: Vec<(String, OsString)>) {
    let _guard = proxy_env_access_guard();
    replace_locus_injected_proxy_env_unlocked(entries);
}

fn locus_injected_proxy_env_keys_to_remove() -> Vec<String> {
    injected_proxy_env_state()
        .lock()
        .map(|state| {
            state
                .entries
                .iter()
                .filter_map(|(key, value)| {
                    let current = std::env::var_os(key)?;
                    if current == *value {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn remove_locus_injected_proxy_entries_from_map(
    map: &mut std::collections::HashMap<OsString, OsString>,
) {
    let entries = injected_proxy_env_state()
        .lock()
        .map(|state| state.entries.clone())
        .unwrap_or_default();
    map.retain(|key, _| {
        !entries
            .iter()
            .any(|(entry_key, _)| env_key_matches_os(key.as_os_str(), entry_key))
    });
}

fn remove_proxy_env_entries_from_map(map: &mut std::collections::HashMap<OsString, OsString>) {
    map.retain(|key, _| {
        !PROXY_ENV_KEYS
            .iter()
            .any(|entry_key| env_key_matches_os(key.as_os_str(), entry_key))
    });
}

fn with_locus_proxy_env_hidden_unlocked<T>(f: impl FnOnce() -> T) -> T {
    let removed = injected_proxy_env_state()
        .lock()
        .map(|state| {
            let mut removed = Vec::new();
            for (key, value) in &state.entries {
                if std::env::var_os(key)
                    .as_deref()
                    .map(|current| current == value.as_os_str())
                    .unwrap_or(false)
                {
                    std::env::remove_var(key);
                    removed.push((key.clone(), value.clone()));
                }
            }
            removed
        })
        .unwrap_or_default();

    let result = f();

    for (key, value) in removed {
        if std::env::var_os(&key).is_none() {
            std::env::set_var(key, value);
        }
    }

    result
}

fn direct_proxy_matcher() -> Matcher {
    Matcher::builder().build()
}

fn first_user_env_value(keys: &[&str]) -> Option<String> {
    for key in keys {
        let Some(raw_value) = std::env::var_os(key) else {
            continue;
        };
        if is_locus_injected_proxy_env(key, &raw_value) {
            continue;
        }
        let value = raw_value.to_string_lossy().trim().to_string();
        if value.is_empty() {
            continue;
        }
        return Some(value);
    }

    None
}

fn first_proxy_value_from_map(
    map: &std::collections::HashMap<OsString, OsString>,
    keys: &[&str],
) -> Option<String> {
    for key in keys {
        let Some((_, raw_value)) = map
            .iter()
            .find(|(entry_key, _)| env_key_matches_os(entry_key.as_os_str(), key))
        else {
            continue;
        };
        let value = raw_value.to_string_lossy().trim().to_string();
        if value.is_empty() {
            continue;
        }
        return Some(value);
    }

    None
}

fn manual_proxy_matcher(config: &ManualProxyConfig) -> Matcher {
    let mut builder = Matcher::builder();
    if !config.all_proxy.trim().is_empty() {
        builder = builder.all(config.all_proxy.trim());
    }
    if !config.http_proxy.trim().is_empty() {
        builder = builder.http(config.http_proxy.trim());
    }
    if !config.https_proxy.trim().is_empty() {
        builder = builder.https(config.https_proxy.trim());
    }
    let no_proxy = manual_no_proxy_process_value(config);
    if !no_proxy.is_empty() {
        builder = builder.no(no_proxy);
    }
    builder.build()
}

fn system_proxy_enabled(config: &SystemProxyConfig) -> bool {
    config.available && config.enabled.unwrap_or(false)
}

fn system_proxy_for_http(config: &SystemProxyConfig) -> Option<String> {
    if !system_proxy_enabled(config) {
        return None;
    }
    config
        .http_proxy
        .clone()
        .or_else(|| config.socks_proxy.clone())
}

fn system_proxy_for_https(config: &SystemProxyConfig) -> Option<String> {
    if !system_proxy_enabled(config) {
        return None;
    }
    config
        .https_proxy
        .clone()
        .or_else(|| config.socks_proxy.clone())
}

fn auto_proxy_matcher_for_system_config_unlocked(system: &SystemProxyConfig) -> Matcher {
    with_locus_proxy_env_hidden_unlocked(|| {
        let env_all = first_user_env_value(&["ALL_PROXY", "all_proxy"]);
        let env_http = first_user_env_value(&["HTTP_PROXY", "http_proxy"]);
        let env_https = first_user_env_value(&["HTTPS_PROXY", "https_proxy"]);

        let mut builder = Matcher::builder();
        if let Some(value) = env_all.as_deref() {
            builder = builder.all(value);
        }
        let http_proxy = env_http.clone().or_else(|| {
            if env_all.is_none() {
                system_proxy_for_http(system)
            } else {
                None
            }
        });
        if let Some(value) = http_proxy.as_deref() {
            builder = builder.http(value);
        }
        let https_proxy = env_https.clone().or_else(|| {
            if env_all.is_none() {
                system_proxy_for_https(system)
            } else {
                None
            }
        });
        if let Some(value) = https_proxy.as_deref() {
            builder = builder.https(value);
        }

        let no_proxy = auto_no_proxy_process_value(system);
        if !no_proxy.is_empty() {
            builder = builder.no(no_proxy);
        }

        builder.build()
    })
}

fn auto_proxy_matcher_unlocked() -> Matcher {
    let system = read_system_proxy_config();
    auto_proxy_matcher_for_system_config_unlocked(&system)
}

fn proxy_matcher_for_config_unlocked(config: &ProxyConfig) -> Matcher {
    match config.mode {
        ProxyMode::Auto => auto_proxy_matcher_unlocked(),
        ProxyMode::Manual => manual_proxy_matcher(&config.manual),
        ProxyMode::Disabled => direct_proxy_matcher(),
    }
}

fn apply_reqwest_proxy_mode_unlocked(builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
    let config = load_proxy_config();
    match config.mode {
        ProxyMode::Disabled => builder.no_proxy(),
        ProxyMode::Auto | ProxyMode::Manual => {
            let matcher = proxy_matcher_for_config_unlocked(&config);
            builder.proxy(reqwest::Proxy::custom(move |url| {
                let target_uri: Uri = url.as_str().parse().ok()?;
                matcher
                    .intercept(&target_uri)
                    .and_then(|proxy| build_process_proxy_env_url(proxy.uri(), proxy.raw_auth()))
            }))
        }
    }
}

pub fn reqwest_client(options: ReqwestClientOptions) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();

    if let Some(value) = options.connect_timeout {
        builder = builder.connect_timeout(value);
    }
    if let Some(value) = options.timeout {
        builder = builder.timeout(value);
    }
    if let Some(value) = options.tcp_keepalive {
        builder = builder.tcp_keepalive(value);
    }
    if let Some(value) = options.tcp_nodelay {
        builder = builder.tcp_nodelay(value);
    }
    if let Some(value) = options.pool_idle_timeout {
        builder = builder.pool_idle_timeout(value);
    }
    if let Some(value) = options.pool_max_idle_per_host {
        builder = builder.pool_max_idle_per_host(value);
    }
    if let Some(value) = options.http2_adaptive_window {
        builder = builder.http2_adaptive_window(value);
    }
    if let Some(value) = options.http2_keep_alive_interval {
        builder = builder.http2_keep_alive_interval(value);
    }
    if let Some(value) = options.http2_keep_alive_timeout {
        builder = builder.http2_keep_alive_timeout(value);
    }
    if let Some(value) = options.user_agent {
        builder = builder.user_agent(value);
    }
    if let Some(value) = options.gzip {
        builder = builder.gzip(value);
    }
    if let Some(value) = options.deflate {
        builder = builder.deflate(value);
    }

    let builder = {
        let _guard = proxy_env_access_guard();
        apply_reqwest_proxy_mode_unlocked(builder)
    };

    builder
        .build()
        .map_err(|error| format!("Failed to create HTTP client: {}", error))
}

pub fn default_reqwest_client() -> Result<reqwest::Client, String> {
    reqwest_client(ReqwestClientOptions::default())
}

pub fn proxy_matcher() -> Matcher {
    let _guard = proxy_env_access_guard();
    let config = load_proxy_config();
    proxy_matcher_for_config_unlocked(&config)
}

pub fn get_proxy_status() -> ProxyStatus {
    let _guard = proxy_env_access_guard();
    get_proxy_status_unlocked()
}

fn get_proxy_status_unlocked() -> ProxyStatus {
    let config = load_proxy_config();
    let environment = proxy_environment_entries_unlocked();
    let matcher = proxy_matcher_for_config_unlocked(&config);
    let routes = proxy_probe_targets()
        .iter()
        .filter_map(|(label, url)| resolve_proxy_route(label, url, &matcher, &config.mode))
        .collect();

    ProxyStatus {
        mode: config.mode,
        config,
        manual: environment.clone(),
        environment,
        system: read_system_proxy_config(),
        routes,
    }
}

pub fn ensure_proxy_env_for_url(url: &str) -> Result<ResolvedProxyRoute, String> {
    let _guard = proxy_env_access_guard();
    ensure_proxy_env_for_url_unlocked(url)
}

fn ensure_proxy_env_for_url_unlocked(url: &str) -> Result<ResolvedProxyRoute, String> {
    let (route, _) = proxy_env_scope_for_url_unlocked(url)?;
    clear_locus_injected_proxy_env_unlocked();
    Ok(route)
}

pub fn with_proxy_env_for_url<T>(
    url: &str,
    f: impl FnOnce(&ResolvedProxyRoute) -> T,
) -> Result<(ResolvedProxyRoute, T), String> {
    let _guard = proxy_env_access_guard();
    let (route, scope) = proxy_env_scope_for_url_unlocked(url)?;
    let restore = apply_proxy_env_scope_unlocked(&scope);
    let result = f(&route);
    restore_proxy_env_scope_unlocked(restore);
    Ok((route, result))
}

fn proxy_env_scope_for_url_unlocked(
    url: &str,
) -> Result<(ResolvedProxyRoute, ProxyEnvScope), String> {
    let target_uri = system_proxy_match_uri(url)?;
    let config = load_proxy_config();

    match config.mode {
        ProxyMode::Disabled => {
            clear_locus_injected_proxy_env_unlocked();
            Ok((
                ResolvedProxyRoute {
                    proxy_state: "direct".to_string(),
                    proxy_env_key: None,
                    proxy_url: None,
                },
                ProxyEnvScope {
                    clear_user_proxy_env: true,
                    entries: Vec::new(),
                },
            ))
        }
        ProxyMode::Auto => {
            let matcher = proxy_matcher_for_config_unlocked(&config);
            let Some(proxy) = matcher.intercept(&target_uri) else {
                clear_locus_injected_proxy_env_unlocked();
                return Ok((
                    ResolvedProxyRoute {
                        proxy_state: "direct".to_string(),
                        proxy_env_key: None,
                        proxy_url: None,
                    },
                    ProxyEnvScope {
                        clear_user_proxy_env: false,
                        entries: Vec::new(),
                    },
                ));
            };
            if let Some((configured_env_key, configured_proxy)) =
                configured_proxy_env_for_target(&target_uri)
            {
                clear_locus_injected_proxy_env_unlocked();
                return Ok((
                    ResolvedProxyRoute {
                        proxy_state: "environment".to_string(),
                        proxy_env_key: Some(configured_env_key),
                        proxy_url: Some(sanitize_proxy_display_url(&configured_proxy)),
                    },
                    ProxyEnvScope {
                        clear_user_proxy_env: false,
                        entries: Vec::new(),
                    },
                ));
            }

            let env_key = proxy_env_key_for_target(&target_uri).to_string();
            let proxy_display = proxy_display_uri(proxy.uri());
            let Some(proxy_url) = build_supported_ureq_proxy_url(proxy.uri(), proxy.raw_auth())?
            else {
                clear_locus_injected_proxy_env_unlocked();
                return Ok((
                    ResolvedProxyRoute {
                        proxy_state: "system_unsupported".to_string(),
                        proxy_env_key: None,
                        proxy_url: Some(proxy_display),
                    },
                    ProxyEnvScope {
                        clear_user_proxy_env: false,
                        entries: Vec::new(),
                    },
                ));
            };

            let mut entries = vec![
                (env_key.clone(), OsString::from(proxy_url.clone())),
                (env_key.to_ascii_lowercase(), OsString::from(proxy_url)),
            ];
            let system = read_system_proxy_config();
            let no_proxy = auto_no_proxy_process_value(&system);
            if !no_proxy.is_empty() {
                entries.push(("NO_PROXY".to_string(), OsString::from(no_proxy.clone())));
                entries.push(("no_proxy".to_string(), OsString::from(no_proxy)));
            }
            clear_locus_injected_proxy_env_unlocked();
            Ok((
                ResolvedProxyRoute {
                    proxy_state: "system".to_string(),
                    proxy_env_key: Some(env_key),
                    proxy_url: Some(proxy_display),
                },
                ProxyEnvScope {
                    clear_user_proxy_env: false,
                    entries,
                },
            ))
        }
        ProxyMode::Manual => {
            let matcher = proxy_matcher_for_config_unlocked(&config);
            let Some(proxy) = matcher.intercept(&target_uri) else {
                clear_locus_injected_proxy_env_unlocked();
                return Ok((
                    ResolvedProxyRoute {
                        proxy_state: "direct".to_string(),
                        proxy_env_key: None,
                        proxy_url: None,
                    },
                    ProxyEnvScope {
                        clear_user_proxy_env: true,
                        entries: Vec::new(),
                    },
                ));
            };

            let env_key = proxy_env_key_for_target(&target_uri).to_string();
            let proxy_display = proxy_display_uri(proxy.uri());
            let Some(proxy_url) = build_supported_ureq_proxy_url(proxy.uri(), proxy.raw_auth())?
            else {
                clear_locus_injected_proxy_env_unlocked();
                return Ok((
                    ResolvedProxyRoute {
                        proxy_state: "manual_unsupported".to_string(),
                        proxy_env_key: None,
                        proxy_url: Some(proxy_display),
                    },
                    ProxyEnvScope {
                        clear_user_proxy_env: true,
                        entries: Vec::new(),
                    },
                ));
            };

            let mut entries = vec![
                (env_key.clone(), OsString::from(proxy_url.clone())),
                (env_key.to_ascii_lowercase(), OsString::from(proxy_url)),
            ];
            let no_proxy = manual_no_proxy_process_value(&config.manual);
            if !no_proxy.is_empty() {
                entries.push(("NO_PROXY".to_string(), OsString::from(no_proxy.clone())));
                entries.push(("no_proxy".to_string(), OsString::from(no_proxy)));
            }
            clear_locus_injected_proxy_env_unlocked();
            Ok((
                ResolvedProxyRoute {
                    proxy_state: "manual".to_string(),
                    proxy_env_key: Some(env_key),
                    proxy_url: Some(proxy_display),
                },
                ProxyEnvScope {
                    clear_user_proxy_env: true,
                    entries,
                },
            ))
        }
    }
}

fn push_unique_env_key(keys: &mut Vec<String>, key: &str) {
    if !keys.iter().any(|existing| env_key_matches(existing, key)) {
        keys.push(key.to_string());
    }
}

fn apply_proxy_env_scope_unlocked(scope: &ProxyEnvScope) -> Vec<(String, Option<OsString>)> {
    let mut keys = Vec::new();
    if scope.clear_user_proxy_env {
        for key in PROXY_ENV_KEYS {
            push_unique_env_key(&mut keys, key);
        }
    }
    for (key, _) in &scope.entries {
        push_unique_env_key(&mut keys, key);
    }

    let restore = keys
        .iter()
        .map(|key| (key.clone(), std::env::var_os(key)))
        .collect::<Vec<_>>();

    if scope.clear_user_proxy_env {
        for key in PROXY_ENV_KEYS {
            std::env::remove_var(key);
        }
    }
    for (key, value) in &scope.entries {
        if value.is_empty() {
            std::env::remove_var(key);
        } else {
            std::env::set_var(key, value);
        }
    }

    restore
}

fn restore_proxy_env_scope_unlocked(restore: Vec<(String, Option<OsString>)>) {
    for (key, value) in restore.into_iter().rev() {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

pub fn apply_proxy_env_to_command(cmd: &mut std::process::Command) {
    let _guard = proxy_env_access_guard();
    for key in locus_injected_proxy_env_keys_to_remove() {
        cmd.env_remove(key);
    }
    let config = load_proxy_config();
    match config.mode {
        ProxyMode::Auto => {
            for (key, value) in auto_proxy_process_env_entries_unlocked() {
                cmd.env(key, value);
            }
        }
        ProxyMode::Disabled => {
            for key in PROXY_ENV_KEYS {
                cmd.env_remove(key);
            }
        }
        ProxyMode::Manual => {
            for key in PROXY_ENV_KEYS {
                cmd.env_remove(key);
            }
            for (key, value) in proxy_process_env_entries_unlocked(&config) {
                cmd.env(key, value);
            }
        }
    }
}

pub fn apply_proxy_env_to_async_command(cmd: &mut tokio::process::Command) {
    let _guard = proxy_env_access_guard();
    for key in locus_injected_proxy_env_keys_to_remove() {
        cmd.env_remove(key);
    }
    let config = load_proxy_config();
    match config.mode {
        ProxyMode::Auto => {
            for (key, value) in auto_proxy_process_env_entries_unlocked() {
                cmd.env(key, value);
            }
        }
        ProxyMode::Disabled => {
            for key in PROXY_ENV_KEYS {
                cmd.env_remove(key);
            }
        }
        ProxyMode::Manual => {
            for key in PROXY_ENV_KEYS {
                cmd.env_remove(key);
            }
            for (key, value) in proxy_process_env_entries_unlocked(&config) {
                cmd.env(key, value);
            }
        }
    }
}

pub fn extend_proxy_env_map(map: &mut std::collections::HashMap<OsString, OsString>) {
    let _guard = proxy_env_access_guard();
    remove_locus_injected_proxy_entries_from_map(map);
    let config = load_proxy_config();
    match config.mode {
        ProxyMode::Auto => {
            for (key, value) in auto_proxy_process_env_entries_for_map_unlocked(map) {
                map.insert(OsString::from(key), value);
            }
        }
        ProxyMode::Disabled => remove_proxy_env_entries_from_map(map),
        ProxyMode::Manual => {
            remove_proxy_env_entries_from_map(map);
            for (key, value) in proxy_process_env_entries_unlocked(&config) {
                map.insert(OsString::from(key), value);
            }
        }
    }
}

pub fn proxy_display_uri(uri: &Uri) -> String {
    let scheme = uri.scheme_str().unwrap_or("http");
    let host = uri.host().unwrap_or("<missing-host>");
    match uri.port_u16() {
        Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
        None => format!("{}://{}", scheme, authority_host(host)),
    }
}

pub fn sanitize_proxy_display_url(raw: &str) -> String {
    if let Some(url) = parse_proxy_url(raw) {
        let scheme = url.scheme();
        let host = url.host_str().unwrap_or("<missing-host>");
        return match url.port_or_known_default() {
            Some(port) => format!("{}://{}:{}", scheme, authority_host(host), port),
            None => format!("{}://{}", scheme, authority_host(host)),
        };
    }

    if let Ok(uri) = raw.parse::<Uri>() {
        return proxy_display_uri(&uri);
    }

    if let Some((scheme, remainder)) = raw.split_once("://") {
        if let Some((_, host_part)) = remainder.rsplit_once('@') {
            return format!("{}://{}", scheme, host_part);
        }
    }

    raw.to_string()
}

pub(crate) fn build_supported_ureq_proxy_url(
    proxy_uri: &Uri,
    auth: Option<(&str, &str)>,
) -> Result<Option<String>, String> {
    let scheme = match proxy_uri.scheme_str() {
        Some("http") => "http",
        Some("socks4") => "socks4",
        Some("socks4a") => "socks4a",
        Some("socks5") | Some("socks5h") => "socks5",
        Some("https") => return Ok(None),
        Some(other) => {
            return Err(format!(
                "Unsupported system proxy scheme for knowledge download: {}",
                other
            ));
        }
        None => return Err("System proxy URI is missing a scheme".to_string()),
    };

    build_proxy_url(proxy_uri, auth, scheme).map(Some)
}

fn proxy_process_env_entries_unlocked(config: &ProxyConfig) -> Vec<(String, OsString)> {
    if config.mode != ProxyMode::Manual {
        return Vec::new();
    }

    let mut entries: Vec<(String, OsString)> = Vec::new();
    push_manual_proxy_env_entry(
        &mut entries,
        "HTTP_PROXY",
        "http_proxy",
        &config.manual.http_proxy,
    );
    push_manual_proxy_env_entry(
        &mut entries,
        "HTTPS_PROXY",
        "https_proxy",
        &config.manual.https_proxy,
    );
    push_manual_proxy_env_entry(
        &mut entries,
        "ALL_PROXY",
        "all_proxy",
        &config.manual.all_proxy,
    );

    if !entries.is_empty() {
        let no_proxy = manual_no_proxy_process_value(&config.manual);
        if !no_proxy.is_empty() {
            entries.push(("NO_PROXY".to_string(), OsString::from(no_proxy.clone())));
            entries.push(("no_proxy".to_string(), OsString::from(no_proxy)));
        }
    }

    entries
}

fn system_proxy_process_env_entries(
    system: &SystemProxyConfig,
    has_all_proxy: bool,
    has_http_proxy: bool,
    has_https_proxy: bool,
) -> Vec<(String, OsString)> {
    if !system_proxy_enabled(system) {
        return Vec::new();
    }

    let mut entries: Vec<(String, OsString)> = Vec::new();
    if !has_all_proxy && !has_http_proxy {
        if let Some(value) = system_proxy_for_http(system) {
            push_manual_proxy_env_entry(&mut entries, "HTTP_PROXY", "http_proxy", &value);
        }
    }
    if !has_all_proxy && !has_https_proxy {
        if let Some(value) = system_proxy_for_https(system) {
            push_manual_proxy_env_entry(&mut entries, "HTTPS_PROXY", "https_proxy", &value);
        }
    }

    if !entries.is_empty() {
        let no_proxy = auto_no_proxy_process_value(system);
        if !no_proxy.is_empty() {
            entries.push(("NO_PROXY".to_string(), OsString::from(no_proxy.clone())));
            entries.push(("no_proxy".to_string(), OsString::from(no_proxy)));
        }
    }

    entries
}

fn auto_proxy_process_env_entries_unlocked() -> Vec<(String, OsString)> {
    let system = read_system_proxy_config();
    system_proxy_process_env_entries(
        &system,
        first_user_env_value(&["ALL_PROXY", "all_proxy"]).is_some(),
        first_user_env_value(&["HTTP_PROXY", "http_proxy"]).is_some(),
        first_user_env_value(&["HTTPS_PROXY", "https_proxy"]).is_some(),
    )
}

fn auto_proxy_process_env_entries_for_map_unlocked(
    map: &std::collections::HashMap<OsString, OsString>,
) -> Vec<(String, OsString)> {
    let system = read_system_proxy_config();
    system_proxy_process_env_entries(
        &system,
        first_proxy_value_from_map(map, &["ALL_PROXY", "all_proxy"]).is_some(),
        first_proxy_value_from_map(map, &["HTTP_PROXY", "http_proxy"]).is_some(),
        first_proxy_value_from_map(map, &["HTTPS_PROXY", "https_proxy"]).is_some(),
    )
}

fn push_manual_proxy_env_entry(
    entries: &mut Vec<(String, OsString)>,
    upper_key: &str,
    lower_key: &str,
    value: &str,
) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    entries.push((upper_key.to_string(), OsString::from(value)));
    entries.push((lower_key.to_string(), OsString::from(value)));
}

fn build_process_proxy_env_url(proxy_uri: &Uri, auth: Option<(&str, &str)>) -> Option<String> {
    let scheme = match proxy_uri.scheme_str()? {
        "http" => "http",
        "https" => "https",
        "socks4" => "socks4",
        "socks4a" => "socks4a",
        "socks5" => "socks5",
        "socks5h" => "socks5h",
        _ => return None,
    };

    build_proxy_url(proxy_uri, auth, scheme).ok()
}

fn build_proxy_url(
    proxy_uri: &Uri,
    auth: Option<(&str, &str)>,
    scheme: &str,
) -> Result<String, String> {
    let host = proxy_uri
        .host()
        .ok_or_else(|| "System proxy URI is missing host".to_string())?;
    let port = proxy_uri.port_u16().unwrap_or(match scheme {
        "http" => 80,
        "https" => 443,
        "socks4" | "socks4a" | "socks5" | "socks5h" => 1080,
        _ => 80,
    });
    let auth_prefix = auth
        .map(|(user, pass)| format!("{user}:{pass}@"))
        .unwrap_or_default();

    Ok(format!(
        "{}://{}{}:{}",
        scheme,
        auth_prefix,
        authority_host(host),
        port
    ))
}

fn proxy_environment_entries_unlocked() -> Vec<ProxyEnvironmentEntry> {
    [
        ("HTTP_PROXY", ProxyEnvironmentEntryKind::Proxy),
        ("http_proxy", ProxyEnvironmentEntryKind::Proxy),
        ("HTTPS_PROXY", ProxyEnvironmentEntryKind::Proxy),
        ("https_proxy", ProxyEnvironmentEntryKind::Proxy),
        ("ALL_PROXY", ProxyEnvironmentEntryKind::Proxy),
        ("all_proxy", ProxyEnvironmentEntryKind::Proxy),
        ("NO_PROXY", ProxyEnvironmentEntryKind::Bypass),
        ("no_proxy", ProxyEnvironmentEntryKind::Bypass),
    ]
    .into_iter()
    .filter_map(|(key, kind)| {
        let raw_value = std::env::var_os(key)?;
        if is_locus_injected_proxy_env(key, &raw_value) {
            return None;
        }
        let value = raw_value.to_string_lossy().trim().to_string();
        if value.is_empty() {
            return None;
        }
        Some(ProxyEnvironmentEntry {
            key: key.to_string(),
            value: match kind {
                ProxyEnvironmentEntryKind::Proxy => sanitize_proxy_display_url(&value),
                ProxyEnvironmentEntryKind::Bypass => value,
            },
            kind,
        })
    })
    .collect()
}

#[cfg(test)]
fn has_configured_proxy_env_unlocked() -> bool {
    proxy_environment_entries_unlocked()
        .iter()
        .any(|entry| entry.kind == ProxyEnvironmentEntryKind::Proxy)
}

#[cfg(test)]
fn has_configured_proxy_env() -> bool {
    let _guard = proxy_env_access_guard();
    has_configured_proxy_env_unlocked()
}

fn proxy_probe_targets() -> [(&'static str, &'static str); 2] {
    [
        ("HTTPS", "https://api.openai.com/"),
        ("HTTP", "http://example.com/"),
    ]
}

fn resolve_proxy_route(
    label: &str,
    url: &str,
    matcher: &Matcher,
    mode: &ProxyMode,
) -> Option<ProxyRoute> {
    let target: Uri = url.parse().ok()?;
    let intercept = matcher.intercept(&target);
    let source = match (&intercept, mode) {
        (Some(_), ProxyMode::Auto) => {
            if configured_proxy_env_for_target(&target).is_some() {
                ProxyRouteSource::Environment
            } else {
                ProxyRouteSource::System
            }
        }
        (Some(_), ProxyMode::Manual) => ProxyRouteSource::Manual,
        (None, _) => ProxyRouteSource::Direct,
        (Some(_), ProxyMode::Disabled) => ProxyRouteSource::Direct,
    };

    Some(ProxyRoute {
        target_label: label.to_string(),
        target_url: url.to_string(),
        proxy_url: intercept.map(|proxy| proxy_display_uri(proxy.uri())),
        source,
    })
}

fn system_proxy_match_uri(url: &str) -> Result<Uri, String> {
    let parsed: Uri = url
        .parse()
        .map_err(|e| format!("Failed to parse proxy target URL '{}': {}", url, e))?;
    let scheme = match parsed.scheme_str() {
        Some("http") | Some("https") => parsed
            .scheme_str()
            .ok_or_else(|| "Proxy target URL is missing a scheme".to_string())?,
        Some(other) => {
            return Err(format!(
                "Unsupported proxy target URL scheme '{}': {}",
                other, url
            ));
        }
        None => return Err(format!("Proxy target URL is missing a scheme: {}", url)),
    };
    let authority = parsed
        .authority()
        .cloned()
        .ok_or_else(|| format!("Proxy target URL is missing an authority: {}", url))?;
    let path_and_query = parsed
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| http::uri::PathAndQuery::from_static("/"));

    Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| format!("Failed to normalize proxy target URL '{}': {}", url, e))
}

fn proxy_env_key_for_target(target_uri: &Uri) -> &'static str {
    match target_uri.scheme_str() {
        Some("http") => "HTTP_PROXY",
        _ => "HTTPS_PROXY",
    }
}

fn configured_proxy_env_for_target(target_uri: &Uri) -> Option<(String, String)> {
    let env_keys = match target_uri.scheme_str() {
        Some("http") => ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"],
        _ => ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"],
    };

    for key in env_keys {
        let Some(value) = std::env::var_os(key) else {
            continue;
        };
        if is_locus_injected_proxy_env(key, &value) {
            continue;
        }
        let value = value.to_string_lossy().trim().to_string();
        if value.is_empty() {
            continue;
        }
        return Some((key.to_string(), value));
    }

    None
}

fn user_bypass_env_values() -> Vec<String> {
    ["NO_PROXY", "no_proxy"]
        .into_iter()
        .filter_map(|key| {
            let raw = std::env::var_os(key)?;
            if is_locus_injected_proxy_env(key, &raw) {
                return None;
            }
            let value = raw.to_string_lossy().trim().to_string();
            (!value.is_empty()).then_some(value)
        })
        .collect()
}

fn push_bypass_entries(entries: &mut Vec<String>, raw: &str, separators: &[char]) {
    for entry in raw
        .split(|ch| separators.contains(&ch))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        entries.push(entry.to_string());
    }
}

fn push_windows_proxy_override_entries(entries: &mut Vec<String>, raw: &str) {
    for entry in raw
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if entry.eq_ignore_ascii_case("<local>") {
            continue;
        }
        entries.push(entry.replace("*.", ""));
    }
}

fn manual_no_proxy_process_value(config: &ManualProxyConfig) -> String {
    let mut entries = Vec::new();
    push_bypass_entries(&mut entries, &config.no_proxy, &[',']);
    push_bypass_entries(&mut entries, LOCAL_PROXY_BYPASS, &[',']);
    entries.sort();
    entries.dedup();
    entries.join(",")
}

fn auto_no_proxy_process_value(system: &SystemProxyConfig) -> String {
    let mut entries = Vec::new();

    for raw in user_bypass_env_values() {
        push_bypass_entries(&mut entries, &raw, &[',']);
    }

    push_bypass_entries(&mut entries, LOCAL_PROXY_BYPASS, &[',']);

    if system_proxy_enabled(system) {
        if let Some(raw) = system.proxy_override.as_deref() {
            push_windows_proxy_override_entries(&mut entries, raw);
        }
    }

    entries.sort();
    entries.dedup();
    entries.join(",")
}

#[cfg(test)]
fn no_proxy_process_value() -> String {
    let system = read_system_proxy_config();
    auto_no_proxy_process_value(&system)
}

fn sanitize_windows_proxy_server(raw: &str) -> String {
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if let Some((scheme, value)) = segment.split_once('=') {
                let default_scheme = default_windows_proxy_value_scheme(scheme.trim());
                format!(
                    "{}={}",
                    scheme.trim(),
                    sanitize_proxy_value_with_default_scheme(value.trim(), default_scheme)
                )
            } else {
                sanitize_proxy_value_with_default_scheme(segment, "http")
            }
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn default_windows_proxy_value_scheme(entry_scheme: &str) -> &'static str {
    match entry_scheme.to_ascii_lowercase().as_str() {
        "socks" | "socks4" | "socks5" => "socks5",
        _ => "http",
    }
}

fn sanitize_proxy_value_with_default_scheme(raw: &str, default_scheme: &str) -> String {
    let value = raw.trim();
    if value.is_empty() {
        return String::new();
    }

    if value.contains("://") {
        sanitize_proxy_display_url(value)
    } else {
        sanitize_proxy_display_url(&format!("{}://{}", default_scheme, value))
    }
}

fn parse_windows_proxy_server(raw: &str) -> (Option<String>, Option<String>, Option<String>) {
    let mut http_proxy = None;
    let mut https_proxy = None;
    let mut socks_proxy = None;
    let mut has_protocol_entries = false;

    for segment in raw
        .split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        let Some((scheme, value)) = segment.split_once('=') else {
            continue;
        };
        has_protocol_entries = true;
        match scheme.trim().to_ascii_lowercase().as_str() {
            "http" => {
                http_proxy = Some(sanitize_proxy_value_with_default_scheme(
                    value.trim(),
                    "http",
                ));
            }
            "https" => {
                https_proxy = Some(sanitize_proxy_value_with_default_scheme(
                    value.trim(),
                    "http",
                ));
            }
            "socks" | "socks4" | "socks5" => {
                socks_proxy = Some(sanitize_proxy_value_with_default_scheme(
                    value.trim(),
                    default_windows_proxy_value_scheme(scheme.trim()),
                ));
            }
            _ => {}
        }
    }

    if !has_protocol_entries {
        let sanitized = sanitize_proxy_value_with_default_scheme(raw.trim(), "http");
        if !sanitized.is_empty() {
            http_proxy = Some(sanitized.clone());
            https_proxy = Some(sanitized);
        }
    }

    (http_proxy, https_proxy, socks_proxy)
}

fn parse_proxy_url(raw: &str) -> Option<Url> {
    Url::parse(raw)
        .ok()
        .or_else(|| Url::parse(&format!("http://{}", raw)).ok())
}

fn authority_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

#[cfg(windows)]
fn read_system_proxy_config() -> SystemProxyConfig {
    #[cfg(test)]
    if let Some(config) = system_proxy_config_override_value() {
        return config;
    }

    read_winhttp_current_user_ie_proxy_config()
        .unwrap_or_else(|| read_windows_registry_proxy_config("windowsRegistryFallback"))
}

#[cfg(windows)]
fn read_winhttp_current_user_ie_proxy_config() -> Option<SystemProxyConfig> {
    use windows::Win32::Networking::WinHttp::{
        WinHttpGetIEProxyConfigForCurrentUser, WINHTTP_CURRENT_USER_IE_PROXY_CONFIG,
    };

    let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    unsafe {
        WinHttpGetIEProxyConfigForCurrentUser(&mut config).ok()?;
    }

    let proxy_server_raw = windows_owned_pwstr_to_string(config.lpszProxy);
    let proxy_override = windows_owned_pwstr_to_string(config.lpszProxyBypass);
    let auto_config_url = windows_owned_pwstr_to_string(config.lpszAutoConfigUrl);
    let (http_proxy, https_proxy, socks_proxy) = proxy_server_raw
        .as_deref()
        .map(parse_windows_proxy_server)
        .unwrap_or((None, None, None));

    Some(SystemProxyConfig {
        platform: std::env::consts::OS.to_string(),
        available: true,
        source: "winHttpCurrentUserIE".to_string(),
        enabled: Some(proxy_server_raw.is_some()),
        auto_detect: Some(config.fAutoDetect.as_bool()),
        auto_config_url,
        proxy_server: proxy_server_raw
            .as_deref()
            .map(sanitize_windows_proxy_server),
        proxy_override,
        http_proxy,
        https_proxy,
        socks_proxy,
    })
}

#[cfg(windows)]
fn windows_owned_pwstr_to_string(value: windows::core::PWSTR) -> Option<String> {
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};

    if value.is_null() {
        return None;
    }

    let text = unsafe { value.to_string().ok() }
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    unsafe {
        let _ = GlobalFree(Some(HGLOBAL(value.as_ptr().cast())));
    }
    text
}

#[cfg(windows)]
fn read_windows_registry_proxy_config(source: &str) -> SystemProxyConfig {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let platform = std::env::consts::OS.to_string();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let settings =
        match hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings") {
            Ok(settings) => settings,
            Err(_) => {
                return SystemProxyConfig {
                    platform,
                    available: false,
                    source: source.to_string(),
                    enabled: None,
                    auto_detect: None,
                    auto_config_url: None,
                    proxy_server: None,
                    proxy_override: None,
                    http_proxy: None,
                    https_proxy: None,
                    socks_proxy: None,
                };
            }
        };

    let enabled = settings
        .get_value::<u32, _>("ProxyEnable")
        .ok()
        .map(|value| value != 0);
    let auto_detect = settings
        .get_value::<u32, _>("AutoDetect")
        .ok()
        .map(|value| value != 0);
    let auto_config_url = settings
        .get_value::<String, _>("AutoConfigURL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let proxy_server_raw = settings
        .get_value::<String, _>("ProxyServer")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let proxy_override = settings
        .get_value::<String, _>("ProxyOverride")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let (http_proxy, https_proxy, socks_proxy) = proxy_server_raw
        .as_deref()
        .map(parse_windows_proxy_server)
        .unwrap_or((None, None, None));

    SystemProxyConfig {
        platform,
        available: true,
        source: source.to_string(),
        enabled,
        auto_detect,
        auto_config_url,
        proxy_server: proxy_server_raw
            .as_deref()
            .map(sanitize_windows_proxy_server),
        proxy_override,
        http_proxy,
        https_proxy,
        socks_proxy,
    }
}

#[cfg(not(windows))]
fn read_system_proxy_config() -> SystemProxyConfig {
    #[cfg(test)]
    if let Some(config) = system_proxy_config_override_value() {
        return config;
    }

    SystemProxyConfig {
        platform: std::env::consts::OS.to_string(),
        available: false,
        source: "systemProxyMatcher".to_string(),
        enabled: None,
        auto_detect: None,
        auto_config_url: None,
        proxy_server: None,
        proxy_override: None,
        http_proxy: None,
        https_proxy: None,
        socks_proxy: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn env_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env test lock")
    }

    struct TempEnvGuard {
        saved: Vec<(String, Option<OsString>)>,
        saved_config: Option<ProxyConfig>,
        saved_system_config: Option<SystemProxyConfig>,
    }

    impl TempEnvGuard {
        fn set(vars: &[(&str, Option<&str>)]) -> Self {
            clear_locus_injected_proxy_env();
            let saved_config = proxy_config_override()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            set_proxy_config_override(Some(ProxyConfig::default()));
            let saved_system_config = system_proxy_config_override()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            set_system_proxy_config_override(Some(test_system_proxy_config(
                false, None, None, None, None, None,
            )));
            let saved = vars
                .iter()
                .map(|(key, _)| ((*key).to_string(), std::env::var_os(key)))
                .collect::<Vec<_>>();
            for (key, value) in vars {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            Self {
                saved,
                saved_config,
                saved_system_config,
            }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            clear_locus_injected_proxy_env();
            set_proxy_config_override(self.saved_config.clone());
            set_system_proxy_config_override(self.saved_system_config.clone());
            for (key, value) in self.saved.iter().rev() {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn test_system_proxy_config(
        enabled: bool,
        proxy_server: Option<&str>,
        http_proxy: Option<&str>,
        https_proxy: Option<&str>,
        socks_proxy: Option<&str>,
        proxy_override: Option<&str>,
    ) -> SystemProxyConfig {
        SystemProxyConfig {
            platform: std::env::consts::OS.to_string(),
            available: true,
            source: "test".to_string(),
            enabled: Some(enabled),
            auto_detect: Some(false),
            auto_config_url: None,
            proxy_server: proxy_server.map(str::to_string),
            proxy_override: proxy_override.map(str::to_string),
            http_proxy: http_proxy.map(str::to_string),
            https_proxy: https_proxy.map(str::to_string),
            socks_proxy: socks_proxy.map(str::to_string),
        }
    }

    #[test]
    fn sanitize_proxy_display_url_hides_credentials() {
        let display = sanitize_proxy_display_url("http://user:pass@127.0.0.1:7890");
        assert_eq!(display, "http://127.0.0.1:7890");
    }

    #[test]
    fn build_supported_ureq_proxy_url_maps_http_proxy() {
        let proxy_uri: Uri = "http://127.0.0.1:7890".parse().expect("proxy uri");
        let proxy_url =
            build_supported_ureq_proxy_url(&proxy_uri, Some(("user", "pass"))).expect("proxy");
        assert_eq!(
            proxy_url.as_deref(),
            Some("http://user:pass@127.0.0.1:7890")
        );
    }

    #[test]
    fn build_supported_ureq_proxy_url_skips_https_proxy_scheme() {
        let proxy_uri: Uri = "https://127.0.0.1:8443".parse().expect("proxy uri");
        let proxy_url = build_supported_ureq_proxy_url(&proxy_uri, None).expect("proxy");
        assert_eq!(proxy_url, None);
    }

    #[test]
    fn build_supported_ureq_proxy_url_maps_socks5h_proxy() {
        let proxy_uri: Uri = "socks5h://127.0.0.1:1080".parse().expect("proxy uri");
        let proxy_url = build_supported_ureq_proxy_url(&proxy_uri, None).expect("proxy");
        assert_eq!(proxy_url.as_deref(), Some("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn parse_windows_proxy_server_defaults_single_value_to_http_proxy() {
        let (http_proxy, https_proxy, socks_proxy) = parse_windows_proxy_server("127.0.0.1:7890");

        assert_eq!(http_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(https_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(socks_proxy, None);
    }

    #[test]
    fn parse_windows_proxy_server_preserves_protocol_specific_entries() {
        let (http_proxy, https_proxy, socks_proxy) = parse_windows_proxy_server(
            "http=127.0.0.1:7890;https=127.0.0.1:7891;socks=127.0.0.1:1080",
        );

        assert_eq!(http_proxy.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(https_proxy.as_deref(), Some("http://127.0.0.1:7891"));
        assert_eq!(socks_proxy.as_deref(), Some("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn configured_proxy_env_ignores_locus_injected_values() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("HTTPS_PROXY", None),
            ("https_proxy", None),
            ("ALL_PROXY", None),
            ("all_proxy", None),
        ]);
        replace_locus_injected_proxy_env(vec![(
            "HTTPS_PROXY".to_string(),
            OsString::from("http://127.0.0.1:65500"),
        )]);
        let target: Uri = "https://api.openai.com/".parse().expect("target uri");

        assert_eq!(configured_proxy_env_for_target(&target), None);
        assert!(!has_configured_proxy_env());
    }

    #[test]
    fn auto_proxy_mode_uses_system_proxy_when_environment_is_empty() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("HTTPS_PROXY", None),
            ("https_proxy", None),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
        ]);
        set_system_proxy_config_override(Some(test_system_proxy_config(
            true,
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            None,
            Some("localhost;127.*;<local>"),
        )));

        let status = get_proxy_status();
        let https_route = status
            .routes
            .iter()
            .find(|route| route.target_label == "HTTPS")
            .expect("https route");
        assert_eq!(https_route.source, ProxyRouteSource::System);
        assert_eq!(
            https_route.proxy_url.as_deref(),
            Some("http://127.0.0.1:7890")
        );

        let (route, scoped_proxy) = with_proxy_env_for_url("https://api.openai.com/", |_| {
            (
                std::env::var("HTTPS_PROXY").ok(),
                std::env::var("NO_PROXY").ok(),
            )
        })
        .expect("proxy scope");

        assert_eq!(route.proxy_state, "system");
        assert_eq!(route.proxy_env_key.as_deref(), Some("HTTPS_PROXY"));
        assert_eq!(route.proxy_url.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(scoped_proxy.0.as_deref(), Some("http://127.0.0.1:7890"));
        assert!(scoped_proxy
            .1
            .as_deref()
            .unwrap_or_default()
            .split(',')
            .any(|entry| entry == "localhost"));
        assert_eq!(std::env::var("HTTPS_PROXY").ok(), None);
    }

    #[test]
    fn auto_proxy_mode_prefers_environment_proxy_over_system_proxy() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("https_proxy", None),
            ("HTTPS_PROXY", Some("http://127.0.0.1:65501")),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
        ]);
        set_system_proxy_config_override(Some(test_system_proxy_config(
            true,
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            None,
            None,
        )));

        let (route, scoped_proxy) = with_proxy_env_for_url("https://api.openai.com/", |_| {
            std::env::var("HTTPS_PROXY").ok()
        })
        .expect("proxy scope");

        assert_eq!(route.proxy_state, "environment");
        assert_eq!(route.proxy_env_key.as_deref(), Some("HTTPS_PROXY"));
        assert_eq!(route.proxy_url.as_deref(), Some("http://127.0.0.1:65501"));
        assert_eq!(scoped_proxy.as_deref(), Some("http://127.0.0.1:65501"));
    }

    #[test]
    fn extend_proxy_env_map_adds_system_proxy_for_auto_mode() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("HTTPS_PROXY", None),
            ("https_proxy", None),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
        ]);
        set_system_proxy_config_override(Some(test_system_proxy_config(
            true,
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            Some("http://127.0.0.1:7890"),
            None,
            Some("corp.local;<local>"),
        )));
        let mut envs: HashMap<OsString, OsString> = HashMap::new();

        extend_proxy_env_map(&mut envs);

        assert_eq!(
            envs.get(&OsString::from("HTTPS_PROXY"))
                .and_then(|value| value.to_str()),
            Some("http://127.0.0.1:7890")
        );
        assert!(envs
            .get(&OsString::from("NO_PROXY"))
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .split(',')
            .any(|entry| entry == "corp.local"));
    }

    #[test]
    fn extend_proxy_env_map_removes_stale_locus_values() {
        let _lock = env_test_lock();
        let stale_proxy = OsString::from("http://127.0.0.1:65500");
        let _guard = TempEnvGuard::set(&[
            ("HTTP_PROXY", None),
            ("http_proxy", None),
            ("HTTPS_PROXY", None),
            ("https_proxy", None),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
            ("REQUEST_METHOD", Some("GET")),
        ]);
        replace_locus_injected_proxy_env(vec![("HTTPS_PROXY".to_string(), stale_proxy.clone())]);
        let mut envs: HashMap<OsString, OsString> = std::env::vars_os().collect();

        extend_proxy_env_map(&mut envs);

        assert_ne!(envs.get(&OsString::from("HTTPS_PROXY")), Some(&stale_proxy));
    }

    #[test]
    fn manual_proxy_mode_overrides_environment_inside_scope_and_restores() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTPS_PROXY", Some("http://127.0.0.1:65501")),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
        ]);
        set_proxy_config_override(Some(ProxyConfig {
            mode: ProxyMode::Manual,
            manual: ManualProxyConfig {
                https_proxy: "http://127.0.0.1:65502".to_string(),
                ..ManualProxyConfig::default()
            },
        }));

        let (route, scoped_proxy) = with_proxy_env_for_url("https://api.openai.com/", |_| {
            std::env::var("HTTPS_PROXY").ok()
        })
        .expect("proxy scope");

        assert_eq!(route.proxy_state, "manual");
        assert_eq!(route.proxy_url.as_deref(), Some("http://127.0.0.1:65502"));
        assert_eq!(scoped_proxy.as_deref(), Some("http://127.0.0.1:65502"));
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:65501")
        );
    }

    #[test]
    fn disabled_proxy_mode_hides_environment_inside_scope_and_restores() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[
            ("HTTPS_PROXY", Some("http://127.0.0.1:65503")),
            ("ALL_PROXY", None),
            ("all_proxy", None),
            ("NO_PROXY", None),
            ("no_proxy", None),
        ]);
        set_proxy_config_override(Some(ProxyConfig {
            mode: ProxyMode::Disabled,
            manual: ManualProxyConfig::default(),
        }));

        let (route, scoped_proxy) = with_proxy_env_for_url("https://api.openai.com/", |_| {
            std::env::var("HTTPS_PROXY").ok()
        })
        .expect("proxy scope");

        assert_eq!(route.proxy_state, "direct");
        assert_eq!(scoped_proxy, None);
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:65503")
        );
    }

    #[test]
    fn no_proxy_process_value_preserves_user_bypass_entries() {
        let _lock = env_test_lock();
        let _guard = TempEnvGuard::set(&[("NO_PROXY", Some("corp.local, 10.0.0.0/8"))]);

        let no_proxy = no_proxy_process_value();

        assert!(no_proxy.split(',').any(|entry| entry == "corp.local"));
        assert!(no_proxy.split(',').any(|entry| entry == "10.0.0.0/8"));
        assert!(no_proxy.split(',').any(|entry| entry == "localhost"));
        assert!(no_proxy.split(',').any(|entry| entry == "127.0.0.1"));
    }
}
