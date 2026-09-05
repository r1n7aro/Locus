use super::*;
use crate::commands::{ApiFormat, CustomProvider, CustomProviderModel};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn definition() -> AgentDef {
    AgentDef {
        id: "explorer".into(),
        name: "Explorer".into(),
        description: String::new(),
        project_types: Vec::new(),
        system_prompt: String::new(),
        env_template: String::new(),
        tools: Vec::new(),
        sub_agents: Vec::new(),
        default: false,
        default_effort: None,
        model_recommendation: None,
        tool_description_overrides: HashMap::new(),
        source: "test".into(),
    }
}

fn parent(store: &SessionStore) -> (AgentInstance, String) {
    let parent_id = store
        .create_session("Parent", None, None, "chat", Some("unity"))
        .unwrap();
    let child_id = store
        .create_session("Child", Some(&parent_id), None, "chat", Some("explorer"))
        .unwrap();
    let definitions = tempfile::tempdir().unwrap();
    let agent_dir = definitions.path().join("agents");
    std::fs::create_dir_all(agent_dir.join("explorer")).unwrap();
    std::fs::write(
        agent_dir.join("explorer/config.json"),
        serde_json::to_vec(&definition()).unwrap(),
    )
    .unwrap();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let mut parent = AgentInstance::new(
        Arc::new(definition()),
        &parent_id,
        LlmBackend::ClaudeCodeCli,
        false,
        Arc::new(AgentDefRegistry::load(Some(&agent_dir), None)),
        Arc::new(ToolRegistry::new()),
        String::new(),
        RawContextStore::default(),
        None,
        "claude_code/parent".into(),
        Some("high".into()),
        Arc::new(None),
        Arc::new(None),
        KnowledgeAccessMode::Full,
        None,
        HashMap::new(),
        cancel_rx,
    );
    parent.codex_fast_mode = true;
    parent.multi_agent_enabled = true;
    parent
        .subagent_model_overrides
        .insert("explorer".into(), "custom/child/luna".into());
    parent.set_subagent_runtime_overrides(
        HashMap::from([("explorer".into(), "max".into())]),
        HashMap::from([("explorer".into(), false)]),
    );
    (parent, child_id)
}

fn custom_backend(endpoint: &str, format: ApiFormat, api_model: &str, key: &str) -> LlmBackend {
    let provider: CustomProvider = serde_json::from_value(json!({
        "id": "fixture", "name": "Fixture", "endpoint": endpoint, "apiFormat": format,
    }))
    .unwrap();
    let model: CustomProviderModel = serde_json::from_value(json!({
        "id": "fixture-model", "name": "Fixture model", "apiModel": api_model,
        "contextLength": if api_model == "gpt-5.6-sol" { 256_000 } else { 64_000 },
        "supportsVision": api_model == "gpt-5.6-sol",
        "supportedReasoningEfforts": if api_model == "gpt-5.6-sol" { vec!["high"] } else { vec!["max"] },
        "reasoningParamFormat": match format {
            ApiFormat::OpenaiChat => "openai_chat_reasoning_effort",
            ApiFormat::OpenaiResponses => "openai_responses_reasoning_effort",
            ApiFormat::AnthropicMessages => "anthropic_thinking",
        },
    }))
    .unwrap();
    crate::commands::custom_backend_from_config(provider, model, key.to_string())
}

struct CapturedRequest {
    headers: String,
    body: Value,
}

/// A terminal 400 captures the actual request without retrying or requiring a
/// protocol-specific streaming fixture. Every address and key is test-local.
async fn capture_server() -> (String, tokio::task::JoinHandle<CapturedRequest>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(15), async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (header_end, content_length) = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0, "request headers ended early");
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).expect("content length");
                    break (end + 4, length);
                }
            };
            while bytes.len() < header_end + content_length {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0, "request body ended early");
                bytes.extend_from_slice(&buffer[..n]);
            }
            socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 8\r\nConnection: close\r\n\r\ncaptured").await.unwrap();
            CapturedRequest {
                headers: String::from_utf8_lossy(&bytes[..header_end]).to_lowercase(),
                body: serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap(),
            }
        }).await.expect("local request timed out")
    });
    (base_url, task)
}

