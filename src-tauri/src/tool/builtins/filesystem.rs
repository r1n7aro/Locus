use super::misc::truncate_utf8_prefix;
use super::{make_exec, ToolDef, ToolResult};
use crate::eol::{apply_line_ending, normalize_lf, resolve_preferred_line_ending};

// ─── read ───────────────────────────────────────────────────────────────────

pub(super) fn read() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::READ);
    ToolDef {
        name: "read".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        }
                    }
                };

                let offset = args
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1)
                    .max(1) as usize;
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;

                let path = std::path::Path::new(&file_path);

                let metadata = match tokio::fs::metadata(&file_path).await {
                    Ok(m) => m,
                    Err(_) => {
                        let hint = if let Some(parent) = path.parent() {
                            if let Some(base) = path.file_name().and_then(|n| n.to_str()) {
                                let base_lower = base.to_lowercase();
                                match tokio::fs::read_dir(parent).await {
                                    Ok(mut entries) => {
                                        let mut suggestions = Vec::new();
                                        while let Ok(Some(entry)) = entries.next_entry().await {
                                            let name =
                                                entry.file_name().to_string_lossy().to_string();
                                            let name_lower = name.to_lowercase();
                                            if name_lower.contains(&base_lower)
                                                || base_lower.contains(&name_lower)
                                            {
                                                suggestions
                                                    .push(parent.join(&name).display().to_string());
                                                if suggestions.len() >= 3 {
                                                    break;
                                                }
                                            }
                                        }
                                        if suggestions.is_empty() {
                                            String::new()
                                        } else {
                                            format!("\n\nDid you mean:\n{}", suggestions.join("\n"))
                                        }
                                    }
                                    Err(_) => String::new(),
                                }
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        };
                        return ToolResult {
                            output: format!("File not found: {}{}", file_path, hint),
                            is_error: true,
                        };
                    }
                };

                if metadata.is_dir() {
                    ToolResult {
                        output: format!(
                            "Cannot read directory '{}': the read tool only reads files. Use the list tool for directories.",
                            file_path
                        ),
                        is_error: true,
                    }
                } else {
                    if ctx.should_redirect_unity_asset_read(&file_path) {
                        return ToolResult {
                            output: format!(
                                "Direct file reads are not recommended for Unity YAML asset '{}'. Use `unity_yaml_list`, `unity_yaml_search`, or `unity_yaml_read` for semantic Unity YAML access. If you still need the raw file content, repeat the same `read` call once more.",
                                file_path
                            ),
                            is_error: true,
                        };
                    }

                    if is_binary_extension(&file_path) {
                        return ToolResult {
                            output: format!("Cannot read binary file: {}", file_path),
                            is_error: true,
                        };
                    }

                    match tokio::fs::read_to_string(&file_path).await {
                        Ok(content) => {
                            let normalized_content = normalize_lf(&content);
                            let lines: Vec<&str> = normalized_content.lines().collect();
                            let total = lines.len();

                            if total < offset && !(total == 0 && offset == 1) {
                                return ToolResult {
                                    output: format!(
                                        "Offset {} is out of range (file has {} lines)",
                                        offset, total
                                    ),
                                    is_error: true,
                                };
                            }

                            let start = (offset - 1).min(total);
                            let end = (start + limit).min(total);
                            let selected = &lines[start..end];

                            let max_bytes: usize = 50 * 1024;
                            let mut bytes = 0;
                            let mut truncated_by_bytes = false;
                            let mut result_lines = Vec::new();

                            for line in selected.iter() {
                                let display = if line.len() > 2000 {
                                    format!(
                                        "{}... (line truncated to 2000 chars)",
                                        truncate_utf8_prefix(line, 2000)
                                    )
                                } else {
                                    line.to_string()
                                };
                                let line_str = display;
                                bytes += line_str.len() + 1;
                                if bytes > max_bytes {
                                    truncated_by_bytes = true;
                                    break;
                                }
                                result_lines.push(line_str);
                            }

                            let last_read_line = start + result_lines.len();
                            let has_more = end < total || truncated_by_bytes;

                            let mut output = format!("<content>\n{}", result_lines.join("\n"));

                            if truncated_by_bytes {
                                output.push_str(&format!(
                                    "\n\n(Output capped at 50KB. Showing lines {}-{}. Use offset={} to continue.)",
                                    offset, last_read_line, last_read_line + 1
                                ));
                            } else if has_more {
                                output.push_str(&format!(
                                    "\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)",
                                    offset,
                                    last_read_line,
                                    total,
                                    last_read_line + 1
                                ));
                            } else {
                                output.push_str(&format!(
                                    "\n\n(End of file — {} lines total)",
                                    total
                                ));
                            }
                            output.push_str("\n</content>");

                            ToolResult {
                                output,
                                is_error: false,
                            }
                        }
                        Err(e) => ToolResult {
                            output: format!("Failed to read file '{}': {}", file_path, e),
                            is_error: true,
                        },
                    }
                }
            })
        }),
    }
}

