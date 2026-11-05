//! Internal-client transport and credential boundary. Never compiled in public releases.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Mutex;
use zeroize::Zeroizing;

static AUTH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LOGIN_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User { pub id: String, pub username: String, pub role: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub id: String,
    pub server_url: String,
    pub user_id: String,
    pub upstream_id: String,
    pub kind: String,
    pub name: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    #[serde(default)] version: u32,
    active: Option<Account>,
    #[serde(default)] bindings: BTreeMap<String, Binding>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Account { server_url: String, user: User }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tokens { access_token: String, refresh_token: String, expires_at: i64, user: User }

#[derive(Deserialize)]
struct Upstream { id: String, name: String, kind: String, #[serde(default)] enabled: bool }
#[derive(Deserialize)]
struct Me { user: User, upstreams: Vec<Upstream> }

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub server_url: Option<String>, pub user: Option<User>, pub upstreams: Vec<Binding>,
    pub server_revocation_confirmed: Option<bool>,
}

pub struct Connection {
    pub server_url: String,
    pub access_token: String,
    pub upstream_id: String,
    pub user_id: String,
    pub binding_id: String,
}

fn config_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::commands::persistent_config_dir()?.join("server_client.json"))
}
fn load_config() -> Result<Config, String> {
    match std::fs::read(config_path()?) {
        Ok(bytes) => {
            let config: Config = serde_json::from_slice(&bytes).map_err(|_| "服务器连接配置损坏")?;
            if config.version != 1 { return Err("不支持的服务器连接配置版本".into()); }
            Ok(config)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config { version: 1, ..Default::default() }),
        Err(_) => Err("无法读取服务器连接配置".into()),
    }
}
fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path()?;
    crate::config::atomic_write_config(&path, &serde_json::to_vec(config).map_err(|_| "无法序列化连接配置")?)
}

pub fn normalize_origin(raw: &str) -> Result<String, String> {
    let url = url::Url::parse(raw.trim()).map_err(|_| "请输入完整的 HTTPS 服务器地址")?;
    let local_http = url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if (url.scheme() != "https" && !local_http) || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
        return Err("服务器地址必须是 HTTPS origin；本机测试可使用 loopback HTTP".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn scoped_key(origin: &str, user_id: &str, profile: &std::path::Path) -> String {
    let scope = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("{}\n{origin}\n{user_id}", profile.to_string_lossy()).as_bytes());
    format!("locus-server/{scope}")
}
fn key(origin: &str, user_id: &str) -> Result<String, String> {
    Ok(scoped_key(origin, user_id, &crate::commands::persistent_config_dir()?))
}
fn client() -> Result<reqwest::Client, String> {
    crate::network::reqwest_client(crate::network::ReqwestClientOptions::new()
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).follow_redirects(false))
}
async fn json_response<T: serde::de::DeserializeOwned>(mut response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => "服务器登录已失效，请重新登录".into(),
            403 => "此账号没有访问权限".into(),
            429 => "请求过于频繁，请稍后重试".into(),
            _ => format!("服务器请求失败（HTTP {}）", status.as_u16()),
        });
    }
    if response.content_length().is_some_and(|n| n > 2 * 1024 * 1024) { return Err("服务器响应过大".into()); }
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| "无法读取服务器响应")? {
        if bytes.len() + chunk.len() > 2 * 1024 * 1024 { return Err("服务器响应过大".into()); }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "服务器返回了无效响应".into())
}
fn load_tokens(account: &Account) -> Result<Tokens, String> {
    let raw = Zeroizing::new(crate::keychain::get_secret(&key(&account.server_url, &account.user.id)?)?
        .ok_or("请在设置中登录此服务器账号")?);
    let tokens: Tokens = serde_json::from_str(&raw).map_err(|_| "登录凭据损坏，请重新登录")?;
    if tokens.user.id != account.user.id { return Err("登录账号与会话绑定不一致".into()); }
    Ok(tokens)
}
fn save_tokens(origin: &str, tokens: &Tokens) -> Result<(), String> {
    let raw = Zeroizing::new(serde_json::to_string(tokens).map_err(|_| "无法保存登录凭据")?);
    crate::keychain::set_secret(&key(origin, &tokens.user.id)?, &raw)
}
async fn access_token_locked(account: &Account, force: bool) -> Result<String, String> {
    let tokens = load_tokens(account)?;
    if !force && tokens.expires_at > chrono::Utc::now().timestamp() + 30 { return Ok(tokens.access_token); }
    let refreshed = client()?.post(format!("{}/auth/refresh", account.server_url))
        .json(&serde_json::json!({ "refreshToken": tokens.refresh_token })).send().await;
    // A refresh may have rotated remotely even if its response was lost. Do not retry an ambiguous token.
    let result = match refreshed {
        Ok(response) => json_response::<Tokens>(response).await,
        Err(_) => Err("登录续期结果未知，请重新登录".into()),
    };
    let refreshed = match result {
        Ok(value) if value.user.id == account.user.id => value,
        _ => { crate::keychain::delete_secret(&key(&account.server_url, &account.user.id)?)?;
            return Err("登录续期失败，请重新登录".into()); }
    };
    if let Err(error) = save_tokens(&account.server_url, &refreshed) {
        let _ = crate::keychain::delete_secret(&key(&account.server_url, &account.user.id)?);
        return Err(error);
    }
    Ok(refreshed.access_token)
}

