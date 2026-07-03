//! Shared retry policy for the LLM HTTP transports.
//!
//! Mirrors the persisted `AppConfig::llm_retry_max_attempts` into a global so
//! the streaming call sites (`llm::chat_completions`, `llm::responses`) can
//! read it without threading config state through every call. Same pattern as
//! `code_tools`: commands persist via `AppConfig` and mirror here.

use std::sync::atomic::{AtomicU32, Ordering};

/// Default number of automatic retries after a retryable failure (connect
/// error, timeout, HTTP 5xx / 429). Matches the historical hardcoded
/// `MAX_RETRIES` so existing installs keep the same behavior.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Upper bound accepted from config/UI; keeps a pathological value from
/// stalling a session behind minutes of backoff.
pub const MAX_RETRIES_LIMIT: u32 = 10;

/// Cap applied to server-provided `Retry-After` values (seconds).
pub const RETRY_AFTER_CAP_SECS: u64 = 60;

static MAX_RETRIES: AtomicU32 = AtomicU32::new(DEFAULT_MAX_RETRIES);

/// Called once from app setup with the persisted value.
pub fn initialize(value: u32) {
    set_max_retries(value);
}

/// Mirror a config change into the global. `0` disables automatic retries.
pub fn set_max_retries(value: u32) {
    MAX_RETRIES.store(clamp_max_retries(value), Ordering::Relaxed);
}

/// Number of automatic retries a transport should attempt after a retryable
/// failure (`0` = single attempt, no retry).
pub fn max_retries() -> u32 {
    clamp_max_retries(MAX_RETRIES.load(Ordering::Relaxed))
}

pub(crate) fn clamp_max_retries(value: u32) -> u32 {
    value.min(MAX_RETRIES_LIMIT)
}

/// Whether a non-2xx HTTP response status is worth retrying before any stream
/// output was consumed: transient server errors (5xx, incl. 529) plus 429
/// rate limits. Every other 4xx is a request problem — retrying replays the
/// same failure, so those must never loop (issue #94's 400s included).
pub(crate) fn is_retryable_http_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Parse a `Retry-After` response header value into seconds. Accepts both
/// RFC 9110 forms: delay-seconds and HTTP-date (IMF-fixdate, which chrono's
/// RFC 2822 parser handles). Values are clamped to [`RETRY_AFTER_CAP_SECS`];
/// dates in the past collapse to 0 and unparseable values yield `None`.
pub(crate) fn parse_retry_after_secs(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(secs.min(RETRY_AFTER_CAP_SECS));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let secs = when
        .signed_duration_since(chrono::Utc::now())
        .num_seconds()
        .max(0) as u64;
    Some(secs.min(RETRY_AFTER_CAP_SECS))
}

/// Extract and parse the `Retry-After` header, when the server sent one.
pub(crate) fn retry_after_secs_from_headers(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after_secs)
}

/// Backoff before retrying attempt `attempt` (0-based) of a failed HTTP
/// status. A server-provided `Retry-After` wins; otherwise 429 starts at 5s
/// (rate limits rarely clear within the 1s transport base) and 5xx keeps the
/// historical 1s exponential base.
pub(crate) fn status_retry_delay_ms(
    status: reqwest::StatusCode,
    retry_after_secs: Option<u64>,
    attempt: u32,
) -> u64 {
    if let Some(secs) = retry_after_secs {
        return secs.min(RETRY_AFTER_CAP_SECS) * 1000;
    }
    let base_ms: u64 = if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        5000
    } else {
        1000
    };
    base_ms * 2u64.pow(attempt.min(6))
}

#[cfg(test)]
mod tests {
    use super::{
        clamp_max_retries, is_retryable_http_status, parse_retry_after_secs,
        status_retry_delay_ms, MAX_RETRIES_LIMIT, RETRY_AFTER_CAP_SECS,
    };
    use reqwest::StatusCode;

