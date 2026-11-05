//! Explicit, opt-in probe of the real installed Claude CLI (>= 2.1.312) with
//! the production Mod and bridge: one Agent entry routes a native type to a
//! Claude subagent whose tools run through the host, and a Locus type to the
//! host's Agent route. Unconfigured types and messages to unknown agents are
//! refused. A second probe runs background agents across turns: task
//! notifications, a subagent's message to the main agent, SendMessage resume
//! and the user's stop, locally and through the SSH host. The model endpoint
//! is a local fixture; no model quota is used.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Mutex};

const MAIN_PROMPT: &str = "LOCUS_PROBE_MAIN_PROMPT_4410";
const CHILD_PROMPT: &str = "LOCUS_PROBE_CHILD_PROMPT_4410";
const CHILD_DONE: &str = "CHILD_DONE_4410";
const LOCUS_RESULT: &str = "LOCUS_AGENT_RESULT_4410";
const ENVIRONMENT: &str = "# Environment\nLOCUS_PROBE_ENV_4410";
const SESSION: &str = "native-agents-probe";

/// What the conversation's subagent host observed.
#[derive(Default, Clone, Debug)]
struct AgentLog {
    registered: Vec<(String, String, bool)>,
    starts: Vec<(String, String, String)>,
    texts: Vec<String>,
    executions: Vec<(String, String)>,
    results: Vec<(String, bool)>,
    notices: Vec<ClaudeCodeAgentNotice>,
    messages: Vec<serde_json::Value>,
}

/// The subagent host: Locus tools run in the probe workspace.
struct ProbeAgents {
    root: String,
    registry: crate::tool::ToolRegistry,
    log: Mutex<AgentLog>,
    changed: tokio::sync::Notify,
}

impl ProbeAgents {
    fn log(&self) -> AgentLog { self.log.lock().unwrap().clone() }
    fn record(&self, change: impl FnOnce(&mut AgentLog)) {
        change(&mut self.log.lock().unwrap());
        self.changed.notify_waiters();
    }
    async fn wait(&self, ready: impl Fn(&AgentLog) -> bool, what: &str) {
        let wait = async { loop { let changed = self.changed.notified(); if ready(&self.log()) { return; } changed.await; } };
        if tokio::time::timeout(Duration::from_secs(60), wait).await.is_err() {
            panic!("timed out waiting for {what}: {:?}", self.log());
        }
    }
}

impl ClaudeNativeAgentHost for ProbeAgents {
    fn registered<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move { self.record(|log| log.registered.push((agent.agent_id.clone(), agent.parent_tool_use_id.clone(), agent.background))) })
    }
    fn tool_started(&self, agent: &ClaudeCodeNativeAgent, id: &str, name: &str, _: &serde_json::Value) {
        self.record(|log| log.starts.push((agent.parent_tool_use_id.clone(), id.into(), name.into())));
    }
    fn text(&self, _: &ClaudeCodeNativeAgent, text: &str) {
        self.record(|log| log.texts.push(text.into()));
    }
    fn execute<'a>(&'a self, agent: &'a ClaudeCodeNativeAgent, _: &'a str, name: &'a str, args: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            self.record(|log| log.executions.push((agent.parent_tool_use_id.clone(), name.into())));
            if name == "SendMessage" {
                self.record(|log| log.messages.push(args.clone()));
                return crate::tool::ToolResult { output: json!({"success":true,"message":"Message sent to the main agent."}).to_string(), is_error: false }.into();
            }
            let result = self.registry
                .execute_with_context(name, &args, crate::tool::ToolExecutionContext {
                    working_dir: Some(self.root.clone()),
                    ..Default::default()
                })
                .await;
            assert!(!result.is_error, "Locus {name} failed: {}", result.output);
            result.into()
        })
    }
    fn tool_finished<'a>(&'a self, _: &'a ClaudeCodeNativeAgent, id: &'a str, result: ClaudeCodeToolResult) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move { self.record(|log| log.results.push((id.into(), result.is_error))) })
    }
    fn notification<'a>(&'a self, notice: &'a ClaudeCodeAgentNotice) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async move { self.record(|log| log.notices.push(notice.clone())) })
    }
    fn process_exited<'a>(&'a self, _: Vec<ClaudeCodeNativeAgent>) -> ClaudeCodeUnitFuture<'a> {
        Box::pin(async {})
    }
}

struct AgentHost {
    agents: Arc<ProbeAgents>,
    main_calls: Vec<(String, String)>,
    external_results: Vec<(String, String, bool)>,
    locus_agent_inputs: Vec<serde_json::Value>,
    before_agents: Vec<String>,
}

