//! Real CLI + production Mod/bridge, deterministic loopback model. No paid API.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Mutex};

fn tool(name: &str) -> ClaudeCodeToolDefinition {
    ClaudeCodeToolDefinition {
        name: name.into(), description: format!("Local test {name}"),
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
        // Deliberately direct: a Skill may bring a normally-direct tool. The
        // session catalog must nevertheless append it as deferred after step 1.
        defer_loading: false,
    }
}
#[derive(Default)]
struct Host { activated: bool, executed: Vec<String>, root: String }
impl ClaudeCodeHost for Host {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, _: ClaudeCodeAssistantMessage) -> Result<(), String> { Ok(()) }
    fn refresh_tools(&self) -> Pin<Box<dyn Future<Output=Result<Option<Vec<ClaudeCodeToolDefinition>>,String>> + Send + '_>> {
        Box::pin(async {
            let mut tools = vec![tool("read_skill")];
            if self.activated { tools.extend([tool("skill_action"), tool("edit")]); }
            Ok(Some(tools))
        })
    }
    fn execute_tool<'a>(&'a mut self, _: &'a str, name: &'a str, args: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            if name == "edit" {
                let native = native_scope::current().expect("Edit must arrive through the native adapter");
                assert_eq!(native.call.tool, "Edit");
                self.executed.push(name.into());
                return crate::tool::ToolRegistry::with_builtins().execute_with_context(name, &args, crate::tool::ToolExecutionContext {
                    working_dir: Some(self.root.clone()), ..Default::default()
                }).await.into();
            }
            match name {
                "read_skill" => self.activated = true,
                "skill_action" => assert!(self.activated, "unactivated Skill executed"),
                _ => panic!("ToolSearch must execute in Claude, not in Locus: {name}"),
            }
            self.executed.push(name.into());
            ClaudeCodeToolResult { output: format!("LOCAL_OK_{name}"), is_error: false, images: None, context: Vec::new() }
        })
    }
}
fn sse(step: usize) -> String {
    let call = match step {
        0 => Some(("mcp__locus__read_skill", json!({}))),
        1 => Some(("ToolSearch", json!({"query":"select:mcp__locus__skill_action","max_results":1}))),
        2 | 4 => Some(("mcp__locus__skill_action", json!({}))),
        5 => Some(("Edit", json!({"file_path":"native-edit.txt","old_string":"before","new_string":"after"}))),
        _ => None,
    };
    let block = call.as_ref().map(|(name,_)| json!({"type":"tool_use","id":format!("toolu_lazy_{step}"),"name":name,"input":{}}))
        .unwrap_or(json!({"type":"text","text":""}));
    let delta = call.as_ref().map(|(_,args)| json!({"type":"input_json_delta","partial_json":args.to_string()}))
        .unwrap_or(json!({"type":"text_delta","text":"NATIVE_LAZY_OK"}));
    [
        json!({"type":"message_start","message":{"id":format!("msg_lazy_{step}"),"type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":block}),
        json!({"type":"content_block_delta","index":0,"delta":delta}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":if call.is_some(){"tool_use"}else{"end_turn"},"stop_sequence":null},"usage":{"output_tokens":20}}),
        json!({"type":"message_stop"}),
    ].iter().map(|event|format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect()
}
fn without_cache_marks(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => map.iter().filter(|(key,_)|key.as_str()!="cache_control")
            .map(|(key,value)|(key.clone(),without_cache_marks(value))).collect(),
        serde_json::Value::Array(items) => items.iter().map(without_cache_marks).collect(),
        value => value.clone(),
    }
}
fn eager(request: &serde_json::Value) -> Vec<serde_json::Value> {
    request["tools"].as_array().unwrap().iter().filter(|tool|tool["defer_loading"]!=true).cloned().collect()
}

fn assert_deferred_notices_match_catalog(value: &serde_json::Value, eager: &[serde_json::Value]) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("type").and_then(|value|value.as_str()) == Some("text") {
                let text = map.get("text").and_then(|value|value.as_str()).unwrap_or_default();
                let mut lines = text.lines();
                while let Some(line) = lines.next() {
                    if !line.starts_with("The following deferred tools are now available via ToolSearch.") { continue; }
                    for name in lines.by_ref() {
                        if name.is_empty() || !name.chars().all(|ch|ch.is_ascii_alphanumeric() || ch=='_') { break; }
                        assert!(!eager.iter().any(|tool|tool["name"]==name), "eager {name} was falsely announced as deferred");
                    }
                }
            }
            for item in map.values() { assert_deferred_notices_match_catalog(item, eager); }
        }
        serde_json::Value::Array(items) => for item in items { assert_deferred_notices_match_catalog(item, eager); },
        _ => {},
    }
}