    #[test]
    fn retryable_statuses_cover_5xx_and_429_only() {
        assert!(is_retryable_http_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_http_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_http_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_http_status(StatusCode::GATEWAY_TIMEOUT));
        assert!(is_retryable_http_status(
            StatusCode::from_u16(529).expect("529 should be a valid extension status")
        ));
        assert!(is_retryable_http_status(StatusCode::TOO_MANY_REQUESTS));

        assert!(!is_retryable_http_status(StatusCode::OK));
        assert!(!is_retryable_http_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_http_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_http_status(StatusCode::FORBIDDEN));
        assert!(!is_retryable_http_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn parses_delay_seconds_and_clamps_to_cap() {
        assert_eq!(parse_retry_after_secs("0"), Some(0));
        assert_eq!(parse_retry_after_secs("5"), Some(5));
        assert_eq!(parse_retry_after_secs(" 30 "), Some(30));
        assert_eq!(parse_retry_after_secs("120"), Some(RETRY_AFTER_CAP_SECS));
        assert_eq!(parse_retry_after_secs("18446744073709551615"), Some(RETRY_AFTER_CAP_SECS));
    }

    #[test]
    fn rejects_garbage_retry_after_values() {
        assert_eq!(parse_retry_after_secs(""), None);
        assert_eq!(parse_retry_after_secs("soon"), None);
        assert_eq!(parse_retry_after_secs("-5"), None);
        assert_eq!(parse_retry_after_secs("5.5"), None);
    }

    #[test]
    fn parses_http_date_retry_after() {
        let future = chrono::Utc::now() + chrono::Duration::seconds(10);
        let secs = parse_retry_after_secs(&future.to_rfc2822()).expect("future date parses");
        assert!((8..=11).contains(&secs), "expected ~10s, got {}", secs);

        // A date in the past means "retry now", not an error.
        let past = chrono::Utc::now() - chrono::Duration::seconds(30);
        assert_eq!(parse_retry_after_secs(&past.to_rfc2822()), Some(0));

        // The wire format servers actually send (IMF-fixdate, GMT zone name).
        assert_eq!(
            parse_retry_after_secs("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(0)
        );

        // Far-future dates clamp to the cap.
        let far = chrono::Utc::now() + chrono::Duration::seconds(3600);
        assert_eq!(
            parse_retry_after_secs(&far.to_rfc2822()),
            Some(RETRY_AFTER_CAP_SECS)
        );
    }

    #[test]
    fn retry_after_wins_over_backoff() {
        assert_eq!(
            status_retry_delay_ms(StatusCode::TOO_MANY_REQUESTS, Some(7), 0),
            7000
        );
        assert_eq!(
            status_retry_delay_ms(StatusCode::INTERNAL_SERVER_ERROR, Some(2), 3),
            2000
        );
    }

    #[test]
    fn backoff_bases_differ_for_429_and_5xx() {
        assert_eq!(status_retry_delay_ms(StatusCode::TOO_MANY_REQUESTS, None, 0), 5000);
        assert_eq!(status_retry_delay_ms(StatusCode::TOO_MANY_REQUESTS, None, 1), 10000);
        assert_eq!(status_retry_delay_ms(StatusCode::TOO_MANY_REQUESTS, None, 2), 20000);
        assert_eq!(status_retry_delay_ms(StatusCode::INTERNAL_SERVER_ERROR, None, 0), 1000);
        assert_eq!(status_retry_delay_ms(StatusCode::BAD_GATEWAY, None, 1), 2000);
        assert_eq!(status_retry_delay_ms(StatusCode::SERVICE_UNAVAILABLE, None, 2), 4000);
    }

    #[test]
    fn clamps_config_values_to_limit() {
        assert_eq!(clamp_max_retries(0), 0);
        assert_eq!(clamp_max_retries(3), 3);
        assert_eq!(clamp_max_retries(MAX_RETRIES_LIMIT), MAX_RETRIES_LIMIT);
        assert_eq!(clamp_max_retries(99), MAX_RETRIES_LIMIT);
        assert_eq!(clamp_max_retries(u32::MAX), MAX_RETRIES_LIMIT);
    }
}
