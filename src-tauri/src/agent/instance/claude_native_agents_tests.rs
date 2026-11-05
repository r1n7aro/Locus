use super::*;
use serde_json::json;

fn definition(id: &str, tools: &[&str]) -> AgentDef {
    AgentDef {
        id: id.into(),
        name: id.into(),
        description: format!("{id} agent."),
        project_types: Vec::new(),
        system_prompt: format!("You are {id}."),
        env_template: String::new(),
        tools: tools.iter().map(|tool| tool.to_string()).collect(),
        sub_agents: Vec::new(),
        delegatable: true,
        default_project_types: Vec::new(),
        default: false,
        default_effort: None,
        model_recommendation: None,
        tool_description_overrides: HashMap::new(),
        source: "test".into(),
    }
}

/// A Claude Code parent whose `explorer` type may run on a Claude Code model
/// (native) or on another provider (Locus).
fn parent(definitions: &tempfile::TempDir) -> AgentInstance {
    parent_in(definitions, "")
}

fn parent_in(definitions: &tempfile::TempDir, working_dir: &str) -> AgentInstance {
    let agent_dir = definitions.path().join("agents");
    let explorer = definition("explorer", &["read", "grep", "bash", "subagent", "todowrite"]);
    std::fs::create_dir_all(agent_dir.join("explorer")).unwrap();
    std::fs::write(agent_dir.join("explorer/config.json"), serde_json::to_vec(&explorer).unwrap()).unwrap();
    std::fs::write(agent_dir.join("explorer/soul.md"), "You are explorer.").unwrap();
    std::fs::write(agent_dir.join("explorer/env.md"), "# Environment\nWorking directory: <working_dir>\n{{#unity}}\nUnity Editor: <unity_version>\n{{/unity}}").unwrap();
    let (_, cancel_rx) = tokio::sync::watch::channel(false);
    let mut parent = AgentInstance::new(
        Arc::new(definition("main", &["read", "grep", "bash", "write", "subagent", "todowrite"])),
        "session",
        LlmBackend::ClaudeCodeCli,
        false,
        Arc::new(AgentDefRegistry::load(Some(&agent_dir), None)),
        Arc::new(ToolRegistry::with_builtins()),
        working_dir.to_string(),
        RawContextStore::default(),
        None,
        "claude_code/opus".into(),
        Some("high".into()),
        Arc::new(None),
        Arc::new(None),
        KnowledgeAccessMode::Full,
        None,
        Default::default(),
        cancel_rx,
    );
    parent.multi_agent_enabled = true;
    parent
        .subagent_model_overrides
        .set("explorer", ["claude_code/sonnet", "custom/child/luna", "custom/gone/model"]);
    parent
}

fn resolve(model: String) -> std::future::Ready<Result<LlmBackend, String>> {
    std::future::ready(if model.starts_with("claude_code/") {
        Ok(LlmBackend::ClaudeCodeCli)
    } else if model.starts_with("custom/child/") {
        Ok(LlmBackend::OpenRouter { api_key: String::new(), base_url: None })
    } else {
        Err("provider removed".into())
    })
}

async fn directory(parent: &AgentInstance) -> DelegationDirectory {
    parent
        .build_claude_delegation_directory(true, vec!["Explore".into()], &[], resolve)
        .await
}

