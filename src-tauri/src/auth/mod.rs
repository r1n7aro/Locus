pub mod codex;

#[cfg(windows)]
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
#[cfg(windows)]
use base64::engine::general_purpose::STANDARD;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[cfg(windows)]
use windows::Win32::Foundation::{LocalFree, HLOCAL};
#[cfg(windows)]
use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

use crate::commands::{
    ApiFormat, CustomEndpoint, CustomEndpointServerTools, CustomReasoningParamFormat,
};
use crate::keychain;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_AI_AUTH_URL: &str = "https://claude.com/cai/oauth/authorize";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_AI_SCOPE: &str =
    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload";
const CLIENT_METADATA_FILE: &str = "claude_client_metadata.json";

const REFRESH_BUFFER_SECS: i64 = 60;
/// Far-future expiry stamped on imported credentials that carry no refresh token
/// (e.g. a long-lived `CLAUDE_CODE_OAUTH_TOKEN` minted by `claude setup-token`).
/// Such tokens cannot be refreshed, so a short synthetic expiry would later push
/// `access_token()` into `refresh()` which — finding no refresh token — wipes the
/// credential. A far-future expiry keeps the token usable until the server itself
/// rejects it (401). 2100-01-01 UTC.
const NON_REFRESHABLE_TOKEN_EXPIRY_SECS: i64 = 4_102_444_800;
const CLAUDE_CODE_IMPORTED_ENDPOINT_ID: &str = "claude-code-import";
const CLAUDE_CODE_IMPORTED_ENDPOINT_NAME: &str = "Claude Code";
const DEFAULT_CLAUDE_CODE_CUSTOM_ENDPOINT_BASE: &str = "https://api.anthropic.com/v1";
const DEFAULT_CLAUDE_CODE_CUSTOM_MODEL: &str = "claude-opus-4-8";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeCodeUserMetadata {
    pub device_id: String,
    pub account_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClaudeClientMetadataState {
    pub device_id: String,
    #[serde(default)]
    pub account_uuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub authenticated: bool,
    pub has_api_key: bool,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthUrlInfo {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeCodeTokenImportKind {
    Oauth,
    CustomEndpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeTokenImportResult {
    pub kind: ClaudeCodeTokenImportKind,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub has_refresh_token: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_endpoint: Option<CustomEndpoint>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
}

// ── AuthState ──

pub struct AuthState {
    tokens: Option<TokenData>,
    client_metadata: ClaudeClientMetadataState,
    client_metadata_path: PathBuf,
    pending_verifier: Option<String>,
    pending_state: Option<String>,
}

impl AuthState {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let client_metadata_path = data_dir.join(CLIENT_METADATA_FILE);
        let tokens = Self::load_from_keychain();

        if tokens.is_some() {
            eprintln!("[Auth] loaded existing tokens from keychain");
        } else {
            eprintln!("[Auth] no existing tokens found");
        }

        let client_metadata = Self::load_client_metadata(&client_metadata_path)
            .unwrap_or_else(Self::bootstrap_client_metadata);

        let state = AuthState {
            tokens,
            client_metadata,
            client_metadata_path,
            pending_verifier: None,
            pending_state: None,
        };

        if !state.client_metadata_path.is_file() {
            let _ = state.save_client_metadata();
        }

        state
    }

    pub fn is_authenticated(&self) -> bool {
        self.tokens.is_some()
    }

    pub fn email(&self) -> Option<String> {
        None
    }

    pub fn claude_code_user_metadata(&mut self) -> Result<ClaudeCodeUserMetadata, String> {
        let mut changed = false;

        if self.client_metadata.device_id.trim().is_empty() {
            self.client_metadata.device_id = generate_device_id();
            changed = true;
        }

        if self
            .client_metadata
            .account_uuid
            .as_deref()
            .map(|v| v.trim().is_empty())
            .unwrap_or(true)
        {
            self.client_metadata.account_uuid = Some(uuid::Uuid::new_v4().to_string());
            changed = true;
        }

        if changed {
            self.save_client_metadata()?;
        }

        Ok(ClaudeCodeUserMetadata {
            device_id: self.client_metadata.device_id.clone(),
            account_uuid: self
                .client_metadata
                .account_uuid
                .clone()
                .ok_or_else(|| "Claude client account_uuid is unavailable".to_string())?,
        })
    }

    pub async fn access_token(&mut self) -> Result<String, String> {
        let tokens = self
            .tokens
            .as_ref()
            .ok_or_else(|| "Not authenticated".to_string())?;

        let now = chrono::Utc::now().timestamp();
        if now >= tokens.expires_at - REFRESH_BUFFER_SECS {
            eprintln!("[Auth] token expired or expiring soon, refreshing...");
            self.refresh().await?;
        }

        Ok(self
            .tokens
            .as_ref()
            .ok_or_else(|| "Token unavailable after refresh".to_string())?
            .access_token
            .clone())
    }

    pub fn get_authorize_url(&mut self) -> AuthUrlInfo {
        let verifier = generate_code_verifier();
        let state = generate_oauth_state();
        let challenge = generate_code_challenge(&verifier);

        self.pending_verifier = Some(verifier);
        self.pending_state = Some(state.clone());

        build_authorize_url(&challenge, &state)
    }

    pub async fn exchange(&mut self, code: &str) -> Result<(), String> {
        let (actual_code, state) = parse_authorization_code(code)?;

        let verifier = self
            .pending_verifier
            .take()
            .ok_or_else(|| "No pending PKCE verifier. Call get_authorize_url first.".to_string())?;
        let expected_state = self
            .pending_state
            .take()
            .ok_or_else(|| "No pending OAuth state. Call get_authorize_url first.".to_string())?;

        let client = crate::network::default_reqwest_client()?;

        if state != expected_state {
            return Err("OAuth state mismatch. Please retry login.".to_string());
        }

        let body = serde_json::json!({
            "code": actual_code,
            "state": state,
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": verifier,
        });

        eprintln!("[Auth] exchanging authorization code for tokens...");

        let resp = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Token exchange request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Token exchange failed ({}): {}",
                status, error_body
            ));
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse token response: {}", e))?;

        let now = chrono::Utc::now().timestamp();
        let tokens = TokenData {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token.unwrap_or_default(),
            expires_at: now + token_resp.expires_in,
        };

        self.tokens = Some(tokens);
        self.save_tokens()?;

        eprintln!("[Auth] tokens exchanged and saved successfully");
        Ok(())
    }

    pub async fn refresh(&mut self) -> Result<(), String> {
        let refresh_token = self
            .tokens
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or_else(|| "No refresh token available".to_string())?;
        if refresh_token.trim().is_empty() {
            return Err("No refresh token available. Please re-login.".to_string());
        }

        let client = crate::network::default_reqwest_client()?;

        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token.clone(),
            "client_id": CLIENT_ID,
            "scope": CLAUDE_AI_SCOPE,
        });

        let resp = client
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Token refresh request failed: {}", e))?;

        let status = resp.status();
        if !status.is_success() {
            let error_body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Token refresh failed ({}): {}. Please re-login.",
                status, error_body
            ));
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {}", e))?;

        let now = chrono::Utc::now().timestamp();
        self.tokens = Some(TokenData {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token.unwrap_or(refresh_token),
            expires_at: now + token_resp.expires_in,
        });

        self.save_tokens()?;
        eprintln!("[Auth] tokens refreshed successfully");
        Ok(())
    }

    pub fn logout(&mut self) {
        self.tokens = None;
        self.pending_verifier = None;
        self.pending_state = None;
        self.client_metadata.account_uuid = None;
        let _ = keychain::delete_secret(keychain::KEY_CLAUDE_TOKENS);
        let _ = self.save_client_metadata();
        eprintln!("[Auth] logged out, tokens cleared from keychain");
    }

    pub async fn import_claude_code_oauth_tokens(
        &mut self,
    ) -> Result<ClaudeCodeTokenImportResult, String> {
        if let Some((source, custom_endpoint)) = load_claude_code_custom_endpoint()? {
            return Ok(ClaudeCodeTokenImportResult {
                kind: ClaudeCodeTokenImportKind::CustomEndpoint,
                source,
                expires_at: None,
                has_refresh_token: false,
                custom_endpoint: Some(custom_endpoint),
            });
        }

        if let Some((source, imported)) = load_claude_code_oauth_credentials()? {
            if imported.access_token.trim().is_empty() && imported.refresh_token.trim().is_empty() {
                return Err("Claude Code OAuth credentials are empty".to_string());
            }

            if imported.access_token.trim().is_empty() {
                return Err(
                    "Claude Code OAuth credentials only include a refresh token. Run `claude` to refresh the access token, then import again."
                        .to_string(),
                );
            }

            let expires_at = imported
                .expires_at
                .unwrap_or(NON_REFRESHABLE_TOKEN_EXPIRY_SECS);
            let now = chrono::Utc::now().timestamp();
            if expires_at <= now + REFRESH_BUFFER_SECS {
                return Err(
                    "Claude Code OAuth access token is expired. Run `claude` to refresh or login again, then import again."
                        .to_string(),
                );
            }

            self.tokens = Some(TokenData {
                access_token: imported.access_token,
                refresh_token: String::new(),
                expires_at,
            });
            self.save_tokens()?;

            return Ok(ClaudeCodeTokenImportResult {
                kind: ClaudeCodeTokenImportKind::Oauth,
                source,
                expires_at: Some(expires_at),
                has_refresh_token: false,
                custom_endpoint: None,
            });
        }

        Err(
            "No Claude Code OAuth credentials or custom endpoint configuration found. Run `claude` and complete /login first."
                .to_string(),
        )
    }

    fn save_tokens(&self) -> Result<(), String> {
        if let Some(ref tokens) = self.tokens {
            Self::save_to_keychain(tokens)?;
        }
        Ok(())
    }

    fn save_to_keychain(tokens: &TokenData) -> Result<(), String> {
        let json = serde_json::to_string(tokens)
            .map_err(|e| format!("Failed to serialize tokens: {}", e))?;
        let payload_bytes = json.len();
        let access_len = tokens.access_token.len();
        let refresh_len = tokens.refresh_token.len();
        let result = keychain::set_secret(keychain::KEY_CLAUDE_TOKENS, &json);
        match &result {
            Ok(()) => eprintln!(
                "[Auth] keychain write success: key={} payload_bytes={} access_len={} refresh_len={} expires_at={}",
                keychain::KEY_CLAUDE_TOKENS,
                payload_bytes,
                access_len,
                refresh_len,
                tokens.expires_at
            ),
            Err(err) => eprintln!(
                "[Auth] keychain write failed: key={} payload_bytes={} access_len={} refresh_len={} expires_at={} error={}",
                keychain::KEY_CLAUDE_TOKENS,
                payload_bytes,
                access_len,
                refresh_len,
                tokens.expires_at,
                err
            ),
        }
        result
    }

    fn load_from_keychain() -> Option<TokenData> {
        match keychain::get_secret(keychain::KEY_CLAUDE_TOKENS) {
            Ok(Some(s)) => {
                let payload_bytes = s.len();
                eprintln!(
                    "[Auth] keychain read hit: key={} payload_bytes={}",
                    keychain::KEY_CLAUDE_TOKENS,
                    payload_bytes
                );
                match serde_json::from_str::<TokenData>(&s) {
                    Ok(t) => {
                        eprintln!(
                            "[Auth] keychain parse success: key={} access_len={} refresh_len={} expires_at={}",
                            keychain::KEY_CLAUDE_TOKENS,
                            t.access_token.len(),
                            t.refresh_token.len(),
                            t.expires_at
                        );
                        Some(t)
                    }
                    Err(e) => {
                        eprintln!(
                            "[Auth] failed to parse keychain tokens: key={} payload_bytes={} error={}",
                            keychain::KEY_CLAUDE_TOKENS,
                            payload_bytes,
                            e
                        );
                        None
                    }
                }
            }
            Ok(None) => {
                eprintln!(
                    "[Auth] keychain read miss: key={}",
                    keychain::KEY_CLAUDE_TOKENS
                );
                None
            }
            Err(e) => {
                eprintln!(
                    "[Auth] keychain read error: key={} error={}",
                    keychain::KEY_CLAUDE_TOKENS,
                    e
                );
                None
            }
        }
    }

    fn save_client_metadata(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.client_metadata)
            .map_err(|e| format!("Failed to serialize Claude client metadata: {}", e))?;
        std::fs::write(&self.client_metadata_path, json)
            .map_err(|e| format!("Failed to write Claude client metadata: {}", e))
    }

    fn load_client_metadata(path: &PathBuf) -> Option<ClaudeClientMetadataState> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<ClaudeClientMetadataState>(&content).ok()
    }

    fn bootstrap_client_metadata() -> ClaudeClientMetadataState {
        ClaudeClientMetadataState {
            device_id: generate_device_id(),
            account_uuid: Some(uuid::Uuid::new_v4().to_string()),
        }
    }
}

