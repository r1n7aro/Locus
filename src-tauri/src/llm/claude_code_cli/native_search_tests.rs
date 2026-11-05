//! Opt-in real CLI/Mod regression; all inference and search responses come from
//! a loopback fixture. No model credentials or external search service is used.
use super::*;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};

fn stream(blocks: Vec<serde_json::Value>, stop: &str) -> String {
    let mut events = vec![json!({"type":"message_start","message":{"id":"msg_search_fixture","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[],"stop_reason":null,"usage":{"input_tokens":10,"output_tokens":0}}})];
    for (index, block) in blocks.into_iter().enumerate() {
        let (start, delta) = if block["type"] == "tool_use" {
            (json!({"type":"tool_use","id":block["id"],"name":block["name"],"input":{}}),
             json!({"type":"input_json_delta","partial_json":block["input"].to_string()}))
        } else {
            (json!({"type":"text","text":""}), json!({"type":"text_delta","text":block["text"]}))
        };
        events.extend([
            json!({"type":"content_block_start","index":index,"content_block":start}),
            json!({"type":"content_block_delta","index":index,"delta":delta}),
            json!({"type":"content_block_stop","index":index}),
        ]);
    }
    events.extend([
        json!({"type":"message_delta","delta":{"stop_reason":stop,"stop_sequence":null},"usage":{"output_tokens":30}}),
        json!({"type":"message_stop"}),
    ]);
    events.iter().map(|e|format!("event: {}\ndata: {e}\n\n",e["type"].as_str().unwrap())).collect()
}
#[derive(Default)]
struct Host { results: Vec<(String, bool)> }
impl ClaudeCodeHost for Host {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, _: ClaudeCodeAssistantMessage) -> Result<(),String> { Ok(()) }
    fn on_external_tool_result<'a>(&'a mut self, id: &'a str, result: ClaudeCodeToolResult)
        -> Pin<Box<dyn Future<Output=Result<(),String>> + Send + 'a>> {
        Box::pin(async move { self.results.push((id.into(),result.is_error)); Ok(()) })
    }
    fn execute_tool<'a>(&'a mut self, _: &'a str, name: &'a str, _: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async move { panic!("Native search must not execute as a Locus tool: {name}") })
    }
}

#[tokio::test]
#[ignore = "requires installed Claude CLI; isolated loopback model/search fixture, run alone"]
async fn real_tui_four_searches_survive_a_twenty_second_first_search() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}",listener.local_addr().unwrap());
    let main_requests = Arc::new(AtomicUsize::new(0));
    let searches = Arc::new(AtomicUsize::new(0));
    let search_times = Arc::new(Mutex::new(Vec::new()));
    let m = main_requests.clone(); let s = searches.clone(); let times = search_times.clone();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                connection = listener.accept() => {
                    let (socket, _) = connection.unwrap();
                    let m=m.clone(); let s=s.clone(); let times=times.clone();
                    connections.spawn(async move {
                        let service = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                            let m=m.clone(); let s=s.clone(); let times=times.clone();
                            async move {
                                let route=req.uri().path().to_string();
                                let bytes=req.into_body().collect().await.unwrap().to_bytes();
                                let request:serde_json::Value=serde_json::from_slice(&bytes).unwrap_or(json!({}));
                                let mut body="{}".to_string(); let mut content_type="application/json";
                                if route == "/v1/messages" {
                                    let native_main=request["tools"].as_array().is_some_and(|tools|tools.iter().any(|t|t["name"]=="WebSearch"));
                                    let search=request["tools"].as_array().is_some_and(|tools|tools.iter().any(|t|t["type"].as_str().is_some_and(|t|t.starts_with("web_search"))));
                                    if native_main {
                                        let round=m.fetch_add(1,Ordering::SeqCst);
                                        let blocks=if round==0 {
                                            (0..4).map(|i|json!({"type":"tool_use","id":format!("toolu_search_{i}"),"name":"WebSearch","input":{"query":format!("fixture query {i}")}})).collect()
                                        } else { vec![json!({"type":"text","text":"SEARCH_LIFECYCLE_OK"})] };
                                        body=stream(blocks,if round==0 {"tool_use"} else {"end_turn"}); content_type="text/event-stream";
                                    } else if search {
                                        let index=s.fetch_add(1,Ordering::SeqCst);
                                        times.lock().unwrap().push(Instant::now());
                                        tokio::time::sleep(Duration::from_secs(if index==0 {20} else {1})).await;
                                        let blocks=vec![json!({"type":"text","text":format!("Fixture search result {index}")})];
                                        if request["stream"]==true { body=stream(blocks,"end_turn"); content_type="text/event-stream"; }
                                        else { body=json!({"id":"msg_search","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":blocks,"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string(); }
                                    } else {
                                        body=json!({"id":"msg_aux","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"text","text":"OK"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}).to_string();
                                    }
                                }
                                Ok::<_,std::convert::Infallible>(hyper::Response::builder().header("Content-Type",content_type).body(Full::new(Bytes::from(body))).unwrap())
                            }
                        });
                        let _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(socket),service).await;
                    });
                }
                _=connections.join_next(), if !connections.is_empty()=>{}
            }
        }
    });
    let _env = super::native_integration_tests::EnvGuard::set(&[
        ("LOCUS_CLAUDE_TUI_PROFILE_ROOT",Some(root.path().join("tui").to_string_lossy().into_owned())),
        ("CLAUDE_CONFIG_DIR",Some(root.path().join("credentials").to_string_lossy().into_owned())),
        ("ANTHROPIC_BASE_URL",Some(url)), ("ANTHROPIC_API_KEY",Some("fixture-key".into())),
        ("ANTHROPIC_AUTH_TOKEN",None), ("CLAUDE_CODE_OAUTH_TOKEN",None), ("CLAUDECODE",None),
        ("CLAUDE_CODE_USE_BEDROCK",None), ("CLAUDE_CODE_USE_VERTEX",None), ("CLAUDE_CODE_USE_FOUNDRY",None),
        ("DISABLE_NONESSENTIAL_TRAFFIC",Some("1".into())), ("NO_PROXY",Some("127.0.0.1,localhost".into())), ("no_proxy",Some("127.0.0.1,localhost".into())),
    ]);
    let id=uuid::Uuid::new_v4().to_string();
    let options=ClaudeCodeCliOptions { remote:None,locus_session_id:id.clone(),cwd:root.path().to_string_lossy().into_owned(),
        system_prompt:"LOCUS_SEARCH_LIFECYCLE".into(), model:"claude-sonnet-4-6".into(),effort:None,resume_session_id:None,tools:vec![],native_tool_search:false,native_agents:false,agents:serde_json::Value::Null };
    let mut host=Host::default();
    let result=tokio::time::timeout(Duration::from_secs(90),run_turn(options,json!({"role":"user","content":"Run the four fixture searches."}),&mut host)).await;
    tui::close_session(&id); server.abort();
    let result=result.expect("native search turn stalled").expect("native search lifecycle failed");
    assert_eq!(result.final_text,"SEARCH_LIFECYCLE_OK");
    assert_eq!(main_requests.load(Ordering::SeqCst),2);
    assert_eq!(searches.load(Ordering::SeqCst),4);
    assert_eq!(host.results.len(),4,"catch/append observers must not duplicate results");
    assert!(host.results.iter().all(|(_,error)| !error),"{:?}",host.results);
    let times=search_times.lock().unwrap();
    assert!(times.iter().max().unwrap().duration_since(*times.iter().min().unwrap())<Duration::from_secs(10),"searches were serialized inside the Mod hook");
    assert!(!result.raw_response.contains("bridge_fault"));
}
