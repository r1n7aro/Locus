//! Real interactive Claude + a local deterministic model endpoint. This probes
//! native input/media/fork/queue semantics without Locus issuing model requests.
use super::*;
use base64::Engine;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper_util::rt::TokioIo;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

const COMPACT_INSTRUCTIONS_FIXTURE: &str = "LOCUS_COMPACT_INSTRUCTIONS_5521";
const COMPACT_TRANSCRIPT_FIXTURE: &str = "C:/locus-fixture/compact-transcripts/claude/s/native.jsonl";
const COMPACT_CARRY_FIXTURE: &str = "LOCUS_CARRY_FORWARD_7741";

#[derive(Default)]
struct Host {
    checkpoints: Vec<serde_json::Value>,
    compactions: Vec<serde_json::Value>,
    interrupted: Vec<String>,
    blocked_tool: Option<Arc<AtomicBool>>,
}
impl ClaudeCodeHost for Host {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, _: ClaudeCodeAssistantMessage) -> Result<(), String> {
        Ok(())
    }
    fn on_interrupted_message(&mut self, message: ClaudeCodeAssistantMessage) -> Result<(),String> {
        self.interrupted.push(message.text); Ok(())
    }
    fn on_checkpoint(&mut self, event: &serde_json::Value) -> Result<(), String> {
        self.checkpoints.push(event.clone());
        Ok(())
    }
    fn on_compaction(&mut self, event: &serde_json::Value) -> Result<(), String> {
        self.compactions.push(event.clone());
        Ok(())
    }
    fn prepare_compaction<'a>(&'a mut self, _: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = serde_json::Value> + Send + 'a>> {
        Box::pin(async {
            json!({"ok":true,"instructions":COMPACT_INSTRUCTIONS_FIXTURE,"transcriptPath":COMPACT_TRANSCRIPT_FIXTURE,
                "carryForward":format!("## Locus Carry-Forward Context\n{COMPACT_CARRY_FIXTURE}")})
        })
    }
    fn execute_tool<'a>(
        &'a mut self,
        _: &'a str,
        name: &'a str,
        _: serde_json::Value,
    ) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move {
            if let Some(blocked) = &self.blocked_tool {
                assert_eq!(name, "write");
                blocked.store(true, Ordering::SeqCst);
                return std::future::pending().await;
            }
            panic!("Attachment test unexpectedly called a tool: {name}")
        })
    }
}
struct Input {
    available: Arc<AtomicBool>,
    claimed: AtomicBool,
    accepted: AtomicBool,
}
impl ClaudeCodeInputSource for Input {
    fn claim(&self) -> Result<Option<ClaudeCodeInput>, String> {
        Ok((self.available.load(Ordering::SeqCst)&&!self.claimed.swap(true,Ordering::SeqCst)).then(||ClaudeCodeInput{
            id:"insert-fixture".into(),message:json!({"role":"user","content":"INSERT_MARKER_67218: stop the previous answer and acknowledge this message."})}))
    }
    fn accepted(&self, id: &str, checkpoint: &serde_json::Value) -> Result<(), String> {
        assert_eq!(id, "insert-fixture");
        recovery::validate_input_checkpoint(checkpoint)?;
        self.accepted.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn refresh_tools(&self)->Pin<Box<dyn Future<Output=Result<Option<Vec<ClaudeCodeToolDefinition>>,String>>+Send+'_>> {
        Box::pin(async {Ok(Some(vec![ClaudeCodeToolDefinition{name:"inserted_skill_probe".into(),description:"Tool activated by an inserted Skill".into(),input_schema:json!({"type":"object","properties":{}}),defer_loading:false}]))})
    }
}
fn response(text: &str) -> String {
    [
        json!({"type":"message_start","message":{"id":"msg_fixture","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":20}}),
        json!({"type":"message_stop"}),
    ].iter().map(|event|format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect()
}

#[tokio::test]
#[ignore = "requires the configured SSH Claude server; uses the normal credential handshake and an isolated session"]
async fn claude_queued_input_real_ssh_continues_tools_and_the_next_turn() {
    struct ProbeInput { ready: Arc<AtomicBool>, claimed: AtomicBool, receipt: Mutex<Option<serde_json::Value>>, id: String }
    impl ClaudeCodeInputSource for ProbeInput {
        fn claim(&self)->Result<Option<ClaudeCodeInput>,String> {
            Ok((self.ready.load(Ordering::SeqCst) && !self.claimed.swap(true,Ordering::SeqCst)).then(||ClaudeCodeInput {
                id:self.id.clone(),message:json!({"role":"user","content":recovery::pending_input_text(&self.id,
                    "Continue now: call mcp__locus__queued_probe with stage after exactly once. Then answer QUEUED_INPUT_OK. Do not retry the before operation.")})
            }))
        }
        fn accepted(&self,id:&str,checkpoint:&serde_json::Value)->Result<(),String> {
            assert_eq!(id,self.id); recovery::validate_input_checkpoint(checkpoint)?;
            *self.receipt.lock().unwrap()=Some(checkpoint.clone()); Ok(())
        }
    }
    struct ProbeHost { ready: Arc<AtomicBool>, executed: Vec<String>, checkpoints: Vec<serde_json::Value> }
    impl ClaudeCodeHost for ProbeHost {
        fn on_text_delta(&mut self,_:String) {}
        fn on_thinking_delta(&mut self,_:String) {}
        fn on_tool_call_start(&mut self,_:String,_:String) {}
        fn on_assistant_message(&mut self,_:ClaudeCodeAssistantMessage)->Result<(),String>{Ok(())}
        fn on_checkpoint(&mut self,event:&serde_json::Value)->Result<(),String>{self.checkpoints.push(event.clone());Ok(())}
        fn execute_tool<'a>(&'a mut self,_:&'a str,name:&'a str,args:serde_json::Value)->ClaudeCodeHostFuture<'a> {
            Box::pin(async move {
                assert_eq!(name,"queued_probe","Only the read-only fixture may execute");
                let stage=args["stage"].as_str().unwrap().to_string();
                self.executed.push(stage.clone());
                if stage=="before" {self.ready.store(true,Ordering::SeqCst);return std::future::pending().await;}
                ClaudeCodeToolResult{output:stage,is_error:false,images:None,context:Vec::new()}
            })
        }
    }
    let root=tempfile::tempdir().unwrap().keep();
    let _env=super::native_integration_tests::EnvGuard::set(&[("LOCUS_CLAUDE_TUI_PROFILE_ROOT",Some(root.join("tui").to_string_lossy().into_owned()))]);
    let remote=ssh::load_settings().unwrap().remote.expect("Configure an SSH connection first");
    let id=uuid::Uuid::new_v4().to_string();
    let ready=Arc::new(AtomicBool::new(false));
    let input=ProbeInput{ready:ready.clone(),claimed:AtomicBool::new(false),receipt:Mutex::new(None),id:uuid::Uuid::new_v4().to_string()};
    let mut host=ProbeHost{ready,executed:vec![],checkpoints:vec![]};
    let options=ClaudeCodeCliOptions{locus_session_id:id.clone(),resume_session_id:None,cwd:root.to_string_lossy().into_owned(),
        model:"claude-sonnet-5-5".into(),effort:Some("low".into()),
        system_prompt:"Test queued inputs. Use only mcp__locus__queued_probe and follow the exact requested stage sequence. Never use other tools.".into(),
        tools:vec![ClaudeCodeToolDefinition{name:"queued_probe".into(),description:"Read-only queued input regression fixture; returns its stage.".into(),
            input_schema:json!({"type":"object","properties":{"stage":{"type":"string","enum":["before","after","next"]}},"required":["stage"]}),defer_loading:false}],
        remote:Some(remote),native_tool_search:false,native_agents:false,agents:serde_json::Value::Null};
    let result=tokio::time::timeout(Duration::from_secs(180),async {
        let first=run_turn_with_inputs(options.clone(),json!({"role":"user","content":"Call mcp__locus__queued_probe with stage before exactly once. While it is running, the user will send a new direction. Follow it when it arrives."}),&mut host,&input).await?;
        std::fs::write(root.join("first-response.jsonl"),&first.raw_response).unwrap();
        let second=run_turn(ClaudeCodeCliOptions{resume_session_id:first.claude_session_id.clone(),..options.clone()},
            json!({"role":"user","content":"Call mcp__locus__queued_probe with stage next exactly once. Then answer NEXT_TURN_OK."}),&mut host).await?;
        std::fs::write(root.join("second-response.jsonl"),&second.raw_response).unwrap();
        let snapshot=recovery::fetch(&id,options.remote.as_ref().unwrap(),first.claude_session_id.as_deref()).await?;
        let receipt=input.receipt.lock().unwrap().clone().ok_or("Missing native input receipt")?;
        assert!(snapshot.messages.iter().any(|m|m.queued_input && m.uuids.iter().any(|uuid|Some(uuid.as_str())==receipt["uuid"].as_str())));
        std::fs::write(root.join("recovered-snapshot.json"),serde_json::to_vec_pretty(&snapshot).unwrap()).unwrap();
        Ok::<_,String>((first,second))
    }).await;
    tui::close_session(&id);
    eprintln!("QUEUED_INPUT_PROBE_ROOT {}",root.display());
    let (first,second)=result.unwrap().unwrap();
    assert_eq!(host.executed,vec!["before","after","next"]);
    assert!(first.final_text.contains("QUEUED_INPUT_OK") && second.final_text.contains("NEXT_TURN_OK"));
    let receipt=input.receipt.lock().unwrap().clone().expect("Missing consumed input receipt");
    assert!(host.checkpoints.iter().any(|e|e["submissionId"]==input.id && e["uuid"]==receipt["uuid"]));
}

