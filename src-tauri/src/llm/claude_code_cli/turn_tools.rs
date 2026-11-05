//! Per-turn tool identities. Streaming starts are not completed calls, and a
//! second observer of a native result must never duplicate persistence/execution.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(super) struct TurnTools {
    pub calls: HashMap<String, Value>,
    started: HashSet<String>,
    completed: HashSet<String>,
    step: Option<u64>,
}
impl TurnTools {
    /// A new model request retires previews from the previous response, not
    /// committed calls. The CLI can recover a truncated stream without ever
    /// committing its last tool input. Such a preview has nothing to execute or
    /// return; retaining it would poison an otherwise successful native turn.
    pub fn start_step(&mut self, step: u64) -> Vec<String> {
        if self.step.is_some_and(|previous| step <= previous) { return Vec::new(); }
        self.step = Some(step);
        let mut abandoned = Vec::new();
        self.started.retain(|id| {
            if self.calls.contains_key(id) { return true; }
            abandoned.push(id.clone());
            false
        });
        abandoned.sort();
        abandoned
    }

    pub fn start(&mut self, id: &str) { self.started.insert(id.into()); }
    pub fn announce(&mut self, block: &Value) -> Result<bool, String> {
        let id = block["id"].as_str().filter(|id| !id.is_empty()).ok_or("Claude tool has no identity")?;
        if let Some(previous) = self.calls.get(id) {
            if previous["name"] != block["name"] || previous["input"] != block["input"] {
                return Err("Claude reused a tool identity for a different operation; no tool was repeated".into());
            }
            return Ok(false);
        }
        self.calls.insert(id.into(), block.clone());
        self.start(id);
        Ok(true)
    }
    pub fn complete(&mut self, id: &str) -> Result<bool, String> {
        if !self.calls.contains_key(id) {
            return Err("Claude returned a result without its tool envelope. Resume from native history; no result was fabricated.".into());
        }
        Ok(self.completed.insert(id.into()))
    }
    pub fn has_pending(&self) -> bool {
        self.started.iter().any(|id| !self.completed.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn concurrent_results_and_native_history_fallback_are_recorded_exactly_once() {
        let mut tools = TurnTools::default();
        for id in ["a", "b", "c", "d"] {
            tools.start(id);
            tools.announce(&json!({"id":id,"name":"WebSearch","input":{"query":id}})).unwrap();
        }
        for id in ["d", "b", "c", "a"] {
            assert!(tools.complete(id).unwrap());
            assert!(!tools.complete(id).unwrap()); // session.append / .catch replay
        }
        assert!(!tools.has_pending());
    }
    #[test]
    fn an_incomplete_streamed_call_cannot_leave_a_running_card_after_complete() {
        let mut tools = TurnTools::default();
        tools.start("only-streamed");
        assert!(tools.has_pending());
        assert!(tools.complete("only-streamed").is_err());
    }
    #[test]
    fn claude_stream_retry_retires_only_uncommitted_previews() {
        use super::super::diagnostics::TurnDiagnostics;
        let mut tools = TurnTools::default();
        let mut diagnostics = TurnDiagnostics::default();
        assert!(tools.start_step(1).is_empty());
        let call = json!({"id":"committed","name":"Grep","input":{"pattern":"one"}});
        tools.start("committed");
        tools.announce(&call).unwrap();
        tools.complete("committed").unwrap();
        // Incident: an empty Grep preview precedes the native API diagnostic;
        // Claude then continues inside the same turn with a new model step.
        tools.start("truncated-grep");
        diagnostics.record(&json!({"kind":"api_error","text":"API Error: Connection lost mid-response. The response above may be incomplete."}));
        assert!(tools.has_pending());
        assert_eq!(tools.start_step(2), vec!["truncated-grep"]);
        diagnostics.start_step();
        tools.announce(&json!({"id":"continued","name":"Bash","input":{"command":"echo done"}})).unwrap();
        tools.complete("continued").unwrap();
        assert!(diagnostics.completion_error(&json!({"reason":"answer"}), tools.has_pending()).is_none());
        assert!(!tools.announce(&call).unwrap());
        assert!(!tools.complete("committed").unwrap());
        // Retiring the GUI preview does not manufacture an envelope/result.
        assert!(!tools.calls.contains_key("truncated-grep"));
        assert!(!tools.completed.contains("truncated-grep"));
        assert!(tools.complete("truncated-grep").is_err());
    }

    #[test]
    fn claude_stream_retry_preserves_real_calls_awaiting_results() {
        use super::super::diagnostics::TurnDiagnostics;
        let mut tools = TurnTools::default();
        tools.start_step(1);
        tools.announce(&json!({"id":"write","name":"Write","input":{"content":"effect"}})).unwrap();
        tools.start("preview");
        assert_eq!(tools.start_step(2), vec!["preview"]);
        assert!(tools.has_pending());
        let error = TurnDiagnostics::default().completion_error(&json!({"reason":"answer"}), tools.has_pending()).unwrap();
        assert!(error.contains("did not return every tool result"));
        assert!(tools.complete("write").unwrap());
        assert!(!tools.has_pending());
    }

    #[test]
    fn claude_stream_preview_retirement_is_scoped_to_new_steps() {
        let mut tools = TurnTools::default();
        tools.start_step(1);
        tools.start("first");
        tools.start("first");
        assert!(tools.start_step(1).is_empty());
        assert!(tools.has_pending());
        assert_eq!(tools.start_step(2), vec!["first"]);
        tools.start("second");
        assert!(tools.start_step(1).is_empty());
        assert!(tools.start_step(2).is_empty());
        assert!(tools.has_pending());
        assert_eq!(tools.start_step(3), vec!["second"]);
        assert!(tools.start_step(4).is_empty());
        assert!(!tools.has_pending());
    }

    #[test]
    fn a_reserved_dispatch_cannot_execute_twice_or_change_its_arguments() {
        let mut tools = TurnTools::default();
        let call = json!({"id":"write","name":"Write","input":{"content":"one"}});
        assert!(tools.announce(&call).unwrap());
        assert!(tools.complete("write").unwrap());
        assert!(!tools.announce(&call).unwrap());
        assert!(!tools.complete("write").unwrap());
        assert!(tools.announce(&json!({"id":"write","name":"Write","input":{"content":"two"}})).is_err());
    }
}
