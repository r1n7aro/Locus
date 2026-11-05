//! Native diagnostics are GUI notices, never model messages or tool results.
use crate::error::{AppError, ErrorSeverity};
use serde_json::Value;
use std::collections::HashSet;

const PENDING_TOOLS: &str = "Claude did not return every tool result. Verify local state before continuing; no result was fabricated and no remote fallback was executed.";

pub(super) use locus_remote_core::claude_ssh::errors::plain_text;

#[derive(Default)]
pub(super) struct TurnDiagnostics {
    seen: HashSet<(String, String)>,
    causes: Vec<String>,
}

impl TurnDiagnostics {
    pub fn start_step(&mut self) {
        // A native retry/fallback may recover. Do not attach an earlier failed
        // attempt's cause to an unrelated later tool or transport failure.
        self.causes.clear();
    }

    pub fn record(&mut self, event: &Value) -> Option<AppError> {
        let kind = event["kind"].as_str().unwrap_or("api_error");
        let text = plain_text(event["text"].as_str().unwrap_or_default());
        // Claude also uses synthetic model rows for user interruption markers.
        if text.is_empty() || matches!(text.as_str(), "interrupt" | "No response requested." | "(no content)" | "[Request interrupted by user]" | "[Request interrupted by user for tool use]") {
            return None;
        }
        if matches!(kind, "api_error" | "error" | "model_refusal_no_fallback" | "tool_denied" | "permission_denied")
            && !self.causes.contains(&text) {
            if self.causes.len() == 8 { self.causes.remove(0); }
            self.causes.push(text.clone());
        }
        // The append and completion hooks can observe the same native record.
        // Bound bookkeeping even on an exceptionally long-running turn.
        if self.seen.len() >= 128 { self.seen.clear(); }
        if !self.seen.insert((kind.to_string(), text.clone())) { return None; }
        Some(AppError::new(format!("claude_code.{kind}"), format!("Claude Code: {text}"))
            .severity(ErrorSeverity::Warning))
    }

    pub fn completion_error(&self, event: &Value, pending_tools: bool) -> Option<String> {
        let reason = event["reason"].as_str().unwrap_or_default();
        let mut details = Vec::new();
        if reason == "error" || reason == "refusal" {
            details.extend(self.causes.iter().cloned());
            // Refusal is a structured verdict, not necessarily present in answer.
            for key in ["category", "explanation"] {
                if let Some(value) = event["refusal"][key].as_str() {
                    let value = plain_text(value);
                    if !value.is_empty() && !details.iter().any(|s| s.contains(&value)) {
                        details.push(format!("{key}: {value}"));
                    }
                }
            }
            if details.is_empty() {
                let answer = plain_text(event["answer"].as_str().unwrap_or_default());
                if !answer.is_empty() { details.push(answer); }
            }
            details.insert(0, format!("Claude turn ended: {reason}."));
        } else if event["isAborted"] == true || reason == "aborted" {
            details.push("Claude turn was interrupted".into());
        } else if pending_tools {
            // Even when completion lacks an error reason, never lose a known
            // native cause behind the generic missing-result guard.
            details.extend(self.causes.iter().cloned());
        }
        if pending_tools { details.push(PENDING_TOOLS.into()); }
        (!details.is_empty()).then(|| details.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upstream_errors_take_precedence_over_unfinished_tool_input() {
        for cause in ["API Error: Output blocked by content filtering policy", "Invalid API key", "Rate limit exceeded", "Credit balance is too low", "Prompt is too long", "Overloaded", "Model not found"] {
            let mut diagnostics = TurnDiagnostics::default();
            let notice = diagnostics.record(&json!({"text":cause})).unwrap();
            assert_eq!(notice.severity, ErrorSeverity::Warning);
            let error = diagnostics.completion_error(&json!({"reason":"error","answer":"old answer"}), true).unwrap();
            assert!(error.find(cause).unwrap() < error.find(PENDING_TOOLS).unwrap());
            assert!(!error.contains("old answer"));
        }
    }

    #[test]
    fn structured_refusal_explanation_survives_without_a_diagnostic_or_answer() {
        let diagnostics = TurnDiagnostics::default();
        let error = diagnostics.completion_error(&json!({"reason":"refusal","refusal":{"category":"policy","explanation":"Output blocked"}}), true).unwrap();
        assert!(error.contains("category: policy"));
        assert!(error.contains("explanation: Output blocked"));
        assert!(error.contains(PENDING_TOOLS));
    }

    #[test]
    fn retries_and_fallback_notices_do_not_fail_a_successful_turn() {
        let mut diagnostics = TurnDiagnostics::default();
        let event = json!({"kind":"api_error","text":"Temporary service error"});
        assert!(diagnostics.record(&event).is_some());
        assert!(diagnostics.record(&event).is_none());
        diagnostics.start_step();
        assert!(diagnostics.record(&json!({"kind":"model_fallback","text":"Switched to configured model"})).is_some());
        assert!(diagnostics.completion_error(&json!({"reason":"answer"}), false).is_none());
        let error = diagnostics.completion_error(&json!({"reason":"error","answer":"New failure"}), false).unwrap();
        assert!(error.contains("New failure"));
        assert!(!error.contains("Temporary service error"));
    }

    #[test]
    fn diagnostic_repeated_in_a_later_step_still_supplies_the_terminal_cause() {
        let mut diagnostics = TurnDiagnostics::default();
        let event = json!({"text":"Output blocked"});
        diagnostics.record(&event);
        diagnostics.start_step();
        assert!(diagnostics.record(&event).is_none());
        assert!(diagnostics.completion_error(&json!({"reason":"error"}), false).unwrap().contains("Output blocked"));
    }

    #[test]
    fn preserves_unknown_errors_cancellation_and_missing_result_guards() {
        let diagnostics = TurnDiagnostics::default();
        assert_eq!(diagnostics.completion_error(&json!({"reason":"error","answer":"New CLI error"}), false).unwrap(), "Claude turn ended: error.\n\nNew CLI error");
        assert!(diagnostics.completion_error(&json!({"reason":"aborted"}), false).unwrap().contains("interrupted"));
        assert_eq!(diagnostics.completion_error(&json!({"reason":"answer"}), true).unwrap(), PENDING_TOOLS);
    }

    #[test]
    fn ignores_empty_and_interruption_markers_and_sanitizes_native_text() {
        let mut diagnostics = TurnDiagnostics::default();
        for text in ["", "interrupt", "No response requested.", "(no content)", "[Request interrupted by user]", "[Request interrupted by user for tool use]"] {
            assert!(diagnostics.record(&json!({"text":text})).is_none());
        }
        assert_eq!(plain_text("\x1b[31mBlocked\x1b[0m\r\n\0reason"), "Blocked\nreason");
        assert_eq!(plain_text(&"中".repeat(8000)).chars().count(), 6000);
    }
}
