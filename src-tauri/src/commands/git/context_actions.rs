//! Argument construction shared with isolated Git integration tests.

fn argument(value: &str) -> Result<&str, String> {
    if value.is_empty() || value.starts_with('-') || value.contains('\0') {
        return Err("Invalid Git argument".into());
    }
    Ok(value)
}

pub(super) fn commit_args(
    rev: &str,
    action: &str,
    mode: Option<&str>,
    name: Option<&str>,
) -> Result<Vec<String>, String> {
    argument(rev)?;
    let args = match action {
        "checkoutDetached" => vec!["checkout", "--detach", rev],
        "reset" => vec![
            "reset",
            match mode.unwrap_or("mixed") {
                "soft" => "--soft",
                "mixed" => "--mixed",
                "hard" => "--hard",
                _ => return Err("Invalid reset mode".into()),
            },
            rev,
        ],
        "cherryPick" | "revert" => {
            let mut args = if action == "cherryPick" {
                vec!["cherry-pick"]
            } else {
                vec!["revert", "--no-edit"]
            };
            if let Some(parent) = mode {
                if parent
                    .parse::<u32>()
                    .ok()
                    .filter(|number| *number > 0)
                    .is_none()
                {
                    return Err("Invalid mainline parent".into());
                }
                args.extend(["--mainline", parent]);
            }
            args.push(rev);
            args
        }
        "createBranch" | "createBranchAndCheckout" | "createTag" => {
            let name = argument(name.ok_or("Name is required")?)?;
            match action {
                "createBranch" => vec!["branch", "--", name, rev],
                "createTag" => vec!["tag", "--", name, rev],
                _ => vec!["checkout", "-b", name, rev],
            }
        }
        _ => return Err(format!("Unknown commit action: {action}")),
    };
    Ok(args.into_iter().map(str::to_owned).collect())
}

pub(super) fn branch_args(
    target: &str,
    kind: &str,
    action: &str,
    name: Option<&str>,
    remote: Option<&str>,
) -> Result<Vec<String>, String> {
    argument(target)?;
    if !matches!(kind, "local" | "remote") {
        return Err("Invalid branch kind".into());
    }
    let full_ref = format!(
        "refs/{}/{}",
        if kind == "local" { "heads" } else { "remotes" },
        target
    );
    let args = match action {
        "switch" if kind == "local" => vec!["switch", target],
        "checkoutTracking" if kind == "remote" => vec!["checkout", "--track", &full_ref],
        "mergeIntoCurrent" => vec!["merge", "--no-edit", &full_ref],
        "rebaseCurrentOnto" => vec!["rebase", &full_ref],
        "rename" if kind == "local" => vec![
            "branch",
            "-m",
            "--",
            target,
            argument(name.ok_or("New branch name is required")?)?,
        ],
        "delete" if kind == "local" => vec!["branch", "-d", "--", target],
        "pull" if kind == "local" => vec!["pull", "--ff-only"],
        "fetch" | "deleteRemote" if kind == "remote" => {
            let remote = argument(remote.ok_or("Remote name is required")?)?;
            let branch = target
                .strip_prefix(&format!("{remote}/"))
                .filter(|name| !name.is_empty() && *name != "HEAD")
                .ok_or("Remote branch does not match the selected remote")?;
            argument(branch)?;
            if action == "fetch" {
                vec!["fetch", "--prune", remote]
            } else {
                // Fully qualify the destination; never delete a same-name tag.
                return Ok(vec![
                    "push".into(),
                    "--delete".into(),
                    remote.into(),
                    format!("refs/heads/{branch}"),
                ]);
            }
        }
        _ => return Err(format!("Unsupported {kind} branch action: {action}")),
    };
    Ok(args.into_iter().map(str::to_owned).collect())
}

pub(super) fn stash_args(reference: &str, action: &str) -> Result<Vec<String>, String> {
    let index = reference
        .strip_prefix("stash@{")
        .and_then(|s| s.strip_suffix('}'));
    if index.and_then(|s| s.parse::<usize>().ok()).is_none() {
        return Err("Invalid stash reference".into());
    }
    let args = match action {
        "applyIndex" | "branch" => vec!["stash", "apply", "--index", reference],
        "apply" | "pop" | "drop" => vec!["stash", action, reference],
        _ => return Err(format!("Unknown stash action: {action}")),
    };
    Ok(args.into_iter().map(str::to_owned).collect())
}

