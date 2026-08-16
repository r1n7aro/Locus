use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const RUNTIME_ROOT_ENV: &str = "LOCUS_RUNTIME_ROOT";
pub(crate) const ISOLATED_RUNTIME_BASE_ENV: &str = "LOCUS_ISOLATED_RUNTIME_BASE";
pub(crate) const RUNTIME_DATA_DIR_ENV: &str = "LOCUS_RUNTIME_DATA_DIR";
pub(crate) const RUNTIME_CONFIG_DIR_ENV: &str = "LOCUS_RUNTIME_CONFIG_DIR";
pub(crate) const RUNTIME_LOG_DIR_ENV: &str = "LOCUS_RUNTIME_LOG_DIR";
pub(crate) const RUNTIME_WORKSPACE_DIR_ENV: &str = "LOCUS_RUNTIME_WORKSPACE_DIR";
pub(crate) const WEBVIEW_DATA_DIR_ENV: &str = "WEBVIEW2_USER_DATA_FOLDER";
pub(crate) const SKIP_ONBOARDING_ENV: &str = "LOCUS_SKIP_ONBOARDING";

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeLaunchOptions {
    pub(crate) runtime_root: Option<PathBuf>,
    pub(crate) data_dir: Option<PathBuf>,
    pub(crate) config_dir: Option<PathBuf>,
    pub(crate) log_dir: Option<PathBuf>,
    pub(crate) workspace_dir: Option<PathBuf>,
    pub(crate) webview_data_dir: Option<PathBuf>,
    pub(crate) skip_onboarding: bool,
}

#[derive(Debug, Default)]
struct ParsedRuntimeArgs {
    help: bool,
    isolated: bool,
    runtime_root: Option<PathBuf>,
    runtime_base: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    log_dir: Option<PathBuf>,
    workspace_dir: Option<PathBuf>,
    webview_data_dir: Option<PathBuf>,
    skip_onboarding: bool,
}

impl RuntimeLaunchOptions {
    pub(crate) fn configure_from_env_args() -> Result<Self, String> {
        let parsed = ParsedRuntimeArgs::parse(std::env::args_os().skip(1).collect())?;
        parsed.apply()?;
        let options = Self::from_environment()?;
        options.print_manifest();
        Ok(options)
    }

    fn from_environment() -> Result<Self, String> {
        Ok(Self {
            runtime_root: directory_from_env(RUNTIME_ROOT_ENV, "runtime root")?,
            data_dir: directory_from_env(RUNTIME_DATA_DIR_ENV, "database directory")?,
            config_dir: directory_from_env(RUNTIME_CONFIG_DIR_ENV, "config directory")?,
            log_dir: directory_from_env(RUNTIME_LOG_DIR_ENV, "log directory")?,
            workspace_dir: directory_from_env(RUNTIME_WORKSPACE_DIR_ENV, "workspace directory")?,
            webview_data_dir: directory_from_env(WEBVIEW_DATA_DIR_ENV, "WebView data directory")?,
            skip_onboarding: bool_from_env(SKIP_ONBOARDING_ENV),
        })
    }

    fn print_manifest(&self) {
        if self.runtime_root.is_none()
            && self.data_dir.is_none()
            && self.config_dir.is_none()
            && self.log_dir.is_none()
            && self.workspace_dir.is_none()
            && self.webview_data_dir.is_none()
        {
            return;
        }

        let value = serde_json::json!({
            "runtimeRoot": display_path(self.runtime_root.as_deref()),
            "databaseDir": display_path(self.data_dir.as_deref()),
            "databaseFile": self.data_dir.as_ref().map(|path| path.join("locus.db").display().to_string()),
            "configDir": display_path(self.config_dir.as_deref()),
            "logDir": display_path(self.log_dir.as_deref()),
            "logFile": self.log_dir.as_ref().map(|path| path.join("locus.log").display().to_string()),
            "workspace": display_path(self.workspace_dir.as_deref()),
            "webviewDataDir": display_path(self.webview_data_dir.as_deref()),
            "skipOnboarding": self.skip_onboarding,
        });
        println!("LOCUS_RUNTIME_JSON {value}");
    }
}

