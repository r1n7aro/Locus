use crate::knowledge_store::{KnowledgeDocument, KnowledgeStorageSource, KnowledgeType};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEBUGGER_SKILL_ID: &str = "kd_skill_builtin_debugger";
const DEBUGGER_DOCUMENT_PATH: &str = "debugger.md";
const DEBUGGER_LOGICAL_PATH: &str = "skill/debugger.md";
const DEBUGGER_PROBE_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
pub(crate) enum SkillRuntimeContextTrigger {
    Command,
    KnowledgeRead,
    Read,
}

impl SkillRuntimeContextTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::KnowledgeRead => "knowledge_read",
            Self::Read => "read",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebuggerDiscoverySource {
    ProcessPath,
    RegistryPath,
    WindowsKits,
    AppExecutionAlias,
    MsixPackage,
}

impl DebuggerDiscoverySource {
    fn as_str(self) -> &'static str {
        match self {
            Self::ProcessPath => "process_path",
            Self::RegistryPath => "registry_path",
            Self::WindowsKits => "windows_kits",
            Self::AppExecutionAlias => "app_execution_alias",
            Self::MsixPackage => "msix_package",
        }
    }
}

#[derive(Debug, Clone)]
struct DebuggerSearchRoot {
    path: PathBuf,
    source: DebuggerDiscoverySource,
    in_path: bool,
    architecture: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeDebuggerAvailability {
    name: String,
    installed: bool,
    available: bool,
    in_path: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    discovery: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    architecture: Option<String>,
    signature_status: String,
}

#[derive(Debug, Clone)]
struct DebuggerProbeSnapshot {
    observed_at: String,
    platform_supported: bool,
    debuggers: Vec<NativeDebuggerAvailability>,
}

#[derive(Debug, Clone)]
struct CachedDebuggerProbe {
    created_at: Instant,
    snapshot: DebuggerProbeSnapshot,
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone)]
struct WindbgMsixPackage {
    root: PathBuf,
    architecture: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillRuntimeContext<'a> {
    schema: &'static str,
    provider: &'static str,
    trigger: &'static str,
    observed_at: &'a str,
    snapshot: bool,
    refresh_after_seconds: u64,
    host: SkillRuntimeHost,
    debuggers: &'a [NativeDebuggerAvailability],
    guidance: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillRuntimeHost {
    os: &'static str,
    architecture: &'static str,
    windows_debuggers_supported: bool,
}

pub(crate) fn for_knowledge_document(
    document: &KnowledgeDocument,
    trigger: SkillRuntimeContextTrigger,
) -> Option<String> {
    if document.doc_type != KnowledgeType::Skill
        || document.storage_source != KnowledgeStorageSource::App
        || document.id.trim() != DEBUGGER_SKILL_ID
        || normalize_logical_path(&document.path) != DEBUGGER_DOCUMENT_PATH
    {
        return None;
    }

    Some(render_debugger_runtime_context(trigger))
}

pub(crate) fn for_selected_skill_source(
    source: &str,
    logical_path: &str,
    content: &str,
    trigger: SkillRuntimeContextTrigger,
) -> Option<String> {
    if !matches!(source.trim(), "app" | "builtin" | "builtIn")
        || normalize_logical_path(logical_path) != DEBUGGER_LOGICAL_PATH
        || skill_frontmatter_id(content).as_deref() != Some(DEBUGGER_SKILL_ID)
    {
        return None;
    }

    Some(render_debugger_runtime_context(trigger))
}

fn normalize_logical_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

fn skill_frontmatter_id(content: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct SkillIdentity {
        id: String,
    }

    let mut lines = content.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim() != "---" {
        return None;
    }

    let mut yaml = String::new();
    let mut closed = false;
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return None;
    }

    serde_yaml::from_str::<SkillIdentity>(&yaml)
        .ok()
        .map(|identity| identity.id.trim().to_string())
}