async fn request(child: &AgentInstance, store: &SessionStore) {
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        child.call_llm(
            store,
            None,
            LlmRequestOptions::default(),
            &["Test"],
            &[],
            &[],
            None,
            |_| {},
            |_| {},
            |_, _| {},
        ),
    )
    .await
    .expect("request timed out");
    assert!(result
        .err()
        .expect("capture endpoint rejects request")
        .contains("captured"));
}

async fn assert_custom_request(format: ApiFormat, same_provider: bool, resumed: bool) {
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path()).unwrap();
    let (mut parent, child_id) = parent(&store);
    let (base, captured) = capture_server().await;
    parent.effective_model = "custom/parent/sol".into();
    parent.backend = custom_backend(
        &format!("{base}/parent/v1"),
        ApiFormat::OpenaiChat,
        "gpt-5.6-sol",
        "parent-key",
    );
    let endpoint = format!(
        "{base}/{}/v1",
        if same_provider { "parent" } else { "child" }
    );
    let child_model = if same_provider {
        "custom/parent/luna"
    } else {
        "custom/child/luna"
    };
    parent
        .subagent_model_overrides
        .insert("explorer".into(), child_model.into());
    if resumed {
        parent.resumed_subagent = Some(crate::async_tasks::SubagentResumeInfo {
            child_session_id: child_id.clone(),
            agent_id: "explorer".into(),
            working_dir: String::new(),
            model_id: child_model.into(),
            effort: Some("max".into()),
            fast_mode: false,
            readonly: true,
        });
        // The parent and defaults can change after a task was suspended.
        parent
            .subagent_model_overrides
            .insert("explorer".into(), "custom/parent/sol".into());
        parent
            .subagent_effort_overrides
            .insert("explorer".into(), "low".into());
    }
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let child_format = format.clone();
    let child_api_model = match format {
        ApiFormat::AnthropicMessages => "claude-opus-4-8",
        _ => "gpt-5.6-luna",
    };
    let child = parent
        .new_subagent_instance_with(
            &store,
            definition(),
            &child_id,
            cancel_rx,
            |model| async move {
                assert_eq!(model, child_model);
                Ok(custom_backend(
                    &endpoint,
                    child_format,
                    child_api_model,
                    "child-key",
                ))
            },
        )
        .await
        .unwrap();
    assert_eq!(child.effective_model, child_model);
    assert_eq!(child.context_limits().effective_context_window, 64_000);
    assert!(!child.supports_image_understanding());
    assert_eq!(child.effort.as_deref(), Some("max"));
    assert!(!child.codex_fast_mode);
    assert_eq!(child.is_plan_readonly_subagent(), resumed);
    let saved = store.load_session(&child_id).unwrap();
    assert_eq!(saved.last_model_id.as_deref(), Some(child_model));
    assert_eq!(saved.last_effort.as_deref(), Some("max"));
    assert_eq!(saved.last_fast_mode, Some(false));
    assert_eq!(parent.effective_model, "custom/parent/sol");
    request(&child, &store).await;
    let captured = captured.await.unwrap();
    let path = match format {
        ApiFormat::OpenaiChat => "chat/completions",
        ApiFormat::OpenaiResponses => "responses",
        ApiFormat::AnthropicMessages => "messages",
    };
    assert!(
        captured.headers.starts_with(&format!(
            "post /{}/v1/{path} ",
            if same_provider { "parent" } else { "child" }
        )),
        "{}",
        captured.headers
    );
    assert!(captured.headers.contains("child-key"));
    assert!(!captured.headers.contains("parent-key"));
    assert_eq!(captured.body["model"], child_api_model);
    match format {
        ApiFormat::OpenaiChat => assert_eq!(captured.body["reasoning_effort"], "max"),
        ApiFormat::OpenaiResponses => assert_eq!(captured.body["reasoning"]["effort"], "max"),
        ApiFormat::AnthropicMessages => assert_eq!(captured.body["output_config"]["effort"], "max"),
    }
}

#[tokio::test]
async fn same_custom_provider_uses_child_api_model() {
    assert_custom_request(ApiFormat::OpenaiChat, true, false).await;
    assert_custom_request(ApiFormat::OpenaiResponses, true, false).await;
}

