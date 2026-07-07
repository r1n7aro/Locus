use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::commands;
use crate::knowledge_index::KnowledgeIndexState;
use crate::knowledge_store::{
    self, KnowledgeConfigSource, KnowledgeConfigSourceKind, KnowledgeDocument,
    KnowledgeExternalSource, KnowledgeInjectMode, KnowledgeLocalSourceMode,
    KnowledgeSourceProvider, KnowledgeType,
};

const LOCAL_REFERENCE_TEMP_ROOT_DIR: &str = ".local-reference-import";
const LOCAL_REFERENCE_BACKUP_DIR: &str = ".local-reference-backup";
const LOCAL_REFERENCE_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const LOCAL_REFERENCE_MAX_DOCS: usize = 2000;
const LOCAL_REFERENCE_WATCH_BATCH_WINDOW_MS: u64 = 750;
const LOCAL_REFERENCE_WATCH_IDLE_POLL_MS: u64 = 250;
const LOCAL_REFERENCE_SUPPORTED_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];
const LOCAL_REFERENCE_IGNORED_DIR_NAMES: &[&str] = &[
    "node_modules",
    "library",
    "temp",
    "logs",
    "obj",
    "bin",
    "target",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceImportStage {
    #[default]
    Idle,
    Scanning,
    Importing,
    Reconciling,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceImportStateKind {
    #[default]
    Idle,
    Running,
    Ready,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceSourceKind {
    File,
    Folder,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalReferenceImportLastOutcome {
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalReferenceImportRequest {
    pub source_path: String,
    pub target_path: String,
    pub mode: KnowledgeLocalSourceMode,
    #[serde(default)]
    pub ai_editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalReferenceScanPreview {
    pub source_kind: LocalReferenceSourceKind,
    pub doc_count: u32,
    pub total_file_count: u32,
    pub skipped_file_count: u32,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalReferenceImportStatus {
    pub state: LocalReferenceImportStateKind,
    pub stage: LocalReferenceImportStage,
    pub running: bool,
    pub target_path: Option<String>,
    pub managed_path: Option<String>,
    pub source_path: Option<String>,
    pub source_kind: Option<LocalReferenceSourceKind>,
    pub mode: Option<KnowledgeLocalSourceMode>,
    pub ai_editable: bool,
    pub source_missing: bool,
    pub imported_at: Option<i64>,
    pub imported_doc_count: u32,
    pub total_file_count: u32,
    pub skipped_file_count: u32,
    pub progress: Option<f32>,
    pub processed_docs: u32,
    pub total_docs: Option<u32>,
    pub current_path: Option<String>,
    pub message: String,
    pub error: Option<String>,
    pub last_outcome: Option<LocalReferenceImportLastOutcome>,
}

#[derive(Default)]
pub struct LocalReferenceImportRuntime {
    pub working_dir: String,
    pub status: LocalReferenceImportStatus,
    pub cancel_requested: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
pub struct LocalReferenceImportState(pub Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>);

pub(crate) struct LocalReferenceWatchEntry {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _watcher: RecommendedWatcher,
}

#[derive(Clone, Default)]
pub struct LocalReferenceWatcherState(
    pub Arc<std::sync::Mutex<HashMap<String, LocalReferenceWatchEntry>>>,
);

#[derive(Debug)]
enum LocalReferenceImportRunError {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
struct PlannedLocalDocument {
    source_rel: String,
    source_abs: PathBuf,
    target_rel: String,
}

#[derive(Debug, Clone)]
pub struct LocalScanOutcome {
    source_kind: LocalReferenceSourceKind,
    planned: Vec<PlannedLocalDocument>,
    total_file_count: u32,
    skipped_file_count: u32,
    total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct LocalReferenceBinding {
    pub source_path: String,
    pub mode: KnowledgeLocalSourceMode,
    pub ai_editable: bool,
    pub synced_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalReferenceSyncStats {
    pub added: u32,
    pub updated: u32,
    pub removed: u32,
    pub unchanged: u32,
    pub skipped_file_count: u32,
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn knowledge_reference_dir(working_dir: &str, target_path: &str) -> PathBuf {
    knowledge_store::knowledge_root(working_dir)
        .join("reference")
        .join(target_path.trim().trim_matches('/').replace('\\', "/"))
}

fn reference_managed_path(target_path: &str) -> String {
    format!("reference/{}", target_path.trim().trim_matches('/'))
}

fn library_dir(working_dir: &str) -> PathBuf {
    Path::new(working_dir).join("Library").join("Locus")
}

fn temp_root_path(working_dir: &str) -> PathBuf {
    library_dir(working_dir).join(LOCAL_REFERENCE_TEMP_ROOT_DIR)
}

fn backup_dir_path(working_dir: &str) -> PathBuf {
    knowledge_store::knowledge_root(working_dir)
        .join("reference")
        .join(LOCAL_REFERENCE_BACKUP_DIR)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to remove directory '{}': {}",
            path.display(),
            error
        )),
    }
}

fn short_token(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(4)
        .map(|byte| format!("{:02x}", byte))
        .collect::<String>()
}

fn stable_document_id(source_root: &str, source_rel: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_root.as_bytes());
    hasher.update(b":");
    hasher.update(source_rel.as_bytes());
    format!(
        "local-{}",
        hasher
            .finalize()
            .iter()
            .take(16)
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>()
    )
}

fn sanitize_segment(title: &str, fallback_prefix: &str, token: &str) -> String {
    let mut sanitized = title
        .trim()
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            _ if ch.is_control() => ' ',
            other => other,
        })
        .collect::<String>();
    sanitized = sanitized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('.')
        .trim()
        .to_string();
    if sanitized.is_empty() {
        sanitized = format!("{}-{}", fallback_prefix, short_token(token));
    }
    let upper = sanitized.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        sanitized = format!("{}-{}", sanitized, short_token(token));
    }
    const MAX_SEGMENT_CHARS: usize = 80;
    if sanitized.chars().count() > MAX_SEGMENT_CHARS {
        sanitized = sanitized.chars().take(MAX_SEGMENT_CHARS).collect();
        sanitized = sanitized.trim().trim_matches('.').to_string();
        if sanitized.is_empty() {
            sanitized = format!("{}-{}", fallback_prefix, short_token(token));
        }
    }
    sanitized
}

fn join_relative_path(prefix: &str, name: &str) -> String {
    let prefix = prefix.trim().trim_matches('/');
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", prefix, name)
    }
}

fn normalize_source_rel(rel: &Path) -> String {
    rel.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_body(raw: &str) -> String {
    let stripped = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let normalized = stripped.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{}\n", trimmed)
    }
}

fn supported_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| {
            LOCAL_REFERENCE_SUPPORTED_EXTENSIONS
                .iter()
                .any(|ext| value.eq_ignore_ascii_case(ext))
        })
        .unwrap_or(false)
}

fn ignored_directory_name(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    let lowered = name.to_ascii_lowercase();
    LOCAL_REFERENCE_IGNORED_DIR_NAMES
        .iter()
        .any(|ignored| lowered == *ignored)
}

fn canonical_path_key(path: &Path) -> String {
    let canonical = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    let left_key = format!("{}/", canonical_path_key(left));
    let right_key = format!("{}/", canonical_path_key(right));
    left_key.starts_with(&right_key) || right_key.starts_with(&left_key)
}

fn normalize_target_path(target_path: &str) -> Result<String, String> {
    let normalized = target_path
        .trim()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string();
    if normalized.is_empty() {
        return Err("本地导入目标目录不能为空。".to_string());
    }
    for segment in normalized.split('/') {
        let trimmed = segment.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
            return Err(format!("本地导入目标目录不合法：{}", target_path));
        }
    }
    Ok(normalized)
}

fn allocate_unique_target_rel(
    prefix: &str,
    stem: &str,
    token: &str,
    used: &mut HashSet<String>,
) -> String {
    let segment = sanitize_segment(stem, "doc", token);
    let base = join_relative_path(prefix, &format!("{}.md", segment));
    if used.insert(base.to_lowercase()) {
        return base;
    }
    let mut index = 2u32;
    loop {
        let candidate = join_relative_path(prefix, &format!("{}-{}.md", segment, index));
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
        index += 1;
    }
}

fn is_cancelled(cancel_requested: Option<&Arc<AtomicBool>>) -> bool {
    cancel_requested
        .map(|flag| flag.load(Ordering::Relaxed))
        .unwrap_or(false)
}

fn is_link_entry(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(windows)]
fn create_directory_link(source: &Path, link_path: &Path) -> Result<(), String> {
    // Junctions do not require developer mode or elevation on Windows.
    match junction::create(source, link_path) {
        Ok(()) => Ok(()),
        Err(junction_error) => std::os::windows::fs::symlink_dir(source, link_path).map_err(
            |symlink_error| {
                format!(
                    "无法创建目录链接（junction：{}；symlink：{}）。",
                    junction_error, symlink_error
                )
            },
        ),
    }
}

#[cfg(not(windows))]
fn create_directory_link(source: &Path, link_path: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, link_path)
        .map_err(|error| format!("无法创建目录链接：{}", error))
}

#[cfg(windows)]
fn create_file_link(source: &Path, link_path: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_file(source, link_path).map_err(|error| {
        format!(
            "无法创建文件链接：{}。Windows 上创建文件符号链接需要启用开发者模式或以管理员身份运行；也可以改用快照导入。",
            error
        )
    })
}

#[cfg(not(windows))]
fn create_file_link(source: &Path, link_path: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, link_path)
        .map_err(|error| format!("无法创建文件链接：{}", error))
}

fn remove_link_entry(link_path: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::symlink_metadata(link_path) else {
        return Ok(());
    };
    if !metadata.file_type().is_symlink() {
        return Err(format!(
            "目标不是链接，拒绝删除：{}",
            link_path.display()
        ));
    }
    let result = if std::fs::metadata(link_path)
        .map(|target| target.is_dir())
        .unwrap_or(true)
    {
        std::fs::remove_dir(link_path)
    } else {
        std::fs::remove_file(link_path)
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        // A dangling directory link may be reported as a file handle on some
        // platforms; retry with the opposite removal primitive before failing.
        Err(_) => std::fs::remove_file(link_path)
            .or_else(|_| std::fs::remove_dir(link_path))
            .map_err(|error| {
                format!("无法删除链接 '{}'：{}", link_path.display(), error)
            }),
    }
}