fn render_debugger_runtime_context(trigger: SkillRuntimeContextTrigger) -> String {
    let snapshot = cached_debugger_probe();
    let any_available = snapshot.debuggers.iter().any(|debugger| debugger.available);
    let any_installed = snapshot.debuggers.iter().any(|debugger| debugger.installed);
    let guidance = if !snapshot.platform_supported {
        "This host is not Windows. CDB and WinDbg are unavailable here; use the cooperative Unity debugger or a platform-appropriate external debugger."
    } else if any_available {
        "A supported native debugger was discovered. Use its resolved executable only when native thread stacks are required. Revalidate the executable and the Unity PID immediately before attach. Discovery does not verify the executable publisher or signature."
    } else if any_installed {
        "A WinDbg installation was detected, but no supported command-line executable or app execution alias was resolved. Revalidate the WinDbg app execution alias with bash before use. Continue with the cooperative Unity debugger when no CLI entry is available."
    } else {
        "No supported native debugger was discovered in PATH, Windows Kits, or the WinDbg app execution aliases. Continue with the cooperative Unity debugger. Recommend Microsoft WinDbg or Debugging Tools for Windows only when native thread stacks are required; do not install automatically."
    };
    let context = SkillRuntimeContext {
        schema: "locus.skill-runtime-context.v1",
        provider: "windows-native-debuggers",
        trigger: trigger.as_str(),
        observed_at: &snapshot.observed_at,
        snapshot: true,
        refresh_after_seconds: DEBUGGER_PROBE_CACHE_TTL.as_secs(),
        host: SkillRuntimeHost {
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            windows_debuggers_supported: snapshot.platform_supported,
        },
        debuggers: &snapshot.debuggers,
        guidance,
    };
    let json = serde_json::to_string_pretty(&context)
        .expect("Skill runtime context contains only serializable values");
    format!(
        "<locus-skill-runtime-context>\n{}\n</locus-skill-runtime-context>",
        json
    )
}

fn cached_debugger_probe() -> DebuggerProbeSnapshot {
    static CACHE: OnceLock<Mutex<Option<CachedDebuggerProbe>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entry) = cached.as_ref() {
        if entry.created_at.elapsed() < DEBUGGER_PROBE_CACHE_TTL {
            return entry.snapshot.clone();
        }
    }

    let snapshot = probe_host_debuggers();
    *cached = Some(CachedDebuggerProbe {
        created_at: Instant::now(),
        snapshot: snapshot.clone(),
    });
    snapshot
}

fn probe_host_debuggers() -> DebuggerProbeSnapshot {
    let platform_supported = cfg!(target_os = "windows");
    let debuggers = if platform_supported {
        let mut debuggers = discover_debuggers(&host_debugger_search_roots());
        #[cfg(target_os = "windows")]
        if let Some(package) = windbg_msix_packages().into_iter().next() {
            mark_windbgx_msix_installed(&mut debuggers, package.architecture.as_deref());
        }
        debuggers
    } else {
        unavailable_debuggers()
    };
    DebuggerProbeSnapshot {
        observed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        platform_supported,
        debuggers,
    }
}

fn unavailable_debuggers() -> Vec<NativeDebuggerAvailability> {
    ["cdb", "windbg", "windbgx"]
        .into_iter()
        .map(|name| NativeDebuggerAvailability {
            name: name.to_string(),
            installed: false,
            available: false,
            in_path: false,
            executable: None,
            discovery: None,
            architecture: None,
            signature_status: "not_checked".to_string(),
        })
        .collect()
}

fn discover_debuggers(search_roots: &[DebuggerSearchRoot]) -> Vec<NativeDebuggerAvailability> {
    ["cdb", "windbg", "windbgx"]
        .into_iter()
        .map(|name| discover_debugger(name, search_roots))
        .collect()
}

fn discover_debugger(
    name: &str,
    search_roots: &[DebuggerSearchRoot],
) -> NativeDebuggerAvailability {
    let file_name = format!("{}.exe", name);
    for root in search_roots {
        let candidate = root.path.join(&file_name);
        if !candidate.is_file() {
            continue;
        }
        let resolved = dunce::canonicalize(&candidate).unwrap_or(candidate);
        return NativeDebuggerAvailability {
            name: name.to_string(),
            installed: true,
            available: true,
            in_path: root.in_path,
            executable: Some(resolved.to_string_lossy().into_owned()),
            discovery: Some(root.source.as_str().to_string()),
            architecture: root
                .architecture
                .clone()
                .or_else(|| infer_debugger_architecture(&resolved)),
            signature_status: "not_checked".to_string(),
        };
    }

    NativeDebuggerAvailability {
        name: name.to_string(),
        installed: false,
        available: false,
        in_path: false,
        executable: None,
        discovery: None,
        architecture: None,
        signature_status: "not_checked".to_string(),
    }
}