fn generate_code_verifier() -> String {
    generate_random_urlsafe_token()
}

fn generate_oauth_state() -> String {
    generate_random_urlsafe_token()
}

fn generate_random_urlsafe_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_code_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    URL_SAFE_NO_PAD.encode(hash)
}

fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for ch in s.bytes() {
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(ch as char);
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{:02X}", ch));
            }
        }
    }
    result
}

fn build_authorize_url(challenge: &str, state: &str) -> AuthUrlInfo {
    let url = format!(
        "{}?code=true&client_id={}&response_type=code&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}",
        CLAUDE_AI_AUTH_URL,
        CLIENT_ID,
        percent_encode(REDIRECT_URI),
        percent_encode(CLAUDE_AI_SCOPE),
        challenge,
        state,
    );

    AuthUrlInfo { url }
}

fn parse_authorization_code(code: &str) -> Result<(String, String), String> {
    let trimmed = code.trim();
    let (actual_code, state) = trimmed
        .split_once('#')
        .ok_or_else(|| "Authorization code must use the format code#state".to_string())?;

    let actual_code = actual_code.trim();
    let state = state.trim();
    if actual_code.is_empty() || state.is_empty() {
        return Err("Authorization code must include both code and state".to_string());
    }

    Ok((actual_code.to_string(), state.to_string()))
}

