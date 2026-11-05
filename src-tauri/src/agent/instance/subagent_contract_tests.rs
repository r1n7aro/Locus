use super::*;

fn registry() -> Arc<AgentDefRegistry> {
    let agent_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../agent");
    Arc::new(AgentDefRegistry::load(Some(agent_dir.as_path()), None))
}

fn instance(session_id: &str) -> AgentInstance {
    let registry = registry();
    let mut tools = ToolRegistry::with_builtins();
    tools.register_subagent_tool(&registry.list_subagent_descriptions());
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let mut agent = AgentInstance::new(
        Arc::new(AgentDef {
            id: "contract-test".to_string(),
            name: "Test".to_string(),
            description: String::new(),
            project_types: Vec::new(),
            system_prompt: String::new(),
            env_template: String::new(),
            tools: vec!["read".to_string()],
            sub_agents: Vec::new(),
            delegatable: true,
            default_project_types: Vec::new(),
            default: false,
            default_effort: None,
            model_recommendation: None,
            tool_description_overrides: HashMap::new(),
            source: "test".to_string(),
        }),
        session_id,
        LlmBackend::ClaudeCodeCli,
        false,
        registry,
        Arc::new(tools),
        String::new(),
        RawContextStore::default(),
        None,
        "anthropic/claude-opus-5".to_string(),
        None,
        Arc::new(None),
        Arc::new(None),
        KnowledgeAccessMode::Full,
        None,
        Default::default(),
        cancel_rx,
    );
    agent.set_multi_agent_enabled(true);
    agent
}

fn delegation_type(agent: &AgentInstance) -> String {
    agent
        .registry
        .list_subagent_descriptions_for_project_type(agent.workspace_project_type())
        .into_iter()
        .next()
        .expect("the bundled agents include a delegation target")
        .0
}

async fn published(agent: &AgentInstance) -> serde_json::Value {
    agent.build_api_tools(&["subagent".to_string()]).await[0]["function"].clone()
}

#[tokio::test]
async fn an_existing_session_keeps_its_subagent_definition_after_agent_settings_change() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let existing = store
        .create_session("Existing", None, None, "chat", None)
        .unwrap();

    let agent = instance(&existing);
    let target = delegation_type(&agent);
    let before = published(&agent).await;
    agent.pin_session_tool_contracts(&store);
    assert_eq!(
        published(&agent).await,
        before,
        "pinning must not change the offered definition"
    );

    // The user now offers two models for this agent type.
    let mut changed = instance(&existing);
    changed
        .subagent_model_overrides
        .set(&target, ["openai/model-alpha", "openai/model-beta"]);
    let live = changed.build_api_tools(&["subagent".to_string()]).await[0]["function"].clone();
    assert_ne!(
        live, before,
        "fixture: the settings change must alter the live description"
    );
    assert!(live["parameters"]["properties"]["model"].is_object());

    // The next run of the existing session still offers the original definition.
    changed.pin_session_tool_contracts(&store);
    assert_eq!(published(&changed).await, before);
    let (lazy_description, lazy_parameters) = {
        let (text, parameters) = changed.tool_description("subagent").unwrap();
        changed.contextualize_tool_description("subagent", text, parameters)
    };
    assert_eq!(lazy_description, before["description"].as_str().unwrap());
    assert!(lazy_parameters["properties"]["model"].is_null());
    let clone = changed.clone_for_background_task(changed.cancel_waiter());
    assert_eq!(published(&clone).await, before);

    // A new session is pinned with the current settings.
    let fresh = store
        .create_session("New", None, None, "chat", None)
        .unwrap();
    let mut new_session = instance(&fresh);
    new_session
        .subagent_model_overrides
        .set(&target, ["openai/model-alpha", "openai/model-beta"]);
    new_session.pin_session_tool_contracts(&store);
    assert_eq!(published(&new_session).await, live);
}

#[tokio::test]
async fn sessions_that_never_offered_subagent_are_not_pinned() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    let session = store
        .create_session("Single", None, None, "chat", None)
        .unwrap();
    let mut agent = instance(&session);
    agent.set_multi_agent_enabled(false);
    agent.pin_session_tool_contracts(&store);
    assert!(store
        .session_tool_contract(&session, "subagent")
        .unwrap()
        .is_none());
    agent.subagent_tool_suppressed = true;
    agent.set_multi_agent_enabled(true);
    agent.pin_session_tool_contracts(&store);
    assert!(store
        .session_tool_contract(&session, "subagent")
        .unwrap()
        .is_none());
}

#[test]
fn stale_calls_are_rejected_with_the_current_types_and_models() {
    let mut agent = instance("session-validation");
    let target = delegation_type(&agent);
    agent
        .subagent_model_overrides
        .set(&target, ["openai/model-alpha", "openai/model-beta"]);

    let error = agent
        .subagent_launch_model(&target, Some("anthropic/claude-opus-5"))
        .unwrap_err();
    assert!(error.contains("current Agent settings"), "{error}");
    assert!(
        error.contains("model-alpha") && error.contains("model-beta"),
        "{error}"
    );
    assert_eq!(
        agent
            .subagent_launch_model(&target, Some("openai/model-beta"))
            .unwrap(),
        "openai/model-beta"
    );

    let error = agent
        .resolve_delegation_target("retired-agent")
        .unwrap_err();
    assert!(
        error.contains("Agent type 'retired-agent' is unknown"),
        "{error}"
    );
    assert!(
        error.contains("Currently available agent types and models:"),
        "{error}"
    );
    assert!(error.contains(&format!("- {target}: ")), "{error}");
    assert!(error.contains("model-alpha` (default)"), "{error}");
    assert_eq!(agent.resolve_delegation_target(&target).unwrap().id, target);
}

#[tokio::test]
async fn a_definition_already_published_to_claude_code_is_adopted_unchanged() {
    let agent = instance("session-claude");
    let published_before = published(&agent).await;
    let adopted = contract_from_published_definition(
        published_before["description"].as_str().unwrap(),
        published_before["parameters"].clone(),
    );
    assert!(!adopted
        .description
        .contains(multi_agent::PROACTIVE_DELEGATION_GUIDANCE));
    assert!(adopted.parameters["properties"][crate::async_tasks::ASYNC_MODE_PARAMETER].is_null());

    let mut changed = instance("session-claude");
    let target = delegation_type(&changed);
    changed
        .subagent_model_overrides
        .set(&target, ["openai/model-alpha", "openai/model-beta"]);
    *changed.pinned_subagent_tool.lock().unwrap() = Some(adopted);
    // Claude Code compares descriptions and schemas exactly.
    assert_eq!(published(&changed).await, published_before);
}