fn mark_windbgx_msix_installed(
    debuggers: &mut [NativeDebuggerAvailability],
    architecture: Option<&str>,
) {
    let Some(windbgx) = debuggers
        .iter_mut()
        .find(|debugger| debugger.name == "windbgx")
    else {
        return;
    };
    windbgx.installed = true;
    if windbgx.discovery.is_none() {
        windbgx.discovery = Some(DebuggerDiscoverySource::MsixPackage.as_str().to_string());
    }
    if windbgx.architecture.is_none() {
        windbgx.architecture = architecture.map(str::to_string);
    }
}

fn infer_debugger_architecture(path: &Path) -> Option<String> {
    path.components().rev().find_map(|component| {
        let component = component.as_os_str().to_string_lossy();
        for architecture in ["x64", "x86", "arm64", "arm"] {
            if component.eq_ignore_ascii_case(architecture) {
                return Some(architecture.to_string());
            }
        }
        None
    })
}

fn host_debugger_search_roots() -> Vec<DebuggerSearchRoot> {
    let process_path = std::env::var_os("PATH");
    let effective_path =
        crate::process_util::augment_path_with_registry_paths(process_path.clone())
            .or_else(|| process_path.clone());
    let process_dirs = split_path_entries(process_path);
    let mut roots: Vec<DebuggerSearchRoot> = Vec::new();

    for directory in split_path_entries(effective_path) {
        let source = if process_dirs
            .iter()
            .any(|process_dir| paths_equal(process_dir, &directory))
        {
            DebuggerDiscoverySource::ProcessPath
        } else {
            DebuggerDiscoverySource::RegistryPath
        };
        push_unique_search_root(
            &mut roots,
            DebuggerSearchRoot {
                source: if is_windows_app_alias_directory(&directory) {
                    DebuggerDiscoverySource::AppExecutionAlias
                } else {
                    source
                },
                path: directory,
                in_path: true,
                architecture: None,
            },
        );
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            push_unique_search_root(
                &mut roots,
                DebuggerSearchRoot {
                    path: PathBuf::from(local_app_data)
                        .join("Microsoft")
                        .join("WindowsApps"),
                    source: DebuggerDiscoverySource::AppExecutionAlias,
                    in_path: false,
                    architecture: None,
                },
            );
        }

        for kit_root in windows_kits_roots() {
            for architecture in preferred_debugger_architectures() {
                push_unique_search_root(
                    &mut roots,
                    DebuggerSearchRoot {
                        path: kit_root.join("Debuggers").join(architecture),
                        source: DebuggerDiscoverySource::WindowsKits,
                        in_path: false,
                        architecture: Some(architecture.to_string()),
                    },
                );
            }
        }

        for package in windbg_msix_packages() {
            for (architecture, directory_name) in
                preferred_msix_debugger_architectures(package.architecture.as_deref())
            {
                push_unique_search_root(
                    &mut roots,
                    DebuggerSearchRoot {
                        path: package.root.join(directory_name),
                        source: DebuggerDiscoverySource::MsixPackage,
                        in_path: false,
                        architecture: Some(architecture.to_string()),
                    },
                );
            }
        }
    }

    roots
}

fn split_path_entries(path: Option<OsString>) -> Vec<PathBuf> {
    path.as_ref()
        .map(|value| std::env::split_paths(value).collect())
        .unwrap_or_default()
}

fn push_unique_search_root(roots: &mut Vec<DebuggerSearchRoot>, candidate: DebuggerSearchRoot) {
    if candidate.path.as_os_str().is_empty()
        || roots
            .iter()
            .any(|root| paths_equal(&root.path, &candidate.path))
    {
        return;
    }
    roots.push(candidate);
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(target_os = "windows") {
        left.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(right.to_string_lossy().trim_end_matches(['\\', '/']))
    } else {
        left == right
    }
}

fn is_windows_app_alias_directory(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .to_ascii_lowercase()
        .ends_with("/microsoft/windowsapps")
}

#[cfg(target_os = "windows")]
fn windows_kits_roots() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let mut roots: Vec<PathBuf> = Vec::new();
    for flags in [KEY_READ | KEY_WOW64_64KEY, KEY_READ | KEY_WOW64_32KEY] {
        let Ok(key) =
            hklm.open_subkey_with_flags(r"SOFTWARE\Microsoft\Windows Kits\Installed Roots", flags)
        else {
            continue;
        };
        for value_name in ["KitsRoot10", "KitsRoot81"] {
            let Ok(value) = key.get_value::<String, _>(value_name) else {
                continue;
            };
            let path = PathBuf::from(value.trim());
            if !path.as_os_str().is_empty() && !roots.iter().any(|root| paths_equal(root, &path)) {
                roots.push(path);
            }
        }
    }
    roots
}