fn scan_local_source(
    source_path: &Path,
    cancel_requested: Option<&Arc<AtomicBool>>,
) -> Result<LocalScanOutcome, String> {
    if source_path.is_file() {
        if !supported_extension(source_path) {
            return Err(format!(
                "不支持的文件类型，目前支持：{}",
                LOCAL_REFERENCE_SUPPORTED_EXTENSIONS.join(", ")
            ));
        }
        let metadata = std::fs::metadata(source_path)
            .map_err(|error| format!("无法读取源文件信息：{}", error))?;
        if metadata.len() > LOCAL_REFERENCE_MAX_FILE_BYTES {
            return Err(format!(
                "源文件超过 {} MB 上限，无法导入。",
                LOCAL_REFERENCE_MAX_FILE_BYTES / (1024 * 1024)
            ));
        }
        let stem = source_path
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "document".to_string());
        let file_name = source_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| stem.clone());
        let mut used = HashSet::new();
        let target_rel = allocate_unique_target_rel("", &stem, &file_name, &mut used);
        return Ok(LocalScanOutcome {
            source_kind: LocalReferenceSourceKind::File,
            planned: vec![PlannedLocalDocument {
                source_rel: file_name,
                source_abs: source_path.to_path_buf(),
                target_rel,
            }],
            total_file_count: 1,
            skipped_file_count: 0,
            total_bytes: metadata.len(),
        });
    }

    if !source_path.is_dir() {
        return Err(format!("本地源路径不存在：{}", source_path.display()));
    }

    let mut entries = Vec::new();
    let mut total_file_count = 0u32;
    let mut skipped_file_count = 0u32;
    let walker = walkdir::WalkDir::new(source_path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().is_dir() {
                !ignored_directory_name(&name)
            } else {
                !name.starts_with('.')
            }
        });
    for entry in walker {
        if is_cancelled(cancel_requested) {
            return Err("已取消本地文档导入。".to_string());
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                skipped_file_count += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        total_file_count = total_file_count.saturating_add(1);
        if !supported_extension(entry.path()) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped_file_count += 1;
                continue;
            }
        };
        if metadata.len() > LOCAL_REFERENCE_MAX_FILE_BYTES {
            skipped_file_count += 1;
            continue;
        }
        entries.push((entry.path().to_path_buf(), metadata.len()));
        if entries.len() > LOCAL_REFERENCE_MAX_DOCS {
            return Err(format!(
                "本地源目录中的可导入文档超过 {} 个上限，请选择更小的目录。",
                LOCAL_REFERENCE_MAX_DOCS
            ));
        }
    }

    let mut used = HashSet::new();
    let mut planned = Vec::with_capacity(entries.len());
    let mut total_bytes = 0u64;
    for (path, size) in entries {
        let rel = path
            .strip_prefix(source_path)
            .map_err(|_| "扫描到的文件不在源目录内。".to_string())?;
        let source_rel = normalize_source_rel(rel);
        if source_rel.is_empty() {
            skipped_file_count += 1;
            continue;
        }
        let prefix = Path::new(&source_rel)
            .parent()
            .map(|parent| {
                parent
                    .components()
                    .filter_map(|component| match component {
                        std::path::Component::Normal(value) => {
                            Some(sanitize_segment(&value.to_string_lossy(), "dir", &source_rel))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let stem = Path::new(&source_rel)
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| "document".to_string());
        let target_rel = allocate_unique_target_rel(&prefix, &stem, &source_rel, &mut used);
        total_bytes += size;
        planned.push(PlannedLocalDocument {
            source_rel,
            source_abs: path,
            target_rel,
        });
    }

    Ok(LocalScanOutcome {
        source_kind: LocalReferenceSourceKind::Folder,
        planned,
        total_file_count,
        skipped_file_count,
        total_bytes,
    })
}

fn document_external_source(
    source_abs: &Path,
    source_rel: &str,
    mode: KnowledgeLocalSourceMode,
    ai_editable: bool,
) -> KnowledgeExternalSource {
    KnowledgeExternalSource {
        provider: KnowledgeSourceProvider::LocalFolder,
        locator: Some(source_abs.to_string_lossy().to_string()),
        source_id: Some(source_rel.to_string()),
        sync_enabled: true,
        local_mode: Some(mode),
        ai_editable: if ai_editable { Some(true) } else { None },
        ..Default::default()
    }
}

fn directory_external_source(
    source_path: &str,
    mode: KnowledgeLocalSourceMode,
    ai_editable: bool,
    synced_at: Option<i64>,
) -> KnowledgeExternalSource {
    KnowledgeExternalSource {
        provider: KnowledgeSourceProvider::LocalFolder,
        locator: Some(source_path.to_string()),
        source_id: None,
        sync_enabled: true,
        local_mode: Some(mode),
        ai_editable: if ai_editable { Some(true) } else { None },
        synced_at,
    }
}

fn build_local_document(
    planned: &PlannedLocalDocument,
    source_root: &str,
    target_path: &str,
    mode: KnowledgeLocalSourceMode,
    ai_editable: bool,
    created_at: Option<i64>,
) -> Result<KnowledgeDocument, String> {
    let raw = std::fs::read_to_string(&planned.source_abs).map_err(|error| {
        format!(
            "无法读取本地文件 '{}'：{}",
            planned.source_abs.display(),
            error
        )
    })?;
    let body = normalize_body(&raw);
    let read_only = !(mode == KnowledgeLocalSourceMode::Snapshot && ai_editable);
    Ok(KnowledgeDocument {
        id: stable_document_id(source_root, &planned.source_rel),
        doc_type: KnowledgeType::Reference,
        path: join_relative_path(target_path, &planned.target_rel),
        title: String::new(),
        inject_mode: KnowledgeInjectMode::None,
        inherit_inject_mode: false,
        inject_mode_source: KnowledgeConfigSource {
            kind: KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        summary_enabled: false,
        command_enabled: false,
        read_only,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::Project,
        inherit_ai_config: false,
        ai_config_source: KnowledgeConfigSource {
            kind: KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        explicit_maintenance_rules: false,
        external_source: Some(document_external_source(
            &planned.source_abs,
            &planned.source_rel,
            mode,
            ai_editable,
        )),
        skill_enabled: None,
        skill_surface: None,
        command_trigger: None,
        argument_hint: None,
        tools: Vec::new(),
        summary: None,
        body,
        maintenance_rules: None,
        created_at: created_at.unwrap_or(0),
        updated_at: 0,
    })
}

fn directory_summary(source_path: &str, source_kind: LocalReferenceSourceKind) -> String {
    match source_kind {
        LocalReferenceSourceKind::File => {
            format!("本地文件导入结果，来源于 {}。", source_path)
        }
        LocalReferenceSourceKind::Folder => {
            format!("本地文件夹导入结果，来源于 {}。", source_path)
        }
    }
}

fn configure_local_directory(
    working_dir: &str,
    target_path: &str,
    source_path: &str,
    source_kind: LocalReferenceSourceKind,
    mode: KnowledgeLocalSourceMode,
    ai_editable: bool,
) -> Result<(), String> {
    let editable = mode == KnowledgeLocalSourceMode::Snapshot && ai_editable;
    let mut config = knowledge_store::default_directory_config_for_type(KnowledgeType::Reference);
    config.summary = directory_summary(source_path, source_kind);
    config.inject_mode = KnowledgeInjectMode::None;
    config.inherit_inject_mode = false;
    config.ai_maintained = false;
    config.inherit_ai_config = false;
    config.allow_create_documents = editable;
    config.allow_create_directories = editable;
    config.allow_move_documents = editable;
    config.allow_move_directories = editable;
    knowledge_store::update_directory_config(
        working_dir,
        KnowledgeType::Reference,
        target_path,
        config,
    )
    .map(|_| ())
}

pub(crate) fn local_binding_from_sources(
    external_sources: &[KnowledgeExternalSource],
) -> Option<LocalReferenceBinding> {
    external_sources
        .iter()
        .find(|source| source.provider == KnowledgeSourceProvider::LocalFolder)
        .and_then(|source| {
            let source_path = source.locator.clone()?;
            Some(LocalReferenceBinding {
                source_path,
                mode: source
                    .local_mode
                    .unwrap_or(KnowledgeLocalSourceMode::Snapshot),
                ai_editable: source.ai_editable == Some(true),
                synced_at: source.synced_at,
            })
        })
}

fn read_local_binding(
    working_dir: &str,
    target_path: &str,
) -> Result<Option<LocalReferenceBinding>, String> {
    let record =
        match knowledge_store::read_directory_config(working_dir, KnowledgeType::Reference, target_path)
        {
            Ok(record) => record,
            Err(_) => return Ok(None),
        };
    if !record.exists {
        return Ok(None);
    }
    Ok(local_binding_from_sources(&record.external_sources))
}

fn ensure_valid_source_path(working_dir: &str, source_path: &str) -> Result<PathBuf, String> {
    let trimmed = source_path.trim();
    if trimmed.is_empty() {
        return Err("请选择要导入的本地文件或文件夹。".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err("本地源路径必须是绝对路径。".to_string());
    }
    if !path.exists() {
        return Err(format!("本地源路径不存在：{}", path.display()));
    }
    let knowledge_root = knowledge_store::knowledge_root(working_dir);
    if paths_overlap(&path, &knowledge_root) {
        return Err("本地源路径不能位于知识库目录内部，也不能包含知识库目录。".to_string());
    }
    Ok(path)
}

fn ensure_valid_target_directory(
    working_dir: &str,
    target_path: &str,
    requested_mode: KnowledgeLocalSourceMode,
) -> Result<(), String> {
    let record = match knowledge_store::read_directory_config(
        working_dir,
        KnowledgeType::Reference,
        target_path,
    ) {
        Ok(record) => record,
        Err(_) => return Ok(()),
    };
    if !record.exists {
        return Ok(());
    }
    if record
        .external_sources
        .iter()
        .any(|source| source.provider != KnowledgeSourceProvider::LocalFolder)
    {
        return Err("目标目录已绑定其他外部源，无法用于本地导入。".to_string());
    }
    match local_binding_from_sources(&record.external_sources) {
        Some(binding) if binding.mode != requested_mode => {
            return Err(
                "目标目录已使用另一种模式导入，请先删除现有导入再切换模式。".to_string(),
            );
        }
        Some(_) => {}
        None => {
            let physical = knowledge_reference_dir(working_dir, target_path);
            let has_content = std::fs::read_dir(&physical)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false);
            if has_content {
                return Err("目标目录已存在且非空，请换一个目录名称。".to_string());
            }
        }
    }
    Ok(())
}

pub fn preview_local_reference_import(
    working_dir: &str,
    source_path: &str,
) -> Result<LocalReferenceScanPreview, String> {
    let source = ensure_valid_source_path(working_dir, source_path)?;
    let outcome = scan_local_source(&source, None)?;
    Ok(LocalReferenceScanPreview {
        source_kind: outcome.source_kind,
        doc_count: outcome.planned.len() as u32,
        total_file_count: outcome.total_file_count,
        skipped_file_count: outcome.skipped_file_count,
        total_bytes: outcome.total_bytes,
    })
}

fn write_documents_to_root(
    root: &Path,
    outcome: &LocalScanOutcome,
    source_root: &str,
    target_path: &str,
    mode: KnowledgeLocalSourceMode,
    ai_editable: bool,
    mut on_progress: impl FnMut(usize, &PlannedLocalDocument),
    cancel_requested: Option<&Arc<AtomicBool>>,
) -> Result<(), String> {
    for (index, planned) in outcome.planned.iter().enumerate() {
        if is_cancelled(cancel_requested) {
            return Err("已取消本地文档导入。".to_string());
        }
        on_progress(index, planned);
        let document =
            build_local_document(planned, source_root, target_path, mode, ai_editable, None)?;
        let file_path =
            knowledge_store::document_path_in_root(root, KnowledgeType::Reference, &document.path)?;
        knowledge_store::save_document_to_path(&file_path, document)?;
    }
    Ok(())
}

fn swap_reference_directory(
    working_dir: &str,
    target_path: &str,
    temp_root: &Path,
) -> Result<(), String> {
    let incoming = temp_root
        .join("reference")
        .join(target_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if !incoming.is_dir() {
        return Err(format!(
            "本地导入临时目录缺失：{}",
            incoming.display()
        ));
    }
    let managed = knowledge_reference_dir(working_dir, target_path);
    let backup = backup_dir_path(working_dir);
    remove_dir_if_exists(&backup)?;
    if let Some(parent) = managed.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建目标目录父级：{}", error))?;
    }
    if managed.exists() {
        std::fs::rename(&managed, &backup)
            .map_err(|error| format!("无法备份现有目标目录：{}", error))?;
    }
    if let Err(error) = std::fs::rename(&incoming, &managed) {
        let restore_result = if backup.exists() {
            std::fs::rename(&backup, &managed)
        } else {
            Ok(())
        };
        let mut message = format!("无法切换本地导入目录：{}", error);
        if let Err(restore_error) = restore_result {
            message = format!("{}（回滚失败：{}）", message, restore_error);
        }
        return Err(message);
    }
    remove_dir_if_exists(&backup)?;
    Ok(())
}

fn list_existing_local_documents(
    working_dir: &str,
    target_path: &str,
) -> Result<Vec<(PathBuf, KnowledgeDocument)>, String> {
    let root = knowledge_reference_dir(working_dir, target_path);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut documents = Vec::new();
    for entry in walkdir::WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|error| format!("无法遍历目标目录：{}", error))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let is_markdown = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("md"))
            .unwrap_or(false);
        if !is_markdown {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&root) else {
            continue;
        };
        let full_rel = join_relative_path(target_path, &normalize_source_rel(rel));
        match knowledge_store::load_document_by_path(
            working_dir,
            KnowledgeType::Reference,
            &full_rel,
        ) {
            Ok(document) => documents.push((entry.path().to_path_buf(), document)),
            Err(error) => {
                eprintln!(
                    "[LocalReference] skipping unreadable document '{}': {}",
                    entry.path().display(),
                    error
                );
            }
        }
    }
    Ok(documents)
}

fn count_markdown_documents(root: &Path) -> u32 {
    if !root.is_dir() {
        return 0;
    }
    walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("md"))
                    .unwrap_or(false)
        })
        .count() as u32
}

