use super::*;
use tempfile::tempdir;

const COMPONENT: &str = "<script setup lang=\"ts\">import { ref } from 'vue'; const count = ref(0);</script>\n<template><button @click=\"count++\">{{ count }}</button></template>\n<style scoped>button { color: var(--text-color); }</style>\n";

fn request(file: &str) -> ViewCreateRequest {
    ViewCreateRequest {
        file_name: Some(file.into()),
        component: Some(COMPONENT.into()),
        ..Default::default()
    }
}

fn project_views(dir: &str) -> Vec<ViewPackageSummary> {
    list_views_sync(dir)
        .unwrap()
        .into_iter()
        .filter(|view| view.source == "project")
        .collect()
}

#[test]
fn initialize_view_creates_an_empty_component_and_only_requested_directories() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    let (request, temporary) = parse_view_create_request(serde_json::json!({
        "fileName": "new-panel.vue", "directories": ["src/components", "unity", "src/components"]
    }))
    .unwrap();
    let created = create_view_sync_with_scope(dir, request, temporary).unwrap();
    let root = PathBuf::from(&created.summary.package_root);
    assert!(root.join("src/components").is_dir());
    assert!(root.join("unity").is_dir());
    assert_eq!(created.files.len(), 1);
    assert_eq!(
        created.files[0].content,
        "<template>\n  <main />\n</template>\n"
    );
    assert!(serde_json::to_value(&created.manifest)
        .unwrap()
        .get("template")
        .is_none());
    assert!(serde_json::to_value(&created.summary)
        .unwrap()
        .get("template")
        .is_none());
    assert!(!root.parent().unwrap().join("package.json").exists());
    for directory in ["../escape", "/absolute", "panel.vue/child", ".locus/data"] {
        assert!(create_view_sync(
            dir,
            ViewCreateRequest {
                id: "panel".into(),
                directories: vec![directory.into()],
                ..Default::default()
            }
        )
        .is_err());
    }
    let panel = create_view_sync(
        dir,
        ViewCreateRequest {
            id: "panel".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read_dir(panel.summary.package_root)
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn single_file_create_uses_filename_without_metadata_or_scaffold() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    let (parsed, temporary) = parse_view_create_request(
        serde_json::json!({ "fileName": "counter.vue", "component": COMPONENT }),
    )
    .unwrap();
    let created = create_view_sync_with_scope(dir, parsed, temporary).unwrap();
    assert_eq!(created.manifest.id, "counter");
    assert_eq!(created.manifest.name, "counter");
    assert_eq!(created.manifest.entry, "counter.vue");
    assert!(created.manifest.style.is_empty());
    assert!(!created.manifest.capabilities.unity);
    let root = PathBuf::from(&created.summary.package_root);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert_eq!(
        std::fs::read_to_string(root.join("counter.vue")).unwrap(),
        COMPONENT
    );
    assert!(!root.parent().unwrap().join("package.json").exists());
    assert!(!root.parent().unwrap().join("src").exists());
    assert_eq!(created.files.len(), 1);
    assert!(created.summary.manifest_path.ends_with("/counter.vue"));
    assert_eq!(project_views(dir).len(), 1);
    assert_eq!(reload_view_sync(dir, "counter").unwrap().name, "counter");
    assert_eq!(resolve_view_package_root(dir, "counter").unwrap(), root);
}

#[test]
fn single_file_metadata_and_create_overrides_stay_in_the_vue_file() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    let mut input = request("inspector.vue");
    input.component = Some(format!(
        "<view>\n{{\"name\":\"Inspector\",\"unity\":true}}\n</view>\n{COMPONENT}"
    ));
    input.icon = Some("InspectionPanel".into());
    input.display_path = Some("Tools/inspector".into());
    let created = create_view_sync(dir, input).unwrap();
    assert_eq!(created.manifest.name, "Inspector");
    assert!(created.manifest.requirements.unwrap().unity_connection);
    assert!(created.manifest.capabilities.unity);
    assert_eq!(created.summary.display_path, "Tools/inspector");
    let root = PathBuf::from(&created.summary.package_root);
    set_view_manifest_name(&created.summary.package_root, "Renamed").unwrap();
    set_view_manifest_display_path(&created.summary.package_root, "Tools/renamed").unwrap();
    let current = read_view_sync(dir, "inspector").unwrap();
    assert_eq!(current.manifest.name, "Renamed");
    assert_eq!(current.summary.display_path, "Tools/renamed");
    let raw = std::fs::read_to_string(root.join("inspector.vue")).unwrap();
    assert!(raw.ends_with(COMPONENT));
    assert_eq!(raw.matches("<view>").count(), 1);
    assert!(!raw.contains("apiVersion"));
    assert!(!raw.contains("schema"));
    assert!(!root.join("view.json").exists());
}

#[test]
fn single_file_export_import_and_plugin_copy_preserve_one_source_file() {
    let workspace = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let output = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    create_view_sync(dir, request("counter.vue")).unwrap();
    let archive = export_view_package_sync(
        dir,
        ViewExportPackageRequest {
            view_id: "counter".into(),
            file_path: output.path().join("counter.zip").to_string_lossy().into(),
        },
    )
    .unwrap();
    let zip = zip::ZipArchive::new(std::fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(zip.file_names().collect::<Vec<_>>(), vec!["counter.vue"]);
    let imported = import_view_package_sync(
        destination.path().to_str().unwrap(),
        ViewImportPackageRequest {
            file_path: archive,
            target_dir_rel_path: None,
        },
    )
    .unwrap();
    assert_eq!(imported.summary.id, "counter");
    let root = PathBuf::from(imported.summary.package_root);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert!(!root.parent().unwrap().join("package.json").exists());
    let target = output.path().join("plugin/views/counter");
    copy_view_package_for_plugin_sync(dir, "counter", &target).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("counter.vue")).unwrap(),
        COMPONENT
    );
    assert_eq!(load_manifest_from_root(&target).unwrap().id, "counter");
    assert!(is_view_package_root(&target));
}