#[cfg(target_os = "windows")]
fn preferred_debugger_architectures() -> &'static [&'static str] {
    match std::env::consts::ARCH {
        "x86_64" => &["x64", "x86", "arm64", "arm"],
        "x86" => &["x86", "x64", "arm64", "arm"],
        "aarch64" => &["arm64", "x64", "x86", "arm"],
        _ => &["x64", "x86", "arm64", "arm"],
    }
}

#[cfg(target_os = "windows")]
fn preferred_msix_debugger_architectures(
    package_architecture: Option<&str>,
) -> Vec<(&'static str, &'static str)> {
    let all = [
        ("x64", "amd64"),
        ("x86", "x86"),
        ("arm64", "arm64"),
        ("arm", "arm"),
    ];
    let mut ordered = Vec::new();
    if let Some(package_architecture) = package_architecture {
        if let Some(candidate) = all
            .iter()
            .find(|(architecture, _)| architecture.eq_ignore_ascii_case(package_architecture))
        {
            ordered.push(*candidate);
        }
    }
    for candidate in all {
        if !ordered.contains(&candidate) {
            ordered.push(candidate);
        }
    }
    ordered
}

#[cfg(target_os = "windows")]
fn windbg_msix_packages() -> Vec<WindbgMsixPackage> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(packages) = hkcu.open_subkey_with_flags(
        r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
        KEY_READ,
    ) else {
        return Vec::new();
    };
    let mut discovered = Vec::new();
    for package_name in packages.enum_keys().flatten() {
        if !package_name
            .to_ascii_lowercase()
            .starts_with("microsoft.windbg_")
        {
            continue;
        }
        let Ok(package) = packages.open_subkey_with_flags(&package_name, KEY_READ) else {
            continue;
        };
        let Ok(package_root) = package.get_value::<String, _>("PackageRootFolder") else {
            continue;
        };
        let root = PathBuf::from(package_root.trim());
        if !root.is_dir() {
            continue;
        }
        let architecture = package_architecture_from_name(&package_name);
        if discovered
            .iter()
            .any(|item: &WindbgMsixPackage| paths_equal(&item.root, &root))
        {
            continue;
        }
        discovered.push(WindbgMsixPackage { root, architecture });
    }
    discovered
}