fn remove_empty_directories(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_empty_directories(&path);
            let is_empty = std::fs::read_dir(&path)
                .map(|mut children| children.next().is_none())
                .unwrap_or(false);
            if is_empty {
                let _ = std::fs::remove_dir(&path);
            }
        }
    }
}

const LOCAL_REFERENCE_BINDING_GONE: &str = "目标目录没有本地导入绑定。";
const LOCAL_LIVE_MAX_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub struct LocalLiveLink {
    pub target_path: String,
    pub source_root: PathBuf,
    pub source_kind: LocalReferenceSourceKind,
}

fn read_live_binding(working_dir: &str, target_path: &str) -> Option<LocalReferenceBinding> {
    read_local_binding(working_dir, target_path)
        .ok()
        .flatten()
        .filter(|binding| binding.mode == KnowledgeLocalSourceMode::Live)
}

fn directory_path_from_sidecar_name(rel_sidecar: &str) -> Option<String> {
    let normalized = rel_sidecar.replace('\\', "/");
    let file_name = Path::new(&normalized).file_name()?.to_string_lossy();
    let dir_name = file_name
        .strip_suffix(".locus-meta")
        .or_else(|| file_name.strip_suffix(".meta"))?;
    if dir_name.is_empty() {
        return None;
    }
    let parent = Path::new(&normalized)
        .parent()
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let parent = parent.trim_matches('/');
    Some(if parent.is_empty() {
        dir_name.to_string()
    } else {
        format!("{}/{}", parent, dir_name)
    })
}

/// Discover every live-linked reference directory by scanning directory
/// sidecar files directly. This must not call back into the knowledge_store
/// listing APIs: those merge live entries via this function, and a cycle here
/// would recurse forever.
pub fn list_local_live_links(working_dir: &str) -> Vec<LocalLiveLink> {
    let reference_root = knowledge_store::knowledge_root(working_dir).join("reference");
    if !reference_root.is_dir() {
        return Vec::new();
    }
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for entry in walkdir::WalkDir::new(&reference_root)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".locus-meta") && !name.ends_with(".meta") {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&reference_root) else {
            continue;
        };
        let Some(target_path) = directory_path_from_sidecar_name(&normalize_source_rel(rel))
        else {
            continue;
        };
        if !seen.insert(target_path.to_lowercase()) {
            continue;
        }
        let Some(binding) = read_live_binding(working_dir, &target_path) else {
            continue;
        };
        let source_root = PathBuf::from(&binding.source_path);
        if source_root.is_dir() {
            links.push(LocalLiveLink {
                target_path,
                source_root,
                source_kind: LocalReferenceSourceKind::Folder,
            });
        } else if source_root.is_file() {
            links.push(LocalLiveLink {
                target_path,
                source_root,
                source_kind: LocalReferenceSourceKind::File,
            });
        }
    }

    links
}

pub fn has_local_live_links(working_dir: &str) -> bool {
    !list_local_live_links(working_dir).is_empty()
}

fn live_synthetic_document(
    link: &LocalLiveLink,
    inner_rel: &str,
    source_file: &Path,
) -> Result<KnowledgeDocument, String> {
    let raw = std::fs::read_to_string(source_file).map_err(|error| {
        format!(
            "无法读取本地链接文件 '{}'：{}",
            source_file.display(),
            error
        )
    })?;
    let body = normalize_body(&raw);
    let source_root = link.source_root.to_string_lossy().to_string();
    let timestamp = std::fs::metadata(source_file)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_else(now_millis);
    let document = KnowledgeDocument {
        id: stable_document_id(&source_root, inner_rel),
        doc_type: KnowledgeType::Reference,
        path: join_relative_path(&link.target_path, inner_rel),
        title: Path::new(inner_rel)
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_else(|| inner_rel.to_string()),
        inject_mode: KnowledgeInjectMode::None,
        inherit_inject_mode: false,
        inject_mode_source: KnowledgeConfigSource {
            kind: KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        summary_enabled: false,
        command_enabled: false,
        read_only: true,
        ai_maintained: false,
        storage_source: knowledge_store::KnowledgeStorageSource::Project,
        inherit_ai_config: false,
        ai_config_source: KnowledgeConfigSource {
            kind: KnowledgeConfigSourceKind::SelfValue,
            path: None,
        },
        explicit_maintenance_rules: false,
        external_source: Some(KnowledgeExternalSource {
            provider: KnowledgeSourceProvider::LocalFolder,
            locator: Some(source_file.to_string_lossy().to_string()),
            source_id: Some(inner_rel.to_string()),
            sync_enabled: true,
            local_mode: Some(KnowledgeLocalSourceMode::Live),
            ..Default::default()
        }),
        skill_enabled: None,
        skill_surface: None,
        command_trigger: None,
        argument_hint: None,
        tools: Vec::new(),
        summary: None,
        body,
        maintenance_rules: None,
        created_at: timestamp,
        updated_at: timestamp,
    };
    Ok(document)
}

/// Text formats that become searchable synthesized documents behind a live
/// link. Other file types still live inside the linked folder (and count
/// toward the total), they just have no document representation yet.
fn is_live_text_document_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    lowered.ends_with(".md") || lowered.ends_with(".markdown") || lowered.ends_with(".txt")
}

fn live_inner_rel_valid(inner_rel: &str) -> bool {
    !inner_rel.is_empty()
        && inner_rel
            .split('/')
            .all(|segment| {
                let trimmed = segment.trim();
                !trimmed.is_empty() && trimmed != "." && trimmed != ".." && !segment.starts_with('.')
            })
}

/// Enumerate synthesized documents for one live link (folder sources walk the
/// link target with follow enabled; file sources read the single linked file).
pub fn list_live_documents_for_link(link: &LocalLiveLink) -> Vec<KnowledgeDocument> {
    let mut documents = Vec::new();
    match link.source_kind {
        LocalReferenceSourceKind::File => {
            let file_name = link
                .source_root
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_default();
            if is_live_text_document_name(&file_name) && link.source_root.is_file() {
                if let Ok(document) = live_synthetic_document(link, &file_name, &link.source_root)
                {
                    documents.push(document);
                }
            }
        }
        LocalReferenceSourceKind::Folder => {
            let walker = walkdir::WalkDir::new(&link.source_root)
                .follow_links(true)
                .max_depth(LOCAL_LIVE_MAX_DEPTH)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|entry| {
                    if entry.depth() == 0 {
                        return true;
                    }
                    let name = entry.file_name().to_string_lossy();
                    if entry.file_type().is_dir() {
                        !ignored_directory_name(&name)
                    } else {
                        !name.starts_with('.')
                    }
                });
            for entry in walker.flatten() {
                if documents.len() >= LOCAL_REFERENCE_MAX_DOCS {
                    eprintln!(
                        "[LocalReference] live link '{}' exceeds {} documents; extra files are not indexed",
                        link.target_path, LOCAL_REFERENCE_MAX_DOCS
                    );
                    break;
                }
                if !entry.file_type().is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy();
                if !is_live_text_document_name(&name) {
                    continue;
                }
                if entry
                    .metadata()
                    .map(|metadata| metadata.len() > LOCAL_REFERENCE_MAX_FILE_BYTES)
                    .unwrap_or(true)
                {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(&link.source_root) else {
                    continue;
                };
                let inner_rel = normalize_source_rel(rel);
                if !live_inner_rel_valid(&inner_rel) {
                    continue;
                }
                if let Ok(document) = live_synthetic_document(link, &inner_rel, entry.path()) {
                    documents.push(document);
                }
            }
        }
    }
    documents
}

