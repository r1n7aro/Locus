use super::*;
use std::collections::HashSet;

#[test]
fn upstream_model_reads_wire_protocols_and_terminal_precedence() {
    assert_eq!(
        from_wire("data: {\"model\":\"adjacent\"}\ndata: {\"choices\":[]}\ndata: [DONE]\n")
            .as_deref(),
        Some("adjacent")
    );
    for (wire, expected) in [
        ("event: response.created\r\ndata: {\"response\":{\"model\":\"early\"}}\r\n\r\nevent: response.completed\r\ndata: {\"response\":{\"model\":\"terminal\"}}\r\n\r\n", Some("terminal")),
        ("data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{}}\n\n", Some("claude")),
        ("data: {\"model\":\"chat-model\",\"choices\":[]}\n\ndata: {\"choices\":[]}\n\ndata: [DONE]\n\n", Some("chat-model")),
        ("data: {\"response\": {\n data ignored\n", None),
        ("data: {\"response\":\n data ignored\n", None),
        ("data: {\"response\":\ndata: {\"model\":\"multiline\"}}\n\n", Some("multiline")),
        ("data: {\"model\":\"first\"}\n\ndata: {\"model\":\"later\"}\n\n", Some("first")),
        ("data: {\"model\":\"unterminated\"}", Some("unterminated")),
        ("{\"model\":\" json-model \"}", Some("json-model")),
        ("{\"model\":12}", None),
        ("{\"model\":\"  \"}", None),
        ("data: {\"model\":\"malformed\"\n\n", None),
        ("{\"choices\":[]}", None),
    ] {
        assert_eq!(from_wire(wire).as_deref(), expected, "{wire}");
    }
}

#[test]
fn upstream_model_records_actual_wire_model_and_does_not_invent_missing_identity() {
    let mut metadata = None;
    assert_eq!(
        record(
            &mut metadata,
            r#"{"model":"wire-alias","messages":["secret prompt"]}"#,
            "data: {\"model\":\"reported\"}\n\n"
        ),
        Some(("wire-alias".into(), "reported".into()))
    );
    assert_eq!(
        metadata,
        Some(json!({"model":"wire-alias","upstream_model":"reported"}))
    );
    let mut next = None;
    assert_eq!(
        record(&mut next, r#"{"model":"wire-alias"}"#, "data: {}\n\n"),
        None
    );
    assert_eq!(from_request(&next.unwrap()), None);

    let mut codex = Some(
        json!({"model":"requested","codex_response":{"version":1,"server_model":"header-model","output":[{"type":"message"}]}}),
    );
    record(
        &mut codex,
        r#"{"model":"requested"}"#,
        r#"{"model":"body-model"}"#,
    );
    let codex = codex.unwrap();
    assert_eq!(codex["upstream_model"], "header-model");
    assert_eq!(codex["codex_response"]["output"][0]["type"], "message");
}

#[test]
fn upstream_model_warning_is_nonfatal_case_insensitive_and_once_per_pair_per_run() {
    let mut seen = HashSet::new();
    assert!(mismatch_warning(&mut seen, None, "session").is_none());
    assert!(
        mismatch_warning(&mut seen, Some(("Model".into(), "MODEL".into())), "session").is_none()
    );
    let pair = Some(("requested".into(), "reported".into()));
    let warning = mismatch_warning(&mut seen, pair.clone(), "session").unwrap();
    assert_eq!(warning.severity, crate::error::ErrorSeverity::Warning);
    assert_eq!(warning.code, "llm.upstream_model_mismatch");
    assert_eq!(warning.operation.as_deref(), Some("upstream-model:session"));
    let detail: Value = serde_json::from_str(warning.detail.as_deref().unwrap()).unwrap();
    assert_eq!(detail["requestedModel"], "requested");
    assert_eq!(detail["reportedModel"], "reported");
    assert!(mismatch_warning(&mut seen, pair.clone(), "session").is_none());
    assert!(mismatch_warning(&mut HashSet::new(), pair, "next-run").is_some());
}
