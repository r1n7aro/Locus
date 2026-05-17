use super::CODEX_CLIENT_VERSION;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_CODEX_PROVIDER_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const MODELS_ENDPOINT_PATH: &str = "/models";
const RESPONSES_ENDPOINT_PATH: &str = "/responses";
const MODEL_CACHE_FILE: &str = "codex_models_cache.json";
const MODEL_CACHE_TTL_MS: i64 = 300_000;
const MODELS_REFRESH_TIMEOUT_SECS: u64 = 5;
const CODEX_ORIGINATOR_HEADER_VALUE: &str = "opencode";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexAvailableModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_speed_tiers: Vec<String>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexModelsResponse {
    #[serde(default)]
    models: Vec<CodexRemoteModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexRemoteModel {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    default_reasoning_level: Option<String>,
    #[serde(default)]
    supported_reasoning_levels: Vec<CodexReasoningLevel>,
    #[serde(default)]
    additional_speed_tiers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexReasoningLevel {
    #[serde(default)]
    effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexModelsCache {
    fetched_at_ms: i64,
    etag: Option<String>,
    client_version: String,
    models: Vec<CodexRemoteModel>,
}

enum CodexModelsFetchOutcome {
    Modified {
        models: Vec<CodexRemoteModel>,
        etag: Option<String>,
    },
    NotModified,
}

pub async fn list_codex_available_models(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    cache_dir: &Path,
) -> Result<Vec<CodexAvailableModel>, String> {
    if let Some(cache) = load_fresh_cache(cache_dir) {
        return Ok(remote_models_to_available(cache.models));
    }

    let stale_cache = load_cache(cache_dir);
    let stale_etag = stale_cache.as_ref().and_then(|cache| cache.etag.as_deref());
    match fetch_remote_models(access_token, account_id, base_url, stale_etag).await {
        Ok(CodexModelsFetchOutcome::Modified { models, etag }) => {
            persist_cache(cache_dir, &models, etag)?;
            Ok(remote_models_to_available(models))
        }
        Ok(CodexModelsFetchOutcome::NotModified) => {
            let mut cache = stale_cache
                .ok_or_else(|| "Codex models endpoint returned 304 without cache".to_string())?;
            cache.fetched_at_ms = now_ms();
            save_cache(cache_dir, &cache)?;
            Ok(remote_models_to_available(cache.models))
        }
        Err(error) => {
            if let Some(cache) = stale_cache {
                eprintln!("[OpenAI Codex] using stale model cache after refresh failure: {error}");
                Ok(remote_models_to_available(cache.models))
            } else {
                Err(error)
            }
        }
    }
}

async fn fetch_remote_models(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
    etag: Option<&str>,
) -> Result<CodexModelsFetchOutcome, String> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(MODELS_REFRESH_TIMEOUT_SECS))
            .timeout(Duration::from_secs(MODELS_REFRESH_TIMEOUT_SECS)),
    )
    .map_err(|e| format!("Failed to create Codex models client: {e}"))?;

    let url = codex_models_endpoint(base_url);
    let mut request = client
        .get(&url)
        .query(&[("client_version", CODEX_CLIENT_VERSION)])
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("originator", CODEX_ORIGINATOR_HEADER_VALUE)
        .header("version", CODEX_CLIENT_VERSION);

    if let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.header("ChatGPT-Account-ID", account_id);
    }
    if let Some(etag) = etag.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Codex models request failed: {e}"))?;

    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(CodexModelsFetchOutcome::NotModified);
    }

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Codex models API error ({} {}): {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            body
        ));
    }

    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let payload = response
        .json::<CodexModelsResponse>()
        .await
        .map_err(|e| format!("Failed to parse Codex models response: {e}"))?;

    Ok(CodexModelsFetchOutcome::Modified {
        models: payload.models,
        etag,
    })
}

fn codex_models_endpoint(base_url: Option<&str>) -> String {
    let base_url = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_PROVIDER_BASE_URL)
        .trim_end_matches('/');
    let base_url = base_url
        .strip_suffix(RESPONSES_ENDPOINT_PATH)
        .unwrap_or(base_url);
    format!("{base_url}{MODELS_ENDPOINT_PATH}")
}

