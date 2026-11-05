use std::sync::Arc;

use super::{ToolDef, ToolExecuteFn, ToolResult};

pub(super) fn truncate_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        &s[..s.floor_char_boundary(max_bytes)]
    }
}

/// Truncate to roughly `max_bytes` keeping the head and the tail, dropping the
/// middle. CLI output carries errors and final results at the END, so a
/// prefix-only cut hides exactly the part the agent needs to diagnose.
pub(super) fn truncate_utf8_middle(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let head_budget = max_bytes / 2;
    let tail_budget = max_bytes - head_budget;
    let head_end = s.floor_char_boundary(head_budget);
    let tail_start = s.ceil_char_boundary(s.len() - tail_budget);
    format!(
        "{}\n\n... [{} bytes truncated in the middle] ...\n\n{}",
        &s[..head_end],
        tail_start - head_end,
        &s[tail_start..]
    )
}

// ─── todowrite ──────────────────────────────────────────────────────────────

pub(super) fn todowrite() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: todowrite tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::TODOWRITE);
    ToolDef {
        name: "todowrite".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
}

/// Offered only while the session has Multi-Agent enabled; the agent loop
/// intercepts calls because delivery needs the sender's session identity.
pub(super) fn send_message() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: send_message tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::SEND_MESSAGE);
    ToolDef {
        name: "send_message".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
}

// ─── ask ─────────────────────────────────────────────────────────────────────

pub(super) fn ask() -> ToolDef {
    let execute: ToolExecuteFn = Arc::new(|_args, _ctx| {
        Box::pin(async {
            ToolResult {
                output: "Error: ask tool should be intercepted by agent loop".to_string(),
                is_error: true,
            }
        })
    });

    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::ASK);
    ToolDef {
        name: "ask_user_question".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute,
    }
}

#[cfg(test)]
mod tests {
    use super::{truncate_utf8_middle, truncate_utf8_prefix};

    #[test]
    fn truncate_utf8_prefix_handles_cjk_boundary() {
        let text = "abc中文def";
        assert_eq!(truncate_utf8_prefix(text, 4), "abc");
        assert_eq!(truncate_utf8_prefix(text, 6), "abc中");
    }

    #[test]
    fn truncate_utf8_prefix_handles_emoji_boundary() {
        let text = "ab😀cd";
        assert_eq!(truncate_utf8_prefix(text, 3), "ab");
        assert_eq!(truncate_utf8_prefix(text, 6), "ab😀");
    }

    #[test]
    fn truncate_utf8_middle_keeps_head_and_tail() {
        let text = format!("HEAD{}TAIL", "x".repeat(1000));
        let truncated = truncate_utf8_middle(&text, 100);
        assert!(truncated.starts_with("HEAD"));
        assert!(truncated.ends_with("TAIL"));
        assert!(truncated.contains("truncated in the middle"));
    }

    #[test]
    fn truncate_utf8_middle_returns_short_input_unchanged() {
        assert_eq!(truncate_utf8_middle("short", 100), "short");
    }

    #[test]
    fn truncate_utf8_middle_respects_char_boundaries() {
        let text = "中".repeat(100);
        let truncated = truncate_utf8_middle(&text, 31);
        assert!(truncated.contains("truncated in the middle"));
        assert!(truncated.starts_with('中'));
        assert!(truncated.ends_with('中'));
    }
}
