use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = fs::read_dir(dir).unwrap_or_else(|error| {
        panic!("failed to scan {}: {error}", dir.display());
    });
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}

fn compact_source(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn count_occurrences(source: &str, pattern: &str) -> usize {
    source.match_indices(pattern).count()
}

#[test]
fn production_backend_has_no_legacy_process_workspace_fact_source() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "State<'_,Arc<Workspace>>",
        "State<Arc<Workspace>>",
        "crate::workspace::Workspace",
        "pubstructWorkspace{",
        "workspace.path.read()",
        "workspace.path.write()",
        "selected_checkout_id",
        "register_and_select",
        "selected_runtime(",
        "get_working_dir(",
        "set_working_dir(",
        "commands::get_working_dir",
        "commands::set_working_dir",
        "handoff_background_watchers",
        "selected_workspace_changed",
        "emit_selected_unity_status_snapshot",
    ];

    for path in rust_files(&source_dir) {
        let source = compact_source(&path);
        for pattern in forbidden {
            assert!(
                !source.contains(pattern),
                "{} reintroduced legacy process workspace fact source `{pattern}`",
                path.display(),
            );
        }
    }
}

#[test]
fn checkout_owned_command_domains_require_an_explicit_scope() {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    for file_name in [
        "asset.rs",
        "csharp_lsp.rs",
        "git.rs",
        "knowledge.rs",
        "plan.rs",
        "ref_graph.rs",
        "skill.rs",
        "skill_external.rs",
        "system.rs",
        "undo.rs",
        "unity_embed.rs",
        "unity_serialized_property.rs",
        "view.rs",
        "mcp.rs",
        "workspace.rs",
        "workspace_service.rs",
    ] {
        let path = commands_dir.join(file_name);
        let source = compact_source(&path);
        assert!(
            !source.contains("Option<WorkspaceRef>"),
            "{} introduced optional checkout scope",
            path.display(),
        );
    }
}

#[test]
fn capability_and_dual_scope_commands_keep_their_reviewed_optional_scope_surface() {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    // These are the only reviewed dual-scope surfaces: Diff can resolve a
    // session-owned checkout, Session accepts legacy app-owned sessions, and
    // Plugin supports the app base plus a checkout overlay.
    let reviewed = [("diff.rs", 2), ("session.rs", 1), ("plugin.rs", 8)];
    for (file_name, expected) in reviewed {
        let path = commands_dir.join(file_name);
        let source = compact_source(&path);
        assert_eq!(
            count_occurrences(&source, "Option<WorkspaceRef>"),
            expected,
            "{} changed its reviewed optional WorkspaceRef surface",
            path.display(),
        );
    }
}

#[test]
fn checkout_ipc_surfaces_keep_their_reviewed_required_scope_counts() {
    let commands_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    for (file_name, expected) in [
        ("workspace.rs", 24),
        ("mcp.rs", 3),
        ("csharp_lsp.rs", 16),
        ("unity_embed.rs", 17),
        ("view.rs", 43),
    ] {
        let path = commands_dir.join(file_name);
        let source = compact_source(&path);
        assert_eq!(
            count_occurrences(&source, "workspace_ref:WorkspaceRef"),
            expected,
            "{} changed its reviewed required WorkspaceRef surface",
            path.display(),
        );
    }
}

#[test]
fn production_workspace_events_cannot_resolve_scope_from_a_root_path() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for path in rust_files(&source_dir) {
        let source = compact_source(&path);
        for forbidden in ["emit_for_workspace_root", "publish_for_root"] {
            assert!(
                !source.contains(forbidden),
                "{} reintroduced generationless workspace event routing `{forbidden}`",
                path.display(),
            );
        }
    }
}

#[test]
fn workspace_lock_diagnostics_use_the_scoped_event_router() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent/workspace_execution_lock.rs");
    let source = compact_source(&path);

    assert!(source.contains("publish_for_scope("));
    assert!(!source.contains("app_handle.emit(WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT"));
}