pub fn list_live_documents(working_dir: &str) -> Vec<KnowledgeDocument> {
    list_local_live_links(working_dir)
        .iter()
        .flat_map(list_live_documents_for_link)
        .collect()
}

/// Reference-relative document paths for all live links, without reading file
/// contents (cheap enough for query-time candidate collection).
pub fn list_live_document_paths(working_dir: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for link in list_local_live_links(working_dir) {
        match link.source_kind {
            LocalReferenceSourceKind::File => {
                let file_name = link
                    .source_root
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default();
                if is_live_text_document_name(&file_name) && link.source_root.is_file() {
                    paths.push(join_relative_path(&link.target_path, &file_name));
                }
            }
            LocalReferenceSourceKind::Folder => {
                let walker = walkdir::WalkDir::new(&link.source_root)
                    .follow_links(true)
                    .max_depth(LOCAL_LIVE_MAX_DEPTH)
                    .sort_by_file_name()
                    .into_iter()
                    .filter_entry(|entry| {
                        if entry.depth() == 0 {
                            return true;
                        }
                        let name = entry.file_name().to_string_lossy();
                        if entry.file_type().is_dir() {
                            !ignored_directory_name(&name)
                        } else {
                            !name.starts_with('.')
                        }
                    });
                let mut count = 0usize;
                for entry in walker.flatten() {
                    if count >= LOCAL_REFERENCE_MAX_DOCS {
                        break;
                    }
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    if !is_live_text_document_name(&entry.file_name().to_string_lossy()) {
                        continue;
                    }
                    let Ok(rel) = entry.path().strip_prefix(&link.source_root) else {
                        continue;
                    };
                    let inner = normalize_source_rel(rel);
                    if !live_inner_rel_valid(&inner) {
                        continue;
                    }
                    paths.push(join_relative_path(&link.target_path, &inner));
                    count += 1;
                }
            }
        }
    }
    paths
}

/// Directory paths contributed by live links: the link target itself plus any
/// sub-directories inside a folder source (relative to the reference root).
pub fn list_live_directories(working_dir: &str) -> Vec<String> {
    let mut directories = Vec::new();
    for link in list_local_live_links(working_dir) {
        directories.push(link.target_path.clone());
        if link.source_kind != LocalReferenceSourceKind::Folder {
            continue;
        }
        let walker = walkdir::WalkDir::new(&link.source_root)
            .follow_links(true)
            .max_depth(LOCAL_LIVE_MAX_DEPTH)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0 || !ignored_directory_name(&entry.file_name().to_string_lossy())
            });
        for entry in walker.flatten() {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&link.source_root) else {
                continue;
            };
            let inner = normalize_source_rel(rel);
            if !inner.is_empty() && live_inner_rel_valid(&inner) {
                directories.push(join_relative_path(&link.target_path, &inner));
            }
        }
    }
    directories
}

/// True when `rel_path` (reference-relative) falls under any live link target,
/// including the target directory itself.
pub fn path_within_live_links(links: &[LocalLiveLink], rel_path: &str) -> bool {
    let normalized = rel_path.trim().trim_matches('/').replace('\\', "/");
    links.iter().any(|link| {
        normalized == link.target_path
            || normalized.starts_with(&format!("{}/", link.target_path))
    })
}

fn live_link_for_document_path<'a>(
    links: &'a [LocalLiveLink],
    rel_path: &str,
) -> Option<(&'a LocalLiveLink, String)> {
    let normalized = rel_path.trim().trim_matches('/').replace('\\', "/");
    for link in links {
        let prefix = format!("{}/", link.target_path);
        if let Some(inner) = normalized.strip_prefix(&prefix) {
            if !inner.is_empty() {
                return Some((link, inner.to_string()));
            }
        }
    }
    None
}

/// Hook for knowledge_store: load a document that lives behind a live link.
/// Returns Ok(None) when the path is not covered by any live link.
///
/// The knowledge path normalizer appends ".md" to every request, so a live
/// document named "contract.txt" arrives here as "contract.txt.md"; try the
/// literal name first, then retry with that forced suffix stripped.
pub fn load_live_document(
    working_dir: &str,
    rel_path: &str,
) -> Result<Option<KnowledgeDocument>, String> {
    let links = list_local_live_links(working_dir);
    let Some((link, inner_rel)) = live_link_for_document_path(&links, rel_path) else {
        return Ok(None);
    };
    let mut candidates = vec![inner_rel.clone()];
    if let Some(stripped) = inner_rel.strip_suffix(".md") {
        if !stripped.is_empty() {
            candidates.push(stripped.to_string());
        }
    }
    for candidate in candidates {
        if !is_live_text_document_name(&candidate) || !live_inner_rel_valid(&candidate) {
            continue;
        }
        let source_file = match link.source_kind {
            LocalReferenceSourceKind::File => {
                let file_name = link
                    .source_root
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !candidate.eq_ignore_ascii_case(&file_name) {
                    continue;
                }
                link.source_root.clone()
            }
            LocalReferenceSourceKind::Folder => link
                .source_root
                .join(candidate.replace('/', std::path::MAIN_SEPARATOR_STR)),
        };
        if !source_file.is_file() {
            continue;
        }
        return live_synthetic_document(link, &candidate, &source_file).map(Some);
    }
    Ok(None)
}

pub fn sync_local_reference_documents(
    working_dir: &str,
    target_path: &str,
) -> Result<(LocalReferenceSyncStats, LocalReferenceBinding, LocalScanOutcome), String> {
    let target_path = normalize_target_path(target_path)?;
    let binding = read_local_binding(working_dir, &target_path)?
        .ok_or_else(|| LOCAL_REFERENCE_BINDING_GONE.to_string())?;
    if binding.mode == KnowledgeLocalSourceMode::Live {
        return Err("实时链接目录不使用文件同步，内容始终来自本地源路径。".to_string());
    }
    let source = PathBuf::from(&binding.source_path);
    if !source.exists() {
        return Err(format!(
            "本地源路径不存在：{}",
            source.display()
        ));
    }
    let source_root = source.to_string_lossy().to_string();
    let outcome = scan_local_source(&source, None)?;
    let existing = list_existing_local_documents(working_dir, &target_path)?;

    let mut existing_by_source_id: HashMap<String, (PathBuf, KnowledgeDocument)> = HashMap::new();
    let mut managed_existing = Vec::new();
    for (file_path, document) in existing {
        let Some(external_source) = document.external_source.as_ref() else {
            continue;
        };
        if external_source.provider != KnowledgeSourceProvider::LocalFolder {
            continue;
        }
        let Some(source_id) = external_source.source_id.clone() else {
            continue;
        };
        managed_existing.push(source_id.clone());
        existing_by_source_id.insert(source_id, (file_path, document));
    }

    let mut stats = LocalReferenceSyncStats {
        skipped_file_count: outcome.skipped_file_count,
        ..Default::default()
    };
    let reference_root = knowledge_reference_dir(working_dir, &target_path);
    let mut seen_source_ids = HashSet::new();

    for planned in &outcome.planned {
        seen_source_ids.insert(planned.source_rel.clone());
        let raw = std::fs::read_to_string(&planned.source_abs).map_err(|error| {
            format!(
                "无法读取本地文件 '{}'：{}",
                planned.source_abs.display(),
                error
            )
        })?;
        let body = normalize_body(&raw);
        match existing_by_source_id.get(&planned.source_rel) {
            Some((existing_path, existing_document)) => {
                let existing_target_rel = existing_path
                    .strip_prefix(&reference_root)
                    .map(normalize_source_rel)
                    .unwrap_or_default();
                let target_unchanged =
                    existing_target_rel.eq_ignore_ascii_case(&planned.target_rel);
                if target_unchanged && normalize_body(&existing_document.body) == body {
                    stats.unchanged += 1;
                    continue;
                }
                let document = build_local_document(
                    planned,
                    &source_root,
                    &target_path,
                    binding.mode,
                    binding.ai_editable,
                    Some(existing_document.created_at),
                )?;
                if !target_unchanged {
                    let _ = std::fs::remove_file(existing_path);
                }
                let file_path = knowledge_store::document_path_in_root(
                    &knowledge_store::knowledge_root(working_dir),
                    KnowledgeType::Reference,
                    &document.path,
                )?;
                knowledge_store::save_document_to_path(&file_path, document)?;
                stats.updated += 1;
            }
            None => {
                let document = build_local_document(
                    planned,
                    &source_root,
                    &target_path,
                    binding.mode,
                    binding.ai_editable,
                    None,
                )?;
                let file_path = knowledge_store::document_path_in_root(
                    &knowledge_store::knowledge_root(working_dir),
                    KnowledgeType::Reference,
                    &document.path,
                )?;
                knowledge_store::save_document_to_path(&file_path, document)?;
                stats.added += 1;
            }
        }
    }

    for source_id in managed_existing {
        if seen_source_ids.contains(&source_id) {
            continue;
        }
        if let Some((file_path, _)) = existing_by_source_id.get(&source_id) {
            std::fs::remove_file(file_path).map_err(|error| {
                format!(
                    "无法删除已移除的文档 '{}'：{}",
                    file_path.display(),
                    error
                )
            })?;
            stats.removed += 1;
        }
    }

    remove_empty_directories(&reference_root);

    let synced_at = now_millis();
    knowledge_store::update_directory_external_sources(
        working_dir,
        KnowledgeType::Reference,
        &target_path,
        vec![directory_external_source(
            &binding.source_path,
            binding.mode,
            binding.ai_editable,
            Some(synced_at),
        )],
    )?;

    Ok((stats, binding, outcome))
}

/// Remove a previous live-link import at the target location: the link entry
/// itself (folder sources) or the linked file inside a real directory (file
/// sources). Never recurses into link targets; refuses real content.
fn remove_live_target_artifacts(target_abs: &Path) -> Result<(), String> {
    if is_link_entry(target_abs) {
        return remove_link_entry(target_abs);
    }
    if !target_abs.exists() {
        return Ok(());
    }
    if !target_abs.is_dir() {
        return Err(format!(
            "目标位置已被文件占用：{}",
            target_abs.display()
        ));
    }
    let entries = std::fs::read_dir(target_abs)
        .map_err(|error| format!("无法检查目标目录：{}", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("无法检查目标目录：{}", error))?;
        if is_link_entry(&entry.path()) {
            remove_link_entry(&entry.path())?;
        } else {
            return Err(format!(
                "目标目录中存在非链接内容，拒绝清理：{}",
                entry.path().display()
            ));
        }
    }
    std::fs::remove_dir(target_abs)
        .map_err(|error| format!("无法移除旧链接目录：{}", error))
}

