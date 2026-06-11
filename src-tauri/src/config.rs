use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CONFIG_FILE_NAME: &str = "config.json";

mod serde_atomic_bool {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Arc<AtomicBool>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(v.load(Ordering::Relaxed))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<AtomicBool>, D::Error> {
        let b = bool::deserialize(d)?;
        Ok(Arc::new(AtomicBool::new(b)))
    }
}

fn default_debug_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn default_view_windows_above_main() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn default_view_open_in_existing_window() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

fn default_unity_background_hook_enabled() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

fn default_unity_sidecar_compiler() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AppCloseBehavior {
    Exit,
    MinimizeToTray,
}

impl Default for AppCloseBehavior {
    fn default() -> Self {
        Self::Exit
    }
}

mod serde_close_behavior {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        v: &Arc<Mutex<AppCloseBehavior>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let value = *v.lock().map_err(serde::ser::Error::custom)?;
        value.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Arc<Mutex<AppCloseBehavior>>, D::Error> {
        let value = AppCloseBehavior::deserialize(d)?;
        Ok(Arc::new(Mutex::new(value)))
    }
}

fn default_close_behavior() -> Arc<Mutex<AppCloseBehavior>> {
    Arc::new(Mutex::new(AppCloseBehavior::Exit))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DynamicToolLoadingMode {
    #[serde(alias = "meta-tool", alias = "meta_tool")]
    MetaTool,
    Direct,
}

impl Default for DynamicToolLoadingMode {
    fn default() -> Self {
        Self::MetaTool
    }
}

mod serde_dynamic_tool_loading_mode {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        v: &Arc<Mutex<DynamicToolLoadingMode>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let value = *v.lock().map_err(serde::ser::Error::custom)?;
        value.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Arc<Mutex<DynamicToolLoadingMode>>, D::Error> {
        let value = DynamicToolLoadingMode::deserialize(d)?;
        Ok(Arc::new(Mutex::new(value)))
    }
}

fn default_dynamic_tool_loading_mode() -> Arc<Mutex<DynamicToolLoadingMode>> {
    Arc::new(Mutex::new(DynamicToolLoadingMode::MetaTool))
}

/// Per-tool switches for the code-analysis tool family. Each flag controls
/// whether the tool is offered to agents at all (disabled tools are filtered
/// out of the request tool list, see
/// `AgentInstance::resolve_effective_tool_names`). `unity_analyzers` is not a
/// tool: it injects Microsoft.Unity.Analyzers into the Roslyn language server
/// so `code_diagnostics` reports Unity-specific rules (UNT*).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct CodeAnalysisToolsConfig {
    pub code_symbol_search: bool,
    pub code_goto_definition: bool,
    pub code_find_references: bool,
    pub code_diagnostics: bool,
    pub code_hover: bool,
    pub unity_code_usages: bool,
    pub unity_analyzers: bool,
}

impl Default for CodeAnalysisToolsConfig {
    fn default() -> Self {
        Self {
            code_symbol_search: true,
            code_goto_definition: true,
            code_find_references: true,
            code_diagnostics: true,
            code_hover: false,
            unity_code_usages: true,
            unity_analyzers: true,
        }
    }
}

mod serde_code_analysis_tools {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        v: &Arc<Mutex<CodeAnalysisToolsConfig>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let value = *v.lock().map_err(serde::ser::Error::custom)?;
        value.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Arc<Mutex<CodeAnalysisToolsConfig>>, D::Error> {
        let value = CodeAnalysisToolsConfig::deserialize(d)?;
        Ok(Arc::new(Mutex::new(value)))
    }
}

fn default_code_analysis_tools() -> Arc<Mutex<CodeAnalysisToolsConfig>> {
    Arc::new(Mutex::new(CodeAnalysisToolsConfig::default()))
}