#[test]
fn checkout_execution_planes_do_not_use_legacy_workspace_or_root_event_routing() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for relative_path in [
        "mcp/server/http.rs",
        "mcp/server/tools.rs",
        "csharp_lsp/mod.rs",
        "csharp_compile/client.rs",
        "csharp_compile/manager.rs",
        "unity_bridge/mod.rs",
        "unity_bridge/transport.rs",
        "unity_bridge/plugin.rs",
        "unity_bridge/background_hook.rs",
        "unity_bridge/state_probe.rs",
        "view.rs",
    ] {
        let path = source_dir.join(relative_path);
        let source = compact_source(&path);
        for forbidden in [
            "crate::workspace::Workspace",
            "workspace.path.read()",
            "workspace.path.write()",
            "emit_for_workspace_root",
            "publish_for_root",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} reintroduced implicit checkout execution `{forbidden}`",
                path.display(),
            );
        }
    }
}

#[test]
fn sdk_and_mcp_execution_require_checkout_scope() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for relative_path in ["sdk.rs", "mcp/server/tools.rs"] {
        let path = source_dir.join(relative_path);
        let source = compact_source(&path);
        assert!(
            !source.contains("crate::workspace::Workspace")
                && !source.contains("workspace.path.read()")
                && !source.contains("workspace.path.write()")
                && !source.contains("main\",\"main"),
            "{} reintroduced implicit workspace execution",
            path.display(),
        );
    }
}

#[test]
fn csharp_compile_events_and_sidecar_keys_keep_exact_generation_identity() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/csharp_compile");
    let module = compact_source(&source_dir.join("mod.rs"));
    let client = compact_source(&source_dir.join("client.rs"));

    assert_eq!(
        count_occurrences(&module, "publish_prevalidated_external_service("),
        2,
        "C# compile status and scope-loss events must use the externally prevalidated service path",
    );
    assert_eq!(count_occurrences(&module, "service_instance_id:Some("), 2);
    assert_eq!(count_occurrences(&module, "service_generation:Some("), 2);
    assert!(module.contains("pubstructCompileScopeId{"));
    assert!(module.contains("pubworkspace_generation:u64"));
    assert!(module.contains("pubservice_generation:u64"));
    assert!(client.contains(
        "{checkout_id}\\0{workspace_generation}\\0{service_generation}\\0{editor_session_id}"
    ));
}

#[test]
fn mcp_integration_writes_require_an_exact_checkout_generation() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let commands = compact_source(&manifest_dir.join("src/commands/mcp.rs"));
    let install = compact_source(&manifest_dir.join("src/mcp/server/install.rs"));

    for command in [
        "mcp_server_integrations",
        "mcp_server_integration_apply",
        "mcp_server_integration_remove",
    ] {
        let start = commands
            .find(&format!("fn{command}("))
            .unwrap_or_else(|| panic!("missing MCP integration command {command}"));
        let signature = &commands[start
            ..commands[start..]
                .find(")->")
                .map(|offset| start + offset)
                .unwrap_or(commands.len())];
        assert!(
            signature.contains("workspace_ref:WorkspaceRef"),
            "{command} must require WorkspaceRef",
        );
        assert!(
            signature.contains("workspace_registry:State<'_,Arc<ProjectRegistry>>"),
            "{command} must resolve WorkspaceRef through ProjectRegistry",
        );
    }
    assert!(commands.contains("workspace_ref.expected_generation.is_none()"));
    assert!(commands.contains("settings.scoped_endpoint_url("));
    assert!(!commands.contains("apply_integration(&id,&settings.endpoint_url()"));
    assert!(!commands.contains("integration_statuses(&settings.endpoint_url()"));
    assert!(install.contains("scoped_entry_name("));
    assert!(install.contains("integration_config_lock()"));
}