/// Count (searchable documents, total files) reachable through a live source.
fn count_live_source_documents(source: &Path) -> (u32, u32) {
    if source.is_file() {
        let name = source
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default();
        let docs = if is_live_text_document_name(&name) { 1 } else { 0 };
        return (docs, 1);
    }
    if !source.is_dir() {
        return (0, 0);
    }
    let walker = walkdir::WalkDir::new(source)
        .follow_links(true)
        .max_depth(LOCAL_LIVE_MAX_DEPTH)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if entry.file_type().is_dir() {
                !ignored_directory_name(&name)
            } else {
                !name.starts_with('.')
            }
        });
    let mut docs = 0u32;
    let mut total = 0u32;
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        total = total.saturating_add(1);
        if docs < LOCAL_REFERENCE_MAX_DOCS as u32
            && is_live_text_document_name(&entry.file_name().to_string_lossy())
        {
            docs += 1;
        }
    }
    (docs, total)
}

async fn run_local_live_link_import(
    app_handle: AppHandle,
    working_dir: String,
    request: LocalReferenceImportRequest,
    target_path: String,
    source: PathBuf,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    watcher_state: LocalReferenceWatcherState,
    cancel_requested: Arc<AtomicBool>,
) -> Result<LocalReferenceImportStatus, LocalReferenceImportRunError> {
    let source_root = source.to_string_lossy().to_string();
    let managed_path = reference_managed_path(&target_path);
    let source_kind = if source.is_file() {
        LocalReferenceSourceKind::File
    } else {
        LocalReferenceSourceKind::Folder
    };

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Scanning;
        status.source_kind = Some(source_kind);
        status.message = "正在扫描本地文档。".to_string();
    })
    .await;

    let (doc_count, total_file_count) = {
        let source = source.clone();
        tauri::async_runtime::spawn_blocking(move || count_live_source_documents(&source))
            .await
            .map_err(|error| {
                LocalReferenceImportRunError::Failed(format!("本地文档扫描任务执行失败：{}", error))
            })?
    };
    if cancel_requested.load(Ordering::Relaxed) {
        return Err(LocalReferenceImportRunError::Cancelled);
    }

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Importing;
        status.total_docs = Some(doc_count);
        status.total_file_count = total_file_count;
        status.message = "正在创建本地链接。".to_string();
    })
    .await;

    let target_abs = knowledge_reference_dir(&working_dir, &target_path);
    remove_live_target_artifacts(&target_abs).map_err(LocalReferenceImportRunError::Failed)?;
    if let Some(parent) = target_abs.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            LocalReferenceImportRunError::Failed(format!("无法创建目标目录父级：{}", error))
        })?;
    }
    match source_kind {
        LocalReferenceSourceKind::Folder => {
            create_directory_link(&source, &target_abs)
                .map_err(LocalReferenceImportRunError::Failed)?;
        }
        LocalReferenceSourceKind::File => {
            std::fs::create_dir_all(&target_abs).map_err(|error| {
                LocalReferenceImportRunError::Failed(format!("无法创建目标目录：{}", error))
            })?;
            let file_name = source
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "document.md".to_string());
            create_file_link(&source, &target_abs.join(&file_name))
                .map_err(LocalReferenceImportRunError::Failed)?;
        }
    }

    configure_local_directory(
        &working_dir,
        &target_path,
        &source_root,
        source_kind,
        KnowledgeLocalSourceMode::Live,
        false,
    )
    .map_err(LocalReferenceImportRunError::Failed)?;

    let imported_at = now_millis();
    knowledge_store::update_directory_external_sources(
        &working_dir,
        KnowledgeType::Reference,
        &target_path,
        vec![directory_external_source(
            &source_root,
            KnowledgeLocalSourceMode::Live,
            false,
            Some(imported_at),
        )],
    )
    .map_err(LocalReferenceImportRunError::Failed)?;

    if let Err(error) = register_live_watcher(
        app_handle.clone(),
        working_dir.clone(),
        target_path.clone(),
        source.clone(),
        knowledge_index_state.clone(),
        watcher_state.clone(),
    ) {
        eprintln!(
            "[LocalReference] failed to start live watcher for '{}': {}",
            target_path, error
        );
    }

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Reconciling;
        status.progress = Some(1.0);
        status.processed_docs = doc_count;
        status.current_path = Some(managed_path.clone());
        status.message = "正在刷新知识索引。".to_string();
    })
    .await;

    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_import_local_reference_docs",
    )
    .await
    .map_err(|error| LocalReferenceImportRunError::Failed(error.message))?;

    Ok(LocalReferenceImportStatus {
        state: LocalReferenceImportStateKind::Ready,
        stage: LocalReferenceImportStage::Ready,
        running: false,
        target_path: Some(target_path.clone()),
        managed_path: Some(managed_path.clone()),
        source_path: Some(source_root),
        source_kind: Some(source_kind),
        mode: Some(KnowledgeLocalSourceMode::Live),
        ai_editable: false,
        source_missing: false,
        imported_at: Some(imported_at),
        imported_doc_count: doc_count,
        total_file_count,
        skipped_file_count: 0,
        progress: Some(1.0),
        processed_docs: doc_count,
        total_docs: Some(doc_count),
        current_path: Some(managed_path),
        message: if doc_count == 0 {
            "本地实时链接已建立；当前没有可检索的文本文档。".to_string()
        } else {
            "本地实时链接已建立。".to_string()
        },
        error: None,
        last_outcome: None,
    })
}

async fn run_local_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    request: LocalReferenceImportRequest,
    target_path: String,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    watcher_state: LocalReferenceWatcherState,
    cancel_requested: Arc<AtomicBool>,
) -> Result<LocalReferenceImportStatus, LocalReferenceImportRunError> {
    let ensure_active = |cancel: &Arc<AtomicBool>| -> Result<(), LocalReferenceImportRunError> {
        if cancel.load(Ordering::Relaxed) {
            Err(LocalReferenceImportRunError::Cancelled)
        } else {
            Ok(())
        }
    };

    let source = ensure_valid_source_path(&working_dir, &request.source_path)
        .map_err(LocalReferenceImportRunError::Failed)?;
    let source_root = source.to_string_lossy().to_string();
    let managed_path = reference_managed_path(&target_path);

    {
        // A live watcher from a previous import must not race the directory
        // swap below; re-register happens after the import lands.
        let watcher_state = watcher_state.clone();
        let target = target_path.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || {
            unregister_live_watcher(&watcher_state, &target);
        })
        .await;
    }

    if request.mode == KnowledgeLocalSourceMode::Live {
        return run_local_live_link_import(
            app_handle,
            working_dir,
            request,
            target_path,
            source,
            state,
            knowledge_index_state,
            watcher_state,
            cancel_requested,
        )
        .await;
    }

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Scanning;
        status.message = "正在扫描本地文档。".to_string();
    })
    .await;

    ensure_active(&cancel_requested)?;
    let outcome = scan_local_source(&source, Some(&cancel_requested))
        .map_err(LocalReferenceImportRunError::Failed)?;
    if outcome.planned.is_empty() {
        return Err(LocalReferenceImportRunError::Failed(format!(
            "所选路径中没有可导入的文档（支持 {}）。",
            LOCAL_REFERENCE_SUPPORTED_EXTENSIONS.join(", ")
        )));
    }

    let total = outcome.planned.len() as u32;
    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Importing;
        status.progress = Some(0.0);
        status.total_docs = Some(total);
        status.processed_docs = 0;
        status.source_kind = Some(outcome.source_kind);
        status.skipped_file_count = outcome.skipped_file_count;
        status.message = "正在导入本地文档。".to_string();
    })
    .await;

    let temp_root = temp_root_path(&working_dir);
    remove_dir_if_exists(&temp_root).map_err(LocalReferenceImportRunError::Failed)?;
    std::fs::create_dir_all(&temp_root)
        .map_err(|error| LocalReferenceImportRunError::Failed(format!(
            "无法创建本地导入临时目录：{}",
            error
        )))?;

    let progress_state = state.clone();
    let progress_working_dir = working_dir.clone();
    let write_result = {
        let outcome = outcome.clone();
        let target_path = target_path.clone();
        let source_root = source_root.clone();
        let temp_root = temp_root.clone();
        let cancel = cancel_requested.clone();
        tauri::async_runtime::spawn_blocking(move || {
            write_documents_to_root(
                &temp_root,
                &outcome,
                &source_root,
                &target_path,
                request.mode,
                request.ai_editable,
                |index, planned| {
                    let state = progress_state.clone();
                    let working_dir = progress_working_dir.clone();
                    let current = planned.target_rel.clone();
                    let processed = index as u32;
                    tauri::async_runtime::block_on(update_runtime_status(
                        state,
                        &working_dir,
                        move |status| {
                            status.processed_docs = processed;
                            status.progress = Some(processed as f32 / total.max(1) as f32);
                            status.current_path = Some(current.clone());
                        },
                    ));
                },
                Some(&cancel),
            )
        })
        .await
        .map_err(|error| LocalReferenceImportRunError::Failed(format!(
            "本地导入任务执行失败：{}",
            error
        )))?
    };
    if let Err(error) = write_result {
        let _ = remove_dir_if_exists(&temp_root);
        if cancel_requested.load(Ordering::Relaxed) {
            return Err(LocalReferenceImportRunError::Cancelled);
        }
        return Err(LocalReferenceImportRunError::Failed(error));
    }

    ensure_active(&cancel_requested).map_err(|error| {
        let _ = remove_dir_if_exists(&temp_root);
        error
    })?;

    update_runtime_status(state.clone(), &working_dir, |status| {
        status.stage = LocalReferenceImportStage::Reconciling;
        status.progress = Some(1.0);
        status.processed_docs = total;
        status.current_path = Some(managed_path.clone());
        status.message = "正在切换目录并刷新知识索引。".to_string();
    })
    .await;

    swap_reference_directory(&working_dir, &target_path, &temp_root)
        .map_err(LocalReferenceImportRunError::Failed)?;
    remove_dir_if_exists(&temp_root).map_err(LocalReferenceImportRunError::Failed)?;

    configure_local_directory(
        &working_dir,
        &target_path,
        &source_root,
        outcome.source_kind,
        request.mode,
        request.ai_editable,
    )
    .map_err(LocalReferenceImportRunError::Failed)?;

    let imported_at = now_millis();
    knowledge_store::update_directory_external_sources(
        &working_dir,
        KnowledgeType::Reference,
        &target_path,
        vec![directory_external_source(
            &source_root,
            request.mode,
            request.ai_editable,
            Some(imported_at),
        )],
    )
    .map_err(LocalReferenceImportRunError::Failed)?;

    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_import_local_reference_docs",
    )
    .await
    .map_err(|error| LocalReferenceImportRunError::Failed(error.message))?;

    Ok(LocalReferenceImportStatus {
        state: LocalReferenceImportStateKind::Ready,
        stage: LocalReferenceImportStage::Ready,
        running: false,
        target_path: Some(target_path.clone()),
        managed_path: Some(managed_path.clone()),
        source_path: Some(source_root),
        source_kind: Some(outcome.source_kind),
        mode: Some(request.mode),
        ai_editable: request.ai_editable,
        source_missing: false,
        imported_at: Some(imported_at),
        imported_doc_count: total,
        total_file_count: outcome.total_file_count,
        skipped_file_count: outcome.skipped_file_count,
        progress: Some(1.0),
        processed_docs: total,
        total_docs: Some(total),
        current_path: Some(managed_path),
        message: "本地文档导入完成。".to_string(),
        error: None,
        last_outcome: None,
    })
}