pub(crate) fn is_binary_extension(filepath: &str) -> bool {
    let binary_exts = [
        ".zip", ".tar", ".gz", ".exe", ".dll", ".so", ".class", ".jar", ".7z", ".bin", ".wasm",
        ".pyc", ".pdf", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".mp4", ".mp3", ".mov",
    ];
    let lower = filepath.to_lowercase();
    binary_exts.iter().any(|ext| lower.ends_with(ext))
}

// ─── write ──────────────────────────────────────────────────────────────────

pub(super) fn write() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::WRITE);
    ToolDef {
        name: "write".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        }
                    }
                };
                let content = match args.get("content").and_then(|v| v.as_str()) {
                    Some(c) => c.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: content".to_string(),
                            is_error: true,
                        }
                    }
                };

                match tokio::fs::metadata(&file_path).await {
                    Ok(metadata) => {
                        let target_kind = if metadata.is_dir() {
                            "directory"
                        } else {
                            "file"
                        };
                        return ToolResult {
                            output: format!(
                                "Path already exists: {} ({})\nUse the write tool only for new files. Use edit for existing files.",
                                file_path, target_kind
                            ),
                            is_error: true,
                        };
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return ToolResult {
                            output: format!("Failed to access path '{}': {}", file_path, error),
                            is_error: true,
                        };
                    }
                }

                if let Some(parent) = std::path::Path::new(&file_path).parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        return ToolResult {
                            output: format!("Failed to create directory: {}", e),
                            is_error: true,
                        };
                    }
                }

                match tokio::fs::write(&file_path, &content).await {
                    Ok(()) => ToolResult {
                        output: format!("Created {}", file_path),
                        is_error: false,
                    },
                    Err(e) => ToolResult {
                        output: format!("Failed to write file '{}': {}", file_path, e),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

// ─── edit ───────────────────────────────────────────────────────────────────

pub(super) fn edit() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::EDIT);
    ToolDef {
        name: "edit".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let file_path = match args.get("filePath").and_then(|v| v.as_str()) {
                    Some(p) => p.to_string(),
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: filePath".to_string(),
                            is_error: true,
                        }
                    }
                };

                let metadata = match tokio::fs::metadata(&file_path).await {
                    Ok(m) => m,
                    Err(_) => {
                        return ToolResult {
                            output: format!("File not found: {}", file_path),
                            is_error: true,
                        }
                    }
                };
                if metadata.is_dir() {
                    return ToolResult {
                        output: format!("Path is a directory: {}", file_path),
                        is_error: true,
                    };
                }

                let content = match tokio::fs::read_to_string(&file_path).await {
                    Ok(c) => c,
                    Err(e) => {
                        return ToolResult {
                            output: format!("Failed to read file '{}': {}", file_path, e),
                            is_error: true,
                        }
                    }
                };
                let file_eol = resolve_preferred_line_ending(
                    ctx.working_dir.as_deref().map(std::path::Path::new),
                    std::path::Path::new(&file_path),
                    Some(&content),
                );

                struct EditOp {
                    old_string: String,
                    new_string: String,
                    replace_all: bool,
                }

                let ops: Vec<EditOp> = if let Some(edits_arr) =
                    args.get("edits").and_then(|v| v.as_array())
                {
                    let mut ops = Vec::with_capacity(edits_arr.len());
                    for (i, edit) in edits_arr.iter().enumerate() {
                        let old_s = match edit.get("oldString").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => {
                                return ToolResult {
                                    output: format!(
                                        "edits[{}]: missing required field 'oldString'",
                                        i
                                    ),
                                    is_error: true,
                                }
                            }
                        };
                        let new_s = match edit.get("newString").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => {
                                return ToolResult {
                                    output: format!(
                                        "edits[{}]: missing required field 'newString'",
                                        i
                                    ),
                                    is_error: true,
                                }
                            }
                        };
                        let repl_all = edit
                            .get("replaceAll")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        ops.push(EditOp {
                            old_string: normalize_lf(&old_s),
                            new_string: normalize_lf(&new_s),
                            replace_all: repl_all,
                        });
                    }
                    if ops.is_empty() {
                        return ToolResult {
                            output: "edits array is empty".to_string(),
                            is_error: true,
                        };
                    }
                    ops
                } else {
                    let old_string = match args.get("oldString").and_then(|v| v.as_str()) {
                        Some(s) => s.to_string(),
                        None => {
                            return ToolResult {
                                output: "Missing required parameter: oldString (or use 'edits' array for batch mode)".to_string(),
                                is_error: true,
                            }
                        }
                    };
                    let new_string = match args.get("newString").and_then(|v| v.as_str()) {
                        Some(s) => s.to_string(),
                        None => {
                            return ToolResult {
                                output: "Missing required parameter: newString (or use 'edits' array for batch mode)".to_string(),
                                is_error: true,
                            }
                        }
                    };
                    let replace_all = args
                        .get("replaceAll")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    vec![EditOp {
                        old_string: normalize_lf(&old_string),
                        new_string: normalize_lf(&new_string),
                        replace_all,
                    }]
                };

                let mut current_content = normalize_lf(&content);
                let mut applied_count = 0;
                let mut start_lines: Vec<usize> = Vec::new();

                for (i, op) in ops.iter().enumerate() {
                    if op.old_string == op.new_string {
                        return ToolResult {
                            output: format!(
                                "Edit {}: oldString and newString are identical, no changes to apply.",
                                i + 1
                            ),
                            is_error: true,
                        };
                    }

                    if op.old_string.is_empty() {
                        let rewritten = apply_line_ending(&op.new_string, file_eol);
                        match tokio::fs::write(&file_path, rewritten).await {
                            Ok(()) => {
                                return ToolResult {
                                    output: format!("Created {}", file_path),
                                    is_error: false,
                                }
                            }
                            Err(e) => {
                                return ToolResult {
                                    output: format!("Failed to write file '{}': {}", file_path, e),
                                    is_error: true,
                                }
                            }
                        }
                    }

                    match do_replace(
                        &current_content,
                        &op.old_string,
                        &op.new_string,
                        op.replace_all,
                    ) {
                        Ok(result) => {
                            let line_no =
                                line_number_at_offset(&current_content, result.match_offset);
                            start_lines.push(line_no);
                            current_content = result.new_content;
                            applied_count += 1;
                        }
                        Err(e) => {
                            return ToolResult {
                                output: if ops.len() > 1 {
                                    format!("Edit {} of {} failed: {}", i + 1, ops.len(), e)
                                } else {
                                    e
                                },
                                is_error: true,
                            };
                        }
                    }
                }

                let rewritten = apply_line_ending(&current_content, file_eol);
                match tokio::fs::write(&file_path, rewritten).await {
                    Ok(()) => {
                        let lines_info = if !start_lines.is_empty() {
                            let nums: Vec<String> =
                                start_lines.iter().map(|n| n.to_string()).collect();
                            format!(" [lines:{}]", nums.join(","))
                        } else {
                            String::new()
                        };
                        ToolResult {
                            output: if applied_count > 1 {
                                format!(
                                    "Edited {} ({} edits applied){}",
                                    file_path, applied_count, lines_info
                                )
                            } else {
                                format!("Edited {}{}", file_path, lines_info)
                            },
                            is_error: false,
                        }
                    }
                    Err(e) => ToolResult {
                        output: format!("Failed to write file '{}': {}", file_path, e),
                        is_error: true,
                    },
                }
            })
        }),
    }
}