fn update_bindings(config: &mut Config, account: &Account, me: Me) -> Result<Status, String> {
    if me.user.id != account.user.id { return Err("服务器返回的账号与登录账号不一致".into()); }
    let upstreams = me.upstreams.into_iter().filter(|u| u.enabled && matches!(u.kind.as_str(), "codex" | "claude"))
        .map(|u| Binding {
            id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL,
                format!("{}\n{}\n{}", account.server_url, account.user.id, u.id).as_bytes()).to_string(),
            server_url: account.server_url.clone(), user_id: account.user.id.clone(), upstream_id: u.id, kind: u.kind, name: u.name,
        }).collect::<Vec<_>>();
    for binding in &upstreams { config.bindings.insert(binding.id.clone(), binding.clone()); }
    save_config(config)?;
    Ok(Status { server_url: Some(account.server_url.clone()), user: Some(me.user), upstreams, server_revocation_confirmed: None })
}

#[tauri::command]
pub async fn server_client_login(server_url: String, username: String, password: String) -> Result<Status, String> {
    let password = Zeroizing::new(password);
    let generation = LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let origin = normalize_origin(&server_url)?;
    if username.trim().is_empty() || password.is_empty() { return Err("请输入账号和密码".into()); }
    let tokens: Tokens = json_response(client()?.post(format!("{origin}/auth/login"))
        .json(&serde_json::json!({"username":username.trim(),"password":password.as_str(),"deviceName":"Locus Internal"}))
        .send().await.map_err(|_| "无法连接服务器，请检查地址与证书")?).await?;
    let me: Me = json_response(client()?.get(format!("{origin}/auth/me")).bearer_auth(&tokens.access_token)
        .send().await.map_err(|_| "无法读取账号权限")?).await?;
    let _lock = AUTH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    if generation != LOGIN_GENERATION.load(Ordering::SeqCst) { return Err("登录操作已取消".into()); }
    let mut config = load_config()?;
    let account = Account { server_url: origin, user: tokens.user.clone() };
    save_tokens(&account.server_url, &tokens)?;
    config.active = Some(account.clone());
    update_bindings(&mut config, &account, me)
}

#[tauri::command]
pub fn server_client_cancel_login() { LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst); }

#[tauri::command]
pub async fn server_client_status() -> Result<Status, String> {
    let _lock = AUTH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let mut config = load_config()?;
    let Some(account) = config.active.clone() else { return Ok(Status::default()); };
    let token = access_token_locked(&account, false).await?;
    let me: Me = json_response(client()?.get(format!("{}/auth/me", account.server_url)).bearer_auth(token)
        .send().await.map_err(|_| "无法连接服务器")?).await?;
    update_bindings(&mut config, &account, me)
}