#[tokio::test]
async fn native_definitions_carry_their_rules_and_rendered_environment() {
    let definitions = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let rules = workspace.path().join("Locus/agent/explorer/rule");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(rules.join("citations.md"), "# Citations\nCite every file you read.").unwrap();
    let working_dir = workspace.path().to_string_lossy().into_owned();
    let parent = parent_in(&definitions, &working_dir);
    let directory = directory(&parent).await;
    let native = directory.entries.iter().find(|entry| entry.backend == DelegationBackend::ClaudeNative).unwrap();
    // The Claude Code order: base prompt, rules, environment, then who runs the tools.
    let rule = native.prompt.find("Cite every file you read.").expect("the explorer rule");
    let environment = native.prompt.find("# Environment").expect("the rendered environment");
    assert!(native.prompt.starts_with("You are explorer.") && rule < environment, "{}", native.prompt);
    assert!(native.prompt.contains("## Agent Rules"), "{}", native.prompt);
    assert!(native.prompt.contains(&AgentInstance::display_working_dir_value(&working_dir)), "{}", native.prompt);
    // Unity blocks are dropped outside a Unity project; the parent's own
    // delegation guidance never reaches a subagent that cannot delegate.
    assert!(!native.prompt.contains("{{#unity}}") && !native.prompt.contains("Unity Editor"), "{}", native.prompt);
    assert!(!native.prompt.contains("<multi_agent_mode>"), "{}", native.prompt);
    assert!(native.prompt.ends_with(NATIVE_TOOLS_NOTE), "{}", native.prompt);
    // The main agent's bridged path rules precede the environment, as in its prompt.
    let tool_rules = native.prompt.find("## Locus tool environment").expect("the bridged tool rules");
    assert!(rule < tool_rules && tool_rules < environment, "{}", native.prompt);
    assert!(native.prompt.contains("Do not translate them to Linux or Git Bash `/f/...` paths"), "{}", native.prompt);
    // A Locus-routed type keeps no prompt: Locus renders its own definition.
    assert!(directory.entries.iter().filter(|entry| entry.backend == DelegationBackend::Locus).all(|entry| entry.prompt.is_empty()));
}

#[tokio::test]
async fn each_type_and_model_is_bound_to_its_resolved_backend() {
    let definitions = tempfile::tempdir().unwrap();
    let parent = parent(&definitions);
    let directory = directory(&parent).await;
    let explorer: Vec<_> = directory.entries.iter().filter(|entry| entry.agent_type == "explorer").collect();
    // The unavailable model is skipped, never replaced by another backend.
    assert_eq!(explorer.len(), 2, "{explorer:?}");

    let native = explorer.iter().find(|entry| entry.model == "claude_code/sonnet").unwrap();
    assert_eq!(native.backend, DelegationBackend::ClaudeNative);
    assert_eq!(native.name, "explorer-sonnet");
    assert_eq!(native.native_model, "sonnet");
    assert!(native.prompt.starts_with("You are explorer."));
    // Delegation and planning tools never reach a native subagent; like a
    // Locus subagent under Multi-Agent, it can still message the main agent.
    assert_eq!(native.tools, ["Read", "Grep", "Bash", "SendMessage"]);

    let locus = explorer.iter().find(|entry| entry.model == "custom/child/luna").unwrap();
    assert_eq!(locus.backend, DelegationBackend::Locus);
    assert!(locus.prompt.is_empty() && locus.native_model.is_empty());
    // Shown in the catalog only; Locus runs its own definition.
    assert_eq!(locus.tools, native.tools);
    assert!(locus.description.contains("Locus subagent"));

    let payload = directory.mod_payload("ENV");
    assert_eq!(payload["environment"], "ENV");
    assert_eq!(payload["version"], DIRECTORY_VERSION);
    assert_eq!(payload["builtins"], json!(["Explore"]));
    let registered = payload["definitions"].as_array().unwrap();
    let native_definition = registered.iter().find(|item| item["name"] == "explorer-sonnet").unwrap();
    assert_eq!(native_definition["backend"], "native");
    assert_eq!(native_definition["model"], "sonnet");
    let locus_definition = registered.iter().find(|item| item["name"] == locus.name).unwrap();
    assert_eq!(locus_definition["backend"], "locus");
    assert_eq!(locus_definition["prompt"], LOCUS_ROUTED_PROMPT);
    assert_eq!(locus_definition["model"], "inherit");
    assert_eq!(locus_definition["tools"], json!(["Read", "Grep", "Bash", "SendMessage"]));

    assert_eq!(directory.entry_for("locus:explorer-sonnet"), Some(*native));
    assert_eq!(directory.entry_for("explorer-sonnet"), Some(*native));
    assert!(directory.entry_for("Explore").is_none());
}

