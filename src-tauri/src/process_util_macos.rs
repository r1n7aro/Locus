//! Finder/LaunchServices do not inherit the interactive shell's Homebrew PATH.
//! Candidates are still checked with `--version` by the common resolver.
use std::path::PathBuf;

pub(super) fn git_candidates() -> Vec<PathBuf> {
    [
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/opt/local/bin/git",
        "/usr/bin/git",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}

pub(super) fn github_cli_candidates() -> Vec<PathBuf> {
    [
        "/opt/homebrew/bin/gh",
        "/usr/local/bin/gh",
        "/opt/local/bin/gh",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect()
}
