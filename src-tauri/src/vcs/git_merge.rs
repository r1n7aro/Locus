use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::process_util::command;
use crate::vcs::undo::ChangedFile;
use crate::vcs::{Checkpoint, GitProvider, VcsProvider};

const STASH_APPLY_ABORT_REF: &str = "refs/locus/stash-apply-abort";
const STASH_APPLY_ABORT_STATE: &str = "locus/stash-apply-abort.json";

// ── Types ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictBlock {
    pub index: usize,
    /// 1-based line number where this block starts in the workspace file
    pub start_line: usize,
    /// 1-based line number where this block ends (inclusive)
    pub end_line: usize,
    /// Content from "ours" / left side (between <<<<<<< and ||||||| or =======)
    pub left_content: String,
    /// Content from "theirs" / right side (between ======= and >>>>>>>)
    pub right_content: String,
    /// Content from base (diff3 style, between ||||||| and =======). Empty if diff2.
    pub base_content: String,
    /// Label extracted from the <<<<<<< marker line (e.g. "HEAD", "Updated upstream")
    pub left_marker_label: Option<String>,
    /// Label extracted from the >>>>>>> marker line (e.g. "feature/x", "Stashed changes")
    pub right_marker_label: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThreeWayContent {
    pub base: Option<String>,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeFileInfo {
    pub conflict_code: String,
    pub semantic_label: String,
    pub workspace_text: Option<String>,
    pub workspace_matches_canonical: bool,
    pub conflict_blocks: Vec<ConflictBlock>,
    pub is_binary: bool,
    pub is_lfs: bool,
    pub is_submodule: bool,
    pub base_oid: String,
    pub left_oid: String,
    pub right_oid: String,
    /// Semantic label for the left/Stage 2 side (e.g. "Current (main)", "Rebase target (main)")
    pub left_label: String,
    /// Semantic label for the right/Stage 3 side (e.g. "Incoming (feature)", "Your commit (abc123)")
    pub right_label: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeApplyMode {
    ResolvedText { text: String },
    TakeStage { stage: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeActionKind {
    Continue,
    Skip,
    Abort,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StashApplyAbortState {
    checkpoint: Checkpoint,
}

// ── Conflict block parsing ──

/// Parser state machine for conflict marker detection.
#[derive(Debug, PartialEq)]
enum MarkerState {
    Outside,
    InLeft,
    InBase,
    InRight,
}

/// Parse conflict markers from text content.
/// Supports both diff2 (`<<<<<<<` / `=======` / `>>>>>>>`) and
/// diff3 (`<<<<<<<` / `|||||||` / `=======` / `>>>>>>>`) styles.
pub fn parse_conflict_blocks(text: &str) -> Vec<ConflictBlock> {
    let mut blocks = Vec::new();
    let mut state = MarkerState::Outside;
    let mut block_start_line: usize = 0;
    let mut left_lines: Vec<&str> = Vec::new();
    let mut base_lines: Vec<&str> = Vec::new();
    let mut right_lines: Vec<&str> = Vec::new();
    let mut left_marker_label: Option<String> = None;

    for (line_idx, line) in text.lines().enumerate() {
        let line_num = line_idx + 1; // 1-based
        let trimmed = line.trim_end();

        match state {
            MarkerState::Outside => {
                if trimmed.starts_with("<<<<<<<") {
                    state = MarkerState::InLeft;
                    block_start_line = line_num;
                    left_lines.clear();
                    base_lines.clear();
                    right_lines.clear();
                    // Extract label after "<<<<<<< "
                    let label = trimmed[7..].trim();
                    left_marker_label = if label.is_empty() {
                        None
                    } else {
                        Some(label.to_string())
                    };
                }
            }
            MarkerState::InLeft => {
                if trimmed.starts_with("|||||||") {
                    // diff3 style — switching to base section
                    state = MarkerState::InBase;
                } else if trimmed.starts_with("=======") {
                    // diff2 style — no base, switching to right
                    state = MarkerState::InRight;
                } else if trimmed.starts_with(">>>>>>>") {
                    // Malformed but handle gracefully: close the block
                    let right_label = {
                        let l = trimmed[7..].trim();
                        if l.is_empty() {
                            None
                        } else {
                            Some(l.to_string())
                        }
                    };
                    blocks.push(ConflictBlock {
                        index: blocks.len(),
                        start_line: block_start_line,
                        end_line: line_num,
                        left_content: left_lines.join("\n"),
                        right_content: String::new(),
                        base_content: String::new(),
                        left_marker_label: left_marker_label.take(),
                        right_marker_label: right_label,
                    });
                    state = MarkerState::Outside;
                } else {
                    left_lines.push(line);
                }
            }
            MarkerState::InBase => {
                if trimmed.starts_with("=======") {
                    state = MarkerState::InRight;
                } else if trimmed.starts_with(">>>>>>>") {
                    // Malformed — close
                    let right_label = {
                        let l = trimmed[7..].trim();
                        if l.is_empty() {
                            None
                        } else {
                            Some(l.to_string())
                        }
                    };
                    blocks.push(ConflictBlock {
                        index: blocks.len(),
                        start_line: block_start_line,
                        end_line: line_num,
                        left_content: left_lines.join("\n"),
                        right_content: String::new(),
                        base_content: base_lines.join("\n"),
                        left_marker_label: left_marker_label.take(),
                        right_marker_label: right_label,
                    });
                    state = MarkerState::Outside;
                } else {
                    base_lines.push(line);
                }
            }
            MarkerState::InRight => {
                if trimmed.starts_with(">>>>>>>") {
                    let right_label = {
                        let l = trimmed[7..].trim();
                        if l.is_empty() {
                            None
                        } else {
                            Some(l.to_string())
                        }
                    };
                    blocks.push(ConflictBlock {
                        index: blocks.len(),
                        start_line: block_start_line,
                        end_line: line_num,
                        left_content: left_lines.join("\n"),
                        right_content: right_lines.join("\n"),
                        base_content: base_lines.join("\n"),
                        left_marker_label: left_marker_label.take(),
                        right_marker_label: right_label,
                    });
                    state = MarkerState::Outside;
                } else {
                    right_lines.push(line);
                }
            }
        }
    }

    blocks
}

// ── Three-way content reading ──

fn git_show_stage(cwd: &str, stage_and_path: &str) -> Option<String> {
    let output = command("git")
        .args(["-c", "core.quotePath=false", "show", stage_and_path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Read the three merge stages from the index.
pub fn read_three_way(cwd: &str, path: &str) -> ThreeWayContent {
    ThreeWayContent {
        base: git_show_stage(cwd, &format!(":1:{}", path)),
        left: git_show_stage(cwd, &format!(":2:{}", path)),
        right: git_show_stage(cwd, &format!(":3:{}", path)),
    }
}

// ── Pristine detection ──

/// Check whether the workspace file is still in its pristine (unedited) state
/// by comparing the parsed conflict blocks against the stage 2/3 content.
///
/// This avoids full-text comparison issues with marker labels, EOL differences,
/// and diff3/merge style differences.
///
/// Strategy: rebuild what the workspace file *should* look like by reconstructing
/// conflict blocks from stage 2/3 content, then compare the non-marker lines
/// between workspace blocks and stage-derived blocks.
pub fn is_workspace_pristine(
    workspace_blocks: &[ConflictBlock],
    three_way: &ThreeWayContent,
) -> bool {
    // If there are no conflict blocks, we can't do structural comparison.
    if workspace_blocks.is_empty() {
        return false;
    }

    let left_full = match &three_way.left {
        Some(s) => s.as_str(),
        None => return false,
    };
    let right_full = match &three_way.right {
        Some(s) => s.as_str(),
        None => return false,
    };

    // Build line sets from stage content for precise matching.
    // We split each stage into lines and, for each workspace conflict block,
    // verify that the block content is a contiguous subsequence of stage lines
    // rather than just an arbitrary substring.
    let left_lines: Vec<&str> = left_full.lines().collect();
    let right_lines: Vec<&str> = right_full.lines().collect();

    for block in workspace_blocks {
        // Verify left side: the block's left_content lines must appear as a
        // contiguous run within stage 2 (ours) lines.
        if !block.left_content.is_empty() {
            let block_lines: Vec<&str> = block.left_content.lines().collect();
            if !contains_contiguous_run(&left_lines, &block_lines) {
                return false;
            }
        }
        // Verify right side: same check against stage 3 (theirs).
        if !block.right_content.is_empty() {
            let block_lines: Vec<&str> = block.right_content.lines().collect();
            if !contains_contiguous_run(&right_lines, &block_lines) {
                return false;
            }
        }
    }

    true
}

/// Check if `needle` lines appear as a contiguous subsequence within `haystack`,
/// comparing with trailing-whitespace-trimmed equality.
fn contains_contiguous_run(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    'outer: for start in 0..=(haystack.len() - needle.len()) {
        for (i, needle_line) in needle.iter().enumerate() {
            if haystack[start + i].trim_end() != needle_line.trim_end() {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

// ── Submodule / binary detection ──

/// Check if any stage for this path has mode 160000 (submodule).
fn is_submodule_conflict(cwd: &str, path: &str) -> bool {
    let output = command("git")
        .args(["ls-files", "-u", "--", path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Each line: "<mode> <hash> <stage>\t<path>"
    stdout.lines().any(|line| line.starts_with("160000 "))
}

/// Check if the file path has a known binary extension.
fn is_known_binary_ext(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "fbx"
            | "obj"
            | "blend"
            | "dae"
            | "3ds"
            | "max"
            | "ma"
            | "mb"
            | "png"
            | "jpg"
            | "jpeg"
            | "psd"
            | "tga"
            | "tif"
            | "tiff"
            | "exr"
            | "hdr"
            | "bmp"
            | "gif"
            | "ico"
            | "webp"
            | "wav"
            | "mp3"
            | "ogg"
            | "aif"
            | "aiff"
            | "flac"
            | "mp4"
            | "mov"
            | "avi"
            | "webm"
            | "mkv"
            | "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "dll"
            | "so"
            | "a"
            | "dylib"
            | "exe"
            | "o"
            | "lib"
            | "zip"
            | "7z"
            | "gz"
            | "rar"
            | "tar"
            | "bz2"
            | "xz"
            | "unitypackage"
            | "cubemap"
    )
}

/// Check if bytes look binary (contain null bytes in first 8KB).
fn is_binary_bytes(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(8192);
    bytes[..check_len].contains(&0)
}

// ── Verify file is still unmerged ──

fn is_file_unmerged(cwd: &str, path: &str) -> bool {
    let output = command("git")
        .args(["ls-files", "-u", "--", path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match output {
        Ok(o) if o.status.success() => !String::from_utf8_lossy(&o.stdout).trim().is_empty(),
        _ => false,
    }
}

// ── Load merge file info ──

pub fn load_merge_file_info(
    cwd: &str,
    path: &str,
    conflict_code: &str,
    base_oid: &str,
    left_oid: &str,
    right_oid: &str,
    is_lfs: bool,
) -> AppResult<MergeFileInfo> {
    let is_submodule = is_submodule_conflict(cwd, path);
    let is_binary = is_known_binary_ext(path) || is_submodule;

    // Read workspace file
    let full_path = std::path::Path::new(cwd).join(path);
    let workspace_text = if is_binary {
        None
    } else {
        match std::fs::read(&full_path) {
            Ok(bytes) => {
                if is_binary_bytes(&bytes) {
                    // Detected as binary at runtime — update flag and skip text processing
                    let (left_label, right_label) = crate::commands::resolve_merge_side_labels(cwd);
                    return Ok(MergeFileInfo {
                        conflict_code: conflict_code.to_string(),
                        semantic_label: crate::commands::conflict_semantic_label(conflict_code)
                            .to_string(),
                        workspace_text: None,
                        workspace_matches_canonical: false,
                        conflict_blocks: vec![],
                        is_binary: true,
                        is_lfs,
                        is_submodule,
                        base_oid: base_oid.to_string(),
                        left_oid: left_oid.to_string(),
                        right_oid: right_oid.to_string(),
                        left_label,
                        right_label,
                    });
                }
                Some(String::from_utf8_lossy(&bytes).to_string())
            }
            Err(_) => None, // File deleted from disk
        }
    };

    // Parse conflict blocks and check pristine state
    let (conflict_blocks, workspace_matches_canonical) = if let Some(ref text) = workspace_text {
        let blocks = parse_conflict_blocks(text);
        if blocks.is_empty() {
            (blocks, false)
        } else {
            let three_way = read_three_way(cwd, path);
            let pristine = is_workspace_pristine(&blocks, &three_way);
            (blocks, pristine)
        }
    } else {
        (vec![], false)
    };

    // 4-layer side label resolution:
    // Layer 1: sentinel files (authority)
    let (mut left_label, mut right_label) = crate::commands::resolve_merge_side_labels(cwd);
    // Layer 2: if Layer 1 returned fallback ("Ours"/"Theirs") and we have pristine conflict
    // markers, try to extract labels from the first block's marker text.
    if left_label == "Ours"
        && right_label == "Theirs"
        && workspace_matches_canonical
        && !conflict_blocks.is_empty()
    {
        let first = &conflict_blocks[0];
        if let (Some(ll), Some(rl)) = (&first.left_marker_label, &first.right_marker_label) {
            if !ll.is_empty() && !rl.is_empty() {
                left_label = ll.clone();
                right_label = rl.clone();
            }
        }
    }
    // Layer 3: app hint — deferred, not implemented yet.
    // Layer 4: fallback already set ("Ours" / "Theirs").

    Ok(MergeFileInfo {
        conflict_code: conflict_code.to_string(),
        semantic_label: crate::commands::conflict_semantic_label(conflict_code).to_string(),
        workspace_text,
        workspace_matches_canonical,
        conflict_blocks,
        is_binary,
        is_lfs,
        is_submodule,
        base_oid: base_oid.to_string(),
        left_oid: left_oid.to_string(),
        right_oid: right_oid.to_string(),
        left_label,
        right_label,
    })
}

// ── Apply merge resolution ──

pub fn apply_merge_resolution(cwd: &str, path: &str, mode: MergeApplyMode) -> AppResult<()> {
    // Sanitize path: reject any path traversal attempts
    if path.contains("..") || std::path::Path::new(path).is_absolute() {
        return Err(AppError::new(
            "merge.invalid_path",
            "Path must be relative and cannot contain '..' components",
        ));
    }

    // Verify file is still unmerged before applying
    if !is_file_unmerged(cwd, path) {
        return Err(AppError::new(
            "merge.already_resolved",
            format!("File '{}' is no longer in a conflicted state", path),
        ));
    }

    match mode {
        MergeApplyMode::ResolvedText { text } => {
            let full_path = std::path::Path::new(cwd).join(path);
            std::fs::write(&full_path, &text).map_err(|e| {
                AppError::new("merge.write_failed", format!("Failed to write file: {}", e))
            })?;
            run_git_add(cwd, path)?;
        }
        MergeApplyMode::TakeStage { stage } => match stage.as_str() {
            "left" => {
                run_git(cwd, &["checkout", "--ours", "--", path], "checkout --ours")?;
                run_git_add(cwd, path)?;
            }
            "right" => {
                run_git(
                    cwd,
                    &["checkout", "--theirs", "--", path],
                    "checkout --theirs",
                )?;
                run_git_add(cwd, path)?;
            }
            "base" => {
                // Read stage 1 content and write to file
                let content = git_show_stage(cwd, &format!(":1:{}", path)).ok_or_else(|| {
                    AppError::new(
                        "merge.no_base",
                        "Base version (stage 1) not available for this conflict",
                    )
                })?;
                let full_path = std::path::Path::new(cwd).join(path);
                std::fs::write(&full_path, &content).map_err(|e| {
                    AppError::new("merge.write_failed", format!("Failed to write file: {}", e))
                })?;
                run_git_add(cwd, path)?;
            }
            "delete" => {
                run_git(cwd, &["rm", "--", path], "rm")?;
            }
            _ => {
                return Err(AppError::new(
                    "merge.invalid_stage",
                    format!(
                        "Unknown stage '{}'. Expected: left, right, base, delete",
                        stage
                    ),
                ));
            }
        },
    }

    Ok(())
}

// ── Execute merge action (continue / skip / abort) ──

fn stash_apply_abort_state_path(cwd: &str) -> Option<PathBuf> {
    let git = crate::commands::git_dir(cwd)?;
    Some(std::path::Path::new(&git).join(STASH_APPLY_ABORT_STATE))
}

fn load_stash_apply_abort_state(cwd: &str) -> Option<StashApplyAbortState> {
    let path = stash_apply_abort_state_path(cwd)?;
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<StashApplyAbortState>(&content).ok()
}

pub fn has_stash_apply_abort_state(cwd: &str) -> bool {
    load_stash_apply_abort_state(cwd).is_some()
}

pub fn clear_stash_apply_abort_state(cwd: &str) {
    if let Some(path) = stash_apply_abort_state_path(cwd) {
        let _ = std::fs::remove_file(path);
    }
    let _ = run_git(
        cwd,
        &["update-ref", "-d", STASH_APPLY_ABORT_REF],
        "clear stash abort ref",
    );
}

pub async fn prepare_stash_apply_abort_state(
    cwd: &str,
    label: &str,
) -> AppResult<Option<Checkpoint>> {
    clear_stash_apply_abort_state(cwd);

    let provider = GitProvider;
    let checkpoint = provider
        .checkpoint(cwd, label)
        .await
        .map_err(|e| AppError::new("merge.stash_abort_checkpoint_failed", e))?;

    let Some(checkpoint) = checkpoint else {
        return Ok(None);
    };

    run_git(
        cwd,
        &["update-ref", STASH_APPLY_ABORT_REF, &checkpoint.id],
        "record stash abort ref",
    )?;

    let path = stash_apply_abort_state_path(cwd).ok_or_else(|| {
        AppError::new(
            "merge.no_git_dir",
            "Unable to resolve repository metadata directory",
        )
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::new(
                "merge.stash_abort_checkpoint_failed",
                format!("Failed to create stash abort state directory: {}", e),
            )
        })?;
    }
    let state = StashApplyAbortState {
        checkpoint: checkpoint.clone(),
    };
    let json = serde_json::to_string_pretty(&state).map_err(|e| {
        AppError::new(
            "merge.stash_abort_checkpoint_failed",
            format!("Failed to serialize stash abort state: {}", e),
        )
    })?;
    std::fs::write(&path, json).map_err(|e| {
        AppError::new(
            "merge.stash_abort_checkpoint_failed",
            format!("Failed to write stash abort state: {}", e),
        )
    })?;

    Ok(Some(checkpoint))
}

fn parse_changed_file(line: &str) -> Option<ChangedFile> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() < 2 {
        return None;
    }

    let raw_status = parts[0].trim();
    let status = raw_status
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "M".to_string());

    if status == "R" && parts.len() >= 3 {
        return Some(ChangedFile {
            status,
            path: parts[2].to_string(),
            old_path: Some(parts[1].to_string()),
        });
    }

    Some(ChangedFile {
        status,
        path: parts[1].to_string(),
        old_path: None,
    })
}

async fn abort_stash_apply_from_checkpoint(cwd: &str) -> AppResult<String> {
    let state = load_stash_apply_abort_state(cwd).ok_or_else(|| {
        AppError::new(
            "merge.stash_abort_checkpoint_missing",
            "Stash apply abort checkpoint was not found",
        )
    })?;

    let changed = GitProvider::diff_files(cwd, &state.checkpoint.id)
        .await
        .map_err(|e| AppError::new("merge.stash_abort_failed", e))?;
    let changed_files: Vec<ChangedFile> = changed
        .iter()
        .filter_map(|line| parse_changed_file(line))
        .collect();

    GitProvider::restore_files(
        cwd,
        &state.checkpoint.id,
        state.checkpoint.index_tree_id.as_deref(),
        &changed_files,
    )
    .await
    .map_err(|e| AppError::new("merge.stash_abort_failed", e))?;

    clear_stash_apply_abort_state(cwd);
    Ok(format!(
        "Aborted stash apply and restored {} changed path(s)",
        changed_files.len()
    ))
}

fn has_unmerged_entries(cwd: &str) -> bool {
    let output = command("git")
        .args(["ls-files", "-u"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };
    !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

fn abort_generic_conflict(cwd: &str) -> AppResult<String> {
    run_git(cwd, &["reset", "--merge"], "reset --merge")
}

/// Detect the current operation kind by checking sentinel files in .git dir.
/// This is independent of what the frontend claims — backend is authoritative.
fn detect_operation_kind(cwd: &str) -> Option<String> {
    let git = crate::commands::git_dir(cwd)?;
    let gp = std::path::Path::new(&git);

    if gp.join("MERGE_HEAD").exists() {
        Some("merge".into())
    } else if gp.join("CHERRY_PICK_HEAD").exists() {
        Some("cherryPick".into())
    } else if gp.join("REVERT_HEAD").exists() {
        Some("revert".into())
    } else if gp.join("rebase-merge").exists() || gp.join("rebase-apply").exists() {
        Some("rebase".into())
    } else {
        None
    }
}

pub async fn execute_merge_action(
    cwd: &str,
    action: MergeActionKind,
    _frontend_operation_kind: &str, // kept for logging but not trusted
) -> AppResult<String> {
    // Backend detects the actual operation kind
    let op_kind = match detect_operation_kind(cwd) {
        Some(kind) => kind,
        None => {
            if matches!(action, MergeActionKind::Abort) {
                if has_stash_apply_abort_state(cwd) {
                    return abort_stash_apply_from_checkpoint(cwd).await;
                }
                if has_unmerged_entries(cwd) {
                    return abort_generic_conflict(cwd);
                }
            }
            return Err(AppError::new(
                "merge.no_operation",
                "No merge, rebase, cherry-pick, revert, or stash apply operation is in progress",
            ));
        }
    };

    // Use --no-edit to accept default merge messages without opening an editor.
    // This is portable (works on Windows where `true` is not in PATH).
    let args: Vec<&str> = match (op_kind.as_str(), &action) {
        ("merge", MergeActionKind::Continue) => {
            vec!["merge", "--continue", "--no-edit"]
        }
        ("merge", MergeActionKind::Abort) => vec!["merge", "--abort"],
        ("merge", MergeActionKind::Skip) => {
            return Err(AppError::new(
                "merge.skip_not_supported",
                "Skip is not supported for merge operations",
            ));
        }

        ("cherryPick", MergeActionKind::Continue) => {
            vec!["cherry-pick", "--continue", "--no-edit"]
        }
        ("cherryPick", MergeActionKind::Abort) => vec!["cherry-pick", "--abort"],
        ("cherryPick", MergeActionKind::Skip) => {
            return Err(AppError::new(
                "merge.skip_not_supported",
                "Skip is not supported for cherry-pick operations",
            ));
        }

        ("revert", MergeActionKind::Continue) => {
            vec!["revert", "--continue", "--no-edit"]
        }
        ("revert", MergeActionKind::Abort) => vec!["revert", "--abort"],
        ("revert", MergeActionKind::Skip) => {
            return Err(AppError::new(
                "merge.skip_not_supported",
                "Skip is not supported for revert operations",
            ));
        }

        ("rebase", MergeActionKind::Continue) => {
            vec!["rebase", "--continue"]
        }
        ("rebase", MergeActionKind::Skip) => vec!["rebase", "--skip"],
        ("rebase", MergeActionKind::Abort) => vec!["rebase", "--abort"],

        _ => {
            return Err(AppError::new(
                "merge.unknown_operation",
                format!("Unknown operation kind: {}", op_kind),
            ));
        }
    };

    run_git(cwd, &args, &format!("{} {:?}", op_kind, action))
}

// ── Git helpers ──

fn run_git(cwd: &str, args: &[&str], label: &str) -> AppResult<String> {
    let output = command("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| AppError::new("merge.exec_failed", format!("git {} failed: {}", label, e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::new(
            "merge.git_error",
            format!("git {} failed: {}", label, stderr.trim()),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_git_add(cwd: &str, path: &str) -> AppResult<()> {
    run_git(cwd, &["add", "--", path], "add")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use tempfile::tempdir;

    fn git_available() -> bool {
        command("git")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = command("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn git_may_fail(cwd: &Path, args: &[&str]) -> std::process::Output {
        command("git")
            .args(args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .expect("git command should run")
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::write(path, content).expect("write file");
    }

    fn setup_repo() -> tempfile::TempDir {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);
        git(repo.path(), &["config", "core.autocrlf", "false"]);

        write_file(&repo.path().join("a.txt"), "base\n");
        write_file(&repo.path().join("b.txt"), "base b\n");
        write_file(&repo.path().join("c.txt"), "base c\n");
        git(repo.path(), &["add", "a.txt", "b.txt", "c.txt"]);
        git(repo.path(), &["commit", "-m", "init"]);
        repo
    }

    fn create_conflicting_stash(repo: &Path) {
        write_file(&repo.join("a.txt"), "stash\n");
        write_file(&repo.join("c.txt"), "stash c\n");
        git(repo, &["stash", "push", "-m", "stash changes"]);

        write_file(&repo.join("a.txt"), "current\n");
        git(repo, &["add", "a.txt"]);
        git(repo, &["commit", "-m", "current"]);
    }

    #[tokio::test]
    async fn stash_apply_abort_checkpoint_restores_staged_pre_apply_changes() {
        if !git_available() {
            return;
        }

        let repo = setup_repo();
        create_conflicting_stash(repo.path());

        write_file(&repo.path().join("b.txt"), "staged b\n");
        git(repo.path(), &["add", "b.txt"]);
        assert_eq!(git(repo.path(), &["status", "--short"]), "M  b.txt");

        prepare_stash_apply_abort_state(&repo.path().to_string_lossy(), "stash apply stash@{0}")
            .await
            .expect("prepare checkpoint");

        let apply = git_may_fail(repo.path(), &["stash", "apply", "stash@{0}"]);
        assert!(!apply.status.success(), "stash apply should conflict");
        assert_eq!(
            git(repo.path(), &["status", "--short"]),
            "UU a.txt\nM  b.txt\nM  c.txt"
        );

        execute_merge_action(
            &repo.path().to_string_lossy(),
            MergeActionKind::Abort,
            "genericConflict",
        )
        .await
        .expect("abort stash apply");

        assert_eq!(git(repo.path(), &["status", "--short"]), "M  b.txt");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("read a"),
            "current\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("b.txt")).expect("read b"),
            "staged b\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("c.txt")).expect("read c"),
            "base c\n"
        );
        assert!(!has_stash_apply_abort_state(&repo.path().to_string_lossy()));
    }

    #[tokio::test]
    async fn generic_conflict_abort_falls_back_to_reset_merge_without_checkpoint() {
        if !git_available() {
            return;
        }

        let repo = setup_repo();
        create_conflicting_stash(repo.path());

        let apply = git_may_fail(repo.path(), &["stash", "apply", "stash@{0}"]);
        assert!(!apply.status.success(), "stash apply should conflict");
        assert_eq!(
            git(repo.path(), &["status", "--short"]),
            "UU a.txt\nM  c.txt"
        );

        execute_merge_action(
            &repo.path().to_string_lossy(),
            MergeActionKind::Abort,
            "genericConflict",
        )
        .await
        .expect("abort generic conflict");

        assert_eq!(git(repo.path(), &["status", "--short"]), "");
        assert_eq!(
            std::fs::read_to_string(repo.path().join("a.txt")).expect("read a"),
            "current\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.path().join("c.txt")).expect("read c"),
            "base c\n"
        );
    }
}
