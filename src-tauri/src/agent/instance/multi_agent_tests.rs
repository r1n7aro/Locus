use super::*;

fn instance() -> AgentInstance {
    let mut tools = ToolRegistry::with_builtins();
    tools.register_subagent_tool(&[("explorer".to_string(), "Research".to_string())]);
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    AgentInstance::new(
        Arc::new(AgentDef {
            id: "multi-agent-test".to_string(),
            name: "Test".to_string(),
            description: String::new(),
            project_types: Vec::new(),
            system_prompt: String::new(),
            env_template: String::new(),
            tools: vec!["read".to_string()],
            sub_agents: Vec::new(),
            default: false,
            default_effort: None,
            model_recommendation: None,
            tool_description_overrides: HashMap::new(),
            source: "test".to_string(),
        }),
        "session-multi-agent",
        LlmBackend::ClaudeCodeCli,
        false,
        Arc::new(AgentDefRegistry::load(None, None)),
        Arc::new(tools),
        "C:/Project".to_string(),
        RawContextStore::default(),
        None,
        "test-model".to_string(),
        Some("max".to_string()),
        Arc::new(None),
        Arc::new(None),
        KnowledgeAccessMode::Full,
        None,
        HashMap::new(),
        cancel_rx,
    )
}

#[tokio::test]
async fn multi_agent_toggle_gates_direct_lazy_and_skill_tool_surfaces() {
    let mut agent = instance();
    let skill_tools = HashSet::from(["subagent".to_string(), "task".to_string()]);
    let requested = vec!["read".to_string(), "subagent".to_string()];
    for enabled in [false, true, false] {
        agent.set_multi_agent_enabled(enabled);
        let allowed = agent.allowed_tool_set_for_active_skills(&skill_tools).await;
        assert_eq!(allowed.contains("subagent"), enabled);
        assert!(allowed.contains("read"));
        let tools = agent.build_api_tools(&requested).await;
        assert_eq!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "subagent"),
            enabled
        );
        let legacy_name = agent.canonical_tool_name("task").unwrap();
        assert_eq!(agent.resolve_api_tool(&legacy_name).is_some(), enabled);
        let loaded = agent
            .execute_tool_load_with_mode_and_skills(
                &serde_json::json!({"tools": ["subagent"]}),
                crate::config::DynamicToolLoadingMode::MetaTool,
                &skill_tools,
            )
            .await;
        let loaded_json: serde_json::Value = serde_json::from_str(&loaded.output).unwrap();
        assert_eq!(
            loaded_json["tools"][0]["status"],
            if enabled { "described" } else { "not_allowed" }
        );
        if enabled {
            let description = tools
                .iter()
                .find(|tool| tool["function"]["name"] == "subagent")
                .unwrap()["function"]["description"]
                .as_str()
                .unwrap();
            assert!(description.contains(PROACTIVE_DELEGATION_GUIDANCE));
            assert!(loaded_json["tools"][0]
                .to_string()
                .contains(PROACTIVE_DELEGATION_GUIDANCE));
        } else {
            assert_eq!(
                agent.tool_runtime_unavailable_reason("subagent"),
                Some("multi_agent_disabled")
            );
            assert!(agent.multi_agent_disabled_result().is_error);
        }
        assert_eq!(agent.effort.as_deref(), Some("max"));
    }
}

#[tokio::test]
async fn multi_agent_disabled_adds_explicit_delegation_policy_to_new_prompts() {
    let mut agent = instance();
    agent.working_dir.clear();
    let disabled = agent.build_system_prompt_parts().await;
    assert!(disabled.rules_prompt.contains(EXPLICIT_DELEGATION_GUIDANCE));
    assert!(disabled.rules_prompt.contains("<multi_agent_mode>"));
    agent.set_multi_agent_enabled(true);
    let enabled = agent.build_system_prompt_parts().await;
    assert!(!enabled.rules_prompt.contains(EXPLICIT_DELEGATION_GUIDANCE));
}

#[tokio::test]
async fn multi_agent_python_policy_survives_overrides_and_lazy_loading() {
    let mut agent = instance();
    Arc::make_mut(&mut agent.def)
        .tool_description_overrides
        .insert(
            "python".to_string(),
            crate::agent::definition::AgentToolDescriptionOverride {
                description: Some("Run workspace Python.".to_string()),
                parameters: None,
            },
        );
    for enabled in [false, true] {
        agent.set_multi_agent_enabled(enabled);
        for async_tasks in [false, true] {
            agent.set_async_tasks_enabled(async_tasks);
            let direct = agent.build_api_tools(&["python".to_string()]).await;
            let description = direct[0]["function"]["description"].as_str().unwrap();
            assert!(description.starts_with("Run workspace Python."));
            assert_eq!(
                description.matches(EXPLICIT_DELEGATION_GUIDANCE).count(),
                usize::from(!enabled)
            );
            let (text, parameters) = agent.tool_description("python").unwrap();
            let (lazy_description, _) =
                agent.contextualize_tool_description("python", text, parameters);
            assert_eq!(lazy_description, description);
        }
    }
}

#[tokio::test]
async fn multi_agent_inspection_preserves_schema_when_session_disables_calls() {
    let mut agent = instance();
    // Inspect an agent that explicitly declares the tool, as the Agent page does.
    Arc::make_mut(&mut agent.def)
        .tools
        .push("subagent".to_string());
    for enabled in [false, true] {
        agent.set_multi_agent_enabled(enabled);
        let items = agent.available_tool_prompt_items().await;
        let item = items.iter().find(|item| item.title == "subagent").unwrap();
        let meta = item.meta.as_ref().unwrap();
        assert!(item.content.contains("Delegate a bounded"));
        assert!(meta["function"]["parameters"]["properties"]["prompt"].is_object());
        assert_eq!(meta["runtimeAvailable"], enabled);
        assert_eq!(
            meta["unavailableReason"],
            if enabled {
                serde_json::Value::Null
            } else {
                serde_json::json!("multi_agent_disabled")
            }
        );
        assert_eq!(agent.resolve_api_tool("subagent").is_some(), enabled);
    }
    agent.subagent_tool_suppressed = true;
    let items = agent.available_tool_prompt_items().await;
    let meta = items
        .iter()
        .find(|item| item.title == "subagent")
        .unwrap()
        .meta
        .as_ref()
        .unwrap();
    assert_eq!(meta["unavailableReason"], "subagent_depth_limit");
    assert!(meta["function"]["parameters"]["properties"]["prompt"].is_object());
    assert!(agent.resolve_api_tool("subagent").is_none());
}

#[tokio::test]
async fn multi_agent_policy_is_preserved_by_background_clones_and_depth_limits() {
    let mut agent = instance();
    agent.set_multi_agent_enabled(true);
    let clone = agent.clone_for_background_task(agent.cancel_waiter());
    assert!(clone.multi_agent_enabled);
    assert!(clone.allowed_tool_set().await.contains("subagent"));
    agent.subagent_tool_suppressed = true;
    assert!(!agent.allowed_tool_set().await.contains("subagent"));
    assert!(agent
        .build_api_tools(&["subagent".to_string()])
        .await
        .is_empty());
    agent.set_multi_agent_enabled(false);
    let clone = agent.clone_for_background_task(agent.cancel_waiter());
    assert!(!clone.multi_agent_enabled);
}