fn remote_models_to_available(mut models: Vec<CodexRemoteModel>) -> Vec<CodexAvailableModel> {
    models.retain(is_listed_model);
    models.sort_by(|left, right| {
        left.priority
            .unwrap_or(i32::MAX)
            .cmp(&right.priority.unwrap_or(i32::MAX))
            .then_with(|| left.slug.cmp(&right.slug))
    });

    models
        .into_iter()
        .enumerate()
        .map(|(index, model)| remote_model_to_available(model, index == 0))
        .collect()
}

fn is_listed_model(model: &CodexRemoteModel) -> bool {
    let slug = model.slug.trim();
    if slug.is_empty() {
        return false;
    }

    !matches!(
        model
            .visibility
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("hide" | "hidden")
    )
}

fn remote_model_to_available(model: CodexRemoteModel, is_default: bool) -> CodexAvailableModel {
    let slug = model.slug.trim().to_string();
    let name = model
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(slug.as_str())
        .to_string();
    let supported_efforts = model
        .supported_reasoning_levels
        .into_iter()
        .filter_map(|level| {
            let effort = level.effort.trim().to_string();
            (!effort.is_empty()).then_some(effort)
        })
        .collect();

    CodexAvailableModel {
        id: format!("openai/{slug}"),
        name,
        provider: "openai_codex".to_string(),
        default_effort: model.default_reasoning_level,
        supported_efforts,
        additional_speed_tiers: model.additional_speed_tiers,
        is_default,
    }
}

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(MODEL_CACHE_FILE)
}

fn load_fresh_cache(cache_dir: &Path) -> Option<CodexModelsCache> {
    let cache = load_cache(cache_dir)?;
    if cache.client_version != CODEX_CLIENT_VERSION {
        return None;
    }
    if now_ms().saturating_sub(cache.fetched_at_ms) > MODEL_CACHE_TTL_MS {
        return None;
    }
    Some(cache)
}

fn load_cache(cache_dir: &Path) -> Option<CodexModelsCache> {
    std::fs::read_to_string(cache_path(cache_dir))
        .ok()
        .and_then(|value| serde_json::from_str::<CodexModelsCache>(&value).ok())
        .filter(|cache| cache.client_version == CODEX_CLIENT_VERSION)
}

fn persist_cache(
    cache_dir: &Path,
    models: &[CodexRemoteModel],
    etag: Option<String>,
) -> Result<(), String> {
    let cache = CodexModelsCache {
        fetched_at_ms: now_ms(),
        etag,
        client_version: CODEX_CLIENT_VERSION.to_string(),
        models: models.to_vec(),
    };
    save_cache(cache_dir, &cache)
}

fn save_cache(cache_dir: &Path, cache: &CodexModelsCache) -> Result<(), String> {
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("Failed to create Codex models cache dir: {e}"))?;
    let json = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("Failed to serialize Codex models cache: {e}"))?;
    std::fs::write(cache_path(cache_dir), json)
        .map_err(|e| format!("Failed to write Codex models cache: {e}"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{
        codex_models_endpoint, remote_models_to_available, CodexReasoningLevel, CodexRemoteModel,
    };

    fn remote(slug: &str, priority: i32, visibility: &str) -> CodexRemoteModel {
        CodexRemoteModel {
            slug: slug.to_string(),
            display_name: Some(slug.to_string()),
            visibility: Some(visibility.to_string()),
            priority: Some(priority),
            default_reasoning_level: Some("medium".to_string()),
            supported_reasoning_levels: vec![
                CodexReasoningLevel {
                    effort: "low".to_string(),
                },
                CodexReasoningLevel {
                    effort: "medium".to_string(),
                },
            ],
            additional_speed_tiers: vec!["fast".to_string()],
        }
    }

    #[test]
    fn models_endpoint_reuses_codex_base_url() {
        assert_eq!(
            codex_models_endpoint(None),
            "https://chatgpt.com/backend-api/codex/models"
        );
        assert_eq!(
            codex_models_endpoint(Some("https://example.test/backend-api/codex/responses")),
            "https://example.test/backend-api/codex/models"
        );
    }

    #[test]
    fn visible_models_are_sorted_and_prefixed() {
        let models = remote_models_to_available(vec![
            remote("codex-auto-review", 1, "hide"),
            remote("gpt-5.5", 2, "list"),
            remote("gpt-5.4", 1, "list"),
        ]);

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["openai/gpt-5.4", "openai/gpt-5.5"]
        );
        assert!(models[0].is_default);
        assert!(!models[1].is_default);
        assert_eq!(models[0].supported_efforts, vec!["low", "medium"]);
        assert_eq!(models[0].additional_speed_tiers, vec!["fast"]);
    }
}