impl ClaudeCodeHost for AgentHost {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> {
        self.main_calls.extend(message.tool_calls.iter().map(|call| (call.id.clone(), call.name.clone())));
        Ok(())
    }
    fn on_external_tool_result<'a>(
        &'a mut self,
        id: &'a str,
        result: ClaudeCodeToolResult,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        self.external_results.push((id.into(), result.output.clone(), result.is_error));
        Box::pin(async { Ok(()) })
    }
    fn execute_tool<'a>(&'a mut self, _: &'a str, name: &'a str, args: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            // Native Agent types run in Claude; only Locus types reach here.
            assert_eq!(name, "Agent", "unexpected parent tool {name}");
            let scope = native_scope::current().expect("Agent must arrive through the native adapter");
            scope.set_result(json!({
                "status": "completed", "prompt": args["prompt"], "agentId": "locus:probe-task",
                "agentType": args["subagent_type"], "content": [{"type": "text", "text": LOCUS_RESULT}],
                "resolvedModel": "fixture/locus", "totalDurationMs": 1, "totalTokens": 0, "totalToolUseCount": 0,
                "usage": {"input_tokens": 0, "output_tokens": 0, "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0, "server_tool_use": null, "service_tier": null, "cache_creation": null},
                "canContinueAgent": false,
            }));
            self.locus_agent_inputs.push(args);
            crate::tool::ToolResult { output: LOCUS_RESULT.into(), is_error: false }.into()
        })
    }
    fn native_agent_host(&self) -> Option<Arc<dyn ClaudeNativeAgentHost>> {
        Some(self.agents.clone())
    }
    fn before_native_agent<'a>(&'a mut self, id: &'a str) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        self.before_agents.push(id.into());
        Box::pin(async {})
    }
}

fn stream(blocks: &[serde_json::Value], stop: &str) -> String {
    // A unique id per response, as the API sends: Claude merges assistant
    // rows that share a message id into one request message.
    let id = format!("msg_probe_{}", uuid::Uuid::new_v4().simple());
    let mut events = vec![json!({"type":"message_start","message":{"id":id,"type":"message","role":"assistant",
        "model":"claude-sonnet-4-6","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}})];
    for (index, block) in blocks.iter().enumerate() {
        if block["type"] == "tool_use" {
            events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":block["id"],"name":block["name"],"input":{}}}));
            events.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"input_json_delta","partial_json":block["input"].to_string()}}));
        } else {
            events.push(json!({"type":"content_block_start","index":index,"content_block":{"type":"text","text":""}}));
            events.push(json!({"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":block["text"]}}));
        }
        events.push(json!({"type":"content_block_stop","index":index}));
    }
    // Auto mode's server-side review, as the API returns it: every call is
    // evaluated and not flagged. Without it Claude falls back to a local
    // classifier this fixture cannot answer.
    let reviewed: serde_json::Map<String, serde_json::Value> = blocks.iter()
        .filter(|block| block["type"] == "tool_use")
        .map(|block| (block["id"].as_str().unwrap().to_string(), json!({"type":"evaluated","outcome":"not_flagged"})))
        .collect();
    events.push(json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null,
        "safeguard_results":[{"type":"dangerous_tool_use","status":{"type":"available","tool_uses":reviewed}}]},"usage":{"output_tokens":10}}));
    events.push(json!({"type":"message_stop"}));
    events.iter().map(|event| format!("event: {}\ndata: {event}\n\n", event["type"].as_str().unwrap())).collect()
}

fn agent_call(id: &str, subagent_type: &str) -> serde_json::Value {
    json!({"type":"tool_use","id":id,"name":"Agent","input":{"description":"Probe agent","prompt":format!("Run {subagent_type}"),"subagent_type":subagent_type}})
}

