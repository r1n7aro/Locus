use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::auth::codex::{CodexAuthState, CodexLoginInfo, CodexPollResult, CodexStatus};
use crate::auth::{AuthState, AuthStatus, AuthUrlInfo};
use crate::keychain;
use crate::llm::codex_usage::CodexRateLimitsResponse;

use crate::error::AppError;
use crate::{ApiKeyState, ProviderKeysState};

pub type CodexAuthStateHandle = Arc<tokio::sync::Mutex<CodexAuthState>>;

const HIDDEN_PROVIDER_IDS: &[&str] = &[];

fn is_provider_hidden(provider_id: &str) -> bool {
    HIDDEN_PROVIDER_IDS.contains(&provider_id)
}

#[tauri::command]
pub async fn get_auth_status(
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
    api_key_state: State<'_, ApiKeyState>,
) -> Result<AuthStatus, AppError> {
    let auth = auth.lock().await;
    let key = api_key_state.read().await;
    Ok(AuthStatus {
        authenticated: auth.is_authenticated(),
        has_api_key: !key.is_empty(),
        email: auth.email(),
    })
}

#[tauri::command]
pub async fn get_auth_url(
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
) -> Result<AuthUrlInfo, AppError> {
    let mut auth = auth.lock().await;
    Ok(auth.get_authorize_url())
}

#[tauri::command]
pub async fn exchange_auth_code(
    code: String,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
) -> Result<bool, AppError> {
    let mut auth = auth.lock().await;
    auth.exchange(&code).await?;
    Ok(true)
}

#[tauri::command]
pub async fn auth_logout(
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
) -> Result<(), AppError> {
    let mut auth = auth.lock().await;
    auth.logout();
    Ok(())
}

#[tauri::command]
pub async fn save_api_key(
    key: String,
    api_key_state: State<'_, ApiKeyState>,
    _app_handle: AppHandle,
) -> Result<bool, AppError> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".into());
    }

    {
        let mut state = api_key_state.write().await;
        *state = key.clone();
    }

    keychain::set_secret(keychain::KEY_OPENROUTER, &key)?;

    eprintln!("[Locus] OpenRouter API key saved to keychain");
    Ok(true)
}

#[tauri::command]
pub async fn clear_api_key(
    api_key_state: State<'_, ApiKeyState>,
    _app_handle: AppHandle,
) -> Result<(), AppError> {
    {
        let mut state = api_key_state.write().await;
        *state = String::new();
    }

    keychain::delete_secret(keychain::KEY_OPENROUTER)?;

    eprintln!("[Locus] OpenRouter API key cleared from keychain");
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub has_key: bool,
    pub key_hint: String,
    /// Auth state for providers whose credentials live outside Locus
    /// (currently only `claude_code`): `Some(false)` means installed but not
    /// logged in; `None` means unknown or not applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logged_in: Option<bool>,
}

