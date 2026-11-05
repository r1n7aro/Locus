use super::*;

fn checkout(store: &SessionStore, checkout_id: &str) {
    store
        .upsert_workspace_checkout(&WorkspaceCheckoutRecord {
            checkout_id: checkout_id.into(),
            project_id: format!("project-{checkout_id}"),
            root_path: format!("F:/{checkout_id}"),
            normalized_root: format!("f:/{checkout_id}"),
            last_opened_at: 1,
        })
        .unwrap();
}

fn service(store: &SessionStore, checkout_id: &str, kind: &str, enabled: bool, policy: &str) {
    store
        .upsert_workspace_service(&WorkspaceServiceRecord {
            checkout_id: checkout_id.into(),
            service_kind: kind.into(),
            service_instance_id: format!("{checkout_id}:{kind}"),
            enabled,
            activation_policy: policy.into(),
            local_config: serde_json::json!({}),
        })
        .unwrap();
}

fn policy_of(store: &SessionStore, checkout_id: &str, kind: &str) -> (bool, String) {
    let record = store
        .list_workspace_services(checkout_id)
        .unwrap()
        .into_iter()
        .find(|record| record.service_kind == kind)
        .unwrap();
    (record.enabled, record.activation_policy)
}

fn scope(checkout_id: &str, bindings: Vec<SessionRunServiceBinding>) -> SessionRunScopeSnapshot {
    SessionRunScopeSnapshot {
        project_id: format!("project-{checkout_id}"),
        checkout_id: checkout_id.into(),
        workspace_generation: 1,
        materialization_epoch: Some(0),
        branch_ref: None,
        head_oid: None,
        service_bindings: bindings,
    }
}

fn unity_binding(checkout_id: &str) -> SessionRunServiceBinding {
    SessionRunServiceBinding {
        service_kind: "unity".into(),
        service_instance_id: format!("{checkout_id}:unity"),
        runtime_generation: 7,
    }
}

#[test]
fn new_database_persists_workspace_service_modes() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    checkout(&store, "fresh");
    service(&store, "fresh", "unity", true, "auto");
    service(&store, "fresh", "blender", true, "lazy");
    assert_eq!(policy_of(&store, "fresh", "unity"), (true, "auto".into()));
    assert_eq!(policy_of(&store, "fresh", "blender"), (true, "lazy".into()));
}

#[test]
fn v54_unity_lazy_records_migrate_to_auto_and_old_sessions_export() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    checkout(&store, "unity-lazy");
    checkout(&store, "unity-off");
    service(&store, "unity-lazy", "unity", true, "lazy");
    service(&store, "unity-lazy", "blender", true, "lazy");
    service(&store, "unity-off", "unity", false, "disabled");
    let session = store
        .create_session_scoped(
            "Before upgrade",
            None,
            Some("project-unity-lazy"),
            Some("unity-lazy"),
            "chat",
            None,
        )
        .unwrap();
    store
        .add_message(&session, MessageRole::User, "keep this history")
        .unwrap();
    store
        .try_start_run_scoped(
            &session,
            "before-run",
            Some(&scope("unity-lazy", vec![unity_binding("unity-lazy")])),
        )
        .unwrap();
    store.update_run_status("before-run", "done", None).unwrap();
    store
        .conn
        .lock()
        .unwrap()
        .pragma_update(None, "user_version", 54)
        .unwrap();
    drop(store);

    let store = SessionStore::new(dir.path()).unwrap();
    assert_eq!(
        store
            .conn
            .lock()
            .unwrap()
            .pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        SessionStore::schema_version()
    );
    assert_eq!(policy_of(&store, "unity-lazy", "unity"), (true, "auto".into()));
    // Only Unity records written by the old detection change meaning.
    assert_eq!(policy_of(&store, "unity-lazy", "blender"), (true, "lazy".into()));
    assert_eq!(policy_of(&store, "unity-off", "unity"), (false, "disabled".into()));
    // Re-running the migration is a no-op.
    SessionStore::migrate_unity_service_auto_policy(&store.conn.lock().unwrap()).unwrap();
    assert_eq!(policy_of(&store, "unity-lazy", "unity"), (true, "auto".into()));

    let output = dir.path().join("context.yaml");
    crate::session::context_export::export_session_context_yaml(
        &store, &session, "F:/unity-lazy", None, None, &output,
    )
    .unwrap();
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    let runs = yaml["sessions"][0]["runs"].as_sequence().unwrap();
    assert_eq!(runs.len(), 1);
}

#[test]
fn late_service_binding_is_recorded_on_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();
    checkout(&store, "lazy");
    let session = store
        .create_session_scoped("Lazy", None, Some("project-lazy"), Some("lazy"), "chat", None)
        .unwrap();
    store
        .try_start_run_scoped(&session, "lazy-run", Some(&scope("lazy", Vec::new())))
        .unwrap();
    assert!(store
        .get_run_scope("lazy-run")
        .unwrap()
        .unwrap()
        .service_bindings
        .is_empty());

    store
        .update_run_service_bindings("lazy-run", &[unity_binding("lazy")])
        .unwrap();
    assert_eq!(
        store.get_run_scope("lazy-run").unwrap().unwrap().service_bindings,
        vec![unity_binding("lazy")]
    );

    // Unscoped runs carry no bindings and stay that way.
    let plain = store.create_session("Plain", None, None, "chat", None).unwrap();
    store.update_run_status("lazy-run", "done", None).unwrap();
    store.try_start_run(&plain, "plain-run").unwrap();
    store
        .update_run_service_bindings("plain-run", &[unity_binding("lazy")])
        .unwrap();
    assert!(store.get_run_scope("plain-run").unwrap().is_none());
}