#[derive(Debug, Clone, Default)]
struct ImportedClaudeCodeOAuth {
    access_token: String,
    refresh_token: String,
    expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct ScoredImportedClaudeCodeOAuth {
    score: i32,
    expires_at: i64,
    credentials: ImportedClaudeCodeOAuth,
}

#[derive(Debug, Clone, Default)]
struct ImportedClaudeCodeCustomEndpoint {
    base_url: Option<String>,
    api_key: Option<String>,
    api_model: Option<String>,
}

fn load_claude_code_custom_endpoint() -> Result<Option<(String, CustomEndpoint)>, String> {
    if let Some(config) = claude_code_custom_endpoint_from_process_env() {
        return Ok(Some((
            "Claude Code environment variables".to_string(),
            build_claude_code_custom_endpoint(config),
        )));
    }

    if let Some(dir) = crate::llm::claude_code_cli::claude_config_dir() {
        let path = dir.join("settings.json");
        if let Some(config) = load_claude_code_custom_endpoint_file(&path)? {
            return Ok(Some((
                format!("Claude Code {}", path.display()),
                build_claude_code_custom_endpoint(config),
            )));
        }
    }

    Ok(None)
}

fn claude_code_custom_endpoint_from_process_env() -> Option<ImportedClaudeCodeCustomEndpoint> {
    let base_url = env_string("ANTHROPIC_BASE_URL");
    let api_key = env_string("ANTHROPIC_AUTH_TOKEN")
        .map(strip_bearer_prefix)
        .and_then(nonempty_string)
        .or_else(|| env_string("ANTHROPIC_API_KEY"));
    let api_model = env_string("ANTHROPIC_MODEL");

    if base_url.is_none() && api_key.is_none() {
        return None;
    }

    Some(ImportedClaudeCodeCustomEndpoint {
        base_url,
        api_key,
        api_model,
    })
}

fn load_claude_code_custom_endpoint_file(
    path: &Path,
) -> Result<Option<ImportedClaudeCodeCustomEndpoint>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to read Claude Code settings file {}: {}",
                path.display(),
                err
            ));
        }
    };
    let value = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|err| format!("Failed to parse Claude Code settings JSON: {}", err))?;
    Ok(parse_claude_code_custom_endpoint_settings(&value))
}

