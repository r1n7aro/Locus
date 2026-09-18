//! Codex-compatible patch syntax with source-ordered matching. Filesystem
//! effects and application policies live in the builtin and agent layers.
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Change {
    Add(String),
    Delete,
    Update {
        move_to: Option<String>,
        chunks: Vec<Chunk>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct FilePatch {
    pub path: String,
    pub change: Change,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Chunk {
    context: Option<String>,
    lines: Vec<(char, String)>,
    eof: bool,
}

impl FilePatch {
    pub(crate) fn targets(&self) -> Vec<(&str, &'static str)> {
        let mut targets = vec![(
            self.path.as_str(),
            if matches!(self.change, Change::Add(_)) {
                "write"
            } else {
                "edit"
            },
        )];
        if let Change::Update {
            move_to: Some(path),
            ..
        } = &self.change
        {
            targets.push((path, "write"));
        }
        targets
    }
}

pub(crate) fn from_arguments(args: &Value) -> Result<Vec<FilePatch>, String> {
    parse(
        args.get("patch")
            .and_then(Value::as_str)
            .ok_or("Missing required parameter: patch")?,
    )
}

pub(crate) fn parse(patch: &str) -> Result<Vec<FilePatch>, String> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    if lines.first() != Some(&"*** Begin Patch") || lines.last() != Some(&"*** End Patch") {
        return Err("Patch must start with *** Begin Patch and end with *** End Patch".into());
    }
    let mut files = Vec::new();
    let mut index = 1;
    while index + 1 < lines.len() {
        let header = lines[index];
        let (kind, path) = ["Add", "Delete", "Update"]
            .into_iter()
            .find_map(|kind| {
                header
                    .strip_prefix(&format!("*** {kind} File: "))
                    .map(|path| (kind, path))
            })
            .ok_or_else(|| format!("Invalid file header at patch line {}: {header}", index + 1))?;
        if path.trim().is_empty() || path.contains('\0') {
            return Err(format!("Invalid path at patch line {}", index + 1));
        }
        index += 1;
        let change = match kind {
            "Add" => {
                let mut text = String::new();
                while index + 1 < lines.len() && lines[index].starts_with('+') {
                    text.push_str(&lines[index][1..]);
                    text.push('\n');
                    index += 1;
                }
                Change::Add(text)
            }
            "Delete" => Change::Delete,
            _ => {
                let move_to = lines[index]
                    .strip_prefix("*** Move to: ")
                    .map(str::to_owned);
                if let Some(path) = &move_to {
                    if path.trim().is_empty() || path.contains('\0') {
                        return Err("Invalid move destination".into());
                    }
                    index += 1;
                }
                let mut chunks = Vec::new();
                while index + 1 < lines.len() && !lines[index].starts_with("*** ") {
                    let mut chunk = Chunk::default();
                    if lines[index] == "@@" {
                        index += 1;
                    } else if let Some(context) = lines[index].strip_prefix("@@ ") {
                        chunk.context = Some(context.to_owned());
                        index += 1;
                    } else if !chunks.is_empty() {
                        return Err(format!("Expected @@ at patch line {}", index + 1));
                    }
                    while index + 1 < lines.len() {
                        let line = lines[index];
                        if line.starts_with("@@") || line.starts_with("*** ") {
                            break;
                        }
                        if !line.is_empty() && !matches!(line.as_bytes()[0], b' ' | b'+' | b'-') {
                            return Err(format!("Invalid change at patch line {}", index + 1));
                        }
                        let (kind, text) = if line.is_empty() {
                            (' ', "")
                        } else {
                            (line.chars().next().unwrap(), &line[1..])
                        };
                        chunk.lines.push((kind, text.to_owned()));
                        index += 1;
                    }
                    if chunk.lines.is_empty() {
                        return Err("An update chunk must contain lines".into());
                    }
                    if lines[index] == "*** End of File" {
                        chunk.eof = true;
                        index += 1;
                    }
                    let eof = chunk.eof;
                    chunks.push(chunk);
                    if eof {
                        break;
                    }
                }
                if chunks.is_empty() && move_to.is_none() {
                    return Err(format!("No changes for {path}"));
                }
                Change::Update { move_to, chunks }
            }
        };
        files.push(FilePatch {
            path: path.to_owned(),
            change,
        });
    }
    if files.is_empty() {
        return Err("Patch contains no file changes".into());
    }
    Ok(files)
}

fn normalized_punctuation(text: &str) -> String {
    text.trim()
        .chars()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{2018}'..='\u{201b}' => '\'',
            '\u{201c}'..='\u{201f}' => '"',
            '\u{a0}' | '\u{2002}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}' => ' ',
            _ => c,
        })
        .collect()
}