struct ReplaceResult {
    new_content: String,
    match_offset: usize,
}

fn do_replace(
    content: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> Result<ReplaceResult, String> {
    fn single_replace(content: &str, matched: &str, new_string: &str, pos: usize) -> ReplaceResult {
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..pos]);
        result.push_str(new_string);
        result.push_str(&content[pos + matched.len()..]);
        ReplaceResult {
            new_content: result,
            match_offset: pos,
        }
    }

    fn check_unique(content: &str, matched: &str) -> Result<usize, String> {
        let first = match content.find(matched) {
            Some(pos) => pos,
            None => {
                return Err(
                    "Internal error: fuzzy match could not be located in content.".to_string(),
                )
            }
        };
        let last = content.rfind(matched).unwrap_or(first);
        if first != last {
            return Err(
                "Found multiple matches for oldString. Provide more surrounding context to make it unique."
                    .to_string(),
            );
        }
        Ok(first)
    }

    if content.contains(old_string) {
        if replace_all {
            let offset = content.find(old_string).unwrap();
            return Ok(ReplaceResult {
                new_content: content.replace(old_string, new_string),
                match_offset: offset,
            });
        }
        let first = check_unique(content, old_string)?;
        return Ok(single_replace(content, old_string, new_string, first));
    }

    if let Some(matched) = line_trimmed_match(content, old_string) {
        if replace_all {
            let offset = content.find(&matched).unwrap_or(0);
            return Ok(ReplaceResult {
                new_content: content.replace(&matched, new_string),
                match_offset: offset,
            });
        }
        let first = check_unique(content, &matched)?;
        return Ok(single_replace(content, &matched, new_string, first));
    }

    if let Some(matched) = whitespace_normalized_match(content, old_string) {
        if replace_all {
            let offset = content.find(&matched).unwrap_or(0);
            return Ok(ReplaceResult {
                new_content: content.replace(&matched, new_string),
                match_offset: offset,
            });
        }
        let first = check_unique(content, &matched)?;
        return Ok(single_replace(content, &matched, new_string, first));
    }

    let trimmed = old_string.trim();
    if trimmed != old_string && content.contains(trimmed) {
        if replace_all {
            let offset = content.find(trimmed).unwrap();
            return Ok(ReplaceResult {
                new_content: content.replace(trimmed, new_string),
                match_offset: offset,
            });
        }
        let first = check_unique(content, trimmed)?;
        return Ok(single_replace(content, trimmed, new_string, first));
    }

    Err(
        "Could not find oldString in the file. It must match exactly, including whitespace, indentation, and line endings."
            .to_string(),
    )
}

