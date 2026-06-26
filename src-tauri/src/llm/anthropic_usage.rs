use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

const ANTHROPIC_USAGE_ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";
const USER_AGENT: &str = "claude-code/2.1.92";
const USAGE_REFRESH_TIMEOUT_SECS: u64 = 8;

#[derive(Debug)]
pub enum AnthropicRateLimitsFetchError {
    Unauthorized(String),
    Other(String),
}

impl AnthropicRateLimitsFetchError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized(_))
    }
}

impl fmt::Display for AnthropicRateLimitsFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(message) | Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AnthropicRateLimitsFetchError {}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnthropicRateLimitsResponse {
    pub fetched_at_ms: i64,
    pub windows: Vec<AnthropicRateLimitWindow>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnthropicRateLimitWindow {
    pub limit_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_minutes: Option<i64>,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AnthropicUsagePayload {
    #[serde(default)]
    five_hour: Option<AnthropicUsageRateLimit>,
    #[serde(default)]
    seven_day: Option<AnthropicUsageRateLimit>,
    #[serde(default)]
    seven_day_oauth_apps: Option<AnthropicUsageRateLimit>,
    #[serde(default)]
    seven_day_opus: Option<AnthropicUsageRateLimit>,
    #[serde(default)]
    seven_day_sonnet: Option<AnthropicUsageRateLimit>,
    #[serde(default)]
    extra_usage: Option<AnthropicUsageExtraUsage>,
    #[serde(default)]
    limits: Option<Vec<AnthropicUsageScopedLimit>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageRateLimit {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageExtraUsage {
    #[serde(default)]
    is_enabled: Option<bool>,
    #[serde(default)]
    utilization: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageScopedLimit {
    #[serde(default, rename = "type")]
    limit_type: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    percent: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
    #[serde(default)]
    scope: Option<AnthropicUsageScopedLimitScope>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageScopedLimitScope {
    #[serde(default)]
    model: Option<AnthropicUsageScopedLimitModel>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsageScopedLimitModel {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

pub async fn fetch_anthropic_rate_limits(
    access_token: &str,
) -> Result<AnthropicRateLimitsResponse, AnthropicRateLimitsFetchError> {
    let client = crate::network::reqwest_client(
        crate::network::ReqwestClientOptions::new()
            .connect_timeout(Duration::from_secs(USAGE_REFRESH_TIMEOUT_SECS))
            .timeout(Duration::from_secs(USAGE_REFRESH_TIMEOUT_SECS)),
    )
    .map_err(|e| {
        AnthropicRateLimitsFetchError::Other(format!(
            "Failed to create Anthropic usage client: {e}"
        ))
    })?;

    let response = client
        .get(ANTHROPIC_USAGE_ENDPOINT)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .header("anthropic-beta", OAUTH_BETA_HEADER)
        .send()
        .await
        .map_err(|e| {
            AnthropicRateLimitsFetchError::Other(format!("Anthropic usage request failed: {e}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = format!(
            "Anthropic usage API error ({} {}): {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            body
        );
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AnthropicRateLimitsFetchError::Unauthorized(message));
        }
        return Err(AnthropicRateLimitsFetchError::Other(message));
    }

    let payload = response
        .json::<AnthropicUsagePayload>()
        .await
        .map_err(|e| {
            AnthropicRateLimitsFetchError::Other(format!(
                "Failed to parse Anthropic usage response: {e}"
            ))
        })?;
    Ok(rate_limits_from_payload(payload))
}

fn rate_limits_from_payload(payload: AnthropicUsagePayload) -> AnthropicRateLimitsResponse {
    let mut windows = Vec::new();
    push_window(&mut windows, "five_hour", payload.five_hour, Some(5 * 60));
    push_window(
        &mut windows,
        "seven_day",
        payload.seven_day,
        Some(7 * 24 * 60),
    );
    push_window(
        &mut windows,
        "seven_day_sonnet",
        payload.seven_day_sonnet,
        Some(7 * 24 * 60),
    );
    push_window(
        &mut windows,
        "seven_day_opus",
        payload.seven_day_opus,
        Some(7 * 24 * 60),
    );
    push_window(
        &mut windows,
        "seven_day_oauth_apps",
        payload.seven_day_oauth_apps,
        Some(7 * 24 * 60),
    );
    push_scoped_limit_windows(&mut windows, payload.limits.unwrap_or_default());
    push_extra_usage_window(&mut windows, payload.extra_usage);

    AnthropicRateLimitsResponse {
        fetched_at_ms: chrono::Utc::now().timestamp_millis(),
        windows,
    }
}

fn push_window(
    windows: &mut Vec<AnthropicRateLimitWindow>,
    limit_id: &str,
    limit: Option<AnthropicUsageRateLimit>,
    window_minutes: Option<i64>,
) {
    let Some(limit) = limit else {
        return;
    };
    let Some(utilization) = limit.utilization else {
        return;
    };
    let used_percent = utilization.clamp(0.0, 100.0);
    windows.push(AnthropicRateLimitWindow {
        limit_id: limit_id.to_string(),
        limit_name: None,
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes,
        resets_at: parse_reset_timestamp(limit.resets_at),
    });
}

fn push_extra_usage_window(
    windows: &mut Vec<AnthropicRateLimitWindow>,
    extra_usage: Option<AnthropicUsageExtraUsage>,
) {
    let Some(extra_usage) = extra_usage else {
        return;
    };
    if extra_usage.is_enabled == Some(false) {
        return;
    }
    let Some(utilization) = extra_usage.utilization else {
        return;
    };
    let used_percent = utilization.clamp(0.0, 100.0);
    windows.push(AnthropicRateLimitWindow {
        limit_id: "extra_usage".to_string(),
        limit_name: None,
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes: None,
        resets_at: None,
    });
}

fn push_scoped_limit_windows(
    windows: &mut Vec<AnthropicRateLimitWindow>,
    limits: Vec<AnthropicUsageScopedLimit>,
) {
    for limit in limits {
        let kind = limit
            .kind
            .as_deref()
            .or(limit.limit_type.as_deref())
            .unwrap_or_default();
        if kind != "weekly_scoped" {
            continue;
        }

        let model_name = limit.scope.as_ref().and_then(|scope| {
            scope.model.as_ref().and_then(|model| {
                model
                    .display_name
                    .as_deref()
                    .or(model.name.as_deref())
                    .or(model.id.as_deref())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
            })
        });
        let Some(model_name) = model_name else {
            continue;
        };

        let Some(percent) = limit.percent.or(limit.utilization) else {
            continue;
        };

        let limit_id = scoped_limit_id(&model_name);
        if windows.iter().any(|window| window.limit_id == limit_id) {
            continue;
        }

        let used_percent = percent.clamp(0.0, 100.0);
        windows.push(AnthropicRateLimitWindow {
            limit_id,
            limit_name: Some(model_name),
            used_percent,
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
            window_minutes: Some(7 * 24 * 60),
            resets_at: parse_reset_timestamp(limit.resets_at),
        });
    }
}

fn scoped_limit_id(model_name: &str) -> String {
    let lower = model_name.to_ascii_lowercase();
    if lower.contains("sonnet") {
        return "seven_day_sonnet".to_string();
    }
    if lower.contains("opus") {
        return "seven_day_opus".to_string();
    }

    let slug = lower
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if slug.is_empty() {
        "weekly_scoped".to_string()
    } else {
        format!("weekly_scoped:{slug}")
    }
}

fn parse_reset_timestamp(value: Option<String>) -> Option<i64> {
    let value = value?.trim().to_string();
    if value.is_empty() {
        return None;
    }
    chrono::DateTime::parse_from_rfc3339(&value)
        .ok()
        .map(|date| date.timestamp())
        .filter(|timestamp| *timestamp > 0)
}

#[cfg(test)]
mod tests {
    use super::{rate_limits_from_payload, AnthropicUsagePayload};
    use serde_json::json;

    #[test]
    fn maps_usage_payload_to_remaining_windows() {
        let payload: AnthropicUsagePayload = serde_json::from_value(json!({
            "five_hour": {
                "utilization": 12,
                "resets_at": "2026-06-27T04:19:00Z"
            },
            "seven_day": {
                "utilization": 3,
                "resets_at": "2026-07-03T00:00:00Z"
            },
            "seven_day_sonnet": {
                "utilization": 0,
                "resets_at": "2026-07-03T00:00:00Z"
            },
            "seven_day_opus": {
                "utilization": null,
                "resets_at": "2026-07-03T00:00:00Z"
            }
        }))
        .expect("usage payload should parse");

        let response = rate_limits_from_payload(payload);

        assert_eq!(response.windows.len(), 3);
        assert_eq!(response.windows[0].limit_id, "five_hour");
        assert_eq!(response.windows[0].limit_name, None);
        assert_eq!(response.windows[0].used_percent, 12.0);
        assert_eq!(response.windows[0].remaining_percent, 88.0);
        assert_eq!(response.windows[0].window_minutes, Some(300));
        assert_eq!(response.windows[0].resets_at, Some(1_782_533_940));
        assert_eq!(response.windows[1].limit_id, "seven_day");
        assert_eq!(response.windows[1].remaining_percent, 97.0);
        assert_eq!(response.windows[2].limit_id, "seven_day_sonnet");
        assert_eq!(response.windows[2].remaining_percent, 100.0);
    }

    #[test]
    fn maps_scoped_usage_limits_to_model_windows() {
        let payload: AnthropicUsagePayload = serde_json::from_value(json!({
            "five_hour": {
                "utilization": 12,
                "resets_at": "2026-06-27T04:19:00Z"
            },
            "limits": [
                {
                    "kind": "weekly_scoped",
                    "percent": 3,
                    "resets_at": "2026-07-03T00:00:00Z",
                    "scope": {
                        "model": {
                            "display_name": "Sonnet only"
                        }
                    }
                },
                {
                    "type": "weekly_scoped",
                    "utilization": 8,
                    "resets_at": "2026-07-03T00:00:00Z",
                    "scope": {
                        "model": {
                            "display_name": "Opus"
                        }
                    }
                }
            ],
            "extra_usage": {
                "is_enabled": true,
                "utilization": 25
            }
        }))
        .expect("usage payload should parse");

        let response = rate_limits_from_payload(payload);

        assert_eq!(response.windows.len(), 4);
        assert_eq!(response.windows[1].limit_id, "seven_day_sonnet");
        assert_eq!(
            response.windows[1].limit_name.as_deref(),
            Some("Sonnet only")
        );
        assert_eq!(response.windows[1].remaining_percent, 97.0);
        assert_eq!(response.windows[2].limit_id, "seven_day_opus");
        assert_eq!(response.windows[2].limit_name.as_deref(), Some("Opus"));
        assert_eq!(response.windows[2].remaining_percent, 92.0);
        assert_eq!(response.windows[3].limit_id, "extra_usage");
        assert_eq!(response.windows[3].remaining_percent, 75.0);
    }
}
