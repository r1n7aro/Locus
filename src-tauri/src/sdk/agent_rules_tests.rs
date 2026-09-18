use super::*;
use std::fs;
use tempfile::tempdir;

fn run(root: &std::path::Path, app: &Option<std::path::PathBuf>, operation: Operation) -> Value {
    operation
        .execute(app, &root.to_string_lossy(), "unity")
        .unwrap()
}

fn app_rules(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let app = root.join("app/agent");
    fs::create_dir_all(app.join("unity/rule")).unwrap();
    fs::write(app.join("unity/rule/default.md"), "# Default\nApp rule").unwrap();
    fs::write(app.join("unity/rule/off.md"), "# Off\nOptional rule").unwrap();
    fs::write(
        app.join("unity/rule_config.json"),
        r#"{"default.md":{"enabled":true,"order":10},"off.md":{"enabled":false,"order":20}}"#,
    )
    .unwrap();
    Some(app)
}

#[test]
fn workspace_rule_lifecycle_preserves_app_defaults_and_other_workspaces() {
    let temp = tempdir().unwrap();
    let app = app_rules(temp.path());
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    assert_eq!(run(&a, &app, Operation::List).as_array().unwrap().len(), 2);
    assert!(!a.join("Locus").exists(), "listing must be read-only");

    let disabled = run(&a, &app, Operation::SetEnabled("default.md".into(), false));
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["order"], 10);
    assert_eq!(disabled["source"], "app");
    assert_eq!(run(&b, &app, Operation::List)[0]["enabled"], true);
    assert!(!a.join("Locus/agent/unity/rule/default.md").exists());

    let added = run(
        &a,
        &app,
        Operation::Save("项目规范".into(), "# 项目规范\n使用项目格式".into()),
    );
    assert_eq!(added["key"], "项目规范.md");
    assert_eq!(added["source"], "project");
    assert_eq!(added["enabled"], true);
    assert!(added["order"].as_i64().unwrap() > 20);
    assert_eq!(
        run(&a, &app, Operation::Read("项目规范.md".into())),
        "# 项目规范\n使用项目格式"
    );
    assert_eq!(run(&b, &app, Operation::List).as_array().unwrap().len(), 2);

    let overlay = run(
        &a,
        &app,
        Operation::Save("default.md".into(), "# Default\nWorkspace override".into()),
    );
    assert_eq!(overlay["source"], "project");
    assert_eq!(
        overlay["enabled"], false,
        "editing must not re-enable a disabled rule"
    );
    assert_eq!(overlay["order"], 10);
    assert_eq!(
        run(&a, &app, Operation::SetEnabled("default.md".into(), true))["enabled"],
        true
    );
    assert_eq!(
        run(&a, &app, Operation::Read("default.md".into())),
        "# Default\nWorkspace override"
    );
    assert_eq!(
        run(&b, &app, Operation::Read("default.md".into())),
        "# Default\nApp rule"
    );
    assert_eq!(
        fs::read_to_string(app.as_ref().unwrap().join("unity/rule_config.json")).unwrap(),
        r#"{"default.md":{"enabled":true,"order":10},"off.md":{"enabled":false,"order":20}}"#
    );

    let inherited_off = run(
        &a,
        &app,
        Operation::Save("off.md".into(), "# Off\nEdited locally".into()),
    );
    assert_eq!(inherited_off["enabled"], false);
    assert_eq!(inherited_off["order"], 20);
    let explorer =
        crate::commands::collect_agent_rule_files(&app, &a.to_string_lossy(), "explorer", false)
            .unwrap();
    assert!(explorer.is_empty(), "rules must be scoped to their Agent");
}

#[test]
fn invalid_writes_do_not_modify_rules_or_create_phantom_overrides() {
    let temp = tempdir().unwrap();
    let root = temp.path().to_string_lossy();
    for name in [
        "",
        "../escape.md",
        "a/b.md",
        "a\\b.md",
        "C:escape",
        "rule.md:stream",
        "bad\nname",
    ] {
        assert!(
            Operation::Save(name.into(), "content".into())
                .execute(&None, &root, "unity")
                .is_err(),
            "{name}"
        );
    }
    for id in ["", "../unity", "a/b", "C:agent", "dev"] {
        assert!(
            Operation::Save("rule.md".into(), "content".into())
                .execute(&None, &root, id)
                .is_err(),
            "{id}"
        );
    }
    assert!(Operation::SetEnabled("missing.md".into(), false)
        .execute(&None, &root, "unity")
        .unwrap_err()
        .contains("not found"));
    assert!(!temp.path().join("Locus").exists());
}

