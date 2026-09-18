use super::*;
use serde_json::{json, Value};

const PATCH: &str = "*** Begin Patch\n*** Update File: a.cs\n@@\n-old\n+new\n*** End Patch";

fn event(state: &mut CodexStreamState, value: Value) {
    process_sse_event_block(
        &format!("data: {value}"),
        false,
        state,
        &|_| {},
        &|_| {},
        &|_, _| {},
    )
    .unwrap();
}

fn item() -> Value {
    json!({"type":"custom_tool_call", "id":"ctc_1", "call_id":"call_1", "namespace":"functions", "name":"apply_patch", "input":PATCH})
}

fn history(tool: ToolCallInfo) -> Vec<ChatMessage> {
    vec![
        serde_json::from_value(json!({"id":"assistant_1", "role":"assistant", "content":"", "createdAt":0, "toolCalls":[tool]})).unwrap(),
        serde_json::from_value(json!({"id":"tool_1", "role":"tool", "content":"Success", "createdAt":0, "toolCallId":"call_1"})).unwrap(),
    ]
}

#[test]
fn custom_stream_is_executable_only_after_completion_and_replays_as_custom() {
    let mut state = CodexStreamState::new();
    let mut added = item();
    added["input"] = json!("");
    event(
        &mut state,
        json!({"type":"response.output_item.added", "item":added}),
    );
    event(
        &mut state,
        json!({"type":"response.custom_tool_call_input.delta", "item_id":"ctc_1", "delta":PATCH}),
    );
    assert_eq!(collect_complete_tool_calls(&state.tool_calls_map).1, 1);
    event(
        &mut state,
        json!({"type":"response.custom_tool_call_input.done", "item_id":"ctc_1", "input":PATCH}),
    );
    event(
        &mut state,
        json!({"type":"response.output_item.done", "item":item()}),
    );
    let (calls, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
    assert_eq!(incomplete, 0);
    assert_eq!(calls.len(), 1);
    let call = calls.into_iter().next().unwrap().tool_call;
    assert_eq!(call.name, "apply_patch");
    assert_eq!(
        serde_json::from_str::<Value>(&call.arguments).unwrap(),
        json!({"patch":PATCH})
    );
    let history = history(call);
    let replay = build_input(&history);
    assert_eq!(
        replay[0],
        json!({"type":"custom_tool_call", "call_id":"call_1", "name":"apply_patch", "input":PATCH})
    );
    assert_eq!(
        replay[1],
        json!({"type":"custom_tool_call_output", "call_id":"call_1", "output":"Success"})
    );
    let metadata = protocol::response_metadata(
        json!({}),
        &[item()],
        "",
        history[0].tool_calls.as_ref().unwrap(),
        &json!({}),
    );
    let metadata = HashMap::from([("assistant_1".into(), metadata)]);
    let opaque = build_input_with_metadata(&history, Some(&metadata));
    assert_eq!(opaque[0], item());
    assert_eq!(opaque[1]["type"], "custom_tool_call_output");
    // Session export/import can retain the existing JSON envelope without a schema change.
    let roundtrip: Vec<ChatMessage> =
        serde_json::from_str(&serde_json::to_string(&history).unwrap()).unwrap();
    assert_eq!(build_input(&roundtrip), replay);
}

#[test]
fn completed_only_custom_response_is_not_lost() {
    let mut state = CodexStreamState::new();
    event(
        &mut state,
        json!({"type":"response.completed", "response":{"id":"resp_1", "status":"completed", "output":[item()]}}),
    );
    let (calls, incomplete) = collect_complete_tool_calls(&state.tool_calls_map);
    assert_eq!(incomplete, 0);
    assert_eq!(calls.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&calls[0].tool_call.arguments).unwrap()["patch"],
        PATCH
    );
}

#[test]
fn patch_declaration_supports_standard_and_lite_requests() {
    let registry = crate::tool::ToolRegistry::with_builtins();
    let tools = vec![
        registry.resolve_api_tool("apply_patch").unwrap(),
        registry.resolve_api_tool("read").unwrap(),
    ];
    for model in ["gpt-5.6-sol", "gpt-6-astra"] {
        let body = build_request_body(
            model,
            "prompt",
            &[],
            &tools,
            None,
            Some("session"),
            None,
            CodexStreamOptions::default(),
        );
        let declarations = if protocol::uses_lite(model) {
            &body["input"][0]["tools"][0]["tools"]
        } else {
            &body["tools"]
        };
        let declarations = declarations.as_array().unwrap();
        assert_eq!(declarations[0]["type"], "custom");
        assert_eq!(declarations[0]["name"], "apply_patch");
        assert_eq!(declarations[0]["format"]["syntax"], "lark");
        assert_eq!(declarations[1]["type"], "function");
        assert!(!declarations.iter().any(|tool| tool["name"] == "edit"));
    }
    let unchanged = convert_tools(&[registry.resolve_api_tool("edit").unwrap()]);
    assert_eq!(unchanged[0]["type"], "function");
    assert_eq!(unchanged[0]["name"], "edit");
    let deferred = build_tool_search_output_item("search_1", Some(&json!({"tools":[{
        "type":"function", "name":"apply_patch", "description":"patch", "parameters":{}, "defer_loading":true
    }]}).to_string()));
    assert_eq!(deferred["tools"][0]["type"], "custom");
    assert_eq!(deferred["tools"][0]["defer_loading"], true);
}