#[tokio::test]
async fn excluded_locus_types_are_left_out_of_the_directory() {
    let definitions = tempfile::tempdir().unwrap();
    let reviewer = definitions.path().join("agents/reviewer");
    std::fs::create_dir_all(&reviewer).unwrap();
    std::fs::write(reviewer.join("config.json"), serde_json::to_vec(&definition("reviewer", &["read"])).unwrap()).unwrap();
    std::fs::write(reviewer.join("soul.md"), "You are reviewer.").unwrap();
    let parent = parent(&definitions);
    let excluded = ["explorer".to_string(), "removed-agent".to_string()];
    let directory = parent
        .build_claude_delegation_directory(true, vec!["Explore".into()], &excluded, resolve)
        .await;
    // Every model of an excluded type is gone, whichever backend it runs on;
    // other types and the builtins stay, and unknown ids are harmless.
    let types: Vec<_> = directory.entries.iter().map(|entry| entry.agent_type.as_str()).collect();
    assert_eq!(types, ["reviewer"], "{:?}", directory.entries);
    assert_eq!(directory.builtins, ["Explore"]);
    assert!(directory.entry_for("locus:explorer-sonnet").is_none());
}

#[tokio::test]
async fn locus_routed_calls_use_the_published_model_and_report_current_choices() {
    let definitions = tempfile::tempdir().unwrap();
    let mut parent = parent(&definitions);
    let directory = directory(&parent).await;
    let locus = directory
        .entries
        .iter()
        .find(|entry| entry.backend == DelegationBackend::Locus)
        .unwrap()
        .clone();
    let subagent_type = format!("locus:{}", locus.name);

    let (entry, args) = parent
        .claude_locus_agent_call(&directory, &json!({"subagent_type": subagent_type, "description": "Scan", "prompt": "Find it"}))
        .unwrap();
    assert_eq!(entry, locus);
    assert_eq!(args, json!({"subagent_type": "explorer", "description": "Scan", "prompt": "Find it", "model": "custom/child/luna"}));

    let native_type = json!({"subagent_type": "locus:explorer-sonnet", "prompt": "Find it"});
    let error = parent.claude_locus_agent_call(&directory, &native_type).unwrap_err();
    assert!(error.contains("does not run in Locus") && error.contains("`locus:explorer-sonnet`") && error.contains("`Explore`"), "{error}");

    for (field, value) in [("isolation", json!("worktree")), ("cwd", json!("/tmp")), ("model", json!("opus"))] {
        let mut input = json!({"subagent_type": subagent_type, "prompt": "Find it"});
        input[field] = value;
        assert!(parent.claude_locus_agent_call(&directory, &input).is_err(), "{field}");
    }
    // Background runs are allowed; the host starts them as notifying tasks.
    let background = json!({"subagent_type": subagent_type, "prompt": "Find it", "run_in_background": true});
    assert!(parent.claude_locus_agent_call(&directory, &background).is_ok());
    assert!(parent.claude_locus_agent_call(&directory, &json!({"subagent_type": subagent_type, "prompt": " "})).is_err());

    // A later settings change removes the model: the call fails, it does not
    // fall back to the parent's or another allowed model.
    parent.subagent_model_overrides.set("explorer", ["claude_code/sonnet"]);
    let error = parent
        .claude_locus_agent_call(&directory, &json!({"subagent_type": subagent_type, "prompt": "Find it"}))
        .unwrap_err();
    assert!(error.contains("no longer available"), "{error}");
    assert!(!error.contains(&format!("`{subagent_type}`")), "{error}");
}

