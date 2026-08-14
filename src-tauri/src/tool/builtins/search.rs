use super::search_core::{run_grep, GrepConfig};
use super::{make_exec, ToolDef, ToolResult};

// ─── grep ───────────────────────────────────────────────────────────────────

pub(super) fn grep() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::GREP);
    ToolDef {
        name: "grep".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
                    Some(pattern) => pattern.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: pattern".to_string(),
                            is_error: true,
                        };
                    }
                };
                let search_path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(str::to_string);
                let search_path = match search_path {
                    Some(path) => path,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: path".to_string(),
                            is_error: true,
                        };
                    }
                };
                let include = args
                    .get("include")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);

                match run_grep(GrepConfig::production(
                    &pattern,
                    &search_path,
                    include.as_deref(),
                    ctx.working_dir.as_deref(),
                )) {
                    Ok(output) => ToolResult {
                        output,
                        is_error: false,
                    },
                    Err(output) => ToolResult {
                        output,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::grep;
    use crate::tool::ToolExecutionContext;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn grep_skips_generated_root_directories_by_default() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path().join("Assets/Scripts")).expect("create scripts");
        std::fs::create_dir_all(root.path().join("Library")).expect("create library");
        std::fs::create_dir_all(root.path().join("BuildPlayer")).expect("create build output");

        std::fs::write(
            root.path().join("Assets/Scripts/PlayerController.cs"),
            "public class PlayerController : MonoBehaviour {}",
        )
        .expect("write gameplay script");
        std::fs::write(
            root.path().join("Library/CachedBindings.cs"),
            "public class CachedBindings : MonoBehaviour {}",
        )
        .expect("write cached script");
        std::fs::write(
            root.path().join("BuildPlayer/GeneratedBootstrap.cs"),
            "public class GeneratedBootstrap : MonoBehaviour {}",
        )
        .expect("write generated build script");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (grep().execute)(
                    json!({
                        "pattern": "MonoBehaviour",
                        "path": root.path().to_string_lossy().to_string(),
                        "include": "*.cs"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("Assets/Scripts/PlayerController.cs"));
        assert!(!result.output.contains("Library/CachedBindings.cs"));
        assert!(!result.output.contains("BuildPlayer/GeneratedBootstrap.cs"));
    }

    #[test]
    fn grep_can_search_explicit_generated_directory_roots() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path()).expect("ensure dir");
        std::fs::write(
            root.path().join("CachedBindings.cs"),
            "public class CachedBindings : MonoBehaviour {}",
        )
        .expect("write cached script");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (grep().execute)(
                    json!({
                        "pattern": "MonoBehaviour",
                        "path": root.path().to_string_lossy().to_string(),
                        "include": "*.cs"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("CachedBindings.cs"));
    }

    #[test]
    fn grep_outputs_workspace_relative_paths_when_searching_subdirectory() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path().join("Assets")).expect("create assets");
        std::fs::write(
            root.path().join("Assets/PlayerPlatformerController.cs"),
            "public class PlayerPlatformerController : MonoBehaviour {}",
        )
        .expect("write script");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (grep().execute)(
                    json!({
                        "pattern": "MonoBehaviour",
                        "path": root.path().join("Assets").to_string_lossy().to_string(),
                        "include": "*.cs"
                    }),
                    ToolExecutionContext {
                        working_dir: Some(root.path().to_string_lossy().to_string()),
                        ..Default::default()
                    },
                )
                .await
            });

        assert!(!result.is_error);
        assert!(
            result
                .output
                .contains("Assets/PlayerPlatformerController.cs:"),
            "grep output should be workspace-relative, got:\n{}",
            result.output
        );
        assert!(
            !result.output.contains("\nPlayerPlatformerController.cs:"),
            "grep output must not strip paths against the search path, got:\n{}",
            result.output
        );
    }

    #[test]
    fn grep_caps_results_and_reports_truncation() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path()).expect("ensure dir");
        // 150 matching lines in a single file: must retain the first 100 by line
        // number, drop the rest, and flag truncation (but NOT early-stop, since
        // 100 < the scan budget of 1000).
        let mut body = String::new();
        for i in 0..150 {
            body.push_str(&format!("hit line {}\n", i));
        }
        std::fs::write(root.path().join("a.cs"), body).expect("write");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (grep().execute)(
                    json!({
                        "pattern": "hit",
                        "path": root.path().to_string_lossy().to_string(),
                        "include": "*.cs"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(
            result.output.contains("showing first 100"),
            "expected truncation header, got:\n{}",
            result.output
        );
        assert!(
            !result.output.contains("STOPPED EARLY"),
            "150 matches is under the scan budget; must not early-stop, got:\n{}",
            result.output
        );
        // Smallest line numbers kept (line 1 = "hit line 0"); largest dropped.
        assert!(result.output.contains("  1:hit line 0"));
        assert!(!result.output.contains("hit line 149"));
    }

    #[test]
    fn grep_stops_early_and_warns_on_very_broad_matches() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path()).expect("ensure dir");
        // 15 files x 100 matching lines = 1500 matches, well past the scan
        // budget (limit * 10 = 1000), so the walk must stop early regardless of
        // thread scheduling.
        for f in 0..15 {
            let mut body = String::new();
            for i in 0..100 {
                body.push_str(&format!("hit {} {}\n", f, i));
            }
            std::fs::write(root.path().join(format!("f{:02}.cs", f)), body).expect("write");
        }

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (grep().execute)(
                    json!({
                        "pattern": "hit",
                        "path": root.path().to_string_lossy().to_string(),
                        "include": "*.cs"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(
            result.output.contains("STOPPED EARLY"),
            "expected early-stop header, got:\n{}",
            result.output
        );
        assert!(
            result.output.contains("non-deterministic"),
            "expected uncertainty warning, got:\n{}",
            result.output
        );
    }
}
