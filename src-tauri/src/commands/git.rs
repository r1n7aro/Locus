use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::auth::CodexAuthStateHandle;
use crate::auth::AuthState;
use crate::config::AppConfig;
use crate::error::AppError;
use crate::llm::anthropic_agent_sdk::{
    self, ClaudeCodeSdkOptions, ClaudeSdkAssistantMessage, ClaudeSdkHost, ClaudeSdkHostFuture,
};
use crate::process_util::{
    async_command, augment_path_with_git, command, discover_git_runtimes, git_env_override,
    git_is_in_path, git_runtime_key, git_version, normalize_git_path, probe_git_runtime,
    program_in_path, refresh_git_resolution, GitDiscoverySource, GitRuntimeCandidate,
};
use crate::session::models::{ChatMessage, MessageRole};
use crate::tool::ToolResult;
use crate::workspace::Workspace;
use crate::AssetDbState;
use crate::{ApiKeyState, ProviderKeysState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitInfo {
    pub hash: String,
    pub short_hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub date: i64,
    pub message: String,
    pub refs: Vec<String>,
    pub is_stash: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitLogResult {
    pub is_repo: bool,
    pub commits: Vec<GitCommitInfo>,
    pub head_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHeadKind {
    Attached,
    Detached,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHeadState {
    pub hash: Option<String>,
    pub kind: GitHeadKind,
    pub ref_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitGraphRefKind {
    LocalBranch,
    RemoteBranch,
    Tag,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitGraphRef {
    pub full_name: String,
    pub short_name: String,
    pub target_hash: String,
    pub kind: GitGraphRefKind,
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkspaceSummary {
    pub change_count: usize,
    pub unstaged_count: usize,
    pub staged_count: usize,
    pub unmerged_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistorySnapshot {
    pub is_repo: bool,
    pub commits: Vec<GitCommitInfo>,
    pub has_more: bool,
    pub head: GitHeadState,
    pub refs: Vec<GitGraphRef>,
    pub stashes: Vec<GitStashEntry>,
    pub workspace: GitWorkspaceSummary,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistorySearchRequest {
    pub query: Option<String>,
    pub use_regex: Option<bool>,
    pub author: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitHistorySearchResultKind {
    Commit,
    Stash,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistorySearchResult {
    pub kind: GitHistorySearchResultKind,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_name: Option<String>,
    pub files: Vec<GitFileChange>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHistorySearchResponse {
    pub is_repo: bool,
    pub results: Vec<GitHistorySearchResult>,
    pub truncated: bool,
}

struct VisibleHistoryPage {
    commits: Vec<GitCommitInfo>,
    has_more: bool,
}

fn build_visible_history_log_args(
    skip: usize,
    chunk_size: usize,
    include_head: bool,
) -> Vec<String> {
    let mut args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "log".to_string(),
        "--topo-order".to_string(),
        "--branches".to_string(),
        "--remotes".to_string(),
        "--tags".to_string(),
        format!("--skip={}", skip),
        format!("--max-count={}", chunk_size),
        "--pretty=format:%H%x00%h%x00%P%x00%an%x00%at%x00%s%x00".to_string(),
    ];

    if include_head {
        args.push("HEAD".to_string());
    }

    args
}

fn is_full_hex_hash(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_stash_index(ref_name: &str) -> Option<usize> {
    ref_name
        .strip_prefix("stash@{")
        .and_then(|rest| rest.strip_suffix('}'))
        .and_then(|value| value.parse::<usize>().ok())
}

fn collect_stash_root_hashes(cwd: &str) -> std::collections::HashSet<String> {
    let output = match command("git")
        .args(["stash", "list", "--format=%H"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return std::collections::HashSet::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && is_full_hex_hash(line))
        .map(|line| line.to_string())
        .collect()
}

fn collect_head_state(cwd: &str) -> GitHeadState {
    let head_hash = command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty());

    let symbolic_ref = command("git")
        .args(["symbolic-ref", "--short", "-q", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(ref_name) = symbolic_ref {
        GitHeadState {
            hash: head_hash,
            kind: GitHeadKind::Attached,
            ref_name: Some(ref_name),
        }
    } else {
        GitHeadState {
            hash: head_hash,
            kind: GitHeadKind::Detached,
            ref_name: None,
        }
    }
}

fn collect_graph_refs(cwd: &str) -> Vec<GitGraphRef> {
    let output = match command("git")
        .args([
            "for-each-ref",
            "--format=%(objectname)%00%(*objectname)%00%(refname)%00%(refname:short)%00%(HEAD)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut refs = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\0').collect();
        if parts.len() < 5 {
            continue;
        }

        let raw_target_hash = parts[0].trim();
        let peeled_target_hash = parts[1].trim();
        let full_name = parts[2].trim().to_string();
        let short_name = parts[3].trim().to_string();
        let is_current = parts[4].trim() == "*";

        if raw_target_hash.is_empty() || full_name.is_empty() || short_name.is_empty() {
            continue;
        }

        let target_hash = if full_name.starts_with("refs/tags/") && !peeled_target_hash.is_empty() {
            peeled_target_hash.to_string()
        } else {
            raw_target_hash.to_string()
        };

        if let Some(branch_name) = full_name.strip_prefix("refs/heads/") {
            refs.push(GitGraphRef {
                full_name: full_name.clone(),
                short_name: short_name.clone(),
                target_hash: target_hash.clone(),
                kind: GitGraphRefKind::LocalBranch,
                is_current,
                remote_name: None,
                branch_name: Some(branch_name.to_string()),
            });
            continue;
        }

        if let Some(remote_ref) = full_name.strip_prefix("refs/remotes/") {
            let (remote_name, branch_name) = if let Some(pos) = remote_ref.find('/') {
                (
                    Some(remote_ref[..pos].to_string()),
                    Some(remote_ref[pos + 1..].to_string()),
                )
            } else {
                (None, Some(remote_ref.to_string()))
            };
            refs.push(GitGraphRef {
                full_name: full_name.clone(),
                short_name: short_name.clone(),
                target_hash: target_hash.clone(),
                kind: GitGraphRefKind::RemoteBranch,
                is_current,
                remote_name,
                branch_name,
            });
            continue;
        }

        if full_name.starts_with("refs/tags/") {
            refs.push(GitGraphRef {
                full_name,
                short_name,
                target_hash,
                kind: GitGraphRefKind::Tag,
                is_current,
                remote_name: None,
                branch_name: None,
            });
        }
    }

    refs.sort_by(|left, right| left.short_name.cmp(&right.short_name));
    refs
}

fn push_commit_ref_label(
    map: &mut std::collections::BTreeMap<String, Vec<String>>,
    hash: &str,
    label: String,
) {
    let entry = map.entry(hash.to_string()).or_default();
    if !entry.iter().any(|existing| existing == &label) {
        entry.push(label);
    }
}

fn build_commit_ref_labels(
    refs: &[GitGraphRef],
    head: &GitHeadState,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut labels = std::collections::BTreeMap::new();

    for rf in refs {
        let label = match rf.kind {
            GitGraphRefKind::Tag => format!("tag: {}", rf.short_name),
            _ => rf.short_name.clone(),
        };
        push_commit_ref_label(&mut labels, &rf.target_hash, label);
    }

    if let Some(head_hash) = &head.hash {
        match head.kind {
            GitHeadKind::Attached => {
                if let Some(ref_name) = &head.ref_name {
                    push_commit_ref_label(&mut labels, head_hash, format!("HEAD -> {}", ref_name));
                }
            }
            GitHeadKind::Detached => {
                push_commit_ref_label(&mut labels, head_hash, "HEAD".to_string());
            }
        }
    }

    labels
}

fn resolve_commit_parent_hashes(cwd: &str, rev: &str) -> Vec<String> {
    let output = match command("git")
        .args(["rev-list", "--parents", "-n", "1", rev])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return vec![],
    };

    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if line.is_empty() {
        return vec![];
    }

    line.split_whitespace()
        .skip(1)
        .map(|value| value.to_string())
        .collect()
}

fn collect_stash_entries(cwd: &str) -> Vec<GitStashEntry> {
    let output = match command("git")
        .args([
            "stash",
            "list",
            "--format=%gd%x00%H%x00%h%x00%an%x00%at%x00%gs",
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return vec![],
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\0').collect();
        if parts.len() < 6 {
            continue;
        }

        let ref_name = parts[0].trim().to_string();
        let Some(index) = parse_stash_index(&ref_name) else {
            continue;
        };

        let hash = parts[1].trim().to_string();
        let parent_hashes = resolve_commit_parent_hashes(cwd, &hash);
        let base_hash = parent_hashes.first().cloned();

        entries.push(GitStashEntry {
            index,
            ref_name,
            hash,
            short_hash: parts[2].trim().to_string(),
            author: parts[3].trim().to_string(),
            date: parts[4].trim().parse().unwrap_or(0),
            message: parts[5].trim().to_string(),
            parent_hashes,
            base_hash,
        });
    }

    entries
}

fn collect_workspace_summary(cwd: &str) -> GitWorkspaceSummary {
    let output = match command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "-z",
            "-uall",
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => {
            return GitWorkspaceSummary {
                change_count: 0,
                unstaged_count: 0,
                staged_count: 0,
                unmerged_count: 0,
            }
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let segments: Vec<&str> = stdout.split('\0').collect();
    let mut unique_paths = std::collections::HashSet::new();
    let mut staged_count = 0usize;
    let mut unstaged_count = 0usize;
    let mut unmerged_count = 0usize;
    let mut i = 0usize;

    while i < segments.len() {
        let entry = segments[i];
        if entry.is_empty() {
            i += 1;
            continue;
        }

        match entry.as_bytes()[0] {
            b'1' => {
                let parts: Vec<&str> = entry.splitn(9, ' ').collect();
                if parts.len() >= 9 {
                    let xy = parts[1].as_bytes();
                    let x = xy.first().copied().unwrap_or(b'.') as char;
                    let y = xy.get(1).copied().unwrap_or(b'.') as char;
                    unique_paths.insert(parts[8].to_string());
                    if x != '.' && x != '?' {
                        staged_count += 1;
                    }
                    if y == '?' || y != '.' {
                        unstaged_count += 1;
                    }
                }
                i += 1;
            }
            b'2' => {
                let parts: Vec<&str> = entry.splitn(10, ' ').collect();
                if parts.len() >= 10 {
                    let xy = parts[1].as_bytes();
                    let x = xy.first().copied().unwrap_or(b'.') as char;
                    let y = xy.get(1).copied().unwrap_or(b'.') as char;
                    unique_paths.insert(parts[9].to_string());
                    if x != '.' {
                        staged_count += 1;
                    }
                    if y != '.' {
                        unstaged_count += 1;
                    }
                }
                if i + 1 < segments.len() {
                    i += 1;
                }
                i += 1;
            }
            b'u' => {
                let parts: Vec<&str> = entry.splitn(11, ' ').collect();
                if parts.len() >= 11 {
                    unique_paths.insert(parts[10].to_string());
                    unmerged_count += 1;
                }
                i += 1;
            }
            b'?' => {
                if entry.len() > 2 {
                    unique_paths.insert(entry[2..].to_string());
                    unstaged_count += 1;
                }
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    GitWorkspaceSummary {
        change_count: unique_paths.len(),
        unstaged_count,
        staged_count,
        unmerged_count,
    }
}

fn load_visible_history(
    cwd: &str,
    skip: usize,
    limit: usize,
    head_hash: Option<&str>,
    ref_labels: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<VisibleHistoryPage, AppError> {
    let stash_root_hashes = collect_stash_root_hashes(cwd);
    let stash_hidden_hashes = collect_stash_hidden_hashes(cwd, &stash_root_hashes);
    let chunk_size = std::cmp::max(limit.saturating_mul(2), 128);
    let target_visible = limit.saturating_add(1);
    let mut raw_skip = 0usize;
    let mut visible_skipped = 0usize;
    let mut commits = Vec::with_capacity(target_visible);

    'outer: loop {
        let args = build_visible_history_log_args(raw_skip, chunk_size, head_hash.is_some());

        let output = command("git")
            .args(&args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git log failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git log failed: {}", stderr).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut raw_count = 0usize;

        for record in stdout.split('\n') {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            raw_count += 1;

            let parts: Vec<&str> = record.split('\0').collect();
            if parts.len() < 6 {
                continue;
            }

            let hash = parts[0].to_string();
            if stash_root_hashes.contains(&hash) || stash_hidden_hashes.contains(&hash) {
                continue;
            }

            if visible_skipped < skip {
                visible_skipped += 1;
                continue;
            }

            let parents: Vec<String> = if parts[2].is_empty() {
                vec![]
            } else {
                parts[2]
                    .split(' ')
                    .filter(|parent| !stash_hidden_hashes.contains(*parent))
                    .map(|parent| parent.to_string())
                    .collect()
            };

            commits.push(GitCommitInfo {
                hash: hash.clone(),
                short_hash: parts[1].to_string(),
                parents,
                author: parts[3].to_string(),
                date: parts[4].parse().unwrap_or(0),
                message: parts[5].to_string(),
                refs: ref_labels.get(&hash).cloned().unwrap_or_default(),
                is_stash: false,
            });

            if commits.len() >= target_visible {
                break 'outer;
            }
        }

        if raw_count < chunk_size {
            break;
        }
        raw_skip += raw_count;
    }

    let has_more = commits.len() > limit;
    if has_more {
        commits.truncate(limit);
    }

    Ok(VisibleHistoryPage { commits, has_more })
}

#[tauri::command]
pub async fn git_history_snapshot(
    workspace: State<'_, Arc<Workspace>>,
    skip: Option<usize>,
    limit: Option<usize>,
) -> Result<GitHistorySnapshot, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(GitHistorySnapshot {
            is_repo: false,
            commits: vec![],
            has_more: false,
            head: GitHeadState {
                hash: None,
                kind: GitHeadKind::Detached,
                ref_name: None,
            },
            refs: vec![],
            stashes: vec![],
            workspace: GitWorkspaceSummary {
                change_count: 0,
                unstaged_count: 0,
                staged_count: 0,
                unmerged_count: 0,
            },
        });
    }

    let check = command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git not found: {}", e))?;

    if !check.status.success() {
        return Ok(GitHistorySnapshot {
            is_repo: false,
            commits: vec![],
            has_more: false,
            head: GitHeadState {
                hash: None,
                kind: GitHeadKind::Detached,
                ref_name: None,
            },
            refs: vec![],
            stashes: vec![],
            workspace: GitWorkspaceSummary {
                change_count: 0,
                unstaged_count: 0,
                staged_count: 0,
                unmerged_count: 0,
            },
        });
    }

    let head = collect_head_state(&cwd);
    let refs = collect_graph_refs(&cwd);
    let ref_labels = build_commit_ref_labels(&refs, &head);
    let history_page = load_visible_history(
        &cwd,
        skip.unwrap_or(0),
        limit.unwrap_or(200),
        head.hash.as_deref(),
        &ref_labels,
    )?;
    let stashes = collect_stash_entries(&cwd);
    let workspace = collect_workspace_summary(&cwd);

    Ok(GitHistorySnapshot {
        is_repo: true,
        commits: history_page.commits,
        has_more: history_page.has_more,
        head,
        refs,
        stashes,
        workspace,
    })
}

const MAX_GIT_HISTORY_SEARCH_RESULTS: usize = 1000;

#[derive(Debug, Clone)]
struct GitHistorySearchFilter {
    query_matcher: Option<GitHistorySearchQueryMatcher>,
    author: String,
    date_from: Option<i64>,
    date_to: Option<i64>,
}

#[derive(Debug, Clone)]
enum GitHistorySearchQueryMatcher {
    Contains(String),
    Regex(Regex),
}

impl GitHistorySearchFilter {
    fn from_request(request: GitHistorySearchRequest) -> Result<Self, AppError> {
        let query = request.query.unwrap_or_default().trim().to_string();
        let query_matcher = if query.is_empty() {
            None
        } else if request.use_regex.unwrap_or(false) {
            Some(GitHistorySearchQueryMatcher::Regex(
                RegexBuilder::new(&query)
                    .case_insensitive(true)
                    .build()
                    .map_err(|err| {
                        AppError::new(
                            "git.invalid_regex",
                            format!("Invalid regular expression: {}", err),
                        )
                    })?,
            ))
        } else {
            Some(GitHistorySearchQueryMatcher::Contains(query.to_lowercase()))
        };

        Ok(Self {
            query_matcher,
            author: request.author.unwrap_or_default().trim().to_lowercase(),
            date_from: request.date_from,
            date_to: request.date_to,
        })
    }

    fn has_any_filter(&self) -> bool {
        self.query_matcher.is_some()
            || !self.author.is_empty()
            || self.date_from.is_some()
            || self.date_to.is_some()
    }

    fn matches_author_date(&self, author: &str, date: i64) -> bool {
        if !self.author.is_empty() && !author.to_lowercase().contains(&self.author) {
            return false;
        }
        if let Some(date_from) = self.date_from {
            if date < date_from {
                return false;
            }
        }
        if let Some(date_to) = self.date_to {
            if date > date_to {
                return false;
            }
        }
        true
    }

    fn matches_file(&self, file: &GitFileChange) -> bool {
        match &self.query_matcher {
            None => true,
            Some(matcher) => {
                git_file_path_matches_query(&file.path, matcher)
                    || file
                        .old_path
                        .as_deref()
                        .is_some_and(|old_path| git_file_path_matches_query(old_path, matcher))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct GitHistorySearchDraft {
    hash: String,
    short_hash: String,
    author: String,
    date: i64,
    message: String,
    files: Vec<GitFileChange>,
}

fn git_file_path_matches_query(path: &str, matcher: &GitHistorySearchQueryMatcher) -> bool {
    let normalized = path.replace('\\', "/");
    let file_name = normalized.rsplit('/').next().unwrap_or(&normalized);
    match matcher {
        GitHistorySearchQueryMatcher::Contains(query) => {
            let normalized = normalized.to_lowercase();
            if normalized.contains(query) {
                return true;
            }
            normalized
                .rsplit('/')
                .next()
                .is_some_and(|file_name| file_name.contains(query))
        }
        GitHistorySearchQueryMatcher::Regex(regex) => {
            regex.is_match(&normalized) || regex.is_match(file_name)
        }
    }
}

#[cfg(test)]
mod git_history_search_filter_tests {
    use super::{
        git_file_path_matches_query, GitHistorySearchFilter, GitHistorySearchQueryMatcher,
        GitHistorySearchRequest,
    };

    #[test]
    fn plain_history_search_matches_case_insensitive_file_name() {
        let matcher = GitHistorySearchQueryMatcher::Contains("player.cs".to_string());

        assert!(git_file_path_matches_query(
            "Assets/Scripts/Player.cs",
            &matcher
        ));
    }

    #[test]
    fn regex_history_search_matches_file_name_anchor() {
        let filter = GitHistorySearchFilter::from_request(GitHistorySearchRequest {
            query: Some(r"^Player\.(cs|prefab)$".to_string()),
            use_regex: Some(true),
            author: None,
            date_from: None,
            date_to: None,
        })
        .expect("regex should compile");
        let matcher = filter.query_matcher.as_ref().expect("query matcher");

        assert!(git_file_path_matches_query(
            "Assets/Scripts/Player.cs",
            matcher
        ));
        assert!(!git_file_path_matches_query(
            "Assets/Scripts/Player.meta",
            matcher
        ));
    }

    #[test]
    fn regex_history_search_reports_invalid_pattern() {
        let error = GitHistorySearchFilter::from_request(GitHistorySearchRequest {
            query: Some("[".to_string()),
            use_regex: Some(true),
            author: None,
            date_from: None,
            date_to: None,
        })
        .expect_err("invalid regex should fail");

        assert_eq!(error.code, "git.invalid_regex");
    }
}

fn push_git_history_search_draft(
    results: &mut Vec<GitHistorySearchResult>,
    truncated: &mut bool,
    draft: Option<GitHistorySearchDraft>,
) {
    let Some(draft) = draft else {
        return;
    };
    if draft.files.is_empty() {
        return;
    }
    if results.len() >= MAX_GIT_HISTORY_SEARCH_RESULTS {
        *truncated = true;
        return;
    }
    results.push(GitHistorySearchResult {
        kind: GitHistorySearchResultKind::Commit,
        hash: draft.hash,
        short_hash: draft.short_hash,
        author: draft.author,
        date: draft.date,
        message: draft.message,
        ref_name: None,
        files: draft.files,
    });
}

fn collect_commit_history_search_results(
    cwd: &str,
    filter: &GitHistorySearchFilter,
    results: &mut Vec<GitHistorySearchResult>,
    truncated: &mut bool,
) -> Result<(), AppError> {
    let stash_root_hashes = collect_stash_root_hashes(cwd);
    let stash_hidden_hashes = collect_stash_hidden_hashes(cwd, &stash_root_hashes);
    let output = command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "log",
            "--all",
            "--topo-order",
            "--find-renames",
            "--name-status",
            "--pretty=format:%x1e%H%x00%h%x00%P%x00%an%x00%at%x00%s",
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git history search failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git history search failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut draft: Option<GitHistorySearchDraft> = None;

    for line in stdout.lines() {
        if let Some(commit_line) = line.strip_prefix('\u{1e}') {
            push_git_history_search_draft(results, truncated, draft.take());
            if *truncated {
                return Ok(());
            }

            let parts: Vec<&str> = commit_line.split('\0').collect();
            if parts.len() < 6 {
                continue;
            }
            let hash = parts[0].trim().to_string();
            if hash.is_empty()
                || stash_root_hashes.contains(&hash)
                || stash_hidden_hashes.contains(&hash)
            {
                continue;
            }

            let author = parts[3].trim().to_string();
            let date = parts[4].trim().parse().unwrap_or(0);
            if !filter.matches_author_date(&author, date) {
                continue;
            }

            draft = Some(GitHistorySearchDraft {
                hash,
                short_hash: parts[1].trim().to_string(),
                author,
                date,
                message: parts[5].trim().to_string(),
                files: Vec::new(),
            });
            continue;
        }

        let Some(current) = draft.as_mut() else {
            continue;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        for file in parse_name_status_lines(line) {
            if filter.matches_file(&file) {
                current.files.push(file);
            }
        }
    }

    push_git_history_search_draft(results, truncated, draft.take());
    Ok(())
}

fn collect_stash_history_search_results(
    cwd: &str,
    filter: &GitHistorySearchFilter,
    results: &mut Vec<GitHistorySearchResult>,
    truncated: &mut bool,
) -> Result<(), AppError> {
    for stash in collect_stash_entries(cwd) {
        if results.len() >= MAX_GIT_HISTORY_SEARCH_RESULTS {
            *truncated = true;
            return Ok(());
        }
        if !filter.matches_author_date(&stash.author, stash.date) {
            continue;
        }

        let args = build_commit_files_args(&stash.hash, true);
        let output = command("git")
            .args(&args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git stash search failed: {}", e))?;

        if !output.status.success() {
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<GitFileChange> = parse_name_status_lines(&stdout)
            .into_iter()
            .filter(|file| filter.matches_file(file))
            .collect();

        if files.is_empty() {
            continue;
        }

        results.push(GitHistorySearchResult {
            kind: GitHistorySearchResultKind::Stash,
            hash: stash.hash,
            short_hash: stash.short_hash,
            author: stash.author,
            date: stash.date,
            message: stash.message,
            ref_name: Some(stash.ref_name),
            files,
        });
    }

    Ok(())
}

#[tauri::command]
pub async fn git_history_search(
    workspace: State<'_, Arc<Workspace>>,
    request: GitHistorySearchRequest,
) -> Result<GitHistorySearchResponse, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(GitHistorySearchResponse {
            is_repo: false,
            results: vec![],
            truncated: false,
        });
    }

    let check = command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git not found: {}", e))?;

    if !check.status.success() {
        return Ok(GitHistorySearchResponse {
            is_repo: false,
            results: vec![],
            truncated: false,
        });
    }

    let filter = GitHistorySearchFilter::from_request(request)?;
    if !filter.has_any_filter() {
        return Ok(GitHistorySearchResponse {
            is_repo: true,
            results: vec![],
            truncated: false,
        });
    }

    let mut results = Vec::new();
    let mut truncated = false;
    collect_commit_history_search_results(&cwd, &filter, &mut results, &mut truncated)?;
    if !truncated {
        collect_stash_history_search_results(&cwd, &filter, &mut results, &mut truncated)?;
    }
    results.sort_by(|left, right| right.date.cmp(&left.date));

    Ok(GitHistorySearchResponse {
        is_repo: true,
        results,
        truncated,
    })
}

fn collect_stash_hidden_hashes(
    cwd: &str,
    stash_roots: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    if stash_roots.is_empty() {
        return std::collections::HashSet::new();
    }

    let mut rev_args: Vec<String> = vec!["rev-parse".into()];
    for hash in stash_roots {
        rev_args.push(format!("{}^2", hash));
        rev_args.push(format!("{}^3", hash));
    }

    let output = match command("git")
        .args(&rev_args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(_) => return std::collections::HashSet::new(),
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim())
        .filter(|line| is_full_hex_hash(line))
        .map(|line| line.to_string())
        .collect()
}

fn is_stash_revision(cwd: &str, rev: &str) -> bool {
    if rev.starts_with("stash@{") {
        return true;
    }
    collect_stash_root_hashes(cwd).contains(rev)
}

fn parse_name_status_lines(stdout: &str) -> Vec<GitFileChange> {
    let mut files = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // format: "M\tpath" or "R100\told\tnew"
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let status_char = parts[0].chars().next().unwrap_or('M').to_string();
        let (path, old_path) = if parts.len() >= 3 {
            (parts[2].to_string(), Some(parts[1].to_string()))
        } else {
            (parts[1].to_string(), None)
        };

        files.push(GitFileChange {
            path,
            old_path,
            status: status_char,
            lfs: false,
            primary_exists_in_workspace: None,
            primary_is_directory_in_workspace: None,
        });
    }
    files
}

fn build_commit_files_args(hash: &str, is_stash_revision: bool) -> Vec<String> {
    if is_stash_revision {
        vec![
            "-c".to_string(),
            "core.quotePath=false".to_string(),
            "stash".to_string(),
            "show".to_string(),
            "--find-renames".to_string(),
            "--name-status".to_string(),
            hash.to_string(),
        ]
    } else {
        vec![
            "-c".to_string(),
            "core.quotePath=false".to_string(),
            "diff-tree".to_string(),
            "--no-commit-id".to_string(),
            "-r".to_string(),
            "--find-renames".to_string(),
            "--name-status".to_string(),
            hash.to_string(),
        ]
    }
}

fn build_compare_files_args(range: &str) -> Vec<String> {
    vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "diff".to_string(),
        "--find-renames".to_string(),
        "--name-status".to_string(),
        range.to_string(),
    ]
}

struct TempGitIndex {
    path: PathBuf,
}

impl TempGitIndex {
    fn create(cwd: &str) -> Result<Self, AppError> {
        let repo_index_path = resolve_repo_index_path(cwd)?;
        let path =
            std::env::temp_dir().join(format!("locus-git-index-{}.idx", uuid::Uuid::new_v4()));

        if repo_index_path.is_file() {
            std::fs::copy(&repo_index_path, &path).map_err(|e| {
                format!(
                    "failed to copy git index '{}' -> '{}': {}",
                    repo_index_path.display(),
                    path.display(),
                    e
                )
            })?;
        } else {
            std::fs::File::create(&path).map_err(|e| {
                format!(
                    "failed to create temporary git index '{}': {}",
                    path.display(),
                    e
                )
            })?;
        }

        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn lock_path(&self) -> PathBuf {
        let mut lock = OsString::from(self.path.as_os_str());
        lock.push(".lock");
        PathBuf::from(lock)
    }
}

impl Drop for TempGitIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.lock_path());
    }
}

fn resolve_repo_index_path(cwd: &str) -> Result<PathBuf, AppError> {
    let output = command("git")
        .args(["rev-parse", "--git-path", "index"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git rev-parse --git-path index failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git rev-parse --git-path index failed: {}", stderr.trim()).into());
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Err("git rev-parse --git-path index returned empty output".into());
    }

    let path = PathBuf::from(&raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(Path::new(cwd).join(path))
    }
}

fn run_git_with_index_output(
    cwd: &str,
    args: &[&str],
    index: Option<&TempGitIndex>,
) -> Result<std::process::Output, AppError> {
    let mut cmd = command("git");
    cmd.args(args).current_dir(cwd);
    cmd.env_remove("GIT_INDEX_FILE");
    if let Some(index) = index {
        cmd.env("GIT_INDEX_FILE", index.path());
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git {} failed: {}", args.join(" "), e).into())
}

fn git_write_tree(cwd: &str, index: Option<&TempGitIndex>) -> Result<String, AppError> {
    let output = run_git_with_index_output(cwd, &["write-tree"], index)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git write-tree failed: {}", stderr.trim()).into());
    }

    let tree = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if tree.is_empty() {
        return Err("git write-tree returned empty output".into());
    }

    Ok(tree)
}

fn load_rename_aware_unstaged_changes(cwd: &str) -> Result<Vec<GitFileChange>, AppError> {
    let temp_index = TempGitIndex::create(cwd)?;
    let base_tree = git_write_tree(cwd, None)?;

    let add_output = run_git_with_index_output(cwd, &["add", "-A"], Some(&temp_index))?;
    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(format!("git add -A (temp index) failed: {}", stderr.trim()).into());
    }

    let temp_tree = git_write_tree(cwd, Some(&temp_index))?;
    if temp_tree == base_tree {
        return Ok(vec![]);
    }

    let output = command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "diff",
            "--name-status",
            "--find-renames",
            &base_tree,
            &temp_tree,
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git diff (temp index) failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff (temp index) failed: {}", stderr.trim()).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_name_status_lines(&stdout))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitProbeResult {
    pub available: bool,
    pub in_path: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_override: Option<String>,
    pub is_repo: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRuntimeInfo {
    pub id: String,
    pub label: String,
    pub path: String,
    pub version: Option<String>,
    pub source: String,
    pub selected: bool,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRuntimeState {
    pub runtimes: Vec<GitRuntimeInfo>,
    pub selected_id: Option<String>,
    pub effective: Option<GitRuntimeInfo>,
    pub missing_selected: bool,
}

const GIT_OVERRIDE_FILE: &str = "git_path_override.txt";
type GitRuntimeDiscoveryCache = Option<Vec<GitRuntimeCandidate>>;

fn git_override_path(app_handle: &AppHandle) -> Result<std::path::PathBuf, AppError> {
    let data_dir = crate::commands::resolve_runtime_storage_dir(app_handle)
        .map_err(|e| format!("Failed to get app data dir: {}", e))?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create app data dir: {}", e))?;
    Ok(data_dir.join(GIT_OVERRIDE_FILE))
}

fn read_saved_git_override(app_handle: &AppHandle) -> Option<String> {
    let Ok(path) = git_override_path(app_handle) else {
        return None;
    };

    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

pub fn restore_saved_git_override(app_handle: &AppHandle) {
    let Some(saved) = read_saved_git_override(app_handle) else {
        return;
    };

    std::env::set_var("LOCUS_GIT_PATH", saved);
    let _ = refresh_git_resolution();
    clear_git_runtime_discovery_cache();
}

fn git_runtime_id_for_path(path: &Path) -> String {
    format!("git:{}", git_runtime_key(path))
}

fn git_runtime_label(source: GitDiscoverySource, version: &str) -> String {
    let source_label = match source {
        GitDiscoverySource::EnvOverride => "Manual Git",
        GitDiscoverySource::Managed => "Managed Git",
        GitDiscoverySource::Path => "PATH Git",
        GitDiscoverySource::CommonLocation => "System Git",
    };
    if version.trim().is_empty() {
        source_label.to_string()
    } else {
        format!("{} {}", source_label, version)
    }
}

fn same_runtime_path(left: &Path, right: &Path) -> bool {
    git_runtime_id_for_path(left) == git_runtime_id_for_path(right)
}

fn git_runtime_info(candidate: GitRuntimeCandidate) -> GitRuntimeInfo {
    let id = git_runtime_id_for_path(&candidate.path);
    let version = candidate.version.trim().to_string();
    GitRuntimeInfo {
        id,
        label: git_runtime_label(candidate.source, &version),
        path: candidate.path.display().to_string(),
        version: if version.is_empty() {
            None
        } else {
            Some(version)
        },
        source: candidate.source.as_str().to_string(),
        selected: false,
        available: true,
    }
}

fn selected_git_path(app_handle: &AppHandle) -> Option<PathBuf> {
    read_saved_git_override(app_handle)
        .or_else(git_env_override)
        .map(PathBuf::from)
}

fn git_runtime_state_sync(
    app_handle: &AppHandle,
    refresh: bool,
) -> Result<GitRuntimeState, AppError> {
    let selected_path = selected_git_path(app_handle);
    let mut candidates = discover_git_runtimes_cached(refresh);

    if let Some(path) = selected_path.as_ref() {
        let already_listed = candidates
            .iter()
            .any(|candidate| same_runtime_path(&candidate.path, path));
        if !already_listed {
            if let Some(candidate) =
                probe_git_runtime(path.clone(), GitDiscoverySource::EnvOverride)
            {
                candidates.insert(0, candidate);
            }
        }
    }

    let mut runtimes: Vec<GitRuntimeInfo> = candidates.into_iter().map(git_runtime_info).collect();

    let selected_id = selected_path.as_ref().and_then(|path| {
        runtimes
            .iter()
            .find(|runtime| same_runtime_path(Path::new(&runtime.path), path))
            .map(|runtime| runtime.id.clone())
    });
    let missing_selected = selected_path.is_some() && selected_id.is_none();
    let effective_id = selected_id.clone().or_else(|| {
        runtimes
            .iter()
            .find(|runtime| runtime.available)
            .map(|runtime| runtime.id.clone())
    });

    for runtime in &mut runtimes {
        runtime.selected = effective_id
            .as_deref()
            .map(|id| runtime.id == id)
            .unwrap_or(false);
    }

    let effective = effective_id
        .as_deref()
        .and_then(|id| runtimes.iter().find(|runtime| runtime.id == id))
        .cloned();

    Ok(GitRuntimeState {
        runtimes,
        selected_id: effective_id,
        effective,
        missing_selected,
    })
}

fn discover_git_runtimes_cached(refresh: bool) -> Vec<GitRuntimeCandidate> {
    let cache = git_runtime_discovery_cache();
    if !refresh {
        if let Some(cached) = cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .cloned()
        {
            return cached;
        }
    }

    let runtimes = discover_git_runtimes(false);
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = Some(runtimes.clone());
    runtimes
}

fn clear_git_runtime_discovery_cache() {
    let cache = git_runtime_discovery_cache();
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cached = None;
}

fn git_runtime_discovery_cache() -> &'static Mutex<GitRuntimeDiscoveryCache> {
    static CACHE: OnceLock<Mutex<GitRuntimeDiscoveryCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

#[tauri::command]
pub async fn git_runtime_state(
    app_handle: AppHandle,
    refresh: Option<bool>,
) -> Result<GitRuntimeState, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        git_runtime_state_sync(&app_handle, refresh.unwrap_or(false))
    })
    .await
    .map_err(|e| {
        AppError::new(
            "git_runtime.join_failed",
            format!("Failed to load Git runtime state: {}", e),
        )
    })?
}

#[tauri::command]
pub async fn git_save_runtime_selection(
    selected_id: String,
    app_handle: AppHandle,
) -> Result<GitRuntimeState, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = git_runtime_state_sync(&app_handle, false)?;
        let Some(selected) = state
            .runtimes
            .iter()
            .find(|runtime| runtime.id == selected_id && runtime.available)
        else {
            return Err(AppError::new(
                "git_runtime.unavailable",
                "Selected Git runtime is unavailable.",
            ));
        };

        let override_path = git_override_path(&app_handle)?;
        std::fs::write(&override_path, &selected.path)
            .map_err(|e| format!("Failed to save Git runtime selection: {}", e))?;
        std::env::set_var("LOCUS_GIT_PATH", &selected.path);
        let _ = refresh_git_resolution();

        git_runtime_state_sync(&app_handle, false)
    })
    .await
    .map_err(|e| {
        AppError::new(
            "git_runtime.join_failed",
            format!("Failed to save Git runtime selection: {}", e),
        )
    })?
}

#[tauri::command]
pub async fn git_probe(workspace: State<'_, Arc<Workspace>>) -> Result<GitProbeResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    let resolved = refresh_git_resolution();
    let available = resolved.is_some();
    let is_repo = if available && !cwd.is_empty() {
        command("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map(|out| {
                out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true"
            })
            .unwrap_or(false)
    } else {
        false
    };

    Ok(GitProbeResult {
        available,
        in_path: git_is_in_path(),
        path: resolved.as_ref().map(|git| git.path.display().to_string()),
        source: resolved.as_ref().map(|git| git.source.as_str().to_string()),
        version: git_version(),
        env_override: git_env_override(),
        is_repo,
    })
}

#[tauri::command]
pub async fn git_head_hash(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Option<String>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(None);
    }

    let output = command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git not found: {}", e))?;

    if !output.status.success() {
        return Ok(None);
    }

    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hash))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInstallManager {
    pub id: String,
    pub label: String,
    pub command: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInstallHelp {
    pub os: String,
    pub package_managers: Vec<GitInstallManager>,
    pub official_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub china_mirror_url: Option<String>,
}

#[tauri::command]
pub async fn git_install_help() -> Result<GitInstallHelp, AppError> {
    #[cfg(target_os = "windows")]
    {
        return Ok(GitInstallHelp {
            os: "windows".to_string(),
            package_managers: vec![
                GitInstallManager {
                    id: "winget".to_string(),
                    label: "winget".to_string(),
                    command: "winget install --id Git.Git -e --source winget --accept-package-agreements --accept-source-agreements".to_string(),
                    available: program_in_path(&["winget.exe", "winget"]),
                },
                GitInstallManager {
                    id: "scoop".to_string(),
                    label: "Scoop".to_string(),
                    command: "scoop install git".to_string(),
                    available: program_in_path(&["scoop.cmd", "scoop.ps1", "scoop"]),
                },
                GitInstallManager {
                    id: "choco".to_string(),
                    label: "Chocolatey".to_string(),
                    command: "choco install git -y".to_string(),
                    available: program_in_path(&["choco.exe", "choco"]),
                },
            ],
            official_url: "https://git-scm.com/download/win".to_string(),
            china_mirror_url: Some(
                "https://mirrors.tuna.tsinghua.edu.cn/github-release/git-for-windows/git/LatestRelease/"
                    .to_string(),
            ),
        });
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(GitInstallHelp {
            os: "macos".to_string(),
            package_managers: vec![GitInstallManager {
                id: "brew".to_string(),
                label: "Homebrew".to_string(),
                command: "brew install git".to_string(),
                available: program_in_path(&["brew"]),
            }],
            official_url: "https://git-scm.com/download/mac".to_string(),
            china_mirror_url: None,
        });
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Ok(GitInstallHelp {
            os: "linux".to_string(),
            package_managers: Vec::new(),
            official_url: "https://git-scm.com/download/linux".to_string(),
            china_mirror_url: None,
        })
    }
}

#[tauri::command]
pub async fn git_install_via(manager: String) -> Result<RunCommandResult, AppError> {
    let manager = manager.trim().to_ascii_lowercase();

    let command_text = match manager.as_str() {
        #[cfg(target_os = "windows")]
        "winget" => "winget install --id Git.Git -e --source winget --accept-package-agreements --accept-source-agreements",
        #[cfg(target_os = "windows")]
        "scoop" => "scoop install git",
        #[cfg(target_os = "windows")]
        "choco" => "choco install git -y",
        #[cfg(target_os = "macos")]
        "brew" => "brew install git",
        _ => {
            return Err(AppError::new(
                "git.install.unsupported_manager",
                format!("Unsupported Git installer: {}", manager),
            ))
        }
    };

    let (shell, flag) = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-lc")
    };

    let output = async_command(shell)
        .arg(flag)
        .arg(command_text)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("Failed to install Git with {}: {}", manager, e))?;

    let _ = refresh_git_resolution();
    clear_git_runtime_discovery_cache();

    Ok(RunCommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

#[tauri::command]
pub async fn git_set_override(path: String, app_handle: AppHandle) -> Result<String, AppError> {
    let trimmed = path.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err(AppError::new(
            "git.override.empty_path",
            "Please select a Git executable or installation directory.",
        ));
    }

    let normalized = normalize_git_path(std::path::Path::new(trimmed)).ok_or_else(|| {
        AppError::new(
            "git.override.invalid_path",
            "No usable Git executable was found at the selected location.",
        )
    })?;

    let normalized = normalized.display().to_string();
    let override_path = git_override_path(&app_handle)?;
    std::fs::write(&override_path, &normalized)
        .map_err(|e| format!("Failed to save Git override: {}", e))?;
    std::env::set_var("LOCUS_GIT_PATH", &normalized);
    let _ = refresh_git_resolution();
    clear_git_runtime_discovery_cache();
    Ok(normalized)
}

#[tauri::command]
pub async fn git_clear_override(app_handle: AppHandle) -> Result<(), AppError> {
    let override_path = git_override_path(&app_handle)?;
    if override_path.exists() {
        std::fs::remove_file(&override_path)
            .map_err(|e| format!("Failed to clear Git override: {}", e))?;
    }
    std::env::remove_var("LOCUS_GIT_PATH");
    let _ = refresh_git_resolution();
    clear_git_runtime_discovery_cache();
    Ok(())
}

#[tauri::command]
pub async fn git_log(
    workspace: State<'_, Arc<Workspace>>,
    skip: Option<usize>,
    limit: Option<usize>,
) -> Result<GitLogResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(GitLogResult {
            is_repo: false,
            commits: vec![],
            head_hash: None,
        });
    }

    let check = command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git not found: {}", e))?;

    if !check.status.success() {
        return Ok(GitLogResult {
            is_repo: false,
            commits: vec![],
            head_hash: None,
        });
    }

    let head_output = command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git rev-parse failed: {}", e))?;

    let head_hash = if head_output.status.success() {
        Some(
            String::from_utf8_lossy(&head_output.stdout)
                .trim()
                .to_string(),
        )
    } else {
        None
    };

    let limit = limit.unwrap_or(200);
    let skip = skip.unwrap_or(0);
    let stash_root_hashes = collect_stash_root_hashes(&cwd);
    let stash_hidden_hashes = collect_stash_hidden_hashes(&cwd, &stash_root_hashes);

    let chunk_size = std::cmp::max(limit.saturating_mul(2), 128);
    let mut raw_skip = 0usize;
    let mut visible_skipped = 0usize;
    let mut commits = Vec::with_capacity(limit);

    'outer: loop {
        let output = command("git")
            .args([
                "-c",
                "core.quotePath=false",
                "log",
                "--all",
                "--topo-order",
                &format!("--skip={}", raw_skip),
                &format!("--max-count={}", chunk_size),
                "--pretty=format:%H%x00%h%x00%P%x00%an%x00%at%x00%s%x00%D%x00",
            ])
            .current_dir(&cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git log failed: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git log failed: {}", stderr).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut raw_count = 0usize;

        for record in stdout.split('\n') {
            let record = record.trim();
            if record.is_empty() {
                continue;
            }
            raw_count += 1;

            let parts: Vec<&str> = record.split('\0').collect();
            if parts.len() < 7 {
                continue;
            }

            let hash = parts[0].to_string();
            if stash_hidden_hashes.contains(&hash) {
                continue;
            }

            if visible_skipped < skip {
                visible_skipped += 1;
                continue;
            }

            let parents: Vec<String> = if parts[2].is_empty() {
                vec![]
            } else {
                parts[2]
                    .split(' ')
                    .filter(|parent| !stash_hidden_hashes.contains(*parent))
                    .map(|parent| parent.to_string())
                    .collect()
            };

            let refs: Vec<String> = if parts[6].is_empty() {
                vec![]
            } else {
                parts[6].split(", ").map(|s| s.trim().to_string()).collect()
            };

            let date: i64 = parts[4].parse().unwrap_or(0);
            let is_stash =
                stash_root_hashes.contains(&hash) || refs.iter().any(|r| r == "refs/stash");

            commits.push(GitCommitInfo {
                hash,
                short_hash: parts[1].to_string(),
                parents,
                author: parts[3].to_string(),
                date,
                message: parts[5].to_string(),
                refs,
                is_stash,
            });

            if commits.len() >= limit {
                break 'outer;
            }
        }

        if raw_count < chunk_size {
            break;
        }
        raw_skip += raw_count;
    }

    Ok(GitLogResult {
        is_repo: true,
        commits,
        head_hash,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileChange {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub lfs: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_exists_in_workspace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_is_directory_in_workspace: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GitBlockedPathReason {
    WindowsReservedName,
    WindowsTrailingDot,
    WindowsTrailingSpace,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBlockedPath {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub reason: GitBlockedPathReason,
    pub segment: String,
}

#[tauri::command]
pub async fn git_commit_body(
    workspace: State<'_, Arc<Workspace>>,
    hash: String,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    let output = command("git")
        .args(["show", "-s", "--format=%b", &hash])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git show failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git show failed: {}", stderr).into());
    }

    let body = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(body)
}

/// Check which paths are tracked by Git LFS via `git check-attr filter`.
fn mark_lfs_files(cwd: &str, files: &mut [GitFileChange]) {
    if files.is_empty() {
        return;
    }
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "check-attr".to_string(),
        "filter".to_string(),
        "--".to_string(),
    ];
    args.extend(paths.iter().map(|p| p.to_string()));
    let output = command("git")
        .args(&args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output format: "path: filter: lfs" (one per line)
    let lfs_paths: std::collections::HashSet<&str> = stdout
        .lines()
        .filter(|line| line.ends_with(": filter: lfs"))
        .filter_map(|line| line.strip_suffix(": filter: lfs"))
        .collect();
    for file in files.iter_mut() {
        if lfs_paths.contains(file.path.as_str()) {
            file.lfs = true;
        }
    }
}

// ── Merge / conflict types ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmergedFileEntry {
    pub path: String,
    /// Raw two-char conflict code from porcelain v2, e.g. "UU", "AA", "DD"
    pub conflict_code: String,
    /// Human-readable label: "both modified", "added by us", etc.
    pub semantic_label: String,
    /// OID of base (stage 1), "0"*40 if absent
    pub base_oid: String,
    /// OID of ours/left (stage 2)
    pub left_oid: String,
    /// OID of theirs/right (stage 3)
    pub right_oid: String,
    pub lfs: bool,
    pub head_mode: String,
    pub stage1_mode: String,
    pub stage2_mode: String,
    pub stage3_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_exists_in_workspace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_is_directory_in_workspace: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeOperationKind {
    Merge,
    CherryPick,
    Rebase,
    Revert,
    GenericConflict,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOperation {
    pub kind: MergeOperationKind,
    pub can_continue: bool,
    pub can_skip: bool,
    pub can_abort: bool,
    pub label: String,
}

pub fn conflict_semantic_label(code: &str) -> &'static str {
    match code {
        "DD" => "both deleted",
        "AU" => "added by us",
        "UD" => "deleted by them",
        "UA" => "added by them",
        "DU" => "deleted by us",
        "AA" => "both added",
        "UU" => "both modified",
        _ => "conflict",
    }
}

/// Resolve the actual .git directory via `git rev-parse --git-dir`.
/// Handles worktrees and subdirectories correctly.
pub fn git_dir(cwd: &str) -> Option<String> {
    let output = command("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // git rev-parse --git-dir may return a relative path
    if std::path::Path::new(&raw).is_absolute() {
        Some(raw)
    } else {
        Some(
            std::path::Path::new(cwd)
                .join(&raw)
                .to_string_lossy()
                .to_string(),
        )
    }
}

/// Resolve a commit hash to a short branch/tag name, falling back to the abbreviated hash.
fn resolve_ref_label(cwd: &str, hash: &str) -> String {
    let short_hash = if hash.len() >= 7 { &hash[..7] } else { hash };
    // Try symbolic name first
    let output = command("git")
        .args(["name-rev", "--name-only", "--no-undefined", hash])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
            // name-rev may append ~N or ^N suffixes; strip for cleaner display
            if let Some(base) = name.split('~').next() {
                if !base.is_empty() && base != hash {
                    return base.to_string();
                }
            }
        }
    }
    short_hash.to_string()
}

/// Get the current HEAD branch name (e.g. "main"), or short hash if detached.
fn current_branch_label(cwd: &str) -> String {
    let output = command("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if name != "HEAD" && !name.is_empty() {
                return name;
            }
        }
    }
    // Detached HEAD — return short hash
    let output = command("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            return String::from_utf8_lossy(&o.stdout).trim().to_string();
        }
    }
    "HEAD".to_string()
}

/// Semantic side labels for merge conflict UI.
/// Returns `(left_label, right_label)` corresponding to git Stage 2 and Stage 3.
pub fn resolve_merge_side_labels(cwd: &str) -> (String, String) {
    let fallback = ("Ours".to_string(), "Theirs".to_string());
    let git = match git_dir(cwd) {
        Some(d) => d,
        None => return fallback,
    };
    let gp = std::path::Path::new(&git);

    if gp.join("MERGE_HEAD").exists() {
        let current = current_branch_label(cwd);
        let merge_head = std::fs::read_to_string(gp.join("MERGE_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let incoming = resolve_ref_label(cwd, &merge_head);
        return (
            format!("Current ({})", current),
            format!("Incoming ({})", incoming),
        );
    }

    if gp.join("CHERRY_PICK_HEAD").exists() {
        let current = current_branch_label(cwd);
        let cp_head = std::fs::read_to_string(gp.join("CHERRY_PICK_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let short = if cp_head.len() >= 7 {
            &cp_head[..7]
        } else {
            &cp_head
        };
        return (
            format!("Current ({})", current),
            format!("Cherry-pick ({})", short),
        );
    }

    if gp.join("REVERT_HEAD").exists() {
        let current = current_branch_label(cwd);
        let rv_head = std::fs::read_to_string(gp.join("REVERT_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let short = if rv_head.len() >= 7 {
            &rv_head[..7]
        } else {
            &rv_head
        };
        return (
            format!("Current ({})", current),
            format!("Revert of ({})", short),
        );
    }

    if gp.join("rebase-merge").exists() {
        let rm = gp.join("rebase-merge");
        // onto commit → the target branch
        let onto = std::fs::read_to_string(rm.join("onto"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let onto_label = if onto.is_empty() {
            "target".to_string()
        } else {
            resolve_ref_label(cwd, &onto)
        };
        // stopped-sha → the commit being replayed
        let stopped = std::fs::read_to_string(rm.join("stopped-sha"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let commit_label = if stopped.len() >= 7 {
            stopped[..7].to_string()
        } else if !stopped.is_empty() {
            stopped
        } else {
            "commit".to_string()
        };
        return (
            format!("Rebase target ({})", onto_label),
            format!("Your commit ({})", commit_label),
        );
    }

    if gp.join("rebase-apply").exists() {
        let ra = gp.join("rebase-apply");
        // head-name stores the branch being rebased (e.g. refs/heads/feature)
        let head_name = std::fs::read_to_string(ra.join("head-name"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let _user_branch = head_name
            .strip_prefix("refs/heads/")
            .unwrap_or(&head_name)
            .to_string();
        // original-commit stores the commit being applied
        let orig = std::fs::read_to_string(ra.join("original-commit"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let commit_label = if orig.len() >= 7 {
            orig[..7].to_string()
        } else if !orig.is_empty() {
            orig
        } else {
            "commit".to_string()
        };
        // For rebase-apply, Stage 2 is still the target (onto)
        let onto = std::fs::read_to_string(ra.join("onto"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let onto_label = if onto.is_empty() {
            "target".to_string()
        } else {
            resolve_ref_label(cwd, &onto)
        };
        return (
            format!("Rebase target ({})", onto_label),
            format!("Your commit ({})", commit_label),
        );
    }

    fallback
}

/// Detect the current merge/rebase/cherry-pick/revert operation by checking sentinel files.
/// Try to resolve a commit hash to a human-readable ref name (branch or tag).
fn resolve_commit_name(cwd: &str, hash: &str) -> Option<String> {
    // Check if hash matches a stash ref first via MERGE_MSG
    // Then try name-rev to find branch/tag names
    let out = command("git")
        .args(["name-rev", "--name-only", "--no-undefined", hash])
        .current_dir(cwd)
        .output()
        .ok()?;
    if out.status.success() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // name-rev may return things like "master~2", we only want clean names
        if !name.is_empty() && !name.contains('~') && !name.contains('^') {
            return Some(name);
        }
    }
    None
}

/// Get the current branch name (HEAD).
fn current_branch_name(cwd: &str) -> Option<String> {
    let out = command("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if out.status.success() {
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !name.is_empty() && name != "HEAD" {
            return Some(name);
        }
    }
    None
}

fn detect_merge_operation(cwd: &str, has_unmerged: bool) -> Option<MergeOperation> {
    if !has_unmerged && crate::vcs::git_merge::has_stash_apply_abort_state(cwd) {
        crate::vcs::git_merge::clear_stash_apply_abort_state(cwd);
    }

    let git = match git_dir(cwd) {
        Some(d) => d,
        None => return None,
    };
    let gp = std::path::Path::new(&git);

    let (kind, label) = if gp.join("MERGE_HEAD").exists() {
        let head = std::fs::read_to_string(gp.join("MERGE_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        // Check MERGE_MSG for stash-related message
        let merge_msg = std::fs::read_to_string(gp.join("MERGE_MSG")).unwrap_or_default();
        let is_stash = merge_msg.contains("stash");

        if is_stash {
            // Stash apply/pop conflict
            let current = current_branch_name(cwd).unwrap_or_else(|| "HEAD".into());
            let short = if head.len() >= 7 { &head[..7] } else { &head };
            (
                MergeOperationKind::Merge,
                format!("Applying stash onto {} ({})", current, short),
            )
        } else {
            // Regular merge — resolve incoming to a branch name
            let incoming = resolve_commit_name(cwd, &head).unwrap_or_else(|| {
                if head.len() >= 7 {
                    head[..7].to_string()
                } else {
                    head.clone()
                }
            });
            let current = current_branch_name(cwd).unwrap_or_else(|| "HEAD".into());
            (
                MergeOperationKind::Merge,
                format!("{} ← {}", current, incoming),
            )
        }
    } else if gp.join("CHERRY_PICK_HEAD").exists() {
        let head = std::fs::read_to_string(gp.join("CHERRY_PICK_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let short = if head.len() >= 7 { &head[..7] } else { &head };
        let current = current_branch_name(cwd).unwrap_or_else(|| "HEAD".into());
        (
            MergeOperationKind::CherryPick,
            format!("Cherry-pick {} onto {}", short, current),
        )
    } else if gp.join("REVERT_HEAD").exists() {
        let head = std::fs::read_to_string(gp.join("REVERT_HEAD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let short = if head.len() >= 7 { &head[..7] } else { &head };
        (
            MergeOperationKind::Revert,
            format!(
                "Revert {} on {}",
                short,
                current_branch_name(cwd).unwrap_or_else(|| "HEAD".into())
            ),
        )
    } else if gp.join("rebase-merge").exists() || gp.join("rebase-apply").exists() {
        let current = current_branch_name(cwd).unwrap_or_else(|| "HEAD".into());
        // Try to read the rebase target from rebase-merge/head-name
        let onto = std::fs::read_to_string(gp.join("rebase-merge").join("head-name"))
            .or_else(|_| std::fs::read_to_string(gp.join("rebase-apply").join("head-name")))
            .unwrap_or_default()
            .trim()
            .trim_start_matches("refs/heads/")
            .to_string();
        if onto.is_empty() {
            (MergeOperationKind::Rebase, format!("Rebasing {}", current))
        } else {
            (MergeOperationKind::Rebase, format!("Rebasing {}", onto))
        }
    } else if has_unmerged && crate::vcs::git_merge::has_stash_apply_abort_state(cwd) {
        (MergeOperationKind::GenericConflict, "Applying stash".into())
    } else if has_unmerged {
        (MergeOperationKind::GenericConflict, "Conflict".into())
    } else {
        return None;
    };

    let can_skip = matches!(kind, MergeOperationKind::Rebase);
    Some(MergeOperation {
        can_continue: !has_unmerged, // all resolved → can continue
        can_skip,
        can_abort: true,
        label,
        kind,
    })
}

fn has_unmerged_entries(cwd: &str) -> bool {
    let output = match command("git")
        .args(["ls-files", "-u"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };

    !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

/// Mark LFS attribute on unmerged entries (same pattern as mark_lfs_files).
fn mark_lfs_unmerged(cwd: &str, files: &mut [UnmergedFileEntry]) {
    if files.is_empty() {
        return;
    }
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let mut args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "check-attr".to_string(),
        "filter".to_string(),
        "--".to_string(),
    ];
    args.extend(paths.iter().map(|p| p.to_string()));
    let output = command("git")
        .args(&args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lfs_paths: std::collections::HashSet<&str> = stdout
        .lines()
        .filter(|line| line.ends_with(": filter: lfs"))
        .filter_map(|line| line.strip_suffix(": filter: lfs"))
        .collect();
    for file in files.iter_mut() {
        if lfs_paths.contains(file.path.as_str()) {
            file.lfs = true;
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct WorkspacePrimaryState {
    exists: bool,
    is_dir: bool,
}

fn resolve_workspace_primary_state(
    cwd: &str,
    meta_path: &str,
    cache: &mut std::collections::HashMap<String, WorkspacePrimaryState>,
) -> Option<WorkspacePrimaryState> {
    let primary_path = meta_path.strip_suffix(".meta")?;
    if let Some(state) = cache.get(primary_path).copied() {
        return Some(state);
    }

    let primary_abs = std::path::Path::new(cwd).join(primary_path);
    let state = match std::fs::metadata(primary_abs) {
        Ok(metadata) => WorkspacePrimaryState {
            exists: true,
            is_dir: metadata.is_dir(),
        },
        Err(_) => WorkspacePrimaryState {
            exists: false,
            is_dir: false,
        },
    };
    cache.insert(primary_path.to_string(), state);
    Some(state)
}

fn annotate_git_file_meta_primary_states(
    cwd: &str,
    files: &mut [GitFileChange],
    cache: &mut std::collections::HashMap<String, WorkspacePrimaryState>,
) {
    for file in files.iter_mut() {
        let Some(state) = resolve_workspace_primary_state(cwd, &file.path, cache) else {
            continue;
        };
        file.primary_exists_in_workspace = Some(state.exists);
        file.primary_is_directory_in_workspace = Some(state.is_dir);
    }
}

fn annotate_unmerged_meta_primary_states(
    cwd: &str,
    files: &mut [UnmergedFileEntry],
    cache: &mut std::collections::HashMap<String, WorkspacePrimaryState>,
) {
    for file in files.iter_mut() {
        let Some(state) = resolve_workspace_primary_state(cwd, &file.path, cache) else {
            continue;
        };
        file.primary_exists_in_workspace = Some(state.exists);
        file.primary_is_directory_in_workspace = Some(state.is_dir);
    }
}

#[derive(Debug, Clone)]
struct ParsedGitStatus {
    unstaged: Vec<GitFileChange>,
    staged: Vec<GitFileChange>,
    blocked: Vec<GitBlockedPath>,
    unmerged: Vec<UnmergedFileEntry>,
}

fn is_windows_reserved_device_name(name: &str) -> bool {
    match name {
        "CON" | "PRN" | "AUX" | "NUL" => true,
        _ => {
            if let Some(rest) = name.strip_prefix("COM") {
                return matches!(rest, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9");
            }
            if let Some(rest) = name.strip_prefix("LPT") {
                return matches!(rest, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9");
            }
            false
        }
    }
}

fn detect_windows_blocked_segment(path: &str) -> Option<(GitBlockedPathReason, String)> {
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }

        let trimmed = segment.trim_end_matches(|ch| ch == ' ' || ch == '.');
        let base = trimmed.split('.').next().unwrap_or(trimmed);
        let upper = base.to_ascii_uppercase();

        if is_windows_reserved_device_name(&upper) {
            return Some((
                GitBlockedPathReason::WindowsReservedName,
                segment.to_string(),
            ));
        }
        if segment.ends_with('.') {
            return Some((
                GitBlockedPathReason::WindowsTrailingDot,
                segment.to_string(),
            ));
        }
        if segment.ends_with(' ') {
            return Some((
                GitBlockedPathReason::WindowsTrailingSpace,
                segment.to_string(),
            ));
        }
    }

    None
}

fn build_blocked_path(
    path: String,
    old_path: Option<String>,
    status: String,
) -> Option<GitBlockedPath> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    let (reason, segment) = detect_windows_blocked_segment(&path)?;
    Some(GitBlockedPath {
        path,
        old_path,
        status,
        reason,
        segment,
    })
}

fn push_unstaged_or_blocked(
    unstaged: &mut Vec<GitFileChange>,
    blocked: &mut Vec<GitBlockedPath>,
    path: String,
    old_path: Option<String>,
    status: String,
) {
    if let Some(entry) = build_blocked_path(path.clone(), old_path.clone(), status.clone()) {
        blocked.push(entry);
        return;
    }

    unstaged.push(GitFileChange {
        path,
        old_path,
        status,
        lfs: false,
        primary_exists_in_workspace: None,
        primary_is_directory_in_workspace: None,
    });
}

fn parse_git_status_porcelain(stdout: &str) -> ParsedGitStatus {
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();
    let mut blocked = Vec::new();
    let mut unmerged = Vec::new();

    let segments: Vec<&str> = stdout.split('\0').collect();
    let mut i = 0;
    while i < segments.len() {
        let entry = segments[i];
        if entry.is_empty() {
            i += 1;
            continue;
        }

        let first_char = entry.as_bytes()[0];
        match first_char {
            b'1' => {
                let parts: Vec<&str> = entry.splitn(9, ' ').collect();
                if parts.len() < 9 {
                    i += 1;
                    continue;
                }
                let xy = parts[1];
                let x = xy.as_bytes().first().copied().unwrap_or(b' ') as char;
                let y = xy.as_bytes().get(1).copied().unwrap_or(b' ') as char;
                let path = parts[8].to_string();

                if x != '.' && x != '?' {
                    staged.push(GitFileChange {
                        path: path.clone(),
                        old_path: None,
                        status: x.to_string(),
                        lfs: false,
                        primary_exists_in_workspace: None,
                        primary_is_directory_in_workspace: None,
                    });
                }
                if y == '?' {
                    push_unstaged_or_blocked(
                        &mut unstaged,
                        &mut blocked,
                        path,
                        None,
                        "?".to_string(),
                    );
                } else if y != '.' {
                    push_unstaged_or_blocked(
                        &mut unstaged,
                        &mut blocked,
                        path,
                        None,
                        y.to_string(),
                    );
                }
                i += 1;
            }
            b'2' => {
                let parts: Vec<&str> = entry.splitn(10, ' ').collect();
                if parts.len() < 10 {
                    i += 1;
                    continue;
                }
                let xy = parts[1];
                let x = xy.as_bytes().first().copied().unwrap_or(b' ') as char;
                let y = xy.as_bytes().get(1).copied().unwrap_or(b' ') as char;
                let path = parts[9].to_string();
                let old_path = if i + 1 < segments.len() {
                    i += 1;
                    Some(segments[i].to_string())
                } else {
                    None
                };

                if x != '.' {
                    staged.push(GitFileChange {
                        path: path.clone(),
                        old_path: old_path.clone(),
                        status: x.to_string(),
                        lfs: false,
                        primary_exists_in_workspace: None,
                        primary_is_directory_in_workspace: None,
                    });
                }
                if y != '.' {
                    push_unstaged_or_blocked(
                        &mut unstaged,
                        &mut blocked,
                        path,
                        old_path,
                        y.to_string(),
                    );
                }
                i += 1;
            }
            b'u' => {
                let parts: Vec<&str> = entry.splitn(11, ' ').collect();
                if parts.len() < 11 {
                    i += 1;
                    continue;
                }
                let conflict_code = parts[1].to_string();
                let stage1_mode = parts[3].to_string();
                let stage2_mode = parts[4].to_string();
                let stage3_mode = parts[5].to_string();
                let head_mode = parts[6].to_string();
                let base_oid = parts[7].to_string();
                let left_oid = parts[8].to_string();
                let right_oid = parts[9].to_string();
                let path = parts[10].to_string();

                let semantic_label = conflict_semantic_label(&conflict_code).to_string();

                unmerged.push(UnmergedFileEntry {
                    path,
                    conflict_code,
                    semantic_label,
                    base_oid,
                    left_oid,
                    right_oid,
                    lfs: false,
                    head_mode,
                    stage1_mode,
                    stage2_mode,
                    stage3_mode,
                    primary_exists_in_workspace: None,
                    primary_is_directory_in_workspace: None,
                });
                i += 1;
            }
            b'?' => {
                let path = if entry.len() > 2 {
                    entry[2..].to_string()
                } else {
                    i += 1;
                    continue;
                };
                push_unstaged_or_blocked(&mut unstaged, &mut blocked, path, None, "?".to_string());
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    ParsedGitStatus {
        unstaged,
        staged,
        blocked,
        unmerged,
    }
}

fn run_git_status_porcelain(cwd: &str) -> Result<String, AppError> {
    let output = command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "-z",
            "-uall",
        ])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git status failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git status failed: {}", stderr).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    pub unstaged: Vec<GitFileChange>,
    pub staged: Vec<GitFileChange>,
    pub blocked: Vec<GitBlockedPath>,
    pub unmerged: Vec<UnmergedFileEntry>,
    pub operation: Option<MergeOperation>,
}

#[tauri::command]
pub async fn git_status(workspace: State<'_, Arc<Workspace>>) -> Result<GitStatusResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(GitStatusResult {
            unstaged: vec![],
            staged: vec![],
            blocked: vec![],
            unmerged: vec![],
            operation: None,
        });
    }

    let stdout = run_git_status_porcelain(&cwd)?;
    let ParsedGitStatus {
        mut unstaged,
        mut staged,
        blocked,
        mut unmerged,
    } = parse_git_status_porcelain(&stdout);

    if unmerged.is_empty() {
        match load_rename_aware_unstaged_changes(&cwd) {
            Ok(rename_aware_unstaged) => {
                unstaged = rename_aware_unstaged;
            }
            Err(err) => {
                eprintln!("[git_status] rename-aware unstaged fallback: {}", err);
            }
        }
    }

    // LFS batch check — combine all paths into a single git check-attr call
    mark_lfs_files(&cwd, &mut staged);
    mark_lfs_files(&cwd, &mut unstaged);
    mark_lfs_unmerged(&cwd, &mut unmerged);

    let mut primary_state_cache = std::collections::HashMap::new();
    annotate_git_file_meta_primary_states(&cwd, &mut staged, &mut primary_state_cache);
    annotate_git_file_meta_primary_states(&cwd, &mut unstaged, &mut primary_state_cache);
    annotate_unmerged_meta_primary_states(&cwd, &mut unmerged, &mut primary_state_cache);

    // Detect merge/rebase/cherry-pick/revert operation via sentinel files
    let operation = detect_merge_operation(&cwd, !unmerged.is_empty());

    Ok(GitStatusResult {
        unstaged,
        staged,
        blocked,
        unmerged,
        operation,
    })
}

#[tauri::command]
pub async fn git_stage(workspace: State<'_, Arc<Workspace>>, path: String) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    let output = command("git")
        .args(["add", "--", &path])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git add failed: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git add failed: {}", stderr).into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_stage_paths(
    workspace: State<'_, Arc<Workspace>>,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(paths);
    let output = command("git")
        .args(&args)
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git add failed: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git add failed: {}", stderr).into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_unstage(
    workspace: State<'_, Arc<Workspace>>,
    path: String,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    let output = command("git")
        .args(["restore", "--staged", "--", &path])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git restore --staged failed: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git restore --staged failed: {}", stderr).into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_discard_file(
    workspace: State<'_, Arc<Workspace>>,
    path: String,
    status: String,
    old_path: Option<String>,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    discard_git_file(&cwd, &path, &status, old_path.as_deref())
}

fn remove_worktree_path_if_exists(path: &std::path::Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() {
                std::fs::remove_dir_all(path)
                    .map_err(|e| format!("failed to remove path {}: {}", path.display(), e))?;
            } else {
                std::fs::remove_file(path)
                    .map_err(|e| format!("failed to remove path {}: {}", path.display(), e))?;
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("failed to inspect path {}: {}", path.display(), e).into()),
    }
}

fn discard_git_file(
    cwd: &str,
    path: &str,
    status: &str,
    old_path: Option<&str>,
) -> Result<(), AppError> {
    if status == "?" || status == "A" {
        // Untracked or added file — remove it from the worktree and index if needed.
        let full = std::path::Path::new(cwd).join(path);
        remove_worktree_path_if_exists(&full)?;
        if status == "A" {
            let output = command("git")
                .args(["rm", "--cached", "--quiet", "--ignore-unmatch", "--", path])
                .current_dir(cwd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .map_err(|e| format!("git rm --cached failed: {}", e))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("git rm --cached failed: {}", stderr).into());
            }
        }
    } else if status == "R" {
        let full = std::path::Path::new(cwd).join(path);
        remove_worktree_path_if_exists(&full)?;

        let output = command("git")
            .args(["rm", "--cached", "--quiet", "--ignore-unmatch", "--", path])
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git rm --cached failed: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git rm --cached failed: {}", stderr).into());
        }

        let restore_path = old_path.unwrap_or(path);
        let output = command("git")
            .args([
                "restore",
                "--source=HEAD",
                "--staged",
                "--worktree",
                "--",
                restore_path,
            ])
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git restore failed: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git restore failed: {}", stderr).into());
        }
    } else {
        // Tracked file — restore from HEAD
        let output = command("git")
            .args(["checkout", "HEAD", "--", path])
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|e| format!("git checkout failed: {}", e))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git checkout failed: {}", stderr).into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn git_unstage_paths(
    workspace: State<'_, Arc<Workspace>>,
    paths: Vec<String>,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    let mut args = vec![
        "restore".to_string(),
        "--staged".to_string(),
        "--".to_string(),
    ];
    args.extend(paths);
    let output = command("git")
        .args(&args)
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git restore --staged failed: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git restore --staged failed: {}", stderr).into());
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStageAllResult {
    pub staged_count: usize,
    pub skipped_count: usize,
    pub blocked: Vec<GitBlockedPath>,
    pub stdout: String,
    pub stderr: String,
}

fn collect_stage_all_pathspecs(files: &[GitFileChange]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();

    for file in files {
        if seen.insert(file.path.clone()) {
            paths.push(file.path.clone());
        }
        if let Some(old_path) = file.old_path.as_ref() {
            if seen.insert(old_path.clone()) {
                paths.push(old_path.clone());
            }
        }
    }

    paths
}

fn run_git_add_with_pathspecs(
    cwd: &str,
    paths: &[String],
) -> Result<std::process::Output, AppError> {
    let mut child = command("git")
        .args([
            "add",
            "--ignore-errors",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ])
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("git add -A failed: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        for path in paths {
            stdin
                .write_all(path.as_bytes())
                .map_err(|e| format!("git add -A failed: {}", e))?;
            stdin
                .write_all(&[0])
                .map_err(|e| format!("git add -A failed: {}", e))?;
        }
    }

    child
        .wait_with_output()
        .map_err(|e| format!("git add -A failed: {}", e).into())
}

#[tauri::command]
pub async fn git_stage_all(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<GitStageAllResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }

    let stdout = run_git_status_porcelain(&cwd)?;
    let parsed = parse_git_status_porcelain(&stdout);
    let stage_paths = collect_stage_all_pathspecs(&parsed.unstaged);

    if stage_paths.is_empty() {
        return Ok(GitStageAllResult {
            staged_count: 0,
            skipped_count: parsed.blocked.len(),
            blocked: parsed.blocked,
            stdout: String::new(),
            stderr: String::new(),
        });
    }

    let output = run_git_add_with_pathspecs(&cwd, &stage_paths)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let message = if stderr.is_empty() {
            "git add -A failed".to_string()
        } else {
            format!("git add -A failed: {}", stderr)
        };
        return Err(message.into());
    }

    Ok(GitStageAllResult {
        staged_count: parsed.unstaged.len(),
        skipped_count: parsed.blocked.len(),
        blocked: parsed.blocked,
        stdout,
        stderr,
    })
}

#[tauri::command]
pub async fn git_unstage_all(workspace: State<'_, Arc<Workspace>>) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    let output = command("git")
        .args(["reset", "HEAD"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git reset HEAD failed: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git reset HEAD failed: {}", stderr).into());
    }
    Ok(())
}

#[tauri::command]
pub async fn git_commit_files(
    workspace: State<'_, Arc<Workspace>>,
    hash: String,
) -> Result<Vec<GitFileChange>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(vec![]);
    }

    let is_stash_revision = is_stash_revision(&cwd, &hash);
    let args = build_commit_files_args(&hash, is_stash_revision);
    let output = command("git")
        .args(&args)
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| {
            if is_stash_revision {
                format!("git stash show failed: {}", e)
            } else {
                format!("git diff-tree failed: {}", e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let label = if is_stash_revision {
            "git stash show failed"
        } else {
            "git diff-tree failed"
        };
        return Err(format!("{}: {}", label, stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = parse_name_status_lines(&stdout);

    mark_lfs_files(&cwd, &mut files);

    Ok(files)
}

#[tauri::command]
pub async fn git_compare_files(
    workspace: State<'_, Arc<Workspace>>,
    from_hash: String,
    to_hash: String,
) -> Result<Vec<GitFileChange>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory".to_string().into());
    }

    let range = format!("{}..{}", from_hash, to_hash);
    let args = build_compare_files_args(&range);
    let output = command("git")
        .args(&args)
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git diff failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let is_bad_revision = stderr.contains("unknown revision")
            || stderr.contains("bad revision")
            || stderr.contains("Invalid revision range");
        if is_bad_revision {
            return Err(AppError::new(
                "git.commit_unreachable",
                "Commit no longer exists or is unreachable",
            )
            .detail(stderr));
        }
        return Err(
            AppError::new("git.diff_failed", format!("git diff failed: {}", stderr)).detail(stderr),
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    const MAX_FILES: usize = 500;
    let mut files = parse_name_status_lines(&stdout);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
    }

    mark_lfs_files(&cwd, &mut files);

    Ok(files)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchInfo {
    pub name: String,
    pub is_current: bool,
    pub short_hash: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRemoteBranch {
    pub name: String,
    pub short_hash: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchesResult {
    pub local: Vec<GitBranchInfo>,
    /// remote_name → branches
    pub remotes: Vec<(String, Vec<GitRemoteBranch>)>,
}

#[tauri::command]
pub async fn git_branches(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<GitBranchesResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(GitBranchesResult {
            local: vec![],
            remotes: vec![],
        });
    }

    let local_output = command("git")
        .args(["-c", "core.quotePath=false", "branch", "-v", "--no-color"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git branch failed: {}", e))?;

    let mut local = Vec::new();
    if local_output.status.success() {
        let stdout = String::from_utf8_lossy(&local_output.stdout);
        for line in stdout.lines() {
            let is_current = line.starts_with('*');
            let line = line.trim_start_matches('*').trim();
            if line.is_empty() {
                continue;
            }
            // format: "branch_name  hash message..."
            let parts: Vec<&str> = line.splitn(3, char::is_whitespace).collect();
            if parts.len() >= 2 {
                let name = parts[0].to_string();
                let short_hash = parts[1].to_string();
                let message = if parts.len() >= 3 {
                    parts[2].trim().to_string()
                } else {
                    String::new()
                };
                local.push(GitBranchInfo {
                    name,
                    is_current,
                    short_hash,
                    message,
                });
            }
        }
    }

    let remote_output = command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "branch",
            "-r",
            "-v",
            "--no-color",
        ])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git branch -r failed: {}", e))?;

    let mut remote_map: std::collections::BTreeMap<String, Vec<GitRemoteBranch>> =
        std::collections::BTreeMap::new();
    if remote_output.status.success() {
        let stdout = String::from_utf8_lossy(&remote_output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // skip HEAD pointer lines like "origin/HEAD -> origin/main"
            if line.contains("->") {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, char::is_whitespace).collect();
            if parts.len() >= 2 {
                let full_name = parts[0];
                let short_hash = parts[1].to_string();
                let message = if parts.len() >= 3 {
                    parts[2].trim().to_string()
                } else {
                    String::new()
                };
                // split "origin/branch_name"
                if let Some(slash_pos) = full_name.find('/') {
                    let remote_name = full_name[..slash_pos].to_string();
                    let branch_name = full_name[slash_pos + 1..].to_string();
                    remote_map
                        .entry(remote_name)
                        .or_default()
                        .push(GitRemoteBranch {
                            name: branch_name,
                            short_hash,
                            message,
                        });
                }
            }
        }
    }

    let remotes: Vec<(String, Vec<GitRemoteBranch>)> = remote_map.into_iter().collect();

    Ok(GitBranchesResult { local, remotes })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStashEntry {
    /// stash index, e.g. 0
    pub index: usize,
    pub ref_name: String,
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: i64,
    pub message: String,
    pub parent_hashes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_hash: Option<String>,
}

#[tauri::command]
pub async fn git_stashes(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<GitStashEntry>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(vec![]);
    }
    Ok(collect_stash_entries(&cwd))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitSubmoduleInfo {
    pub path: String,
    pub name: String,
    pub hash: String,
    pub status: String,
}

#[tauri::command]
pub async fn git_submodules(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<Vec<GitSubmoduleInfo>, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Ok(vec![]);
    }

    let output = command("git")
        .args(["-c", "core.quotePath=false", "submodule", "status"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git submodule status failed: {}", e))?;

    let mut modules = Vec::new();
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            // format: " hash path (desc)" or "-hash path" (uninitialized) or "+hash path" (modified)
            let (status, rest) = if line.starts_with('-') {
                ("uninitialized".to_string(), &line[1..])
            } else if line.starts_with('+') {
                ("modified".to_string(), &line[1..])
            } else {
                ("ok".to_string(), line)
            };
            let parts: Vec<&str> = rest.splitn(3, char::is_whitespace).collect();
            if parts.len() >= 2 {
                let hash = parts[0].to_string();
                let path = parts[1].to_string();
                let name = path.split('/').last().unwrap_or(&path).to_string();
                modules.push(GitSubmoduleInfo {
                    path,
                    name,
                    hash,
                    status,
                });
            }
        }
    }

    Ok(modules)
}

#[tauri::command]
pub async fn git_init_unity(workspace: State<'_, Arc<Workspace>>) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }

    // 1. git init
    let init = command("git")
        .arg("init")
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git init failed: {}", e))?;
    if !init.status.success() {
        return Err(format!("git init failed: {}", String::from_utf8_lossy(&init.stderr)).into());
    }

    let gitignore = r#"# ── Unity generated ──
/[Ll]ibrary/
/[Tt]emp/
/[Oo]bj/
/[Bb]uild/
/[Bb]uilds/
/[Ll]ogs/
/[Uu]ser[Ss]ettings/

# ── IDE ──
/.idea/
/.vs/
/.vscode/
*.csproj
*.unityproj
*.sln
*.suo
*.tmp
*.user
*.userprefs
*.pidb
*.booproj
*.svd
*.pdb
*.mdb
*.opendb
*.VC.db

# ── OS ──
.DS_Store
Thumbs.db

# ── Gradle (Android build) ──
ExportedObj/
*.apk
*.aab
*.unitypackage
*.app

# ── Crash reports ──
sysinfo.txt
crashlytics-build.properties

# ── Recordings / Profiling ──
/[Aa]ssets/[Ss]treamingAssets/aa.meta
/[Aa]ssets/[Ss]treamingAssets/aa/
"#;
    std::fs::write(std::path::Path::new(&cwd).join(".gitignore"), gitignore)
        .map_err(|e| format!("Failed to write .gitignore: {}", e))?;

    let gitattributes = r#"# ── Text line endings ──
* text=auto eol=lf
*.bat text eol=crlf
*.cmd text eol=crlf

# Unity text assets
*.meta text eol=lf
*.unity text eol=lf
*.prefab text eol=lf
*.asset text eol=lf
*.mat text eol=lf
*.anim text eol=lf
*.controller text eol=lf
*.asmdef text eol=lf
*.shader text eol=lf
*.compute text eol=lf
*.cginc text eol=lf
*.hlsl text eol=lf
*.uxml text eol=lf
*.uss text eol=lf

# ── Git LFS ──
# 3D models
*.fbx filter=lfs diff=lfs merge=lfs -text
*.obj filter=lfs diff=lfs merge=lfs -text
*.blend filter=lfs diff=lfs merge=lfs -text
*.dae filter=lfs diff=lfs merge=lfs -text
*.3ds filter=lfs diff=lfs merge=lfs -text
*.max filter=lfs diff=lfs merge=lfs -text
*.ma filter=lfs diff=lfs merge=lfs -text
*.mb filter=lfs diff=lfs merge=lfs -text

# Textures
*.png filter=lfs diff=lfs merge=lfs -text
*.jpg filter=lfs diff=lfs merge=lfs -text
*.jpeg filter=lfs diff=lfs merge=lfs -text
*.psd filter=lfs diff=lfs merge=lfs -text
*.tga filter=lfs diff=lfs merge=lfs -text
*.tif filter=lfs diff=lfs merge=lfs -text
*.tiff filter=lfs diff=lfs merge=lfs -text
*.exr filter=lfs diff=lfs merge=lfs -text
*.hdr filter=lfs diff=lfs merge=lfs -text
*.bmp filter=lfs diff=lfs merge=lfs -text
*.gif filter=lfs diff=lfs merge=lfs -text

# Audio
*.wav filter=lfs diff=lfs merge=lfs -text
*.mp3 filter=lfs diff=lfs merge=lfs -text
*.ogg filter=lfs diff=lfs merge=lfs -text
*.aif filter=lfs diff=lfs merge=lfs -text
*.aiff filter=lfs diff=lfs merge=lfs -text

# Video
*.mp4 filter=lfs diff=lfs merge=lfs -text
*.mov filter=lfs diff=lfs merge=lfs -text
*.avi filter=lfs diff=lfs merge=lfs -text
*.webm filter=lfs diff=lfs merge=lfs -text

# Fonts
*.ttf filter=lfs diff=lfs merge=lfs -text
*.otf filter=lfs diff=lfs merge=lfs -text

# Unity specific
*.unitypackage filter=lfs diff=lfs merge=lfs -text
# NOTE: .asset, .prefab, .cubemap are text-serializable YAML when Force Text
# serialization is enabled. Do NOT LFS-track them — it breaks text/semantic diff.

# Compiled / binary
*.dll filter=lfs diff=lfs merge=lfs -text
*.so filter=lfs diff=lfs merge=lfs -text
*.a filter=lfs diff=lfs merge=lfs -text
*.dylib filter=lfs diff=lfs merge=lfs -text
*.exe filter=lfs diff=lfs merge=lfs -text

# Compressed
*.zip filter=lfs diff=lfs merge=lfs -text
*.7z filter=lfs diff=lfs merge=lfs -text
*.gz filter=lfs diff=lfs merge=lfs -text
*.rar filter=lfs diff=lfs merge=lfs -text
*.tar filter=lfs diff=lfs merge=lfs -text
"#;
    std::fs::write(
        std::path::Path::new(&cwd).join(".gitattributes"),
        gitattributes,
    )
    .map_err(|e| format!("Failed to write .gitattributes: {}", e))?;

    let _ = command("git")
        .args(["lfs", "install"])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();

    let _ = command("git")
        .args(["add", ".gitignore", ".gitattributes"])
        .current_dir(&cwd)
        .output();
    let _ = command("git")
        .args(["commit", "-m", "Initial commit: Unity .gitignore + Git LFS"])
        .current_dir(&cwd)
        .output();

    Ok("Git repository initialized with Unity .gitignore and Git LFS configuration.".to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitUserConfig {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GitConfigScope {
    Repo,
    Global,
}

impl GitConfigScope {
    fn flag(self) -> &'static str {
        match self {
            GitConfigScope::Repo => "--local",
            GitConfigScope::Global => "--global",
        }
    }

    fn label(self) -> &'static str {
        match self {
            GitConfigScope::Repo => "repo",
            GitConfigScope::Global => "global",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConfigEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConfigScopeSnapshot {
    pub scope: GitConfigScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub entries: Vec<GitConfigEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitConfigSnapshot {
    pub repo: GitConfigScopeSnapshot,
    pub global: GitConfigScopeSnapshot,
}

fn read_git_config_value(
    args: &[&str],
    cwd: Option<&str>,
    label: &str,
) -> Result<String, AppError> {
    let mut cmd = command("git");
    cmd.args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = cwd.filter(|value| !value.trim().is_empty()) {
        cmd.current_dir(dir);
    }

    let output = cmd.output().map_err(|e| format!("{}: {}", label, e))?;

    if !output.status.success() {
        return Ok(String::new());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git_config(
    scope: GitConfigScope,
    cwd: Option<&str>,
    args: &[&str],
    label: &str,
) -> Result<std::process::Output, AppError> {
    let mut cmd = command("git");
    cmd.arg("config")
        .arg(scope.flag())
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(dir) = cwd.filter(|value| !value.trim().is_empty()) {
        cmd.current_dir(dir);
    }

    cmd.output()
        .map_err(|e| AppError::new("git.config.exec", format!("{}: {}", label, e)))
}

fn parse_git_config_list(output: &[u8]) -> Vec<GitConfigEntry> {
    String::from_utf8_lossy(output)
        .split('\0')
        .filter_map(|raw| {
            let raw = raw.trim_end_matches('\r');
            if raw.is_empty() {
                return None;
            }

            if let Some((key, value)) = raw.split_once('\n') {
                return Some(GitConfigEntry {
                    key: key.trim().to_string(),
                    value: value.to_string(),
                });
            }

            raw.split_once('=').map(|(key, value)| GitConfigEntry {
                key: key.trim().to_string(),
                value: value.to_string(),
            })
        })
        .filter(|entry| !entry.key.is_empty())
        .collect()
}

fn read_git_config_origin(scope: GitConfigScope, cwd: Option<&str>) -> Option<String> {
    let output = run_git_config(
        scope,
        cwd,
        &["--show-origin", "--list"],
        "Failed to read git config origin",
    )
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().find_map(|line| {
        let source = line
            .split_once('\t')
            .map(|(origin, _)| origin)
            .or_else(|| line.split_once(' ').map(|(origin, _)| origin))?
            .trim();
        if source.is_empty() {
            return None;
        }
        Some(source.strip_prefix("file:").unwrap_or(source).to_string())
    })
}

fn read_git_config_entries(
    scope: GitConfigScope,
    cwd: Option<&str>,
) -> Result<Vec<GitConfigEntry>, AppError> {
    if scope == GitConfigScope::Repo && !cwd.map(is_git_repo_dir).unwrap_or(false) {
        return Err(AppError::new(
            "git.no_repo",
            "Current workspace is not a Git repository",
        ));
    }

    let label = format!("Failed to read {} git config", scope.label());
    let output = run_git_config(scope, cwd, &["--list", "--null"], &label)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if scope == GitConfigScope::Global
            && (stderr.is_empty()
                || stderr.contains("unable to read config file")
                || stderr.contains("No such file"))
        {
            return Ok(vec![]);
        }
        return Err(AppError::new(
            "git.config.read_failed",
            format!("{}: {}", label, stderr),
        ));
    }

    Ok(parse_git_config_list(&output.stdout))
}

fn read_git_config_scope_snapshot(
    scope: GitConfigScope,
    cwd: Option<&str>,
) -> Result<GitConfigScopeSnapshot, AppError> {
    Ok(GitConfigScopeSnapshot {
        scope,
        path: read_git_config_origin(scope, cwd),
        entries: read_git_config_entries(scope, cwd)?,
    })
}

fn is_safe_git_config_key(key: &str) -> bool {
    if key.is_empty() || key.starts_with('-') || !key.contains('.') {
        return false;
    }

    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':'))
}

fn normalize_git_config_entries(
    entries: Vec<GitConfigEntry>,
) -> Result<Vec<GitConfigEntry>, AppError> {
    let mut normalized = Vec::with_capacity(entries.len());

    for entry in entries {
        let key = entry.key.trim().to_string();
        if !is_safe_git_config_key(&key) {
            return Err(AppError::new(
                "git.config.invalid_key",
                format!("Invalid git config key: {}", entry.key),
            ));
        }
        if key.contains('\0') || entry.value.contains('\0') || key.contains('\n') {
            return Err(AppError::new(
                "git.config.invalid_value",
                "Git config keys and values must not contain NUL characters",
            ));
        }
        normalized.push(GitConfigEntry {
            key,
            value: entry.value,
        });
    }

    Ok(normalized)
}

fn write_git_config_entries(
    scope: GitConfigScope,
    cwd: Option<&str>,
    entries: Vec<GitConfigEntry>,
) -> Result<GitConfigScopeSnapshot, AppError> {
    if scope == GitConfigScope::Repo && !cwd.map(is_git_repo_dir).unwrap_or(false) {
        return Err(AppError::new(
            "git.no_repo",
            "Current workspace is not a Git repository",
        ));
    }

    let entries = normalize_git_config_entries(entries)?;
    let existing = read_git_config_entries(scope, cwd)?;
    let mut keys = std::collections::BTreeSet::new();

    for entry in existing.iter().chain(entries.iter()) {
        keys.insert(entry.key.clone());
    }

    for key in keys {
        let _ = run_git_config(
            scope,
            cwd,
            &["--unset-all", key.as_str()],
            "Failed to unset git config key",
        );
    }

    for entry in &entries {
        let output = run_git_config(
            scope,
            cwd,
            &["--add", entry.key.as_str(), entry.value.as_str()],
            "Failed to write git config key",
        )?;
        if !output.status.success() {
            return Err(AppError::new(
                "git.config.write_failed",
                format!(
                    "Failed to write {}: {}",
                    entry.key,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
    }

    read_git_config_scope_snapshot(scope, cwd)
}

fn is_git_repo_dir(cwd: &str) -> bool {
    if cwd.trim().is_empty() {
        return false;
    }

    command("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
        .unwrap_or(false)
}

fn read_git_user_config_value(cwd: Option<&str>, key: &str) -> Result<String, AppError> {
    let label = format!("Failed to read git {}", key);

    if let Some(dir) = cwd.filter(|value| !value.trim().is_empty()) {
        if is_git_repo_dir(dir) {
            let value = read_git_config_value(&["config", "--get", key], Some(dir), &label)?;
            if !value.is_empty() {
                return Ok(value);
            }
        }
    }

    let value = read_git_config_value(&["config", "--global", "--get", key], None, &label)?;
    if !value.is_empty() {
        return Ok(value);
    }

    read_git_config_value(&["config", "--system", "--get", key], None, &label)
}

#[tauri::command]
pub async fn git_check_user_config(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<GitUserConfig, AppError> {
    let cwd = workspace.path.read().await.clone();
    let cwd = if cwd.trim().is_empty() {
        None
    } else {
        Some(cwd.as_str())
    };

    let name = read_git_user_config_value(cwd, "user.name")?;
    let email = read_git_user_config_value(cwd, "user.email")?;

    Ok(GitUserConfig { name, email })
}

#[tauri::command]
pub async fn git_config_snapshot(
    workspace: State<'_, Arc<Workspace>>,
) -> Result<GitConfigSnapshot, AppError> {
    let cwd = workspace.path.read().await.clone();
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return Err(AppError::new(
            "git.no_workspace",
            "No working directory set",
        ));
    }

    Ok(GitConfigSnapshot {
        repo: read_git_config_scope_snapshot(GitConfigScope::Repo, Some(cwd))?,
        global: read_git_config_scope_snapshot(GitConfigScope::Global, None)?,
    })
}

#[tauri::command]
pub async fn git_save_config(
    workspace: State<'_, Arc<Workspace>>,
    scope: GitConfigScope,
    entries: Vec<GitConfigEntry>,
) -> Result<GitConfigScopeSnapshot, AppError> {
    let cwd = workspace.path.read().await.clone();
    let cwd = cwd.trim();
    let cwd = if scope == GitConfigScope::Repo {
        if cwd.is_empty() {
            return Err(AppError::new(
                "git.no_workspace",
                "No working directory set",
            ));
        }
        Some(cwd)
    } else {
        None
    };

    write_git_config_entries(scope, cwd, entries)
}

#[tauri::command]
pub async fn git_set_user_config(name: String, email: String) -> Result<(), AppError> {
    let n = name.trim().to_string();
    let e = email.trim().to_string();
    if n.is_empty() || e.is_empty() {
        return Err("Name and email must not be empty".to_string().into());
    }

    let out = command("git")
        .args(["config", "--global", "user.name", &n])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to set user.name: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "Failed to set user.name: {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }

    let out = command("git")
        .args(["config", "--global", "user.email", &e])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to set user.email: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "Failed to set user.email: {}",
            String::from_utf8_lossy(&out.stderr)
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod git_name_status_tests {
    use super::{
        build_commit_files_args, build_compare_files_args, load_rename_aware_unstaged_changes,
        parse_name_status_lines,
    };
    use crate::process_util::command;
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
    fn parse_name_status_lines_preserves_rename_old_path() {
        let files = parse_name_status_lines("R100\tAssets/Old.prefab\tAssets/New.prefab\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].path, "Assets/New.prefab");
        assert_eq!(files[0].old_path.as_deref(), Some("Assets/Old.prefab"));
    }

    #[test]
    fn commit_file_args_enable_rename_detection() {
        let commit_args = build_commit_files_args("abc123", false);
        assert!(commit_args.iter().any(|arg| arg == "--find-renames"));

        let stash_args = build_commit_files_args("stash@{0}", true);
        assert!(stash_args.iter().any(|arg| arg == "--find-renames"));
    }

    #[test]
    fn compare_file_args_enable_rename_detection() {
        let args = build_compare_files_args("a..b");
        assert!(args.iter().any(|arg| arg == "--find-renames"));
    }

    #[test]
    fn rename_aware_unstaged_detects_plain_worktree_rename() {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);

        std::fs::write(repo.path().join("old.txt"), "base\n").expect("write old file");
        git(repo.path(), &["add", "old.txt"]);
        git(repo.path(), &["commit", "-m", "init"]);

        std::fs::rename(repo.path().join("old.txt"), repo.path().join("new.txt"))
            .expect("rename file");

        let files = load_rename_aware_unstaged_changes(repo.path().to_str().expect("repo path"))
            .expect("rename-aware unstaged changes");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, "R");
        assert_eq!(files[0].path, "new.txt");
        assert_eq!(files[0].old_path.as_deref(), Some("old.txt"));
    }
}

#[cfg(test)]
mod git_path_block_tests {
    use super::{detect_windows_blocked_segment, parse_git_status_porcelain, GitBlockedPathReason};

    #[test]
    fn detects_reserved_windows_device_names_with_extensions() {
        let blocked = detect_windows_blocked_segment("Assets/NUL.txt")
            .expect("reserved device names should be blocked");
        assert!(matches!(
            blocked.0,
            GitBlockedPathReason::WindowsReservedName
        ));
        assert_eq!(blocked.1, "NUL.txt");

        let blocked = detect_windows_blocked_segment("Assets/com1.asset")
            .expect("COM1 aliases should be blocked");
        assert!(matches!(
            blocked.0,
            GitBlockedPathReason::WindowsReservedName
        ));
        assert_eq!(blocked.1, "com1.asset");
    }

    #[test]
    fn detects_windows_trailing_dot_and_space_segments() {
        let blocked = detect_windows_blocked_segment("Assets/foo.")
            .expect("trailing dot segments should be blocked");
        assert!(matches!(
            blocked.0,
            GitBlockedPathReason::WindowsTrailingDot
        ));
        assert_eq!(blocked.1, "foo.");

        let blocked = detect_windows_blocked_segment("Assets/bar ")
            .expect("trailing space segments should be blocked");
        assert!(matches!(
            blocked.0,
            GitBlockedPathReason::WindowsTrailingSpace
        ));
        assert_eq!(blocked.1, "bar ");
    }

    #[test]
    fn parse_status_keeps_reserved_names_out_of_stageable_list_on_windows() {
        if !cfg!(target_os = "windows") {
            return;
        }

        let parsed = parse_git_status_porcelain("? nul\0");
        assert!(parsed.unstaged.is_empty());
        assert_eq!(parsed.blocked.len(), 1);
        assert_eq!(parsed.blocked[0].path, "nul");
    }
}

#[cfg(test)]
mod git_user_config_tests {
    use super::read_git_user_config_value;
    use crate::process_util::command;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use tempfile::tempdir;

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct GlobalConfigGuard {
        previous: Option<String>,
    }

    impl GlobalConfigGuard {
        fn new(path: &Path) -> Self {
            let previous = std::env::var("GIT_CONFIG_GLOBAL").ok();
            std::env::set_var("GIT_CONFIG_GLOBAL", path);
            Self { previous }
        }
    }

    impl Drop for GlobalConfigGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var("GIT_CONFIG_GLOBAL", value);
            } else {
                std::env::remove_var("GIT_CONFIG_GLOBAL");
            }
        }
    }

    fn git(cwd: &Path, args: &[&str]) {
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

    fn git_global(args: &[&str]) {
        let output = command("git")
            .args(args)
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
    fn read_git_user_config_prefers_repo_local_values() {
        let _lock = env_lock().lock().expect("env lock");
        let global_home = tempdir().expect("global temp dir");
        let global_config = global_home.path().join(".gitconfig");
        let _guard = GlobalConfigGuard::new(&global_config);

        git_global(&["config", "--global", "user.name", "Global User"]);
        git_global(&["config", "--global", "user.email", "global@example.com"]);

        let repo = tempdir().expect("repo temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Repo User"]);
        git(repo.path(), &["config", "user.email", "repo@example.com"]);

        let cwd = repo.path().to_str().expect("repo path");
        let name = read_git_user_config_value(Some(cwd), "user.name").expect("name");
        let email = read_git_user_config_value(Some(cwd), "user.email").expect("email");

        assert_eq!(name, "Repo User");
        assert_eq!(email, "repo@example.com");
    }

    #[test]
    fn read_git_user_config_falls_back_to_global_outside_repo() {
        let _lock = env_lock().lock().expect("env lock");
        let global_home = tempdir().expect("global temp dir");
        let global_config = global_home.path().join(".gitconfig");
        let _guard = GlobalConfigGuard::new(&global_config);

        git_global(&["config", "--global", "user.name", "Global User"]);
        git_global(&["config", "--global", "user.email", "global@example.com"]);

        let project = tempdir().expect("project temp dir");
        let cwd = project.path().to_str().expect("project path");

        let name = read_git_user_config_value(Some(cwd), "user.name").expect("name");
        let email = read_git_user_config_value(Some(cwd), "user.email").expect("email");

        assert_eq!(name, "Global User");
        assert_eq!(email, "global@example.com");
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCommandResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[tauri::command]
pub async fn run_command(
    workspace: State<'_, Arc<Workspace>>,
    command: String,
) -> Result<RunCommandResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }

    let (shell, flag) = if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let mut output = async_command(shell);
    output
        .arg(flag)
        .arg(&command)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(path) = augment_path_with_git(std::env::var_os("PATH")) {
        output.env("PATH", path);
    }

    let output = output
        .output()
        .await
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    Ok(RunCommandResult {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

#[tauri::command]
pub async fn git_commit(
    message: String,
    description: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }
    if message.trim().is_empty() {
        return Err("Commit message cannot be empty".to_string().into());
    }

    // Build full commit message: title + optional description
    let full_message = if let Some(desc) = description.filter(|d| !d.trim().is_empty()) {
        format!("{}\n\n{}", message.trim(), desc.trim())
    } else {
        message.trim().to_string()
    };

    let output = command("git")
        .args(["commit", "-m", &full_message])
        .current_dir(&cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("git commit failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git commit failed: {}", stderr).into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiCommitMessage {
    pub title: String,
    pub description: String,
}

const COMMIT_DIFF_CHAR_LIMIT: usize = 8_000;
const COMMIT_SECTION_CHAR_LIMIT: usize = 1_500;
const COMMIT_STYLE_LOG_DEPTH: usize = 50;
const COMMIT_STYLE_SAMPLE_LIMIT: usize = 8;

fn run_git_stdout(cwd: &str, args: &[&str], label: &str) -> Result<String, AppError> {
    let output = command("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("{} failed: {}", label, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("{} failed: {}", label, stderr).into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_git_stdout_optional(cwd: &str, args: &[&str]) -> String {
    command("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default()
}

fn truncate_for_prompt(text: &str, max_chars: usize, label: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(none)".to_string();
    }

    let total_chars = trimmed.chars().count();
    if total_chars <= max_chars {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(max_chars).collect();
    format!(
        "{}\n\n... ({} truncated, {} chars total)",
        truncated.trim_end(),
        label,
        total_chars
    )
}

fn is_low_signal_commit_subject(subject: &str) -> bool {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        return true;
    }

    let normalized: String = trimmed
        .chars()
        .filter(|c| c.is_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(c))
        .flat_map(|c| c.to_lowercase())
        .collect();

    if normalized.is_empty() {
        return true;
    }

    if normalized.chars().count() <= 2 {
        return true;
    }

    let mut chars = normalized.chars();
    if let Some(first) = chars.next() {
        if chars.clone().all(|c| c == first) {
            return true;
        }
    }

    let is_single_ascii_token = trimmed.split_whitespace().count() == 1
        && normalized.is_ascii()
        && !trimmed.contains(':')
        && !trimmed.contains('：');
    if is_single_ascii_token && normalized.chars().count() <= 4 {
        return true;
    }

    matches!(
        normalized.as_str(),
        "asd" | "aas" | "ads" | "assd" | "test" | "test1" | "tmp" | "temp" | "wip"
    )
}

fn collect_commit_style_samples(cwd: &str) -> Vec<String> {
    let depth = COMMIT_STYLE_LOG_DEPTH.to_string();
    let raw = run_git_stdout_optional(cwd, &["log", "--format=%s", "-n", &depth]);

    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !is_low_signal_commit_subject(line))
        .take(COMMIT_STYLE_SAMPLE_LIMIT)
        .map(|line| line.to_string())
        .collect()
}

fn build_commit_generation_context(cwd: &str, diff_stat: &str, diff_detail: &str) -> String {
    let branch = run_git_stdout_optional(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let branch = branch.trim();
    let branch = if branch.is_empty() {
        "HEAD".to_string()
    } else {
        branch.to_string()
    };

    let recent_samples = collect_commit_style_samples(cwd);
    let recent_samples = if recent_samples.is_empty() {
        "(no high-signal recent commit titles found)".to_string()
    } else {
        recent_samples
            .into_iter()
            .map(|line| format!("- {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let staged_name_status = run_git_stdout_optional(
        cwd,
        &["diff", "--cached", "--name-status", "--find-renames"],
    );
    let staged_numstat =
        run_git_stdout_optional(cwd, &["diff", "--cached", "--numstat", "--find-renames"]);
    let staged_summary =
        run_git_stdout_optional(cwd, &["diff", "--cached", "--summary", "--find-renames"]);

    [
        format!("## Current Branch\n{}", branch),
        format!("## Recent Commit Title Samples\n{}", recent_samples),
        format!(
            "## Staged Files (name-status)\n{}",
            truncate_for_prompt(
                &staged_name_status,
                COMMIT_SECTION_CHAR_LIMIT,
                "staged file list"
            )
        ),
        format!(
            "## Staged Files (numstat)\n{}",
            truncate_for_prompt(&staged_numstat, COMMIT_SECTION_CHAR_LIMIT, "staged numstat")
        ),
        format!(
            "## Staged Summary\n{}",
            truncate_for_prompt(&staged_summary, COMMIT_SECTION_CHAR_LIMIT, "staged summary")
        ),
        format!(
            "## Staged Diff Stat\n{}",
            truncate_for_prompt(diff_stat, COMMIT_SECTION_CHAR_LIMIT, "diff stat")
        ),
        format!(
            "## Staged Diff Detail\n{}",
            truncate_for_prompt(diff_detail, COMMIT_DIFF_CHAR_LIMIT, "staged diff")
        ),
    ]
    .join("\n\n")
}

fn strip_description_marker(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let trimmed = trimmed
        .trim_start_matches('-')
        .trim_start_matches('*')
        .trim_start_matches('•')
        .trim();

    let digit_prefix_len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_prefix_len > 0 {
        let remainder = trimmed[digit_prefix_len..].trim_start();
        for marker in [".", ")", "、"] {
            if let Some(rest) = remainder.strip_prefix(marker) {
                return rest.trim_start().to_string();
            }
        }
    }

    trimmed.to_string()
}

fn normalize_commit_title(title: &str) -> String {
    let first_line = title
        .trim()
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(title);
    first_line
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

fn normalize_commit_description(description: &str) -> String {
    let trimmed = description
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut raw_points: Vec<String> = trimmed
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    if raw_points.len() <= 1 {
        raw_points = trimmed
            .replace("。", "。\n")
            .replace("；", "；\n")
            .replace("; ", ";\n")
            .replace(". ", ".\n")
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
    }

    let mut bullets = Vec::new();
    for raw in raw_points {
        let point = strip_description_marker(&raw);
        if point.is_empty() || bullets.iter().any(|existing: &String| existing == &point) {
            continue;
        }
        bullets.push(point);
        if bullets.len() >= 5 {
            break;
        }
    }

    bullets
        .into_iter()
        .map(|line| format!("- {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Default)]
struct GitClaudeSdkHost {
    streamed_text: String,
    last_assistant: Option<ClaudeSdkAssistantMessage>,
}

impl ClaudeSdkHost for GitClaudeSdkHost {
    fn on_text_delta(&mut self, delta: String) {
        self.streamed_text.push_str(&delta);
    }

    fn on_thinking_delta(&mut self, _delta: String) {}

    fn on_tool_call_start(&mut self, _tool_call_id: String, _tool_name: String) {}

    fn on_assistant_message(&mut self, message: ClaudeSdkAssistantMessage) -> Result<(), String> {
        self.last_assistant = Some(message);
        Ok(())
    }

    fn execute_tool<'a>(
        &'a mut self,
        _request_id: &'a str,
        tool_name: &'a str,
        _arguments: serde_json::Value,
    ) -> ClaudeSdkHostFuture<'a> {
        Box::pin(async move {
            ToolResult {
                output: format!(
                    "Tool '{}' is not available while generating git commit messages.",
                    tool_name
                ),
                is_error: true,
            }
        })
    }
}

async fn generate_commit_message_with_anthropic_sdk(
    cwd: &str,
    selected_model: &str,
    user_prompt: &str,
    debug: bool,
) -> Result<String, AppError> {
    let mut host = GitClaudeSdkHost::default();
    let turn = anthropic_agent_sdk::run_turn(
        ClaudeCodeSdkOptions {
            locus_session_id: format!("git_commit_{}", uuid::Uuid::new_v4()),
            cwd: cwd.to_string(),
            system_prompt: "You are a git commit message generator.".to_string(),
            model: selected_model.to_string(),
            resume_session_id: None,
            server_name: "locus".to_string(),
            tools: Vec::new(),
            debug,
        },
        serde_json::json!({
            "role": "user",
            "content": user_prompt,
        }),
        &mut host,
    )
    .await
    .map_err(|e| format!("Anthropic Agent SDK failed: {}", e))?;

    let response_text = host
        .last_assistant
        .as_ref()
        .map(|message| message.text.trim().to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            let text = turn.final_text.trim().to_string();
            (!text.is_empty()).then_some(text)
        })
        .unwrap_or_else(|| host.streamed_text.trim().to_string());

    if response_text.is_empty() {
        return Err(
            "Anthropic Agent SDK returned an empty commit message response"
                .to_string()
                .into(),
        );
    }

    Ok(response_text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitMessageModelProvider {
    OpenRouter,
    OpenAiCodex,
    AnthropicSdk,
    AnthropicDirect,
}

fn classify_commit_message_model_provider(
    selected_model: &str,
) -> Option<CommitMessageModelProvider> {
    let selected_model = selected_model.trim();
    if selected_model.is_empty() {
        return None;
    }

    if selected_model.starts_with("openrouter/") {
        return Some(CommitMessageModelProvider::OpenRouter);
    }

    if selected_model.starts_with("openai/") {
        return Some(CommitMessageModelProvider::OpenAiCodex);
    }

    if selected_model.starts_with("anthropic_sdk/") {
        return Some(CommitMessageModelProvider::AnthropicSdk);
    }

    if !selected_model.contains('/') {
        return Some(CommitMessageModelProvider::AnthropicDirect);
    }

    None
}

fn is_codex_unauthorized_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("401 unauthorized")
        || lower.contains("http error: 401")
        || lower.contains("api error (401")
}

async fn resolve_codex_commit_request_auth(
    auth: &CodexAuthStateHandle,
    force_refresh: bool,
) -> Result<(String, Option<String>), String> {
    let mut guard = auth.lock().await;
    if force_refresh {
        guard.retry_validation().await?;
    }
    let access_token = guard.access_token().await?;
    Ok((access_token, guard.account_id()))
}

#[tauri::command]
pub async fn git_generate_commit_message(
    model: Option<String>,
    workspace: State<'_, Arc<Workspace>>,
    config: State<'_, Arc<AppConfig>>,
    auth: State<'_, Arc<tokio::sync::Mutex<AuthState>>>,
    api_key_state: State<'_, ApiKeyState>,
    _provider_keys: State<'_, ProviderKeysState>,
    codex: State<'_, CodexAuthStateHandle>,
) -> Result<AiCommitMessage, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err("No working directory set".to_string().into());
    }

    let diff_stat = run_git_stdout(
        &cwd,
        &["diff", "--cached", "--stat"],
        "git diff --cached --stat",
    )?;
    let diff_detail = run_git_stdout(
        &cwd,
        &["diff", "--cached", "--no-color", "--unified=1"],
        "git diff --cached",
    )?;

    if diff_stat.trim().is_empty() && diff_detail.trim().is_empty() {
        return Err("No staged changes to generate commit message for"
            .to_string()
            .into());
    }

    let workspace_prompt_path = std::path::Path::new(&cwd).join("Locus/prompt/commit-message.md");
    let prompt_template = if workspace_prompt_path.exists() {
        std::fs::read_to_string(&workspace_prompt_path)
            .unwrap_or_else(|_| crate::prompt::commit::COMMIT_MESSAGE.to_string())
    } else {
        crate::prompt::commit::COMMIT_MESSAGE.to_string()
    };

    let prompt_context = build_commit_generation_context(&cwd, &diff_stat, &diff_detail);
    let user_prompt = prompt_template.replace("{{diff}}", &prompt_context);

    let user_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: MessageRole::User,
        content: user_prompt.clone(),
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        prompt_prefix: None,
        prompt_suffix: None,
        response_id: None,
        content_order: None,
        thinking_order: None,
        tool_calls: None,
        tool_call_id: None,
        images: None,
        asset_refs: None,
        thinking_content: None,
        thinking_duration: None,
        thinking_signature: None,
        knowledge_proposal: None,
        render_parts: None,
    };

    let history = vec![user_msg];

    let selected_model = model.as_deref().unwrap_or(&config.model);
    let provider = classify_commit_message_model_provider(selected_model);

    let response_text = if matches!(provider, Some(CommitMessageModelProvider::OpenRouter)) {
        let api_key = api_key_state.read().await.clone();
        if api_key.is_empty() {
            return Err("OpenRouter API key not configured".to_string().into());
        }
        let api_model = crate::agent::instance::resolve_openrouter_model(selected_model);
        let resp = crate::llm::openrouter::stream_chat(
            &api_key,
            &api_model,
            "You are a git commit message generator.",
            &history,
            &[],
            config.base_url.as_deref(),
            None,
            Some("OpenRouter"),
            &[],
            None,
            config.debug_enabled(),
            |_| {},
            |_, _| {},
        )
        .await?;
        resp.text
    } else if matches!(provider, Some(CommitMessageModelProvider::OpenAiCodex)) {
        let transport = crate::commands::load_codex_model_config()
            .map(|config| config.transport)
            .unwrap_or_default();
        let actual_model = selected_model
            .trim()
            .strip_prefix("openai/")
            .unwrap_or(selected_model);
        let session_id = format!("git_commit_{}", uuid::Uuid::new_v4());
        let mut turn_state = crate::llm::codex::TurnState::default();
        let (access_token, account_id) = resolve_codex_commit_request_auth(&codex, false)
            .await
            .map_err(|e| format!("OpenAI Codex token failed (please re-login): {}", e))?;
        let resp = match crate::llm::codex::stream_chat(
            &access_token,
            account_id.as_deref(),
            transport,
            config.base_url.as_deref(),
            actual_model,
            "You are a git commit message generator.",
            &history,
            &[],
            None,
            config.debug_enabled(),
            Some(&session_id),
            None,
            &mut turn_state,
            &|_| {},
            &|_| {},
            &|_, _| {},
        )
        .await
        {
            Ok(resp) => resp,
            Err(error) if is_codex_unauthorized_error(&error) => {
                let (access_token, account_id) = resolve_codex_commit_request_auth(&codex, true)
                    .await
                    .map_err(|e| format!("OpenAI Codex token refresh failed: {}", e))?;
                crate::llm::codex::stream_chat(
                    &access_token,
                    account_id.as_deref(),
                    transport,
                    config.base_url.as_deref(),
                    actual_model,
                    "You are a git commit message generator.",
                    &history,
                    &[],
                    None,
                    config.debug_enabled(),
                    Some(&session_id),
                    None,
                    &mut turn_state,
                    &|_| {},
                    &|_| {},
                    &|_, _| {},
                )
                .await?
            }
            Err(error) => return Err(error.into()),
        };
        resp.text
    } else if matches!(provider, Some(CommitMessageModelProvider::AnthropicSdk)) {
        generate_commit_message_with_anthropic_sdk(
            &cwd,
            selected_model,
            &user_prompt,
            config.debug_enabled(),
        )
        .await?
    } else if matches!(provider, Some(CommitMessageModelProvider::AnthropicDirect)) {
        let mut auth_guard = auth.lock().await;
        if !auth_guard.is_authenticated() {
            return Err("Not logged in to Anthropic, please log in from settings"
                .to_string()
                .into());
        }
        let token = auth_guard
            .access_token()
            .await
            .map_err(|e| format!("Anthropic token failed: {}", e))?;
        let user_metadata = auth_guard
            .claude_code_user_metadata()
            .map_err(|e| format!("Anthropic metadata failed: {}", e))?;
        let resp = crate::llm::anthropic::stream_chat(
            &token,
            selected_model,
            &user_metadata,
            &["You are a git commit message generator."],
            &history,
            &[],
            config.base_url.as_deref(),
            None,
            None,
            |_| {},
            |_| {},
            |_, _| {},
        )
        .await?;
        resp.text
    } else {
        return Err(format!("Unrecognized model provider: {}", selected_model).into());
    };

    let json_str = response_text.trim();
    let json_str = json_str
        .strip_prefix("```json")
        .or_else(|| json_str.strip_prefix("```"))
        .unwrap_or(json_str);
    let json_str = json_str.strip_suffix("```").unwrap_or(json_str).trim();

    match serde_json::from_str::<AiCommitMessage>(json_str) {
        Ok(msg) => {
            let title = normalize_commit_title(&msg.title);
            Ok(AiCommitMessage {
                title: if title.is_empty() {
                    "update".to_string()
                } else {
                    title
                },
                description: normalize_commit_description(&msg.description),
            })
        }
        Err(_) => Ok(AiCommitMessage {
            title: {
                let title =
                    normalize_commit_title(response_text.lines().next().unwrap_or("update"));
                if title.is_empty() {
                    "update".to_string()
                } else {
                    title
                }
            },
            description: normalize_commit_description(
                &response_text.lines().skip(1).collect::<Vec<_>>().join("\n"),
            ),
        }),
    }
}

#[cfg(test)]
mod commit_message_prompt_tests {
    use super::{
        classify_commit_message_model_provider, is_low_signal_commit_subject,
        normalize_commit_description, normalize_commit_title, CommitMessageModelProvider,
    };

    #[test]
    fn filters_placeholder_commit_subjects() {
        assert!(is_low_signal_commit_subject("asd"));
        assert!(is_low_signal_commit_subject("t"));
        assert!(is_low_signal_commit_subject("v2"));
        assert!(!is_low_signal_commit_subject("before split diff"));
        assert!(!is_low_signal_commit_subject("【项目】初始化"));
    }

    #[test]
    fn normalizes_description_into_bullets() {
        let desc = "1. 更新协作页提交弹窗\n2. 补充分支和 diff 上下文";
        assert_eq!(
            normalize_commit_description(desc),
            "- 更新协作页提交弹窗\n- 补充分支和 diff 上下文"
        );
    }

    #[test]
    fn keeps_commit_title_single_line() {
        assert_eq!(
            normalize_commit_title("`【项目】Chat`\n- extra"),
            "【项目】Chat"
        );
    }

    #[test]
    fn classifies_openai_codex_models_for_commit_generation() {
        assert_eq!(
            classify_commit_message_model_provider("openai/gpt-5.4"),
            Some(CommitMessageModelProvider::OpenAiCodex)
        );
    }
}

// ── Merge commands ──

#[tauri::command]
pub async fn git_merge_file(
    workspace: State<'_, Arc<Workspace>>,
    path: String,
    conflict_code: String,
    base_oid: String,
    left_oid: String,
    right_oid: String,
    is_lfs: bool,
) -> Result<crate::vcs::git_merge::MergeFileInfo, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }
    crate::vcs::git_merge::load_merge_file_info(
        &cwd,
        &path,
        &conflict_code,
        &base_oid,
        &left_oid,
        &right_oid,
        is_lfs,
    )
}

#[tauri::command]
pub async fn git_merge_apply(
    workspace: State<'_, Arc<Workspace>>,
    path: String,
    mode: crate::vcs::git_merge::MergeApplyMode,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }
    crate::vcs::git_merge::apply_merge_resolution(&cwd, &path, mode)
}

#[tauri::command]
pub async fn git_merge_action(
    workspace: State<'_, Arc<Workspace>>,
    action: crate::vcs::git_merge::MergeActionKind,
    operation_kind: String,
) -> Result<String, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }
    crate::vcs::git_merge::execute_merge_action(&cwd, action, &operation_kind).await
}

// ── Merge semantic commands ──

#[tauri::command]
pub async fn git_merge_semantic_session(
    app_handle: AppHandle,
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    request: crate::merge::types::MergeSessionRequest,
) -> Result<crate::merge::types::MergeSessionPayload, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }

    let key = crate::merge::session::merge_session_key(
        &request.file_path,
        &request.base_oid,
        &request.left_oid,
        &request.right_oid,
    );

    // Check cache first.
    if let Some(session_lock) = crate::merge::session::get_merge_session(&key) {
        let session = session_lock
            .read()
            .map_err(|_| AppError::new("merge.lock_error", "Failed to read merge session"))?;
        return Ok(build_merge_payload(&key, &request.file_path, &session));
    }

    // Build new session.
    let session = match crate::merge::session::build_merge_session(
        &cwd,
        &request.file_path,
        &request.base_oid,
        &request.left_oid,
        &request.right_oid,
        &ref_graph_state,
        Some(&app_handle),
    ) {
        Ok(session) => session,
        Err(error) => {
            return Ok(build_merge_unavailable_payload(
                &key,
                &request.file_path,
                error.message,
            ));
        }
    };

    let payload = build_merge_payload(&key, &request.file_path, &session);
    crate::merge::session::cache_merge_session(&key, session);
    Ok(payload)
}

#[tauri::command]
pub async fn git_merge_semantic_target(
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    request: crate::merge::types::MergeTargetRequest,
) -> Result<crate::merge::types::MergeTargetInspector, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }

    let session_lock =
        crate::merge::session::get_merge_session(&request.merge_key).ok_or_else(|| {
            AppError::new("merge.session_expired", "Merge session not found in cache")
        })?;

    let mut session = session_lock
        .write()
        .map_err(|_| AppError::new("merge.lock_error", "Failed to write merge session"))?;
    crate::merge::inspector::materialize_merge_target(
        &mut session,
        &request.target_id,
        &cwd,
        &ref_graph_state,
    )
}

fn read_merge_workspace_bytes(cwd: &str, file_path: &str) -> Option<Vec<u8>> {
    let full_path = std::path::Path::new(cwd).join(file_path);
    std::fs::read(&full_path).ok()
}

fn verify_merge_workspace_hash(
    session_lock: &std::sync::Arc<std::sync::RwLock<crate::merge::types::MergeSemanticSession>>,
    workspace_bytes: Option<&[u8]>,
) -> Result<(), AppError> {
    let current_hash = crate::merge::session::hash_workspace_bytes(workspace_bytes);
    let session_read = session_lock
        .read()
        .map_err(|_| AppError::new("merge.lock_error", "Failed to read merge session"))?;
    if current_hash != session_read.workspace_hash {
        return Err(AppError::new(
            "merge.workspace_modified",
            "Workspace file has been modified since the merge session was opened. Use text-mode resolution or reset the file first.",
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn git_merge_semantic_validate(
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    request: crate::merge::types::MergeApplyRequest,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }

    let workspace_bytes = read_merge_workspace_bytes(&cwd, &request.file_path);
    let session_lock =
        crate::merge::session::get_merge_session(&request.merge_key).ok_or_else(|| {
            AppError::new("merge.session_expired", "Merge session not found in cache")
        })?;
    verify_merge_workspace_hash(&session_lock, workspace_bytes.as_deref())?;

    let mut session = session_lock
        .write()
        .map_err(|_| AppError::new("merge.lock_error", "Failed to write merge session"))?;
    crate::merge::inspector::materialize_all_merge_targets(&mut session, &cwd, &ref_graph_state)?;
    crate::merge::patch::assemble_resolved_yaml(&session, &request.resolutions)?;
    Ok(())
}

#[tauri::command]
pub async fn git_merge_semantic_apply(
    workspace: State<'_, Arc<Workspace>>,
    ref_graph_state: State<'_, AssetDbState>,
    request: crate::merge::types::MergeApplyRequest,
) -> Result<(), AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "merge.no_workspace",
            "No working directory set",
        ));
    }

    // Guard: re-verify workspace file hasn't been modified since the merge
    // session was built. This prevents overwriting manual edits made outside
    // the app — including edits in non-conflict regions that the previous
    // conflict-block-only check would have missed.
    let workspace_bytes = read_merge_workspace_bytes(&cwd, &request.file_path);

    let session_lock =
        crate::merge::session::get_merge_session(&request.merge_key).ok_or_else(|| {
            AppError::new("merge.session_expired", "Merge session not found in cache")
        })?;

    // Compare workspace content hash with the snapshot taken at session build time.
    verify_merge_workspace_hash(&session_lock, workspace_bytes.as_deref())?;

    let mut session = session_lock
        .write()
        .map_err(|_| AppError::new("merge.lock_error", "Failed to write merge session"))?;

    crate::merge::inspector::materialize_all_merge_targets(&mut session, &cwd, &ref_graph_state)?;

    let assembled = crate::merge::patch::assemble_resolved_yaml(&session, &request.resolutions)?;
    match assembled {
        crate::merge::patch::AssembledMerge::DeleteFile => {
            crate::vcs::git_merge::apply_merge_resolution(
                &cwd,
                &request.file_path,
                crate::vcs::git_merge::MergeApplyMode::TakeStage {
                    stage: "delete".into(),
                },
            )
        }
        crate::merge::patch::AssembledMerge::ResolvedText(resolved_text) => {
            let workspace_bytes = workspace_bytes.ok_or_else(|| {
                AppError::new(
                    "merge.workspace_modified",
                    "Workspace file has been deleted since the merge session was opened.",
                )
            })?;

            // Semantic merge normalizes to LF internally. When writing the resolved
            // file back, prefer the repository EOL rule and only fall back to the
            // current worktree style when no explicit rule exists.
            let workspace_text = String::from_utf8_lossy(&workspace_bytes);
            let line_ending = crate::eol::resolve_preferred_line_ending(
                Some(std::path::Path::new(&cwd)),
                std::path::Path::new(&request.file_path),
                Some(&workspace_text),
            );
            let mut final_text = crate::eol::apply_line_ending(&resolved_text, line_ending);
            // Ensure trailing newline — Unity YAML files always end with one.
            if !final_text.ends_with('\n') {
                final_text.push_str(line_ending.as_str());
            }

            // Write and stage the resolved file.
            crate::vcs::git_merge::apply_merge_resolution(
                &cwd,
                &request.file_path,
                crate::vcs::git_merge::MergeApplyMode::ResolvedText { text: final_text },
            )
        }
    }
}

fn build_merge_payload(
    key: &str,
    path: &str,
    session: &crate::merge::types::MergeSemanticSession,
) -> crate::merge::types::MergeSessionPayload {
    let default_target_id = session.targets.first().map(|t| t.id.clone());

    crate::merge::types::MergeSessionPayload {
        key: key.to_string(),
        file_path: path.to_string(),
        semantic_available: true,
        fallback_reason: None,
        asset_kind: Some(session.asset_kind.clone()),
        layout: Some(session.layout.clone()),
        summary: Some(session.summary.clone()),
        tree: Some(session.tree.clone()),
        targets: Some(session.targets.clone()),
        default_target_id,
        inspector: None, // Built on demand from the cached session locator.
    }
}

fn build_merge_unavailable_payload(
    key: &str,
    path: &str,
    fallback_reason: String,
) -> crate::merge::types::MergeSessionPayload {
    crate::merge::types::MergeSessionPayload {
        key: key.to_string(),
        file_path: path.to_string(),
        semantic_available: false,
        fallback_reason: Some(fallback_reason),
        asset_kind: None,
        layout: None,
        summary: None,
        tree: None,
        targets: None,
        default_target_id: None,
        inspector: None,
    }
}

// ── Context-menu actions ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitActionResult {
    pub status: String,
    pub message: String,
    pub stdout: String,
    pub stderr: String,
}

impl GitActionResult {
    fn success(msg: impl Into<String>, stdout: String, stderr: String) -> Self {
        Self {
            status: "success".into(),
            message: msg.into(),
            stdout,
            stderr,
        }
    }
    fn conflict(msg: impl Into<String>, stdout: String, stderr: String) -> Self {
        Self {
            status: "conflict".into(),
            message: msg.into(),
            stdout,
            stderr,
        }
    }
}

#[cfg(test)]
mod git_action_tests {
    use super::run_git_action;
    use crate::process_util::command;
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
    fn run_git_action_returns_error_for_non_conflict_failures() {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);

        std::fs::write(repo.path().join("README.md"), "base\n").expect("write readme");
        git(repo.path(), &["add", "README.md"]);
        git(repo.path(), &["commit", "-m", "init"]);

        let result = run_git_action(
            repo.path().to_str().expect("repo path"),
            &["switch", "missing-branch"],
            "切换分支",
        );

        assert!(result.is_err());
    }
}

#[cfg(test)]
mod git_discard_tests {
    use super::discard_git_file;
    use crate::eol::normalize_lf;
    use crate::process_util::command;
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

    fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo() -> tempfile::TempDir {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);

        std::fs::write(repo.path().join("old.txt"), "base\n").expect("write base file");
        git(repo.path(), &["add", "old.txt"]);
        git(repo.path(), &["commit", "-m", "init"]);

        repo
    }

    fn init_repo_with_gitattributes(attributes: &str) -> tempfile::TempDir {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);

        std::fs::write(repo.path().join(".gitattributes"), attributes).expect("write attributes");
        std::fs::write(repo.path().join("old.txt"), "base\n").expect("write base file");
        git(repo.path(), &["add", ".gitattributes", "old.txt"]);
        git(repo.path(), &["commit", "-m", "init"]);

        repo
    }

    #[test]
    fn discard_renamed_file_restores_old_path_from_worktree() {
        let repo = init_repo();
        std::fs::rename(repo.path().join("old.txt"), repo.path().join("new.txt"))
            .expect("rename file");

        discard_git_file(
            repo.path().to_str().expect("repo path"),
            "new.txt",
            "R",
            Some("old.txt"),
        )
        .expect("discard should succeed");

        assert!(repo.path().join("old.txt").exists());
        assert!(!repo.path().join("new.txt").exists());
        assert_eq!(
            normalize_lf(
                &std::fs::read_to_string(repo.path().join("old.txt")).expect("read restored file")
            ),
            "base\n"
        );
        assert_eq!(git_stdout(repo.path(), &["status", "--short"]), "");
    }

    #[test]
    fn discard_staged_renamed_file_restores_old_path_and_index() {
        let repo = init_repo();
        git(repo.path(), &["mv", "old.txt", "new.txt"]);

        discard_git_file(
            repo.path().to_str().expect("repo path"),
            "new.txt",
            "R",
            Some("old.txt"),
        )
        .expect("discard should succeed");

        assert!(repo.path().join("old.txt").exists());
        assert!(!repo.path().join("new.txt").exists());
        assert_eq!(
            normalize_lf(
                &std::fs::read_to_string(repo.path().join("old.txt")).expect("read restored file")
            ),
            "base\n"
        );
        assert_eq!(git_stdout(repo.path(), &["status", "--short"]), "");
    }

    #[test]
    fn discard_respects_repo_eol_rule_for_restored_file() {
        let repo = init_repo_with_gitattributes("* text=auto eol=lf\n");
        std::fs::rename(repo.path().join("old.txt"), repo.path().join("new.txt"))
            .expect("rename file");

        discard_git_file(
            repo.path().to_str().expect("repo path"),
            "new.txt",
            "R",
            Some("old.txt"),
        )
        .expect("discard should succeed");

        assert_eq!(
            std::fs::read(repo.path().join("old.txt")).expect("read restored bytes"),
            b"base\n"
        );
        assert_eq!(git_stdout(repo.path(), &["status", "--short"]), "");
    }

    #[test]
    fn discard_added_file_removes_worktree_and_index() {
        let repo = init_repo();
        std::fs::write(repo.path().join("added.txt"), "added\n").expect("write added file");
        git(repo.path(), &["add", "added.txt"]);

        discard_git_file(
            repo.path().to_str().expect("repo path"),
            "added.txt",
            "A",
            None,
        )
        .expect("discard should succeed");

        assert!(!repo.path().join("added.txt").exists());
        assert_eq!(git_stdout(repo.path(), &["status", "--short"]), "");
    }
}

#[cfg(test)]
mod git_config_tests {
    use super::{
        parse_git_config_list, read_git_config_entries, write_git_config_entries, GitConfigEntry,
        GitConfigScope,
    };
    use crate::process_util::command;
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

    fn git_stdout_lines(cwd: &std::path::Path, args: &[&str]) -> Vec<String> {
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
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn parse_git_config_list_reads_null_separated_entries() {
        let entries = parse_git_config_list(b"user.name\nJane\0remote.origin.fetch\n+refs/*\0");

        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.key.as_str(), entry.value.as_str()))
                .collect::<Vec<_>>(),
            vec![("user.name", "Jane"), ("remote.origin.fetch", "+refs/*")]
        );
    }

    #[test]
    fn write_git_config_entries_updates_values_and_preserves_multivalue_keys() {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Old User"]);
        git(repo.path(), &["config", "user.email", "old@example.com"]);
        git(
            repo.path(),
            &[
                "config",
                "--add",
                "remote.origin.fetch",
                "+refs/heads/main:refs/remotes/origin/main",
            ],
        );

        let mut entries = read_git_config_entries(
            GitConfigScope::Repo,
            Some(repo.path().to_str().expect("repo path")),
        )
        .expect("read config");

        for entry in &mut entries {
            if entry.key == "user.name" {
                entry.value = "New User".to_string();
            }
        }
        entries.push(GitConfigEntry {
            key: "remote.origin.fetch".to_string(),
            value: "+refs/tags/*:refs/tags/*".to_string(),
        });

        write_git_config_entries(
            GitConfigScope::Repo,
            Some(repo.path().to_str().expect("repo path")),
            entries,
        )
        .expect("write config");

        assert_eq!(
            git_stdout_lines(
                repo.path(),
                &["config", "--local", "--get-all", "user.name"],
            ),
            vec!["New User".to_string()]
        );
        assert_eq!(
            git_stdout_lines(
                repo.path(),
                &["config", "--local", "--get-all", "remote.origin.fetch"]
            ),
            vec![
                "+refs/heads/main:refs/remotes/origin/main".to_string(),
                "+refs/tags/*:refs/tags/*".to_string(),
            ]
        );
    }
}

#[cfg(test)]
mod graph_ref_tests {
    use super::{
        build_commit_ref_labels, build_visible_history_log_args, collect_graph_refs,
        collect_head_state, collect_stash_hidden_hashes, collect_stash_root_hashes,
        load_visible_history,
    };
    use crate::process_util::command;
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

    fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn collect_graph_refs_peels_annotated_tags_to_commit_hash() {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);
        git(repo.path(), &["config", "tag.gpgsign", "false"]);

        std::fs::write(repo.path().join("README.md"), "base\n").expect("write readme");
        git(repo.path(), &["add", "README.md"]);
        git(repo.path(), &["commit", "-m", "init"]);
        git(repo.path(), &["tag", "-a", "v1.0.0", "-m", "release"]);

        let head_hash = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
        let refs = collect_graph_refs(repo.path().to_str().expect("repo path"));
        let tag = refs
            .iter()
            .find(|entry| entry.full_name == "refs/tags/v1.0.0")
            .expect("annotated tag ref");

        assert_eq!(tag.target_hash, head_hash);
    }

    #[test]
    fn visible_history_walk_excludes_stash_refspace() {
        let args = build_visible_history_log_args(12, 128, true);

        assert!(args.iter().any(|arg| arg == "--branches"));
        assert!(args.iter().any(|arg| arg == "--remotes"));
        assert!(args.iter().any(|arg| arg == "--tags"));
        assert!(!args.iter().any(|arg| arg == "--all"));
        assert_eq!(args.last().map(String::as_str), Some("HEAD"));
    }

    #[test]
    fn load_visible_history_filters_stash_helper_commits() {
        let repo = tempdir().expect("temp dir");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "commit.gpgsign", "false"]);

        std::fs::write(repo.path().join("tracked.txt"), "base\n").expect("write tracked");
        std::fs::write(repo.path().join("other.txt"), "base\n").expect("write other");
        git(repo.path(), &["add", "tracked.txt", "other.txt"]);
        git(repo.path(), &["commit", "-m", "init"]);

        std::fs::write(repo.path().join("tracked.txt"), "base\nstaged\n").expect("write staged");
        std::fs::write(repo.path().join("other.txt"), "base\nunstaged\n").expect("write unstaged");
        git(repo.path(), &["add", "tracked.txt"]);
        git(repo.path(), &["stash", "push", "-m", "test stash"]);

        let stash_roots = collect_stash_root_hashes(repo.path().to_str().expect("repo path"));
        assert_eq!(stash_roots.len(), 1);

        let stash_hidden =
            collect_stash_hidden_hashes(repo.path().to_str().expect("repo path"), &stash_roots);
        assert!(!stash_hidden.is_empty());

        let head = collect_head_state(repo.path().to_str().expect("repo path"));
        let refs = collect_graph_refs(repo.path().to_str().expect("repo path"));
        let ref_labels = build_commit_ref_labels(&refs, &head);
        let page = load_visible_history(
            repo.path().to_str().expect("repo path"),
            0,
            20,
            head.hash.as_deref(),
            &ref_labels,
        )
        .expect("history page");

        assert_eq!(page.commits.len(), 1);
        assert_eq!(page.commits[0].message, "init");
        assert!(page
            .commits
            .iter()
            .all(|commit| !commit.message.starts_with("index on ")));
    }
}

fn run_git_action(cwd: &str, args: &[&str], label: &str) -> Result<GitActionResult, AppError> {
    let output = command("git")
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| AppError::new("git.exec", format!("{} failed: {}", label, e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        return Ok(GitActionResult::success(label, stdout, stderr));
    }

    // Check if this resulted in a real conflict state.
    let stderr_lower = stderr.to_lowercase();
    let has_unmerged = has_unmerged_entries(cwd);
    if stderr_lower.contains("conflict") || stderr_lower.contains("unmerged") {
        return Ok(GitActionResult::conflict(
            format!("{}: 存在冲突，请解决后继续", label),
            stdout,
            stderr,
        ));
    }

    // Also check sentinel files and the real index state for conflict flows.
    if detect_merge_operation(cwd, has_unmerged).is_some() {
        return Ok(GitActionResult::conflict(
            format!("{}: 存在冲突，请解决后继续", label),
            stdout,
            stderr,
        ));
    }

    Err(AppError::new(
        "git.action_failed",
        format!("{} failed: {}", label, stderr.trim()),
    ))
}

#[tauri::command]
pub async fn git_commit_action(
    workspace: State<'_, Arc<Workspace>>,
    rev: String,
    action: String,
    mode: Option<String>,
    branch_name: Option<String>,
) -> Result<GitActionResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "git.no_workspace",
            "No working directory set",
        ));
    }

    match action.as_str() {
        "cherryPick" => run_git_action(&cwd, &["cherry-pick", &rev], "Cherry-pick"),
        "checkoutDetached" => {
            run_git_action(&cwd, &["checkout", "--detach", &rev], "Checkout detached")
        }
        "reset" => {
            let m = mode.as_deref().unwrap_or("mixed");
            let flag = match m {
                "soft" => "--soft",
                "hard" => "--hard",
                _ => "--mixed",
            };
            run_git_action(&cwd, &["reset", flag, &rev], &format!("Reset ({})", m))
        }
        "revert" => run_git_action(&cwd, &["revert", "--no-edit", &rev], "Revert"),
        "createBranchAndCheckout" => {
            let name = branch_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::new("git.missing_param", "Branch name is required"))?;
            run_git_action(
                &cwd,
                &["checkout", "-b", name, &rev],
                &format!("创建并切换到分支 {}", name),
            )
        }
        _ => Err(AppError::new(
            "git.unknown_action",
            format!("Unknown commit action: {}", action),
        )),
    }
}

#[tauri::command]
pub async fn git_branch_action(
    workspace: State<'_, Arc<Workspace>>,
    target: String,
    target_kind: String,
    action: String,
    new_name: Option<String>,
) -> Result<GitActionResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "git.no_workspace",
            "No working directory set",
        ));
    }

    match action.as_str() {
        "switch" => run_git_action(
            &cwd,
            &["switch", &target],
            &format!("已切换到分支 {}", target),
        ),
        "checkoutTracking" => run_git_action(
            &cwd,
            &["checkout", "--track", &target],
            &format!("已检出跟踪分支 {}", target),
        ),
        "mergeIntoCurrent" => run_git_action(
            &cwd,
            &["merge", &target, "--no-edit"],
            &format!("已将 {} 合并到当前分支", target),
        ),
        "rebaseCurrentOnto" => run_git_action(
            &cwd,
            &["rebase", &target],
            &format!("已将当前分支变基到 {}", target),
        ),
        "rename" => {
            let name = new_name
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::new("git.missing_param", "New branch name is required"))?;
            run_git_action(
                &cwd,
                &["branch", "-m", &target, name],
                &format!("已重命名分支为 {}", name),
            )
        }
        "delete" => run_git_action(
            &cwd,
            &["branch", "-d", &target],
            &format!("已删除分支 {}", target),
        ),
        _ => Err(AppError::new(
            "git.unknown_action",
            format!("Unknown branch action: {}", action),
        )),
    }
}

#[tauri::command]
pub async fn git_stash_action(
    workspace: State<'_, Arc<Workspace>>,
    ref_name: String,
    action: String,
) -> Result<GitActionResult, AppError> {
    let cwd = workspace.path.read().await.clone();
    if cwd.is_empty() {
        return Err(AppError::new(
            "git.no_workspace",
            "No working directory set",
        ));
    }

    match action.as_str() {
        "apply" | "pop" => {
            let label = if action == "apply" {
                "已应用 Stash"
            } else {
                "已应用并移除 Stash"
            };
            crate::vcs::git_merge::prepare_stash_apply_abort_state(
                &cwd,
                &format!("stash {} {}", action, ref_name),
            )
            .await?;

            let args = if action == "apply" {
                ["stash", "apply", ref_name.as_str()]
            } else {
                ["stash", "pop", ref_name.as_str()]
            };
            let result = run_git_action(&cwd, &args, label);
            match &result {
                Ok(action_result) if action_result.status == "conflict" => {}
                _ => crate::vcs::git_merge::clear_stash_apply_abort_state(&cwd),
            }
            result
        }
        "drop" => run_git_action(
            &cwd,
            &["stash", "drop", &ref_name],
            &format!("已删除 {}", ref_name),
        ),
        _ => Err(AppError::new(
            "git.unknown_action",
            format!("Unknown stash action: {}", action),
        )),
    }
}