async fn update_runtime_status<F>(
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
    working_dir: &str,
    mutate: F,
) where
    F: FnOnce(&mut LocalReferenceImportStatus),
{
    let mut runtime = state.lock().await;
    runtime.working_dir = working_dir.to_string();
    mutate(&mut runtime.status);
}

async fn set_runtime_status(
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
    working_dir: &str,
    status: LocalReferenceImportStatus,
) {
    let mut runtime = state.lock().await;
    runtime.working_dir = working_dir.to_string();
    runtime.status = status;
}

pub async fn start_local_reference_import(
    app_handle: AppHandle,
    working_dir: String,
    request: LocalReferenceImportRequest,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
    watcher_state: LocalReferenceWatcherState,
) -> Result<LocalReferenceImportStatus, String> {
    let mut request = request;
    if request.mode == KnowledgeLocalSourceMode::Live {
        request.ai_editable = false;
    }
    let target_path = normalize_target_path(&request.target_path)?;
    let source = ensure_valid_source_path(&working_dir, &request.source_path)?;
    if request.mode == KnowledgeLocalSourceMode::Live
        && source.is_file()
        && !is_live_text_document_name(&source.file_name().unwrap_or_default().to_string_lossy())
    {
        return Err(
            "实时链接的单文件源目前仅支持文本文档（.md / .markdown / .txt)；其他格式请选择所在文件夹或使用快照导入。"
                .to_string(),
        );
    }
    ensure_valid_target_directory(&working_dir, &target_path, request.mode)?;

    let managed_path = reference_managed_path(&target_path);
    let starting_status = LocalReferenceImportStatus {
        state: LocalReferenceImportStateKind::Running,
        stage: LocalReferenceImportStage::Scanning,
        running: true,
        target_path: Some(target_path.clone()),
        managed_path: Some(managed_path.clone()),
        source_path: Some(request.source_path.trim().to_string()),
        source_kind: None,
        mode: Some(request.mode),
        ai_editable: request.ai_editable,
        source_missing: false,
        imported_at: None,
        imported_doc_count: 0,
        total_file_count: 0,
        skipped_file_count: 0,
        progress: None,
        processed_docs: 0,
        total_docs: None,
        current_path: Some(managed_path),
        message: "正在准备本地文档导入。".to_string(),
        error: None,
        last_outcome: None,
    };

    {
        let mut runtime = state.lock().await;
        if runtime.status.running {
            return Err("本地文档导入任务仍在进行中。".to_string());
        }
        runtime.working_dir = working_dir.clone();
        runtime.cancel_requested.store(false, Ordering::Relaxed);
        runtime.status = starting_status.clone();
    }

    let cancel_requested = {
        let runtime = state.lock().await;
        runtime.cancel_requested.clone()
    };
    let state_for_task = state.clone();
    let working_dir_for_task = working_dir.clone();
    let request_for_task = request.clone();
    let target_path_for_task = target_path.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = run_local_reference_import(
            app_handle,
            working_dir_for_task.clone(),
            request_for_task,
            target_path_for_task.clone(),
            state_for_task.clone(),
            knowledge_index_state,
            watcher_state,
            cancel_requested,
        )
        .await;
        match outcome {
            Ok(status) => {
                set_runtime_status(state_for_task, &working_dir_for_task, status).await;
            }
            Err(LocalReferenceImportRunError::Cancelled) => {
                let fallback = status_from_binding(
                    &working_dir_for_task,
                    Some(&target_path_for_task),
                )
                .unwrap_or_default();
                set_runtime_status(
                    state_for_task,
                    &working_dir_for_task,
                    LocalReferenceImportStatus {
                        running: false,
                        stage: LocalReferenceImportStage::Idle,
                        last_outcome: Some(LocalReferenceImportLastOutcome::Cancelled),
                        message: "已取消本地文档导入。".to_string(),
                        error: None,
                        progress: None,
                        processed_docs: 0,
                        total_docs: None,
                        current_path: None,
                        ..fallback
                    },
                )
                .await;
            }
            Err(LocalReferenceImportRunError::Failed(error)) => {
                let fallback = status_from_binding(
                    &working_dir_for_task,
                    Some(&target_path_for_task),
                )
                .unwrap_or_default();
                set_runtime_status(
                    state_for_task,
                    &working_dir_for_task,
                    LocalReferenceImportStatus {
                        running: false,
                        state: LocalReferenceImportStateKind::Error,
                        stage: LocalReferenceImportStage::Error,
                        message: error.clone(),
                        error: Some(error),
                        progress: None,
                        current_path: None,
                        ..fallback
                    },
                )
                .await;
            }
        }
    });

    Ok(starting_status)
}

fn status_from_binding(
    working_dir: &str,
    target_path: Option<&str>,
) -> Result<LocalReferenceImportStatus, String> {
    let Some(target_path) = target_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(LocalReferenceImportStatus {
            message: "尚未选择本地导入目录。".to_string(),
            ..Default::default()
        });
    };
    let target_path = normalize_target_path(target_path)?;
    let Some(binding) = read_local_binding(working_dir, &target_path)? else {
        return Ok(LocalReferenceImportStatus {
            target_path: Some(target_path.clone()),
            managed_path: Some(reference_managed_path(&target_path)),
            message: "该目录尚未绑定本地导入来源。".to_string(),
            ..Default::default()
        });
    };
    let source = PathBuf::from(&binding.source_path);
    let source_missing = !source.exists();
    let source_kind = if source.is_file() {
        Some(LocalReferenceSourceKind::File)
    } else if source.is_dir() {
        Some(LocalReferenceSourceKind::Folder)
    } else {
        None
    };
    let (doc_count, total_file_count) = if binding.mode == KnowledgeLocalSourceMode::Live {
        if source_missing {
            (0, 0)
        } else {
            count_live_source_documents(&source)
        }
    } else {
        let count = count_markdown_documents(&knowledge_reference_dir(working_dir, &target_path));
        (count, count)
    };
    Ok(LocalReferenceImportStatus {
        state: LocalReferenceImportStateKind::Ready,
        stage: LocalReferenceImportStage::Ready,
        running: false,
        target_path: Some(target_path.clone()),
        managed_path: Some(reference_managed_path(&target_path)),
        source_path: Some(binding.source_path.clone()),
        source_kind,
        mode: Some(binding.mode),
        ai_editable: binding.ai_editable,
        source_missing,
        imported_at: binding.synced_at,
        imported_doc_count: doc_count,
        total_file_count,
        skipped_file_count: 0,
        progress: None,
        processed_docs: 0,
        total_docs: None,
        current_path: None,
        message: if source_missing {
            "本地源路径已不存在，同步不可用。".to_string()
        } else {
            "本地文档已导入。".to_string()
        },
        error: None,
        last_outcome: None,
    })
}

pub async fn get_local_reference_import_status(
    working_dir: &str,
    target_path: Option<&str>,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
) -> Result<LocalReferenceImportStatus, String> {
    let runtime_status = {
        let runtime = state.lock().await;
        if runtime.working_dir == working_dir
            && (runtime.status.running
                || runtime.status.error.is_some()
                || runtime.status.last_outcome.is_some())
        {
            Some(runtime.status.clone())
        } else {
            None
        }
    };
    if let Some(status) = runtime_status {
        let matches_target = match (target_path, status.target_path.as_deref()) {
            (Some(requested), Some(current)) => {
                normalize_target_path(requested).ok().as_deref() == Some(current)
            }
            (None, _) => true,
            _ => false,
        };
        if matches_target {
            return Ok(status);
        }
    }
    status_from_binding(working_dir, target_path)
}

pub async fn cancel_local_reference_import(
    working_dir: &str,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
) -> Result<LocalReferenceImportStatus, String> {
    let runtime = state.lock().await;
    if !runtime.status.running {
        return Ok(runtime.status.clone());
    }
    runtime.cancel_requested.store(true, Ordering::Relaxed);
    let mut status = runtime.status.clone();
    status.message = "正在取消本地文档导入。".to_string();
    let _ = working_dir;
    Ok(status)
}

pub async fn resync_local_reference_docs(
    app_handle: AppHandle,
    working_dir: String,
    target_path: String,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    state: Arc<tokio::sync::Mutex<LocalReferenceImportRuntime>>,
) -> Result<LocalReferenceImportStatus, String> {
    {
        let runtime = state.lock().await;
        if runtime.status.running {
            return Err("本地文档导入任务仍在进行中。".to_string());
        }
    }
    let normalized_target = normalize_target_path(&target_path)?;
    let binding = read_local_binding(&working_dir, &normalized_target)?
        .ok_or_else(|| LOCAL_REFERENCE_BINDING_GONE.to_string())?;

    if binding.mode == KnowledgeLocalSourceMode::Live {
        // Live links always serve current content; a manual sync just
        // refreshes the search index against the source path.
        if !PathBuf::from(&binding.source_path).exists() {
            return Err(format!("本地源路径不存在：{}", binding.source_path));
        }
        commands::reconcile_and_emit_knowledge_changed(
            &app_handle,
            &working_dir,
            knowledge_index_state,
            "knowledge_sync_local_reference_docs",
        )
        .await
        .map_err(|error| error.message)?;
        let mut status = status_from_binding(&working_dir, Some(&normalized_target))?;
        status.message = "已刷新本地链接的知识索引。".to_string();
        return Ok(status);
    }

    let sync_working_dir = working_dir.clone();
    let sync_target = normalized_target.clone();
    let (stats, _, _) = tauri::async_runtime::spawn_blocking(move || {
        sync_local_reference_documents(&sync_working_dir, &sync_target)
    })
    .await
    .map_err(|error| format!("本地文档同步任务执行失败：{}", error))??;

    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_sync_local_reference_docs",
    )
    .await
    .map_err(|error| error.message)?;

    let mut status = status_from_binding(&working_dir, Some(&normalized_target))?;
    status.message = format!(
        "本地文档同步完成：新增 {}、更新 {}、删除 {}。",
        stats.added, stats.updated, stats.removed
    );
    Ok(status)
}

