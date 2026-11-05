use super::*;
use crate::llm::claude_code_cli::native_integration_tests::EnvGuard;

struct CommandHost;
impl ClaudeCodeHost for CommandHost {
    fn on_text_delta(&mut self, _: String) {}
    fn on_thinking_delta(&mut self, _: String) {}
    fn on_tool_call_start(&mut self, _: String, _: String) {}
    fn on_assistant_message(&mut self, _: ClaudeCodeAssistantMessage) -> Result<(), String> { Ok(()) }
    fn execute_tool<'a>(&'a mut self, _: &'a str, _: &'a str, _: serde_json::Value) -> ClaudeCodeHostFuture<'a> {
        Box::pin(async { panic!("Starting an idle TUI must not execute a tool") })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires explicitly authorized LOCUS_CLAUDE_SSH_TEST_CONFIG_DIR and LOCUS_CLAUDE_SSH_TEST_MODEL; starts idle remote TUIs only"]
async fn real_ssh_idle_startup_uses_saved_connection_without_inference() {
    let config_dir = std::env::var("LOCUS_CLAUDE_SSH_TEST_CONFIG_DIR").expect("select an isolated test config directory");
    let model = std::env::var("LOCUS_CLAUDE_SSH_TEST_MODEL").expect("select the permitted model");
    let _env = EnvGuard::set(&[("LOCUS_RUNTIME_CONFIG_DIR", Some(config_dir))]);
    let settings = super::super::ssh::load_settings().unwrap();
    let remote = settings.remote.expect("test profile must already use SSH");
    assert_eq!(settings.authorized_target.as_deref(), Some(remote.fingerprint().as_str()));
    for sample in 1..=2 {
        let options = ClaudeCodeCliOptions {
            remote: Some(remote.clone()), locus_session_id: uuid::Uuid::new_v4().to_string(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(), system_prompt: String::new(),
            model: model.clone(), effort: None, resume_session_id: None, tools: vec![], native_tool_search: false, native_agents: false, agents: serde_json::Value::Null,
        };
        let started = Instant::now();
        let session = connection_status::start_cached(&options, settings.permission_mode).await.unwrap();
        let transport_ms = started.elapsed().as_millis();
        assert_eq!(connection_status(Some(&options.locus_session_id), None).unwrap().phase, connection_status::ConnectionPhase::Starting);
        let mut bridge = session.bridge.lock().await;
        let result = await_ready(&session, &options, &mut CommandHost, &mut bridge).await;
        let ready_ms = started.elapsed().as_millis();
        assert_eq!(connection_status(Some(&options.locus_session_id), None).unwrap().phase, connection_status::ConnectionPhase::Ready);
        assert!(connection_status(Some("unrelated-conversation"), None).is_none());
        session.remote.as_ref().unwrap().stop();
        assert_eq!(connection_status(Some(&options.locus_session_id), None).unwrap().phase, connection_status::ConnectionPhase::Disconnected);
        close_session(&options.locus_session_id);
        assert!(connection_status(Some(&options.locus_session_id), None).is_none());
        result.unwrap();
        assert_eq!(session.options.read().unwrap()["active"], false);
        std::println!("LOCUS_IDLE_STARTUP {}", json!({"sample":sample,"transportMs":transport_ms,"readyMs":ready_ms,"modelPromptsSubmitted":0}));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real Claude CLI; run alone with --ignored --test-threads=1"]
async fn real_tui_auto_and_bypass_initialize_without_chat_commands() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("source");
    std::fs::create_dir_all(&config).unwrap();
    let _env = EnvGuard::set(&[
        ("LOCUS_CLAUDE_TUI_PROFILE_ROOT", Some(root.path().join("tui").to_string_lossy().into_owned())),
        ("CLAUDE_CONFIG_DIR", Some(config.to_string_lossy().into_owned())),
        ("ANTHROPIC_BASE_URL", Some("http://127.0.0.1:9".into())),
        ("ANTHROPIC_API_KEY", Some("locus-fixture-key".into())),
        ("ANTHROPIC_AUTH_TOKEN", None), ("CLAUDE_CODE_OAUTH_TOKEN", None),
        ("CLAUDE_CODE_USE_BEDROCK", None), ("CLAUDE_CODE_USE_VERTEX", None),
        ("CLAUDE_CODE_USE_FOUNDRY", None), ("DISABLE_NONESSENTIAL_TRAFFIC", Some("1".into())),
    ]);
    for mode in [super::super::ssh::PermissionMode::Auto, super::super::ssh::PermissionMode::BypassPermissions] {
        let options = ClaudeCodeCliOptions {
            remote: None, locus_session_id: format!("permission-probe-{}", mode.as_str()),
            cwd: root.path().to_string_lossy().into_owned(), system_prompt: "Locus permission probe".into(),
            model: "claude-sonnet-4-6".into(), effort: None, resume_session_id: None, tools: vec![], native_tool_search: false, native_agents: false, agents: serde_json::Value::Null,
        };
        let session = start(&options, mode, false).await.unwrap();
        let mut bridge = session.bridge.lock().await;
        let result = tokio::time::timeout(Duration::from_secs(40), await_ready(
            &session, &options, &mut CommandHost, &mut bridge,
        )).await;
        session.stop();
        let diagnostic = String::from_utf8_lossy(&session.diagnostic.lock().unwrap()).into_owned();
        result.unwrap_or_else(|_| panic!("{} timed out: {diagnostic}", mode.as_str()))
            .unwrap_or_else(|error| panic!("{} failed: {error}; {diagnostic}", mode.as_str()));
        assert!(session.ready.load(Ordering::SeqCst));
        assert!(session.session_id.lock().unwrap().is_some());
    }
}