#[tokio::test(flavor="multi_thread", worker_threads=2)]
#[ignore="requires Claude Code >=2.1.299; isolated local fixture, run alone with --ignored --test-threads=1"]
async fn real_tui_first_tool_search_and_late_skill_preserve_cache_prefix_on_resume() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("source");
    std::fs::create_dir_all(&config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = requests.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{
                    let (stream,_)=accepted.unwrap(); let seen=seen.clone();
                    connections.spawn(async move {
                        let service=hyper::service::service_fn(move |req:hyper::Request<hyper::body::Incoming>|{
                            let seen=seen.clone();
                            async move {
                                let route=req.uri().path().to_string();
                                let bytes=req.into_body().collect().await.unwrap().to_bytes();
                                let request:serde_json::Value=serde_json::from_slice(&bytes).unwrap_or(json!({}));
                                let main=route=="/v1/messages" && request["system"].to_string().contains("LOCUS_NATIVE_LAZY_PREFIX") && request["tools"].as_array().is_some_and(|tools| !tools.is_empty());
                                let (kind,body)=if main {
                                    let step={let mut requests=seen.lock().unwrap(); let step=requests.len(); requests.push(request); step};
                                    ("text/event-stream",sse(step))
                                } else {("application/json",json!({"id":"msg_aux","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string())};
                                Ok::<_,std::convert::Infallible>(hyper::Response::builder().header("Content-Type",kind).body(Full::new(Bytes::from(body))).unwrap())
                            }
                        });
                        let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;
                    });
                }
                _=connections.join_next(),if !connections.is_empty()=>{}
            }
        }
    });
    let _env = native_integration_tests::EnvGuard::set(&[
        ("LOCUS_CLAUDE_TUI_PROFILE_ROOT",Some(root.path().join("tui").to_string_lossy().into_owned())),
        ("LOCUS_RUNTIME_CONFIG_DIR",Some(config.to_string_lossy().into_owned())),
        ("CLAUDE_CONFIG_DIR",Some(config.to_string_lossy().into_owned())),
        ("ANTHROPIC_BASE_URL",Some(url)),("ANTHROPIC_API_KEY",Some("locus-fixture-key".into())),
        ("ANTHROPIC_AUTH_TOKEN",None),("CLAUDE_CODE_OAUTH_TOKEN",None),("CLAUDECODE",None),
        ("CLAUDE_CODE_USE_BEDROCK",None),("CLAUDE_CODE_USE_VERTEX",None),("CLAUDE_CODE_USE_FOUNDRY",None),
        // The launcher, not the caller's environment, owns the session mode.
        ("ENABLE_TOOL_SEARCH",Some("false".into())),
        ("DISABLE_NONESSENTIAL_TRAFFIC",Some("1".into())),
    ]);
    let id=uuid::Uuid::new_v4().to_string();
    let options=ClaudeCodeCliOptions {
        remote:None,locus_session_id:id.clone(),cwd:root.path().to_string_lossy().into_owned(),
        system_prompt:"LOCUS_NATIVE_LAZY_PREFIX".into(),model:"claude-sonnet-4-6".into(),effort:None,
        resume_session_id:None,
        tools:["read_skill","read","edit","write","bash","grep","list","ask_user_question","todowrite","python"].into_iter().map(tool).collect(),
        native_tool_search:true,
        native_agents:false,
        agents:serde_json::Value::Null,
    };
    std::fs::write(root.path().join("native-edit.txt"), "before").unwrap();
    let mut host=Host { root: root.path().to_string_lossy().into_owned(), ..Default::default() };
    let first=tokio::time::timeout(Duration::from_secs(60),run_turn(options.clone(),json!({"role":"user","content":"Load the Skill and use its tool"}),&mut host)).await.unwrap().unwrap();
    assert_eq!(first.final_text,"NATIVE_LAZY_OK");
    eprintln!("LAZY_BEFORE_RESUME requests={} calls={:?}", requests.lock().unwrap().len(), host.executed);
    // turn.complete is observed before Claude's asynchronous journal writer
    // necessarily flushes. Resume a proven transcript, not a killed write.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let flushed = fork::find_transcript(&ssh::profile_root(&id).unwrap(), first.claude_session_id.as_deref().unwrap())
            .ok().and_then(|path| std::fs::read_to_string(path).ok()).is_some_and(|text| text.contains("NATIVE_LAZY_OK"));
        if flushed { break; }
        assert!(Instant::now() < deadline, "native transcript did not flush");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tui::close_session(&id);
    let resumed=ClaudeCodeCliOptions {resume_session_id:first.claude_session_id, ..options};
    let second=tokio::time::timeout(Duration::from_secs(60),run_turn(resumed,json!({"role":"user","content":"Use the loaded tool again"}),&mut host)).await.unwrap().unwrap();
    tui::close_session(&id);server.abort();
    assert_eq!(second.final_text,"NATIVE_LAZY_OK");
    let requests=requests.lock().unwrap();
    if let Ok(path) = std::env::var("LOCUS_CLAUDE_LAZY_EVIDENCE_DIR") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(Path::new(&path).join("requests.json"), serde_json::to_vec_pretty(&*requests).unwrap()).unwrap();
        std::fs::write(Path::new(&path).join("events.jsonl"), format!("{}\n{}", first.raw_response, second.raw_response)).unwrap();
    }
    eprintln!("LAZY_AFTER_RESUME requests={} calls={:?}", requests.len(), host.executed);
    assert_eq!(host.executed,vec!["read_skill","skill_action","skill_action","edit"]);
    assert_eq!(std::fs::read_to_string(root.path().join("native-edit.txt")).unwrap(), "after");
    assert_eq!(requests.len(),7);
    let prefix=eager(&requests[0]);
    assert!(prefix.iter().any(|tool|tool["name"]=="ToolSearch"),"ToolSearch must be eager before its FIRST use");
    for name in ["Read", "Write", "Edit", "Bash", "Glob", "Grep", "AskUserQuestion", "mcp__locus__python"] {
        assert!(requests[0]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == name), "missing {name}");
    }
    assert!(!requests[0]["tools"].to_string().contains("mcp__locus__skill_action"));
    for (index,request) in requests.iter().enumerate() {
        assert_eq!(eager(request),prefix,"eager prefix changed at step {index}");
        assert_eq!(request["system"],requests[0]["system"],"system prefix changed at step {index}");
        assert!(request["tools"].as_array().unwrap().iter().all(|tool| {
            tool["name"].as_str().and_then(|name|name.strip_prefix("mcp__locus__"))
                .is_none_or(|name|native_tools::native_equivalent_for_locus_tool(name).is_none())
        }), "covered MCP alias leaked at step {index}");
        assert_deferred_notices_match_catalog(&request["messages"], &prefix);
        if index>0 {
            let before=requests[index-1]["messages"].as_array().unwrap();
            let current=request["messages"].as_array().unwrap();
            assert_eq!(without_cache_marks(&json!(&current[..before.len()])),without_cache_marks(&json!(before)),"history prefix changed at step {index}");
        }
    }
    assert!(requests[1]["messages"].to_string().contains("mcp__locus__skill_action"),"missing native discovery delta");
    assert!(requests[2]["messages"].to_string().contains("tool_reference"));
    for request in &requests[2..] {
        assert!(request["tools"].as_array().unwrap().iter().any(|tool|tool["name"]=="mcp__locus__skill_action" && tool["defer_loading"]==true));
    }
}