fn parse_claude_code_custom_endpoint_settings(
    value: &serde_json::Value,
) -> Option<ImportedClaudeCodeCustomEndpoint> {
    let env = value.get("env");
    let base_url = json_string(value, &["base_url", "baseUrl"])
        .or_else(|| env.and_then(|env| json_string(env, &["ANTHROPIC_BASE_URL"])));
    let api_key = env
        .and_then(|env| json_string(env, &["ANTHROPIC_AUTH_TOKEN"]))
        .map(strip_bearer_prefix)
        .and_then(nonempty_string)
        .or_else(|| env.and_then(|env| json_string(env, &["ANTHROPIC_API_KEY"])))
        .or_else(|| json_string(value, &["apiKey", "api_key"]));
    let api_model = env
        .and_then(|env| json_string(env, &["ANTHROPIC_MODEL"]))
        .or_else(|| json_string(value, &["apiModel", "api_model", "model"]));

    if base_url.is_none() && api_key.is_none() {
        return None;
    }

    Some(ImportedClaudeCodeCustomEndpoint {
        base_url,
        api_key,
        api_model,
    })
}

fn build_claude_code_custom_endpoint(config: ImportedClaudeCodeCustomEndpoint) -> CustomEndpoint {
    let api_model = normalize_imported_claude_model(config.api_model);
    CustomEndpoint {
        id: CLAUDE_CODE_IMPORTED_ENDPOINT_ID.to_string(),
        name: CLAUDE_CODE_IMPORTED_ENDPOINT_NAME.to_string(),
        api_model: api_model.clone(),
        endpoint: normalize_anthropic_messages_base_url(config.base_url),
        api_format: ApiFormat::AnthropicMessages,
        api_key: config.api_key.unwrap_or_default(),
        context_length: imported_claude_context_length(&api_model),
        beta_flags: Vec::new(),
        supported_reasoning_efforts: ["low", "medium", "high", "xhigh", "max"]
            .into_iter()
            .map(str::to_string)
            .collect(),
        reasoning_param_format: Some(CustomReasoningParamFormat::AnthropicThinking),
        replay_reasoning_content: Some(false),
        server_tools: CustomEndpointServerTools { web_search: false },
        supports_tool_lazy_loading: false,
        supports_vision: true,
    }
}

fn env_string(name: &str) -> Option<String> {
    std::env::var(name).ok().and_then(nonempty_string)
}

