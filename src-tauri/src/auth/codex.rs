//! Desktop dependencies for the shared Codex OAuth implementation.
use std::sync::Arc;
use crate::keychain;
pub use locus_remote_core::codex_auth::{CodexTokenData, CodexLoginInfo, CodexPollResult, CodexStatus};
use locus_remote_core::codex_auth::{CodexAuthState as SharedAuthState, CodexClientFactory, CodexTokenStore};

#[derive(Debug)]
pub struct CodexAuthState {
    inner: SharedAuthState,
    #[cfg(feature = "server-client")]
    server: Option<crate::server_client::Binding>,
}

fn desktop_client() -> CodexClientFactory {
    Arc::new(|| crate::network::rustls_reqwest_client(locus_remote_core::network::codex_http_options(locus_remote_core::network::CodexHttpProfile::Authentication)))
}

impl CodexAuthState {
    pub fn new(_data_dir: &std::path::Path) -> Self {
        let store = Arc::new(DesktopTokenStore);
        // Preserve the existing desktop behavior: an unavailable vault starts signed out.
        let tokens = store.load_legacy();
        if tokens.is_some() {
            eprintln!("[Codex] loaded existing tokens from keychain");
        } else {
            eprintln!("[Codex] no existing tokens found in keychain");
        }
        Self {
            inner: SharedAuthState::from_tokens(tokens, store, desktop_client()),
            #[cfg(feature = "server-client")]
            server: None,
        }
    }

    #[cfg(feature = "server-client")]
    pub fn for_server(binding: crate::server_client::Binding) -> Self {
        Self { inner: SharedAuthState::from_tokens(None, Arc::new(DetachedTokenStore), desktop_client()), server: Some(binding) }
    }

    pub fn is_authenticated(&self) -> bool {
        #[cfg(feature = "server-client")]
        if let Some(binding) = &self.server { return crate::server_client::has_binding_credentials(binding); }
        self.inner.is_authenticated()
    }

    pub fn status(&self) -> CodexStatus {
        #[cfg(feature = "server-client")]
        if self.server.is_some() {
            return CodexStatus { authenticated: self.is_authenticated(), account_id: None, validation_failed: false, validation_error: None };
        }
        self.inner.status()
    }

    pub async fn access_token(&mut self) -> Result<String, String> {
        #[cfg(feature = "server-client")]
        if let Some(binding) = &self.server {
            return Ok(crate::server_client::credential_for_binding(&binding.id).await?.access_token);
        }
        self.inner.access_token().await
    }

    pub fn account_id(&self) -> Option<String> {
        #[cfg(feature = "server-client")]
        if self.server.is_some() { return None; }
        self.inner.account_id()
    }

    pub async fn retry_validation(&mut self) -> Result<CodexStatus, String> {
        #[cfg(feature = "server-client")]
        if let Some(binding) = &self.server {
            crate::server_client::refresh_binding(&binding.id).await?;
            return Ok(CodexStatus { authenticated: true, account_id: None, validation_failed: false, validation_error: None });
        }
        self.inner.retry_validation().await
    }

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

        self.inner.import_codex_cli_json(&value).await
    }

    #[cfg(test)]
    pub(crate) fn for_test(access_token: &str, account_id: &str) -> Self {
        Self { inner: SharedAuthState::from_tokens(Some(CodexTokenData {
            access_token: access_token.into(), refresh_token: String::new(),
            expires_at: chrono::Utc::now().timestamp_millis() + 3_600_000,
            account_id: Some(account_id.into()), validation_failed: false, validation_error: None,
        }), Arc::new(TestTokenStore), desktop_client()),
            #[cfg(feature = "server-client")]
            server: None,
        }
    }
}

impl std::ops::Deref for CodexAuthState {
    type Target = SharedAuthState;
    fn deref(&self) -> &Self::Target { &self.inner }
}
impl std::ops::DerefMut for CodexAuthState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.inner }
}

#[cfg(feature = "server-client")]
struct DetachedTokenStore;
#[cfg(feature = "server-client")]
impl CodexTokenStore for DetachedTokenStore {
    fn load(&self) -> Result<Option<CodexTokenData>, String> { Ok(None) }
    fn save(&self, _: &CodexTokenData) -> Result<(), String> { Err("服务器上游凭据由管理员管理".into()) }
    fn delete(&self) -> Result<(), String> { Ok(()) }
}

/// Resolve the Codex CLI `auth.json` path: honor `CODEX_HOME` first, else
/// `~/.codex`.
fn codex_cli_auth_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?;
    Some(home.join("auth.json"))
}

struct DesktopTokenStore;
impl CodexTokenStore for DesktopTokenStore {
    fn save(&self, tokens: &CodexTokenData) -> Result<(), String> {
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

    fn load(&self) -> Result<Option<CodexTokenData>, String> {
        Ok(self.load_legacy())
    }

    fn delete(&self) -> Result<(), String> {
        keychain::delete_secret(keychain::KEY_CODEX_TOKENS)
    }
}

impl DesktopTokenStore {
    fn load_legacy(&self) -> Option<CodexTokenData> {
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
struct TestTokenStore;
#[cfg(test)]
impl CodexTokenStore for TestTokenStore {
    fn load(&self) -> Result<Option<CodexTokenData>, String> { Ok(None) }
    fn save(&self, _: &CodexTokenData) -> Result<(), String> { Ok(()) }
    fn delete(&self) -> Result<(), String> { Ok(()) }
}
