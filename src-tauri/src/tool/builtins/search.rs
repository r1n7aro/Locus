use super::filesystem::is_binary_extension;
use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};
use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

// ─── grep ───────────────────────────────────────────────────────────────────

/// A single matched line. Ordered by `(rel_path, line_num)` so a bounded
/// max-heap can retain the *smallest* `limit` matches — i.e. "first N by path" —
/// without ever sorting (or even holding) every match in the tree.
struct GrepMatch {
    rel_path: String,
    line_num: u64,
    line_text: String,
}

impl PartialEq for GrepMatch {
    fn eq(&self, other: &Self) -> bool {
        self.line_num == other.line_num && self.rel_path == other.rel_path
    }
}
impl Eq for GrepMatch {}
impl Ord for GrepMatch {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rel_path
            .cmp(&other.rel_path)
            .then(self.line_num.cmp(&other.line_num))
    }
}
impl PartialOrd for GrepMatch {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Collects matching lines from a single file via the ripgrep search engine,
/// stopping the search of that file once `limit` matches have been seen.
struct FileSink {
    lines: Vec<(u64, String)>,
    limit: usize,
    max_line_length: usize,
    capped: bool,
}

impl Sink for FileSink {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch<'_>,
    ) -> Result<bool, std::io::Error> {
        let start = mat.line_number().unwrap_or(0);
        for (offset, raw) in mat.lines().enumerate() {
            let text = String::from_utf8_lossy(raw);
            let trimmed = text.trim();
            let display = if trimmed.len() > self.max_line_length {
                format!("{}...", truncate_utf8_prefix(trimmed, self.max_line_length))
            } else {
                trimmed.to_string()
            };
            self.lines.push((start + offset as u64, display));
            if self.lines.len() >= self.limit {
                // Reached the per-file cap. There may be more matches in this
                // file, but we stop scanning it; flag potential truncation so
                // the caller knows the per-file total is a lower bound.
                self.capped = true;
                return Ok(false);
            }
        }
        Ok(true)
    }
}

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

                // ripgrep's regex matcher — same `regex` crate syntax as before,
                // tuned for line-oriented search.
                let matcher = match RegexMatcher::new(&pattern) {
                    Ok(m) => Arc::new(m),
                    Err(e) => {
                        return ToolResult {
                            output: format!("Invalid regex pattern '{}': {}", pattern, e),
                            is_error: true,
                        };
                    }
                };

                let max_line_length: usize = 500;
                let limit: usize = 100;
                // Stop the whole walk once this many matches have been seen. The
                // heap still keeps only the smallest `limit`; the extra headroom
                // makes the retained subset a closer approximation of the true
                // first-N while bounding work on very broad patterns.
                let scan_budget = limit.saturating_mul(10);

                let mut builder = ignore::WalkBuilder::new(&search_path);
                builder
                    .hidden(true)
                    .git_ignore(true)
                    .git_global(true)
                    .git_exclude(true)
                    .follow_links(false)
                    .threads(num_cpus());

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

                // Bounded top-k: retain only the smallest `limit` matches by
                // (path, line). Never holds the whole tree's matches in memory,
                // and avoids the O(N log N) full sort the previous version did.
                let heap = Arc::new(Mutex::new(BinaryHeap::<GrepMatch>::new()));
                let total = Arc::new(AtomicUsize::new(0));
                // More matches were discarded than the heap retains (its total is
                // still exact unless `file_capped` is also set).
                let heap_overflowed = Arc::new(AtomicBool::new(false));
                // A single file hit its per-file cap, so `total` is a lower bound.
                let file_capped = Arc::new(AtomicBool::new(false));
                // The walk quit early on the scan budget before visiting every
                // file — results are an incomplete, non-deterministic subset
                // rather than the true first-N by path. Doubles as the quit flag.
                let early_stopped = Arc::new(AtomicBool::new(false));

                // Strip output paths against the workspace root, not the search
                // path, so results can be passed directly to read/edit. Files
                // outside the strip base keep absolute paths via the per-file
                // fallback below.
                let base_path = ctx
                    .working_dir
                    .as_deref()
                    .and_then(|wd| dunce::canonicalize(std::path::Path::new(wd)).ok())
                    .unwrap_or_else(|| {
                        dunce::canonicalize(std::path::Path::new(&search_path))
                            .unwrap_or_else(|_| std::path::PathBuf::from(&search_path))
                    });
                let search_root = Arc::new(std::path::PathBuf::from(&search_path));
                let base_arc = Arc::new(base_path);

                let walker = builder.build_parallel();
                walker.run(|| {
                    let matcher_ref = matcher.clone();
                    let heap_ref = heap.clone();
                    let total_ref = total.clone();
                    let heap_overflowed_ref = heap_overflowed.clone();
                    let file_capped_ref = file_capped.clone();
                    let early_ref = early_stopped.clone();
                    let base_ref = base_arc.clone();
                    let search_root_ref = search_root.clone();
                    // One streaming searcher per worker thread (Searcher is not
                    // Sync). The default (no mmap) uses a bounded roll buffer, so
                    // even huge files are never read fully into memory.
                    let mut searcher = SearcherBuilder::new()
                        .binary_detection(BinaryDetection::quit(b'\x00'))
                        .line_number(true)
                        .build();

                    Box::new(move |entry| {
                        // Global early stop: another worker reached the budget.
                        if early_ref.load(AtomicOrdering::Relaxed) {
                            return ignore::WalkState::Quit;
                        }

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

                        let mut sink = FileSink {
                            lines: Vec::new(),
                            limit,
                            max_line_length,
                            capped: false,
                        };
                        if searcher.search_path(&*matcher_ref, path, &mut sink).is_err() {
                            return ignore::WalkState::Continue;
                        }
                        if sink.lines.is_empty() {
                            return ignore::WalkState::Continue;
                        }
                        if sink.capped {
                            file_capped_ref.store(true, AtomicOrdering::Relaxed);
                        }

                        let rel = dunce::canonicalize(path)
                            .ok()
                            .and_then(|abs| {
                                abs.strip_prefix(base_ref.as_path())
                                    .ok()
                                    .map(|r| r.to_path_buf())
                            })
                            .map(|r| r.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_else(|| path.display().to_string().replace('\\', "/"));

                        let seen_before =
                            total_ref.fetch_add(sink.lines.len(), AtomicOrdering::Relaxed);
                        let seen_after = seen_before + sink.lines.len();

                        if let Ok(mut guard) = heap_ref.lock() {
                            for (line_num, line_text) in sink.lines {
                                let item = GrepMatch {
                                    rel_path: rel.clone(),
                                    line_num,
                                    line_text,
                                };
                                if guard.len() < limit {
                                    guard.push(item);
                                } else if guard.peek().map_or(false, |top| item < *top) {
                                    // A smaller (earlier) match displaces the
                                    // current largest retained one.
                                    guard.pop();
                                    guard.push(item);
                                    heap_overflowed_ref.store(true, AtomicOrdering::Relaxed);
                                } else {
                                    heap_overflowed_ref.store(true, AtomicOrdering::Relaxed);
                                }
                            }
                        }

                        // The matches from this file are already merged above, so
                        // quitting here loses nothing.
                        if seen_after >= scan_budget {
                            early_ref.store(true, AtomicOrdering::Relaxed);
                            return ignore::WalkState::Quit;
                        }

                        ignore::WalkState::Continue
                    })
                });

                let heap = match Arc::try_unwrap(heap) {
                    Ok(mutex) => mutex.into_inner().unwrap(),
                    Err(arc) => arc
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|m| GrepMatch {
                            rel_path: m.rel_path.clone(),
                            line_num: m.line_num,
                            line_text: m.line_text.clone(),
                        })
                        .collect(),
                };

                // into_sorted_vec yields ascending (path, line) order. When the
                // walk completed, this is exactly "first N by path". When the walk
                // stopped early, it is the first N by path *of the files visited*.
                let final_matches = heap.into_sorted_vec();

                if final_matches.is_empty() {
                    return ToolResult {
                        output: "No matches found".to_string(),
                        is_error: false,
                    };
                }

                let shown = final_matches.len();
                let total_seen = total.load(AtomicOrdering::Relaxed);
                let early = early_stopped.load(AtomicOrdering::Relaxed);
                let file_capped = file_capped.load(AtomicOrdering::Relaxed);
                let more_exist = early
                    || file_capped
                    || heap_overflowed.load(AtomicOrdering::Relaxed)
                    || total_seen > shown;
                // `total` undercounts only when a file was capped or the walk quit
                // early; otherwise it is the exact match count.
                let total_label = if file_capped || early {
                    format!("{}+", total_seen)
                } else {
                    total_seen.to_string()
                };

                let header = if early {
                    format!(
                        "Found {} matches; search STOPPED EARLY at the scan budget — showing {} of an incomplete subset",
                        total_label, shown
                    )
                } else if more_exist {
                    format!(
                        "Found {} matches (showing first {} by path)",
                        total_label, shown
                    )
                } else {
                    format!("Found {} matches", total_seen)
                };
                let mut out = vec![header];

                let mut current_file = String::new();
                for m in &final_matches {
                    if current_file != m.rel_path {
                        current_file = m.rel_path.clone();
                        out.push(format!("\n{}:", m.rel_path));
                    }
                    out.push(format!("  {}:{}", m.line_num, m.line_text));
                }

                if early {
                    out.push(format!(
                        "\n⚠ Incomplete & non-deterministic: the search stopped after {} matches without visiting every file, so these are NOT guaranteed to be the globally first matches by path and may differ between runs. Narrow `pattern`, `path`, or `include` (or search a more specific subdirectory) for complete, stable results.",
                        scan_budget
                    ));
                } else if more_exist {
                    out.push(format!(
                        "\n({} of {} shown — the first by path. Narrow `pattern`, `path`, or `include` to see the rest.)",
                        shown, total_label
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
