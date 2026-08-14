use grep_regex::RegexMatcher;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use std::collections::BinaryHeap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

pub(crate) struct GrepConfig<'a> {
    pattern: &'a str,
    search_path: &'a str,
    include: Option<&'a str>,
    working_dir: Option<&'a str>,
    max_line_length: usize,
    limit: usize,
    scan_budget: usize,
    threads: usize,
}

impl<'a> GrepConfig<'a> {
    pub(crate) fn production(
        pattern: &'a str,
        search_path: &'a str,
        include: Option<&'a str>,
        working_dir: Option<&'a str>,
    ) -> Self {
        let limit = 100usize;
        Self {
            pattern,
            search_path,
            include,
            working_dir,
            max_line_length: 500,
            limit,
            scan_budget: limit.saturating_mul(10),
            threads: available_parallelism(),
        }
    }
}

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
                self.capped = true;
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(crate) fn run_grep(config: GrepConfig<'_>) -> Result<String, String> {
    let matcher = RegexMatcher::new(config.pattern)
        .map(Arc::new)
        .map_err(|error| format!("Invalid regex pattern '{}': {}", config.pattern, error))?;

    let mut builder = ignore::WalkBuilder::new(config.search_path);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .threads(config.threads);

    let mut overrides = ignore::overrides::OverrideBuilder::new(config.search_path);
    if let Some(include) = config.include {
        for pattern in glob_pattern_to_simple(include) {
            let _ = overrides.add(&pattern);
        }
    }
    if let Ok(override_set) = overrides.build() {
        builder.overrides(override_set);
    }

    let heap = Arc::new(Mutex::new(BinaryHeap::<GrepMatch>::new()));
    let total = Arc::new(AtomicUsize::new(0));
    let heap_overflowed = Arc::new(AtomicBool::new(false));
    let file_capped = Arc::new(AtomicBool::new(false));
    let early_stopped = Arc::new(AtomicBool::new(false));

    let base_path = config
        .working_dir
        .and_then(|working_dir| dunce::canonicalize(Path::new(working_dir)).ok())
        .unwrap_or_else(|| {
            dunce::canonicalize(Path::new(config.search_path))
                .unwrap_or_else(|_| PathBuf::from(config.search_path))
        });
    let search_root_path = PathBuf::from(config.search_path);
    // Lexical stripping preserves the old canonicalized output only when the
    // supplied root is already canonical. Relative roots, `..`, symlinks and
    // case aliases keep the canonicalize fallback.
    let can_strip_lexically = dunce::canonicalize(&search_root_path)
        .ok()
        .is_some_and(|canonical| canonical == search_root_path);
    let search_root = Arc::new(search_root_path);
    let base_path = Arc::new(base_path);

    builder.build_parallel().run(|| {
        let matcher = matcher.clone();
        let heap = heap.clone();
        let total = total.clone();
        let heap_overflowed = heap_overflowed.clone();
        let file_capped = file_capped.clone();
        let early_stopped = early_stopped.clone();
        let base_path = base_path.clone();
        let search_root = search_root.clone();
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .line_number(true)
            .build();

        Box::new(move |entry| {
            if early_stopped.load(AtomicOrdering::Relaxed) {
                return ignore::WalkState::Quit;
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => return ignore::WalkState::Continue,
            };
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir());
            if should_skip_generated_root_entry(search_root.as_path(), path) {
                return if is_dir {
                    ignore::WalkState::Skip
                } else {
                    ignore::WalkState::Continue
                };
            }
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                return ignore::WalkState::Continue;
            }
            if is_binary_path(path) {
                return ignore::WalkState::Continue;
            }

            let mut sink = FileSink {
                lines: Vec::new(),
                limit: config.limit,
                max_line_length: config.max_line_length,
                capped: false,
            };
            if searcher.search_path(&*matcher, path, &mut sink).is_err() {
                return ignore::WalkState::Continue;
            }
            if sink.lines.is_empty() {
                return ignore::WalkState::Continue;
            }
            if sink.capped {
                file_capped.store(true, AtomicOrdering::Relaxed);
            }

            let relative_path = can_strip_lexically
                .then(|| {
                    path.strip_prefix(base_path.as_path())
                        .ok()
                        .map(Path::to_path_buf)
                })
                .flatten()
                .or_else(|| {
                    dunce::canonicalize(path).ok().and_then(|absolute| {
                        absolute
                            .strip_prefix(base_path.as_path())
                            .ok()
                            .map(Path::to_path_buf)
                    })
                })
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| path.display().to_string().replace('\\', "/"));

            let seen_before = total.fetch_add(sink.lines.len(), AtomicOrdering::Relaxed);
            let seen_after = seen_before + sink.lines.len();
            if let Ok(mut retained) = heap.lock() {
                for (line_num, line_text) in sink.lines {
                    let item = GrepMatch {
                        rel_path: relative_path.clone(),
                        line_num,
                        line_text,
                    };
                    if retained.len() < config.limit {
                        retained.push(item);
                    } else if retained.peek().is_some_and(|largest| item < *largest) {
                        retained.pop();
                        retained.push(item);
                        heap_overflowed.store(true, AtomicOrdering::Relaxed);
                    } else {
                        heap_overflowed.store(true, AtomicOrdering::Relaxed);
                    }
                }
            }

            if seen_after >= config.scan_budget {
                early_stopped.store(true, AtomicOrdering::Relaxed);
                return ignore::WalkState::Quit;
            }
            ignore::WalkState::Continue
        })
    });

    let heap = match Arc::try_unwrap(heap) {
        Ok(mutex) => mutex.into_inner().unwrap(),
        Err(heap) => heap
            .lock()
            .unwrap()
            .iter()
            .map(|item| GrepMatch {
                rel_path: item.rel_path.clone(),
                line_num: item.line_num,
                line_text: item.line_text.clone(),
            })
            .collect(),
    };
    let final_matches = heap.into_sorted_vec();
    if final_matches.is_empty() {
        return Ok("No matches found".to_string());
    }

    let shown = final_matches.len();
    let total_seen = total.load(AtomicOrdering::Relaxed);
    let early = early_stopped.load(AtomicOrdering::Relaxed);
    let file_was_capped = file_capped.load(AtomicOrdering::Relaxed);
    let more_exist = early
        || file_was_capped
        || heap_overflowed.load(AtomicOrdering::Relaxed)
        || total_seen > shown;
    let total_label = if file_was_capped || early {
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
    let mut output = vec![header];
    let mut current_file = String::new();
    for matched in &final_matches {
        if current_file != matched.rel_path {
            current_file = matched.rel_path.clone();
            output.push(format!("\n{}:", matched.rel_path));
        }
        output.push(format!("  {}:{}", matched.line_num, matched.line_text));
    }

    if early {
        output.push(format!(
            "\n⚠ Incomplete & non-deterministic: the search stopped after {} matches without visiting every file, so these are NOT guaranteed to be the globally first matches by path and may differ between runs. Narrow `pattern`, `path`, or `include` (or search a more specific subdirectory) for complete, stable results.",
            config.scan_budget
        ));
    } else if more_exist {
        output.push(format!(
            "\n({} of {} shown — the first by path. Narrow `pattern`, `path`, or `include` to see the rest.)",
            shown, total_label
        ));
    }

    Ok(output.join("\n"))
}

fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(4)
}

pub(super) fn should_skip_generated_root_entry(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let Some(first_component) = relative.components().next() else {
        return false;
    };
    let name = first_component.as_os_str().to_string_lossy();
    let name = name.trim();
    [
        "library",
        "temp",
        "obj",
        "logs",
        "usersettings",
        "memorycaptures",
        "recordings",
    ]
    .iter()
    .any(|generated| name.eq_ignore_ascii_case(generated))
        || name
            .get(.."build".len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("build"))
}

fn is_binary_path(path: &Path) -> bool {
    let binary_extensions = [
        "zip", "tar", "gz", "exe", "dll", "so", "class", "jar", "7z", "bin", "wasm", "pyc", "pdf",
        "png", "jpg", "jpeg", "gif", "webp", "mp4", "mp3", "mov",
    ];
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            binary_extensions
                .iter()
                .any(|binary| extension.eq_ignore_ascii_case(binary))
        })
}

fn truncate_utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        value
    } else {
        &value[..value.floor_char_boundary(max_bytes)]
    }
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
