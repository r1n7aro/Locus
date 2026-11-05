//! Startup stages come from the helper, not guessed from stderr or elapsed time.
use super::errors as turn_retry;
use super::errors as diagnostics;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Ssh,
    History,
    Environment,
    Tui,
}
impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Ssh => "SSH connection",
            Self::History => "History synchronization",
            Self::Environment => "Environment preparation",
            Self::Tui => "TUI initialization",
        }
    }
}

pub struct Handshake {
    stage: Stage,
    mode: String,
}
impl Handshake {
    pub fn new(request: &Value) -> Self {
        Self { stage: Stage::Ssh, mode: request["mode"].as_str().unwrap_or("session").into() }
    }

    pub fn progress(&mut self, frame: &Value) {
        if frame["protocol"] != 1 || frame["type"] != "progress" { return; }
        // Ignore unknown stages and backwards transitions. A progress frame is
        // evidence that SSH reached the helper, not evidence of a submitted input.
        match (self.stage, frame["stage"].as_str(), self.mode.as_str()) {
            (Stage::Ssh, Some("history"), "history" | "archive" | "archive_read") => self.stage = Stage::History,
            (Stage::Ssh, Some("environment"), "inspect" | "prepare" | "session" | "fork" | "rewind") => self.stage = Stage::Environment,
            (Stage::Environment, Some("tui"), "session") => self.stage = Stage::Tui,
            _ => {},
        }
    }

    pub fn context(&self, error: String) -> String {
        let operation = match self.mode.as_str() {
            "history" => "reading native history",
            "archive" | "archive_read" => "transferring native history",
            "inspect" => "inspecting the environment",
            "prepare" => "preparing the environment",
            "fork" => "forking native history",
            "rewind" => "rewinding native history",
            _ => "starting Claude",
        };
        turn_retry::with_context(&format!("{} ({operation})", self.stage.label()), error)
    }

    pub fn failed(
        &self, timed_out: bool, timeout: Duration, failure: Option<String>, diagnostic: &[u8],
    ) -> String {
        let diagnostic = diagnostics::plain_text(&String::from_utf8_lossy(diagnostic));
        let has_failure = failure.is_some();
        let mut error = failure.unwrap_or_else(|| if timed_out {
            format!("Timed out after {}s waiting for the remote handshake", timeout.as_secs())
        } else {
            turn_retry::disconnected("SSH output closed before the remote handshake")
        });
        if !diagnostic.is_empty() { error.push_str(&format!("\n{diagnostic}")); }
        let error = turn_retry::ssh_startup_error(error);
        // Only a timeout observed at this boundary opts into recovery. Never
        // broaden retry classification to auth/plugin/history validation text.
        // Starting another attempt still MUST synchronize native history first.
        let error = if timed_out && !has_failure && !turn_retry::startup_rejected(&error)
            && matches!(self.stage, Stage::Ssh | Stage::History)
            && !turn_retry::is_disconnected(&error)
        {
            turn_retry::disconnected(error)
        } else { error };
        self.context(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn progress(stage: &str) -> Value {
        json!({"type":"progress","protocol":1,"stage":stage})
    }
    fn timeout(handshake: &Handshake) -> String {
        handshake.failed(true, Duration::from_secs(45), None, b"")
    }

    #[test]
    fn history_handshake_timeout_is_retryable_before_and_after_helper_progress() {
        let mut handshake = Handshake::new(&json!({"mode":"history"}));
        let error = timeout(&handshake);
        assert!(turn_retry::is_disconnected(&error), "{error}");
        assert!(error.contains("SSH connection (reading native history)"));
        handshake.progress(&progress("history"));
        let error = timeout(&handshake);
        assert!(turn_retry::is_disconnected(&error), "{error}");
        assert!(error.contains("History synchronization") && error.contains("45s"));
    }

    #[test]
    fn pooled_archive_startup_never_claims_to_start_claude() {
        let mut handshake = Handshake::new(&json!({"mode":"archive_read"}));
        handshake.progress(&progress("history"));
        let error = timeout(&handshake);
        assert!(error.contains("transferring native history"));
        assert!(turn_retry::is_disconnected(&error));
        assert!(!error.contains("starting Claude"));
    }

    #[test]
    fn environment_and_tui_timeouts_do_not_opt_into_transport_recovery() {
        let mut handshake = Handshake::new(&json!({"mode":"session"}));
        handshake.progress(&progress("environment"));
        assert!(timeout(&handshake).starts_with("Environment preparation"));
        assert!(!turn_retry::is_disconnected(&timeout(&handshake)));
        handshake.progress(&progress("tui"));
        assert!(timeout(&handshake).starts_with("TUI initialization"));
        assert!(!turn_retry::is_disconnected(&timeout(&handshake)));
        // A proven transport loss remains retryable even during initialization.
        let error = handshake.failed(false, Duration::ZERO,
            Some(turn_retry::disconnected("SSH output closed")), b"");
        assert!(turn_retry::is_disconnected(&error) && error.contains("TUI initialization"));
    }

    #[test]
    fn explicit_rejections_and_protocol_failures_are_not_retried() {
        let handshake = Handshake::new(&json!({"mode":"history"}));
        for error in ["Invalid remote history", "keychain authentication timed out", "Claude plugin validation failed", "SSH frame exceeds size limit"] {
            let error = handshake.failed(true, Duration::from_secs(45), Some(error.into()), b"");
            assert!(!turn_retry::is_disconnected(&error), "{error}");
        }
        for diagnostic in ["Permission denied (publickey)", "Host key verification failed"] {
            let error = handshake.failed(true, Duration::from_secs(45), None, diagnostic.as_bytes());
            assert!(!turn_retry::is_disconnected(&error), "{error}");
            assert!(error.contains(diagnostic));
        }
    }

    #[test]
    fn progress_cannot_mislabel_a_different_operation_or_move_backwards() {
        let mut handshake = Handshake::new(&json!({"mode":"history"}));
        handshake.progress(&progress("environment"));
        handshake.progress(&progress("tui"));
        handshake.progress(&json!({"type":"progress","protocol":2,"stage":"history"}));
        assert_eq!(handshake.stage, Stage::Ssh);
        handshake.progress(&progress("history"));
        handshake.progress(&progress("ssh"));
        assert_eq!(handshake.stage, Stage::History);
    }

    #[test]
    fn stderr_does_not_hide_transport_failure_and_is_sanitized() {
        let handshake = Handshake::new(&json!({"mode":"history"}));
        let error = handshake.failed(false, Duration::ZERO,
            Some(turn_retry::disconnected("SSH output closed")), b"\x1b[31mserver not responding\x1b[0m");
        assert!(turn_retry::is_disconnected(&error));
        assert!(error.contains("SSH output closed") && error.contains("server not responding"));
        assert!(!error.contains('\x1b'));
    }
}
