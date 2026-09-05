use serde_json::Value;

/// Plan access covers observation, including runtime inspection, independently
/// of the scheduler's source-file mutation classification.
pub(super) fn allows_observation(tool_name: &str, args: &Value) -> bool {
    match tool_name {
        "bash" | "unity_execute" => args.get("readonly").and_then(Value::as_bool) == Some(true),
        "python" => crate::tool::builtins::python_is_readonly(args),
        "view_wait" => matches!(
            args.get("condition").and_then(Value::as_str),
            Some(
                "runtimeReady"
                    | "selectorVisible"
                    | "selectorHidden"
                    | "textPresent"
                    | "textAbsent"
                    | "noConsoleError"
            )
        ),
        "read"
        | "grep"
        | "list"
        | "ask_user_question"
        | "todowrite"
        | "web_fetch"
        | "code_find_references"
        | "code_goto_definition"
        | "code_symbol_search"
        | "code_diagnostics"
        | "code_hover"
        | "unity_code_usages"
        | "unity_asset_search"
        | "unity_ref_search"
        | "unity_yaml_read"
        | "unity_yaml_search"
        | "unity_capture_viewport"
        | "unity_get_console_log"
        | "unity_test_list"
        | "view_property_read"
        | "view_property_discover"
        | "view_capture"
        | "view_snapshot"
        | "view_console_read"
        | "knowledge_query"
        | "skill_list"
        | "config_query"
        | "tool_load" => true,
        _ => false,
    }
}

pub(super) fn editor_status_change_violation(current: &str, requested: &str) -> Option<String> {
    (current != requested).then(|| {
        format!(
            "Plan mode keeps Unity in its current state ({current}); changing to {requested} requires leaving Plan mode. Inspect the current state or use file reads and code_diagnostics."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_mutations_require_implementation_mode_even_with_readonly_arguments() {
        for tool in [
            "unity_recompile",
            "unity_hot_reload",
            "unity_set_play_mode",
            "unity_test_run",
            "unity_run_states",
            "view_property_write",
            "view_property_apply",
            "view_debug_eval",
            "view_wait",
        ] {
            assert!(
                !allows_observation(tool, &json!({"readonly": true})),
                "{tool}"
            );
        }
    }

    #[test]
    fn observation_scripts_require_an_explicit_readonly_contract() {
        for tool in ["bash", "python", "unity_execute"] {
            assert!(
                allows_observation(tool, &json!({"readonly": true})),
                "{tool}"
            );
            assert!(
                !allows_observation(tool, &json!({"readonly": false})),
                "{tool}"
            );
            assert!(!allows_observation(tool, &json!({})), "{tool}");
        }
        assert!(allows_observation("python", &json!({"action": "help"})));
        assert!(allows_observation(
            "view_wait",
            &json!({"condition": "runtimeReady"})
        ));
        assert!(!allows_observation(
            "view_wait",
            &json!({"condition": "expression", "expression": "location.reload()"})
        ));
    }

    #[test]
    fn diagnostics_and_concrete_inspection_remain_available() {
        for tool in [
            "code_diagnostics",
            "unity_yaml_read",
            "unity_get_console_log",
            "unity_test_list",
            "view_property_read",
            "view_property_discover",
            "web_fetch",
        ] {
            assert!(allows_observation(tool, &json!({})), "{tool}");
        }
    }

    #[test]
    fn readonly_unity_inspection_cannot_request_a_different_editor_state() {
        for current in ["editing", "playing", "playing_paused"] {
            for requested in ["editing", "playing", "playing_paused"] {
                assert_eq!(
                    editor_status_change_violation(current, requested).is_some(),
                    current != requested
                );
            }
        }
    }
}