mod serde_string_mutex {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Arc<Mutex<String>>, s: S) -> Result<S::Ok, S::Error> {
        let value = v.lock().map_err(serde::ser::Error::custom)?;
        value.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Mutex<String>>, D::Error> {
        let value = String::deserialize(d)?;
        Ok(Arc::new(Mutex::new(value)))
    }
}

fn default_skill_package_namespace() -> Arc<Mutex<String>> {
    Arc::new(Mutex::new(String::new()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_debug_flag", with = "serde_atomic_bool")]
    pub debug: Arc<AtomicBool>,
    #[serde(default = "default_debug_flag", with = "serde_atomic_bool")]
    pub file_tool_workspace_boundary: Arc<AtomicBool>,
    #[serde(default = "default_close_behavior", with = "serde_close_behavior")]
    pub close_behavior: Arc<Mutex<AppCloseBehavior>>,
    #[serde(
        default = "default_dynamic_tool_loading_mode",
        with = "serde_dynamic_tool_loading_mode"
    )]
    pub dynamic_tool_loading_mode: Arc<Mutex<DynamicToolLoadingMode>>,
    #[serde(
        default = "default_skill_package_namespace",
        with = "serde_string_mutex"
    )]
    pub default_skill_package_namespace: Arc<Mutex<String>>,
    #[serde(
        default = "default_view_windows_above_main",
        with = "serde_atomic_bool"
    )]
    pub view_windows_above_main: Arc<AtomicBool>,
    #[serde(
        default = "default_view_open_in_existing_window",
        with = "serde_atomic_bool"
    )]
    pub view_open_in_existing_window: Arc<AtomicBool>,
    #[serde(
        default = "default_unity_background_hook_enabled",
        with = "serde_atomic_bool"
    )]
    pub unity_background_hook_enabled: Arc<AtomicBool>,
    #[serde(default = "default_debug_flag", with = "serde_atomic_bool")]
    pub csharp_lsp_enabled: Arc<AtomicBool>,
    /// Compile unity_execute / unity_run_states / View Script snippets in
    /// the CoreCLR compile-server sidecar instead of inside the Unity Editor
    /// process. Default on (phase 6 rollout); any sidecar failure falls back
    /// to the in-Unity path at runtime, and the fallback path stays for at
    /// least one release cycle.
    #[serde(default = "default_unity_sidecar_compiler", with = "serde_atomic_bool")]
    pub unity_sidecar_compiler: Arc<AtomicBool>,
    #[serde(
        default = "default_code_analysis_tools",
        with = "serde_code_analysis_tools"
    )]
    pub code_analysis_tools: Arc<Mutex<CodeAnalysisToolsConfig>>,
    #[serde(skip)]
    config_path: Arc<Mutex<Option<PathBuf>>>,
}

impl AppConfig {
    pub fn load(data_dir: &Path) -> Self {
        let primary_path = stable_config_path(data_dir);
        Self::load_from_path(&primary_path)
    }

    fn load_from_path(primary_path: &Path) -> Self {
        if let Some(mut config) = Self::try_load_file(primary_path) {
            println!(
                "[Locus] config loaded from persistent path: {:?}",
                dunce::canonicalize(primary_path).unwrap_or(primary_path.to_path_buf())
            );
            config.set_config_path(primary_path.to_path_buf());
            return config;
        }

        println!("[Locus] config not found in any path, creating defaults");

        let model = std::env::var("LOCUS_MODEL")
            .unwrap_or_else(|_| "openrouter/claude-sonnet-4.6".to_string());

        let base_url = std::env::var("LOCUS_BASE_URL").ok();

        let debug = std::env::var("LOCUS_DEBUG")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);

        let config = AppConfig {
            model,
            base_url,
            debug: Arc::new(AtomicBool::new(debug)),
            file_tool_workspace_boundary: default_debug_flag(),
            close_behavior: default_close_behavior(),
            dynamic_tool_loading_mode: default_dynamic_tool_loading_mode(),
            default_skill_package_namespace: default_skill_package_namespace(),
            view_windows_above_main: default_view_windows_above_main(),
            view_open_in_existing_window: default_view_open_in_existing_window(),
            unity_background_hook_enabled: default_unity_background_hook_enabled(),
            csharp_lsp_enabled: default_debug_flag(),
            unity_sidecar_compiler: default_unity_sidecar_compiler(),
            code_analysis_tools: default_code_analysis_tools(),
            config_path: Arc::new(Mutex::new(Some(primary_path.to_path_buf()))),
        };

