use serde::{Deserialize, Serialize};

use crate::keychain;

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
#[allow(dead_code)]
pub const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const DEVICE_URL: &str = "https://auth.openai.com/codex/device";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub account_id: Option<String>,
    #[serde(default)]
    pub validation_failed: bool,
    #[serde(default)]
    pub validation_error: Option<String>,
}

#[derive(Serialize)]
struct UserCodeRequest {
    client_id: String,
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    user_code: String,
    interval: String,
}

#[derive(Serialize)]
struct DeviceTokenRequest {
    device_auth_id: String,
    user_code: String,
}

#[derive(Deserialize)]
struct DeviceTokenSuccess {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
    id_token: Option<String>,
}

fn default_expires_in() -> u64 {
    3600
}

#[derive(Deserialize)]
struct RefreshTokenResponse {
    access_token: String,
    refresh_token: String,
    #[serde(default = "default_expires_in")]
    expires_in: u64,
    id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexLoginInfo {
    pub user_code: String,
    pub url: String,
    pub device_auth_id: String,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum CodexPollResult {
    Pending,
    Success,
    Failed { message: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    pub authenticated: bool,
    pub account_id: Option<String>,
    pub validation_failed: bool,
    pub validation_error: Option<String>,
}

const REFRESH_TOKEN_EXPIRED_CODE: &str = "refresh_token_expired";
const REFRESH_TOKEN_REUSED_CODE: &str = "refresh_token_reused";
const REFRESH_TOKEN_INVALIDATED_CODE: &str = "refresh_token_invalidated";

const REFRESH_TOKEN_EXPIRED_MESSAGE: &str =
    "ChatGPT 订阅验证失败：refresh token 已过期。请重试验证或重新登录。";
const REFRESH_TOKEN_REUSED_MESSAGE: &str =
    "ChatGPT 订阅验证失败：refresh token 已失效。请重试验证或重新登录。";
const REFRESH_TOKEN_INVALIDATED_MESSAGE: &str =
    "ChatGPT 订阅验证失败：refresh token 已被撤销。请重试验证或重新登录。";
const REFRESH_TIMEOUT_MESSAGE: &str = "ChatGPT 订阅验证失败：刷新请求超时。请检查网络后重试验证。";
const REFRESH_CONNECT_MESSAGE: &str =
    "ChatGPT 订阅验证失败：无法连接验证服务。请检查网络后重试验证。";

fn extract_account_id_from_jwt(token: &str) -> Option<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let padding = (4 - parts[1].len() % 4) % 4;
    let padded = format!("{}{}", parts[1], "=".repeat(padding));

    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(&padded)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    if let Some(id) = claims.get("chatgpt_account_id").and_then(|v| v.as_str()) {
        return Some(id.to_string());
    }
    if let Some(id) = claims
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
    {
        return Some(id.to_string());
    }
    if let Some(id) = claims
        .get("organizations")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|o| o.get("id"))
        .and_then(|v| v.as_str())
    {
        return Some(id.to_string());
    }
    None
}

fn extract_account_id(access_token: &str, id_token: Option<&str>) -> Option<String> {
    if let Some(id_tok) = id_token {
        if let Some(id) = extract_account_id_from_jwt(id_tok) {
            return Some(id);
        }
    }
    extract_account_id_from_jwt(access_token)
}

/// Decode the base64url-encoded payload of a `header.payload.signature` JWT and
/// return the `exp` claim (seconds since epoch) converted to milliseconds. The
/// decode mirrors `extract_account_id_from_jwt`. Returns `None` when the token
/// is malformed or has no numeric `exp`.
fn extract_jwt_exp_millis(token: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let padding = (4 - parts[1].len() % 4) % 4;
    let padded = format!("{}{}", parts[1], "=".repeat(padding));

    use base64::Engine;
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(&padded)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    claims
        .get("exp")
        .and_then(serde_json::Value::as_i64)
        .map(|exp_secs| exp_secs * 1000)
}

/// Tokens extracted from a Codex CLI `auth.json`. Returned by
/// [`parse_codex_cli_auth`] so the field extraction can be unit-tested without
/// touching the keychain. `api_key_only` flags the case where the file holds a
/// non-empty `OPENAI_API_KEY` but no OAuth tokens (Codex CLI in API-key mode).
#[derive(Debug, Default, PartialEq, Eq)]
struct ImportedCodexTokens {
    access_token: String,
    refresh_token: String,
    id_token: Option<String>,
    account_id: Option<String>,
    api_key_only: bool,
}

fn json_str_field(obj: &serde_json::Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(serde_json::Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Pure extraction of the relevant fields from a parsed Codex CLI `auth.json`.
/// Accepts the standard `{ "tokens": { ... } }` shape, falls back to tokens at
/// the document root, and tolerates a non-null `OPENAI_API_KEY` (API-key mode).
/// Returns `None` only when the JSON value is not an object.
fn parse_codex_cli_auth(value: &serde_json::Value) -> Option<ImportedCodexTokens> {
    if !value.is_object() {
        return None;
    }

    // Prefer the nested `tokens` object; fall back to the document root so we
    // tolerate format drift where the tokens live at the top level.
    let tokens_obj = value.get("tokens").filter(|v| v.is_object());
    let source = tokens_obj.unwrap_or(value);

    let access_token = json_str_field(source, "access_token").unwrap_or_default();
    let refresh_token = json_str_field(source, "refresh_token").unwrap_or_default();
    let id_token = json_str_field(source, "id_token");
    // The account id can live next to the tokens or, depending on CLI version,
    // at the document root.
    let account_id =
        json_str_field(source, "account_id").or_else(|| json_str_field(value, "account_id"));

    let api_key_only = access_token.is_empty()
        && refresh_token.is_empty()
        && json_str_field(value, "OPENAI_API_KEY").is_some();

    Some(ImportedCodexTokens {
        access_token,
        refresh_token,
        id_token,
        account_id,
        api_key_only,
    })
}

/// Resolve the Codex CLI `auth.json` path: honor `CODEX_HOME` first, else
/// `~/.codex`.
fn codex_cli_auth_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?;
    Some(home.join("auth.json"))
}

fn extract_refresh_token_error_code(body: &str) -> Option<String> {
    if body.trim().is_empty() {
        return None;
    }

    let serde_json::Value::Object(map) = serde_json::from_str::<serde_json::Value>(body).ok()?
    else {
        return None;
    };

    if let Some(error_value) = map.get("error") {
        match error_value {
            serde_json::Value::Object(obj) => {
                if let Some(code) = obj.get("code").and_then(serde_json::Value::as_str) {
                    return Some(code.to_string());
                }
            }
            serde_json::Value::String(code) => return Some(code.to_string()),
            _ => {}
        }
    }

    map.get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn refresh_failure_invalidates_login(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::UNAUTHORIZED {
        return false;
    }

    matches!(
        extract_refresh_token_error_code(body)
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            REFRESH_TOKEN_EXPIRED_CODE | REFRESH_TOKEN_REUSED_CODE | REFRESH_TOKEN_INVALIDATED_CODE
        )
    )
}

fn refresh_http_failure_message(status: reqwest::StatusCode, body: &str) -> String {
    match extract_refresh_token_error_code(body)
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some(REFRESH_TOKEN_EXPIRED_CODE) => REFRESH_TOKEN_EXPIRED_MESSAGE.to_string(),
        Some(REFRESH_TOKEN_REUSED_CODE) => REFRESH_TOKEN_REUSED_MESSAGE.to_string(),
        Some(REFRESH_TOKEN_INVALIDATED_CODE) => REFRESH_TOKEN_INVALIDATED_MESSAGE.to_string(),
        _ => format!(
            "ChatGPT 订阅验证失败：服务端返回 HTTP {}。请重试验证。",
            status.as_u16()
        ),
    }
}

fn refresh_request_failure_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return REFRESH_TIMEOUT_MESSAGE.to_string();
    }
    if error.is_connect() {
        return REFRESH_CONNECT_MESSAGE.to_string();
    }

    "ChatGPT 订阅验证失败：刷新请求未完成。请重试验证。".to_string()
}

