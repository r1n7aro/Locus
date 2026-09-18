use super::*;
use serde_json::json;

fn context(root: &Path) -> ToolExecutionContext {
    ToolExecutionContext {
        working_dir: Some(root.to_string_lossy().into_owned()),
        ..Default::default()
    }
}

#[tokio::test]
async fn apply_patch_add_update_delete_move_and_preserve_crlf() {
    let root = tempfile::tempdir().unwrap();
    for (path, content) in [
        ("edit.txt", "old\r\n"),
        ("move.txt", "move\n"),
        ("delete.txt", "delete\n"),
    ] {
        std::fs::write(root.path().join(path), content).unwrap();
    }
    let result = (apply_patch().execute)(json!({"patch":"*** Begin Patch\n*** Add File: nested/new.txt\n+new\n*** Update File: edit.txt\n@@\n-old\n+edited\n*** Update File: move.txt\n*** Move to: nested/moved.txt\n@@\n-move\n+moved\n*** Delete File: delete.txt\n*** End Patch"}), context(root.path())).await;
    assert!(!result.is_error, "{}", result.output);
    assert_eq!(
        std::fs::read_to_string(root.path().join("edit.txt")).unwrap(),
        "edited\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("nested/moved.txt")).unwrap(),
        "moved\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("nested/new.txt")).unwrap(),
        "new\n"
    );
    assert!(!root.path().join("move.txt").exists());
    assert!(!root.path().join("delete.txt").exists());
}

#[tokio::test]
async fn apply_patch_validates_all_files_before_writing() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file.txt");
    std::fs::write(&path, "old\n").unwrap();
    let result = (apply_patch().execute)(json!({"patch":"*** Begin Patch\n*** Add File: new.txt\n+new\n*** Update File: file.txt\n@@\n-missing\n+changed\n*** End Patch"}), context(root.path())).await;
    assert!(result.is_error);
    assert!(!root.path().join("new.txt").exists());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "old\n");
}

#[tokio::test]
async fn apply_patch_rejects_existing_destinations_and_path_aliases() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file.txt"), "old\n").unwrap();
    for body in [
        "*** Add File: file.txt\n+new",
        "*** Update File: file.txt\n*** Move to: file.txt\n@@\n-old\n+new",
        "*** Delete File: file.txt\n*** Add File: ./file.txt\n+new",
    ] {
        let result = (apply_patch().execute)(
            json!({"patch":format!("*** Begin Patch\n{body}\n*** End Patch")}),
            context(root.path()),
        )
        .await;
        assert!(result.is_error, "{body}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("file.txt")).unwrap(),
            "old\n"
        );
    }
}

#[tokio::test]
async fn apply_patch_rejects_stale_prepared_content() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("file.txt");
    std::fs::write(&path, "old\n").unwrap();
    let changes = prepare(&json!({"patch":"*** Begin Patch\n*** Update File: file.txt\n@@\n-old\n+new\n*** End Patch"}), &context(root.path())).await.unwrap();
    std::fs::write(&path, "external\n").unwrap();
    assert!(commit(&changes[0])
        .await
        .unwrap_err()
        .contains("Edit conflict"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external\n");
}

#[tokio::test]
async fn apply_patch_knowledge_preflight_is_readonly_and_success_reports_frontmatter() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("Locus/knowledge/memory/context.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "old\n").unwrap();
    let identity =
        crate::workspace_service::identity::ProjectIdResolver::resolve(root.path()).unwrap();
    let runtime = crate::workspace_service::WorkspaceRuntime::new(identity, Vec::new(), 1);
    let mut ctx = context(root.path());
    ctx.execution = Some(std::sync::Arc::new(
        crate::workspace_service::AgentExecutionContext::new(
            runtime,
            std::collections::HashMap::new(),
        ),
    ));
    let body = "*** Update File: Locus/knowledge/memory/context.md\n@@\n-old\n+new\n";
    let invalid = json!({"patch":format!("*** Begin Patch\n{body}*** Delete File: missing.txt\n*** End Patch")});
    let result = (apply_patch().execute)(invalid, ctx.clone()).await;
    assert!(result.is_error);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
    let result = (apply_patch().execute)(
        json!({"patch":format!("*** Begin Patch\n{body}*** End Patch")}),
        ctx,
    )
    .await;
    assert!(!result.is_error, "{}", result.output);
    assert!(result.output.contains("Generated frontmatter:\n---\n"));
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("---\n"));
    assert!(content.ends_with("new\n"));
}