pub async fn delete_local_reference_docs(
    app_handle: AppHandle,
    working_dir: String,
    target_path: String,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    watcher_state: LocalReferenceWatcherState,
) -> Result<(), String> {
    let normalized_target = normalize_target_path(&target_path)?;
    let Some(binding) = read_local_binding(&working_dir, &normalized_target)? else {
        return Err("目标目录没有本地导入绑定。".to_string());
    };
    {
        let watcher_state = watcher_state.clone();
        let target = normalized_target.clone();
        tauri::async_runtime::spawn_blocking(move || {
            unregister_live_watcher(&watcher_state, &target);
        })
        .await
        .map_err(|error| format!("本地源监听注销失败：{}", error))?;
    }
    if binding.mode == KnowledgeLocalSourceMode::Live {
        // Remove only the link artifacts and the sidecar; the source files on
        // disk must never be touched.
        let target_abs = knowledge_reference_dir(&working_dir, &normalized_target);
        remove_live_target_artifacts(&target_abs)?;
        knowledge_store::delete_directory_config_sidecars(
            &working_dir,
            KnowledgeType::Reference,
            &normalized_target,
        )?;
    } else {
        knowledge_store::delete_external_reference_directory(&working_dir, &normalized_target)?;
    }
    commands::reconcile_and_emit_knowledge_changed(
        &app_handle,
        &working_dir,
        knowledge_index_state,
        "knowledge_delete_local_reference_docs",
    )
    .await
    .map_err(|error| error.message)?;
    Ok(())
}

pub fn register_live_watcher(
    app_handle: AppHandle,
    working_dir: String,
    target_path: String,
    source_path: PathBuf,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    watcher_state: LocalReferenceWatcherState,
) -> Result<(), String> {
    unregister_live_watcher(&watcher_state, &target_path);

    let watch_file_name = if source_path.is_file() {
        source_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
    } else {
        None
    };
    let (watch_root, recursive_mode) = if source_path.is_file() {
        let parent = source_path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| "无法监听没有父目录的文件。".to_string())?;
        (parent, RecursiveMode::NonRecursive)
    } else if source_path.is_dir() {
        (source_path.clone(), RecursiveMode::Recursive)
    } else {
        return Err(format!(
            "本地源路径不存在：{}",
            source_path.display()
        ));
    };

    let (tx, rx) = std::sync::mpsc::channel();
    let mut os_watcher = RecommendedWatcher::new(tx, Config::default())
        .map_err(|error| format!("无法创建本地源监听器：{}", error))?;
    os_watcher
        .watch(&watch_root, recursive_mode)
        .map_err(|error| {
            format!(
                "无法监听本地源路径 '{}'：{}",
                watch_root.display(),
                error
            )
        })?;

    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = stop.clone();
    let worker_target = target_path.clone();
    let worker = std::thread::Builder::new()
        .name(format!("local-reference-watcher-{}", short_token(&target_path)))
        .spawn(move || {
            live_watcher_loop(
                rx,
                worker_stop,
                app_handle,
                working_dir,
                worker_target,
                watch_file_name,
                knowledge_index_state,
            );
        })
        .map_err(|error| format!("无法启动本地源监听线程：{}", error))?;

    let mut entries = watcher_state
        .0
        .lock()
        .map_err(|_| "本地源监听状态已损坏。".to_string())?;
    entries.insert(
        target_path,
        LocalReferenceWatchEntry {
            stop,
            worker: Some(worker),
            _watcher: os_watcher,
        },
    );
    Ok(())
}

fn relevant_watch_event(event: &Event, watch_file_name: Option<&str>) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    match watch_file_name {
        Some(file_name) => event.paths.iter().any(|path| {
            path.file_name()
                .map(|value| value.to_string_lossy().eq_ignore_ascii_case(file_name))
                .unwrap_or(false)
        }),
        None => true,
    }
}