#[test]
fn single_file_runtime_storage_logs_and_reload_keep_identity_isolated() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    for file in ["one.vue", "two.vue"] {
        create_view_sync(dir, request(file)).unwrap();
    }
    view_storage_set_sync(
        dir,
        ViewStorageSetRequest {
            view_id: "one".into(),
            key: "count".into(),
            value: serde_json::json!(3),
        },
    )
    .unwrap();
    let value = view_storage_get_sync(
        dir,
        ViewStorageGetRequest {
            view_id: "two".into(),
            key: "count".into(),
        },
    )
    .unwrap();
    assert!(value.is_none());
    append_view_frontend_log_sync(
        dir,
        ViewFrontendLogRequest {
            view_id: "one".into(),
            level: "log".into(),
            message: "hello".into(),
        },
    )
    .unwrap();
    let root = resolve_view_package_root(dir, "one").unwrap();
    assert_eq!(view_file_watch_roots(&root).unwrap().len(), 1);
    std::fs::write(
        root.join("one.vue"),
        COMPONENT.replace("count = ref(0)", "count = ref(10)"),
    )
    .unwrap();
    let read = read_view_sync(dir, "one").unwrap();
    assert!(read
        .files
        .iter()
        .any(|file| file.content.contains("count = ref(10)")));
    assert_eq!(project_views(dir).len(), 2);
}

#[test]
fn single_file_temporary_names_are_unique_and_stay_out_of_the_catalog() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    let one = create_view_sync_with_scope(dir, request("counter.vue"), true).unwrap();
    let two = create_view_sync_with_scope(dir, request("counter.vue"), true).unwrap();
    assert_ne!(one.manifest.id, two.manifest.id);
    assert_eq!(one.manifest.entry, format!("{}.vue", one.manifest.id));
    assert!(one.summary.temporary);
    assert!(project_views(dir).is_empty());
    // Only the two roots created by this test belong to it.
    for detail in [one, two] {
        let root = resolve_view_package_root(dir, &detail.manifest.id).unwrap();
        assert_eq!(root, PathBuf::from(detail.summary.package_root));
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn single_file_invalid_inputs_do_not_reserve_an_id_or_write_scaffolding() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    for file in [
        "../counter.vue",
        "src/counter.vue",
        "counter.ts",
        "content.vue",
    ] {
        assert!(create_view_sync(dir, request(file)).is_err());
    }
    for source in [
        " ",
        "<view>{broken}</view><template/>",
        "<view>{\"id\":\"other\"}</view><template/>",
    ] {
        let mut input = request("counter.vue");
        input.component = Some(source.into());
        assert!(create_view_sync(dir, input).is_err());
    }
    assert!(parse_view_create_request(serde_json::json!({
        "fileName": "counter.vue", "template": "blank", "component": COMPONENT
    }))
    .unwrap_err()
    .contains("templates have been removed"));
    let mut mismatch = request("counter.vue");
    mismatch.id = "other".into();
    assert!(create_view_sync(dir, mismatch).is_err());
    assert!(!workspace.path().join(VIEW_ROOT_RELATIVE).exists());
    create_view_sync(dir, request("counter.vue")).unwrap();
    assert!(create_view_sync(dir, request("counter.vue")).is_err());
}

#[test]
fn single_file_discovery_works_without_the_create_api_and_ignores_legacy_helpers() {
    let workspace = tempdir().unwrap();
    let dir = workspace.path().to_str().unwrap();
    let root = workspace.path().join(VIEW_ROOT_RELATIVE).join("manual");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("counter.vue"), COMPONENT).unwrap();
    assert_eq!(project_views(dir)[0].id, "counter");
    assert_eq!(
        read_view_sync(dir, "counter").unwrap().manifest.entry,
        "counter.vue"
    );
    let old = super::tests::create_legacy_view_fixture(
        dir,
        ViewCreateRequest {
            id: "legacy".into(),
            ..Default::default()
        },
    )
    .unwrap();
    std::fs::write(
        Path::new(&old.summary.package_root).join("helper.vue"),
        COMPONENT,
    )
    .unwrap();
    assert_eq!(project_views(dir).len(), 2);
    assert_eq!(old.manifest.entry, "src/main.ts");
    assert_eq!(old.manifest.style, "src/style.css");
}

#[test]
fn single_file_header_parser_preserves_sfc_code_and_does_not_read_script_strings() {
    let script = "<script setup>const example = '<view>{\"name\":\"fake\"}</view>';</script><template>hello</template>";
    assert_eq!(
        single_file::manifest("counter.vue", script).unwrap().name,
        "counter"
    );
    let mut meta = single_file::metadata(script).unwrap();
    meta.name = Some("A </view> label".into());
    let updated = single_file::with_metadata(script, &meta).unwrap();
    assert!(updated.ends_with(script));
    assert_eq!(
        single_file::manifest("counter.vue", &updated).unwrap().name,
        "A </view> label"
    );
    assert_eq!(
        single_file::with_metadata(&updated, &meta).unwrap(),
        updated
    );
}