impl ParsedRuntimeArgs {
    fn parse(args: Vec<OsString>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut index = 0usize;
        while index < args.len() {
            let raw = args[index].to_string_lossy();
            let (name, inline) = raw.split_once('=').unwrap_or((&raw, ""));
            match name {
                "--locus-runtime-help" => parsed.help = true,
                "--locus-isolated" => parsed.isolated = true,
                "--locus-runtime-root" => {
                    parsed.runtime_root = Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-runtime-base" => {
                    parsed.runtime_base = Some(read_path_value(name, inline, &args, &mut index)?);
                    parsed.isolated = true;
                }
                "--locus-data-dir" | "--locus-database-dir" => {
                    parsed.data_dir = Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-config-dir" => {
                    parsed.config_dir = Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-log-dir" => {
                    parsed.log_dir = Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-workspace" => {
                    parsed.workspace_dir = Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-webview-data-dir" => {
                    parsed.webview_data_dir =
                        Some(read_path_value(name, inline, &args, &mut index)?)
                }
                "--locus-skip-onboarding" => parsed.skip_onboarding = true,
                _ => {}
            }
            index += 1;
        }
        Ok(parsed)
    }

    fn apply(mut self) -> Result<(), String> {
        if self.help {
            print_runtime_help();
            std::process::exit(0);
        }
        if self.skip_onboarding && !self.isolated && self.runtime_root.is_none() {
            return Err(
                "--locus-skip-onboarding requires --locus-isolated or --locus-runtime-root"
                    .to_string(),
            );
        }
        if self.isolated && self.runtime_root.is_none() {
            let runtime_base = match self.runtime_base.as_ref() {
                Some(base) => Some(ensure_directory(base, "isolated runtime base")?),
                None => directory_from_env(ISOLATED_RUNTIME_BASE_ENV, "isolated runtime base")?,
            };
            self.runtime_root = Some(create_unique_runtime_root(runtime_base.as_deref())?);
        }

        if let Some(root) = self.runtime_root.as_ref() {
            validate_absolute(root, "--locus-runtime-root")?;
            self.data_dir.get_or_insert_with(|| root.join("database"));
            self.config_dir.get_or_insert_with(|| root.join("config"));
            self.log_dir.get_or_insert_with(|| root.join("logs"));
            self.workspace_dir
                .get_or_insert_with(|| root.join("workspace"));
            self.webview_data_dir
                .get_or_insert_with(|| root.join("webview"));
        }

        apply_directory_override(RUNTIME_ROOT_ENV, self.runtime_root.as_deref())?;
        apply_directory_override(RUNTIME_DATA_DIR_ENV, self.data_dir.as_deref())?;
        apply_directory_override(RUNTIME_CONFIG_DIR_ENV, self.config_dir.as_deref())?;
        apply_directory_override(RUNTIME_LOG_DIR_ENV, self.log_dir.as_deref())?;
        apply_directory_override(RUNTIME_WORKSPACE_DIR_ENV, self.workspace_dir.as_deref())?;
        apply_directory_override(WEBVIEW_DATA_DIR_ENV, self.webview_data_dir.as_deref())?;
        if self.skip_onboarding {
            std::env::set_var(SKIP_ONBOARDING_ENV, "1");
        }

        if let Some(root) = self.runtime_root.as_ref() {
            let system_temp = ensure_directory(&root.join("system-temp"), "system temp directory")?;
            std::env::set_var("TEMP", &system_temp);
            std::env::set_var("TMP", &system_temp);
        }
        Ok(())
    }
}

fn print_runtime_help() {
    eprintln!(
        "Locus isolated runtime options:\n\
  --locus-isolated                 Create a complete generated runtime\n\
  --locus-runtime-root <dir>       Root for unspecified isolated directories\n\
  --locus-runtime-base <dir>       Parent for an automatically named isolated runtime\n\
  --locus-database-dir <dir>       Directory containing locus.db\n\
  --locus-config-dir <dir>         Persistent application configuration directory\n\
  --locus-log-dir <dir>            Directory containing locus.log\n\
  --locus-workspace <dir>          Initial workspace\n\
  --locus-webview-data-dir <dir>   WebView2 profile and local storage\n\
  --locus-skip-onboarding          Open directly in Chat for this runtime\n\
  --locus-runtime-help             Show this help"
    );
}

pub(crate) fn runtime_config_dir_from_env() -> Result<Option<PathBuf>, String> {
    directory_from_env(RUNTIME_CONFIG_DIR_ENV, "config directory")
}

pub(crate) fn runtime_log_dir_from_env() -> Result<Option<PathBuf>, String> {
    directory_from_env(RUNTIME_LOG_DIR_ENV, "log directory")
}