fn nonempty_string(value: String) -> Option<String> {
    let trimmed = value.trim().to_string();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn strip_bearer_prefix(value: String) -> String {
    let trimmed = value.trim();
    if trimmed
        .get(..7)
        .map(|prefix| prefix.eq_ignore_ascii_case("bearer "))
        .unwrap_or(false)
    {
        trimmed[7..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_anthropic_messages_base_url(base_url: Option<String>) -> String {
    let mut normalized = base_url
        .and_then(nonempty_string)
        .unwrap_or_else(|| DEFAULT_CLAUDE_CODE_CUSTOM_ENDPOINT_BASE.to_string())
        .trim_end_matches('/')
        .to_string();

    if normalized
        .to_ascii_lowercase()
        .trim_end_matches('/')
        .ends_with("/messages")
    {
        let without_messages = normalized
            .trim_end_matches('/')
            .strip_suffix("/messages")
            .unwrap_or(&normalized)
            .trim_end_matches('/')
            .to_string();
        normalized = without_messages;
    }

    if endpoint_has_version_suffix(&normalized) {
        normalized
    } else {
        format!("{}/v1", normalized.trim_end_matches('/'))
    }
}

fn endpoint_has_version_suffix(endpoint: &str) -> bool {
    let path = url::Url::parse(endpoint)
        .ok()
        .map(|url| url.path().to_string())
        .unwrap_or_else(|| endpoint.to_string());
    let last = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    last.len() > 1
        && last
            .strip_prefix('v')
            .map(|digits| digits.chars().all(|ch| ch.is_ascii_digit()))
            .unwrap_or(false)
}

fn normalize_imported_claude_model(model: Option<String>) -> String {
    let raw = model
        .and_then(nonempty_string)
        .unwrap_or_else(|| DEFAULT_CLAUDE_CODE_CUSTOM_MODEL.to_string());
    let without_provider = raw
        .strip_prefix("anthropic/")
        .or_else(|| raw.strip_prefix("openrouter/"))
        .or_else(|| raw.strip_prefix("claude_code/"))
        .unwrap_or(&raw);
    let without_context = without_provider
        .strip_suffix("[1m]")
        .unwrap_or(without_provider)
        .trim();

    match without_context {
        "opus" => DEFAULT_CLAUDE_CODE_CUSTOM_MODEL.to_string(),
        "sonnet" => "claude-sonnet-5".to_string(),
        "fable" => "claude-fable-5".to_string(),
        "haiku" => "claude-haiku-4-5".to_string(),
        "claude-opus-4.8" => "claude-opus-4-8".to_string(),
        "claude-opus-4.7" => "claude-opus-4-7".to_string(),
        "claude-sonnet-4.6" => "claude-sonnet-4-6".to_string(),
        "claude-opus-4.6" => "claude-opus-4-6".to_string(),
        "claude-haiku-4.5" => "claude-haiku-4-5".to_string(),
        other if other.is_empty() => DEFAULT_CLAUDE_CODE_CUSTOM_MODEL.to_string(),
        other => other.to_string(),
    }
}

fn imported_claude_context_length(model: &str) -> u32 {
    let m = model.to_ascii_lowercase();
    if m.contains("claude-opus-4-8")
        || m.contains("claude-opus-4.8")
        || m.contains("claude-fable-5")
        || m.contains("claude-sonnet-5")
        || m.contains("claude-opus-4-7")
        || m.contains("claude-opus-4.7")
        || m.contains("claude-opus-4-6")
        || m.contains("claude-opus-4.6")
        || m.contains("claude-sonnet-4-6")
        || m.contains("claude-sonnet-4.6")
    {
        1_000_000
    } else if m.contains("claude")
        || m.contains("opus")
        || m.contains("sonnet")
        || m.contains("haiku")
    {
        200_000
    } else {
        256_000
    }
}

fn load_claude_code_oauth_credentials() -> Result<Option<(String, ImportedClaudeCodeOAuth)>, String>
{
    if let Some(dir) = crate::llm::claude_code_cli::claude_config_dir() {
        let path = dir.join(".credentials.json");
        if let Some(credentials) = load_claude_code_oauth_credentials_file(&path)? {
            return Ok(Some((
                format!("Claude Code {}", path.display()),
                credentials,
            )));
        }
    }

    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(Some((
                "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
                ImportedClaudeCodeOAuth {
                    access_token: token,
                    refresh_token: String::new(),
                    // Unknown expiry — `setup-token` bearers are long-lived. Leave
                    // it to the import path to stamp the non-refreshable far-future
                    // expiry rather than a synthetic 1h window that self-destructs.
                    expires_at: None,
                },
            )));
        }
    }

    if let Some((source, credentials)) = load_claude_desktop_oauth_credentials()? {
        return Ok(Some((source, credentials)));
    }

    Ok(None)
}

fn load_claude_code_oauth_credentials_file(
    path: &Path,
) -> Result<Option<ImportedClaudeCodeOAuth>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to read Claude Code credentials file {}: {}",
                path.display(),
                err
            ));
        }
    };
    let value = serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|err| format!("Failed to parse Claude Code credentials JSON: {}", err))?;
    Ok(parse_claude_code_oauth_credentials(&value))
}

fn parse_claude_code_oauth_credentials(
    value: &serde_json::Value,
) -> Option<ImportedClaudeCodeOAuth> {
    let oauth = value.get("claudeAiOauth")?;
    let access_token = json_string(oauth, &["accessToken", "access_token"]).unwrap_or_default();
    let refresh_token = json_string(oauth, &["refreshToken", "refresh_token"]).unwrap_or_default();
    if access_token.is_empty() && refresh_token.is_empty() {
        return None;
    }
    Some(ImportedClaudeCodeOAuth {
        access_token,
        refresh_token,
        expires_at: json_expiry_seconds(
            oauth,
            &[
                "expiresAt",
                "expires_at",
                "accessTokenExpiresAt",
                "access_token_expires_at",
                "expiry",
                "expiration",
            ],
        ),
    })
}

fn load_claude_desktop_oauth_credentials(
) -> Result<Option<(String, ImportedClaudeCodeOAuth)>, String> {
    for dir in claude_desktop_data_dirs() {
        if let Some(credentials) = load_claude_desktop_oauth_credentials_dir(&dir)? {
            return Ok(Some((
                format!("Claude Desktop {}", dir.join("config.json").display()),
                credentials,
            )));
        }
    }

    Ok(None)
}

#[cfg(windows)]
fn claude_desktop_data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        let packages = local_app_data.join("Packages");
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("Claude_") {
                    dirs.push(
                        entry
                            .path()
                            .join("LocalCache")
                            .join("Roaming")
                            .join("Claude"),
                    );
                }
            }
        }

        dirs.push(local_app_data.join("Claude"));
        dirs.push(local_app_data.join("Claude-3p"));
    }

    if let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) {
        dirs.push(app_data.join("Claude"));
        dirs.push(app_data.join("Claude-3p"));
    }

    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[cfg(not(windows))]