// ── CodexAuthState ──

#[derive(Debug)]
pub struct CodexAuthState {
    tokens: Option<CodexTokenData>,
}

impl CodexAuthState {
    pub fn new(_data_dir: &std::path::Path) -> Self {
        let tokens = Self::load_from_keychain();

        if tokens.is_some() {
            eprintln!("[Codex] loaded existing tokens from keychain");
        } else {
            eprintln!("[Codex] no existing tokens found in keychain");
        }
        CodexAuthState { tokens }
    }

    pub fn is_authenticated(&self) -> bool {
        self.tokens.is_some()
    }

    pub fn status(&self) -> CodexStatus {
        CodexStatus {
            authenticated: self.is_authenticated(),
            account_id: self.tokens.as_ref().and_then(|t| t.account_id.clone()),
            validation_failed: self
                .tokens
                .as_ref()
                .map(|t| t.validation_failed)
                .unwrap_or(false),
            validation_error: self
                .tokens
                .as_ref()
                .and_then(|t| t.validation_error.clone()),
        }
    }

    pub async fn access_token(&mut self) -> Result<String, String> {
        let tokens = self.tokens.as_ref().ok_or("Not authenticated")?;
        let now_ms = chrono::Utc::now().timestamp_millis();

        if now_ms >= tokens.expires_at - 60_000 {
            if tokens.validation_failed {
                return Err(tokens
                    .validation_error
                    .clone()
                    .unwrap_or_else(|| "ChatGPT 订阅验证失败。请在设置中重试验证。".to_string()));
            }
            eprintln!("[Codex] token expiring, refreshing...");
            self.refresh().await?;
        }

        Ok(self
            .tokens
            .as_ref()
            .ok_or("Token unavailable after refresh")?
            .access_token
            .clone())
    }