fn bool_from_env(key: &str) -> bool {
    std::env::var(key)
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "True"))
        .unwrap_or(false)
}

fn directory_from_env(key: &str, label: &str) -> Result<Option<PathBuf>, String> {
    let Some(value) = std::env::var_os(key) else {
        return Ok(None);
    };
    let value = value.to_string_lossy().trim().to_string();
    if value.is_empty() {
        return Err(format!("{key} cannot be empty"));
    }
    let path = PathBuf::from(value);
    validate_absolute(&path, key)?;
    ensure_directory(&path, label).map(Some)
}

fn apply_directory_override(key: &str, path: Option<&Path>) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    validate_absolute(path, key)?;
    let path = ensure_directory(path, key)?;
    std::env::set_var(key, path);
    Ok(())
}

fn ensure_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(path)
        .map_err(|error| format!("Failed to create {label} '{}': {error}", path.display()))?;
    Ok(dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

fn validate_absolute(path: &Path, option: &str) -> Result<(), String> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(format!(
            "{option} requires an absolute path: {}",
            path.display()
        ))
    }
}

fn read_path_value(
    name: &str,
    inline: &str,
    args: &[OsString],
    index: &mut usize,
) -> Result<PathBuf, String> {
    let value = if inline.is_empty() {
        let Some(next) = args.get(*index + 1) else {
            return Err(format!("{name} requires a directory"));
        };
        *index += 1;
        next.clone()
    } else {
        OsString::from(inline)
    };
    let path = PathBuf::from(value);
    validate_absolute(&path, name)?;
    Ok(path)
}

fn create_unique_runtime_root(runtime_base: Option<&Path>) -> Result<PathBuf, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let base = runtime_base
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    for suffix in 0..100u32 {
        let name = if suffix == 0 {
            format!("locus-runtime-{}-{timestamp}", std::process::id())
        } else {
            format!("locus-runtime-{}-{timestamp}-{suffix}", std::process::id())
        };
        let path = base.join(name);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(dunce::canonicalize(&path).unwrap_or(path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to create isolated runtime root '{}': {error}",
                    path.display()
                ))
            }
        }
    }
    Err("Failed to allocate a unique isolated runtime root".to_string())
}

fn display_path(path: Option<&Path>) -> Option<String> {
    path.map(|value| value.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::ParsedRuntimeArgs;
    use std::ffi::OsString;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_individual_runtime_directories() {
        let root = tempfile::tempdir().expect("tempdir");
        let data = root.path().join("db");
        let config = root.path().join("config");
        let log = root.path().join("log");
        let workspace = root.path().join("workspace");
        let parsed = ParsedRuntimeArgs::parse(args(&[
            "--locus-database-dir",
            data.to_str().unwrap(),
            &format!("--locus-config-dir={}", config.display()),
            "--locus-log-dir",
            log.to_str().unwrap(),
            "--locus-workspace",
            workspace.to_str().unwrap(),
        ]))
        .expect("parse");

        assert_eq!(parsed.data_dir.as_deref(), Some(data.as_path()));
        assert_eq!(parsed.config_dir.as_deref(), Some(config.as_path()));
        assert_eq!(parsed.log_dir.as_deref(), Some(log.as_path()));
        assert_eq!(parsed.workspace_dir.as_deref(), Some(workspace.as_path()));
    }

    #[test]
    fn rejects_relative_runtime_paths() {
        let error = ParsedRuntimeArgs::parse(args(&["--locus-data-dir", "relative/db"]))
            .expect_err("relative path should fail");
        assert!(error.contains("requires an absolute path"));
    }

    #[test]
    fn runtime_base_enables_isolation() {
        let root = tempfile::tempdir().expect("tempdir");
        let parsed = ParsedRuntimeArgs::parse(args(&[
            "--locus-runtime-base",
            root.path().to_str().unwrap(),
        ]))
        .expect("parse");

        assert!(parsed.isolated);
        assert_eq!(parsed.runtime_base.as_deref(), Some(root.path()));
    }

    #[test]
    fn parses_skip_onboarding_for_isolated_profiles() {
        let parsed =
            ParsedRuntimeArgs::parse(args(&["--locus-isolated", "--locus-skip-onboarding"]))
                .expect("parse");

        assert!(parsed.isolated);
        assert!(parsed.skip_onboarding);
    }
}