fn claude_desktop_data_dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn load_claude_desktop_oauth_credentials_dir(
    dir: &Path,
) -> Result<Option<ImportedClaudeCodeOAuth>, String> {
    let config_path = dir.join("config.json");
    let local_state_path = dir.join("Local State");
    let raw_config = match std::fs::read_to_string(&config_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "Failed to read Claude Desktop config file {}: {}",
                config_path.display(),
                err
            ));
        }
    };

    let config = serde_json::from_str::<serde_json::Value>(&raw_config)
        .map_err(|err| format!("Failed to parse Claude Desktop config JSON: {}", err))?;
    let Some(encrypted_cache) = json_string(&config, &["oauth:tokenCache"]) else {
        return Ok(None);
    };

    let raw_local_state = std::fs::read_to_string(&local_state_path).map_err(|err| {
        format!(
            "Failed to read Claude Desktop Local State file {}: {}",
            local_state_path.display(),
            err
        )
    })?;
    let local_state = serde_json::from_str::<serde_json::Value>(&raw_local_state)
        .map_err(|err| format!("Failed to parse Claude Desktop Local State JSON: {}", err))?;

    let decrypted = decrypt_claude_desktop_token_cache(&encrypted_cache, &local_state)?;
    let cache = serde_json::from_slice::<serde_json::Value>(&decrypted)
        .map_err(|err| format!("Failed to parse Claude Desktop OAuth token cache: {}", err))?;
    Ok(parse_claude_desktop_oauth_token_cache(&cache))
}

#[cfg(windows)]
fn decrypt_claude_desktop_token_cache(
    encrypted_cache: &str,
    local_state: &serde_json::Value,
) -> Result<Vec<u8>, String> {
    let encrypted_key = local_state
        .get("os_crypt")
        .and_then(|value| json_string(value, &["encrypted_key"]))
        .ok_or_else(|| {
            "Claude Desktop Local State is missing os_crypt.encrypted_key".to_string()
        })?;
    let encrypted_key_bytes = STANDARD
        .decode(encrypted_key)
        .map_err(|err| format!("Failed to decode Claude Desktop encrypted key: {}", err))?;
    let dpapi_payload = encrypted_key_bytes
        .strip_prefix(b"DPAPI")
        .ok_or_else(|| "Claude Desktop encrypted key does not use DPAPI prefix".to_string())?;
    let key = windows_unprotect_data(dpapi_payload)?;

    let blob = STANDARD
        .decode(encrypted_cache)
        .map_err(|err| format!("Failed to decode Claude Desktop OAuth token cache: {}", err))?;
    decrypt_chromium_aes_gcm_blob(&key, &blob)
}

#[cfg(not(windows))]
fn decrypt_claude_desktop_token_cache(
    _encrypted_cache: &str,
    _local_state: &serde_json::Value,
) -> Result<Vec<u8>, String> {
    Err("Claude Desktop OAuth import is currently only implemented on Windows".to_string())
}

#[cfg(windows)]
fn windows_unprotect_data(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(&mut input, None, None, None, None, 0, &mut output)
            .map_err(|err| format!("Failed to decrypt Claude Desktop DPAPI key: {}", err))?;
        let decrypted = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(decrypted)
    }
}

#[cfg(windows)]
fn decrypt_chromium_aes_gcm_blob(key: &[u8], blob: &[u8]) -> Result<Vec<u8>, String> {
    let payload = blob
        .strip_prefix(b"v10")
        .or_else(|| blob.strip_prefix(b"v11"))
        .ok_or_else(|| {
            "Claude Desktop OAuth token cache uses an unsupported encryption prefix".to_string()
        })?;
    if payload.len() <= 12 + 16 {
        return Err("Claude Desktop OAuth token cache is too short".to_string());
    }

    let (nonce, ciphertext_and_tag) = payload.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Claude Desktop OAuth token cache key is invalid".to_string())?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext_and_tag)
        .map_err(|err| {
            format!(
                "Failed to decrypt Claude Desktop OAuth token cache: {}",
                err
            )
        })
}

fn parse_claude_desktop_oauth_token_cache(
    value: &serde_json::Value,
) -> Option<ImportedClaudeCodeOAuth> {
    let object = value.as_object()?;
    let now = chrono::Utc::now().timestamp();
    let mut best: Option<ScoredImportedClaudeCodeOAuth> = None;

    for (cache_key, entry) in object {
        let Some(credentials) = parse_claude_desktop_oauth_token_entry(entry) else {
            continue;
        };
        let Some(candidate) = score_claude_desktop_oauth_candidate(cache_key, credentials, now)
        else {
            continue;
        };
        let replace = best
            .as_ref()
            .map(|current| {
                candidate.score > current.score
                    || (candidate.score == current.score
                        && candidate.expires_at > current.expires_at)
            })
            .unwrap_or(true);
        if replace {
            best = Some(candidate);
        }
    }

    best.map(|candidate| candidate.credentials)
}

fn parse_claude_desktop_oauth_token_entry(
    value: &serde_json::Value,
) -> Option<ImportedClaudeCodeOAuth> {
    let access_token =
        json_string(value, &["token", "accessToken", "access_token"]).unwrap_or_default();
    let refresh_token = json_string(value, &["refreshToken", "refresh_token"]).unwrap_or_default();
    if access_token.is_empty() && refresh_token.is_empty() {
        return None;
    }

    Some(ImportedClaudeCodeOAuth {
        access_token,
        refresh_token,
        expires_at: json_expiry_seconds(
            value,
            &[
                "expiresAt",
                "expires_at",
                "accessTokenExpiresAt",
                "access_token_expires_at",
                "expiry",
                "expiration",
            ],
        ),
    })
}

