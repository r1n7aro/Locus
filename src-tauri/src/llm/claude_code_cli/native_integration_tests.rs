//! Explicit, opt-in probe of the real installed Claude CLI + production Mod /
//! Rust bridge + Locus's actual read tool. The model endpoint is a local fixture.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Mutex};

pub(super) struct EnvGuard(Vec<(String, Option<OsString>)>);
impl EnvGuard {
    pub(super) fn set(values: &[(&str, Option<String>)]) -> Self {
        let previous = values
            .iter()
            .map(|(key, value)| {
                let old = std::env::var_os(key);
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                };
                (key.to_string(), old)
            })
            .collect();
        Self(previous)
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

struct ReadHost {
    root: String,
    seen: bool,
    executed: usize,
    exit_during_tool: bool,
    native_id: Option<String>,
    registry: crate::tool::ToolRegistry,
}
impl ClaudeCodeHost for ReadHost {
    fn on_session_started(&mut self,id:&str)->Result<(),String> { self.native_id=Some(id.into()); Ok(()) }
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(), String> {
        if let Some(call) = message.tool_calls.first() {
            assert_eq!(call.id, "toolu_native_read");
            assert_eq!(call.name, "read");
            self.seen = true;
        }
        Ok(())
    }
    fn execute_tool<'a>(
        &'a mut self,
        id: &'a str,
        name: &'a str,
        args: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            if self.exit_during_tool {
                super::tui::close_session("native-probe");
                // A pending GUI question must not hide a dead CLI forever.
                std::future::pending::<()>().await;
            }
            assert!(
                self.seen,
                "assistant must be persisted before native execution"
            );
            assert_eq!(id, "toolu_native_read");
            assert_eq!(name, "read");
            self.executed += 1;
            let result = self.registry
                .execute_with_context(
                    name,
                    &args,
                    crate::tool::ToolExecutionContext {
                        working_dir: Some(self.root.clone()),
                        ..Default::default()
                    },
                )
                .await;
            assert!(!result.is_error, "Locus read failed: {}", result.output);
            result.into()
        })
    }
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.299; run alone with --ignored --test-threads=1"]
async fn real_tui_ephemeral_native_read_uses_locus_tool() {
    read_probe(false, false, false).await;
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.299; run alone with --ignored --test-threads=1"]
async fn real_tui_persistent_read_uses_locus_tool() {
    read_probe(true, false, false).await;
}

#[tokio::test]
#[ignore = "requires Claude Code 2.1.299; run alone with --ignored --test-threads=1"]
async fn real_tui_exit_interrupts_pending_local_tool() {
    read_probe(true, true, false).await;
}

#[tokio::test]
#[ignore = "requires installed Claude CLI; isolated local model fixture, run alone"]
async fn real_tui_crashed_tool_resumes_same_native_session_without_retry() {
    read_probe(true,true,true).await;
}

async fn read_probe(persistent: bool, exit_during_tool: bool, recover: bool) {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("profile");
    std::fs::create_dir(&config).unwrap();
    let secret = uuid::Uuid::new_v4().to_string();
    let file = root.path().join("source.txt");
    std::fs::write(&file, &secret).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let evidence = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let seen = evidence.clone();
    let expected = secret.clone();
    let path = file.to_string_lossy().into_owned();
    let offline = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let offline_server = offline.clone();
    let disconnected = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let disconnected_server = disconnected.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                connection=listener.accept()=>{
                    let (stream,_)=connection.unwrap();
                    if offline_server.load(std::sync::atomic::Ordering::SeqCst){
                        disconnected_server.fetch_add(1,std::sync::atomic::Ordering::SeqCst);drop(stream);continue;
                    }
                    let seen=seen.clone();let expected=expected.clone();let path=path.clone();
                    let offline=offline_server.clone();let disconnected=disconnected_server.clone();
                    connections.spawn(async move {
                        let service=hyper::service::service_fn(move |req:hyper::Request<hyper::body::Incoming>|{
                            let seen=seen.clone();let expected=expected.clone();let path=path.clone();
                            let offline=offline.clone();let disconnected=disconnected.clone();
                            async move {
                                // Also interrupt reused keep-alive connections.
                                if offline.load(std::sync::atomic::Ordering::SeqCst){
                                    disconnected.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                                    return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset,"injected network outage"));
                                }
                                let route=req.uri().path().to_string();
                                let bytes=req.into_body().collect().await.unwrap().to_bytes();
                                let request:serde_json::Value=serde_json::from_slice(&bytes).unwrap_or(json!({}));
                                let mut body="{}".to_string();let mut content_type="application/json";
                                if route=="/v1/messages" && request["tools"].as_array().is_some_and(|tools|tools.iter().any(|t|t["name"]=="Read")) {
                                    assert!(request["system"].to_string().contains("LOCUS_SYSTEM_NATIVE_READ"));
                                    assert!(request["tools"].as_array().unwrap().iter().any(|t|t["name"]=="Read"));
                                    let text=request["messages"].to_string();
                                    let is_done=text.contains(&expected) || (recover && text.contains("RECOVERY_CONTINUE"));
                                    if seen.lock().unwrap().len()>0 {assert!(is_done,"local read result missing from next request: {text}");}
                                    let tools = request["tools"].as_array().unwrap();
                                    assert!(!tools.iter().any(|t|t["name"]=="mcp__locus__read" || t["name"]=="mcp__locus__locus_task_output"));
                                    if !persistent { assert!(tools.iter().any(|t|t["name"]=="mcp__locus__python")); }
                                    seen.lock().unwrap().push(request);
                                    let block=if is_done {json!({"type":"text","text":""})}
                                        else {json!({"type":"tool_use","id":"toolu_native_read","name":"Read","input":{}})};
                                    let delta=if is_done {json!({"type":"text_delta","text":"NATIVE_READ_OK"})}
                                        else {json!({"type":"input_json_delta","partial_json":json!({"file_path":path}).to_string()})};
                                    let events=vec![
                                        json!({"type":"message_start","message":{"id":"msg_native","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":10,"output_tokens":0}}}),
                                        json!({"type":"content_block_start","index":0,"content_block":block}),
                                        json!({"type":"content_block_delta","index":0,"delta":delta}),
                                        json!({"type":"content_block_stop","index":0}),
                                        json!({"type":"message_delta","delta":{"stop_reason":if is_done {"end_turn"} else {"tool_use"},"stop_sequence":null},"usage":{"output_tokens":10}}),
                                        json!({"type":"message_stop"}),
                                    ];
                                    body=events.iter().map(|event|format!("event: {}\ndata: {}\n\n",event["type"].as_str().unwrap(),event)).collect();
                                    content_type="text/event-stream";
                                } else if route=="/v1/messages" {
                                    body=json!({"id":"msg_aux","type":"message","role":"assistant","model":"claude-sonnet-4-6",
                                        "content":[{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string();
                                }
                                Ok::<_,std::io::Error>(hyper::Response::builder().header("Content-Type",content_type)
                                    .body(Full::new(Bytes::from(body))).unwrap())
                            }
                        });
                        let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).await;
                    });
                }
                _=connections.join_next(),if !connections.is_empty()=>{}
            }
        }
    });
    let _env = EnvGuard::set(&[
        (
            "LOCUS_CLAUDE_TUI_PROFILE_ROOT",
            Some(root.path().join("tui").to_string_lossy().into_owned()),
        ),
        (
            "CLAUDE_CONFIG_DIR",
            Some(config.to_string_lossy().into_owned()),
        ),
        ("ANTHROPIC_BASE_URL", Some(url)),
        ("ANTHROPIC_API_KEY", Some("locus-fixture-key".into())),
        ("ANTHROPIC_AUTH_TOKEN", None),
        ("CLAUDE_CODE_OAUTH_TOKEN", None),
        ("CLAUDECODE", None),
        ("CLAUDE_CODE_USE_BEDROCK", None),
        ("CLAUDE_CODE_USE_VERTEX", None),
        ("CLAUDE_CODE_USE_FOUNDRY", None),
        ("DISABLE_NONESSENTIAL_TRAFFIC", Some("1".into())),
        ("NO_PROXY", Some("127.0.0.1,localhost".into())),
        ("no_proxy", Some("127.0.0.1,localhost".into())),
    ]);
    let mut registry = crate::tool::ToolRegistry::new();
    crate::tool::builtins::register_all(&mut registry);
    let mut host = ReadHost {
        root: root.path().to_string_lossy().into_owned(),
        seen: false,
        executed: 0,
        exit_during_tool,
        native_id: None,
        registry,
    };
    let options = ClaudeCodeCliOptions {
        remote: None,
        locus_session_id: "native-probe".into(),
        cwd: host.root.clone(),
        system_prompt: "LOCUS_SYSTEM_NATIVE_READ".into(),
        model: "claude-sonnet-4-6".into(),
        effort: None,
        resume_session_id: None,
        tools: vec![],
        native_tool_search: false,
        native_agents: false,
        agents: serde_json::Value::Null,
    };
    if exit_during_tool {
        let error = tokio::time::timeout(Duration::from_secs(20), run_turn(
            options.clone(), json!({"role":"user","content":"Read the probe file"}), &mut host,
        )).await.expect("dead CLI left a local tool waiting").unwrap_err();
        assert!(error.contains("exited"), "{error}");
        assert_eq!(host.executed, 0);
        if recover {
            let native=host.native_id.clone().expect("identity must survive interrupted first turn");
            let journal=super::native_journal::Journal::new(super::ssh::profile_root("native-probe").unwrap().join("calls"));
            assert_eq!(journal.recover_dispatch("toolu_native_read").unwrap()["status"],"failed");
            host.exit_during_tool=false;
            let mut resumed=options.clone();resumed.resume_session_id=Some(native.clone());
            let result=tokio::time::timeout(Duration::from_secs(60),run_turn(resumed,
                json!({"role":"user","content":format!("RECOVERY_CONTINUE. {}",super::native_journal::RECOVERY_FAILURE)}),&mut host)).await.unwrap().unwrap();
            assert_eq!(result.claude_session_id.as_deref(),Some(native.as_str()));
            assert_eq!(result.final_text,"NATIVE_READ_OK");
            assert_eq!(host.executed,0,"recovery must not invoke the interrupted tool again");
            super::tui::close_session("native-probe");
            let requests=evidence.lock().unwrap();
            assert!(requests.len()>=2);
            // The real CLI owns protocol repair. Check its actual outbound
            // context, without manufacturing tool_result rows in Locus.
            let messages=requests.last().unwrap()["messages"].as_array().unwrap();
            for (index,message) in messages.iter().enumerate() {
                for block in message["content"].as_array().into_iter().flatten().filter(|b|b["type"]=="tool_use") {
                    assert!(messages.get(index+1).into_iter().flat_map(|m|m["content"].as_array()).flatten()
                        .any(|result|result["type"]=="tool_result" && result["tool_use_id"]==block["id"]),"native resume left an unpaired tool call");
                }
            }
        }
        server.abort();
        return;
    }
    if persistent {
        let mut resume_id = None;
        for iteration in 0..3 {
            if iteration == 1 {
                offline.store(true, std::sync::atomic::Ordering::SeqCst);
                let offline = offline.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    offline.store(false, std::sync::atomic::Ordering::SeqCst);
                });
            }
            if iteration == 2 {
                super::tui::close_session("native-probe");
            }
            let mut options = options.clone();
            options.resume_session_id = resume_id.clone();
            let result = tokio::time::timeout(
                Duration::from_secs(60),
                run_turn(
                    options.clone(),
                    json!({"role":"user","content":"Read the probe file"}),
                    &mut host,
                ),
            )
            .await;
            if !matches!(&result, Ok(Ok(_))) {
                super::tui::close_session("native-probe");
            }
            let result = result.expect("TUI timeout").expect("TUI bridge");
            assert_eq!(result.final_text, "NATIVE_READ_OK");
            assert_eq!(result.upstream_model.as_deref(), Some("claude-sonnet-4-6"));
            if let Some(id) = resume_id.as_ref() {
                assert_eq!(result.claude_session_id.as_ref(), Some(id));
            }
            resume_id = result.claude_session_id;
            eprintln!(
                "TUI_PROBE turn={} native_executions={} same_session={}",
                iteration + 1,
                host.executed,
                resume_id.as_deref().unwrap_or("missing")
            );
        }
        super::tui::close_session("native-probe");
        server.abort();
        assert_eq!(host.executed, 1);
        assert!(evidence.lock().unwrap().len() >= 4);
        assert!(disconnected.load(std::sync::atomic::Ordering::SeqCst) > 0);
        return;
    }
    // Verify the real CLI catalog keeps native Read and the renamed Python,
    // even if a caller supplies a duplicate Locus read definition.
    let tools = ["read", "python"].into_iter().map(|name| {
        let (description, input_schema) = host.registry.tool_description(name).unwrap();
        ClaudeCodeToolDefinition { name: name.into(), description, input_schema, defer_loading: false }
    }).collect();
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        run_ephemeral_turn(
            ClaudeCodeCliOptions {
                remote: None,
                locus_session_id: "native-probe".into(),
                cwd: host.root.clone(),
                system_prompt: "LOCUS_SYSTEM_NATIVE_READ".into(),
                model: "claude-sonnet-4-6".into(),
                effort: None,
                resume_session_id: None,
                tools,
                native_tool_search: false,
                native_agents: false,
                agents: serde_json::Value::Null,
            },
            json!({"role":"user","content":"Read the probe file"}),
            &mut host,
        ),
    )
    .await;
    server.abort();
    let result = result
        .expect("real CLI timed out")
        .expect("real CLI native bridge");
    assert_eq!(result.final_text, "NATIVE_READ_OK");
    assert_eq!(host.executed, 1);
    // TUI may issue an extra inference step; the invariant is one local read
    // and its real output in every subsequent captured request.
    assert!(evidence.lock().unwrap().len() >= 2);
}