/// The main thread delegates four times in a row; the native subagent reads
/// the probe file through the host. The conversation state picks each step.
fn fixture_reply(request: &serde_json::Value, path: &str, secret: &str) -> (String, &'static str) {
    let system = request["system"].to_string();
    let messages = request["messages"].to_string();
    if system.contains(CHILD_PROMPT) {
        // In auto mode Claude requires the report through SubagentHandback;
        // plain final text would not reach the parent.
        let handback = request["tools"].as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "SubagentHandback"));
        return if messages.contains("toolu_child_handback") {
            (stream(&[json!({"type":"text","text":"Reported."})], "end_turn"), "child-after-handback")
        } else if messages.contains(secret) && handback {
            (stream(&[json!({"type":"tool_use","id":"toolu_child_handback","name":"SubagentHandback","input":{"message":CHILD_DONE}})], "tool_use"), "child-handback")
        } else if messages.contains(secret) {
            (stream(&[json!({"type":"text","text":CHILD_DONE})], "end_turn"), "child-done")
        } else {
            (stream(&[json!({"type":"tool_use","id":"toolu_child_read","name":"Read","input":{"file_path":path}})], "tool_use"), "child-read")
        };
    }
    if !system.contains(MAIN_PROMPT) {
        return (stream(&[json!({"type":"text","text":"OK"})], "end_turn"), "auxiliary");
    }
    let (id, reply, step) = if messages.contains("belongs to this conversation") {
        return (stream(&[json!({"type":"text","text":"MAIN_DONE_4410"})], "end_turn"), "main-done");
    } else if messages.contains("Unknown subagent_type") {
        ("toolu_send", vec![json!({"type":"tool_use","id":"toolu_send","name":"SendMessage","input":{"to":"probe","message":"hello","summary":"probe"}})], "main-send")
    } else if messages.contains(LOCUS_RESULT) {
        ("toolu_agent_builtin", vec![agent_call("toolu_agent_builtin", "general-purpose")], "main-builtin")
    } else if messages.contains(CHILD_DONE) {
        ("toolu_agent_locus", vec![agent_call("toolu_agent_locus", "locus:probe-locus")], "main-locus")
    } else {
        ("toolu_agent_native", vec![json!({"type":"text","text":"Delegating."}), agent_call("toolu_agent_native", "locus:probe-native")], "main-native")
    };
    // A step whose call already has a result did not produce the expected
    // outcome; end the turn instead of repeating it until the timeout.
    if messages.contains(id) {
        return (stream(&[json!({"type":"text","text":format!("MAIN_STUCK_AT_{step}")})], "end_turn"), "main-stuck");
    }
    (stream(&reply, "tool_use"), step)
}

type Requests = Arc<Mutex<Vec<(String, serde_json::Value)>>>;
type Reply = Arc<dyn Fn(&serde_json::Value) -> (String, &'static str) + Send + Sync>;

/// The fixture model endpoint, recording each streamed request's step. A
/// `*-hold` step never answers: its agent waits until it is stopped.
async fn fixture_server(reply: Reply) -> (String, Requests, tokio::task::JoinHandle<()>) {
    fixture_server_at("127.0.0.1:0", reply).await
}

/// The fixture on a fixed local address, for a remote Claude that reaches it
/// through an SSH tunnel.
async fn fixture_server_at(address: &str, reply: Reply) -> (String, Requests, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests: Requests = Arc::default();
    let recorded = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let recorded = recorded.clone();
            let reply = reply.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let recorded = recorded.clone();
                    let reply = reply.clone();
                    async move {
                        let route = req.uri().path().to_string();
                        let bytes = req.into_body().collect().await.unwrap().to_bytes();
                        let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(json!({}));
                        let (body, content_type) = if route == "/v1/messages" && request["stream"] == true {
                            let (body, step) = reply(&request);
                            recorded.lock().unwrap().push((step.to_string(), request));
                            if step.ends_with("-hold") {
                                tokio::time::sleep(Duration::from_secs(600)).await;
                            }
                            (body, "text/event-stream")
                        } else if route == "/v1/messages/count_tokens" {
                            (json!({"input_tokens":10}).to_string(), "application/json")
                        } else if route == "/v1/messages" {
                            (json!({"id":"msg_aux","type":"message","role":"assistant","model":"claude-sonnet-4-6",
                                "content":[{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string(), "application/json")
                        } else {
                            ("{}".to_string(), "application/json")
                        };
                        Ok::<_, std::io::Error>(hyper::Response::builder().header("Content-Type", content_type)
                            .body(Full::new(Bytes::from(body))).unwrap())
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket), service).await;
            });
        }
    });
    (url, requests, server)
}

/// Claude and Locus state under the kept probe root, the fixture endpoint,
/// and the inherited variables a native-agent launch must override.
fn probe_env(root: &std::path::Path, config: &std::path::Path, url: String) -> super::native_integration_tests::EnvGuard {
    super::native_integration_tests::EnvGuard::set(&[
        ("LOCUS_CLAUDE_TUI_PROFILE_ROOT", Some(root.join("tui").to_string_lossy().into_owned())),
        // Connection state is written here, never to the user's settings.
        ("LOCUS_RUNTIME_CONFIG_DIR", Some(config.to_string_lossy().into_owned())),
        ("CLAUDE_CONFIG_DIR", Some(config.to_string_lossy().into_owned())),
        ("ANTHROPIC_BASE_URL", Some(url)),
        ("ANTHROPIC_API_KEY", Some("locus-fixture-key".into())),
        ("ANTHROPIC_AUTH_TOKEN", None),
        ("CLAUDE_CODE_OAUTH_TOKEN", None),
        ("CLAUDECODE", None),
        ("CLAUDE_CODE_USE_BEDROCK", None),
        ("CLAUDE_CODE_USE_VERTEX", None),
        ("CLAUDE_CODE_USE_FOUNDRY", None),
        ("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", Some("1".into())),
        ("CLAUDE_AUTO_BACKGROUND_TASKS", Some("1".into())),
        // Would force every Agent into the background; the launch overrides it.
        ("CLAUDE_CODE_FORK_SUBAGENT", Some("1".into())),
        ("DISABLE_NONESSENTIAL_TRAFFIC", Some("1".into())),
        // Claude's debug log (hook decisions, spawn) under the kept profile.
        ("DEBUG", Some("1".into())),
        ("NO_PROXY", Some("127.0.0.1,localhost".into())),
        ("no_proxy", Some("127.0.0.1,localhost".into())),
    ])
}