fn score_claude_desktop_oauth_candidate(
    cache_key: &str,
    credentials: ImportedClaudeCodeOAuth,
    now: i64,
) -> Option<ScoredImportedClaudeCodeOAuth> {
    let lower = cache_key.to_ascii_lowercase();
    if !lower.contains("https://api.anthropic.com") || !lower.contains("user:inference") {
        return None;
    }

    let expires_at = credentials
        .expires_at
        .unwrap_or(NON_REFRESHABLE_TOKEN_EXPIRY_SECS);
    if expires_at <= now + REFRESH_BUFFER_SECS {
        return None;
    }

    let mut score = 0;
    if lower.contains(&CLIENT_ID.to_ascii_lowercase()) {
        score += 1_000;
    }
    if lower.contains("user:sessions:claude_code") {
        score += 500;
    }
    if lower.contains("user:profile") {
        score += 100;
    }
    if lower.contains("user:file_upload") {
        score += 50;
    }
    if lower.contains("user:mcp_servers") {
        score += 25;
    }
    if !credentials.refresh_token.is_empty() {
        score += 5;
    }

    Some(ScoredImportedClaudeCodeOAuth {
        score,
        expires_at,
        credentials,
    })
}

fn json_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn json_expiry_seconds(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    for key in keys {
        let Some(raw) = value.get(*key) else {
            continue;
        };
        if let Some(num) = raw.as_i64() {
            return Some(normalize_epoch_seconds(num));
        }
        if let Some(num) = raw.as_u64().and_then(|value| i64::try_from(value).ok()) {
            return Some(normalize_epoch_seconds(num));
        }
        if let Some(text) = raw
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Ok(num) = text.parse::<i64>() {
                return Some(normalize_epoch_seconds(num));
            }
            if let Ok(date) = chrono::DateTime::parse_from_rfc3339(text) {
                return Some(date.timestamp());
            }
        }
    }
    None
}

fn normalize_epoch_seconds(value: i64) -> i64 {
    if value > 10_000_000_000 {
        value / 1000
    } else {
        value
    }
}

