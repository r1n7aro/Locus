//! Locates the local TeX toolchain: engines, dvisvgm and the distribution
//! (MiKTeX needs `--disable-installer` to stay offline). Detection reads the
//! process `PATH` plus the default install folders, because a Locus started
//! from the shell or the Start menu may not see a freshly installed TeX on
//! `PATH`. The result is cached until a refresh is requested.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use super::pipeline::Engine;

const VERSION_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    pub xelatex: Option<String>,
    pub pdflatex: Option<String>,
    pub lualatex: Option<String>,
    pub dvisvgm: Option<String>,
    /// `miktex`, `texlive` or `unknown`; absent without any engine.
    pub distribution: Option<String>,
    /// First line of the engine's `--version` output.
    pub version: Option<String>,
}

impl Toolchain {
    pub fn program(&self, engine: Engine) -> Option<&str> {
        match engine {
            Engine::Xelatex => self.xelatex.as_deref(),
            Engine::Pdflatex => self.pdflatex.as_deref(),
            Engine::Lualatex => self.lualatex.as_deref(),
        }
    }

    pub fn is_miktex(&self) -> bool {
        self.distribution.as_deref() == Some("miktex")
    }

    pub fn has_engine(&self) -> bool {
        self.xelatex.is_some() || self.pdflatex.is_some() || self.lualatex.is_some()
    }
}

fn cache() -> &'static Mutex<Option<Toolchain>> {
    static CACHE: std::sync::OnceLock<Mutex<Option<Toolchain>>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Cached toolchain; detects on first use or when `refresh` is set.
pub async fn get(refresh: bool) -> Toolchain {
    if !refresh {
        if let Some(cached) = cache().lock().ok().and_then(|guard| guard.clone()) {
            return cached;
        }
    }
    let detected = detect().await;
    if let Ok(mut guard) = cache().lock() {
        *guard = Some(detected.clone());
    }
    detected
}

async fn detect() -> Toolchain {
    let dirs = tokio::task::spawn_blocking(search_dirs).await.unwrap_or_default();
    let find = |name: &str| find_program(&dirs, name).map(|path| path.to_string_lossy().to_string());
    let mut toolchain = Toolchain {
        xelatex: find("xelatex"),
        pdflatex: find("pdflatex"),
        lualatex: find("lualatex"),
        dvisvgm: find("dvisvgm"),
        distribution: None,
        version: None,
    };
    let probe = toolchain
        .xelatex
        .clone()
        .or_else(|| toolchain.pdflatex.clone())
        .or_else(|| toolchain.lualatex.clone());
    if let Some(program) = probe {
        let version = version_line(&program).await;
        toolchain.distribution = Some(
            match version.as_deref() {
                Some(line) if line.contains("MiKTeX") => "miktex",
                Some(line) if line.contains("TeX Live") => "texlive",
                _ if program.to_ascii_lowercase().contains("miktex") => "miktex",
                _ if program.to_ascii_lowercase().contains("texlive") => "texlive",
                _ => "unknown",
            }
            .to_string(),
        );
        toolchain.version = version;
    }
    toolchain
}

async fn version_line(program: &str) -> Option<String> {
    let mut command = crate::process_util::async_command(program);
    command
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend(default_install_dirs());
    let mut unique = Vec::new();
    for dir in dirs {
        if !dir.as_os_str().is_empty() && !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

/// Default binary folders of MiKTeX and TeX Live, newest TeX Live first.
fn default_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs/MiKTeX/miktex/bin/x64"));
        }
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(var) {
                dirs.push(PathBuf::from(&base).join("MiKTeX/miktex/bin/x64"));
                dirs.push(PathBuf::from(&base).join("MiKTeX/miktex/bin"));
            }
        }
        for root in ["C:/texlive", "D:/texlive"] {
            for year in texlive_years(Path::new(root)) {
                for arch in ["windows", "win64", "win32"] {
                    dirs.push(year.join("bin").join(arch));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/TeX/texbin"));
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for year in texlive_years(Path::new("/usr/local/texlive")) {
            dirs.push(year.join("bin/x86_64-linux"));
            dirs.push(year.join("bin/aarch64-linux"));
        }
        dirs.push(PathBuf::from("/usr/bin"));
    }
    dirs
}

fn texlive_years(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut years = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.len() == 4 && name.chars().all(|ch| ch.is_ascii_digit()))
        })
        .collect::<Vec<_>>();
    years.sort();
    years.reverse();
    years
}

fn find_program(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) { &[".exe", ".bat", ".cmd"] } else { &[""] };
    for dir in dirs {
        for suffix in candidates {
            let path = dir.join(format!("{name}{suffix}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_in_the_first_matching_dir() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("a");
        let second = temp.path().join("b");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let file = if cfg!(windows) { "xelatex.exe" } else { "xelatex" };
        std::fs::write(second.join(file), "").unwrap();
        let found = find_program(&[first, second.clone()], "xelatex").unwrap();
        assert_eq!(found, second.join(file));
        assert!(find_program(&[second], "dvisvgm").is_none());
    }

    #[test]
    fn orders_texlive_years_newest_first() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["2023", "2025", "texmf-local", "2024"] {
            std::fs::create_dir_all(temp.path().join(name)).unwrap();
        }
        let years = texlive_years(temp.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(years, vec!["2025", "2024", "2023"]);
    }
}