#[tokio::test]
async fn different_custom_provider_uses_child_endpoint_credentials_and_protocol() {
    assert_custom_request(ApiFormat::OpenaiResponses, false, false).await;
    assert_custom_request(ApiFormat::AnthropicMessages, false, false).await;
}

#[tokio::test]
async fn resumed_subagent_keeps_saved_model_and_resolves_its_backend_again() {
    assert_custom_request(ApiFormat::OpenaiResponses, false, true).await;
}

#[tokio::test]
async fn builtin_parent_can_spawn_custom_child() {
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path()).unwrap();
    let (parent, child_id) = parent(&store);
    let (base, captured) = capture_server().await;
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let child = parent
        .new_subagent_instance_with(
            &store,
            definition(),
            &child_id,
            cancel_rx,
            |model| async move {
                assert_eq!(model, "custom/child/luna");
                Ok(custom_backend(
                    &base,
                    ApiFormat::OpenaiResponses,
                    "gpt-5.6-luna",
                    "child-key",
                ))
            },
        )
        .await
        .unwrap();
    request(&child, &store).await;
    assert_eq!(captured.await.unwrap().body["model"], "gpt-5.6-luna");
}

#[tokio::test]
async fn custom_parent_can_spawn_openrouter_child() {
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path()).unwrap();
    let (mut parent, child_id) = parent(&store);
    let (base, captured) = capture_server().await;
    parent.effective_model = "custom/parent/sol".into();
    parent.backend = custom_backend(
        &format!("{base}/parent"),
        ApiFormat::OpenaiResponses,
        "gpt-5.6-sol",
        "parent-key",
    );
    parent
        .subagent_model_overrides
        .insert("explorer".into(), "openrouter/openai/gpt-5.6-luna".into());
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let child = parent
        .new_subagent_instance_with(
            &store,
            definition(),
            &child_id,
            cancel_rx,
            |model| async move {
                assert_eq!(model, "openrouter/openai/gpt-5.6-luna");
                Ok(LlmBackend::OpenRouter {
                    api_key: "openrouter-key".into(),
                    base_url: Some(base.clone()),
                })
            },
        )
        .await
        .unwrap();
    request(&child, &store).await;
    let captured = captured.await.unwrap();
    assert!(captured
        .headers
        .starts_with("post /api/v1/chat/completions "));
    assert!(captured.headers.contains("openrouter-key"));
    assert_eq!(captured.body["model"], "openai/gpt-5.6-luna");
}

#[tokio::test]
async fn inherited_model_also_resolves_fresh_backend() {
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path()).unwrap();
    let (mut parent, child_id) = parent(&store);
    parent.subagent_model_overrides.clear();
    parent.subagent_effort_overrides.clear();
    parent.subagent_fast_mode_overrides.clear();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let child = parent
        .new_subagent_instance_with(
            &store,
            definition(),
            &child_id,
            cancel_rx,
            |model| async move {
                assert_eq!(model, "claude_code/parent");
                Ok(LlmBackend::ClaudeCodeCli)
            },
        )
        .await
        .unwrap();
    assert_eq!(child.effort.as_deref(), Some("high"));
    assert!(child.codex_fast_mode);
    assert_eq!(
        store
            .load_session(&child_id)
            .unwrap()
            .last_model_id
            .as_deref(),
        Some("claude_code/parent")
    );
}

#[tokio::test]
async fn unavailable_child_model_fails_without_falling_back_or_overwriting_saved_settings() {
    let temp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(temp.path()).unwrap();
    let (parent, child_id) = parent(&store);
    store
        .set_session_execution_state(&child_id, "custom/previous/model", Some("low"), false, None)
        .unwrap();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let result = parent
        .new_subagent_instance_with(&store, definition(), &child_id, cancel_rx, |_| async {
            Err("Custom model config not found".into())
        })
        .await;
    let error = result.err().expect("invalid child model must fail");
    assert!(error.contains("custom/child/luna"));
    assert!(error.contains("Custom model config not found"));
    let saved = store.load_session(&child_id).unwrap();
    assert_eq!(
        saved.last_model_id.as_deref(),
        Some("custom/previous/model")
    );
    assert_eq!(saved.last_effort.as_deref(), Some("low"));
    assert_eq!(saved.last_fast_mode, Some(false));
}
