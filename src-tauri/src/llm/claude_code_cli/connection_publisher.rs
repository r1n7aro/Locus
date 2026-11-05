//! Pushes the Claude Code connection overview to every window when it changes,
//! so renderers no longer poll `get_claude_session_connection`. Sampling reads
//! the same in-process state as that command: it never opens SSH, unlocks a
//! keychain or probes the server.
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::Emitter;

/// App-wide event: `{ status: ConnectionStatus | null, error: string | null }`.
pub const CLAUDE_CONNECTION_STATUS_EVENT: &str = "claude-connection-status";

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// Transport counters (bytes, ages, latency) move on every sample of a live
/// connection; they alone republish at most this often.
const COUNTER_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const VOLATILE_KEYS: &[&str] = &[
    "latencyMs",
    "elapsedMs",
    "stageElapsedMs",
    "receivedBytes",
    "sentBytes",
    "lastReceiveAgeMs",
    "partialFrameBytes",
    "stageReceivedBytes",
    "stageBytesPerSecond",
    "heartbeatAgeMs",
    "pendingHeartbeats",
    "timeline",
];

struct Published {
    signature: String,
    value: serde_json::Value,
    at: Instant,
}

pub(crate) fn start(app: tauri::AppHandle, config: Arc<crate::config::AppConfig>) {
    tauri::async_runtime::spawn(async move {
        let mut published: Option<Published> = None;
        loop {
            tokio::time::sleep(SAMPLE_INTERVAL).await;
            let value = sample(&config);
            let signature = stable_signature(&value);
            let due = match &published {
                None => true,
                Some(last) if last.signature != signature => true,
                Some(last) => last.value != value && last.at.elapsed() >= COUNTER_REFRESH_INTERVAL,
            };
            if !due {
                continue;
            }
            let _ = app.emit(CLAUDE_CONNECTION_STATUS_EVENT, value.clone());
            published = Some(Published { signature, value, at: Instant::now() });
        }
    });
}

fn sample(config: &crate::config::AppConfig) -> serde_json::Value {
    if !config.claude_code_cli_enabled() {
        return serde_json::json!({ "status": null, "error": null });
    }
    match super::ssh::load_settings() {
        Ok(settings) => serde_json::json!({
            "status": super::tui::connection_overview(None, None, None, &settings),
            "error": null,
        }),
        Err(error) => serde_json::json!({ "status": null, "error": error }),
    }
}

fn stable_signature(value: &serde_json::Value) -> String {
    fn strip(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for key in VOLATILE_KEYS {
                    map.remove(*key);
                }
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut stable = value.clone();
    strip(&mut stable);
    stable.to_string()
}

#[cfg(test)]
mod tests {
    use super::stable_signature;

    #[test]
    fn transport_counters_do_not_change_the_signature_but_phase_does() {
        let sample = |phase: &str, received: u64, latency: f64| serde_json::json!({
            "status": { "phase": phase, "latencyMs": latency, "diagnostics": { "active": {
                "stage": "ready", "receivedBytes": received, "timeline": [{ "stage": "ready", "elapsedMs": 3 }],
            } } },
            "error": null,
        });
        assert_eq!(stable_signature(&sample("ready", 10, 12.0)), stable_signature(&sample("ready", 99, 40.5)));
        assert_ne!(stable_signature(&sample("ready", 10, 12.0)), stable_signature(&sample("starting", 10, 12.0)));
    }
}