#[test]
fn corrupt_workspace_config_is_reported_without_clobbering_files() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join("Locus/agent/unity");
    fs::create_dir_all(dir.join("rule")).unwrap();
    fs::write(dir.join("rule_config.json"), "{invalid").unwrap();
    fs::write(dir.join("rule/keep.md"), "Original").unwrap();
    let error = Operation::Save("keep.md".into(), "Changed".into())
        .execute(&None, &temp.path().to_string_lossy(), "unity")
        .unwrap_err();
    assert!(error.contains("Invalid rule config"));
    assert_eq!(
        fs::read_to_string(dir.join("rule/keep.md")).unwrap(),
        "Original"
    );
    assert_eq!(
        fs::read_to_string(dir.join("rule_config.json")).unwrap(),
        "{invalid"
    );
}

#[test]
fn concurrent_rule_updates_keep_every_override() {
    let temp = tempdir().unwrap();
    std::thread::scope(|scope| {
        for index in 0..8 {
            let root = temp.path();
            scope.spawn(move || {
                let key = format!("rule-{index}.md");
                run(
                    root,
                    &None,
                    Operation::Save(key.clone(), format!("# Rule {index}")),
                );
                run(root, &None, Operation::SetEnabled(key, false));
            });
        }
    });
    let rules = run(temp.path(), &None, Operation::List);
    assert_eq!(rules.as_array().unwrap().len(), 8);
    assert!(rules
        .as_array()
        .unwrap()
        .iter()
        .all(|rule| rule["enabled"] == false));
}

#[test]
fn plugin_rules_can_be_read_but_enablement_stays_plugin_managed() {
    let temp = tempdir().unwrap();
    let plugin = temp
        .path()
        .join(crate::plugin::PROJECT_PLUGINS_RELATIVE)
        .join("com.example.rules");
    fs::create_dir_all(plugin.join("rules")).unwrap();
    fs::write(
        plugin.join(crate::plugin::PLUGIN_MANIFEST_FILE_NAME),
        json!({
            "schemaVersion": 1, "id": "com.example.rules", "name": "Rules", "version": "1.0.0",
            "components": {"agents": [], "rules": [], "skills": [], "views": []}
        })
        .to_string(),
    )
    .unwrap();
    fs::write(plugin.join("rules/example.md"), "# Plugin rule").unwrap();
    let rules = run(temp.path(), &None, Operation::List);
    let key = rules[0]["key"].as_str().unwrap();
    assert_eq!(
        run(temp.path(), &None, Operation::Read(key.into())),
        "# Plugin rule"
    );
    assert!(Operation::SetEnabled(key.into(), false)
        .execute(&None, &temp.path().to_string_lossy(), "unity")
        .unwrap_err()
        .contains("controlled by plugin state"));
    assert!(!temp
        .path()
        .join("Locus/agent/unity/rule_config.json")
        .exists());
}

#[test]
fn rpc_requires_explicit_checkout_and_action_specific_parameters() {
    assert!(serde_json::from_value::<Params>(json!({"agentId": "unity"})).is_err());
    let mut value = json!({"agentId": "unity", "workspaceRef": {"checkoutId": "checkout-a", "expectedGeneration": 7}});
    let params: Params = serde_json::from_value(value.clone()).unwrap();
    assert!(Operation::parse("list", &params).is_ok());
    for action in ["read", "save", "set_enabled", "delete"] {
        assert!(Operation::parse(action, &params).is_err());
    }
    value["workingDir"] = json!("another-workspace");
    assert!(serde_json::from_value::<Params>(value).is_err());
    assert!(serde_json::from_value::<Params>(
        json!({"agentId": "unity", "workspaceRef": {"checkoutId": "a"}, "enabled": "false"})
    )
    .is_err());
}

#[cfg(windows)]
#[test]
fn workspace_rule_writes_reject_directory_junctions_outside_checkout() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&outside).unwrap();
    junction::create(&outside, workspace.join("Locus")).unwrap();
    let result = Operation::Save("escape.md".into(), "content".into()).execute(
        &None,
        &workspace.to_string_lossy(),
        "unity",
    );
    assert!(result
        .unwrap_err()
        .contains("outside the selected workspace"));
    assert!(!outside.join("agent").exists());
    junction::delete(workspace.join("Locus")).unwrap();
}