fn live_watcher_loop(
    rx: std::sync::mpsc::Receiver<notify::Result<Event>>,
    stop: Arc<AtomicBool>,
    app_handle: AppHandle,
    working_dir: String,
    target_path: String,
    watch_file_name: Option<String>,
    knowledge_index_state: Arc<KnowledgeIndexState>,
) {
    while !stop.load(Ordering::Relaxed) {
        let first = match rx.recv_timeout(Duration::from_millis(LOCAL_REFERENCE_WATCH_IDLE_POLL_MS))
        {
            Ok(event) => Some(event),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let Some(first) = first else {
            continue;
        };

        let mut batch = vec![first];
        let deadline = Instant::now() + Duration::from_millis(LOCAL_REFERENCE_WATCH_BATCH_WINDOW_MS);
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining) {
                Ok(event) => batch.push(event),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let has_relevant = batch.iter().any(|entry| match entry {
            Ok(event) => relevant_watch_event(event, watch_file_name.as_deref()),
            Err(_) => true,
        });
        if !has_relevant {
            continue;
        }

        // Live links serve content straight from the source path; the only
        // job here is refreshing the search index and notifying the UI.
        if read_live_binding(&working_dir, &target_path).is_none() {
            // The bound directory was removed outside the local import flow;
            // keep the entry inert until the next restore pass.
            stop.store(true, Ordering::Relaxed);
            break;
        }
        if let Err(error) =
            tauri::async_runtime::block_on(commands::reconcile_and_emit_knowledge_changed(
                &app_handle,
                &working_dir,
                knowledge_index_state.clone(),
                "local_reference_live_sync",
            ))
        {
            eprintln!(
                "[LocalReference] live index refresh failed for '{}': {}",
                target_path, error.message
            );
        }
    }
}

pub fn unregister_live_watcher(watcher_state: &LocalReferenceWatcherState, target_path: &str) {
    let entry = {
        let Ok(mut entries) = watcher_state.0.lock() else {
            return;
        };
        entries.remove(target_path)
    };
    if let Some(mut entry) = entry {
        entry.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = entry.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn clear_live_watchers(watcher_state: &LocalReferenceWatcherState) {
    let entries = {
        let Ok(mut entries) = watcher_state.0.lock() else {
            return;
        };
        std::mem::take(&mut *entries)
    };
    for (_, mut entry) in entries {
        entry.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = entry.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn restore_live_watchers(
    app_handle: AppHandle,
    working_dir: String,
    knowledge_index_state: Arc<KnowledgeIndexState>,
    watcher_state: LocalReferenceWatcherState,
) {
    clear_live_watchers(&watcher_state);
    for link in list_local_live_links(&working_dir) {
        if let Err(error) = register_live_watcher(
            app_handle.clone(),
            working_dir.clone(),
            link.target_path.clone(),
            link.source_root,
            knowledge_index_state.clone(),
            watcher_state.clone(),
        ) {
            eprintln!(
                "[LocalReference] failed to restore live watcher for '{}': {}",
                link.target_path, error
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, content).expect("write file");
    }

    fn seed_import(
        working_dir: &str,
        source_root: &Path,
        target_path: &str,
        mode: KnowledgeLocalSourceMode,
        ai_editable: bool,
    ) {
        let outcome = scan_local_source(source_root, None).expect("scan source");
        let knowledge_root = knowledge_store::knowledge_root(working_dir);
        write_documents_to_root(
            &knowledge_root,
            &outcome,
            &source_root.to_string_lossy(),
            target_path,
            mode,
            ai_editable,
            |_, _| {},
            None,
        )
        .expect("write documents");
        configure_local_directory(
            working_dir,
            target_path,
            &source_root.to_string_lossy(),
            outcome.source_kind,
            mode,
            ai_editable,
        )
        .expect("configure directory");
        knowledge_store::update_directory_external_sources(
            working_dir,
            KnowledgeType::Reference,
            target_path,
            vec![directory_external_source(
                &source_root.to_string_lossy(),
                mode,
                ai_editable,
                Some(1),
            )],
        )
        .expect("bind directory");
    }

    #[test]
    fn scan_filters_unsupported_and_ignored_entries() {
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "# Guide\n");
        write_file(source.path(), "notes.TXT", "plain text");
        write_file(source.path(), "image.png", "not text");
        write_file(source.path(), ".hidden/secret.md", "hidden");
        write_file(source.path(), "node_modules/pkg/readme.md", "ignored");
        write_file(source.path(), "docs/deep/topic.markdown", "topic");

        let outcome = scan_local_source(source.path(), None).expect("scan");
        assert_eq!(outcome.source_kind, LocalReferenceSourceKind::Folder);
        assert_eq!(
            outcome.total_file_count, 4,
            "unsupported files count toward the total, ignored dirs do not"
        );
        let rels: Vec<_> = outcome
            .planned
            .iter()
            .map(|planned| planned.source_rel.as_str())
            .collect();
        assert_eq!(rels, vec!["docs/deep/topic.markdown", "guide.md", "notes.TXT"]);
        let targets: Vec<_> = outcome
            .planned
            .iter()
            .map(|planned| planned.target_rel.as_str())
            .collect();
        assert_eq!(targets, vec!["docs/deep/topic.md", "guide.md", "notes.md"]);
    }

    #[test]
    fn scan_allocates_unique_targets_for_conflicting_stems() {
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "notes.md", "one");
        write_file(source.path(), "notes.txt", "two");

        let outcome = scan_local_source(source.path(), None).expect("scan");
        let targets: Vec<_> = outcome
            .planned
            .iter()
            .map(|planned| planned.target_rel.as_str())
            .collect();
        assert_eq!(targets, vec!["notes.md", "notes-2.md"]);
    }

    #[test]
    fn scan_single_file_source() {
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "manual.txt", "manual body");

        let outcome =
            scan_local_source(&source.path().join("manual.txt"), None).expect("scan file");
        assert_eq!(outcome.source_kind, LocalReferenceSourceKind::File);
        assert_eq!(outcome.planned.len(), 1);
        assert_eq!(outcome.planned[0].source_rel, "manual.txt");
        assert_eq!(outcome.planned[0].target_rel, "manual.md");
    }

    #[test]
    fn snapshot_documents_editable_flag_controls_read_only() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "# Guide\nbody\n");

        seed_import(
            &working_dir,
            source.path(),
            "local-editable",
            KnowledgeLocalSourceMode::Snapshot,
            true,
        );
        let editable_doc = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-editable/guide.md",
        )
        .expect("load editable document");
        assert!(!editable_doc.read_only);
        assert_eq!(
            editable_doc
                .external_source
                .as_ref()
                .and_then(|source| source.local_mode),
            Some(KnowledgeLocalSourceMode::Snapshot)
        );

        seed_import(
            &working_dir,
            source.path(),
            "local-readonly",
            KnowledgeLocalSourceMode::Snapshot,
            false,
        );
        let readonly_doc = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-readonly/guide.md",
        )
        .expect("load readonly document");
        assert!(readonly_doc.read_only);

        seed_import(
            &working_dir,
            source.path(),
            "local-live",
            KnowledgeLocalSourceMode::Live,
            true,
        );
        let live_doc = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-live/guide.md",
        )
        .expect("load live document");
        assert!(
            live_doc.read_only,
            "live documents stay read-only even when aiEditable was requested"
        );
    }

    #[test]
    fn sync_applies_additions_updates_and_removals() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "keep.md", "keep v1");
        write_file(source.path(), "drop.md", "drop v1");

        seed_import(
            &working_dir,
            source.path(),
            "local-sync",
            KnowledgeLocalSourceMode::Snapshot,
            false,
        );

        write_file(source.path(), "keep.md", "keep v2");
        write_file(source.path(), "fresh.txt", "fresh body");
        std::fs::remove_file(source.path().join("drop.md")).expect("remove source doc");

        let (stats, binding, _) =
            sync_local_reference_documents(&working_dir, "local-sync").expect("sync");
        assert_eq!(stats.added, 1, "stats={:?}", stats);
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.removed, 1);
        assert_eq!(binding.mode, KnowledgeLocalSourceMode::Snapshot);

        let kept = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-sync/keep.md",
        )
        .expect("load kept document");
        assert_eq!(normalize_body(&kept.body), "keep v2\n");
        let fresh = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-sync/fresh.md",
        )
        .expect("load fresh document");
        assert_eq!(normalize_body(&fresh.body), "fresh body\n");
        assert!(knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-sync/drop.md",
        )
        .is_err());

        let record = knowledge_store::read_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            "local-sync",
        )
        .expect("read directory config");
        let synced = local_binding_from_sources(&record.external_sources).expect("binding");
        assert!(synced.synced_at.unwrap_or(0) > 1);
    }

    #[test]
    fn sync_preserves_unmanaged_documents_in_editable_snapshot() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "guide");

        seed_import(
            &working_dir,
            source.path(),
            "local-mixed",
            KnowledgeLocalSourceMode::Snapshot,
            true,
        );

        let manual = KnowledgeDocument {
            external_source: None,
            ..build_local_document(
                &PlannedLocalDocument {
                    source_rel: "manual.md".to_string(),
                    source_abs: source.path().join("guide.md"),
                    target_rel: "manual.md".to_string(),
                },
                &source.path().to_string_lossy(),
                "local-mixed",
                KnowledgeLocalSourceMode::Snapshot,
                true,
                None,
            )
            .expect("build manual document")
        };
        let manual_path = knowledge_store::document_path_in_root(
            &knowledge_store::knowledge_root(&working_dir),
            KnowledgeType::Reference,
            &manual.path,
        )
        .expect("manual path");
        knowledge_store::save_document_to_path(&manual_path, manual).expect("save manual doc");

        let (stats, _, _) =
            sync_local_reference_documents(&working_dir, "local-mixed").expect("sync");
        assert_eq!(stats.removed, 0);
        assert!(knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-mixed/manual.md",
        )
        .is_ok());
    }

    #[test]
    fn source_inside_knowledge_root_is_rejected() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let inside = knowledge_store::knowledge_root(&working_dir).join("reference");
        std::fs::create_dir_all(&inside).expect("create inside dir");

        let error = ensure_valid_source_path(&working_dir, &inside.to_string_lossy())
            .expect_err("must reject knowledge-root source");
        assert!(error.contains("知识库目录"));
    }

    #[test]
    fn normalize_body_strips_bom_and_normalizes_newlines() {
        assert_eq!(normalize_body("\u{feff}a\r\nb\r"), "a\nb\n");
        assert_eq!(normalize_body("\n\nbody\n\n"), "body\n");
        assert_eq!(normalize_body("  "), "  \n");
        assert_eq!(normalize_body(""), "");
    }

    #[test]
    fn target_path_normalization_rejects_traversal() {
        assert!(normalize_target_path("../escape").is_err());
        assert!(normalize_target_path("a/../b").is_err());
        assert!(normalize_target_path("  ").is_err());
        assert_eq!(
            normalize_target_path("\\docs\\local\\").expect("normalize"),
            "docs/local"
        );
    }

    fn seed_live_link(working_dir: &str, source_root: &Path, target_path: &str) {
        let target_abs = knowledge_reference_dir(working_dir, target_path);
        std::fs::create_dir_all(target_abs.parent().expect("target parent"))
            .expect("create reference root");
        create_directory_link(source_root, &target_abs).expect("create link");
        configure_local_directory(
            working_dir,
            target_path,
            &source_root.to_string_lossy(),
            LocalReferenceSourceKind::Folder,
            KnowledgeLocalSourceMode::Live,
            false,
        )
        .expect("configure live directory");
        knowledge_store::update_directory_external_sources(
            working_dir,
            KnowledgeType::Reference,
            target_path,
            vec![directory_external_source(
                &source_root.to_string_lossy(),
                KnowledgeLocalSourceMode::Live,
                false,
                Some(1),
            )],
        )
        .expect("bind live directory");
    }

    #[test]
    fn live_link_serves_current_source_content_without_copying() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "guide v1");
        write_file(source.path(), "nested/topic.md", "topic body");
        write_file(source.path(), "notes.txt", "plain notes");
        write_file(source.path(), "contract.pdf", "binary-ish");

        seed_live_link(&working_dir, source.path(), "local-live");

        let links = list_local_live_links(&working_dir);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target_path, "local-live");

        let mut paths = list_live_document_paths(&working_dir);
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "local-live/guide.md",
                "local-live/nested/topic.md",
                "local-live/notes.txt",
            ]
        );

        let (doc_count, total_files) = count_live_source_documents(source.path());
        assert_eq!(doc_count, 3);
        assert_eq!(total_files, 4, "non-text files still count toward the total");

        let directories = list_live_directories(&working_dir);
        assert!(directories.contains(&"local-live".to_string()));
        assert!(directories.contains(&"local-live/nested".to_string()));

        let doc = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-live/guide.md",
        )
        .expect("load live document");
        assert!(doc.read_only);
        assert_eq!(normalize_body(&doc.body), "guide v1\n");
        assert_eq!(
            doc.external_source.as_ref().and_then(|source| source.local_mode),
            Some(KnowledgeLocalSourceMode::Live)
        );

        // Text documents keep their original extension in the tree; the load
        // path strips the ".md" suffix the normalizer force-appends.
        let txt = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-live/notes.txt",
        )
        .expect("load live txt document");
        assert_eq!(normalize_body(&txt.body), "plain notes\n");
        assert!(knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-live/contract.pdf",
        )
        .is_err());

        // Edits to the source are visible immediately: no copy is involved.
        write_file(source.path(), "guide.md", "guide v2");
        let doc = knowledge_store::load_document_by_path(
            &working_dir,
            KnowledgeType::Reference,
            "local-live/guide.md",
        )
        .expect("reload live document");
        assert_eq!(normalize_body(&doc.body), "guide v2\n");

        let items =
            knowledge_store::list_documents(&working_dir, Some(KnowledgeType::Reference), None)
                .expect("list documents");
        let live_paths: Vec<_> = items
            .iter()
            .filter(|item| item.path.starts_with("local-live/"))
            .map(|item| item.path.clone())
            .collect();
        assert_eq!(live_paths.len(), 3, "live docs listed exactly once: {:?}", live_paths);
    }

    #[test]
    fn live_link_without_text_documents_still_counts_files() {
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "a.pdf", "pdf-a");
        write_file(source.path(), "b.docx", "docx-b");

        let (doc_count, total_files) = count_live_source_documents(source.path());
        assert_eq!(doc_count, 0);
        assert_eq!(total_files, 2);
    }

    #[test]
    fn live_link_sync_refuses_file_copy_semantics() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "guide");

        seed_live_link(&working_dir, source.path(), "local-live");

        let error = sync_local_reference_documents(&working_dir, "local-live")
            .expect_err("live directories must not run file sync");
        assert!(error.contains("实时链接"), "{}", error);
    }

    #[test]
    fn removing_live_target_keeps_source_files_intact() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let source = TempDir::new().expect("source dir");
        write_file(source.path(), "guide.md", "guide");

        seed_live_link(&working_dir, source.path(), "local-live");
        let target_abs = knowledge_reference_dir(&working_dir, "local-live");
        assert!(target_abs.join("guide.md").is_file());

        remove_live_target_artifacts(&target_abs).expect("remove live target");
        assert!(!target_abs.exists());
        assert!(
            source.path().join("guide.md").is_file(),
            "source file must survive link removal"
        );
    }

    #[test]
    fn remove_live_target_refuses_real_content() {
        let workspace = TempDir::new().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let real_dir = knowledge_reference_dir(&working_dir, "real-folder");
        std::fs::create_dir_all(&real_dir).expect("create real dir");
        std::fs::write(real_dir.join("data.md"), "real content").expect("write real doc");

        let error = remove_live_target_artifacts(&real_dir)
            .expect_err("must refuse to clear real content");
        assert!(error.contains("非链接"), "{}", error);
        assert!(real_dir.join("data.md").is_file());
    }

    #[test]
    fn path_within_live_links_matches_target_and_children() {
        let links = vec![LocalLiveLink {
            target_path: "docs/local-live".to_string(),
            source_root: PathBuf::from("C:/src"),
            source_kind: LocalReferenceSourceKind::Folder,
        }];
        assert!(path_within_live_links(&links, "docs/local-live"));
        assert!(path_within_live_links(&links, "docs/local-live/guide.md"));
        assert!(!path_within_live_links(&links, "docs/local-live-other/guide.md"));
        assert!(!path_within_live_links(&links, "docs"));
    }
}