fn line_number_at_offset(content: &str, offset: usize) -> usize {
    content[..offset].matches('\n').count() + 1
}

fn line_trimmed_match(content: &str, find: &str) -> Option<String> {
    let line_sep = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let orig: Vec<&str> = content.lines().collect();
    let mut search: Vec<&str> = find.lines().collect();
    if search.last().map(|l| l.is_empty()).unwrap_or(false) {
        search.pop();
    }
    if search.is_empty() {
        return None;
    }
    for i in 0..=orig.len().saturating_sub(search.len()) {
        if search.iter().enumerate().all(|(j, l)| {
            orig.get(i + j)
                .map(|o| o.trim() == l.trim())
                .unwrap_or(false)
        }) {
            let matched: Vec<&str> = orig[i..i + search.len()].to_vec();
            return Some(matched.join(line_sep));
        }
    }
    None
}

fn whitespace_normalized_match(content: &str, find: &str) -> Option<String> {
    let normalize = |t: &str| -> String { t.split_whitespace().collect::<Vec<&str>>().join(" ") };
    let nf = normalize(find);

    for line in content.lines() {
        if normalize(line) == nf {
            return Some(line.to_string());
        }
    }

    let line_sep = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let fl: Vec<&str> = find.lines().collect();
    if fl.len() <= 1 {
        return None;
    }
    let cl: Vec<&str> = content.lines().collect();
    for i in 0..=cl.len().saturating_sub(fl.len()) {
        let block = cl[i..i + fl.len()].join(line_sep);
        if normalize(&block) == nf {
            return Some(block);
        }
    }
    None
}

// ─── list ───────────────────────────────────────────────────────────────────