#[tauri::command]
pub async fn server_client_logout() -> Result<Status, String> {
    LOGIN_GENERATION.fetch_add(1, Ordering::SeqCst);
    let _lock = AUTH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let mut config = load_config()?;
    let Some(account) = config.active.take() else { return Ok(Status::default()); };
    let confirmed = if let (Ok(tokens), Ok(client)) = (load_tokens(&account), client()) {
        client.post(format!("{}/auth/logout", account.server_url)).bearer_auth(&tokens.access_token)
            .send().await.is_ok_and(|response| response.status().is_success())
    } else { false };
    crate::keychain::delete_secret(&key(&account.server_url, &account.user.id)?)?;
    save_config(&config)?;
    Ok(Status { server_revocation_confirmed: Some(confirmed), ..Default::default() })
}

pub fn binding_for_model(model: &str) -> Result<Option<(Binding, String)>, String> {
    let Some(rest) = model.strip_prefix("locus-server/") else { return Ok(None); };
    let (id, model) = rest.split_once('/').ok_or("无效的服务器模型引用")?;
    uuid::Uuid::parse_str(id).map_err(|_| "无效的服务器连接标识")?;
    let config = load_config()?;
    let binding = config.bindings.get(id).cloned().ok_or("此服务器连接尚未在本机登录")?;
    let valid = (binding.kind == "codex" && model.starts_with("openai/")) ||
        (binding.kind == "claude" && model.starts_with("claude_code/"));
    if !valid { return Err("模型与服务器上游类型不匹配".into()); }
    Ok(Some((binding, model.into())))
}
pub fn protocol_model_id(model: &str) -> &str {
    let provider = provider_model_id(model);
    if provider.starts_with("openai/") { provider } else { model }
}
pub fn provider_model_id(model: &str) -> &str {
    model.strip_prefix("locus-server/").and_then(|rest| rest.split_once('/'))
        .map(|(_, model)| model).filter(|model| model.starts_with("openai/") || model.starts_with("claude_code/")).unwrap_or(model)
}
pub async fn credential_for_binding(binding_id: &str) -> Result<Connection, String> {
    let _lock = AUTH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let config = load_config()?;
    let binding = config.bindings.get(binding_id).ok_or("服务器连接不存在，请重新登录")?;
    let account = config.active.as_ref().filter(|account| account.server_url == binding.server_url && account.user.id == binding.user_id)
        .ok_or("请登录此会话绑定的服务器账号")?;
    let access_token = access_token_locked(account, false).await?;
    Ok(Connection { server_url: binding.server_url.clone(), access_token, upstream_id: binding.upstream_id.clone(),
        user_id: binding.user_id.clone(), binding_id: binding.id.clone() })
}
pub async fn refresh_binding(binding_id: &str) -> Result<(), String> {
    let _lock = AUTH_LOCK.get_or_init(|| Mutex::new(())).lock().await;
    let config = load_config()?;
    let binding = config.bindings.get(binding_id).ok_or("服务器连接不存在")?;
    let account = config.active.as_ref().filter(|a| a.server_url == binding.server_url && a.user.id == binding.user_id)
        .ok_or("请登录此会话绑定的服务器账号")?;
    access_token_locked(account, true).await.map(|_| ())
}
pub fn codex_base_url(binding: &Binding) -> String { format!("{}/api/codex/{}", binding.server_url, binding.upstream_id) }
pub fn has_binding_credentials(binding: &Binding) -> bool {
    load_config().ok().and_then(|config| config.active)
        .filter(|a| a.server_url == binding.server_url && a.user.id == binding.user_id)
        .and_then(|account| load_tokens(&account).ok()).is_some()
}
pub fn binding_is_active(id:&str)->bool {
    load_config().is_ok_and(|config|config.bindings.get(id).is_some_and(|binding|
        config.active.as_ref().is_some_and(|account|account.server_url==binding.server_url&&account.user.id==binding.user_id)))
}
pub fn is_codex_proxy_url(url: &str) -> bool {
    load_config().is_ok_and(|config| config.bindings.values()
        .any(|binding| binding.kind == "codex" && codex_base_url(binding) == url.trim_end_matches('/')))
}

