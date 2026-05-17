use super::CODEX_CLIENT_VERSION;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

const DEFAULT_CODEX_PROVIDER_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const RESPONSES_ENDPOINT_PATH: &str = "/responses";
const MODELS_ENDPOINT_PATH: &str = "/models";
const USAGE_ENDPOINT_PATH: &str = "/usage";
const CODEX_ORIGINATOR_HEADER_VALUE: &str = "opencode";
const USAGE_REFRESH_TIMEOUT_SECS: u64 = 8;

#[derive(Debug)]
pub enum CodexRateLimitsFetchError {
    Unauthorized(String),
    Other(String),
}

impl CodexRateLimitsFetchError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized(_))
    }
}

impl fmt::Display for CodexRateLimitsFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(message) | Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CodexRateLimitsFetchError {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitsResponse {
    pub fetched_at_ms: i64,
    pub rate_limits: CodexRateLimitSnapshot,
    pub rate_limits_by_limit_id: HashMap<String, CodexRateLimitSnapshot>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitSnapshot {
    pub limit_id: Option<String>,
    pub limit_name: Option<String>,
    pub primary: Option<CodexRateLimitWindow>,
    pub secondary: Option<CodexRateLimitWindow>,
    pub credits: Option<CodexCreditsSnapshot>,
    pub plan_type: Option<String>,
    pub rate_limit_reached_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexRateLimitWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodexCreditsSnapshot {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct CodexUsagePayload {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexUsageRateLimit>,
    #[serde(default)]
    credits: Option<CodexUsageCredits>,
    #[serde(default)]
    additional_rate_limits: Option<Vec<CodexAdditionalRateLimit>>,
    #[serde(default)]
    rate_limit_reached_type: Option<CodexRateLimitReachedPayload>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageRateLimit {
    #[serde(default)]
    primary_window: Option<CodexUsageWindow>,
    #[serde(default)]
    secondary_window: Option<CodexUsageWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageWindow {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    limit_window_seconds: Option<i64>,
    #[serde(default)]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageCredits {
    #[serde(default)]
    has_credits: Option<bool>,
    #[serde(default)]
    unlimited: Option<bool>,
    #[serde(default)]
    balance: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexAdditionalRateLimit {
    #[serde(default)]
    limit_name: Option<String>,
    #[serde(default)]
    metered_feature: Option<String>,
    #[serde(default)]
    rate_limit: Option<CodexUsageRateLimit>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimitReachedPayload {
    #[serde(rename = "type")]
    kind: String,
}

pub async fn fetch_codex_rate_limits(
    access_token: &str,
    account_id: Option<&str>,
    base_url: Option<&str>,
) -> Result<CodexRateLimitsResponse, CodexRateLimitsFetchError> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(USAGE_REFRESH_TIMEOUT_SECS))
            .timeout(Duration::from_secs(USAGE_REFRESH_TIMEOUT_SECS)),
    )
    .map_err(|e| {
        CodexRateLimitsFetchError::Other(format!("Failed to create Codex usage client: {e}"))
    })?;

    let url = codex_usage_endpoint(base_url);
    let mut request = client
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("originator", CODEX_ORIGINATOR_HEADER_VALUE)
        .header("version", CODEX_CLIENT_VERSION);

    if let Some(account_id) = account_id.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.header("ChatGPT-Account-ID", account_id);
    }

    let response = request.send().await.map_err(|e| {
        CodexRateLimitsFetchError::Other(format!("Codex usage request failed: {e}"))
    })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = format!(
            "Codex usage API error ({} {}): {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            body
        );
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CodexRateLimitsFetchError::Unauthorized(message));
        }
        return Err(CodexRateLimitsFetchError::Other(message));
    }