pub(super) fn list() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::LIST);
    ToolDef {
        name: "list".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let root_path = args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string());
                let root_path = match root_path {
                    Some(path) => path,
                    None => {
                        return ToolResult {
                            output: "Missing required parameter: path".to_string(),
                            is_error: true,
                        }
                    }
                };

                let max_depth = args
                    .get("depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2)
                    .min(5) as usize;

                let max_items = args
                    .get("max_items")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(500)
                    .min(1000) as usize;

                let max_total = args
                    .get("max_total")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1000)
                    .min(5000) as usize;

                let include_files = args
                    .get("include_files")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let root = std::path::PathBuf::from(&root_path);
                if !root.is_dir() {
                    return ToolResult {
                        output: format!("Directory not found: {}", root_path),
                        is_error: true,
                    };
                }

                let mut output = String::new();
                let mut total_count: usize = 0;
                list_dir_recursive(
                    &root,
                    &root,
                    0,
                    max_depth,
                    max_items,
                    max_total,
                    include_files,
                    &mut total_count,
                    &mut output,
                );

                if output.is_empty() {
                    output = "(empty directory)".to_string();
                }

                ToolResult {
                    output,
                    is_error: false,
                }
            })
        }),
    }
}

fn list_dir_recursive(
    base: &std::path::Path,
    dir: &std::path::Path,
    current_depth: usize,
    max_depth: usize,
    max_items: usize,
    max_total: usize,
    include_files: bool,
    total_count: &mut usize,
    output: &mut String,
) {
    if *total_count >= max_total {
        return;
    }

    let mut entries: Vec<std::fs::DirEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());

    let mut dirs: Vec<std::fs::DirEntry> = Vec::new();
    let mut files: Vec<std::fs::DirEntry> = Vec::new();
    for entry in entries {
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if super::should_skip_generated_root_entry(base, &entry.path()) {
                    continue;
                }
                dirs.push(entry);
            } else if include_files {
                let name = entry.file_name();
                if !name.to_string_lossy().ends_with(".meta") {
                    files.push(entry);
                }
            }
        }
    }

    let indent = "  ".repeat(current_depth);
    let total = dirs.len() + files.len();
    let mut shown = 0;

    for entry in &dirs {
        if shown >= max_items || *total_count >= max_total {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("{}{}/", indent, name));
        shown += 1;
        *total_count += 1;

        if current_depth + 1 < max_depth {
            list_dir_recursive(
                base,
                &entry.path(),
                current_depth + 1,
                max_depth,
                max_items,
                max_total,
                include_files,
                total_count,
                output,
            );
        }
    }

    for entry in &files {
        if shown >= max_items || *total_count >= max_total {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("{}{}", indent, name));
        shown += 1;
        *total_count += 1;
    }

    let actually_shown = shown;
    if total > actually_shown {
        if !output.is_empty() {
            output.push('\n');
        }
        if *total_count >= max_total {
            output.push_str(&format!(
                "{}... (total limit reached, {} more in this dir)",
                indent,
                total - actually_shown
            ));
        } else {
            output.push_str(&format!(
                "{}... and {} more",
                indent,
                total - actually_shown
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{edit, list, read, write};
    use crate::process_util::command;
    use crate::tool::ToolExecutionContext;
    use serde_json::json;
    use tempfile::tempdir;

    fn git(cwd: &std::path::Path, args: &[&str]) {
        let output = command("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn write_creates_new_file_when_path_does_not_exist() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("Assets/Scripts/NewFile.cs");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "public class NewFile {}\n"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("Created"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read created file"),
            "public class NewFile {}\n"
        );
    }

    #[test]
    fn write_rejects_existing_file_paths() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("existing.txt");
        std::fs::write(&target, "before").expect("seed existing file");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "after"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("Path already exists"));
        assert!(result.output.contains("Use edit for existing files"));
        assert_eq!(
            std::fs::read_to_string(&target).expect("read untouched file"),
            "before"
        );
    }

    #[test]
    fn write_rejects_existing_directory_paths() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("existing-dir");
        std::fs::create_dir_all(&target).expect("create existing dir");
        let target_str = target.to_string_lossy().to_string();

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (write().execute)(
                    json!({
                        "filePath": target_str,
                        "content": "after"
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(result.is_error);
        assert!(result.output.contains("Path already exists"));
        assert!(result.output.contains("(directory)"));
    }

    #[test]
    fn list_skips_generated_root_directories_by_default() {
        let root = tempdir().expect("temp dir");
        std::fs::create_dir_all(root.path().join("Assets/Scripts")).expect("create scripts");
        std::fs::create_dir_all(root.path().join("Library")).expect("create library");
        std::fs::create_dir_all(root.path().join("BuildPlayer")).expect("create build output");

        std::fs::write(
            root.path().join("Assets/Scripts/PlayerController.cs"),
            "public class PlayerController : MonoBehaviour {}",
        )
        .expect("write gameplay script");
        std::fs::write(root.path().join("Library/cache.db"), "cached").expect("write cache");
        std::fs::write(root.path().join("BuildPlayer/game.exe"), "binary").expect("write build");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (list().execute)(
                    json!({
                        "path": root.path().to_string_lossy().to_string(),
                        "depth": 3,
                        "include_files": true
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("Assets/"));
        assert!(result.output.contains("PlayerController.cs"));
        assert!(!result.output.contains("Library/"));
        assert!(!result.output.contains("BuildPlayer/"));
    }

    #[test]
    fn list_can_browse_explicit_generated_directory_roots() {
        let root = tempdir().expect("temp dir");
        std::fs::write(root.path().join("cache.db"), "cached").expect("write cache");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (list().execute)(
                    json!({
                        "path": root.path().to_string_lossy().to_string(),
                        "depth": 2,
                        "include_files": true
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("cache.db"));
    }

    #[test]
    fn read_normalizes_crlf_content_to_lf_output() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("crlf.txt");
        std::fs::write(&target, "alpha\r\nbeta\r\n").expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (read().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "offset": 1,
                        "limit": 20
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error);
        assert!(result.output.contains("<content>\nalpha\nbeta"));
        assert!(!result.output.contains('\r'));
    }

    #[test]
    fn edit_accepts_lf_old_string_for_crlf_file_and_preserves_crlf() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("player.cs");
        std::fs::write(
            &target,
            "class Player\r\n{\r\n    void Fire()\r\n    {\r\n        Shoot();\r\n    }\r\n}\r\n",
        )
        .expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "    void Fire()\n    {\n        Shoot();\n    }\n",
                        "newString": "    void Fire()\n    {\n        Shoot();\n        Reload();\n    }\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            "class Player\r\n{\r\n    void Fire()\r\n    {\r\n        Shoot();\r\n        Reload();\r\n    }\r\n}\r\n"
        );
    }

    #[test]
    fn edit_normalizes_mixed_eol_file_to_preferred_style() {
        let root = tempdir().expect("temp dir");
        let target = root.path().join("mixed.txt");
        std::fs::write(&target, "alpha\r\nbeta\ngamma\r\n").expect("seed mixed eol file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "alpha\nbeta\ngamma\n",
                        "newString": "alpha\nbeta\ndelta\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext::default(),
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read_to_string(&target).expect("read edited file"),
            "alpha\r\nbeta\r\ndelta\r\n"
        );
    }

    #[test]
    fn edit_prefers_repo_eol_rule_over_current_file_style() {
        let root = tempdir().expect("temp dir");
        git(root.path(), &["init", "-b", "main"]);
        git(root.path(), &["config", "user.name", "Test User"]);
        git(root.path(), &["config", "user.email", "test@example.com"]);
        std::fs::write(root.path().join(".gitattributes"), "*.txt text eol=lf\n")
            .expect("write attributes");

        let target = root.path().join("notes.txt");
        std::fs::write(&target, "alpha\r\nbeta\r\n").expect("seed crlf file");

        let result = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                (edit().execute)(
                    json!({
                        "filePath": target.to_string_lossy().to_string(),
                        "oldString": "alpha\nbeta\n",
                        "newString": "alpha\nbeta\ngamma\n",
                        "replaceAll": false
                    }),
                    ToolExecutionContext {
                        working_dir: Some(root.path().to_string_lossy().to_string()),
                        ..ToolExecutionContext::default()
                    },
                )
                .await
            });

        assert!(!result.is_error, "{}", result.output);
        assert_eq!(
            std::fs::read(&target).expect("read edited bytes"),
            b"alpha\nbeta\ngamma\n"
        );
    }
}