    pub fn account_id(&self) -> Option<String> {
        self.tokens.as_ref().and_then(|t| t.account_id.clone())
    }

    pub async fn retry_validation(&mut self) -> Result<CodexStatus, String> {
        if !self.is_authenticated() {
            return Err("Not authenticated".to_string());
        }
        self.refresh().await?;
        Ok(self.status())
    }

    /// Import the ChatGPT-subscription OAuth tokens that the OpenAI Codex CLI
    /// persists at `$CODEX_HOME/auth.json` (default `~/.codex/auth.json`) into
    /// Locus's Codex auth state, so a user who already ran `codex login` can
    /// reuse that session without the device-code flow.
    ///
    /// Codex CLI uses the same OAuth `CLIENT_ID` as Locus, so the imported
    /// refresh token works against the existing [`Self::refresh`] path.
    ///
    /// Caveat: OpenAI refresh tokens can rotate. Once Locus refreshes the
    /// imported token, the Codex CLI's own stored refresh token may be
    /// invalidated — inherent to importing a shared, possibly-rotating
    /// credential. Re-importing is idempotent and overwrites the stored tokens.
    pub async fn import_codex_cli_tokens(&mut self) -> Result<CodexStatus, String> {
        let path = codex_cli_auth_path()
            .ok_or_else(|| "无法解析 Codex CLI 凭据路径（找不到 home 目录）。".to_string())?;

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err("未找到 Codex CLI 登录凭据，请先运行 `codex login`。".to_string());
            }
            Err(err) => {
                return Err(format!("读取 Codex CLI 凭据失败：{err}"));
            }
        };

        let value: serde_json::Value = serde_json::from_str(&contents)
            .map_err(|err| format!("解析 Codex CLI 凭据失败：{err}"))?;

        let imported = parse_codex_cli_auth(&value)
            .ok_or_else(|| "Codex CLI 凭据格式无法识别。".to_string())?;

        if imported.access_token.is_empty() && imported.refresh_token.is_empty() {
            if imported.api_key_only {
                return Err("检测到 Codex CLI 使用 API key 模式，暂不支持导入；请用 ChatGPT 登录（codex login）后再导入。".to_string());
            }
            return Err("Codex CLI 凭据为空。".to_string());
        }

        let account_id = imported
            .account_id
            .clone()
            .or_else(|| extract_account_id(&imported.access_token, imported.id_token.as_deref()));

        // Codex CLI's auth.json does not store an expiry; derive it from the
        // access token's `exp` claim. If that is unavailable, treat the token as
        // already expired so the first `access_token()` call refreshes it.
        let expires_at = extract_jwt_exp_millis(&imported.access_token)
            .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

        self.tokens = Some(CodexTokenData {
            access_token: imported.access_token,
            refresh_token: imported.refresh_token,
            expires_at,
            account_id,
            validation_failed: false,
            validation_error: None,
        });
        self.save_tokens()?;

        eprintln!("[Codex] imported tokens from Codex CLI auth.json");
        Ok(self.status())
    }

    pub async fn start_login(&self) -> Result<CodexLoginInfo, String> {
        let client = crate::network::default_reqwest_client()?;
        let url = format!("{}/api/accounts/deviceauth/usercode", ISSUER);

        let resp = client
            .post(&url)
            .json(&UserCodeRequest {
                client_id: CLIENT_ID.to_string(),
            })
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Device auth request failed: {body}"));
        }

        let data: UserCodeResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let interval = data.interval.parse::<u64>().unwrap_or(5).max(1);

        Ok(CodexLoginInfo {
            user_code: data.user_code,
            url: DEVICE_URL.to_string(),
            device_auth_id: data.device_auth_id,
            interval,
        })
    }

    pub async fn poll_login(
        &mut self,
        device_auth_id: &str,
        user_code: &str,
    ) -> Result<CodexPollResult, String> {
        // The UI may enqueue one more poll while a successful exchange is still
        // propagating back to the frontend. Make late duplicate polls idempotent
        // and ensure tokens are persisted to keychain.
        if self.is_authenticated() {
            let _ = self.save_tokens();
            return Ok(CodexPollResult::Success);
        }

        let client = crate::network::default_reqwest_client()?;
        let url = format!("{}/api/accounts/deviceauth/token", ISSUER);

        let resp = client
            .post(&url)
            .json(&DeviceTokenRequest {
                device_auth_id: device_auth_id.to_string(),
                user_code: user_code.to_string(),
            })
            .send()
            .await
            .map_err(|e| format!("Poll request failed: {e}"))?;

        let status = resp.status();

        if status == 403 || status == 404 {
            return Ok(CodexPollResult::Pending);
        }

        if !status.is_success() {
            return Ok(CodexPollResult::Failed {
                message: format!("Poll failed with status {status}"),
            });
        }

        let data: DeviceTokenSuccess = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse poll response: {e}"))?;

        self.exchange_code(&data.authorization_code, &data.code_verifier)
            .await
    }

    async fn exchange_code(
        &mut self,
        authorization_code: &str,
        code_verifier: &str,
    ) -> Result<CodexPollResult, String> {
        if self.is_authenticated() {
            return Ok(CodexPollResult::Success);
        }

        let client = crate::network::default_reqwest_client()?;

        let params = [
            ("grant_type", "authorization_code"),
            ("code", authorization_code),
            ("redirect_uri", &format!("{}/deviceauth/callback", ISSUER)),
            ("client_id", CLIENT_ID),
            ("code_verifier", code_verifier),
        ];

        let resp = client
            .post(format!("{}/oauth/token", ISSUER))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await
            .map_err(|e| format!("Token exchange request failed: {e}"))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Ok(CodexPollResult::Failed {
                message: format!("Token exchange failed: {body}"),
            });
        }

        let token_resp: OAuthTokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse token response: {e}"))?;

        let account_id =
            extract_account_id(&token_resp.access_token, token_resp.id_token.as_deref());

        let now_ms = chrono::Utc::now().timestamp_millis();
        self.tokens = Some(CodexTokenData {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at: now_ms + (token_resp.expires_in as i64 * 1000),
            account_id,
            validation_failed: false,
            validation_error: None,
        });
        self.save_tokens()?;

        eprintln!("[Codex] login success");
        Ok(CodexPollResult::Success)
    }

    async fn refresh(&mut self) -> Result<(), String> {
        let refresh_token = self
            .tokens
            .as_ref()
            .map(|t| t.refresh_token.clone())
            .ok_or("No refresh token")?;

        let client = crate::network::default_reqwest_client()?;
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
            ("client_id", CLIENT_ID),
        ];

        let resp = client
            .post(format!("{}/oauth/token", ISSUER))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await
            .map_err(|error| {
                let message = refresh_request_failure_message(&error);
                self.mark_validation_failed(message.clone());
                eprintln!("[Codex] refresh request failed, marked validation_failed: {error}");
                message
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let message = refresh_http_failure_message(status, &text);
            self.mark_validation_failed(message.clone());
            if refresh_failure_invalidates_login(status, &text) {
                eprintln!(
                    "[Codex] refresh failed with invalid auth but preserving local tokens (status={}): {}",
                    status.as_u16(),
                    text
                );
                return Err(message);
            }

            eprintln!(
                "[Codex] refresh failed and marked validation_failed for retry (status={}): {}",
                status.as_u16(),
                text
            );
            return Err(message);
        }

        let token_resp: RefreshTokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse refresh response: {e}"))?;

        let now_ms = chrono::Utc::now().timestamp_millis();
        if let Some(ref mut t) = self.tokens {
            let new_account_id =
                extract_account_id(&token_resp.access_token, token_resp.id_token.as_deref())
                    .or_else(|| t.account_id.clone());

            t.access_token = token_resp.access_token;
            t.refresh_token = token_resp.refresh_token;
            t.expires_at = now_ms + (token_resp.expires_in as i64 * 1000);
            t.account_id = new_account_id;
            t.validation_failed = false;
            t.validation_error = None;
        }
        self.save_tokens()?;
        eprintln!("[Codex] tokens refreshed");
        Ok(())
    }

    pub fn logout(&mut self) {
        self.clear_local_tokens();
        eprintln!("[Codex] logged out, tokens cleared from keychain");
    }

    fn clear_local_tokens(&mut self) {
        self.tokens = None;
        let _ = keychain::delete_secret(keychain::KEY_CODEX_TOKENS);
    }

    fn mark_validation_failed(&mut self, message: String) {
        if let Some(ref mut t) = self.tokens {
            t.validation_failed = true;
            t.validation_error = Some(message);
        }
        if let Err(err) = self.save_tokens() {
            eprintln!("[Codex] failed to persist validation failure state: {err}");
        }
    }

    fn save_tokens(&self) -> Result<(), String> {
        if let Some(ref t) = self.tokens {
            Self::save_to_keychain(t)?;
        }
        Ok(())
    }

    fn save_to_keychain(tokens: &CodexTokenData) -> Result<(), String> {
        let json = serde_json::to_string(tokens).map_err(|e| format!("Serialize failed: {e}"))?;
        let payload_bytes = json.len();
        let access_len = tokens.access_token.len();
        let refresh_len = tokens.refresh_token.len();
        let result = keychain::set_secret(keychain::KEY_CODEX_TOKENS, &json);
        match &result {
            Ok(()) => eprintln!(
                "[Codex] keychain write success: key={} payload_bytes={} access_len={} refresh_len={} has_account_id={} validation_failed={} expires_at={}",
                keychain::KEY_CODEX_TOKENS,
                payload_bytes,
                access_len,
                refresh_len,
                tokens.account_id.is_some(),
                tokens.validation_failed,
                tokens.expires_at
            ),
            Err(err) => eprintln!(
                "[Codex] keychain write failed: key={} payload_bytes={} access_len={} refresh_len={} has_account_id={} validation_failed={} expires_at={} error={}",
                keychain::KEY_CODEX_TOKENS,
                payload_bytes,
                access_len,
                refresh_len,
                tokens.account_id.is_some(),
                tokens.validation_failed,
                tokens.expires_at,
                err
            ),
        }
        result
    }

    fn load_from_keychain() -> Option<CodexTokenData> {
        match keychain::get_secret(keychain::KEY_CODEX_TOKENS) {
            Ok(Some(s)) => {
                let payload_bytes = s.len();
                eprintln!(
                    "[Codex] keychain read hit: key={} payload_bytes={}",
                    keychain::KEY_CODEX_TOKENS,
                    payload_bytes
                );
                match serde_json::from_str::<CodexTokenData>(&s) {
                    Ok(t) => {
                        eprintln!(
                            "[Codex] keychain parse success: key={} access_len={} refresh_len={} has_account_id={} validation_failed={} expires_at={}",
                            keychain::KEY_CODEX_TOKENS,
                            t.access_token.len(),
                            t.refresh_token.len(),
                            t.account_id.is_some(),
                            t.validation_failed,
                            t.expires_at
                        );
                        Some(t)
                    }
                    Err(e) => {
                        eprintln!(
                            "[Codex] failed to parse keychain tokens: key={} payload_bytes={} error={}",
                            keychain::KEY_CODEX_TOKENS,
                            payload_bytes,
                            e
                        );
                        None
                    }
                }
            }
            Ok(None) => {
                eprintln!(
                    "[Codex] keychain read miss: key={}",
                    keychain::KEY_CODEX_TOKENS
                );
                None
            }
            Err(e) => {
                eprintln!(
                    "[Codex] keychain read error: key={} error={}",
                    keychain::KEY_CODEX_TOKENS,
                    e
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_jwt_exp_millis, extract_refresh_token_error_code, parse_codex_cli_auth,
        refresh_failure_invalidates_login,
    };

    /// Build a syntactically valid `header.payload.signature` JWT whose payload
    /// is the base64url encoding of `payload_json`. Header/signature are dummy.
    fn make_jwt(payload_json: &str) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        format!("header.{payload}.signature")
    }

    #[test]
    fn parse_codex_cli_auth_reads_standard_tokens_shape() {
        let value = serde_json::json!({
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": "id-jwt",
                "access_token": "  access-jwt  ",
                "refresh_token": "refresh-opaque",
                "account_id": "acct-123"
            },
            "last_refresh": "2026-06-27T00:00:00Z"
        });

        let parsed = parse_codex_cli_auth(&value).expect("object");
        assert_eq!(parsed.access_token, "access-jwt");
        assert_eq!(parsed.refresh_token, "refresh-opaque");
        assert_eq!(parsed.id_token.as_deref(), Some("id-jwt"));
        assert_eq!(parsed.account_id.as_deref(), Some("acct-123"));
        assert!(!parsed.api_key_only);
    }

    #[test]
    fn parse_codex_cli_auth_flags_api_key_only_mode() {
        let value = serde_json::json!({
            "OPENAI_API_KEY": "sk-abc123",
            "tokens": null
        });

        let parsed = parse_codex_cli_auth(&value).expect("object");
        assert!(parsed.access_token.is_empty());
        assert!(parsed.refresh_token.is_empty());
        assert!(parsed.api_key_only);
    }

    #[test]
    fn parse_codex_cli_auth_supports_top_level_fallback() {
        let value = serde_json::json!({
            "access_token": "root-access",
            "refresh_token": "root-refresh",
            "account_id": "root-acct"
        });

        let parsed = parse_codex_cli_auth(&value).expect("object");
        assert_eq!(parsed.access_token, "root-access");
        assert_eq!(parsed.refresh_token, "root-refresh");
        assert_eq!(parsed.account_id.as_deref(), Some("root-acct"));
        assert!(!parsed.api_key_only);
    }

    #[test]
    fn parse_codex_cli_auth_rejects_non_object() {
        assert!(parse_codex_cli_auth(&serde_json::json!("nope")).is_none());
        assert!(parse_codex_cli_auth(&serde_json::json!(["a", "b"])).is_none());
    }

    #[test]
    fn extract_jwt_exp_millis_reads_exp_claim() {
        let token = make_jwt(r#"{"exp": 1700000000}"#);
        assert_eq!(extract_jwt_exp_millis(&token), Some(1_700_000_000_000));
    }

    #[test]
    fn extract_jwt_exp_millis_handles_missing_or_malformed() {
        assert_eq!(extract_jwt_exp_millis(&make_jwt(r#"{"sub":"x"}"#)), None);
        assert_eq!(extract_jwt_exp_millis("not-a-jwt"), None);
        assert_eq!(extract_jwt_exp_millis("a.b"), None);
    }

    #[test]
    fn refresh_failure_only_invalidates_login_for_401_known_refresh_token_codes() {
        use reqwest::StatusCode;

        assert!(refresh_failure_invalidates_login(
            StatusCode::UNAUTHORIZED,
            r#"{"error":{"code":"refresh_token_expired","message":"expired"}}"#
        ));
        assert!(refresh_failure_invalidates_login(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"refresh_token_reused"}"#
        ));
        assert!(refresh_failure_invalidates_login(
            StatusCode::UNAUTHORIZED,
            r#"{"code":"refresh_token_invalidated"}"#
        ));
        assert!(!refresh_failure_invalidates_login(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"temporarily_unavailable","error_description":"upstream error"}"#
        ));
        assert!(!refresh_failure_invalidates_login(
            StatusCode::BAD_REQUEST,
            r#"{"error":{"code":"refresh_token_expired"}}"#
        ));
        assert!(!refresh_failure_invalidates_login(
            StatusCode::UNAUTHORIZED,
            "<html>gateway timeout</html>"
        ));
    }

    #[test]
    fn refresh_error_code_extraction_supports_upstream_shapes() {
        assert_eq!(
            extract_refresh_token_error_code(
                r#"{"error":{"code":"refresh_token_expired","message":"expired"}}"#
            ),
            Some("refresh_token_expired".to_string())
        );
        assert_eq!(
            extract_refresh_token_error_code(r#"{"error":"refresh_token_reused"}"#),
            Some("refresh_token_reused".to_string())
        );
        assert_eq!(
            extract_refresh_token_error_code(r#"{"code":"refresh_token_invalidated"}"#),
            Some("refresh_token_invalidated".to_string())
        );
        assert_eq!(extract_refresh_token_error_code(""), None);
        assert_eq!(
            extract_refresh_token_error_code("<html>bad gateway</html>"),
            None
        );
    }
}
