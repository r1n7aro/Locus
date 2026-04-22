use std::time::Duration;

use similar::{Algorithm, ChangeTag, TextDiff};

use super::types::{DiffHunk, DiffLine, DiffLineKind, DiffStats};

pub(crate) const MAX_TEXT_DIFF_LINES: usize = 100_000;
pub(crate) const MAX_TEXT_DIFF_BYTES: usize = 4 * 1024 * 1024;

const GUARDED_TEXT_DIFF_LINE_THRESHOLD: usize = 4_000;
const GUARDED_TEXT_DIFF_BYTE_THRESHOLD: usize = 512 * 1024;
const GUARDED_TEXT_DIFF_TIMEOUT: Duration = Duration::from_millis(750);

pub fn compute_hunks(old: &str, new: &str, context: usize) -> Vec<DiffHunk> {
    let max_lines = old.lines().count().max(new.lines().count());
    let combined_bytes = old.len().saturating_add(new.len());
    let diff = if max_lines >= GUARDED_TEXT_DIFF_LINE_THRESHOLD
        || combined_bytes >= GUARDED_TEXT_DIFF_BYTE_THRESHOLD
    {
        let mut config = TextDiff::configure();
        config.algorithm(Algorithm::Patience);
        config.timeout(GUARDED_TEXT_DIFF_TIMEOUT);
        config.diff_lines(old, new)
    } else {
        TextDiff::from_lines(old, new)
    };
    let mut hunks = Vec::new();

    for group in diff.grouped_ops(context) {
        let mut lines = Vec::new();
        let mut old_start = usize::MAX;
        let mut new_start = usize::MAX;
        let mut old_count = 0usize;
        let mut new_count = 0usize;

        for op in &group {
            for change in diff.iter_changes(op) {
                let old_idx = change.old_index();
                let new_idx = change.new_index();

                if let Some(i) = old_idx {
                    if old_start == usize::MAX {
                        old_start = i;
                    }
                    old_count = i.saturating_sub(old_start) + 1;
                }
                if let Some(i) = new_idx {
                    if new_start == usize::MAX {
                        new_start = i;
                    }
                    new_count = i.saturating_sub(new_start) + 1;
                }

                let content = change.value().to_string();
                let (kind, old_line_no, new_line_no) = match change.tag() {
                    ChangeTag::Equal => (
                        DiffLineKind::Context,
                        old_idx.map(|i| i + 1),
                        new_idx.map(|i| i + 1),
                    ),
                    ChangeTag::Delete => (DiffLineKind::Delete, old_idx.map(|i| i + 1), None),
                    ChangeTag::Insert => (DiffLineKind::Add, None, new_idx.map(|i| i + 1)),
                };

                lines.push(DiffLine {
                    kind,
                    content,
                    old_line_no,
                    new_line_no,
                });
            }
        }

        let old_start_1 = if old_start == usize::MAX {
            1
        } else {
            old_start + 1
        };
        let new_start_1 = if new_start == usize::MAX {
            1
        } else {
            new_start + 1
        };

        hunks.push(DiffHunk {
            header: format!(
                "@@ -{},{} +{},{} @@",
                old_start_1, old_count, new_start_1, new_count
            ),
            old_start: old_start_1,
            old_count,
            new_start: new_start_1,
            new_count,
            lines,
        });
    }

    hunks
}

pub(crate) fn truncate_for_preview(hunks: Vec<DiffHunk>) -> Vec<DiffHunk> {
    let mut result = Vec::new();
    let mut total_lines = 0usize;

    for mut hunk in hunks.into_iter().take(2) {
        let remaining = 120usize.saturating_sub(total_lines);
        if remaining == 0 {
            break;
        }
        if hunk.lines.len() > remaining {
            hunk.lines.truncate(remaining);
        }
        total_lines += hunk.lines.len();
        result.push(hunk);
    }

    result
}

pub(crate) fn count_stats(hunks: &[DiffHunk]) -> DiffStats {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    for hunk in hunks {
        for line in &hunk.lines {
            match line.kind {
                DiffLineKind::Add => additions += 1,
                DiffLineKind::Delete => deletions += 1,
                DiffLineKind::Context => {}
            }
        }
    }
    DiffStats {
        additions,
        deletions,
        changed_hunks: hunks.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn compute_hunks_bounds_runtime_for_large_distinct_inputs() {
        let old = (0..8_000)
            .map(|idx| format!("old-line-{idx:05}\n"))
            .collect::<String>();
        let new = (0..8_000)
            .map(|idx| format!("new-line-{idx:05}\n"))
            .collect::<String>();

        let started = std::time::Instant::now();
        let hunks = compute_hunks(&old, &new, 3);
        let elapsed = started.elapsed();

        assert!(!hunks.is_empty());
        assert!(
            elapsed < Duration::from_secs(5),
            "large distinct diff took too long: {:?}",
            elapsed
        );
    }
}
