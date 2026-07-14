use std::borrow::Cow;
use std::time::Duration;

use similar::{Algorithm, ChangeTag, TextDiff};

use super::types::{DiffHunk, DiffLine, DiffLineKind, DiffStats};

pub(crate) const MAX_TEXT_DIFF_LINES: usize = 100_000;
pub(crate) const MAX_TEXT_DIFF_BYTES: usize = 4 * 1024 * 1024;

const GUARDED_TEXT_DIFF_LINE_THRESHOLD: usize = 4_000;
const GUARDED_TEXT_DIFF_BYTE_THRESHOLD: usize = 512 * 1024;
const GUARDED_TEXT_DIFF_TIMEOUT: Duration = Duration::from_millis(750);

fn normalize_for_text_diff(text: &str) -> Cow<'_, str> {
    if text.as_bytes().contains(&b'\r') {
        Cow::Owned(crate::eol::normalize_lf(text))
    } else {
        Cow::Borrowed(text)
    }
}

/// Compute diff hunks between two text blobs.
///
/// `old_start_offset` and `new_start_offset` (both 1-based) shift the
/// reported line numbers so callers can map the relative `old`/`new` line
/// indices back to a surrounding document (e.g. an edit tool's preview,
/// where `old`/`new` are the diffed snippet and the offset is the snippet's
/// first source-file line number). Pass `None` to keep the legacy 1-based
/// behaviour where line numbers are relative to each input.
pub fn compute_hunks(
    old: &str,
    new: &str,
    context: usize,
    old_start_offset: Option<usize>,
    new_start_offset: Option<usize>,
) -> Vec<DiffHunk> {
    let old = normalize_for_text_diff(old);
    let new = normalize_for_text_diff(new);
    let old = old.as_ref();
    let new = new.as_ref();
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
    let old_offset = old_start_offset.unwrap_or(1);
    let new_offset = new_start_offset.unwrap_or(1);
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
                        old_idx.map(|i| i + old_offset),
                        new_idx.map(|i| i + new_offset),
                    ),
                    ChangeTag::Delete => (DiffLineKind::Delete, old_idx.map(|i| i + old_offset), None),
                    ChangeTag::Insert => (DiffLineKind::Add, None, new_idx.map(|i| i + new_offset)),
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
            old_offset
        } else {
            old_start + old_offset
        };
        let new_start_1 = if new_start == usize::MAX {
            new_offset
        } else {
            new_start + new_offset
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
    fn compute_hunks_ignores_line_ending_only_changes() {
        let hunks = compute_hunks("alpha\r\nbeta\r\ngamma\r\n", "alpha\nbeta\ngamma\n", 3, None, None);

        assert!(hunks.is_empty());
    }

    #[test]
    fn compute_hunks_keeps_content_changes_when_line_endings_differ() {
        let hunks = compute_hunks("alpha\r\nbeta\r\ngamma\r\n", "alpha\nBETA\ngamma\n", 3, None, None);
        let stats = count_stats(&hunks);
        let deletes: Vec<&str> = hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .filter(|line| matches!(line.kind, DiffLineKind::Delete))
            .map(|line| line.content.as_str())
            .collect();
        let adds: Vec<&str> = hunks
            .iter()
            .flat_map(|hunk| hunk.lines.iter())
            .filter(|line| matches!(line.kind, DiffLineKind::Add))
            .map(|line| line.content.as_str())
            .collect();

        assert_eq!(stats.additions, 1);
        assert_eq!(stats.deletions, 1);
        assert_eq!(deletes, vec!["beta\n"]);
        assert_eq!(adds, vec!["BETA\n"]);
    }

    #[test]
    fn compute_hunks_bounds_runtime_for_large_distinct_inputs() {
        let old = (0..8_000)
            .map(|idx| format!("old-line-{idx:05}\n"))
            .collect::<String>();
        let new = (0..8_000)
            .map(|idx| format!("new-line-{idx:05}\n"))
            .collect::<String>();

        let started = std::time::Instant::now();
        let hunks = compute_hunks(&old, &new, 3, None, None);
        let elapsed = started.elapsed();

        assert!(!hunks.is_empty());
        assert!(
            elapsed < Duration::from_secs(5),
            "large distinct diff took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn compute_hunks_shifts_line_numbers_with_offsets() {
        // Simulates the edit-tool case: old/new are a snippet that lives at
        // source lines 42-45 in the surrounding document. The reported line
        // numbers must reflect the surrounding document, not the snippet
        // (which itself starts at 1).
        let old = "alpha\nbeta\ngamma\n";
        let new = "alpha\nBETA\ngamma\n";
        let hunks = compute_hunks(old, new, 3, Some(42), Some(42));

        let hunk = hunks.first().expect("expected one hunk");
        assert_eq!(hunk.old_start, 43, "oldStart should be 42 + 1");
        assert_eq!(hunk.new_start, 43, "newStart should be 42 + 1");

        let deletes: Vec<Option<usize>> =
            hunk.lines.iter().map(|l| l.old_line_no).collect();
        let adds: Vec<Option<usize>> =
            hunk.lines.iter().map(|l| l.new_line_no).collect();

        // Old snippet lines 1..3 -> source 42..44 (alpha context, beta delete, gamma context).
        assert_eq!(deletes, vec![Some(42), Some(43), Some(44)]);
        // New snippet lines 1..3 -> source 42..44 (alpha context, BETA add, gamma context).
        assert_eq!(adds, vec![Some(42), None, Some(43), Some(44)]);
    }

    #[test]
    fn compute_hunks_one_offset_matches_legacy_one_based() {
        // `Some(1)` reproduces the legacy 1-based per-string behaviour
        // (the IPC layer maps `0` or `None` to `1` for the call site).
        let hunks_none = compute_hunks("alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n", 3, None, None);
        let hunks_one = compute_hunks("alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n", 3, Some(1), Some(1));

        assert_eq!(hunks_none.len(), hunks_one.len());
        for (a, b) in hunks_none.iter().zip(hunks_one.iter()) {
            assert_eq!(a.old_start, b.old_start);
            assert_eq!(a.new_start, b.new_start);
            let a_line_nos: Vec<_> = a
                .lines
                .iter()
                .map(|l| (l.old_line_no, l.new_line_no))
                .collect();
            let b_line_nos: Vec<_> = b
                .lines
                .iter()
                .map(|l| (l.old_line_no, l.new_line_no))
                .collect();
            assert_eq!(a_line_nos, b_line_nos);
        }
    }
}
