use super::*;
use serde_json::json;

#[tokio::test]
async fn apply_patch_choice_is_subscription_gpt_only_and_respects_edit_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = crate::agent::instance::tests::native_plan_test_instance(&root);
    agent.codex_use_apply_patch.store(true, Ordering::Relaxed);
    agent.effective_model = "openai/gpt-5.6-sol".into();
    assert!(!agent.uses_codex_apply_patch());
    agent.backend = LlmBackend::OpenAiCodex {
        auth: Arc::new(tokio::sync::Mutex::new(
            crate::auth::codex::CodexAuthState::new(root.path()),
        )),
        transport: crate::commands::CodexTransportMode::Http,
        base_url: None,
    };
    for model in ["gpt-5.3-codex", "openai/gpt-5.6-sol", "openai/gpt-6-astra"] {
        agent.effective_model = model.into();
        let names = agent.resolve_configured_tool_names().await;
        assert!(names.contains(&"apply_patch".into()));
        assert!(!names.contains(&"edit".into()));
    }
    assert!(!agent.is_tool_enabled("apply_patch", &HashMap::from([("edit".into(), false)])));
    assert_eq!(
        agent.configured_tool_load_mode("apply_patch", &HashMap::from([("edit".into(), false)])),
        ToolLoadMode::Lazy
    );
    assert!(agent.tool_call_needs_undo_tracking("apply_patch", &json!({})));
    assert!(matches!(
        agent.workspace_execution_request_for_tool("apply_patch", &json!({})),
        Some(WorkspaceExecutionLockRequest::Exclusive)
    ));
    agent.effective_model = "codex-auto-review".into();
    assert!(!agent.uses_codex_apply_patch());
    agent.effective_model = "gpt-6-astra".into();
    agent.codex_use_apply_patch.store(false, Ordering::Relaxed);
    let names = agent.resolve_configured_tool_names().await;
    assert!(names.contains(&"edit".into()));
    assert!(!names.contains(&"apply_patch".into()));
}

#[test]
fn apply_patch_checks_every_path_and_plan_target() {
    let root = tempfile::tempdir().unwrap();
    let agent = crate::agent::instance::tests::native_plan_test_instance(&root);
    let escaping = json!({"patch":"*** Begin Patch\n*** Update File: inside.txt\n*** Move to: ../outside.txt\n@@\n-old\n+new\n*** End Patch"});
    assert!(agent.validate_patch_paths(&escaping, true).is_some());
    let plan_file = root.path().join("plan.md");
    let runtime = PlanRuntime::Main {
        plan_file: plan_file.clone(),
    };
    let edit = json!({"patch":format!("*** Begin Patch\n*** Update File: {}\n@@\n-old\n+new\n*** End Patch",plan_file.display())});
    assert!(agent
        .plan_mode_tool_violation(&runtime, "apply_patch", &edit)
        .is_none());
    assert!(agent
        .plan_mode_tool_violation(&PlanRuntime::Subagent, "apply_patch", &edit)
        .is_some());
    assert!(agent
        .plan_mode_tool_violation(&runtime, "apply_patch", &escaping)
        .is_some());
    let delete = json!({"patch":format!("*** Begin Patch\n*** Delete File: {}\n*** End Patch",plan_file.display())});
    assert!(agent
        .plan_mode_tool_violation(&runtime, "apply_patch", &delete)
        .is_some());
}

#[test]
fn apply_patch_preview_matches_the_selected_subscription_model() {
    let root = tempfile::tempdir().unwrap();
    let mut agent = crate::agent::instance::tests::native_plan_test_instance(&root);
    agent.codex_use_apply_patch.store(true, Ordering::Relaxed);
    agent.configure_preview_lazy_tool_renderer(
        Some("openai/gpt-6-astra"),
        crate::config::DynamicToolLoadingMode::Direct,
        None,
    );
    assert!(agent.uses_codex_apply_patch());
    agent.configure_preview_lazy_tool_renderer(
        Some("custom/provider/gpt-6-astra"),
        crate::config::DynamicToolLoadingMode::Direct,
        None,
    );
    assert!(!agent.uses_codex_apply_patch());
}