#[tauri::command]
pub async fn server_client_usage(binding_id: String) -> Result<crate::llm::codex_usage::CodexRateLimitsResponse, String> {
    let binding = load_config()?.bindings.remove(&binding_id).ok_or("服务器连接不存在")?;
    if binding.kind != "codex" { return Err("此上游不支持额度查询".into()); }
    let connection = credential_for_binding(&binding.id).await?;
    match crate::llm::codex_usage::fetch_codex_rate_limits(&connection.access_token, None, Some(&codex_base_url(&binding))).await {
        Ok(value) => Ok(value),
        Err(error) if error.is_unauthorized() => {
            refresh_binding(&binding.id).await?;
            let connection = credential_for_binding(&binding.id).await?;
            crate::llm::codex_usage::fetch_codex_rate_limits(&connection.access_token, None, Some(&codex_base_url(&binding)))
                .await.map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
pub async fn server_client_models() -> Result<Vec<crate::llm::codex_models::CodexAvailableModel>, String> {
    let status = server_client_status().await?;
    let mut models = Vec::new();
    let mut failures = Vec::new();
    for binding in &status.upstreams {
        let result: Result<Vec<crate::llm::codex_models::CodexAvailableModel>, String> = async {
          let connection = credential_for_binding(&binding.id).await?;
          let available = if binding.kind == "codex" {
            let cache_dir = crate::commands::persistent_config_dir()?.join("server-models").join(&binding.id);
            crate::llm::codex_models::list_codex_available_models(&connection.access_token,
                None, Some(&codex_base_url(binding)), &cache_dir, true, false).await?
        } else {
            #[derive(Deserialize)]
            struct RemoteModel { id: String, name: String }
            let remote: Vec<RemoteModel> = json_response(client()?.get(format!("{}/api/claude/{}/models", binding.server_url, binding.upstream_id))
                .bearer_auth(&connection.access_token).send().await.map_err(|_| "无法读取服务器模型")?).await?;
            remote.into_iter().filter(|model| model.id.starts_with("claude_code/")).map(|model|
                crate::llm::codex_models::CodexAvailableModel { id: model.id, name: model.name,
                    provider: "claude_code".into(), default_effort: None, supported_efforts: Vec::new(),
                    additional_speed_tiers: Vec::new(), effective_context_window: None, is_default: false }).collect()
          };
          Ok(available)
        }.await;
        let mut available = match result {
            Ok(available) => available,
            Err(_) => { failures.push(binding.name.clone()); continue; }
        };
        for model in &mut available {
            model.id = format!("locus-server/{}/{}", binding.id, model.id);
            model.name = format!("{} · {}", model.name, binding.name);
            model.is_default = false;
        }
        models.extend(available);
    }
    if models.is_empty() && !failures.is_empty() {
        return Err(format!("无法读取上游模型（{}），请管理员检查登录和连接。", failures.into_iter().take(4).collect::<Vec<_>>().join("、")));
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_rejects_credentials_paths_and_insecure_remote_hosts() {
        for bad in ["http://10.0.0.1", "https://user:secret@host", "https://host/login", "https://host?token=x", "https://host/#x"] {
            assert!(normalize_origin(bad).is_err(), "{bad}");
        }
        assert_eq!(normalize_origin("https://Locus.Example:443/").unwrap(), "https://locus.example");
        assert_eq!(normalize_origin("http://127.0.0.1:4321").unwrap(), "http://127.0.0.1:4321");
    }
    #[test]
    fn only_codex_execution_model_is_unwrapped() {
        assert_eq!(protocol_model_id("locus-server/id/openai/gpt-5"), "openai/gpt-5");
        assert_eq!(protocol_model_id("openai/gpt-5"), "openai/gpt-5");
        assert_eq!(protocol_model_id("locus-server/id/claude_code/model"), "locus-server/id/claude_code/model");
        let profile = std::path::Path::new("profile-one");
        assert_ne!(scoped_key("https://a", "user", profile), scoped_key("https://b", "user", profile));
        assert_ne!(scoped_key("https://a", "one", profile), scoped_key("https://a", "two", profile));
        assert_ne!(scoped_key("https://a", "user", profile), scoped_key("https://a", "user", std::path::Path::new("profile-two")));
    }
}
