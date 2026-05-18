use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use super::{Checkpoint, VcsChangedPath, VcsProvider, VcsRevisionRef};
use crate::process_util::{async_command, augment_path_with_git};

pub struct GitProvider;

struct TempGitIndex {
    path: PathBuf,
}

impl TempGitIndex {
    async fn create(working_dir: &str) -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("locus-git-index-{}.idx", uuid::Uuid::new_v4()));
        let repo_index_path = GitProvider::repo_index_path(working_dir).await?;

        if repo_index_path.is_file() {
            tokio::fs::copy(&repo_index_path, &path)
                .await
                .map_err(|e| {
                    format!(
                        "failed to copy git index '{}' -> '{}': {}",
                        repo_index_path.display(),
                        path.display(),
                        e
                    )
                })?;
        } else {
            tokio::fs::File::create(&path).await.map_err(|e| {
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

impl GitProvider {
    async fn git(working_dir: &str, args: &[&str]) -> Result<String, String> {
        Self::git_with_index(working_dir, args, None).await
    }

    fn format_git_failure(
        args: &[&str],
        status: std::process::ExitStatus,
        stderr: &[u8],
    ) -> String {
        let command = format!("git {}", args.join(" "));
        let stderr = String::from_utf8_lossy(stderr).trim().to_string();
        if stderr.is_empty() {
            format!("{} failed with status {}", command, status)
        } else {
            format!("{} failed: {}", command, stderr)
        }
    }

    fn format_workspace_snapshot_failure(working_dir: &str, label: &str, error: &str) -> String {
        let mut detail = format!(
            "[UndoManager] workspace snapshot failed; workspace='{}'; label='{}'; reason={}",
            working_dir, label, error
        );

        if Self::should_auto_remove_root_nul(error) {
            detail.push_str(
                "; hint=Git cannot index a root file named NUL on Windows. Remove or rename ./NUL and retry.",
            );
        }

        detail
    }

    fn log_workspace_snapshot_failure(working_dir: &str, label: &str, error: String) -> String {
        let detail = Self::format_workspace_snapshot_failure(working_dir, label, &error);
        eprintln!("{}", detail);
        detail
    }

    async fn git_with_index(
        working_dir: &str,
        args: &[&str],
        index: Option<&TempGitIndex>,
    ) -> Result<String, String> {
        let mut cmd = async_command("git");
        // Keep non-ASCII paths in UTF-8 instead of Git's quoted octal form.
        cmd.arg("-c")
            .arg("core.quotePath=false")
            .args(args)
            .current_dir(working_dir);
        cmd.env_remove("GIT_INDEX_FILE");
        if let Some(index) = index {
            cmd.env("GIT_INDEX_FILE", index.path());
        }
        let output = cmd
            .output()
            .await
            .map_err(|e| format!("failed to run git: {}", e))?;

        if !output.status.success() {
            return Err(Self::format_git_failure(
                args,
                output.status,
                &output.stderr,
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn repo_index_path(working_dir: &str) -> Result<PathBuf, String> {
        let raw = Self::git(working_dir, &["rev-parse", "--git-path", "index"]).await?;
        if raw.is_empty() {
            return Err("git rev-parse --git-path index returned empty output".to_string());
        }

        let path = PathBuf::from(&raw);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(Path::new(working_dir).join(path))
        }
    }

    // Capture the current working tree into a stash-style commit using a temporary
    // index so the real staged/untracked state stays untouched.
    async fn snapshot_worktree_once(working_dir: &str, label: &str) -> Result<String, String> {
        let temp_index = TempGitIndex::create(working_dir)
            .await
            .map_err(|error| format!("create temporary git index failed: {}", error))?;
        Self::git_with_index(working_dir, &["add", "-A"], Some(&temp_index))
            .await
            .map_err(|error| {
                format!("stage workspace into temporary git index failed: {}", error)
            })?;
        Self::git_with_index(
            working_dir,
            &["stash", "create", "-m", label],
            Some(&temp_index),
        )
        .await
        .map_err(|error| format!("create stash-style workspace snapshot failed: {}", error))
    }

    fn should_auto_remove_root_nul(error: &str) -> bool {
        if !cfg!(target_os = "windows") {
            return false;
        }

        let lower = error.to_ascii_lowercase();
        lower.contains("unable to index file 'nul'")
            || lower.contains("short read while indexing nul")
    }

    #[cfg(target_os = "windows")]
    async fn remove_root_nul_with_rm(working_dir: &str) -> Result<(), String> {
        let mut cmd = async_command("sh");
        cmd.arg("-lc")
            .arg("rm -f -- ./NUL ./nul")
            .current_dir(working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(path) = augment_path_with_git(std::env::var_os("PATH")) {
            cmd.env("PATH", path);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| format!("failed to run sh for root NUL cleanup: {}", e))?;

        if !output.status.success() {
            return Err(format!(
                "rm -f cleanup failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    async fn remove_root_nul_with_rm(_working_dir: &str) -> Result<(), String> {
        Ok(())
    }

    async fn snapshot_worktree(working_dir: &str, label: &str) -> Result<String, String> {
        match Self::snapshot_worktree_once(working_dir, label).await {
            Ok(sha) => Ok(sha),
            Err(error) if Self::should_auto_remove_root_nul(&error) => {
                eprintln!(
                    "[UndoManager] detected root NUL while snapshotting '{}'; attempting auto-remove via rm -f",
                    working_dir
                );
                if let Err(cleanup_error) = Self::remove_root_nul_with_rm(working_dir).await {
                    return Err(Self::log_workspace_snapshot_failure(
                        working_dir,
                        label,
                        format!("{} (auto-remove root NUL failed: {})", error, cleanup_error),
                    ));
                }

                eprintln!(
                    "[UndoManager] auto-remove root NUL completed for '{}'; retrying snapshot",
                    working_dir
                );
                Self::snapshot_worktree_once(working_dir, label)
                    .await
                    .map_err(|retry_error| {
                        Self::log_workspace_snapshot_failure(
                            working_dir,
                            label,
                            format!(
                                "{} (after auto-removing root NUL, retry failed: {})",
                                error, retry_error
                            ),
                        )
                    })
            }
            Err(error) => Err(Self::log_workspace_snapshot_failure(
                working_dir,
                label,
                error,
            )),
        }
    }

    // Capture the real index as a standalone tree so undo can restore staged state
    // without forcing previously untracked files into the index.
    async fn snapshot_index_tree(working_dir: &str) -> Result<Option<String>, String> {
        let temp_index = TempGitIndex::create(working_dir).await?;
        let tree = Self::git_with_index(working_dir, &["write-tree"], Some(&temp_index)).await?;
        if tree.is_empty() {
            Ok(None)
        } else {
            Ok(Some(tree))
        }
    }

    async fn empty_tree_id(working_dir: &str) -> Result<String, String> {
        let args = ["hash-object", "-t", "tree", "--stdin"];
        let mut cmd = async_command("git");
        cmd.args(args)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to run git: {}", e))?;
        drop(child.stdin.take());

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("failed to run git: {}", e))?;

        if !output.status.success() {
            return Err(Self::format_git_failure(
                &args,
                output.status,
                &output.stderr,
            ));
        }

        let tree_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if tree_id.is_empty() {
            Err("git hash-object -t tree --stdin returned empty output".to_string())
        } else {
            Ok(tree_id)
        }
    }
}

impl VcsProvider for GitProvider {
    async fn checkpoint(
        &self,
        working_dir: &str,
        label: &str,
    ) -> Result<Option<Checkpoint>, String> {
        let stash_msg = format!("locus checkpoint: {}", label);
        let sha = Self::snapshot_worktree(working_dir, &stash_msg).await?;
        let index_tree_id = match Self::snapshot_index_tree(working_dir).await {
            Ok(tree) => tree,
            Err(e) => {
                eprintln!(
                    "[UndoManager] failed to capture pre-round index snapshot for '{}': {}",
                    working_dir, e
                );
                None
            }
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        // When the working tree is clean, git stash create returns empty.
        // Use HEAD as the checkpoint so that after_round can still diff
        // against it and detect files changed by tools in this round.
        let checkpoint_id = if sha.is_empty() {
            let head = Self::git(working_dir, &["rev-parse", "HEAD"]).await?;
            if head.is_empty() {
                return Ok(None);
            }
            head
        } else {
            sha
        };

        Ok(Some(Checkpoint {
            id: checkpoint_id,
            index_tree_id,
            label: label.to_string(),
            created_at: now,
        }))
    }

    async fn rollback(&self, working_dir: &str, checkpoint_id: &str) -> Result<(), String> {
        Self::git(working_dir, &["checkout", "--", "."]).await?;

        Self::git(working_dir, &["clean", "-fd"]).await?;

        Self::git(working_dir, &["stash", "apply", checkpoint_id]).await?;

        Ok(())
    }

    async fn discard(&self, _working_dir: &str, _checkpoint_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn is_available(&self, working_dir: &str) -> bool {
        Self::git(working_dir, &["rev-parse", "--is-inside-work-tree"])
            .await
            .map(|out| out == "true")
            .unwrap_or(false)
    }

    fn name(&self) -> &'static str {
        "git"
    }

    async fn current_bindable_revision(&self, working_dir: &str) -> Option<VcsRevisionRef> {
        let hash = Self::git(working_dir, &["rev-parse", "HEAD"]).await.ok()?;
        if hash.is_empty() {
            return None;
        }
        let short = if hash.len() > 7 { &hash[..7] } else { &hash };
        Some(VcsRevisionRef {
            provider: "git".to_string(),
            revision_id: hash.clone(),
            revision_kind: "commit".to_string(),
            display: short.to_string(),
        })
    }

    async fn compare_paths(
        &self,
        working_dir: &str,
        from_revision: &str,
        to_revision: &str,
    ) -> Result<Vec<VcsChangedPath>, String> {
        let output = Self::git(
            working_dir,
            &["diff", "--name-status", from_revision, to_revision],
        )
        .await?;

        let mut paths = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() >= 2 {
                let kind_char = parts[0].chars().next().unwrap_or('M');
                let (change_kind, actual_path, old_path) = if kind_char == 'R' && parts.len() >= 3 {
                    ("R", parts[2].to_string(), Some(parts[1].to_string()))
                } else {
                    (
                        match kind_char {
                            'A' => "A",
                            'D' => "D",
                            _ => "M",
                        },
                        parts[1].to_string(),
                        None,
                    )
                };
                paths.push(VcsChangedPath {
                    path: actual_path,
                    change_kind: change_kind.to_string(),
                    old_path,
                });
            }
        }
        Ok(paths)
    }
}

impl GitProvider {
    /// Return the current branch name (empty string if detached HEAD).
    pub async fn current_branch(working_dir: &str) -> Result<String, String> {
        Self::git(working_dir, &["branch", "--show-current"]).await
    }

    /// Return the last `n` commits as one-line summaries.
    pub async fn recent_commits(working_dir: &str, n: usize) -> Result<String, String> {
        Self::git(
            working_dir,
            &["log", "--oneline", &format!("-{}", n), "--no-decorate"],
        )
        .await
    }

    /// Return a compact summary of uncommitted changes (staged + unstaged + untracked).
    pub async fn uncommitted_summary(working_dir: &str) -> Result<String, String> {
        Self::git(working_dir, &["status", "--short"]).await
    }

    pub async fn restore_files(
        working_dir: &str,
        checkpoint_id: &str,
        index_tree_id: Option<&str>,
        files: &[super::undo::ChangedFile],
    ) -> Result<(), String> {
        for f in files {
            let full_path = std::path::Path::new(working_dir).join(&f.path);
            match f.status.as_str() {
                "A" => {
                    let _ = tokio::fs::remove_file(&full_path).await;
                    let meta_path = format!("{}.meta", full_path.display());
                    let _ = tokio::fs::remove_file(&meta_path).await;
                }
                "R" => {
                    let _ = tokio::fs::remove_file(&full_path).await;
                    let meta_path = format!("{}.meta", full_path.display());
                    let _ = tokio::fs::remove_file(&meta_path).await;

                    let restore_path = f.old_path.as_deref().unwrap_or(&f.path);
                    Self::git(
                        working_dir,
                        &[
                            "restore",
                            &format!("--source={}", checkpoint_id),
                            "--worktree",
                            "--",
                            restore_path,
                        ],
                    )
                    .await?;
                }
                _ => {
                    Self::git(
                        working_dir,
                        &[
                            "restore",
                            &format!("--source={}", checkpoint_id),
                            "--worktree",
                            "--",
                            &f.path,
                        ],
                    )
                    .await?;
                }
            }
        }

        if let Some(index_tree_id) = index_tree_id {
            let mut restore_targets = std::collections::BTreeSet::new();
            for file in files {
                restore_targets.insert(file.path.clone());
                if let Some(old_path) = &file.old_path {
                    restore_targets.insert(old_path.clone());
                }
            }

            for path in restore_targets {
                if Self::tree_contains_path(working_dir, index_tree_id, &path).await? {
                    Self::git(
                        working_dir,
                        &[
                            "restore",
                            &format!("--source={}", index_tree_id),
                            "--staged",
                            "--",
                            &path,
                        ],
                    )
                    .await?;
                } else {
                    Self::git(
                        working_dir,
                        &["rm", "--cached", "--quiet", "--ignore-unmatch", "--", &path],
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    async fn tree_contains_path(
        working_dir: &str,
        tree_id: &str,
        path: &str,
    ) -> Result<bool, String> {
        let output = Self::git(
            working_dir,
            &["ls-tree", "-r", "--name-only", tree_id, "--", path],
        )
        .await?;
        Ok(output.lines().any(|line| line.trim() == path))
    }

    pub async fn diff_files(working_dir: &str, checkpoint_id: &str) -> Result<Vec<String>, String> {
        let after_sha = Self::snapshot_worktree(working_dir, "locus diff temp").await?;
        let after_ref = if after_sha.is_empty() {
            match Self::git(working_dir, &["rev-parse", "HEAD"]).await {
                Ok(head) if !head.is_empty() => head,
                Ok(_) | Err(_) => Self::empty_tree_id(working_dir).await?,
            }
        } else {
            after_sha
        };

        let output = Self::git(
            working_dir,
            &[
                "diff-tree",
                "-r",
                "--name-status",
                "--find-renames",
                "--no-commit-id",
                checkpoint_id,
                &after_ref,
            ],
        )
        .await?;

        Ok(output
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::GitProvider;
    use crate::eol::normalize_lf;
    use crate::process_util::command;
    use crate::vcs::undo::ChangedFile;
    use crate::vcs::VcsProvider;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;

    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    fn git_available() -> bool {
        crate::process_util::resolve_git().is_some()
    }

    fn temp_repo_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("locus-{}-{}", name, uuid::Uuid::new_v4()))
    }

    fn git(cwd: &Path, args: &[&str]) -> String {
        let output = command("git")
            .args(["-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false"])
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[test]
    fn git_failure_message_includes_status_when_stderr_is_empty() {
        let status = {
            #[cfg(unix)]
            {
                ExitStatus::from_raw(1 << 8)
            }

            #[cfg(windows)]
            {
                ExitStatus::from_raw(1)
            }
        };

        let message = GitProvider::format_git_failure(&["rev-parse", "HEAD"], status, b"");
        assert_eq!(
            message,
            format!("git rev-parse HEAD failed with status {}", status)
        );
    }

    fn write_file(path: &Path, content: &str) {
        std::fs::write(path, content).expect("write file");
    }

    fn setup_repo(name: &str) -> PathBuf {
        let repo = temp_repo_dir(name);
        std::fs::create_dir_all(&repo).expect("create temp repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.name", "test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        write_file(&repo.join(".gitattributes"), "* text=auto eol=lf\n");
        write_file(&repo.join("tracked.txt"), "base\n");
        git(&repo, &["add", ".gitattributes", "tracked.txt"]);
        git(&repo, &["commit", "-m", "init"]);
        repo
    }

    fn setup_repo_with_gitattributes(name: &str, attributes: &str) -> PathBuf {
        let repo = temp_repo_dir(name);
        std::fs::create_dir_all(&repo).expect("create temp repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.name", "test"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        write_file(&repo.join(".gitattributes"), attributes);
        write_file(&repo.join("tracked.txt"), "base\n");
        git(&repo, &["add", ".gitattributes", "tracked.txt"]);
        git(&repo, &["commit", "-m", "init"]);
        repo
    }

    #[tokio::test]
    async fn checkpoint_keeps_real_index_unchanged_and_captures_untracked() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-checkpoint");
        write_file(&repo.join("tracked.txt"), "base\nuser-staged\n");
        git(&repo, &["add", "tracked.txt"]);
        write_file(&repo.join("untracked.txt"), "user-untracked\n");

        let before_status = git(&repo, &["status", "--short"]);
        assert_eq!(before_status, "M  tracked.txt\n?? untracked.txt");

        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        let after_status = git(&repo, &["status", "--short"]);
        assert_eq!(after_status, before_status);

        let untracked_ref = format!("{}:untracked.txt", checkpoint.id);
        let tracked_ref = format!("{}:tracked.txt", checkpoint.id);
        assert_eq!(git(&repo, &["show", &tracked_ref]), "base\nuser-staged");
        assert_eq!(git(&repo, &["show", &untracked_ref]), "user-untracked");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn diff_and_restore_preserve_real_index_state() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-diff-restore");
        write_file(&repo.join("tracked.txt"), "base\nuser-staged\n");
        git(&repo, &["add", "tracked.txt"]);
        write_file(&repo.join("untracked.txt"), "user-untracked\n");

        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        write_file(&repo.join("tracked.txt"), "base\nuser-staged\nagent-more\n");
        write_file(&repo.join("untracked.txt"), "user-untracked\nagent-more\n");
        write_file(&repo.join("new.txt"), "new-file\n");

        let changed = GitProvider::diff_files(&repo.to_string_lossy(), &checkpoint.id)
            .await
            .expect("diff_files should succeed");
        let mut changed = changed;
        changed.sort();
        assert_eq!(
            changed,
            vec![
                "A\tnew.txt".to_string(),
                "M\ttracked.txt".to_string(),
                "M\tuntracked.txt".to_string()
            ]
        );

        let status_after_diff = git(&repo, &["status", "--short"]);
        assert_eq!(
            status_after_diff,
            "MM tracked.txt\n?? new.txt\n?? untracked.txt"
        );

        GitProvider::restore_files(
            &repo.to_string_lossy(),
            &checkpoint.id,
            checkpoint.index_tree_id.as_deref(),
            &[
                ChangedFile {
                    status: "M".to_string(),
                    path: "tracked.txt".to_string(),
                    old_path: None,
                },
                ChangedFile {
                    status: "M".to_string(),
                    path: "untracked.txt".to_string(),
                    old_path: None,
                },
                ChangedFile {
                    status: "A".to_string(),
                    path: "new.txt".to_string(),
                    old_path: None,
                },
            ],
        )
        .await
        .expect("restore should succeed");

        assert_eq!(
            normalize_lf(
                &std::fs::read_to_string(repo.join("tracked.txt")).expect("tracked after restore")
            ),
            "base\nuser-staged\n"
        );
        assert_eq!(
            normalize_lf(
                &std::fs::read_to_string(repo.join("untracked.txt"))
                    .expect("untracked after restore")
            ),
            "user-untracked\n"
        );
        assert!(!repo.join("new.txt").exists());
        assert_eq!(
            git(&repo, &["status", "--short"]),
            "M  tracked.txt\n?? untracked.txt"
        );

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn diff_files_reports_delete_when_round_ends_clean() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-diff-clean-delete");
        write_file(&repo.join("test1.cs"), "class T {}\n");

        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        std::fs::remove_file(repo.join("test1.cs")).expect("delete file");

        let changed = GitProvider::diff_files(&repo.to_string_lossy(), &checkpoint.id)
            .await
            .expect("diff_files should succeed");
        assert_eq!(changed, vec!["D\ttest1.cs".to_string()]);
        assert_eq!(git(&repo, &["status", "--short"]), "");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn diff_files_detects_renamed_file_with_small_edit() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-diff-rename");
        let old_dir = repo.join("GameLogic").join("MVVM");
        let new_dir = repo.join("UIModule").join("MVVM");
        std::fs::create_dir_all(&old_dir).expect("create old dir");
        let old_path = old_dir.join("AsyncCommand.cs");
        write_file(
            &old_path,
            "namespace GameLogic.MVVM\n{\n    public sealed class AsyncCommand : ICommand\n    {\n        public void Execute() {}\n    }\n}\n",
        );
        git(&repo, &["add", "GameLogic/MVVM/AsyncCommand.cs"]);
        git(&repo, &["commit", "-m", "add async command"]);

        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        std::fs::create_dir_all(&new_dir).expect("create new dir");
        let new_path = new_dir.join("AsyncCommand.cs");
        std::fs::rename(&old_path, &new_path).expect("rename file");
        write_file(
            &new_path,
            "namespace UIModule.MVVM\n{\n    public sealed class AsyncCommand : ICommand\n    {\n        public void Execute() {}\n    }\n}\n",
        );

        let changed = GitProvider::diff_files(&repo.to_string_lossy(), &checkpoint.id)
            .await
            .expect("diff_files should succeed");
        assert_eq!(changed.len(), 1);
        let parts: Vec<&str> = changed[0].split('\t').collect();
        assert_eq!(parts.len(), 3);
        assert!(
            parts[0].starts_with('R'),
            "expected rename, got {changed:?}"
        );
        assert_eq!(parts[1], "GameLogic/MVVM/AsyncCommand.cs");
        assert_eq!(parts[2], "UIModule/MVVM/AsyncCommand.cs");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn diff_files_preserves_utf8_paths_in_output() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-diff-utf8-path");

        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        let knowledge_dir = repo
            .join("Locus")
            .join("knowledge")
            .join("design")
            .join("system");
        std::fs::create_dir_all(&knowledge_dir).expect("create knowledge dir");
        write_file(&knowledge_dir.join("主要玩法.md"), "# 主要玩法\n");

        let changed = GitProvider::diff_files(&repo.to_string_lossy(), &checkpoint.id)
            .await
            .expect("diff_files should succeed");
        assert_eq!(
            changed,
            vec!["A\tLocus/knowledge/design/system/主要玩法.md".to_string()]
        );

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn restore_reverts_index_changes_made_after_checkpoint() {
        if !git_available() {
            return;
        }

        let repo = setup_repo("git-restore-index");
        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        write_file(&repo.join("tracked.txt"), "base\nagent-staged\n");
        git(&repo, &["add", "tracked.txt"]);
        write_file(&repo.join("new.txt"), "new-file\n");
        git(&repo, &["add", "new.txt"]);

        let mut changed = GitProvider::diff_files(&repo.to_string_lossy(), &checkpoint.id)
            .await
            .expect("diff_files should succeed");
        changed.sort();
        assert_eq!(
            changed,
            vec!["A\tnew.txt".to_string(), "M\ttracked.txt".to_string()]
        );

        GitProvider::restore_files(
            &repo.to_string_lossy(),
            &checkpoint.id,
            checkpoint.index_tree_id.as_deref(),
            &[
                ChangedFile {
                    status: "M".to_string(),
                    path: "tracked.txt".to_string(),
                    old_path: None,
                },
                ChangedFile {
                    status: "A".to_string(),
                    path: "new.txt".to_string(),
                    old_path: None,
                },
            ],
        )
        .await
        .expect("restore should succeed");

        assert_eq!(
            normalize_lf(
                &std::fs::read_to_string(repo.join("tracked.txt")).expect("tracked after restore")
            ),
            "base\n"
        );
        assert!(!repo.join("new.txt").exists());
        assert_eq!(git(&repo, &["status", "--short"]), "");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[tokio::test]
    async fn restore_files_respects_repo_eol_rule_for_worktree_output() {
        if !git_available() {
            return;
        }

        let repo = setup_repo_with_gitattributes("git-restore-eol", "* text=auto eol=lf\n");
        let provider = GitProvider;
        let checkpoint = provider
            .checkpoint(&repo.to_string_lossy(), "agent round")
            .await
            .expect("checkpoint should succeed")
            .expect("checkpoint should exist");

        write_file(&repo.join("tracked.txt"), "base\nagent-change\n");
        git(&repo, &["add", "tracked.txt"]);

        GitProvider::restore_files(
            &repo.to_string_lossy(),
            &checkpoint.id,
            checkpoint.index_tree_id.as_deref(),
            &[ChangedFile {
                status: "M".to_string(),
                path: "tracked.txt".to_string(),
                old_path: None,
            }],
        )
        .await
        .expect("restore should succeed");

        assert_eq!(
            std::fs::read(repo.join("tracked.txt")).expect("tracked bytes after restore"),
            b"base\n"
        );
        assert_eq!(git(&repo, &["status", "--short"]), "");

        std::fs::remove_dir_all(&repo).expect("cleanup temp repo");
    }

    #[test]
    fn detects_root_nul_snapshot_failures() {
        let error = "git add -A failed: error: short read while indexing nul\nerror: unable to index file 'nul'";
        assert!(GitProvider::should_auto_remove_root_nul(error));
        assert!(!GitProvider::should_auto_remove_root_nul(
            "git add -A failed: fatal: adding files failed"
        ));
    }

    #[test]
    fn workspace_snapshot_failure_log_includes_context() {
        let message = GitProvider::format_workspace_snapshot_failure(
            "C:\\repo",
            "agent round",
            "git add -A failed: fatal: adding files failed",
        );

        assert!(message.contains("workspace='C:\\repo'"));
        assert!(message.contains("label='agent round'"));
        assert!(message.contains("reason=git add -A failed: fatal: adding files failed"));
    }
}