#[tauri::command]
pub async fn get_providers(
    api_key_state: State<'_, ApiKeyState>,
    provider_keys: State<'_, ProviderKeysState>,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
) -> Result<Vec<ProviderStatus>, AppError> {
    let openrouter_key = api_key_state.read().await.clone();
    let keys = provider_keys.read().await;
    let auth_guard = auth.lock().await;
    let (claude_code_available, claude_code_hint) =
        crate::llm::claude_code_cli::claude_cli_status();
    let claude_code_logged_in = if claude_code_available {
        match crate::llm::claude_code_cli::claude_cli_login_status().0 {
            crate::llm::claude_code_cli::ClaudeCliLoginState::LoggedIn => Some(true),
            crate::llm::claude_code_cli::ClaudeCliLoginState::LoggedOut => Some(false),
            crate::llm::claude_code_cli::ClaudeCliLoginState::Unknown => None,
        }
    } else {
        None
    };

    let mut providers = vec![
        ProviderStatus {
            id: "openrouter".to_string(),
            name: "OpenRouter".to_string(),
            has_key: !openrouter_key.is_empty(),
            key_hint: mask_key(&openrouter_key),
            logged_in: None,
        },
        ProviderStatus {
            id: "anthropic".to_string(),
            name: "Anthropic (OAuth)".to_string(),
            has_key: auth_guard.is_authenticated(),
            key_hint: auth_guard.email().unwrap_or_default(),
            logged_in: None,
        },
    ];

    if !is_provider_hidden("claude_code") {
        providers.push(ProviderStatus {
            id: "claude_code".to_string(),
            name: "Claude Code CLI".to_string(),
            has_key: claude_code_available,
            key_hint: claude_code_hint,
            logged_in: claude_code_logged_in,
        });
    }

    for (id, key) in keys.iter() {
        if id != "openrouter" && !is_provider_hidden(id) {
            providers.push(ProviderStatus {
                id: id.clone(),
                name: provider_display_name(id),
                has_key: !key.is_empty(),
                key_hint: mask_key(key),
                logged_in: None,
            });
        }
    }

    let known: [&str; 0] = [];
    for id in &known {
        if is_provider_hidden(id) {
            continue;
        }
        if !providers.iter().any(|p| p.id == *id) {
            providers.push(ProviderStatus {
                id: id.to_string(),
                name: provider_display_name(id),
                has_key: false,
                key_hint: String::new(),
                logged_in: None,
            });
        }
    }

    Ok(providers)
}

/// Live connectivity / auth check for the Claude Code CLI: spawns the CLI
/// exactly as a real turn would (same hermetic flags, full environment, proxy)
/// and reports the outcome. Returns the model reply on success, or a
/// human-readable error explaining why the turn could not complete.
#[tauri::command]
pub async fn test_claude_code_cli() -> Result<String, AppError> {
    crate::llm::claude_code_cli::run_login_test()
        .await
        .map_err(|e| e.into())
}

#[tauri::command]
pub async fn save_provider_key(
    provider: String,
    key: String,
    api_key_state: State<'_, ApiKeyState>,
    provider_keys: State<'_, ProviderKeysState>,
    app_handle: AppHandle,
) -> Result<bool, AppError> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".into());
    }

    if provider == "openrouter" {
        let mut state = api_key_state.write().await;
        *state = key.clone();
        keychain::set_secret(keychain::KEY_OPENROUTER, &key)?;
    } else {
        let mut keys = provider_keys.write().await;
        keys.insert(provider.clone(), key.clone());
        keychain::set_secret(&keychain::provider_key_name(&provider), &key)?;
        // Update the index file so we know which provider IDs exist
        if let Ok(data_dir) = crate::commands::resolve_runtime_storage_dir(&app_handle) {
            let _ = save_provider_key_index(&data_dir, &keys);
        }
    }

    eprintln!("[Locus] provider key saved to keychain: {}", provider);
    Ok(true)
}

#[tauri::command]
pub async fn delete_provider_key(
    provider: String,
    api_key_state: State<'_, ApiKeyState>,
    provider_keys: State<'_, ProviderKeysState>,
    app_handle: AppHandle,
) -> Result<(), AppError> {
    if provider == "openrouter" {
        let mut state = api_key_state.write().await;
        *state = String::new();
        keychain::delete_secret(keychain::KEY_OPENROUTER)?;
    } else {
        let mut keys = provider_keys.write().await;
        keys.remove(&provider);
        keychain::delete_secret(&keychain::provider_key_name(&provider))?;
        if let Ok(data_dir) = crate::commands::resolve_runtime_storage_dir(&app_handle) {
            let _ = save_provider_key_index(&data_dir, &keys);
        }
    }

    eprintln!("[Locus] provider key deleted from keychain: {}", provider);
    Ok(())
}

fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if key.chars().count() <= 8 {
        return "****".to_string();
    }
    let prefix = key.chars().take(6).collect::<String>();
    let suffix = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("{}...{}", prefix, suffix)
}