/// The connection's permission mode, restored to the default afterwards.
struct PermissionGuard;
impl PermissionGuard {
    fn set(mode: super::ssh::PermissionMode) -> Self {
        let mut settings = super::ssh::load_settings().unwrap();
        settings.permission_mode = mode;
        super::ssh::save_settings(&settings).unwrap();
        PermissionGuard
    }
}
impl Drop for PermissionGuard {
    fn drop(&mut self) {
        if let Ok(mut settings) = super::ssh::load_settings() {
            settings.permission_mode = super::ssh::PermissionMode::default();
            let _ = super::ssh::save_settings(&settings);
        }
    }
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.312; run alone with --ignored --test-threads=1"]
async fn real_tui_single_agent_entry_routes_native_and_locus_types() {
    // Locus's default: the subagent reports through SubagentHandback.
    probe(super::ssh::PermissionMode::Auto).await;
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.312; run alone with --ignored --test-threads=1"]
async fn real_tui_single_agent_entry_routes_in_bypass_mode() {
    // No hand-back contract: the subagent's final text is its result.
    probe(super::ssh::PermissionMode::BypassPermissions).await;
}

async fn probe(permission: super::ssh::PermissionMode) {
    // Native-agent conversations always launch in bypass mode: no hand-back.
    let handback = false;
    let _ = permission == super::ssh::PermissionMode::Auto;
    // Kept for inspection: native transcripts and Claude's logs live here.
    let root = tempfile::tempdir().unwrap().keep();
    std::eprintln!("NATIVE_AGENTS_PROBE_ROOT {} mode={}", root.display(), permission.as_str());
    let config = root.as_path().join("profile");
    std::fs::create_dir(&config).unwrap();
    let secret = uuid::Uuid::new_v4().to_string();
    let file = root.as_path().join("source.txt");
    std::fs::write(&file, &secret).unwrap();
    let path = file.to_string_lossy().into_owned();
    let expected = secret.clone();
    let (url, requests, server) = fixture_server(Arc::new(move |request| fixture_reply(request, &path, &expected))).await;
    let _env = probe_env(&root, &config, url);
    let _permission = PermissionGuard::set(permission);
    let mut registry = crate::tool::ToolRegistry::new();
    crate::tool::builtins::register_all(&mut registry);
    let workspace = root.as_path().to_string_lossy().into_owned();
    let agents = Arc::new(ProbeAgents { root: workspace.clone(), registry, log: Mutex::default(), changed: tokio::sync::Notify::new() });
    let mut host = AgentHost { agents: agents.clone(), main_calls: Vec::new(), external_results: Vec::new(), locus_agent_inputs: Vec::new(), before_agents: Vec::new() };
    let options = ClaudeCodeCliOptions {
        remote: None,
        locus_session_id: format!("{SESSION}-{}", permission.as_str()),
        cwd: workspace.clone(),
        system_prompt: MAIN_PROMPT.into(),
        model: "claude-sonnet-4-6".into(),
        effort: None,
        resume_session_id: None,
        tools: vec![],
        native_tool_search: false,
        native_agents: true,
        // A Locus type registers without tools: it is never spawned.
        agents: json!({"version": 1, "builtins": [], "environment": ENVIRONMENT, "definitions": [
            {"name": "probe-native", "backend": "native", "description": "Probe native agent", "prompt": CHILD_PROMPT, "tools": ["Read"], "model": "claude-sonnet-4-6"},
            {"name": "probe-locus", "backend": "locus", "description": "Probe Locus agent", "prompt": "Locus runs this agent type.", "tools": [], "model": "inherit"},
        ]}),
    };
    let result = tokio::time::timeout(Duration::from_secs(90), run_turn(
        options, json!({"role":"user","content":"Delegate the probe work"}), &mut host,
    )).await;
    super::tui::close_session(&format!("{SESSION}-{}", permission.as_str()));
    server.abort();
    let requests = requests.lock().unwrap().clone();
    let steps: Vec<&str> = requests.iter().map(|(step, _)| step.as_str()).collect();
    std::eprintln!("NATIVE_AGENTS_PROBE steps={steps:?} main_calls={:?} host_starts={:?} executions={:?} texts={:?} results={:?} external={:?} locus={:?}",
        host.main_calls, agents.log().starts, agents.log().executions, agents.log().texts, agents.log().results, host.external_results, host.locus_agent_inputs);
    if let Some((_, last)) = requests.last() {
        let messages: Vec<char> = last["messages"].to_string().chars().collect();
        let tail: String = messages[messages.len().saturating_sub(4000)..].iter().collect();
        std::eprintln!("NATIVE_AGENTS_PROBE last_request_messages_tail={tail}");
    }
    let result = result.expect("native agents probe timed out").expect("native agents probe failed");
    assert_eq!(result.final_text, "MAIN_DONE_4410");

    // The catalog offers the published types only; no builtin is configured.
    let main = &requests.iter().find(|(step, _)| step == "main-native").expect("main request").1;
    let tools: Vec<&str> = main["tools"].as_array().unwrap().iter().filter_map(|tool| tool["name"].as_str()).collect();
    let catalog = main["messages"].to_string();
    assert!(tools.contains(&"Agent"), "{tools:?}");
    // Claude lists SendMessage with its deferred tools.
    assert!(tools.contains(&"SendMessage") || catalog.contains("\\nSendMessage\\n"), "{tools:?}");
    assert!(!tools.iter().any(|tool| tool.starts_with("Team")) && !catalog.contains("TeamCreate"), "{tools:?}");
    assert!(catalog.contains("locus:probe-native") && catalog.contains("locus:probe-locus"), "{catalog}");
    assert!(!catalog.contains("general-purpose:") && !catalog.contains("- Explore:"), "{catalog}");

    // The native subagent: its own prompt, the Locus environment, its tool
    // executed by the host, and its result returned to the parent.
    let child = &requests.iter().find(|(step, _)| step == "child-read").expect("native subagent request").1;
    assert!(child["messages"].to_string().contains("LOCUS_PROBE_ENV_4410"), "{}", child["messages"]);
    assert!(!child["system"].to_string().contains(MAIN_PROMPT));
    // In auto mode the subagent reports once through SubagentHandback, and
    // the main thread receives that report as a message in the same run;
    // the hand-back is not a nested Locus call. Otherwise its final text is
    // the Agent result, shown on the card as it streams.
    let report = if handback { "child-handback" } else { "child-done" };
    assert!(steps.contains(&report), "{steps:?}");
    assert!(!steps.contains(&if handback { "child-done" } else { "child-handback" }), "{steps:?}");
    if !handback {
        assert!(agents.log().texts.iter().any(|text| text.contains(CHILD_DONE)), "{:?}", agents.log().texts);
    }
    let after = &requests.iter().find(|(step, _)| step == "main-locus").expect("main request after the native agent").1;
    assert!(after["messages"].to_string().contains(CHILD_DONE), "{}", after["messages"]);
    assert_eq!(agents.log().starts, [("toolu_agent_native".to_string(), "toolu_child_read".to_string(), "read".to_string())]);
    assert_eq!(agents.log().executions, [("toolu_agent_native".to_string(), "read".to_string())]);
    let outcome = |id: &str| host.external_results.iter().find(|(call, _, _)| call == id).cloned();
    let native = outcome("toolu_agent_native").expect("native Agent result");
    assert!(native.1.contains(CHILD_DONE) && !native.2, "{native:?}");
    // The parent's assistant rounds contain only its own calls.
    assert!(host.main_calls.iter().all(|(id, _)| id != "toolu_child_read"), "{:?}", host.main_calls);

    // The Locus type ran in the host with Claude's structured result.
    assert_eq!(host.locus_agent_inputs.len(), 1);
    assert_eq!(host.locus_agent_inputs[0]["subagent_type"], "locus:probe-locus");
    assert!(requests.iter().any(|(step, request)| step == "main-builtin" && request["messages"].to_string().contains(LOCUS_RESULT)));

    // Unconfigured types and messages to unknown agents are refused without
    // running anything.
    let builtin = outcome("toolu_agent_builtin").expect("refused builtin");
    assert!(builtin.2 && builtin.1.contains("Unknown subagent_type"), "{builtin:?}");
    let send = outcome("toolu_send").expect("refused SendMessage");
    assert!(send.2 && send.1.contains("No agent \"probe\" belongs to this conversation"), "{send:?}");
    assert_eq!(steps.iter().filter(|step| step.starts_with("child")).count(), 2, "{steps:?}");
}

const BG_PROMPT: &str = "LOCUS_PROBE_BACKGROUND_SCAN_4410";
const STOP_PROMPT: &str = "LOCUS_PROBE_STOPPABLE_WAIT_4410";
const BG_DONE: &str = "BG_DONE_4410";
const PROGRESS: &str = "BG_PROGRESS_4410";
const RESUME: &str = "RESUME_4410";
const RESUMED_DONE: &str = "RESUMED_DONE_4410";
/// The user's prompt of each main turn; an agent id follows as `[agent:<id>]`.
const TURNS: [&str; 4] = ["START_BACKGROUND_4410", "CONTINUE_AGENT_4410", "START_STOPPABLE_4410", "RESUME_STOPPED_4410"];

/// The latest turn's prompt and the agent id it names. One request message
/// can hold several turns' prompts; the last one is the current turn.
fn current_turn(request: &serde_json::Value) -> (&'static str, String) {
    for message in request["messages"].as_array().into_iter().flatten().rev() {
        if message["role"] != "user" { continue; }
        let text = message["content"].to_string();
        if let Some((at, turn)) = TURNS.iter().filter_map(|turn| text.rfind(*turn).map(|at| (at, *turn))).max() {
            let rest = &text[at..];
            let agent = rest.split("[agent:").nth(1).and_then(|rest| rest.split(']').next()).unwrap_or_default();
            return (turn, agent.to_string());
        }
    }
    ("", String::new())
}

/// Background agents across four turns. The first reads the probe file
/// through the host, messages the main agent and finishes; SendMessage
/// continues it; the second waits in its first request until it is stopped.
fn background_reply(request: &serde_json::Value, path: &str, secret: &str) -> (String, &'static str) {
    let system = request["system"].to_string();
    let messages = request["messages"].to_string();
    let answered = |id: &str| messages.contains(&format!("\"tool_use_id\":\"{id}\""));
    let text = |text: &str| stream(&[json!({"type":"text","text":text})], "end_turn");
    let call = |block: serde_json::Value| stream(&[block], "tool_use");
    if system.contains(CHILD_PROMPT) {
        return if messages.contains(STOP_PROMPT) {
            (text("never sent"), "child-hold")
        } else if messages.contains(RESUME) {
            (text(RESUMED_DONE), "child-resumed")
        } else if answered("toolu_bg_message") {
            (text(BG_DONE), "child-bg-done")
        } else if messages.contains(secret) {
            (call(json!({"type":"tool_use","id":"toolu_bg_message","name":"SendMessage","input":{"to":"main","message":PROGRESS,"summary":"progress"}})), "child-bg-message")
        } else {
            (call(json!({"type":"tool_use","id":"toolu_bg_read","name":"Read","input":{"file_path":path}})), "child-bg-read")
        };
    }
    if !system.contains(MAIN_PROMPT) {
        return (text("OK"), "auxiliary");
    }
    let start = |id: &str, prompt: &str| json!({"type":"tool_use","id":id,"name":"Agent","input":{"description":"Background probe",
        "prompt":prompt,"subagent_type":"locus:probe-native","run_in_background":true}});
    let message = |id: &str, to: &str| json!({"type":"tool_use","id":id,"name":"SendMessage","input":{"to":to,"message":RESUME,"summary":"continue"}});
    match current_turn(request) {
        ("START_BACKGROUND_4410", _) if answered("toolu_agent_bg") => (text("MAIN_BG_STARTED"), "main-bg-started"),
        ("START_BACKGROUND_4410", _) => (call(start("toolu_agent_bg", BG_PROMPT)), "main-bg-start"),
        ("CONTINUE_AGENT_4410", _) if answered("toolu_resume") => (text("MAIN_RESUME_SENT"), "main-resume-sent"),
        ("CONTINUE_AGENT_4410", agent) => (call(message("toolu_resume", &agent)), "main-resume"),
        ("START_STOPPABLE_4410", _) if answered("toolu_agent_stop") => (text("MAIN_STOP_STARTED"), "main-stop-started"),
        ("START_STOPPABLE_4410", _) => (call(start("toolu_agent_stop", STOP_PROMPT)), "main-stop-start"),
        ("RESUME_STOPPED_4410", _) if answered("toolu_resume_stopped") => (text("MAIN_RESUME_REFUSED"), "main-resume-refused"),
        ("RESUME_STOPPED_4410", agent) => (call(message("toolu_resume_stopped", &agent)), "main-resume-stopped"),
        _ => (text("MAIN_UNKNOWN_TURN"), "main-unknown"),
    }
}

async fn probe_turn(options: ClaudeCodeCliOptions, host: &mut AgentHost, text: &str) -> ClaudeCodeTurnResult {
    // An SSH launch may first install Claude Code (the host's setup window).
    let limit = if options.remote.is_some() { locus_remote_core::claude_launch::AUTHORIZED_STARTUP_SECONDS + 90 } else { 90 };
    tokio::time::timeout(Duration::from_secs(limit), run_turn(options, json!({"role":"user","content":text}), host)).await
        .unwrap_or_else(|_| panic!("turn {text:?} timed out"))
        .unwrap_or_else(|error| panic!("turn {text:?} failed: {error}"))
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.312; run alone with --ignored --test-threads=1"]
async fn real_tui_background_agents_notify_resume_and_stop() {
    background_probe(None, "127.0.0.1:0").await;
}

/// The same four turns through the SSH host, with the remote CLI. Set
/// `LOCUS_CLAUDE_SSH_NATIVE_PROBE` to the connection as `SshConfig` JSON and
/// `LOCUS_CLAUDE_SSH_NATIVE_PROBE_FIXTURE` to the local address a tunnel
/// exposes on the host. The remote Claude's settings must point
/// `ANTHROPIC_BASE_URL` at that tunnel with a fake key; no model quota is used.
/// Without `cliPath`, a host whose Claude Code is older than native agents
/// need gets it installed by the first launch.
#[tokio::test]
#[ignore = "requires an SSH host with Claude Code 2.1.312 and a tunnel to the fixture; run alone with --ignored --test-threads=1"]
async fn real_ssh_background_agents_notify_resume_and_stop() {
    let remote: super::ssh::SshConfig = serde_json::from_str(&std::env::var("LOCUS_CLAUDE_SSH_NATIVE_PROBE")
        .expect("LOCUS_CLAUDE_SSH_NATIVE_PROBE: the SSH connection as JSON")).expect("an SSH connection");
    remote.validate().unwrap();
    let fixture = std::env::var("LOCUS_CLAUDE_SSH_NATIVE_PROBE_FIXTURE")
        .expect("LOCUS_CLAUDE_SSH_NATIVE_PROBE_FIXTURE: the local address the tunnel forwards to");
    background_probe(Some(remote), &fixture).await;
}

async fn background_probe(remote: Option<super::ssh::SshConfig>, fixture: &str) {
    let root = tempfile::tempdir().unwrap().keep();
    std::eprintln!("NATIVE_AGENTS_BACKGROUND_ROOT {} remote={}", root.display(), remote.is_some());
    let config = root.as_path().join("profile");
    std::fs::create_dir(&config).unwrap();
    let secret = uuid::Uuid::new_v4().to_string();
    let file = root.as_path().join("source.txt");
    std::fs::write(&file, &secret).unwrap();
    let path = file.to_string_lossy().into_owned();
    let expected = secret.clone();
    let (url, requests, server) = fixture_server_at(fixture, Arc::new(move |request| background_reply(request, &path, &expected))).await;
    let _env = probe_env(&root, &config, url);
    // The connection's default; a native-agent conversation launches in bypass mode regardless.
    let _permission = PermissionGuard::set(super::ssh::PermissionMode::Auto);
    let mut registry = crate::tool::ToolRegistry::new();
    crate::tool::builtins::register_all(&mut registry);
    let workspace = root.as_path().to_string_lossy().into_owned();
    let agents = Arc::new(ProbeAgents { root: workspace.clone(), registry, log: Mutex::default(), changed: tokio::sync::Notify::new() });
    let mut host = AgentHost { agents: agents.clone(), main_calls: Vec::new(), external_results: Vec::new(), locus_agent_inputs: Vec::new(), before_agents: Vec::new() };
    // The host keeps every SSH session's state; a fresh id per run.
    let session = match remote {
        Some(_) => format!("{SESSION}-ssh-{}", uuid::Uuid::new_v4().simple()),
        None => format!("{SESSION}-background"),
    };
    let options = || ClaudeCodeCliOptions {
        remote: remote.clone(),
        locus_session_id: session.clone(),
        cwd: workspace.clone(),
        system_prompt: MAIN_PROMPT.into(),
        model: "claude-sonnet-4-6".into(),
        effort: None,
        resume_session_id: None,
        tools: vec![],
        native_tool_search: false,
        native_agents: true,
        agents: json!({"version": 1, "builtins": [], "environment": ENVIRONMENT, "definitions": [
            {"name": "probe-native", "backend": "native", "description": "Probe native agent", "prompt": CHILD_PROMPT, "tools": ["Read", "SendMessage"], "model": "claude-sonnet-4-6"},
        ]}),
    };
    let report = |agents: &ProbeAgents, requests: &Requests| {
        let steps: Vec<String> = requests.lock().unwrap().iter().map(|(step, _)| step.clone()).collect();
        format!("steps={steps:?} log={:?}", agents.log())
    };

    // 1. The main turn ends once the agent runs in the background. The agent
    // keeps working while the conversation is idle: its read runs through the
    // host, it messages the main agent, and its result arrives as a notice.
    let first = probe_turn(options(), &mut host, "START_BACKGROUND_4410").await;
    assert_eq!(first.final_text, "MAIN_BG_STARTED", "{}", report(&agents, &requests));
    agents.wait(|log| !log.notices.is_empty(), "the background agent's notification").await;
    let log = agents.log();
    let (agent, card, background) = log.registered[0].clone();
    assert_eq!((card.as_str(), background), ("toolu_agent_bg", true), "{log:?}");
    let notice = &log.notices[0];
    assert_eq!((notice.agent_id.as_str(), notice.tool_use_id.as_str(), notice.status.as_str(), notice.superseded),
        (agent.as_str(), "toolu_agent_bg", "completed", false), "{notice:?}");
    assert!(notice.result.as_deref().is_some_and(|result| result.contains(BG_DONE)), "{notice:?}");
    assert_eq!(log.messages, [json!({"to":"main","message":PROGRESS,"summary":"progress"})]);
    assert!(log.executions.contains(&("toolu_agent_bg".to_string(), "read".to_string())), "{log:?}");
    let launched = host.external_results.iter().find(|(id, ..)| id == "toolu_agent_bg").expect("the Agent card's result");
    assert!(!launched.2 && launched.1.contains("Running in the background as agent"), "{launched:?}");
    // Claude's own notification never started a main turn.
    assert!(!requests.lock().unwrap().iter().any(|(_, request)| request["messages"].to_string().contains("<task-notification>")));

    // 2. SendMessage continues the finished agent as a new background run.
    let second = probe_turn(options(), &mut host, &format!("CONTINUE_AGENT_4410 [agent:{agent}]")).await;
    assert_eq!(second.final_text, "MAIN_RESUME_SENT", "{}", report(&agents, &requests));
    agents.wait(|log| log.notices.len() >= 2, "the resumed run's notification").await;
    let log = agents.log();
    assert_eq!(log.registered[1], (agent.clone(), "toolu_agent_bg".to_string(), true), "{log:?}");
    let resumed = &log.notices[1];
    assert_eq!((resumed.tool_use_id.as_str(), resumed.status.as_str(), resumed.superseded), ("toolu_resume", "completed", false), "{resumed:?}");
    assert!(resumed.result.as_deref().is_some_and(|result| result.contains(RESUMED_DONE)), "{resumed:?}");

    // 3. The user stops a running background agent: Locus persists the stop,
    // the Mod's input poll runs Claude's TaskStop, and the agent ends.
    let third = probe_turn(options(), &mut host, "START_STOPPABLE_4410").await;
    assert_eq!(third.final_text, "MAIN_STOP_STARTED", "{}", report(&agents, &requests));
    let held = async {
        while !requests.lock().unwrap().iter().any(|(step, _)| step == "child-hold") {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(60), held).await.expect("the stoppable agent never sent its request");
    agents.wait(|log| log.registered.len() >= 3, "the stoppable agent's registration").await;
    let stoppable = agents.log().registered[2].0.clone();
    super::tui::stop_native_agent(&session, &stoppable).unwrap();
    agents.wait(|log| log.notices.iter().any(|notice| notice.agent_id == stoppable && notice.status == "killed"), "the user's stop").await;
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(
        super::ssh::profile_root(&session).unwrap().join("native_agents.json")).unwrap()).unwrap();
    assert_eq!(saved["stopped"], json!([stoppable]), "{saved}");
    assert_eq!(requests.lock().unwrap().iter().filter(|(step, _)| step == "child-hold").count(), 1);

    // 4. The stopped agent is not continued without the user.
    let fourth = probe_turn(options(), &mut host, &format!("RESUME_STOPPED_4410 [agent:{stoppable}]")).await;
    assert_eq!(fourth.final_text, "MAIN_RESUME_REFUSED", "{}", report(&agents, &requests));
    let refused = host.external_results.iter().find(|(id, ..)| id == "toolu_resume_stopped").expect("the refused SendMessage");
    assert!(refused.2 && refused.1.contains("The user stopped agent"), "{refused:?}");
    assert_eq!(agents.log().registered.len(), 3, "{:?}", agents.log());

    // 5. Over SSH, the conversation's archive carries both agents'
    // transcripts and metadata beside the main transcript.
    if let Some(remote) = &remote {
        let native_id = fourth.claude_session_id.clone().expect("the Claude session id");
        let connection = super::connection::NativeConnection::Ssh(remote.clone());
        let delta = super::archive::download(&session, &connection, &native_id, &Default::default(), None).await.expect("the archive");
        let files = delta.apply(&Default::default(), &native_id).expect("the archived files");
        for id in [&agent, &stoppable] {
            for suffix in ["jsonl", "meta.json"] {
                assert!(files.contains_key(&format!("subagents/agent-{id}.{suffix}")), "{:?}", files.keys().collect::<Vec<_>>());
            }
        }
        let transcript = String::from_utf8_lossy(&files[&format!("subagents/agent-{agent}.jsonl")]).into_owned();
        assert!(transcript.contains(BG_DONE) && transcript.contains(RESUMED_DONE), "{transcript}");
    }
    super::tui::close_session(&session);
    server.abort();
}