fn seek(lines: &[&str], pattern: &[&str], start: usize, eof: bool) -> Option<usize> {
    let last = lines.len().checked_sub(pattern.len())?;
    let start = if eof { start.max(last) } else { start };
    for level in 0..4 {
        for index in start..=last {
            if pattern
                .iter()
                .zip(&lines[index..])
                .all(|(a, b)| match level {
                    0 => a == b,
                    1 => a.trim_end() == b.trim_end(),
                    2 => a.trim() == b.trim(),
                    _ => normalized_punctuation(a) == normalized_punctuation(b),
                })
            {
                return Some(index);
            }
        }
    }
    None
}

pub(crate) fn update_content(original: &str, chunks: &[Chunk]) -> Result<String, String> {
    let normalized = crate::eol::normalize_lf(original);
    let lines: Vec<&str> = normalized.lines().collect();
    let mut cursor = 0;
    let mut output = Vec::new();
    for chunk in chunks {
        let search_start = match &chunk.context {
            Some(context) => seek(&lines, &[context], cursor, false)
                .map(|index| index + 1)
                .ok_or_else(|| format!("Failed to find context: {context}"))?,
            None => cursor,
        };
        let mut old: Vec<&str> = chunk
            .lines
            .iter()
            .filter(|(kind, _)| *kind != '+')
            .map(|(_, text)| text.as_str())
            .collect();
        let start = if old.is_empty() {
            Some(lines.len())
        } else {
            seek(&lines, &old, search_start, chunk.eof)
        };
        let (start, drop_last) = if let Some(start) = start {
            (start, false)
        } else if old.last() == Some(&"") {
            old.pop();
            (
                seek(&lines, &old, search_start, chunk.eof)
                    .ok_or_else(|| format!("Failed to find expected lines:\n{}", old.join("\n")))?,
                true,
            )
        } else {
            return Err(format!(
                "Failed to find expected lines:\n{}",
                old.join("\n")
            ));
        };
        if start < cursor {
            return Err("Patch chunks overlap or are out of source order".into());
        }
        output.extend(lines[cursor..start].iter().map(|line| (*line).to_owned()));
        let mut source_index = start;
        let dropped_old = drop_last
            .then(|| chunk.lines.iter().rposition(|(kind, _)| *kind != '+'))
            .flatten();
        let dropped_new = drop_last
            .then(|| chunk.lines.iter().rposition(|(kind, _)| *kind != '-'))
            .flatten()
            .filter(|index| chunk.lines[*index].1.is_empty());
        for (index, (kind, text)) in chunk.lines.iter().enumerate() {
            match kind {
                ' ' => {
                    if Some(index) != dropped_new {
                        output.push(
                            lines
                                .get(source_index)
                                .unwrap_or(&text.as_str())
                                .to_string(),
                        );
                    }
                    if Some(index) != dropped_old {
                        source_index += 1;
                    }
                }
                '-' => {
                    if Some(index) != dropped_old {
                        source_index += 1;
                    }
                }
                '+' if Some(index) != dropped_new => output.push(text.clone()),
                '+' => {}
                _ => unreachable!(),
            }
        }
        cursor = start + old.len();
    }
    output.extend(lines[cursor..].iter().map(|line| (*line).to_owned()));
    let mut text = output.join("\n");
    if !output.is_empty() {
        text.push('\n');
    }
    Ok(text)
}

#[cfg(test)]
#[path = "apply_patch_tests.rs"]
mod tests;