#[test]
fn native_agent_tools_follow_parent_authority_and_exclude_delegation() {
    let locus_tools: Vec<String> = ["read", "grep", "subagent", "todowrite", "bash", "unity_execute", "list", "web_fetch", "tool_call", "send_message"]
        .iter()
        .map(|tool| tool.to_string())
        .collect();
    let allowed: HashSet<String> = ["read", "grep", "bash", "unity_execute", "subagent", "todowrite", "list", "tool_call"]
        .iter()
        .map(|tool| tool.to_string())
        .collect();
    // A subagent may message the main agent although the parent's catalog
    // offers no send_message in a native conversation.
    assert_eq!(
        native_agent_tools(&locus_tools, &allowed, false),
        ["Read", "Grep", "Bash", "mcp__locus__unity_execute", "Glob", "SendMessage"]
    );
    assert!(!native_agent_tools(&locus_tools[..9], &allowed, false).contains(&"SendMessage".to_string()));
    assert_eq!(
        native_agent_tools(&locus_tools, &allowed, true).last().map(String::as_str),
        Some("ToolSearch")
    );
    for tool in ["subagent", "send_message", "todowrite", "tool_call", "tool_load", "exit_plan_mode"] {
        assert!(native_agent_tool_refusal(tool).is_some(), "{tool}");
    }
    for tool in ["read", "bash", "ask_user_question", "unity_execute"] {
        assert!(native_agent_tool_refusal(tool).is_none(), "{tool}");
    }
}

#[test]
fn locus_agent_results_use_the_native_agent_result_shape() {
    let entry = DelegationEntry {
        name: "explorer-luna".into(),
        backend: DelegationBackend::Locus,
        agent_type: "explorer".into(),
        model: "custom/child/luna".into(),
        description: String::new(),
        prompt: String::new(),
        tools: Vec::new(),
        native_model: String::new(),
    };
    let input = json!({"subagent_type": "locus:explorer-luna", "prompt": "Find it"});
    let result = locus_agent_result(&entry, &input, "Task id: t-1\nFound it.", 42, 3, "toolu_1");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["agentId"], "locus:t-1");
    assert_eq!(result["agentType"], "locus:explorer-luna");
    assert_eq!(result["content"], json!([{"type": "text", "text": "Found it."}]));
    assert_eq!(result["resolvedModel"], "custom/child/luna");
    assert_eq!(result["totalDurationMs"], 42);
    assert_eq!(result["totalToolUseCount"], 3);
    // SendMessage continues a Locus subagent through its task id.
    assert_eq!(result["canContinueAgent"], true);
    // Without a task id the call id names the agent; agentId is never null.
    let untracked = locus_agent_result(&entry, &input, "Found it.", 1, 0, "toolu_1");
    assert_eq!(untracked["agentId"], "locus:toolu_1");
    assert_eq!(untracked["canContinueAgent"], false);

    // A background launch is completed-shaped, so Claude tracks no task of
    // its own; the text names the id for SendMessage and TaskStop.
    let launched = locus_background_agent_result(&entry, &input, "t-2");
    assert_eq!(launched["status"], "completed");
    assert_eq!(launched["agentId"], "locus:t-2");
    let text = launched["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("locus:t-2") && text.contains("delivered to you automatically"), "{text}");
    assert_eq!(launched["canContinueAgent"], true);

    assert!(unsupported_agent_option(&json!({"run_in_background": true, "isolation": null, "cwd": ""})).is_none());
}

#[test]
fn builtin_subagent_environment_carries_the_bridged_tool_rules() {
    let environment = claude_subagent_environment("F:/Project");
    let rules = environment.find("## Locus tool environment").expect("the bridged tool rules");
    let section = environment.find("# Environment\n").expect("the environment section");
    assert!(rules < section, "{environment}");
    for rule in ["Do not prepend `cd` to that same directory", "Do not translate them to Linux or Git Bash `/f/...` paths"] {
        assert!(environment.contains(rule), "{rule}: {environment}");
    }
    assert!(environment.contains("Working directory: F:/Project"), "{environment}");
}
