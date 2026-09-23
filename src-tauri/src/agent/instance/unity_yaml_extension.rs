use std::future::Future;

use crate::tool::ToolResult;
use crate::unity_serialized_property::property_tree::PropertyTreePath;

/// Extensions replace whole-asset tool text. Child paths always retain the
/// shared Property Tree semantics, including live unsaved values and limits.
pub(super) async fn read_with_extension<E, EF, D, DF>(
    path: &PropertyTreePath,
    args: &serde_json::Value,
    extension: E,
    default_read: D,
) -> ToolResult
where
    E: FnOnce() -> EF,
    EF: Future<Output = Result<Option<ToolResult>, String>>,
    D: FnOnce() -> DF,
    DF: Future<Output = ToolResult>,
{
    let reader = match args.get("reader") {
        None => "auto",
        Some(serde_json::Value::String(reader))
            if matches!(reader.as_str(), "auto" | "default") =>
        {
            reader
        }
        _ => {
            return ToolResult {
                output: "Invalid reader. Allowed values: auto, default.".to_string(),
                is_error: true,
            }
        }
    };
    let legacy_detail = args
        .get("detail")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let hierarchical = path.asset_path.to_ascii_lowercase().ends_with(".unity")
        || path.asset_path.to_ascii_lowercase().ends_with(".prefab");
    let note = if reader == "auto"
        && path.segments.is_empty()
        && !hierarchical
        && matches!(legacy_detail, "" | "components")
    {
        match extension().await {
            Ok(Some(mut result)) => {
                result.output = super::AgentInstance::apply_unity_property_tree_output_budget(
                    result.output,
                    args,
                );
                return result;
            }
            Ok(None) => None,
            Err(note) => Some(note),
        }
    } else {
        None
    };
    let mut result = default_read().await;
    if let Some(note) = note {
        // Reserve space for the fallback reason even when the default tree
        // already fills the tool budget. Compile diagnostics can be very long.
        eprintln!("[unity_yaml_read] {}", note);
        let limit = super::AgentInstance::unity_property_tree_output_char_limit(args);
        let mut excerpt = note.chars().take(limit / 4).collect::<String>();
        if excerpt.len() < note.len() {
            excerpt.push('…');
        }
        let note = format!("\nNote: {}\n", excerpt);
        result.output = super::AgentInstance::apply_unity_property_tree_char_limit(
            result.output,
            limit.saturating_sub(note.chars().count()),
        );
        result.output.push_str(&note);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::Cell;

    fn output(text: &str) -> ToolResult {
        ToolResult {
            output: text.to_string(),
            is_error: false,
        }
    }

    // Exercise the public tool entry with a real temporary package and YAML,
    // not just matcher/dispatcher helpers. No window or Unity process is opened.
    #[cfg(target_os = "windows")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn yaml_read_extension_real_entry_dispatches_modern_and_legacy_roots() {
        let project = tempfile::tempdir().unwrap();
        let working_dir = project.path().to_str().unwrap();
        let package = project.path().join("Locus/skills/action-reader");
        std::fs::create_dir_all(package.join("unity/Editor")).unwrap();
        std::fs::create_dir_all(project.path().join("Assets")).unwrap();
        std::fs::write(package.join("skill.json"), serde_json::json!({
            "schema": "locus.skill.v1", "id": "test-action-reader", "name": "Test Action Reader",
            "unityYamlReadExtensions": [{ "name": "action-reader", "match": { "classIds": [114] },
                "path": "unity/Editor/ActionReader.cs" }]
        }).to_string()).unwrap();
        std::fs::write(
            package.join("SKILL.md"),
            "---\nsummary: Test typed Action reads.\n---\n# Test Action Reader\n",
        )
        .unwrap();
        std::fs::write(package.join("unity/Editor/ActionReader.cs"),
            "public static class ActionReader { public static string Read(string args) { return null; } }").unwrap();
        std::fs::write(project.path().join("Assets/Action.asset"),
            "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n  m_Name: Action\n  events:\n  - amount: 7\n").unwrap();
        assert!(crate::commands::has_unity_yaml_read_extensions_for_working_dir(working_dir));
        let app = tauri::Builder::default()
            .any_thread()
            .build(tauri::generate_context!())
            .unwrap();
        let asset_db = std::sync::Arc::new(std::sync::Mutex::new(None));
        for args in [
            json!({ "path": "Assets/Action.asset" }),
            json!({ "file_path": "Assets/Action.asset" }),
        ] {
            let result = super::super::AgentInstance::execute_unity_yaml_read(
                app.handle(),
                working_dir,
                asset_db.clone(),
                &args,
            )
            .await;
            assert!(!result.is_error, "{}", result.output);
            assert!(
                result.output.contains("[source: disk YAML"),
                "{}",
                result.output
            );
            assert!(
                result
                    .output
                    .contains("Note: yaml-read extension 'action-reader'"),
                "{}",
                result.output
            );
            assert!(result.output.contains("Unity Editor is not connected"));
        }
        for args in [
            json!({ "path": "Assets/Action.asset", "reader": "default" }),
            json!({ "file_path": "Assets/Action.asset", "detail": "document" }),
            json!({ "path": "Assets/Action.asset/events/0" }),
        ] {
            let result = super::super::AgentInstance::execute_unity_yaml_read(
                app.handle(),
                working_dir,
                asset_db.clone(),
                &args,
            )
            .await;
            assert!(!result.is_error, "{}", result.output);
            assert!(
                !result.output.contains("yaml-read extension"),
                "{}",
                result.output
            );
            assert!(result.output.contains("amount"), "{}", result.output);
        }
    }

    #[tokio::test]
    async fn yaml_read_extension_replaces_root_text_before_default_read() {
        let path = PropertyTreePath::parse("", "Assets/Action.asset").unwrap();
        let result = read_with_extension(
            &path,
            &json!({}),
            || async { Ok(Some(output("typed Action reader"))) },
            || async { panic!("successful extension must skip default reads") },
        )
        .await;
        assert_eq!(result.output, "typed Action reader");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn yaml_read_extension_falls_back_once_for_reader_failures() {
        let path = PropertyTreePath::parse("", "Assets/Action.asset").unwrap();
        for reason in [
            "empty output",
            "Unity Editor is not connected",
            "compile error",
            "invoke error: reader threw",
        ] {
            for failed_default in [false, true] {
                let calls = Cell::new(0);
                let result = read_with_extension(
                    &path,
                    &json!({}),
                    || async {
                        Err(format!(
                            "yaml-read extension 'Action' (Skill package 'ecs') failed: {reason}"
                        ))
                    },
                    || async {
                        calls.set(calls.get() + 1);
                        ToolResult {
                            output: "[source: live Editor]\nDefault tree".to_string(),
                            is_error: failed_default,
                        }
                    },
                )
                .await;
                assert_eq!(calls.get(), 1);
                assert!(result
                    .output
                    .starts_with("[source: live Editor]\nDefault tree\nNote:"));
                assert!(result.output.contains(reason));
                assert!(result.output.contains("Skill package 'ecs'"));
                assert_eq!(result.is_error, failed_default);
            }
        }
    }

    #[tokio::test]
    async fn yaml_read_extension_no_match_preserves_default_output() {
        let path = PropertyTreePath::parse("", "Assets/Action.asset").unwrap();
        let result = read_with_extension(
            &path,
            &json!({}),
            || async { Ok(None) },
            || async { output("[source: disk YAML]\nDefault tree") },
        )
        .await;
        assert_eq!(result.output, "[source: disk YAML]\nDefault tree");
    }

    #[tokio::test]
    async fn yaml_read_extension_keeps_fallback_reason_within_output_budget() {
        let path = PropertyTreePath::parse("", "Assets/Action.asset").unwrap();
        let result = read_with_extension(
            &path,
            &json!({ "__round_output_char_limit": 2000 }),
            || async { Err(format!("reader failed: {}", "diagnostic ".repeat(2000))) },
            || async { output(&"tree line\n".repeat(2000)) },
        )
        .await;
        assert!(result.output.contains("Note: reader failed:"));
        assert!(result.output.chars().count() <= 2000);
    }

    #[tokio::test]
    async fn yaml_read_extension_bypasses_children_hierarchy_and_explicit_default() {
        for (input, args) in [
            ("Assets/Action.asset/events/0", json!({})),
            ("Assets/Action.asset/bakedRootMotion/140", json!({})),
            ("Assets/Scene.unity", json!({})),
            ("Assets/Hero.prefab", json!({})),
            ("Assets/Action.asset", json!({ "reader": "default" })),
            ("Assets/Action.asset", json!({ "detail": "document" })),
            (
                "Assets/Action.asset",
                json!({ "detail": "prefab_overrides" }),
            ),
        ] {
            let path = PropertyTreePath::parse("", input).unwrap();
            let result = read_with_extension(
                &path,
                &args,
                || async { panic!("extension must not intercept {input} {args}") },
                || async { output("Default tree") },
            )
            .await;
            assert_eq!(result.output, "Default tree");
        }
    }

    #[tokio::test]
    async fn yaml_read_extension_validates_reader_before_dispatch() {
        let path = PropertyTreePath::parse("", "Assets/Action.asset").unwrap();
        for args in [json!({ "reader": "typo" }), json!({ "reader": false })] {
            let result = read_with_extension(
                &path,
                &args,
                || async { panic!("invalid reader") },
                || async { panic!("invalid reader") },
            )
            .await;
            assert!(result.is_error);
        }
    }
}