pub(super) fn push_args(
    target: &str,
    remote: &str,
    destination: &str,
) -> Result<Vec<String>, String> {
    argument(target)?;
    argument(remote)?;
    if !destination.starts_with("refs/heads/") || destination.contains('\n') {
        return Err("Invalid upstream branch".into());
    }
    Ok(vec![
        "push".into(),
        remote.into(),
        format!("refs/heads/{target}:{destination}"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn git(root: &Path, args: &[&str]) -> String {
        let result = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8_lossy(&result.stdout).trim().to_owned()
    }
    fn run(root: &Path, args: Vec<String>) {
        git(root, &args.iter().map(String::as_str).collect::<Vec<_>>());
    }
    fn repo() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "locus-git-menu-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Menu test"]);
        git(&root, &["config", "user.email", "menu@example.test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "tag.gpgsign", "false"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["commit", "--allow-empty", "-m", "initial"]);
        root
    }
    #[test]
    fn validates_options_and_remote_identity() {
        assert!(commit_args("--all", "revert", None, None).is_err());
        assert!(commit_args("HEAD", "reset", Some("oops"), None).is_err());
        assert!(commit_args("HEAD", "createTag", None, Some("--force")).is_err());
        assert!(commit_args("HEAD", "revert", Some("0"), None).is_err());
        assert!(branch_args(
            "origin/feature",
            "remote",
            "deleteRemote",
            None,
            Some("upstream")
        )
        .is_err());
        assert!(branch_args("main", "local", "deleteRemote", None, Some("origin")).is_err());
        assert_eq!(
            branch_args(
                "team/origin/feature/nested",
                "remote",
                "deleteRemote",
                None,
                Some("team/origin")
            )
            .unwrap(),
            [
                "push",
                "--delete",
                "team/origin",
                "refs/heads/feature/nested"
            ]
        );
    }
    #[test]
    fn creates_refs_without_moving_head_and_switches_only_when_requested() {
        let root = repo();
        run(
            &root,
            commit_args("HEAD", "createBranch", None, Some("feature/白盒")).unwrap(),
        );
        run(
            &root,
            commit_args("HEAD", "createTag", None, Some("v-test")).unwrap(),
        );
        assert_eq!(git(&root, &["symbolic-ref", "--short", "HEAD"]), "main");
        assert_eq!(
            git(&root, &["rev-parse", "refs/tags/v-test"]),
            git(&root, &["rev-parse", "refs/heads/feature/白盒"])
        );
        run(
            &root,
            commit_args("HEAD", "createBranchAndCheckout", None, Some("feature/new")).unwrap(),
        );
        assert_eq!(
            git(&root, &["symbolic-ref", "--short", "HEAD"]),
            "feature/new"
        );
        run(
            &root,
            branch_args(
                "feature/new",
                "local",
                "rename",
                Some("feature/renamed"),
                None,
            )
            .unwrap(),
        );
        assert_eq!(
            git(&root, &["symbolic-ref", "--short", "HEAD"]),
            "feature/renamed"
        );
        run(
            &root,
            branch_args("feature/白盒", "local", "delete", None, None).unwrap(),
        );
    }
    #[test]
    fn cherry_picks_and_reverts_merge_commits_with_mainline() {
        let root = repo();
        git(&root, &["checkout", "-b", "feature"]);
        std::fs::write(root.join("feature.txt"), "feature\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "feature"]);
        git(&root, &["checkout", "main"]);
        git(&root, &["merge", "--no-ff", "feature", "-m", "merge"]);
        let merge = git(&root, &["rev-parse", "HEAD"]);
        run(
            &root,
            commit_args(&merge, "revert", Some("1"), None).unwrap(),
        );
        assert!(!root.join("feature.txt").exists());
        run(
            &root,
            commit_args(&merge, "cherryPick", Some("1"), None).unwrap(),
        );
        assert_eq!(
            std::fs::read_to_string(root.join("feature.txt")).unwrap(),
            "feature\n"
        );
    }
    #[test]
    fn deletes_only_the_selected_remote_branch() {
        let root = repo();
        let remote = root.join("remote.git");
        git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        git(
            &root,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&root, &["branch", "feature/nested"]);
        git(&root, &["tag", "feature/nested"]);
        git(
            &root,
            &[
                "push",
                "origin",
                "refs/heads/feature/nested",
                "refs/tags/feature/nested",
            ],
        );
        run(
            &root,
            branch_args(
                "origin/feature/nested",
                "remote",
                "deleteRemote",
                None,
                Some("origin"),
            )
            .unwrap(),
        );
        let refs = git(&root, &["ls-remote", "origin"]);
        assert!(!refs.contains("refs/heads/feature/nested"));
        assert!(refs.contains("refs/tags/feature/nested"));
        assert!(!git(&root, &["rev-parse", "refs/heads/feature/nested"]).is_empty());
    }

    #[test]
    fn pushes_only_the_selected_branch_to_its_explicit_upstream() {
        let root = repo();
        let remote = root.join("remote.git");
        git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        git(
            &root,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&root, &["branch", "unrelated"]);
        git(
            &root,
            &[
                "config",
                "remote.origin.push",
                "refs/heads/unrelated:refs/heads/unrelated",
            ],
        );
        run(
            &root,
            push_args("main", "origin", "refs/heads/published").unwrap(),
        );
        let refs = git(&root, &["ls-remote", "origin"]);
        assert!(refs.contains("refs/heads/published"));
        assert!(!refs.contains("refs/heads/unrelated"));
        assert!(!refs.contains("refs/heads/main"));
    }

    #[test]
    fn stash_restore_preserves_index_and_multi_drop_keeps_unselected_stashes() {
        let root = repo();
        std::fs::write(root.join("staged.txt"), "staged\n").unwrap();
        std::fs::write(root.join("loose.txt"), "untracked\n").unwrap();
        git(&root, &["add", "staged.txt"]);
        git(&root, &["stash", "push", "-u", "-m", "oldest"]);
        run(&root, stash_args("stash@{0}", "applyIndex").unwrap());
        assert_eq!(
            git(&root, &["diff", "--cached", "--name-only"]),
            "staged.txt"
        );
        assert!(root.join("loose.txt").exists());
        assert_eq!(
            git(&root, &["stash", "list", "--format=%s"])
                .lines()
                .count(),
            1
        );
        git(&root, &["stash", "push", "-u", "-m", "keep"]);
        let kept = git(&root, &["rev-parse", "stash@{0}"]);
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        git(&root, &["stash", "push", "-u", "-m", "newest"]);
        for reference in ["stash@{2}", "stash@{0}"] {
            run(&root, stash_args(reference, "drop").unwrap());
        }
        assert_eq!(git(&root, &["rev-parse", "stash@{0}"]), kept);
        assert!(stash_args("--all", "drop").is_err());
    }

    #[test]
    fn stash_branch_restores_original_base_and_drops_only_after_success() {
        let root = repo();
        let base = git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("saved.txt"), "saved\n").unwrap();
        git(&root, &["add", "saved.txt"]);
        git(&root, &["stash", "push", "-m", "saved"]);
        git(&root, &["commit", "--allow-empty", "-m", "advance"]);
        run(
            &root,
            commit_args(
                "stash@{0}^1",
                "createBranchAndCheckout",
                None,
                Some("from-stash"),
            )
            .unwrap(),
        );
        run(&root, stash_args("stash@{0}", "branch").unwrap());
        run(&root, stash_args("stash@{0}", "drop").unwrap());
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), base);
        assert_eq!(
            git(&root, &["symbolic-ref", "--short", "HEAD"]),
            "from-stash"
        );
        assert_eq!(
            git(&root, &["diff", "--cached", "--name-only"]),
            "saved.txt"
        );
        assert!(git(&root, &["stash", "list"]).is_empty());
    }
}