fn interrupted_tool_response() -> String {
    [
        json!({"type":"message_start","message":{"id":"msg_interrupted","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"usage":{"input_tokens":100,"output_tokens":0}}}),
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_compact_interrupted","name":"Write","input":{}}}),
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"file_path\":\"must-not-write.txt\",\"content\":\"interrupted\"}"}}),
        json!({"type":"content_block_stop","index":0}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":20}}),
        json!({"type":"message_stop"}),
    ].iter().map(|event|format!("event: {}\ndata: {event}\n\n",event["type"].as_str().unwrap())).collect()
}

#[tokio::test]
#[ignore = "requires real Claude 2.1.299; run with --ignored --test-threads=1"]
async fn real_tui_native_attachments_fork_compact_and_insert() {
    let root = tempfile::tempdir().unwrap().keep();
    let evidence = root.as_path().to_path_buf();
    eprintln!("NATIVE_FEATURE_ROOT {}", evidence.display());
    std::fs::create_dir_all(root.as_path().join("db")).unwrap();
    let _env = super::native_integration_tests::EnvGuard::set(&[
        (
            "CLAUDE_CONFIG_DIR",
            Some(
                root.as_path()
                    .join("credentials")
                    .to_string_lossy()
                    .into_owned(),
            ),
        ),
        (
            "LOCUS_CLAUDE_TUI_PROFILE_ROOT",
            Some(root.as_path().join("tui").to_string_lossy().into_owned()),
        ),
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
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let _endpoint = super::native_integration_tests::EnvGuard::set(&[(
        "ANTHROPIC_BASE_URL",
        Some(format!("http://{}", listener.local_addr().unwrap())),
    )]);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_server = requests.clone();
    let slow = Arc::new(AtomicBool::new(false));
    let slow_server = slow.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let requests = requests_server.clone();
            let slow = slow_server.clone();
            let evidence = evidence.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(
                    move |req: hyper::Request<hyper::body::Incoming>| {
                        let requests = requests.clone();
                        let slow = slow.clone();
                        let evidence = evidence.clone();
                        async move {
                            let route = req.uri().path().to_string();
                            let bytes = req.into_body().collect().await.unwrap().to_bytes();
                            let value: serde_json::Value =
                                serde_json::from_slice(&bytes).unwrap_or(json!({}));
                            let messages = value["messages"].to_string();
                            let main = route == "/v1/messages"
                                && value["tools"].as_array().is_some_and(|tools| {
                                    tools.iter().any(|tool| tool["name"] == "Read")
                                });
                            let mut text =
                                "NATIVE_COMPACT_SUMMARY attachment secret: FILE_ATTACHMENT_83914";
                            if main {
                                requests.lock().unwrap().push(value.clone());
                                let _ = std::fs::write(
                                    evidence.join("last-request.json"),
                                    value.to_string(),
                                );
                                if messages.contains("INSERT_MARKER_67218") {
                                    text = "INSERT_ACCEPTED_67218";
                                } else if messages.contains("START_SLOW_INSERT") {
                                    let body=response("PARTIAL_BEFORE_INSERT");
                                    let split=body.find("event: content_block_stop").unwrap();
                                    let chunks=vec![body[..split].to_string(),body[split..].to_string()];
                                    let stream=futures::stream::unfold((0,chunks,slow),|(index,chunks,slow)|async move {
                                        if index>=chunks.len(){return None;}
                                        if index==1 {
                                            tokio::time::sleep(Duration::from_millis(500)).await;
                                            slow.store(true,Ordering::SeqCst);
                                            tokio::time::sleep(Duration::from_secs(20)).await;
                                        }
                                        let frame=Ok::<_,std::convert::Infallible>(Frame::data(Bytes::from(chunks[index].clone())));
                                        Some((frame,(index+1,chunks,slow)))
                                    });
                                    return Ok::<_,std::io::Error>(hyper::Response::builder().header("Content-Type","text/event-stream").body(StreamBody::new(stream).boxed_unsync()).unwrap());
                                } else {
                                    text = "ATTACHMENT_OK_83914";
                                }
                            }
                            let body = if route == "/v1/messages" {
                                // Native compaction reuses the tool catalog and
                                // includes the old prompt in its input. Only the
                                // explicit fixture turn should request a tool.
                                let interrupt = value["messages"].as_array().and_then(|rows| rows.last())
                                    .and_then(|row| row["content"].as_array())
                                    .is_some_and(|blocks| blocks.iter().any(|block| block["text"] == "INTERRUPT_BEFORE_COMPACT"));
                                if main && interrupt {
                                    interrupted_tool_response()
                                } else {
                                    response(text)
                                }
                            } else {
                                "{}".into()
                            };
                            Ok::<_, std::io::Error>(
                                hyper::Response::builder()
                                    .header(
                                        "Content-Type",
                                        if route == "/v1/messages" {
                                            "text/event-stream"
                                        } else {
                                            "application/json"
                                        },
                                    )
                                    .body(Full::new(Bytes::from(body)).boxed_unsync())
                                    .unwrap(),
                            )
                        }
                    },
                );
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(socket), service)
                    .await;
            });
        }
    });
    let store = crate::session::store::SessionStore::new(&root.as_path().join("db")).unwrap();
    let session = store
        .create_session("Native features", None, None, "chat", None)
        .unwrap();
    let options = ClaudeCodeCliOptions {
        remote: None,
        locus_session_id: session.clone(),
        cwd: root.as_path().to_string_lossy().into_owned(),
        system_prompt: "Native attachment test".into(),
        model: "claude-sonnet-4-6".into(),
        effort: None,
        resume_session_id: None,
        tools: vec![],
        native_tool_search: false,
        native_agents: false,
        agents: serde_json::Value::Null,
    };
    let file = root.as_path().join("space name.txt");
    std::fs::write(&file, "FILE_ATTACHMENT_83914").unwrap();
    let image = |data: &[u8]| json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":base64::engine::general_purpose::STANDARD.encode(data)}});
    // An incompressible, valid PNG beyond @file's 256 KiB preload limit.
    let mut pixels = vec![0u8; 512 * 256 * 3];
    let mut random = 1729u32;
    for pixel in &mut pixels {
        random ^= random << 13; random ^= random >> 17; random ^= random << 5;
        *pixel = random as u8;
    }
    let mut large_image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut large_image, 512, 256);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
    }
    assert!(large_image.len() > 256 * 1024);
    let pending_a=uuid::Uuid::new_v4().to_string(); let pending_b=uuid::Uuid::new_v4().to_string();
    let user = json!({"role":"user","locusMessageId":"latest-input","locusPendingUsers":[
        {"id":"pending-a","token":pending_a,"text":recovery::pending_input_text(&pending_a,&format!("PENDING_USER_A\n<locus-local-files>\n- file: `{}`\n</locus-local-files>",file.display()))},
        {"id":"pending-b","token":pending_b,"text":recovery::pending_input_text(&pending_b,"PENDING_USER_B")}],
        "content":[image(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/icons/32x32.png"))),image(&large_image),{"type":"text","text":"ATTACHMENT_TEST"}]});
    let mut host = Host::default();
    eprintln!("NATIVE_FEATURE_STAGE attachments");
    let run = tokio::time::timeout(
        Duration::from_secs(60),
        run_turn(options.clone(), user.clone(), &mut host),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(run.final_text, "ATTACHMENT_OK_83914");
    let first = requests.lock().unwrap()[0].clone();
    let wire_messages=first["messages"].as_array().unwrap();
    let positions=["PENDING_USER_A","PENDING_USER_B","ATTACHMENT_TEST"].map(|marker| {
        let index=wire_messages.iter().position(|m|m["content"].to_string().contains(marker)).expect("pending text missing from native model input");
        assert_eq!(wire_messages[index]["role"],"user"); index
    });
    assert!(positions[0]<=positions[1] && positions[1]<=positions[2]);
    let receipts:Vec<_>=host.checkpoints.iter().filter(|e|e["user"]==true).collect();
    assert_eq!(receipts.iter().map(|e|e["messageId"].as_str().unwrap()).collect::<Vec<_>>(),vec!["pending-a","pending-b","latest-input"]);
    assert_eq!(receipts.iter().map(|e|e["uuid"].as_str().unwrap()).collect::<std::collections::HashSet<_>>().len(),3);
    let content = first["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .collect::<Vec<_>>();
    assert_eq!(
        content
            .iter()
            .filter(|block| block["type"] == "image")
            .count(),
        2,
        "native images absent from first request"
    );
    assert!(
        first["messages"]
            .to_string()
            .contains("FILE_ATTACHMENT_83914"),
        "native @file snapshot absent"
    );
    assert!(
        host.checkpoints
            .iter()
            .any(|event| event["role"] == "assistant"),
        "native checkpoint missing"
    );
    let native_id = run.claude_session_id.unwrap();
    let receipt_count=host.checkpoints.len();
    // Lost acknowledgement / restart can present the same pending-note tokens
    // again. Native API history must suppress only those exact accepted rows.
    run_turn(ClaudeCodeCliOptions{resume_session_id:Some(native_id.clone()),..options.clone()},
        json!({"role":"user","locusMessageId":"retry-input","locusPendingUsers":user["locusPendingUsers"],"content":"RETRY_CURRENT_INPUT"}),
        &mut host).await.unwrap();
    assert!(!host.checkpoints[receipt_count..].iter().any(|e|matches!(e["messageId"].as_str(),Some("pending-a"|"pending-b"))));
    let retry_wire=requests.lock().unwrap().last().unwrap()["messages"].to_string();
    assert_eq!(retry_wire.matches("PENDING_USER_A").count(),1);
    assert_eq!(retry_wire.matches("PENDING_USER_B").count(),1);
    let message = store
        .add_message(
            &session,
            crate::session::models::MessageRole::Assistant,
            "ATTACHMENT_OK_83914",
        )
        .unwrap();
    store
        .set_message_cli_session_id(&session, &message, "claude_code", &native_id)
        .unwrap();
    let point = host
        .checkpoints
        .iter()
        .rev()
        .find(|event| event["role"] == "assistant")
        .unwrap();
    store.set_message_response_request_metadata(&session,&message,&json!({"claude_connection":null,"claude_checkpoint":{"version":1,"uuid":point["uuid"],"sessionId":native_id}})).unwrap();
    let branch = store.fork_session(&session, Some("Native branch")).unwrap();
    super::fork::prepare(&store, &session, &branch)
        .await
        .unwrap();
    eprintln!("NATIVE_FEATURE_STAGE fork");
    let branch_id = store
        .latest_cli_session_id(&branch, "claude_code")
        .unwrap()
        .unwrap();
    assert_ne!(branch_id, native_id);
    let branch_options = ClaudeCodeCliOptions {
        locus_session_id: branch.clone(),
        resume_session_id: Some(branch_id.clone()),
        ..options.clone()
    };
    let branched = tokio::time::timeout(
        Duration::from_secs(45),
        run_turn(
            branch_options.clone(),
            json!({"role":"user","content":"FORK_TEST: what was attached?"}),
            &mut host,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        branched.claude_session_id.as_deref(),
        Some(branch_id.as_str())
    );
    assert!(requests.lock().unwrap().last().unwrap()["messages"]
        .to_string()
        .contains("FILE_ATTACHMENT_83914"));
    assert_eq!(
        requests.lock().unwrap().last().unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().into_iter().flatten())
            .filter(|block| block["type"] == "image")
            .count(),
        2
    );
    eprintln!("NATIVE_FEATURE_STAGE rewind");
    let boundary=store.get_messages(&branch).unwrap()[0].id.clone();
    store.add_message(&branch,crate::session::models::MessageRole::User,"FORK_TEST: what was attached?").unwrap();
    assert_eq!(super::rewind::rewind(&store,&branch,Some(&boundary)).await.unwrap(),Some(1));
    let rewound_id=store.latest_cli_session_id(&branch,"claude_code").unwrap().unwrap();
    assert_ne!(rewound_id,branch_id);
    let rewound=run_turn(ClaudeCodeCliOptions{resume_session_id:Some(rewound_id.clone()),..branch_options},
        json!({"role":"user","content":"AFTER_REWIND: acknowledge the remaining attachment"}),&mut host).await.unwrap();
    assert_eq!(rewound.claude_session_id.as_deref(),Some(rewound_id.as_str()));
    let sent=requests.lock().unwrap().last().unwrap()["messages"].to_string();
    assert!(!sent.contains("FORK_TEST"),"rewound request retained the discarded turn");
    assert!(sent.contains("FILE_ATTACHMENT_83914"),"rewind lost native attachments");
    super::tui::close_session(&branch);
    eprintln!("NATIVE_FEATURE_STAGE interrupt-before-compact");
    let blocked = Arc::new(AtomicBool::new(false));
    host.blocked_tool = Some(blocked.clone());
    {
        let turn = run_turn(
            ClaudeCodeCliOptions { resume_session_id: Some(native_id.clone()), ..options.clone() },
            json!({"role":"user","content":"INTERRUPT_BEFORE_COMPACT"}),
            &mut host,
        );
        tokio::pin!(turn);
        tokio::select! {
            result = &mut turn => panic!("Expected an interrupted tool, got {result:?}"),
            ready = tokio::time::timeout(Duration::from_secs(45), async {
                while !blocked.load(Ordering::SeqCst) { tokio::time::sleep(Duration::from_millis(10)).await; }
            }) => ready.unwrap(),
        }
        // Dropping the active turn is the same cancellation path as Stop.
    }
    host.blocked_tool = None;
    assert!(!super::tui::has_live_session(&session));
    assert!(!root.as_path().join("must-not-write.txt").exists());
    eprintln!("NATIVE_FEATURE_STAGE compact");
    // /compact must also work as the first input after a cancelled turn or
    // idle process eviction, when the Mod has no in-memory checkpoint yet.
    super::tui::close_session(&session);
    let options = ClaudeCodeCliOptions {
        resume_session_id: Some(native_id.clone()),
        ..options
    };
    let compact = tokio::time::timeout(
        Duration::from_secs(60),
        run_turn(
            options.clone(),
            json!({"role":"user","content":"/compact","locusControl":"compact"}),
            &mut host,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        host.compactions
            .iter()
            .any(|event| event["type"] == "compact_done"),
        "native compact missing: {}",
        compact.final_text
    );
    // Locus additions: summarizer instructions, and the carry-forward block
    // as the compaction's SessionStart context, which Claude stores after the
    // summary (interactive Claude does not pass compaction rows through
    // session.append, so the summary itself stays native).
    assert!(requests.lock().unwrap().iter().any(|request| request.to_string().contains(COMPACT_INSTRUCTIONS_FIXTURE)),
        "summarizer request lacked the Locus compaction instructions");
    let done = host.compactions.iter().rev().find(|event| event["type"] == "compact_done").unwrap();
    assert!(done["carryForward"].as_str().is_some_and(|text| text.contains(COMPACT_CARRY_FIXTURE)), "compact receipt lacked carry-forward: {done}");
    assert!(!done["summary"].as_str().unwrap().contains(COMPACT_CARRY_FIXTURE), "the native summary must stay unmodified");
    assert_eq!(done["transcriptPath"], COMPACT_TRANSCRIPT_FIXTURE);
    // Claude flushes transcript rows asynchronously.
    let stored = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let text = super::tui::local_native_transcript(&session, &native_id)
                .and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
            if let Some(line) = text.lines().find(|line| line.contains("hook_additional_context") && line.contains(COMPACT_CARRY_FIXTURE)) {
                break line.to_string();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }).await.expect("Claude did not store the Locus carry-forward context");
    assert!(stored.contains("\"hookEvent\":\"SessionStart\""), "unexpected carry-forward row: {stored}");
    assert!(super::tui::has_synchronized_session(&session));
    super::tui::require_recovery(&session);
    assert!(super::tui::has_live_session(&session), "keep native rows flushing after compaction");
    assert!(!super::tui::has_synchronized_session(&session), "compaction must not clear interrupted-tool recovery");
    eprintln!("NATIVE_FEATURE_STAGE compact-fork");
    let receipt = host
        .compactions
        .iter()
        .rev()
        .find(|event| event["type"] == "compact_done")
        .unwrap();
    let marker = store
        .add_message(
            &session,
            crate::session::models::MessageRole::Assistant,
            "/compact complete",
        )
        .unwrap();
    store
        .set_message_cli_session_id(&session, &marker, "claude_code", &native_id)
        .unwrap();
    store.set_message_response_request_metadata(&session,&marker,&json!({"claude_connection":null,"claude_checkpoint":receipt["checkpoint"],"claude_compaction":receipt})).unwrap();
    let compact_branch = store
        .fork_session(&session, Some("Compacted branch"))
        .unwrap();
    super::fork::prepare(&store, &session, &compact_branch)
        .await
        .unwrap();
    let compact_id = store
        .latest_cli_session_id(&compact_branch, "claude_code")
        .unwrap()
        .unwrap();
    let compact_options = ClaudeCodeCliOptions {
        locus_session_id: compact_branch.clone(),
        resume_session_id: Some(compact_id),
        ..options.clone()
    };
    tokio::time::timeout(
        Duration::from_secs(45),
        run_turn(
            compact_options,
            json!({"role":"user","content":"COMPACT_FORK_TEST"}),
            &mut host,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        requests.lock().unwrap().last().unwrap()["messages"]
            .to_string()
            .contains("This session is being continued from a previous conversation"),
        "fork restored the pre-compaction context"
    );
    assert!(!requests.lock().unwrap().last().unwrap()["messages"].to_string().contains("ATTACHMENT_TEST"),"fork replayed the original uncompacted prompt");
    assert!(requests.lock().unwrap().last().unwrap()["messages"].to_string().contains(COMPACT_CARRY_FIXTURE),
        "resumed context lacked the carry-forward block");
    super::tui::close_session(&compact_branch);
    eprintln!("NATIVE_FEATURE_STAGE insert");
    let inputs = Input {
        available: slow,
        claimed: AtomicBool::new(false),
        accepted: AtomicBool::new(false),
    };
    let inserted = tokio::time::timeout(
        Duration::from_secs(60),
        run_turn_with_inputs(
            options,
            json!({"role":"user","content":"START_SLOW_INSERT"}),
            &mut host,
            &inputs,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        inputs.accepted.load(Ordering::SeqCst),
        "insert lacked native consumption ACK"
    );
    assert_eq!(inserted.final_text, "INSERT_ACCEPTED_67218");
    let continued: Vec<String> = requests.lock().unwrap().iter().map(|request| request["messages"].to_string())
        .filter(|messages| messages.contains("START_SLOW_INSERT")).collect();
    assert!(!continued.is_empty() && continued.iter().all(|messages| messages.contains(COMPACT_CARRY_FIXTURE)),
        "the compacted session's next request lacked the carry-forward block");
    assert!(requests.lock().unwrap().last().unwrap()["tools"].as_array().unwrap().iter().any(|tool|tool["name"]=="mcp__locus__inserted_skill_probe"),"inserted Skill tool was absent from the next native model request");
    assert_eq!(host.interrupted,vec!["PARTIAL_BEFORE_INSERT"],"interrupted text must be persisted separately from the successor response");
    super::tui::close_session(&session);
    server.abort();
}