fn generate_device_id() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_auth_state() -> AuthState {
        let dir = tempdir().expect("temp dir");
        AuthState {
            tokens: None,
            client_metadata: ClaudeClientMetadataState {
                device_id: "device-id".to_string(),
                account_uuid: Some("account-uuid".to_string()),
            },
            client_metadata_path: dir.path().join(CLIENT_METADATA_FILE),
            pending_verifier: None,
            pending_state: None,
        }
    }

    #[test]
    fn authorize_url_uses_independent_state_parameter() {
        let url = build_authorize_url("verifier-value", "state-value").url;
        assert!(url.contains("code_challenge=verifier-value"));
        assert!(url.contains("&state=state-value"));
        assert!(!url.contains("&state=verifier-value"));
    }

    #[test]
    fn parse_authorization_code_requires_state_suffix() {
        let parsed = parse_authorization_code("abc123#state456").expect("parse");
        assert_eq!(parsed.0, "abc123");
        assert_eq!(parsed.1, "state456");

        let err = parse_authorization_code("abc123").unwrap_err();
        assert!(err.contains("code#state"));
    }

    #[test]
    fn get_authorize_url_stores_distinct_pending_values() {
        let mut auth = test_auth_state();
        let info = auth.get_authorize_url();

        let pending_state = auth.pending_state.as_ref().expect("pending state");

        assert!(!pending_state.is_empty());
        assert!(info.url.contains(&format!("&state={}", pending_state)));
    }

    #[test]
    fn parses_claude_code_oauth_credentials() {
        let value = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "access-token",
                "refreshToken": "refresh-token",
                "expiresAt": 1_700_000_000_000i64
            }
        });

        let parsed = parse_claude_code_oauth_credentials(&value).expect("credentials");

        assert_eq!(parsed.access_token, "access-token");
        assert_eq!(parsed.refresh_token, "refresh-token");
        assert_eq!(parsed.expires_at, Some(1_700_000_000));
    }

    #[test]
    fn parses_claude_code_oauth_credentials_with_refresh_only() {
        let value = serde_json::json!({
            "claudeAiOauth": {
                "refreshToken": "refresh-token"
            }
        });

        let parsed = parse_claude_code_oauth_credentials(&value).expect("credentials");

        assert!(parsed.access_token.is_empty());
        assert_eq!(parsed.refresh_token, "refresh-token");
    }

    #[test]
    fn ignores_claude_code_credentials_without_tokens() {
        let value = serde_json::json!({
            "claudeAiOauth": {
                "tokenType": "Bearer"
            }
        });

        assert!(parse_claude_code_oauth_credentials(&value).is_none());
    }

    #[test]
    fn parses_claude_desktop_oauth_token_cache() {
        let future_ms = 4_102_444_800_000i64;
        let value = serde_json::json!({
            "other-client:https://api.anthropic.com:user:inference user:office": {
                "token": "office-token",
                "expiresAt": future_ms
            },
            format!("{}:org:https://api.anthropic.com:user:inference user:file_upload user:profile user:sessions:claude_code", CLIENT_ID): {
                "token": "code-token",
                "refreshToken": "refresh-token",
                "expiresAt": future_ms
            },
            format!("{}:org:https://api.anthropic.com:user:inference user:file_upload user:profile", CLIENT_ID): {
                "token": "profile-token",
                "refreshToken": "profile-refresh-token",
                "expiresAt": future_ms
            }
        });

        let parsed = parse_claude_desktop_oauth_token_cache(&value).expect("desktop token");

        assert_eq!(parsed.access_token, "code-token");
        assert_eq!(parsed.refresh_token, "refresh-token");
        assert_eq!(parsed.expires_at, Some(4_102_444_800));
    }

    #[test]
    fn ignores_expired_claude_desktop_oauth_token_cache_entries() {
        let expired_ms = 1_700_000_000_000i64;
        let value = serde_json::json!({
            format!("{}:org:https://api.anthropic.com:user:inference user:profile user:sessions:claude_code", CLIENT_ID): {
                "token": "expired-token",
                "expiresAt": expired_ms
            }
        });

        assert!(parse_claude_desktop_oauth_token_cache(&value).is_none());
    }

    #[test]
    fn parses_claude_code_custom_endpoint_from_settings_env() {
        let value = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:15721",
                "ANTHROPIC_AUTH_TOKEN": "Bearer sk-proxy",
                "ANTHROPIC_MODEL": "claude-opus-4.8"
            }
        });

        let endpoint = build_claude_code_custom_endpoint(
            parse_claude_code_custom_endpoint_settings(&value).expect("custom endpoint"),
        );

        assert_eq!(endpoint.id, "claude-code-import");
        assert_eq!(endpoint.name, "Claude Code");
        assert_eq!(endpoint.endpoint, "http://127.0.0.1:15721/v1");
        assert_eq!(endpoint.api_key, "sk-proxy");
        assert_eq!(endpoint.api_model, "claude-opus-4-8");
        assert_eq!(endpoint.context_length, 1_000_000);
        assert_eq!(endpoint.api_format, ApiFormat::AnthropicMessages);
        assert_eq!(
            endpoint.reasoning_param_format,
            Some(CustomReasoningParamFormat::AnthropicThinking)
        );
    }

    #[test]
    fn parses_claude_code_custom_endpoint_from_ccswitch_top_level_fields() {
        let value = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.kimi.com/coding/",
                "ANTHROPIC_AUTH_TOKEN": "",
                "ANTHROPIC_MODEL": "claude-sonnet-4.6"
            },
            "apiKey": "sk-kimi",
            "base_url": "https://api.kimi.com/coding/v1"
        });

        let endpoint = build_claude_code_custom_endpoint(
            parse_claude_code_custom_endpoint_settings(&value).expect("custom endpoint"),
        );

        assert_eq!(endpoint.endpoint, "https://api.kimi.com/coding/v1");
        assert_eq!(endpoint.api_key, "sk-kimi");
        assert_eq!(endpoint.api_model, "claude-sonnet-4-6");
        assert_eq!(endpoint.context_length, 1_000_000);
    }

    #[test]
    fn defaults_claude_code_custom_endpoint_to_opus_4_8() {
        let endpoint = build_claude_code_custom_endpoint(ImportedClaudeCodeCustomEndpoint {
            api_key: Some("sk-official".to_string()),
            ..Default::default()
        });

        assert_eq!(endpoint.endpoint, "https://api.anthropic.com/v1");
        assert_eq!(endpoint.api_key, "sk-official");
        assert_eq!(endpoint.api_model, "claude-opus-4-8");
        assert_eq!(endpoint.context_length, 1_000_000);
    }

    #[test]
    fn normalizes_claude_code_custom_endpoint_messages_url() {
        let endpoint = build_claude_code_custom_endpoint(ImportedClaudeCodeCustomEndpoint {
            base_url: Some("https://proxy.example/v1/messages".to_string()),
            api_key: Some("sk-proxy".to_string()),
            api_model: Some("claude_code/claude-opus-4.8[1m]".to_string()),
        });

        assert_eq!(endpoint.endpoint, "https://proxy.example/v1");
        assert_eq!(endpoint.api_model, "claude-opus-4-8");
    }

    #[test]
    fn maps_claude_code_custom_endpoint_aliases_to_current_models() {
        let sonnet = build_claude_code_custom_endpoint(ImportedClaudeCodeCustomEndpoint {
            api_key: Some("sk-proxy".to_string()),
            api_model: Some("sonnet".to_string()),
            ..Default::default()
        });
        let fable = build_claude_code_custom_endpoint(ImportedClaudeCodeCustomEndpoint {
            api_key: Some("sk-proxy".to_string()),
            api_model: Some("fable".to_string()),
            ..Default::default()
        });

        assert_eq!(sonnet.api_model, "claude-sonnet-5");
        assert_eq!(sonnet.context_length, 1_000_000);
        assert_eq!(fable.api_model, "claude-fable-5");
        assert_eq!(fable.context_length, 1_000_000);
    }

    #[tokio::test]
    async fn exchange_state_mismatch_is_rejected_before_token_request() {
        let mut auth = test_auth_state();
        auth.pending_verifier = Some("verifier-value".to_string());
        auth.pending_state = Some("expected-state".to_string());

        let err = auth
            .exchange("auth-code#wrong-state")
            .await
            .expect_err("state mismatch should fail");
        assert!(err.contains("state mismatch"));
        assert!(auth.pending_verifier.is_none());
        assert!(auth.pending_state.is_none());
    }
}
