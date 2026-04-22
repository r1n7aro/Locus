use std::path::Path;

use crate::process_util::command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

#[derive(Debug, Default)]
struct GitAttrState {
    eol: Option<LineEnding>,
    text_unset: bool,
}

pub fn normalize_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn detect_preferred_line_ending(text: &str) -> LineEnding {
    if text.contains("\r\n") {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

pub fn apply_line_ending(text: &str, line_ending: LineEnding) -> String {
    let normalized = normalize_lf(text);
    match line_ending {
        LineEnding::Lf => normalized,
        LineEnding::Crlf => normalized.replace('\n', "\r\n"),
    }
}

pub fn resolve_preferred_line_ending(
    git_cwd: Option<&Path>,
    path: &Path,
    current_text: Option<&str>,
) -> LineEnding {
    resolve_repo_line_ending(git_cwd, path).unwrap_or_else(|| {
        current_text
            .map(detect_preferred_line_ending)
            .unwrap_or(LineEnding::Lf)
    })
}

fn resolve_repo_line_ending(git_cwd: Option<&Path>, path: &Path) -> Option<LineEnding> {
    let (current_dir, attr_path) = git_attr_context(git_cwd, path)?;
    let output = command("git")
        .args([
            "-c",
            "core.quotePath=false",
            "check-attr",
            "eol",
            "text",
            "--",
            &attr_path,
        ])
        .current_dir(current_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut attrs = GitAttrState::default();
    for line in stdout.lines() {
        let mut parts = line.splitn(3, ": ");
        let _path = parts.next()?;
        let attr = parts.next()?;
        let info = parts.next()?;
        match attr {
            "eol" => match info {
                "lf" => attrs.eol = Some(LineEnding::Lf),
                "crlf" => attrs.eol = Some(LineEnding::Crlf),
                _ => {}
            },
            "text" => {
                if info == "unset" {
                    attrs.text_unset = true;
                }
            }
            _ => {}
        }
    }

    if attrs.text_unset {
        None
    } else {
        attrs.eol
    }
}

fn git_attr_context<'a>(git_cwd: Option<&'a Path>, path: &'a Path) -> Option<(&'a Path, String)> {
    if let Some(cwd) = git_cwd {
        return Some((cwd, path.to_string_lossy().replace('\\', "/")));
    }

    if path.is_absolute() {
        let cwd = path.parent().unwrap_or(path);
        return Some((cwd, path.to_string_lossy().to_string()));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{
        apply_line_ending, detect_preferred_line_ending, normalize_lf,
        resolve_preferred_line_ending, LineEnding,
    };
    use crate::process_util::command;
    use std::path::Path;

    fn init_repo(name: &str) -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect(name);
        let output = command("git")
            .args(["init", "-b", "main"])
            .current_dir(repo.path())
            .output()
            .expect("git init");
        assert!(output.status.success());
        let output = command("git")
            .args(["config", "user.name", "test"])
            .current_dir(repo.path())
            .output()
            .expect("git config user.name");
        assert!(output.status.success());
        let output = command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(repo.path())
            .output()
            .expect("git config user.email");
        assert!(output.status.success());
        repo
    }

    #[test]
    fn normalize_and_apply_line_endings_round_trip_text() {
        assert_eq!(
            normalize_lf("alpha\r\nbeta\rgamma\n"),
            "alpha\nbeta\ngamma\n"
        );
        assert_eq!(
            apply_line_ending("alpha\r\nbeta\n", LineEnding::Crlf),
            "alpha\r\nbeta\r\n"
        );
        assert_eq!(detect_preferred_line_ending("alpha\r\n"), LineEnding::Crlf);
        assert_eq!(detect_preferred_line_ending("alpha\n"), LineEnding::Lf);
    }

    #[test]
    fn repo_rule_overrides_current_file_style() {
        let repo = init_repo("eol repo");
        std::fs::write(
            repo.path().join(".gitattributes"),
            "* text=auto eol=lf\n*.cmd text eol=crlf\n",
        )
        .expect("write gitattributes");

        assert_eq!(
            resolve_preferred_line_ending(
                Some(repo.path()),
                Path::new("notes.txt"),
                Some("alpha\r\nbeta\r\n"),
            ),
            LineEnding::Lf
        );
        assert_eq!(
            resolve_preferred_line_ending(
                Some(repo.path()),
                Path::new("script.cmd"),
                Some("echo off\n"),
            ),
            LineEnding::Crlf
        );
    }

    #[test]
    fn falls_back_to_current_style_when_repo_rule_is_unspecified() {
        let repo = init_repo("eol fallback");
        assert_eq!(
            resolve_preferred_line_ending(
                Some(repo.path()),
                Path::new("notes.txt"),
                Some("alpha\r\nbeta\r\n"),
            ),
            LineEnding::Crlf
        );
    }
}
