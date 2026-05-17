pub mod codex;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use crate::keychain;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_AI_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPE: &str = "org:create_api_key user:profile user:inference";
const CLIENT_METADATA_FILE: &str = "claude_client_metadata.json";

const REFRESH_BUFFER_SECS: i64 = 60;

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

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
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
            refresh_token: token_resp.refresh_token,
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

        let client = crate::network::default_reqwest_client()?;

        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
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
            self.tokens = None;
            let _ = keychain::delete_secret(keychain::KEY_CLAUDE_TOKENS);
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
            refresh_token: token_resp.refresh_token,
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
        percent_encode(SCOPE),
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