#[cfg(target_os = "windows")]
fn package_architecture_from_name(package_name: &str) -> Option<String> {
    let normalized = package_name.to_ascii_lowercase();
    ["arm64", "x64", "x86", "arm"]
        .into_iter()
        .find(|architecture| normalized.contains(&format!("_{}__", architecture)))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_fake_executable(path: &Path) {
        std::fs::create_dir_all(path.parent().expect("executable parent")).unwrap();
        std::fs::write(path, b"test").unwrap();
    }

    fn debugger_document(storage_source: KnowledgeStorageSource) -> KnowledgeDocument {
        KnowledgeDocument {
            id: DEBUGGER_SKILL_ID.to_string(),
            doc_type: KnowledgeType::Skill,
            path: DEBUGGER_DOCUMENT_PATH.to_string(),
            title: "Debugger".to_string(),
            inject_mode: crate::knowledge_store::KnowledgeInjectMode::Excerpt,
            inherit_inject_mode: false,
            inject_mode_source: Default::default(),
            summary_enabled: true,
            command_enabled: true,
            read_only: true,
            ai_edit_mode: crate::knowledge_store::KnowledgeAiEditMode::Disabled,
            ai_maintained: false,
            storage_source,
            inherit_ai_config: false,
            ai_config_source: Default::default(),
            explicit_maintenance_rules: false,
            external_source: None,
            skill_enabled: Some(true),
            skill_surface: Some(crate::knowledge_store::SkillSurface::Both),
            command_trigger: Some("/debug".to_string()),
            argument_hint: None,
            tools: Vec::new(),
            summary: Some("Debug Unity".to_string()),
            body: "Use the debugger".to_string(),
            maintenance_rules: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn selected_skill_context_requires_the_builtin_app_debugger_identity() {
        let content = "---\nid: kd_skill_builtin_debugger\n---\n\n# Debugger\n";
        assert!(for_selected_skill_source(
            "app",
            DEBUGGER_LOGICAL_PATH,
            content,
            SkillRuntimeContextTrigger::Command
        )
        .is_some());
        assert!(for_selected_skill_source(
            "project",
            DEBUGGER_LOGICAL_PATH,
            content,
            SkillRuntimeContextTrigger::Command
        )
        .is_none());
        assert!(for_selected_skill_source(
            "app",
            "skill/custom/debugger.md",
            content,
            SkillRuntimeContextTrigger::Command
        )
        .is_none());
        assert!(for_selected_skill_source(
            "app",
            DEBUGGER_LOGICAL_PATH,
            "---\nid: another-skill\n---\n",
            SkillRuntimeContextTrigger::Command
        )
        .is_none());
    }

    #[test]
    fn knowledge_read_context_rejects_a_project_document_with_the_builtin_id() {
        let app_document = debugger_document(KnowledgeStorageSource::App);
        let project_document = debugger_document(KnowledgeStorageSource::Project);
        let mut wrong_path_document = app_document.clone();
        wrong_path_document.path = "custom/debugger.md".to_string();

        assert!(
            for_knowledge_document(&app_document, SkillRuntimeContextTrigger::KnowledgeRead)
                .is_some()
        );
        assert!(for_knowledge_document(
            &project_document,
            SkillRuntimeContextTrigger::KnowledgeRead
        )
        .is_none());
        assert!(for_knowledge_document(
            &wrong_path_document,
            SkillRuntimeContextTrigger::KnowledgeRead
        )
        .is_none());
    }

    #[test]
    fn discovery_prefers_path_and_falls_back_to_windows_kits() {
        let temp = tempdir().unwrap();
        let path_dir = temp.path().join("path-bin");
        let kit_dir = temp
            .path()
            .join("Windows Kits")
            .join("Debuggers")
            .join("x64");
        create_fake_executable(&path_dir.join("cdb.exe"));
        create_fake_executable(&path_dir.join("windbgx.exe"));
        create_fake_executable(&kit_dir.join("cdb.exe"));
        create_fake_executable(&kit_dir.join("windbg.exe"));

        let discovered = discover_debuggers(&[
            DebuggerSearchRoot {
                path: path_dir.clone(),
                source: DebuggerDiscoverySource::ProcessPath,
                in_path: true,
                architecture: None,
            },
            DebuggerSearchRoot {
                path: kit_dir,
                source: DebuggerDiscoverySource::WindowsKits,
                in_path: false,
                architecture: Some("x64".to_string()),
            },
        ]);

        let cdb = &discovered[0];
        assert!(cdb.installed);
        assert!(cdb.available);
        assert!(cdb.in_path);
        assert_eq!(cdb.discovery.as_deref(), Some("process_path"));
        assert!(cdb
            .executable
            .as_deref()
            .is_some_and(|path| path.ends_with("cdb.exe")));

        let windbg = &discovered[1];
        assert!(windbg.available);
        assert!(!windbg.in_path);
        assert_eq!(windbg.discovery.as_deref(), Some("windows_kits"));
        assert_eq!(windbg.architecture.as_deref(), Some("x64"));

        let windbgx = &discovered[2];
        assert!(windbgx.available);
        assert!(windbgx.in_path);
    }

    #[test]
    fn runtime_context_is_structured_and_marks_discovery_as_a_snapshot() {
        let context = render_debugger_runtime_context(SkillRuntimeContextTrigger::Read);
        assert!(context.starts_with("<locus-skill-runtime-context>\n"));
        assert!(context.ends_with("\n</locus-skill-runtime-context>"));
        assert!(context.contains("\"provider\": \"windows-native-debuggers\""));
        assert!(context.contains("\"trigger\": \"read\""));
        assert!(context.contains("\"snapshot\": true"));
        assert!(context.contains("\"refreshAfterSeconds\": 30"));
        assert!(context.contains("\"installed\":"));
        assert!(context.contains("\"signatureStatus\": \"not_checked\""));
        assert!(context.contains("\"name\": \"cdb\""));
        assert!(context.contains("\"name\": \"windbg\""));
        assert!(context.contains("\"name\": \"windbgx\""));
    }

    #[test]
    fn msix_registration_marks_windbg_installed_without_claiming_cli_availability() {
        let mut debuggers = unavailable_debuggers();
        mark_windbgx_msix_installed(&mut debuggers, Some("x64"));

        let windbgx = debuggers
            .iter()
            .find(|debugger| debugger.name == "windbgx")
            .unwrap();
        assert!(windbgx.installed);
        assert!(!windbgx.available);
        assert_eq!(windbgx.discovery.as_deref(), Some("msix_package"));
        assert_eq!(windbgx.architecture.as_deref(), Some("x64"));
        assert!(windbgx.executable.is_none());
    }
}