fn provider_display_name(id: &str) -> String {
    match id {
        "openrouter" => "OpenRouter".to_string(),
        "anthropic" => "Anthropic (OAuth)".to_string(),
        "claude_code" => "Claude Code CLI".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::mask_key;

    #[test]
    fn mask_key_handles_unicode_boundaries() {
        assert_eq!(mask_key("中文中文中文中文中"), "中文中文中文...文中文中");
    }
}

/// Load provider keys from OS keychain.
/// We store a JSON list of provider IDs so we know which keys to look up.
pub fn load_provider_keys_from_keychain(data_dir: &std::path::Path) -> HashMap<String, String> {
    // Load index of provider IDs
    let index_path = data_dir.join("provider_key_ids.json");
    let ids: Vec<String> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut result = HashMap::new();
    for id in ids {
        if let Ok(Some(key)) = keychain::get_secret(&keychain::provider_key_name(&id)) {
            if !key.is_empty() {
                result.insert(id, key);
            }
        }
    }
    result
}

/// Persist the provider key ID index (not the secrets themselves).
fn save_provider_key_index(
    data_dir: &std::path::Path,
    keys: &HashMap<String, String>,
) -> Result<(), String> {
    let ids: Vec<String> = keys.keys().cloned().collect();
    let index_path = data_dir.join("provider_key_ids.json");
    let json = serde_json::to_string(&ids)
        .map_err(|e| format!("Failed to serialize provider key index: {}", e))?;
    std::fs::write(&index_path, json)
        .map_err(|e| format!("Failed to write provider key index: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn codex_status(codex: State<'_, CodexAuthStateHandle>) -> Result<CodexStatus, AppError> {
    Ok(codex.lock().await.status())
}

#[tauri::command]
pub async fn codex_start_login(
    codex: State<'_, CodexAuthStateHandle>,
) -> Result<CodexLoginInfo, AppError> {
    codex
        .lock()
        .await
        .start_login()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn codex_poll_login(
    device_auth_id: String,
    user_code: String,
    codex: State<'_, CodexAuthStateHandle>,
) -> Result<CodexPollResult, AppError> {
    codex
        .lock()
        .await
        .poll_login(&device_auth_id, &user_code)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn codex_logout(codex: State<'_, CodexAuthStateHandle>) -> Result<(), AppError> {
    codex.lock().await.logout();
    Ok(())
}

#[tauri::command]
pub async fn codex_retry_auth(
    codex: State<'_, CodexAuthStateHandle>,
) -> Result<CodexStatus, AppError> {
    codex
        .lock()
        .await
        .retry_validation()
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn codex_rate_limits(
    codex: State<'_, CodexAuthStateHandle>,
    config: State<'_, Arc<crate::config::AppConfig>>,
) -> Result<CodexRateLimitsResponse, AppError> {
    let (access_token, account_id) = {
        let mut guard = codex.lock().await;
        let access_token = guard.access_token().await.map_err(AppError::from)?;
        let account_id = guard.account_id();
        (access_token, account_id)
    };

    match crate::llm::codex_usage::fetch_codex_rate_limits(
        &access_token,
        account_id.as_deref(),
        config.base_url.as_deref(),
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(error) if error.is_unauthorized() => {
            let (access_token, account_id) = {
                let mut guard = codex.lock().await;
                guard.retry_validation().await.map_err(AppError::from)?;
                let access_token = guard.access_token().await.map_err(AppError::from)?;
                let account_id = guard.account_id();
                (access_token, account_id)
            };

            crate::llm::codex_usage::fetch_codex_rate_limits(
                &access_token,
                account_id.as_deref(),
                config.base_url.as_deref(),
            )
            .await
            .map_err(|err| AppError::from(err.to_string()))
        }
        Err(error) => Err(AppError::from(error.to_string())),
    }
}