        if let Err(err) = Self::persist_to_path(&config, primary_path) {
            eprintln!(
                "[Locus] failed to write default config to '{}': {}",
                primary_path.display(),
                err
            );
        } else {
            println!(
                "[Locus] default config written to {:?}",
                dunce::canonicalize(primary_path).unwrap_or(primary_path.to_path_buf())
            );
        }

        config
    }

    fn try_load_file(path: &Path) -> Option<Self> {
        let content = fs::read_to_string(path).ok()?;
        let (config, scrubbed_legacy_secret) = Self::parse_content(&content).ok()?;
        if scrubbed_legacy_secret {
            if let Err(err) = Self::persist_to_path(&config, path) {
                eprintln!(
                    "[Locus] failed to scrub legacy api_key from '{}': {}",
                    path.display(),
                    err
                );
            } else {
                println!(
                    "[Locus] scrubbed legacy api_key from config: {:?}",
                    dunce::canonicalize(path).unwrap_or(path.to_path_buf())
                );
            }
        }
        Some(config)
    }

    fn parse_content(content: &str) -> Result<(Self, bool), String> {
        let mut value: Value =
            serde_json::from_str(content).map_err(|e| format!("failed to parse config: {}", e))?;
        let scrubbed_legacy_secret = Self::remove_legacy_api_key(&mut value);
        let config = serde_json::from_value::<AppConfig>(value)
            .map_err(|e| format!("failed to deserialize config: {}", e))?;
        Ok((config, scrubbed_legacy_secret))
    }

    fn remove_legacy_api_key(value: &mut Value) -> bool {
        let Some(obj) = value.as_object_mut() else {
            return false;
        };
        let removed_snake = obj.remove("api_key").is_some();
        let removed_camel = obj.remove("apiKey").is_some();
        removed_snake || removed_camel
    }

    fn set_config_path(&mut self, path: PathBuf) {
        if let Ok(mut guard) = self.config_path.lock() {
            *guard = Some(path);
        }
    }

    pub fn debug_enabled(&self) -> bool {
        self.debug.load(Ordering::Relaxed)
    }

    pub fn set_debug_enabled(&self, value: bool) -> Result<(), String> {
        self.debug.store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn file_tool_workspace_boundary_enabled(&self) -> bool {
        self.file_tool_workspace_boundary.load(Ordering::Relaxed)
    }

    pub fn set_file_tool_workspace_boundary_enabled(&self, value: bool) -> Result<(), String> {
        self.file_tool_workspace_boundary
            .store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn close_behavior(&self) -> AppCloseBehavior {
        self.close_behavior
            .lock()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn set_close_behavior(&self, value: AppCloseBehavior) -> Result<(), String> {
        *self
            .close_behavior
            .lock()
            .map_err(|e| format!("close behavior lock poisoned: {}", e))? = value;
        self.persist()
    }

    pub fn dynamic_tool_loading_mode(&self) -> DynamicToolLoadingMode {
        self.dynamic_tool_loading_mode
            .lock()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn set_dynamic_tool_loading_mode(
        &self,
        value: DynamicToolLoadingMode,
    ) -> Result<(), String> {
        *self
            .dynamic_tool_loading_mode
            .lock()
            .map_err(|e| format!("dynamic tool loading mode lock poisoned: {}", e))? = value;
        self.persist()
    }

    pub fn default_skill_package_namespace(&self) -> String {
        self.default_skill_package_namespace
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn set_default_skill_package_namespace(&self, value: String) -> Result<(), String> {
        *self
            .default_skill_package_namespace
            .lock()
            .map_err(|e| format!("default skill package namespace lock poisoned: {}", e))? = value;
        self.persist()
    }

    pub fn view_windows_above_main_enabled(&self) -> bool {
        self.view_windows_above_main.load(Ordering::Relaxed)
    }

    pub fn set_view_windows_above_main_enabled(&self, value: bool) -> Result<(), String> {
        self.view_windows_above_main.store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn view_open_in_existing_window_enabled(&self) -> bool {
        self.view_open_in_existing_window.load(Ordering::Relaxed)
    }

    pub fn set_view_open_in_existing_window_enabled(&self, value: bool) -> Result<(), String> {
        self.view_open_in_existing_window
            .store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn unity_background_hook_enabled(&self) -> bool {
        self.unity_background_hook_enabled.load(Ordering::Relaxed)
    }

    pub fn csharp_lsp_enabled(&self) -> bool {
        self.csharp_lsp_enabled.load(Ordering::Relaxed)
    }

    pub fn set_csharp_lsp_enabled(&self, value: bool) -> Result<(), String> {
        self.csharp_lsp_enabled.store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn unity_sidecar_compiler_enabled(&self) -> bool {
        self.unity_sidecar_compiler.load(Ordering::Relaxed)
    }

    pub fn set_unity_sidecar_compiler_enabled(&self, value: bool) -> Result<(), String> {
        self.unity_sidecar_compiler.store(value, Ordering::Relaxed);
        self.persist()
    }

    pub fn code_analysis_tools(&self) -> CodeAnalysisToolsConfig {
        self.code_analysis_tools
            .lock()
            .map(|guard| *guard)
            .unwrap_or_default()
    }

    pub fn set_code_analysis_tools(&self, value: CodeAnalysisToolsConfig) -> Result<(), String> {
        *self
            .code_analysis_tools
            .lock()
            .map_err(|e| format!("code analysis tools lock poisoned: {}", e))? = value;
        self.persist()
    }

    pub fn set_unity_background_hook_enabled(&self, value: bool) -> Result<(), String> {
        self.unity_background_hook_enabled
            .store(value, Ordering::Relaxed);
        self.persist()
    }

    fn persist(&self) -> Result<(), String> {
        let path = self
            .config_path
            .lock()
            .map_err(|e| format!("config path lock poisoned: {}", e))?
            .clone();
        let Some(path) = path else {
            return Err("config path is unknown; cannot persist".to_string());
        };
        Self::persist_to_path(self, &path)
    }

    fn persist_to_path(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create config dir '{}': {}", parent.display(), e)
            })?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize config: {}", e))?;
        fs::write(path, json)
            .map_err(|e| format!("failed to write config '{}': {}", path.display(), e))?;
        Ok(())
    }
}

fn stable_config_path(data_dir: &Path) -> PathBuf {
    crate::commands::persistent_config_dir()
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .unwrap_or_else(|err| {
            eprintln!(
                "[Locus] failed to resolve persistent config dir, falling back to runtime storage: {}",
                err
            );
            data_dir.join(CONFIG_FILE_NAME)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    struct EnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&'static str, Option<&str>)]) -> Self {
            let saved = vars
                .iter()
                .map(|(key, value)| {
                    let previous = std::env::var(key).ok();
                    match value {
                        Some(next) => std::env::set_var(key, next),
                        None => std::env::remove_var(key),
                    }
                    (*key, previous)
                })
                .collect();
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).rev() {
                match value {
                    Some(previous) => std::env::set_var(key, previous),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn load_from_path_does_not_persist_openrouter_key_from_env() {
        let _env_lock = ENV_LOCK.lock().expect("env lock");
        let _env_guard = EnvGuard::set(&[
            ("OPENROUTER_API_KEY", Some("or-secret-value")),
            ("LOCUS_MODEL", Some("test-model")),
            ("LOCUS_BASE_URL", None),
            ("LOCUS_DEBUG", Some("0")),
        ]);
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");

        let config = AppConfig::load_from_path(&config_path);
        let written = fs::read_to_string(&config_path).expect("written config");

        assert_eq!(config.model, "test-model");
        assert!(!written.contains("api_key"));
        assert!(!written.contains("or-secret-value"));
    }

    #[test]
    fn load_from_path_scrubs_legacy_api_key_from_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "api_key": "or-legacy-secret",
  "model": "legacy-model",
  "base_url": "https://example.com",
  "debug": true
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);
        let written = fs::read_to_string(&config_path).expect("scrubbed config");

        assert_eq!(config.model, "legacy-model");
        assert_eq!(config.base_url.as_deref(), Some("https://example.com"));
        assert!(config.debug_enabled());
        assert!(!config.file_tool_workspace_boundary_enabled());
        assert!(!written.contains("api_key"));
        assert!(!written.contains("or-legacy-secret"));
    }

    #[test]
    fn file_tool_workspace_boundary_defaults_to_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert!(!config.file_tool_workspace_boundary_enabled());
    }

    #[test]
    fn close_behavior_defaults_to_exit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert_eq!(config.close_behavior(), AppCloseBehavior::Exit);
    }

    #[test]
    fn dynamic_tool_loading_mode_defaults_to_meta_tool() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert_eq!(
            config.dynamic_tool_loading_mode(),
            DynamicToolLoadingMode::MetaTool
        );
    }

    #[test]
    fn default_skill_package_namespace_defaults_to_empty() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert_eq!(config.default_skill_package_namespace(), "");
    }

    #[test]
    fn view_windows_above_main_defaults_to_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert!(!config.view_windows_above_main_enabled());
    }

    #[test]
    fn view_open_in_existing_window_defaults_to_enabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert!(config.view_open_in_existing_window_enabled());
    }

    #[test]
    fn unity_sidecar_compiler_defaults_to_enabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert!(config.unity_sidecar_compiler_enabled());
    }

    #[test]
    fn unity_background_hook_defaults_to_enabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        fs::write(
            &config_path,
            r#"{
  "model": "legacy-model",
  "debug": false
}"#,
        )
        .expect("legacy config");

        let config = AppConfig::load_from_path(&config_path);

        assert!(config.unity_background_hook_enabled());
    }

    #[test]
    fn close_behavior_persists_minimize_to_tray() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_close_behavior(AppCloseBehavior::MinimizeToTray)
            .expect("persist close behavior");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert_eq!(reloaded.close_behavior(), AppCloseBehavior::MinimizeToTray);
    }

    #[test]
    fn dynamic_tool_loading_mode_persists_direct() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_dynamic_tool_loading_mode(DynamicToolLoadingMode::Direct)
            .expect("persist dynamic tool loading mode");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert_eq!(
            reloaded.dynamic_tool_loading_mode(),
            DynamicToolLoadingMode::Direct
        );
    }

    #[test]
    fn default_skill_package_namespace_persists() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_default_skill_package_namespace("studio.tools".to_string())
            .expect("persist skill package namespace");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert_eq!(reloaded.default_skill_package_namespace(), "studio.tools");
    }

    #[test]
    fn view_windows_above_main_persists_enabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_view_windows_above_main_enabled(true)
            .expect("persist view window z-order setting");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert!(reloaded.view_windows_above_main_enabled());
    }

    #[test]
    fn view_open_in_existing_window_persists_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_view_open_in_existing_window_enabled(false)
            .expect("persist view tab opening setting");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert!(!reloaded.view_open_in_existing_window_enabled());
    }

    #[test]
    fn unity_background_hook_persists_disabled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let config_path = temp.path().join("config.json");
        let config = AppConfig::load_from_path(&config_path);

        config
            .set_unity_background_hook_enabled(false)
            .expect("persist unity background hook setting");

        let reloaded = AppConfig::load_from_path(&config_path);
        assert!(!reloaded.unity_background_hook_enabled());
    }
}
