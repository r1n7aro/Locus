use super::filesystem::is_binary_extension;
use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};

// ─── grep ───────────────────────────────────────────────────────────────────

pub(super) fn grep() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::GREP);
    ToolDef {
        name: "grep".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
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
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
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
                    .map(|s| s.to_string());

                let regex = match regex::Regex::new(&pattern) {
                    Ok(r) => r,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Invalid regex pattern '{}': {}", pattern, e),
                            is_error: true,
                        };
                    }
                };

                let max_line_length: usize = 500;
                let limit: usize = 100;

                #[derive(Clone)]
                struct Match {
                    rel_path: String,
                    line_num: usize,
                    line_text: String,
                }

                let mut builder = ignore::WalkBuilder::new(&search_path);
                builder
                    .hidden(true)
                    .git_ignore(true)
                    .git_global(true)
                    .git_exclude(true)
                    .follow_links(false)
                    .threads(num_cpus());

                let mut types_builder = ignore::types::TypesBuilder::new();
                types_builder.add_defaults();

                let mut overrides = ignore::overrides::OverrideBuilder::new(&search_path);
                if let Some(ref inc) = include {
                    let patterns = glob_pattern_to_simple(inc);
                    for p in &patterns {
                        let _ = overrides.add(p);
                    }
                }

                if let Ok(ov) = overrides.build() {
                    builder.overrides(ov);
                }

                let matches_arc = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Match>::new()));

                let base_path = dunce::canonicalize(std::path::Path::new(&search_path))
                    .unwrap_or_else(|_| std::path::PathBuf::from(&search_path));
                let search_root = std::sync::Arc::new(std::path::PathBuf::from(&search_path));

                let walker = builder.build_parallel();
                let regex_arc = std::sync::Arc::new(regex);
                let base_arc = std::sync::Arc::new(base_path);

                walker.run(|| {
                    let regex_ref = regex_arc.clone();
                    let matches_ref = matches_arc.clone();
                    let base_ref = base_arc.clone();
                    let search_root_ref = search_root.clone();

                    Box::new(move |entry| {
                        let entry = match entry {
                            Ok(e) => e,
                            Err(_) => return ignore::WalkState::Continue,
                        };

                        let path = entry.path();
                        let is_dir = entry.file_type().map_or(false, |ft| ft.is_dir());
                        if super::should_skip_generated_root_entry(search_root_ref.as_path(), path)
                        {
                            return if is_dir {
                                ignore::WalkState::Skip
                            } else {
                                ignore::WalkState::Continue
                            };
                        }

                        if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                            return ignore::WalkState::Continue;
                        }

                        if is_binary_extension(&path.display().to_string()) {
                            return ignore::WalkState::Continue;
                        }

                        let content = match std::fs::read_to_string(path) {
                            Ok(c) => c,
                            Err(_) => return ignore::WalkState::Continue,
                        };

                        let rel = dunce::canonicalize(path)
                            .ok()
                            .and_then(|abs| {
                                abs.strip_prefix(base_ref.as_path())
                                    .ok()
                                    .map(|r| r.to_path_buf())
                            })
                            .map(|r| r.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_else(|| path.display().to_string().replace('\\', "/"));

                        let mut local_matches = Vec::new();
                        for (i, line) in content.lines().enumerate() {
                            if regex_ref.is_match(line) {
                                let text = line.trim();
                                let text = if text.len() > max_line_length {
                                    format!("{}...", truncate_utf8_prefix(text, max_line_length))
                                } else {
                                    text.to_string()
                                };
                                local_matches.push(Match {
                                    rel_path: rel.clone(),
                                    line_num: i + 1,
                                    line_text: text,
                                });
                            }
                        }

                        if !local_matches.is_empty() {
                            if let Ok(mut guard) = matches_ref.lock() {
                                guard.extend(local_matches);
                            }
                        }

                        ignore::WalkState::Continue
                    })
                });

                let mut matches = match std::sync::Arc::try_unwrap(matches_arc) {
                    Ok(mutex) => mutex.into_inner().unwrap(),
                    Err(arc) => arc.lock().unwrap().clone(),
                };

                if matches.is_empty() {
                    return ToolResult {
                        output: "No matches found".to_string(),
                        is_error: false,
                    };
                }

                matches.sort_by(|a, b| {
                    a.rel_path
                        .cmp(&b.rel_path)
                        .then(a.line_num.cmp(&b.line_num))
                });

                let total = matches.len();
                let truncated = total > limit;
                let final_matches = if truncated {
                    &matches[..limit]
                } else {
                    &matches[..]
                };

                let mut out = vec![format!(
                    "Found {} matches{}",
                    total,
                    if truncated {
                        format!(" (showing first {})", limit)
                    } else {
                        String::new()
                    }
                )];

                let mut current_file = String::new();
                for m in final_matches {
                    if current_file != m.rel_path {
                        current_file = m.rel_path.clone();
                        out.push(format!("\n{}:", m.rel_path));
                    }
                    out.push(format!("  {}:{}", m.line_num, m.line_text));
                }

                if truncated {
                    out.push(format!(
                        "\n({}/{} shown, narrow pattern or path)",
                        limit, total
                    ));
                }

                ToolResult {
                    output: out.join("\n"),
                    is_error: false,
                }
            })
        }),
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

fn glob_pattern_to_simple(pattern: &str) -> Vec<String> {
    if pattern.contains('{') && pattern.contains('}') {
        if let Some(start) = pattern.find('{') {
            if let Some(end) = pattern.find('}') {
                let prefix = &pattern[..start];
                let suffix = &pattern[end + 1..];
                let inner = &pattern[start + 1..end];
                return inner
                    .split(',')
                    .map(|part| format!("{}{}{}", prefix, part.trim(), suffix))
                    .collect();
            }
        }
    }
    vec![pattern.to_string()]
}

#[allow(dead_code)]
fn matches_include(filename: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pat| {
        if let Some(ext) = pat.strip_prefix("*.") {
            filename.ends_with(&format!(".{}", ext))
        } else {
            filename == pat
        }
    })
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
}