    let payload = response.json::<CodexUsagePayload>().await.map_err(|e| {
        CodexRateLimitsFetchError::Other(format!("Failed to parse Codex usage response: {e}"))
    })?;
    Ok(rate_limits_from_payload(payload))
}

fn rate_limits_from_payload(payload: CodexUsagePayload) -> CodexRateLimitsResponse {
    let plan_type = payload.plan_type.map(normalize_plan_type);
    let rate_limit_reached_type = payload
        .rate_limit_reached_type
        .map(|details| details.kind)
        .filter(|kind| !kind.trim().is_empty());
    let mut snapshots = vec![make_rate_limit_snapshot(
        Some("codex".to_string()),
        None,
        payload.rate_limit,
        payload.credits,
        plan_type.clone(),
        rate_limit_reached_type,
    )];

    if let Some(additional) = payload.additional_rate_limits {
        snapshots.extend(additional.into_iter().filter_map(|details| {
            let limit_id = details
                .metered_feature
                .as_deref()
                .or(details.limit_name.as_deref())
                .map(normalize_limit_id)
                .filter(|value| !value.is_empty())?;
            Some(make_rate_limit_snapshot(
                Some(limit_id),
                details.limit_name.filter(|value| !value.trim().is_empty()),
                details.rate_limit,
                None,
                plan_type.clone(),
                None,
            ))
        }));
    }

    let rate_limits_by_limit_id: HashMap<String, CodexRateLimitSnapshot> = snapshots
        .iter()
        .cloned()
        .map(|snapshot| {
            let limit_id = snapshot
                .limit_id
                .clone()
                .unwrap_or_else(|| "codex".to_string());
            (limit_id, snapshot)
        })
        .collect();

    let rate_limits = rate_limits_by_limit_id
        .get("codex")
        .cloned()
        .unwrap_or_else(|| snapshots[0].clone());

    CodexRateLimitsResponse {
        fetched_at_ms: chrono::Utc::now().timestamp_millis(),
        rate_limits,
        rate_limits_by_limit_id,
    }
}

fn make_rate_limit_snapshot(
    limit_id: Option<String>,
    limit_name: Option<String>,
    rate_limit: Option<CodexUsageRateLimit>,
    credits: Option<CodexUsageCredits>,
    plan_type: Option<String>,
    rate_limit_reached_type: Option<String>,
) -> CodexRateLimitSnapshot {
    let (primary, secondary) = match rate_limit {
        Some(details) => (
            map_rate_limit_window(details.primary_window),
            map_rate_limit_window(details.secondary_window),
        ),
        None => (None, None),
    };

    CodexRateLimitSnapshot {
        limit_id,
        limit_name,
        primary,
        secondary,
        credits: credits.map(map_credits),
        plan_type,
        rate_limit_reached_type,
    }
}

fn map_rate_limit_window(window: Option<CodexUsageWindow>) -> Option<CodexRateLimitWindow> {
    let window = window?;
    let used_percent = window.used_percent.unwrap_or(0.0).clamp(0.0, 100.0);
    let window_minutes = window
        .limit_window_seconds
        .and_then(window_minutes_from_seconds);
    let resets_at = window.reset_at.filter(|value| *value > 0);
    let has_data = used_percent > 0.0 || window_minutes.is_some() || resets_at.is_some();

    has_data.then_some(CodexRateLimitWindow {
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes,
        resets_at,
    })
}

fn map_credits(credits: CodexUsageCredits) -> CodexCreditsSnapshot {
    CodexCreditsSnapshot {
        has_credits: credits.has_credits.unwrap_or(false),
        unlimited: credits.unlimited.unwrap_or(false),
        balance: credits
            .balance
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    }
}

fn window_minutes_from_seconds(seconds: i64) -> Option<i64> {
    if seconds <= 0 {
        return None;
    }
    Some((seconds + 59) / 60)
}

fn normalize_limit_id(value: impl AsRef<str>) -> String {
    value.as_ref().trim().to_ascii_lowercase().replace('-', "_")
}

fn normalize_plan_type(value: String) -> String {
    value.trim().to_ascii_lowercase()
}

fn codex_usage_endpoint(base_url: Option<&str>) -> String {
    let base_url = base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CODEX_PROVIDER_BASE_URL)
        .trim_end_matches('/');
    let base_url = base_url
        .strip_suffix(RESPONSES_ENDPOINT_PATH)
        .or_else(|| base_url.strip_suffix(MODELS_ENDPOINT_PATH))
        .or_else(|| base_url.strip_suffix(USAGE_ENDPOINT_PATH))
        .unwrap_or(base_url);
    format!("{base_url}{USAGE_ENDPOINT_PATH}")
}

#[cfg(test)]
mod tests {
    use super::{codex_usage_endpoint, rate_limits_from_payload, CodexUsagePayload};
    use serde_json::json;

    #[test]
    fn usage_endpoint_reuses_codex_base_url() {
        assert_eq!(
            codex_usage_endpoint(None),
            "https://chatgpt.com/backend-api/codex/usage"
        );
        assert_eq!(
            codex_usage_endpoint(Some("https://example.test/backend-api/codex/responses")),
            "https://example.test/backend-api/codex/usage"
        );
        assert_eq!(
            codex_usage_endpoint(Some("https://example.test/backend-api/codex/models")),
            "https://example.test/backend-api/codex/usage"
        );
    }

    #[test]
    fn maps_usage_payload_to_remaining_windows() {
        let payload: CodexUsagePayload = serde_json::from_value(json!({
            "plan_type": "pro",
            "rate_limit": {
                "primary_window": {
                    "used_percent": 42,
                    "limit_window_seconds": 3600,
                    "reset_at": 1735689720
                },
                "secondary_window": {
                    "used_percent": 5,
                    "limit_window_seconds": 604800,
                    "reset_at": 1736294400
                }
            },
            "credits": {
                "has_credits": true,
                "unlimited": false,
                "balance": "25"
            },
            "rate_limit_reached_type": {
                "type": "workspace_member_usage_limit_reached"
            },
            "additional_rate_limits": [
                {
                    "limit_name": "codex_other",
                    "metered_feature": "codex-other",
                    "rate_limit": {
                        "primary_window": {
                            "used_percent": 88,
                            "limit_window_seconds": 1800,
                            "reset_at": 1735693200
                        }
                    }
                }
            ]
        }))
        .expect("usage payload should parse");

        let response = rate_limits_from_payload(payload);
        let primary = response.rate_limits.primary.expect("primary window");
        let secondary = response.rate_limits.secondary.expect("secondary window");
        assert_eq!(primary.remaining_percent, 58.0);
        assert_eq!(primary.window_minutes, Some(60));
        assert_eq!(secondary.remaining_percent, 95.0);
        assert_eq!(secondary.window_minutes, Some(10080));
        assert_eq!(response.rate_limits.plan_type.as_deref(), Some("pro"));
        assert_eq!(
            response.rate_limits.rate_limit_reached_type.as_deref(),
            Some("workspace_member_usage_limit_reached")
        );
        assert!(response.rate_limits_by_limit_id.contains_key("codex_other"));
    }
}
