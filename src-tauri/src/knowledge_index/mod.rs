pub mod chunker;
pub mod db;
pub mod embedding;
pub mod semantic_rank;
pub mod tantivy_index;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

use crate::knowledge_store::{
    self, DirectorySearchAccess, KnowledgeDocument, KnowledgeListItem, KnowledgeSearchHit,
    KnowledgeSearchMatchSection, KnowledgeType,
};
use crate::unity_docs;

pub use embedding::{
    EmbeddingConfig, EmbeddingDownloadError, EmbeddingDownloadNetworkStatus,
    EmbeddingLocalModelCatalog, EmbeddingLocalModelDirectoryInspection, EmbeddingModelPreset,
    EmbeddingRuntimeTestResult, EmbeddingStatus,
};

use self::db::{
    ChunkRecord, ChunkRow, DocIndexState, DocumentCatalogRow, DocumentPersistUpdate,
    EmbeddingBackfillPersistUpdate, EmbeddingRow, KnowledgeDb, ManagedDirectorySnapshotRow,
    ManagedRetrievalSummaryCacheRow,
};
use self::embedding::{EmbeddingActivationProgress, EmbeddingManager};
use self::semantic_rank::{
    passes_semantic_score_threshold, semantic_confidence, should_use_semantic_recall,
};
use self::tantivy_index::{
    KnowledgeTantivyIndex, LexicalBulkWriterGuard, LexicalDocumentRecord, LexicalHit,
};

pub const INDEX_VERSION: i32 = 3;
const TANTIVY_BATCH_DOCS: usize = 128;
const PARALLEL_PREPARE_DOC_THRESHOLD: usize = 16;
const PREPARING_ANALYSIS_BATCH_DOCS: usize = 64;
const LARGE_LEXICAL_REBUILD_DOC_THRESHOLD: usize = 128;
const PREPARING_PROGRESS_EMIT_DOC_INTERVAL: usize = 24;
const UNITY_IMPORT_BULK_MAX_DOCS_PER_COMMIT: usize = 1024;
const UNITY_IMPORT_BULK_MAX_CHUNKS_PER_COMMIT: usize = 8192;
const KNOWLEDGE_RUNTIME_LOCK_RETRY_ATTEMPTS: usize = 20;
const KNOWLEDGE_RUNTIME_LOCK_RETRY_DELAY_MS: u64 = 50;
pub const KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT: &str = "knowledge-lexical-rebuild-status";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGeneralConfig {
    pub enabled: bool,
    pub lexical_search_enabled: bool,
    pub semantic_search_enabled: bool,
}

impl Default for KnowledgeGeneralConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lexical_search_enabled: false,
            semantic_search_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LexicalRebuildStatus {
    pub running: bool,
    pub stage: Option<String>,
    pub detail: Option<String>,
    pub current_file: Option<String>,
    #[serde(default)]
    pub progress: Option<f32>,
    pub processed_docs: Option<usize>,
    pub total_docs: Option<usize>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeFullTextOverview {
    pub enabled: bool,
    pub indexable_item_count: usize,
    pub indexed_item_count: usize,
    pub fresh_item_count: usize,
    pub stale_item_count: usize,
    pub pending_item_count: usize,
    pub chunk_count: usize,
    pub last_build_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSemanticOverview {
    pub enabled: bool,
    pub ready: bool,
    pub backend: String,
    pub model: String,
    pub device_route: String,
    pub device_name: String,
    pub indexed_item_count: usize,
    pub embedded_chunk_count: usize,
    pub pending_item_count: usize,
    pub coverage_ratio: f64,
    pub stage: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgePerformanceOverview {
    pub db_bytes: u64,
    pub lexical_index_bytes: u64,
    pub local_model_bytes: u64,
    pub gpu_memory_bytes: u64,
    pub gpu_dedicated_memory_bytes: u64,
    pub total_bytes: u64,
    pub avg_chunks_per_item: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeOverview {
    pub total_document_count: usize,
    pub full_text: KnowledgeFullTextOverview,
    pub semantic: KnowledgeSemanticOverview,
    pub performance: KnowledgePerformanceOverview,
}

pub struct KnowledgeRuntime {
    pub db: KnowledgeDb,
    pub tantivy: KnowledgeTantivyIndex,
    pub embedding_mgr: EmbeddingManager,
}

impl KnowledgeRuntime {
    pub fn open(library_dir: &Path, model_storage_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(library_dir)
            .map_err(|e| format!("Failed to create knowledge library dir: {}", e))?;
        embedding::migrate_legacy_managed_model_root(library_dir, model_storage_dir)?;
        let db = KnowledgeDb::open_or_recover(&library_dir.join("knowledge_index.db"))?;
        let tantivy = KnowledgeTantivyIndex::open_or_recover(library_dir)?;
        let config = embedding::load_config(library_dir);
        let embedding_mgr = EmbeddingManager::new(config, model_storage_dir);
        Ok(Self {
            db,
            tantivy,
            embedding_mgr,
        })
    }
}

pub struct KnowledgeIndexState {
    db: RwLock<Arc<KnowledgeDb>>,
    tantivy: RwLock<Arc<KnowledgeTantivyIndex>>,
    embedding_mgr: RwLock<Arc<tokio::sync::Mutex<EmbeddingManager>>>,
    embedding_status: RwLock<Arc<StdMutex<EmbeddingStatus>>>,
    lexical_rebuild_status: RwLock<Arc<StdMutex<LexicalRebuildStatus>>>,
    app_handle: Option<AppHandle>,
    embedding_download_cancel_requested: Arc<AtomicBool>,
    catalog_bootstrapped_workspaces: Arc<tokio::sync::Mutex<HashSet<String>>>,
}

type PreviousKnowledgeRuntime = (
    Arc<KnowledgeDb>,
    Arc<KnowledgeTantivyIndex>,
    Arc<tokio::sync::Mutex<EmbeddingManager>>,
    Arc<StdMutex<EmbeddingStatus>>,
    Arc<StdMutex<LexicalRebuildStatus>>,
);

impl KnowledgeIndexState {
    pub fn new(
        db_inst: KnowledgeDb,
        tantivy_inst: KnowledgeTantivyIndex,
        embedding_mgr: EmbeddingManager,
    ) -> Self {
        Self::new_with_optional_app_handle(db_inst, tantivy_inst, embedding_mgr, None)
    }

    pub fn new_with_app_handle(
        db_inst: KnowledgeDb,
        tantivy_inst: KnowledgeTantivyIndex,
        embedding_mgr: EmbeddingManager,
        app_handle: AppHandle,
    ) -> Self {
        Self::new_with_optional_app_handle(db_inst, tantivy_inst, embedding_mgr, Some(app_handle))
    }

    fn new_with_optional_app_handle(
        db_inst: KnowledgeDb,
        tantivy_inst: KnowledgeTantivyIndex,
        embedding_mgr: EmbeddingManager,
        app_handle: Option<AppHandle>,
    ) -> Self {
        let embedding_status = embedding_mgr.status();
        Self {
            db: RwLock::new(Arc::new(db_inst)),
            tantivy: RwLock::new(Arc::new(tantivy_inst)),
            embedding_mgr: RwLock::new(Arc::new(tokio::sync::Mutex::new(embedding_mgr))),
            embedding_status: RwLock::new(Arc::new(StdMutex::new(embedding_status))),
            lexical_rebuild_status: RwLock::new(Arc::new(StdMutex::new(
                LexicalRebuildStatus::default(),
            ))),
            app_handle,
            embedding_download_cancel_requested: Arc::new(AtomicBool::new(false)),
            catalog_bootstrapped_workspaces: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        }
    }

    pub fn db(&self) -> Arc<KnowledgeDb> {
        self.db.read().unwrap().clone()
    }

    pub fn tantivy(&self) -> Arc<KnowledgeTantivyIndex> {
        self.tantivy.read().unwrap().clone()
    }

    pub fn embedding_mgr(&self) -> Arc<tokio::sync::Mutex<EmbeddingManager>> {
        self.embedding_mgr.read().unwrap().clone()
    }

    pub fn embedding_status_snapshot(&self) -> EmbeddingStatus {
        self.embedding_status
            .read()
            .unwrap()
            .lock()
            .unwrap()
            .clone()
    }

    pub fn set_embedding_status(&self, status: EmbeddingStatus) {
        *self.embedding_status.read().unwrap().lock().unwrap() = status;
    }

    pub fn lexical_rebuild_status_snapshot(&self) -> LexicalRebuildStatus {
        self.lexical_rebuild_status
            .read()
            .unwrap()
            .lock()
            .unwrap()
            .clone()
    }

    pub fn set_lexical_rebuild_status(&self, status: LexicalRebuildStatus) {
        *self.lexical_rebuild_status.read().unwrap().lock().unwrap() = status.clone();
        if let Some(app_handle) = &self.app_handle {
            if let Err(error) = app_handle.emit(KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT, status) {
                eprintln!(
                    "[KnowledgeIndex] failed to emit {}: {}",
                    KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT, error
                );
            }
        }
    }

    pub fn reset_embedding_download_cancel(&self) {
        self.embedding_download_cancel_requested
            .store(false, Ordering::Relaxed);
    }

    pub fn request_embedding_download_cancel(&self) {
        self.embedding_download_cancel_requested
            .store(true, Ordering::Relaxed);
    }

    pub fn embedding_download_cancel_requested(&self) -> Arc<AtomicBool> {
        self.embedding_download_cancel_requested.clone()
    }

    pub async fn rebuild(
        &self,
        library_dir: &Path,
        model_storage_dir: &Path,
    ) -> Result<(), String> {
        let normalized_target_dir = normalize_library_dir_path(library_dir);
        if self
            .current_library_dir()
            .as_ref()
            .is_some_and(|current_dir| *current_dir == normalized_target_dir)
        {
            let release_library_dir = transient_release_library_dir();
            let release_runtime =
                open_runtime_async(&release_library_dir, model_storage_dir).await?;
            let previous_runtime = self.install_runtime(release_runtime).await;
            drop(previous_runtime);
            let runtime =
                open_runtime_with_lock_retry_async(library_dir, model_storage_dir).await?;
            let release_runtime = self.install_runtime(runtime).await;
            drop(release_runtime);
            let cleanup_dir = release_library_dir;
            let _ =
                tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&cleanup_dir)).await;
            return Ok(());
        }

        let runtime = open_runtime_with_lock_retry_async(library_dir, model_storage_dir).await?;
        let previous_runtime = self.install_runtime(runtime).await;
        drop(previous_runtime);
        Ok(())
    }

    fn current_library_dir(&self) -> Option<PathBuf> {
        self.tantivy
            .read()
            .unwrap()
            .index_dir()
            .parent()
            .map(normalize_library_dir_path)
    }

    async fn install_runtime(&self, runtime: KnowledgeRuntime) -> PreviousKnowledgeRuntime {
        let embedding_status = runtime.embedding_mgr.status();
        let new_db = Arc::new(runtime.db);
        let new_tantivy = Arc::new(runtime.tantivy);
        let new_embedding_mgr = Arc::new(tokio::sync::Mutex::new(runtime.embedding_mgr));
        let new_embedding_status = Arc::new(StdMutex::new(embedding_status));
        let new_lexical_rebuild_status = Arc::new(StdMutex::new(LexicalRebuildStatus::default()));

        let db = {
            let mut guard = self.db.write().unwrap();
            std::mem::replace(&mut *guard, new_db)
        };
        let tantivy = {
            let mut guard = self.tantivy.write().unwrap();
            std::mem::replace(&mut *guard, new_tantivy)
        };
        let embedding_mgr = {
            let mut guard = self.embedding_mgr.write().unwrap();
            std::mem::replace(&mut *guard, new_embedding_mgr)
        };
        let embedding_status = {
            let mut guard = self.embedding_status.write().unwrap();
            std::mem::replace(&mut *guard, new_embedding_status)
        };
        let lexical_rebuild_status = {
            let mut guard = self.lexical_rebuild_status.write().unwrap();
            std::mem::replace(&mut *guard, new_lexical_rebuild_status)
        };

        self.reset_embedding_download_cancel();
        self.catalog_bootstrapped_workspaces.lock().await.clear();

        (
            db,
            tantivy,
            embedding_mgr,
            embedding_status,
            lexical_rebuild_status,
        )
    }
}

fn normalize_library_dir_path(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn transient_release_library_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    std::env::temp_dir()
        .join("locus")
        .join("knowledge-index-detached")
        .join(format!("{}-{}", std::process::id(), stamp))
}

fn open_runtime_with_lock_retry(
    library_dir: &Path,
    model_storage_dir: &Path,
) -> Result<KnowledgeRuntime, String> {
    let mut last_err = None;
    for attempt in 0..KNOWLEDGE_RUNTIME_LOCK_RETRY_ATTEMPTS {
        match KnowledgeRuntime::open(library_dir, model_storage_dir) {
            Ok(runtime) => return Ok(runtime),
            Err(err)
                if is_retryable_tantivy_lock_error(&err)
                    && attempt + 1 < KNOWLEDGE_RUNTIME_LOCK_RETRY_ATTEMPTS =>
            {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(KNOWLEDGE_RUNTIME_LOCK_RETRY_DELAY_MS));
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        format!(
            "Failed to reopen knowledge runtime after {} attempts",
            KNOWLEDGE_RUNTIME_LOCK_RETRY_ATTEMPTS
        )
    }))
}

async fn open_runtime_async(
    library_dir: &Path,
    model_storage_dir: &Path,
) -> Result<KnowledgeRuntime, String> {
    let library_dir = library_dir.to_path_buf();
    let model_storage_dir = model_storage_dir.to_path_buf();
    tokio::task::spawn_blocking(move || KnowledgeRuntime::open(&library_dir, &model_storage_dir))
        .await
        .map_err(|e| format!("Knowledge runtime open task join error: {}", e))?
}

async fn open_runtime_with_lock_retry_async(
    library_dir: &Path,
    model_storage_dir: &Path,
) -> Result<KnowledgeRuntime, String> {
    let library_dir = library_dir.to_path_buf();
    let model_storage_dir = model_storage_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        open_runtime_with_lock_retry(&library_dir, &model_storage_dir)
    })
    .await
    .map_err(|e| format!("Knowledge runtime reopen task join error: {}", e))?
}

fn is_retryable_tantivy_lock_error(error: &str) -> bool {
    error.contains("Failed to acquire Lockfile")
        || error.contains("LockBusy")
        || error.contains("index.lock")
        || error.contains("(os error 32)")
        || error.contains("used by another process")
        || error.contains("另一个程序正在使用此文件")
}

#[derive(Debug, Default)]
pub struct ReconcileReport {
    pub added: usize,
    pub removed: usize,
    pub stale: usize,
    pub rebuilt: usize,
    pub embedding_attempted_docs: usize,
    pub embedding_failed_docs: usize,
    pub last_embedding_failed_file: Option<String>,
    pub last_embedding_failure: Option<String>,
}

#[derive(Debug, Clone)]
struct ManagedDirectorySnapshotPersistence {
    managed_path: String,
    snapshot: Option<ManagedDirectorySnapshotRow>,
}

#[derive(Debug, Default)]
struct ManagedDirectoryReuseDecision {
    excluded_prefixes: Vec<(KnowledgeType, String)>,
    retained_doc_ids: HashSet<String>,
    snapshot_persistence: Vec<ManagedDirectorySnapshotPersistence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingActivationBackfillStrategy {
    None,
    VectorOnly,
    FullReconcile,
}

fn should_surface_lexical_rebuild(force_rebuild: bool, total_docs: usize) -> bool {
    force_rebuild || total_docs >= LARGE_LEXICAL_REBUILD_DOC_THRESHOLD
}

fn should_emit_prepare_progress(processed_docs: usize, total_docs: usize) -> bool {
    processed_docs == total_docs || processed_docs % PREPARING_PROGRESS_EMIT_DOC_INTERVAL == 0
}

fn lexical_rebuild_detail(stage: &str, current_file: Option<&str>) -> String {
    match stage {
        "preparing" => current_file
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("Scanning {}", value))
            .unwrap_or_else(|| "Preparing lexical index rebuild".to_string()),
        "cleaning" => "Removing stale lexical documents".to_string(),
        "indexing" => current_file
            .filter(|value| !value.trim().is_empty())
            .map(|value| format!("Indexing {}", value))
            .unwrap_or_else(|| "Building lexical index".to_string()),
        "committing" => "Committing Tantivy index changes".to_string(),
        "completed" => "Lexical index rebuild completed".to_string(),
        "error" => "Lexical index rebuild failed".to_string(),
        _ => "Lexical index rebuild".to_string(),
    }
}

fn lexical_progress_ratio(processed_docs: usize, total_docs: usize) -> f32 {
    if total_docs == 0 {
        return 1.0;
    }
    (processed_docs as f32 / total_docs as f32).clamp(0.0, 1.0)
}

fn lexical_stage_progress(stage: &str, processed_docs: usize, total_docs: usize) -> f32 {
    let ratio = lexical_progress_ratio(processed_docs, total_docs);
    match stage {
        "preparing" => {
            if total_docs == 0 {
                0.12
            } else {
                (0.12 + 0.18 * ratio).clamp(0.0, 1.0)
            }
        }
        "cleaning" => {
            if total_docs == 0 {
                0.36
            } else {
                (0.30 + 0.14 * ratio).clamp(0.0, 1.0)
            }
        }
        "indexing" => {
            if total_docs == 0 {
                0.52
            } else {
                (0.44 + 0.42 * ratio).clamp(0.0, 1.0)
            }
        }
        "committing" => {
            if total_docs == 0 {
                0.90
            } else {
                (0.86 + 0.14 * ratio).clamp(0.0, 1.0)
            }
        }
        "completed" => 1.0,
        _ => ratio,
    }
}

fn embedding_progress_ratio(processed_docs: usize, total_docs: usize) -> f32 {
    if total_docs == 0 {
        return 1.0;
    }
    (processed_docs as f32 / total_docs as f32).clamp(0.0, 1.0)
}

fn embedding_stage_progress(stage: &str, processed_docs: usize, total_docs: usize) -> f32 {
    let ratio = embedding_progress_ratio(processed_docs, total_docs);
    match stage {
        "preparing" => {
            if total_docs == 0 {
                0.08
            } else {
                (0.08 + 0.14 * ratio).clamp(0.0, 1.0)
            }
        }
        "cleaning" => {
            if total_docs == 0 {
                0.26
            } else {
                (0.22 + 0.10 * ratio).clamp(0.0, 1.0)
            }
        }
        "indexing" => {
            if total_docs == 0 {
                0.54
            } else {
                (0.32 + 0.58 * ratio).clamp(0.0, 1.0)
            }
        }
        "committing" => {
            if total_docs == 0 {
                0.96
            } else {
                (0.90 + 0.08 * ratio).clamp(0.0, 1.0)
            }
        }
        "completed" | "ready" => 1.0,
        _ => ratio,
    }
}

fn embedding_rebuild_detail(
    stage: &str,
    processed_docs: usize,
    total_docs: usize,
    current_file: Option<&str>,
) -> String {
    match stage {
        "preparing" => match current_file.filter(|value| !value.trim().is_empty()) {
            Some(file_name) => format!(
                "Preparing vector index · {} / {} · {}",
                processed_docs, total_docs, file_name
            ),
            None => format!(
                "Preparing vector index · {} / {}",
                processed_docs, total_docs
            ),
        },
        "cleaning" => format!(
            "Cleaning vector index · {} / {}",
            processed_docs, total_docs
        ),
        "indexing" => match current_file.filter(|value| !value.trim().is_empty()) {
            Some(file_name) => format!(
                "Building vector index · {} / {} · {}",
                processed_docs, total_docs, file_name
            ),
            None => format!(
                "Building vector index · {} / {}",
                processed_docs, total_docs
            ),
        },
        "committing" => {
            format!(
                "Persisting vector batches · {} / {}",
                processed_docs, total_docs
            )
        }
        "completed" | "ready" => "Vector index rebuild completed".to_string(),
        "error" => "Vector index rebuild failed".to_string(),
        _ => format!("Vector index rebuild · {} / {}", processed_docs, total_docs),
    }
}

fn embedding_failure_summary(report: &ReconcileReport) -> Option<String> {
    embedding_failure_summary_from_counts(
        report.embedding_attempted_docs,
        report.embedding_failed_docs,
    )
}

fn embedding_failure_summary_from_counts(
    attempted_docs: usize,
    failed_docs: usize,
) -> Option<String> {
    if failed_docs == 0 || attempted_docs == 0 {
        return None;
    }

    Some(if failed_docs == attempted_docs {
        format!("Vector indexing failed for all {} documents", failed_docs)
    } else {
        format!(
            "Vector indexing completed with failures: {} / {} documents failed",
            failed_docs, attempted_docs
        )
    })
}

fn set_lexical_rebuild_progress_status(
    state: &KnowledgeIndexState,
    started_at: &str,
    stage: &str,
    processed_docs: usize,
    total_docs: Option<usize>,
    current_file: Option<&str>,
) {
    state.set_lexical_rebuild_status(LexicalRebuildStatus {
        running: true,
        stage: Some(stage.to_string()),
        detail: Some(lexical_rebuild_detail(stage, current_file)),
        current_file: current_file.map(|value| value.to_string()),
        progress: total_docs.map(|total| lexical_stage_progress(stage, processed_docs, total)),
        processed_docs: Some(processed_docs),
        total_docs,
        error: None,
        started_at: Some(started_at.to_string()),
        completed_at: None,
    });
}

fn set_lexical_rebuild_final_status(
    state: &KnowledgeIndexState,
    started_at: &str,
    stage: &str,
    detail: String,
    processed_docs: Option<usize>,
    total_docs: Option<usize>,
    error: Option<String>,
) {
    state.set_lexical_rebuild_status(LexicalRebuildStatus {
        running: false,
        stage: Some(stage.to_string()),
        detail: Some(detail),
        current_file: None,
        progress: match (stage, processed_docs, total_docs) {
            ("completed", _, _) => Some(1.0),
            (_, Some(processed), Some(total)) => {
                Some(lexical_stage_progress(stage, processed, total))
            }
            _ => None,
        },
        processed_docs,
        total_docs,
        error,
        started_at: Some(started_at.to_string()),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
    });
}

fn set_embedding_rebuild_progress_status(
    state: &KnowledgeIndexState,
    stage: &str,
    processed_docs: usize,
    total_docs: usize,
    current_file: Option<&str>,
    detail_override: Option<String>,
) {
    let mut status = state.embedding_status_snapshot();
    status.activating = true;
    status.error = None;
    status.index_progress =
        Some(embedding_stage_progress(stage, processed_docs, total_docs) as f64);
    status.processed_docs = Some(processed_docs);
    status.total_docs = Some(total_docs);
    status.stage = Some(stage.to_string());
    status.current_file = if matches!(stage, "preparing" | "indexing") {
        current_file
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.to_string())
    } else {
        None
    };
    status.detail = Some(detail_override.unwrap_or_else(|| {
        embedding_rebuild_detail(stage, processed_docs, total_docs, current_file)
    }));
    state.set_embedding_status(status);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RebuildReason {
    Added,
    Forced,
    LexicalStale,
    VectorStale,
    EmbeddingBackfill,
}

#[derive(Debug, Clone)]
struct SemanticHit {
    doc_id: String,
    section: String,
    score: f32,
    snippet: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeywordSearchKind {
    LexicalIndex,
    TextScan,
}

#[derive(Debug, Clone)]
struct FusionEntry {
    doc_id: String,
    lexical_rank: Option<usize>,
    lexical_kind: Option<KeywordSearchKind>,
    semantic_rank: Option<usize>,
    semantic_score: Option<f32>,
    section: String,
    snippet: String,
}

#[derive(Debug, Clone)]
struct CachedDocumentEntry {
    item: KnowledgeListItem,
    estimated_tokens: u64,
}

#[derive(Debug, Clone, Default)]
pub struct CachedKnowledgeListPage {
    pub items: Vec<KnowledgeListItem>,
    pub next_offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DirectoryAccessCacheKey {
    doc_type: KnowledgeType,
    path: String,
}

#[derive(Debug, Clone)]
struct PreparedIndexUpdate {
    state: DocIndexState,
    lexical: Option<LexicalDocumentRecord>,
    lexical_remove_doc_id: Option<String>,
    chunks: Vec<ChunkRecord>,
    embeddings: Option<Vec<Vec<f32>>>,
    embedding_attempted: bool,
    embedding_failure: Option<EmbeddingDocumentFailure>,
}

#[derive(Debug, Clone)]
struct EmbeddingDocumentFailure {
    document_path: String,
    message: String,
}

#[derive(Debug, Clone)]
struct PreparedEmbeddingBackfillUpdate {
    state: DocIndexState,
    chunks: Vec<ChunkRow>,
    embeddings: Option<Vec<Vec<f32>>>,
    embedding_attempted: bool,
    embedding_failure: Option<EmbeddingDocumentFailure>,
}

#[derive(Debug, Default, Clone, Copy)]
struct BatchCommitTimings {
    db_elapsed_ms: u64,
    lexical_elapsed_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct PreparedBatchCommitReport {
    prepared_count: usize,
    chunk_count: usize,
    lexical_doc_count: usize,
    embedding_attempted_docs: usize,
    embedding_failed_docs: usize,
    embedding_vector_count: usize,
    prepare_elapsed_ms: u64,
    db_elapsed_ms: u64,
    lexical_elapsed_ms: Option<u64>,
    total_elapsed_ms: u64,
    last_embedding_failure: Option<EmbeddingDocumentFailure>,
}

#[derive(Debug)]
struct PreparedDocumentAnalysis {
    catalog_row: DocumentCatalogRow,
    desired_state: DocIndexState,
    rebuild_reason: Option<RebuildReason>,
    lexical_sync: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RebuildPlan {
    reason: Option<RebuildReason>,
    lexical_sync: bool,
}

#[derive(Debug, Clone)]
struct PendingEmbeddingBackfillDocument {
    state: DocIndexState,
    chunks: Vec<ChunkRow>,
}

#[derive(Debug, Default)]
struct EmbeddingBackfillSelection {
    pending: Vec<PendingEmbeddingBackfillDocument>,
    skipped_docs_without_chunks: usize,
}

#[derive(Debug, Default)]
struct EmbeddingVectorBackfillReport {
    rebuilt: usize,
    skipped_docs_without_chunks: usize,
    embedding_attempted_docs: usize,
    embedding_failed_docs: usize,
    last_embedding_failed_file: Option<String>,
    last_embedding_failure: Option<String>,
}

pub fn library_dir_for_working_dir(working_dir: &str) -> PathBuf {
    Path::new(working_dir).join("Library").join("Locus")
}

pub fn no_workspace_library_dir() -> PathBuf {
    std::env::temp_dir()
        .join("locus")
        .join("knowledge-index-no-workspace")
}

pub fn general_config_path(library_dir: &Path) -> PathBuf {
    library_dir.join("knowledge_config.json")
}

pub fn load_general_config(library_dir: &Path) -> KnowledgeGeneralConfig {
    let path = general_config_path(library_dir);
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => KnowledgeGeneralConfig::default(),
    }
}

pub fn save_general_config(
    library_dir: &Path,
    config: &KnowledgeGeneralConfig,
) -> Result<(), String> {
    std::fs::create_dir_all(library_dir)
        .map_err(|e| format!("Failed to create knowledge config dir: {}", e))?;
    let path = general_config_path(library_dir);
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize knowledge config: {}", e))?;
    std::fs::write(&path, data).map_err(|e| format!("Failed to write knowledge config: {}", e))
}

fn apply_general_search_config(
    access: DirectorySearchAccess,
    config: &KnowledgeGeneralConfig,
) -> DirectorySearchAccess {
    if !config.enabled {
        return DirectorySearchAccess {
            lexical_enabled: false,
            vector_enabled: false,
        };
    }
    DirectorySearchAccess {
        lexical_enabled: access.lexical_enabled && config.lexical_search_enabled,
        vector_enabled: access.vector_enabled && config.semantic_search_enabled,
    }
}

pub async fn maybe_auto_activate_embedding_runtime(
    state: Arc<KnowledgeIndexState>,
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Result<(), String> {
    let mgr_handle = state.embedding_mgr();
    let should_activate = {
        let guard = mgr_handle.lock().await;
        guard.config().enabled && !guard.is_ready()
    };
    if !should_activate {
        let status = mgr_handle.lock().await.status();
        state.set_embedding_status(status);
        return Ok(());
    }
    activate_embedding_runtime(
        state,
        working_dir,
        app_knowledge_dir,
        EmbeddingActivationBackfillStrategy::None,
    )
    .await
}

pub async fn activate_embedding_runtime(
    state: Arc<KnowledgeIndexState>,
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    backfill_strategy: EmbeddingActivationBackfillStrategy,
) -> Result<(), String> {
    let mgr_handle = state.embedding_mgr();
    let should_activate_runtime = {
        let mgr = mgr_handle.lock().await;
        if !mgr.config().enabled {
            return Err("Embedding is not enabled in config".to_string());
        }
        !mgr.is_ready()
    };

    if should_activate_runtime {
        {
            let mgr = mgr_handle.lock().await;
            let mut status = mgr.status();
            status.activating = true;
            status.stage = Some("preparing".to_string());
            status.detail = Some("Preparing embedding runtime".to_string());
            status.error = None;
            state.set_embedding_status(status);
        }

        let activation_state = state.clone();
        let activation_mgr_handle = mgr_handle.clone();
        let activation_join = tokio::task::spawn_blocking(move || {
            let started_at = Instant::now();
            let mut mgr = activation_mgr_handle.blocking_lock();
            let mut status = mgr.status();
            status.activating = true;
            status.stage = Some("preparing".to_string());
            status.detail = Some("Preparing embedding runtime".to_string());
            activation_state.set_embedding_status(status.clone());

            let result = mgr.activate_with_progress(&mut |progress| {
                match progress {
                    EmbeddingActivationProgress::Stage { stage, detail } => {
                        status.stage = Some(stage.to_string());
                        status.detail = detail;
                    }
                    EmbeddingActivationProgress::Download {
                        file_name,
                        downloaded_bytes,
                        total_bytes,
                        progress,
                    } => {
                        status.stage = Some("downloading_model".to_string());
                        status.current_file = Some(file_name);
                        status.downloaded_bytes = Some(downloaded_bytes);
                        status.total_bytes = Some(total_bytes);
                        status.model_download_progress = Some(progress);
                    }
                }
                status.activating = true;
                activation_state.set_embedding_status(status.clone());
            });

            if let Err(err) = result {
                let mut failed = mgr.status();
                failed.activating = false;
                failed.error = Some(err.clone());
                failed.stage = Some("error".to_string());
                activation_state.set_embedding_status(failed);
                tracing::warn!(
                    log_module = "knowledge_index",
                    elapsed_ms = started_at.elapsed().as_millis() as u64,
                    error = %err,
                    "embedding runtime activation failed"
                );
                return Err(err);
            }
            tracing::info!(
                log_module = "knowledge_index",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "embedding runtime activation completed"
            );
            Ok(())
        })
        .await;

        let activation_result = match activation_join {
            Ok(result) => result,
            Err(err) => {
                let message = format!("Embedding runtime activation task failed: {err}");
                let mut failed = state.embedding_status_snapshot();
                failed.activating = false;
                failed.error = Some(message.clone());
                failed.stage = Some("error".to_string());
                failed.detail = Some(message.clone());
                state.set_embedding_status(failed);
                return Err(message);
            }
        };

        if let Err(err) = activation_result {
            return Err(err);
        }
    }

    if matches!(
        backfill_strategy,
        EmbeddingActivationBackfillStrategy::FullReconcile
    ) {
        let mut last_logged_stage: Option<String> = None;
        let reconcile_result = reconcile_workspace_internal(
            working_dir,
            app_knowledge_dir,
            state.clone(),
            true,
            false,
            false,
            |stage, processed, total, current_file| {
                if last_logged_stage.as_deref() != Some(stage) {
                    tracing::info!(
                        log_module = "knowledge_index",
                        workspace = working_dir,
                        stage,
                        processed_docs = processed,
                        total_docs = total,
                        current_file = current_file.unwrap_or(""),
                        "embedding rebuild stage"
                    );
                    last_logged_stage = Some(stage.to_string());
                }
                set_embedding_rebuild_progress_status(
                    state.as_ref(),
                    stage,
                    processed,
                    total,
                    current_file,
                    None,
                );
            },
        )
        .await;

        let report = match reconcile_result {
            Ok(report) => report,
            Err(err) => {
                tracing::error!(
                    log_module = "knowledge_index",
                    workspace = working_dir,
                    error = %err,
                    "embedding runtime activation reconcile failed"
                );
                let mut failed = state.embedding_status_snapshot();
                failed.activating = false;
                failed.stage = Some("error".to_string());
                failed.error = Some(err.clone());
                failed.detail = Some(err.clone());
                state.set_embedding_status(failed);
                return Err(err);
            }
        };

        let mut next = state.embedding_status_snapshot();
        let mgr_status = state.embedding_mgr().lock().await.status();
        next.enabled = mgr_status.enabled;
        next.ready = mgr_status.ready;
        next.activating = false;
        next.model_downloaded = mgr_status.model_downloaded;
        next.model_download_progress = mgr_status.model_download_progress;
        next.downloaded_bytes = mgr_status.downloaded_bytes;
        next.total_bytes = mgr_status.total_bytes;
        next.download_network = mgr_status.download_network;
        next.last_test_summary = mgr_status.last_test_summary;
        next.last_test_passed = mgr_status.last_test_passed;
        next.index_progress = Some(1.0);
        next.failed_docs =
            (report.embedding_failed_docs > 0).then_some(report.embedding_failed_docs);
        next.last_failed_file = report.last_embedding_failed_file.clone();
        next.last_failure = report.last_embedding_failure.clone();
        next.current_file = None;

        if let Some(summary) = embedding_failure_summary(&report) {
            next.processed_docs = next.total_docs.or(next.processed_docs);
            if report.embedding_failed_docs == report.embedding_attempted_docs {
                tracing::error!(
                    log_module = "knowledge_index",
                    workspace = working_dir,
                    attempted_docs = report.embedding_attempted_docs,
                    failed_docs = report.embedding_failed_docs,
                    last_failed_file = report.last_embedding_failed_file.as_deref().unwrap_or(""),
                    "embedding rebuild failed for all attempted documents"
                );
                next.stage = Some("error".to_string());
                next.error = Some(summary.clone());
                next.detail = Some(summary.clone());
                state.set_embedding_status(next);
                return Err(summary);
            }
            tracing::warn!(
                log_module = "knowledge_index",
                workspace = working_dir,
                attempted_docs = report.embedding_attempted_docs,
                failed_docs = report.embedding_failed_docs,
                last_failed_file = report.last_embedding_failed_file.as_deref().unwrap_or(""),
                "embedding rebuild completed with partial failures"
            );
            next.stage = Some("ready".to_string());
            next.error = None;
            next.detail = Some(summary);
            state.set_embedding_status(next);
            return Ok(());
        }

        next.error = None;
        next.detail = None;
        next.stage = Some("ready".to_string());
        tracing::info!(
            log_module = "knowledge_index",
            workspace = working_dir,
            attempted_docs = report.embedding_attempted_docs,
            failed_docs = report.embedding_failed_docs,
            total_docs = next.total_docs.unwrap_or_default(),
            "embedding rebuild completed"
        );
        state.set_embedding_status(next);
        return Ok(());
    }

    if matches!(
        backfill_strategy,
        EmbeddingActivationBackfillStrategy::VectorOnly
    ) {
        let report =
            vector_backfill_embeddings_internal(working_dir, app_knowledge_dir, state.clone())
                .await?;
        let mut next = state.embedding_status_snapshot();
        let mgr_status = state.embedding_mgr().lock().await.status();
        next.enabled = mgr_status.enabled;
        next.ready = mgr_status.ready;
        next.activating = false;
        next.model_downloaded = mgr_status.model_downloaded;
        next.model_download_progress = mgr_status.model_download_progress;
        next.downloaded_bytes = mgr_status.downloaded_bytes;
        next.total_bytes = mgr_status.total_bytes;
        next.download_network = mgr_status.download_network;
        next.last_test_summary = mgr_status.last_test_summary;
        next.last_test_passed = mgr_status.last_test_passed;
        next.index_progress = Some(1.0);
        next.failed_docs =
            (report.embedding_failed_docs > 0).then_some(report.embedding_failed_docs);
        next.last_failed_file = report.last_embedding_failed_file.clone();
        next.last_failure = report.last_embedding_failure.clone();
        next.processed_docs = Some(report.rebuilt);
        next.total_docs = Some(report.rebuilt);
        next.current_file = None;

        if let Some(summary) = embedding_failure_summary_from_counts(
            report.embedding_attempted_docs,
            report.embedding_failed_docs,
        ) {
            if report.embedding_failed_docs == report.embedding_attempted_docs
                && report.embedding_attempted_docs > 0
            {
                next.stage = Some("error".to_string());
                next.error = Some(summary.clone());
                next.detail = Some(summary.clone());
                state.set_embedding_status(next);
                return Err(summary);
            }
            next.stage = Some("ready".to_string());
            next.error = None;
            next.detail = Some(summary);
            state.set_embedding_status(next);
            return Ok(());
        }

        next.error = None;
        next.stage = Some("ready".to_string());
        next.detail = if report.skipped_docs_without_chunks > 0 {
            Some(format!(
                "Vector backfill completed · {} docs updated · {} docs still need a full reconcile",
                report.rebuilt, report.skipped_docs_without_chunks
            ))
        } else {
            None
        };
        state.set_embedding_status(next);
        return Ok(());
    }

    let mut ready = state.embedding_mgr().lock().await.status();
    ready.activating = false;
    ready.stage = Some("ready".to_string());
    ready.index_progress = Some(1.0);
    state.set_embedding_status(ready);
    Ok(())
}

pub async fn deactivate_embedding_runtime(state: Arc<KnowledgeIndexState>) -> Result<(), String> {
    let mgr_handle = state.embedding_mgr();
    let mut mgr = mgr_handle.lock().await;
    mgr.deactivate();
    let status = mgr.status();
    drop(mgr);
    state.set_embedding_status(status);
    Ok(())
}

pub async fn download_local_embedding_model(
    state: Arc<KnowledgeIndexState>,
    model_storage_dir: &Path,
    model_id: &str,
) -> Result<(), EmbeddingDownloadError> {
    let download_source = {
        let mgr_handle = state.embedding_mgr();
        let mgr = mgr_handle.lock().await;
        mgr.config().local_model_download_source.clone()
    };
    let download_network = embedding::prepare_local_model_download_network(&download_source)
        .map_err(EmbeddingDownloadError::Failed)?;
    eprintln!("[Knowledge] 正在连接 {}", model_id);
    state.reset_embedding_download_cancel();
    let mut status = state.embedding_status_snapshot();
    status.activating = true;
    status.stage = Some("preparing".to_string());
    status.detail = Some(format!(
        "正在连接 {}",
        match download_source.as_str() {
            "hf-mirror" => "HF-Mirror",
            _ => "Hugging Face",
        }
    ));
    status.error = None;
    status.current_file = None;
    status.downloaded_bytes = None;
    status.total_bytes = None;
    status.model_download_progress = Some(0.0);
    status.download_network = Some(download_network);
    state.set_embedding_status(status.clone());

    let cancel_requested = state.embedding_download_cancel_requested();
    let result = embedding::download_local_model_with_progress(
        model_storage_dir,
        model_id,
        &download_source,
        cancel_requested.as_ref(),
        &mut |progress| {
            match progress {
                EmbeddingActivationProgress::Stage { stage, detail } => {
                    status.stage = Some(stage.to_string());
                    status.detail = detail;
                }
                EmbeddingActivationProgress::Download {
                    file_name,
                    downloaded_bytes,
                    total_bytes,
                    progress,
                } => {
                    status.stage = Some("downloading_model".to_string());
                    status.current_file = Some(file_name);
                    status.downloaded_bytes = Some(downloaded_bytes);
                    status.total_bytes = Some(total_bytes);
                    status.model_download_progress = Some(progress);
                }
            }
            status.activating = true;
            state.set_embedding_status(status.clone());
        },
    );
    state.reset_embedding_download_cancel();

    match result {
        Ok(()) => {
            let mut next = state.embedding_status_snapshot();
            next.model_downloaded = true;
            next.activating = false;
            next.stage = Some("ready".to_string());
            next.detail = Some(format!("{} 已下载", model_id));
            next.model_download_progress = Some(1.0);
            if let Some(total_bytes) = next.total_bytes {
                next.downloaded_bytes = Some(total_bytes);
            }
            next.current_file = None;
            next.error = None;
            state.set_embedding_status(next);
            Ok(())
        }
        Err(EmbeddingDownloadError::Cancelled) => {
            let mut cancelled = state.embedding_status_snapshot();
            cancelled.model_downloaded = false;
            cancelled.activating = false;
            cancelled.stage = Some("cancelled".to_string());
            cancelled.detail = Some("已取消下载，正在清理临时文件。".to_string());
            cancelled.current_file = None;
            cancelled.downloaded_bytes = None;
            cancelled.total_bytes = None;
            cancelled.model_download_progress = Some(0.0);
            cancelled.error = None;
            state.set_embedding_status(cancelled);
            Err(EmbeddingDownloadError::Cancelled)
        }
        Err(EmbeddingDownloadError::Failed(err)) => {
            let mut failed = state.embedding_status_snapshot();
            failed.activating = false;
            failed.stage = Some("error".to_string());
            failed.error = Some(err.clone());
            state.set_embedding_status(failed);
            Err(EmbeddingDownloadError::Failed(err))
        }
    }
}

pub async fn rebuild_lexical_index_runtime(
    state: Arc<KnowledgeIndexState>,
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
) -> Result<usize, String> {
    let started_at = chrono::Utc::now().to_rfc3339();
    set_lexical_rebuild_progress_status(state.as_ref(), &started_at, "preparing", 0, None, None);

    let mut last_total_docs: Option<usize> = None;

    let result = reconcile_workspace_internal(
        working_dir,
        app_knowledge_dir,
        state.clone(),
        true,
        true,
        false,
        |stage, processed, total, title| {
            last_total_docs = Some(total);
            set_lexical_rebuild_progress_status(
                state.as_ref(),
                &started_at,
                stage,
                processed,
                Some(total),
                title,
            );
        },
    )
    .await;

    match result {
        Ok(report) => {
            let total_docs = last_total_docs.unwrap_or(report.removed + report.rebuilt);
            set_lexical_rebuild_final_status(
                state.as_ref(),
                &started_at,
                "completed",
                lexical_rebuild_detail("completed", None),
                Some(total_docs),
                Some(total_docs),
                None,
            );
            Ok(report.rebuilt)
        }
        Err(err) => {
            set_lexical_rebuild_final_status(
                state.as_ref(),
                &started_at,
                "error",
                err.clone(),
                None,
                last_total_docs,
                Some(err.clone()),
            );
            Err(err)
        }
    }
}

pub async fn reconcile_workspace(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
) -> Result<ReconcileReport, String> {
    let timer_started_at = Instant::now();
    eprintln!(
        "[KnowledgeIndex] reconcile start workspace={} app_root={}",
        working_dir,
        app_knowledge_dir
            .map(|value| value.display().to_string())
            .unwrap_or_else(|| "<none>".to_string())
    );
    let status_started_at = chrono::Utc::now().to_rfc3339();
    let mut progress_started = false;
    let mut last_total_docs: Option<usize> = None;

    let result = reconcile_workspace_internal(
        working_dir,
        app_knowledge_dir,
        state.clone(),
        false,
        false,
        false,
        |stage, processed, total, title| {
            progress_started = true;
            last_total_docs = Some(total);
            set_lexical_rebuild_progress_status(
                state.as_ref(),
                &status_started_at,
                stage,
                processed,
                Some(total),
                title,
            );
        },
    )
    .await;

    match result {
        Ok(report) => {
            if progress_started {
                let total_docs = last_total_docs.unwrap_or(report.removed + report.rebuilt);
                set_lexical_rebuild_final_status(
                    state.as_ref(),
                    &status_started_at,
                    "completed",
                    lexical_rebuild_detail("completed", None),
                    Some(total_docs),
                    Some(total_docs),
                    None,
                );
            }
            eprintln!(
                "[KnowledgeIndex] reconcile finished workspace={} elapsed_ms={} added={} removed={} stale={} rebuilt={}",
                working_dir,
                timer_started_at.elapsed().as_millis(),
                report.added,
                report.removed,
                report.stale,
                report.rebuilt
            );
            Ok(report)
        }
        Err(err) => {
            if progress_started {
                set_lexical_rebuild_final_status(
                    state.as_ref(),
                    &status_started_at,
                    "error",
                    err.clone(),
                    None,
                    last_total_docs,
                    Some(err.clone()),
                );
            }
            eprintln!(
                "[KnowledgeIndex] reconcile failed workspace={} elapsed_ms={} error={}",
                working_dir,
                timer_started_at.elapsed().as_millis(),
                err
            );
            Err(err)
        }
    }
}

pub(crate) async fn reconcile_workspace_internal<F>(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
    force_rebuild: bool,
    force_lexical_sync: bool,
    always_surface_progress: bool,
    mut on_rebuild_progress: F,
) -> Result<ReconcileReport, String>
where
    F: FnMut(&str, usize, usize, Option<&str>),
{
    let db = state.db();
    let tantivy = state.tantivy();
    let library_dir = library_dir_for_working_dir(working_dir);
    let mgr_handle = state.embedding_mgr();
    if let Ok(mgr) = mgr_handle.try_lock() {
        return reconcile_documents_sync(
            working_dir,
            app_knowledge_dir,
            &db,
            &tantivy,
            Some(&mgr),
            &mgr.backend_signature_json(),
            mgr.config().enabled && mgr.is_ready(),
            force_rebuild,
            force_lexical_sync,
            always_surface_progress,
            &mut on_rebuild_progress,
        );
    }

    let fallback_mgr = EmbeddingManager::new(embedding::load_config(&library_dir), &library_dir);
    reconcile_documents_sync(
        working_dir,
        app_knowledge_dir,
        &db,
        &tantivy,
        None,
        &fallback_mgr.backend_signature_json(),
        false,
        force_rebuild,
        force_lexical_sync,
        always_surface_progress,
        &mut on_rebuild_progress,
    )
}

async fn vector_backfill_embeddings_internal(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
) -> Result<EmbeddingVectorBackfillReport, String> {
    let db = state.db();
    let mgr_handle = state.embedding_mgr();
    let mgr = mgr_handle.lock().await;
    if !mgr.is_ready() {
        return Err("Embedding runtime is not ready".to_string());
    }

    vector_backfill_embeddings_sync(
        working_dir,
        app_knowledge_dir,
        &db,
        &mgr,
        &mgr.backend_signature_json(),
        &mut |stage, processed, total, current_file, detail| {
            set_embedding_rebuild_progress_status(
                state.as_ref(),
                stage,
                processed,
                total,
                current_file,
                detail,
            );
        },
    )
}

fn vector_backfill_embeddings_sync<F>(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    embedding_mgr: &EmbeddingManager,
    backend_signature: &str,
    on_progress: &mut F,
) -> Result<EmbeddingVectorBackfillReport, String>
where
    F: FnMut(&str, usize, usize, Option<&str>, Option<String>),
{
    let selection =
        collect_vector_backfill_candidates(working_dir, app_knowledge_dir, db, backend_signature)?;
    let total_docs = selection.pending.len();
    let mut report = EmbeddingVectorBackfillReport {
        skipped_docs_without_chunks: selection.skipped_docs_without_chunks,
        ..EmbeddingVectorBackfillReport::default()
    };

    if total_docs == 0 {
        tracing::info!(
            log_module = "knowledge_index",
            workspace = working_dir,
            skipped_docs_without_chunks = report.skipped_docs_without_chunks,
            "vector-only embedding backfill skipped because no documents required updates"
        );
        return Ok(report);
    }

    on_progress(
        "preparing",
        0,
        total_docs,
        None,
        Some(format!("Preparing vector backfill · {} docs", total_docs)),
    );

    let batch_total = (total_docs + TANTIVY_BATCH_DOCS - 1) / TANTIVY_BATCH_DOCS;
    let mut processed_docs = 0usize;

    for (batch_index, batch) in selection.pending.chunks(TANTIVY_BATCH_DOCS).enumerate() {
        let current_file = batch.last().map(|item| item.state.doc_path.as_str());
        on_progress("indexing", processed_docs, total_docs, current_file, None);

        let pending_chunk_count = batch.iter().map(|item| item.chunks.len()).sum::<usize>();
        let pending_docs = batch.len();
        on_progress(
            "committing",
            processed_docs + pending_docs,
            total_docs,
            None,
            Some(format!(
                "Persisting vector batch {} / {} · {} docs · {} chunks",
                batch_index + 1,
                batch_total,
                pending_docs,
                pending_chunk_count
            )),
        );

        let batch_report = prepare_and_commit_embedding_backfill_batch(
            db,
            embedding_mgr,
            batch,
            batch_index + 1,
            batch_total,
        )?;

        processed_docs += batch_report.prepared_count;
        report.rebuilt += batch_report.prepared_count;
        report.embedding_attempted_docs += batch_report.embedding_attempted_docs;
        report.embedding_failed_docs += batch_report.embedding_failed_docs;
        if let Some(failure) = batch_report.last_embedding_failure {
            report.last_embedding_failed_file = Some(failure.document_path);
            report.last_embedding_failure = Some(failure.message);
        }

        on_progress(
            "committing",
            processed_docs,
            total_docs,
            None,
            Some(format!(
                "Persisted vector batch {} / {} · {} docs · {} chunks · SQLite {} ms",
                batch_index + 1,
                batch_total,
                batch_report.prepared_count,
                batch_report.chunk_count,
                batch_report.db_elapsed_ms
            )),
        );
    }

    Ok(report)
}

fn collect_vector_backfill_candidates(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    backend_signature: &str,
) -> Result<EmbeddingBackfillSelection, String> {
    let mut selection = EmbeddingBackfillSelection::default();
    let mut access_cache = HashMap::new();
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));

    for existing_state in db.list_all_index_states()? {
        let doc_type = knowledge_type_from_str(&existing_state.doc_type)?;
        let access = apply_general_search_config(
            cached_document_search_access(
                working_dir,
                app_knowledge_dir,
                doc_type,
                &existing_state.doc_path,
                &mut access_cache,
            )?,
            &general_config,
        );
        if !access.vector_enabled {
            continue;
        }

        let mut desired_state = existing_state.clone();
        desired_state.embedding_backend =
            build_embedding_backend_state_marker(backend_signature, access);
        desired_state.stale = 0;

        let plan = rebuild_plan_for_document(
            false,
            false,
            &existing_state,
            &desired_state,
            needs_embedding_backfill(db, &existing_state.doc_id)?,
        );
        if !matches!(
            plan.reason,
            Some(RebuildReason::VectorStale) | Some(RebuildReason::EmbeddingBackfill)
        ) {
            continue;
        }

        let chunks = db.get_chunks(&existing_state.doc_id)?;
        if chunks.is_empty() {
            selection.skipped_docs_without_chunks += 1;
            tracing::warn!(
                log_module = "knowledge_index",
                workspace = working_dir,
                doc_id = %existing_state.doc_id,
                doc_path = %existing_state.doc_path,
                "vector-only embedding backfill skipped document without stored chunks"
            );
            continue;
        }

        selection.pending.push(PendingEmbeddingBackfillDocument {
            state: desired_state,
            chunks,
        });
    }

    Ok(selection)
}

fn prepare_embedding_backfill_batch(
    embedding_mgr: &EmbeddingManager,
    batch: &[PendingEmbeddingBackfillDocument],
) -> Result<Vec<PreparedEmbeddingBackfillUpdate>, String> {
    batch
        .iter()
        .map(|document| {
            let mut state = document.state.clone();
            state.stale = 0;
            let texts = document
                .chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<Vec<_>>();
            let mut embedding_failure = None;
            let embeddings = match embedding_mgr
                .embed_documents(&texts)
                .ok_or_else(|| "Embedding runtime is unavailable".to_string())?
            {
                Ok(vectors) => Some(vectors),
                Err(err) => {
                    tracing::error!(
                        log_module = "knowledge_index",
                        document_path = %state.doc_path,
                        error = %err,
                        "vector-only embedding backfill failed for document"
                    );
                    embedding_failure = Some(EmbeddingDocumentFailure {
                        document_path: state.doc_path.clone(),
                        message: err,
                    });
                    None
                }
            };

            Ok(PreparedEmbeddingBackfillUpdate {
                state,
                chunks: document.chunks.clone(),
                embeddings,
                embedding_attempted: true,
                embedding_failure,
            })
        })
        .collect()
}

fn commit_embedding_backfill_updates_batch(
    db: &KnowledgeDb,
    prepared: &[PreparedEmbeddingBackfillUpdate],
) -> Result<BatchCommitTimings, String> {
    if prepared.is_empty() {
        return Ok(BatchCommitTimings::default());
    }

    let persist_updates = prepared
        .iter()
        .map(|update| EmbeddingBackfillPersistUpdate {
            state: &update.state,
            chunks: &update.chunks,
            embeddings: update.embeddings.as_deref(),
        })
        .collect::<Vec<_>>();

    let db_started = Instant::now();
    db.apply_embedding_backfill_updates(&persist_updates)?;
    Ok(BatchCommitTimings {
        db_elapsed_ms: db_started.elapsed().as_millis() as u64,
        lexical_elapsed_ms: None,
    })
}

fn prepare_and_commit_embedding_backfill_batch(
    db: &KnowledgeDb,
    embedding_mgr: &EmbeddingManager,
    batch: &[PendingEmbeddingBackfillDocument],
    batch_index: usize,
    batch_total: usize,
) -> Result<PreparedBatchCommitReport, String> {
    let total_started = Instant::now();
    let prepare_started = Instant::now();
    let prepared = prepare_embedding_backfill_batch(embedding_mgr, batch)?;
    let mut report = PreparedBatchCommitReport {
        prepared_count: prepared.len(),
        chunk_count: prepared
            .iter()
            .map(|update| update.chunks.len())
            .sum::<usize>(),
        embedding_attempted_docs: prepared
            .iter()
            .filter(|update| update.embedding_attempted)
            .count(),
        embedding_failed_docs: prepared
            .iter()
            .filter(|update| update.embedding_failure.is_some())
            .count(),
        embedding_vector_count: prepared
            .iter()
            .filter_map(|update| update.embeddings.as_ref())
            .map(|vectors| vectors.len())
            .sum::<usize>(),
        last_embedding_failure: prepared
            .iter()
            .filter_map(|update| update.embedding_failure.clone())
            .last(),
        prepare_elapsed_ms: prepare_started.elapsed().as_millis() as u64,
        ..PreparedBatchCommitReport::default()
    };

    tracing::info!(
        log_module = "knowledge_index",
        batch_index,
        batch_total,
        prepared_docs = report.prepared_count,
        chunks = report.chunk_count,
        embedding_attempted_docs = report.embedding_attempted_docs,
        embedding_failed_docs = report.embedding_failed_docs,
        embedding_vectors = report.embedding_vector_count,
        prepare_elapsed_ms = report.prepare_elapsed_ms,
        "vector-only embedding backfill batch start"
    );

    match commit_embedding_backfill_updates_batch(db, &prepared) {
        Ok(timings) => {
            report.db_elapsed_ms = timings.db_elapsed_ms;
            report.total_elapsed_ms = total_started.elapsed().as_millis() as u64;
            tracing::info!(
                log_module = "knowledge_index",
                batch_index,
                batch_total,
                prepared_docs = report.prepared_count,
                chunks = report.chunk_count,
                embedding_attempted_docs = report.embedding_attempted_docs,
                embedding_failed_docs = report.embedding_failed_docs,
                embedding_vectors = report.embedding_vector_count,
                prepare_elapsed_ms = report.prepare_elapsed_ms,
                db_elapsed_ms = report.db_elapsed_ms,
                total_elapsed_ms = report.total_elapsed_ms,
                "vector-only embedding backfill batch completed"
            );
        }
        Err(err) => {
            report.total_elapsed_ms = total_started.elapsed().as_millis() as u64;
            tracing::error!(
                log_module = "knowledge_index",
                batch_index,
                batch_total,
                prepared_docs = report.prepared_count,
                chunks = report.chunk_count,
                embedding_attempted_docs = report.embedding_attempted_docs,
                embedding_failed_docs = report.embedding_failed_docs,
                embedding_vectors = report.embedding_vector_count,
                prepare_elapsed_ms = report.prepare_elapsed_ms,
                total_elapsed_ms = report.total_elapsed_ms,
                error = %err,
                "vector-only embedding backfill batch failed"
            );
            return Err(err);
        }
    }

    Ok(report)
}

pub async fn reconcile_unity_reference_import<F>(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
    mut on_rebuild_progress: F,
) -> Result<ReconcileReport, String>
where
    F: FnMut(&str, usize, usize, Option<&str>),
{
    let db = state.db();
    let tantivy = state.tantivy();
    let library_dir = library_dir_for_working_dir(working_dir);
    let mgr_handle = state.embedding_mgr();
    if let Ok(mgr) = mgr_handle.try_lock() {
        return reconcile_unity_reference_import_sync(
            working_dir,
            app_knowledge_dir,
            &db,
            &tantivy,
            Some(&mgr),
            &mgr.backend_signature_json(),
            &mut on_rebuild_progress,
        );
    }

    let fallback_mgr = EmbeddingManager::new(embedding::load_config(&library_dir), &library_dir);
    reconcile_unity_reference_import_sync(
        working_dir,
        app_knowledge_dir,
        &db,
        &tantivy,
        None,
        &fallback_mgr.backend_signature_json(),
        &mut on_rebuild_progress,
    )
}

fn reconcile_unity_reference_import_sync<F>(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    tantivy: &KnowledgeTantivyIndex,
    embedding_mgr: Option<&EmbeddingManager>,
    backend_signature: &str,
    on_rebuild_progress: &mut F,
) -> Result<ReconcileReport, String>
where
    F: FnMut(&str, usize, usize, Option<&str>),
{
    let current_snapshot = unity_docs::current_unity_reference_managed_snapshot(working_dir)?
        .ok_or_else(|| {
            "Unity reference managed snapshot is unavailable after import".to_string()
        })?;
    if current_snapshot.document_count != current_snapshot.expected_document_count {
        return Err("Unity reference managed snapshot is incomplete after import".to_string());
    }

    let documents = unity_docs::list_managed_documents(
        working_dir,
        Some(unity_docs::UNITY_REFERENCE_MANAGED_DIR),
    )?;
    if documents.len() != current_snapshot.document_count {
        return Err(format!(
            "Unity reference managed document count mismatch after import: snapshot={} actual={}",
            current_snapshot.document_count,
            documents.len()
        ));
    }
    let new_doc_ids = documents
        .iter()
        .map(|document| document.id.clone())
        .collect::<HashSet<_>>();
    let existing_rows = db.list_document_catalog_entries_with_prefix(
        KnowledgeType::Reference.as_str(),
        unity_docs::UNITY_REFERENCE_MANAGED_DIR,
    )?;
    let existing_doc_ids = existing_rows
        .iter()
        .map(|row| row.doc_id.clone())
        .collect::<HashSet<_>>();
    let removed_doc_ids = existing_rows
        .into_iter()
        .map(|row| row.doc_id)
        .filter(|doc_id| !new_doc_ids.contains(doc_id))
        .collect::<Vec<_>>();

    let total = removed_doc_ids.len() + documents.len();
    let mut report = ReconcileReport::default();
    report.removed = removed_doc_ids.len();
    let mut access_cache = HashMap::new();
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));
    let mut scanned = 0usize;
    let mut indexed_docs = 0usize;
    let mut pending_catalog_rows = Vec::with_capacity(UNITY_IMPORT_BULK_MAX_DOCS_PER_COMMIT);
    let mut pending_prepared = Vec::with_capacity(UNITY_IMPORT_BULK_MAX_DOCS_PER_COMMIT);
    let mut pending_chunk_count = 0usize;
    let mut bulk_writer = tantivy.unity_import_bulk_writer()?;

    on_rebuild_progress("preparing", 0, documents.len(), None);

    if !removed_doc_ids.is_empty() {
        on_rebuild_progress("cleaning", removed_doc_ids.len(), total, None);
        bulk_writer.apply_grouped_batch(&removed_doc_ids, &[], &[])?;
        db.delete_documents(&removed_doc_ids)?;
    }

    for documents_batch in documents.chunks(PREPARING_ANALYSIS_BATCH_DOCS) {
        populate_document_access_cache_for_batch(
            working_dir,
            app_knowledge_dir,
            documents_batch,
            &mut access_cache,
        )?;

        let analyzed_batch = documents_batch
            .iter()
            .map(|document| {
                let access = apply_general_search_config(
                    cached_document_search_access(
                        working_dir,
                        app_knowledge_dir,
                        document.doc_type,
                        &document.path,
                        &mut access_cache,
                    )?,
                    &general_config,
                );
                let state = build_index_state(document, backend_signature, access);
                let catalog_row = build_document_catalog_row(document, access)?;
                Ok((document.clone(), state, access, catalog_row))
            })
            .collect::<Result<Vec<_>, String>>()?;

        scanned += analyzed_batch.len();
        if should_emit_prepare_progress(scanned, documents.len()) {
            on_rebuild_progress(
                "preparing",
                scanned,
                documents.len(),
                analyzed_batch
                    .last()
                    .map(|(document, _, _, _)| document.path.as_str()),
            );
        }

        let pending_rebuild_batch = analyzed_batch
            .iter()
            .map(|(document, state, access, _)| (document.clone(), state.clone(), *access, true))
            .collect::<Vec<_>>();
        let prepared_updates = prepare_rebuild_batch(embedding_mgr, &pending_rebuild_batch)?;

        for ((document, _, _, catalog_row), prepared_update) in
            analyzed_batch.into_iter().zip(prepared_updates.into_iter())
        {
            if existing_doc_ids.contains(&document.id) {
                report.stale += 1;
            } else {
                report.added += 1;
            }
            pending_catalog_rows.push(catalog_row);
            pending_chunk_count += prepared_update
                .lexical
                .as_ref()
                .map(|record| record.chunks.len())
                .unwrap_or(0);
            pending_prepared.push(prepared_update);

            let indexed_progress = report.removed + indexed_docs + pending_prepared.len();
            if should_emit_prepare_progress(
                indexed_progress.saturating_sub(report.removed),
                documents.len(),
            ) {
                on_rebuild_progress("indexing", indexed_progress, total, Some(&document.path));
            }

            if should_flush_unity_import_bulk_batch(pending_prepared.len(), pending_chunk_count) {
                on_rebuild_progress("committing", indexed_progress, total, None);
                indexed_docs += commit_unity_import_bulk_batch(
                    db,
                    &mut bulk_writer,
                    &pending_catalog_rows,
                    &pending_prepared,
                    &existing_doc_ids,
                )?;
                pending_catalog_rows.clear();
                pending_prepared.clear();
                pending_chunk_count = 0;
            }
        }
    }

    if !pending_prepared.is_empty() || !pending_catalog_rows.is_empty() {
        on_rebuild_progress("committing", total, total, None);
        indexed_docs += commit_unity_import_bulk_batch(
            db,
            &mut bulk_writer,
            &pending_catalog_rows,
            &pending_prepared,
            &existing_doc_ids,
        )?;
    }

    report.rebuilt = indexed_docs;

    if let Some(snapshot_row) = managed_snapshot_row_from_snapshot(Some(&current_snapshot)) {
        db.upsert_managed_directory_snapshot(&snapshot_row)?;
    } else {
        db.delete_managed_directory_snapshot(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?;
    }
    refresh_unity_managed_retrieval_summary_cache(working_dir, app_knowledge_dir, db)?;

    Ok(report)
}

fn should_flush_unity_import_bulk_batch(pending_docs: usize, pending_chunks: usize) -> bool {
    pending_docs >= UNITY_IMPORT_BULK_MAX_DOCS_PER_COMMIT
        || pending_chunks >= UNITY_IMPORT_BULK_MAX_CHUNKS_PER_COMMIT
}

fn commit_unity_import_bulk_batch(
    db: &KnowledgeDb,
    bulk_writer: &mut LexicalBulkWriterGuard<'_>,
    catalog_rows: &[DocumentCatalogRow],
    prepared: &[PreparedIndexUpdate],
    existing_doc_ids: &HashSet<String>,
) -> Result<usize, String> {
    if catalog_rows.is_empty() && prepared.is_empty() {
        return Ok(0);
    }

    let lexical_removals = prepared
        .iter()
        .filter_map(|update| update.lexical_remove_doc_id.clone())
        .collect::<Vec<_>>();
    let lexical_replacements = prepared
        .iter()
        .filter_map(|update| {
            update.lexical.as_ref().and_then(|record| {
                existing_doc_ids
                    .contains(&record.doc_id)
                    .then_some(record.doc_id.clone())
            })
        })
        .collect::<Vec<_>>();
    let lexical_docs = prepared
        .iter()
        .filter_map(|update| update.lexical.clone())
        .collect::<Vec<_>>();

    if !lexical_removals.is_empty() || !lexical_replacements.is_empty() || !lexical_docs.is_empty()
    {
        bulk_writer.apply_grouped_batch(&lexical_removals, &lexical_replacements, &lexical_docs)?;
    }

    let persist_updates = prepared
        .iter()
        .map(|update| DocumentPersistUpdate {
            state: &update.state,
            chunks: &update.chunks,
            embeddings: update.embeddings.as_deref(),
        })
        .collect::<Vec<_>>();
    db.apply_document_updates(&persist_updates)?;
    db.upsert_document_catalog_entries(catalog_rows)?;
    Ok(prepared.len())
}

fn reconcile_documents_sync<F>(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    tantivy: &KnowledgeTantivyIndex,
    embedding_mgr: Option<&EmbeddingManager>,
    backend_signature: &str,
    embedding_backfill_ready: bool,
    force_rebuild: bool,
    force_lexical_sync: bool,
    always_surface_progress: bool,
    on_rebuild_progress: &mut F,
) -> Result<ReconcileReport, String>
where
    F: FnMut(&str, usize, usize, Option<&str>),
{
    let progress_tracks_all_work = force_rebuild || always_surface_progress;
    if progress_tracks_all_work {
        on_rebuild_progress("preparing", 0, 0, None);
    }
    let reuse_decision = plan_managed_directory_reuse(
        working_dir,
        app_knowledge_dir,
        db,
        backend_signature,
        embedding_backfill_ready,
        force_rebuild,
    )?;
    eprintln!(
        "[KnowledgeIndex] reconcile decision workspace={} excluded_prefixes={:?} retained_doc_ids={}",
        working_dir,
        reuse_decision.excluded_prefixes,
        reuse_decision.retained_doc_ids.len()
    );
    let documents = load_all_documents(
        working_dir,
        app_knowledge_dir,
        &reuse_decision.excluded_prefixes,
    )?;
    eprintln!(
        "[KnowledgeIndex] reconcile loaded documents workspace={} load_count={}",
        working_dir,
        documents.len()
    );
    let preparation_total = documents.len();
    let mut doc_ids: HashSet<String> = documents.iter().map(|doc| doc.id.clone()).collect();
    doc_ids.extend(reuse_decision.retained_doc_ids.iter().cloned());
    let existing_states = db.list_all_index_states()?;
    let mut surface_progress = progress_tracks_all_work;
    if surface_progress {
        on_rebuild_progress("preparing", 0, preparation_total, None);
    }
    let mut report = ReconcileReport::default();
    let mut rebuild_queue = Vec::new();
    let mut state_map = HashMap::new();
    let mut removed_doc_ids = Vec::new();
    let mut removed_lexical_doc_ids = Vec::new();
    let mut catalog_rows = Vec::with_capacity(documents.len());
    let mut access_cache = HashMap::new();
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));
    let automatic_lexical_progress_enabled =
        general_config.enabled && general_config.lexical_search_enabled;

    for state in existing_states {
        if !doc_ids.contains(&state.doc_id) {
            removed_doc_ids.push(state.doc_id.clone());
            let (_, access) = parse_embedding_backend_state_marker(&state.embedding_backend);
            if access.lexical_enabled {
                removed_lexical_doc_ids.push(state.doc_id.clone());
            }
        } else {
            state_map.insert(state.doc_id.clone(), state);
        }
    }

    let mut scanned = 0usize;
    for documents_batch in documents.chunks(PREPARING_ANALYSIS_BATCH_DOCS) {
        populate_document_access_cache_for_batch(
            working_dir,
            app_knowledge_dir,
            documents_batch,
            &mut access_cache,
        )?;

        let batch =
            batch_document_search_inputs(documents_batch.to_vec(), &access_cache, &general_config);
        let parallelize_prepare_analysis =
            should_parallelize_prepare_analysis(embedding_backfill_ready, &batch);
        let current_file = batch.last().map(|(document, _)| document.path.as_str());
        let analyzed = analyze_prepare_batch(
            db,
            &batch,
            &state_map,
            &backend_signature,
            embedding_backfill_ready,
            force_rebuild,
            force_lexical_sync,
            parallelize_prepare_analysis,
        )?;
        scanned += batch.len();
        if surface_progress && should_emit_prepare_progress(scanned, preparation_total) {
            on_rebuild_progress("preparing", scanned, preparation_total, current_file);
        }

        for ((document, access), analysis) in batch.into_iter().zip(analyzed.into_iter()) {
            catalog_rows.push(analysis.catalog_row);

            match analysis.rebuild_reason {
                Some(RebuildReason::Added) => {
                    report.added += 1;
                    rebuild_queue.push((
                        document,
                        analysis.desired_state,
                        access,
                        analysis.lexical_sync,
                    ));
                }
                Some(RebuildReason::LexicalStale) => {
                    report.stale += 1;
                    rebuild_queue.push((
                        document,
                        analysis.desired_state,
                        access,
                        analysis.lexical_sync,
                    ));
                }
                Some(RebuildReason::VectorStale) => {
                    report.stale += 1;
                    rebuild_queue.push((
                        document,
                        analysis.desired_state,
                        access,
                        analysis.lexical_sync,
                    ));
                }
                Some(RebuildReason::Forced) | Some(RebuildReason::EmbeddingBackfill) => {
                    rebuild_queue.push((
                        document,
                        analysis.desired_state,
                        access,
                        analysis.lexical_sync,
                    ));
                }
                None => {}
            }
        }
    }

    let total = removed_doc_ids.len() + rebuild_queue.len();
    let lexical_total = removed_lexical_doc_ids.len()
        + rebuild_queue
            .iter()
            .filter(|(_, _, _, lexical_sync)| *lexical_sync)
            .count();
    let callback_total = if progress_tracks_all_work {
        total
    } else {
        lexical_total
    };
    if !surface_progress
        && automatic_lexical_progress_enabled
        && should_surface_lexical_rebuild(false, callback_total)
    {
        surface_progress = true;
        on_rebuild_progress("preparing", callback_total, callback_total, None);
    }

    if !removed_lexical_doc_ids.is_empty() {
        tantivy.remove_docs(&removed_lexical_doc_ids)?;
    }
    if !removed_doc_ids.is_empty() {
        db.delete_documents(&removed_doc_ids)?;
        report.removed = removed_doc_ids.len();
        if surface_progress {
            let cleaned = if progress_tracks_all_work {
                report.removed
            } else {
                removed_lexical_doc_ids.len()
            };
            on_rebuild_progress("cleaning", cleaned, callback_total, None);
        }
    }
    db.upsert_document_catalog_entries(&catalog_rows)?;

    let mut pending_batch = Vec::with_capacity(TANTIVY_BATCH_DOCS);
    let mut lexical_processed = removed_lexical_doc_ids.len();
    for (index, (document, state, access, lexical_sync)) in rebuild_queue.into_iter().enumerate() {
        let processed_all = report.removed + index + 1;
        if surface_progress {
            if progress_tracks_all_work {
                on_rebuild_progress(
                    "indexing",
                    processed_all,
                    callback_total,
                    Some(&document.path),
                );
            } else if lexical_sync {
                on_rebuild_progress(
                    "indexing",
                    lexical_processed + 1,
                    callback_total,
                    Some(&document.path),
                );
            }
        }
        pending_batch.push((document, state, access, lexical_sync));
        if lexical_sync {
            lexical_processed += 1;
        }

        if pending_batch.len() >= TANTIVY_BATCH_DOCS {
            let batch_has_lexical_sync = pending_batch
                .iter()
                .any(|(_, _, _, lexical_sync)| *lexical_sync);
            if surface_progress {
                if progress_tracks_all_work {
                    on_rebuild_progress("committing", processed_all, callback_total, None);
                } else if batch_has_lexical_sync {
                    on_rebuild_progress("committing", lexical_processed, callback_total, None);
                }
            }
            let batch_report =
                prepare_and_commit_rebuild_batch(db, tantivy, embedding_mgr, &pending_batch)?;
            report.rebuilt += batch_report.prepared_count;
            report.embedding_attempted_docs += batch_report.embedding_attempted_docs;
            report.embedding_failed_docs += batch_report.embedding_failed_docs;
            if let Some(failure) = batch_report.last_embedding_failure {
                report.last_embedding_failed_file = Some(failure.document_path);
                report.last_embedding_failure = Some(failure.message);
            }
            pending_batch.clear();
        }
    }

    if !pending_batch.is_empty() {
        let batch_has_lexical_sync = pending_batch
            .iter()
            .any(|(_, _, _, lexical_sync)| *lexical_sync);
        if surface_progress {
            if progress_tracks_all_work {
                on_rebuild_progress("committing", total, callback_total, None);
            } else if batch_has_lexical_sync {
                on_rebuild_progress("committing", lexical_processed, callback_total, None);
            }
        }
        let batch_report =
            prepare_and_commit_rebuild_batch(db, tantivy, embedding_mgr, &pending_batch)?;
        report.rebuilt += batch_report.prepared_count;
        report.embedding_attempted_docs += batch_report.embedding_attempted_docs;
        report.embedding_failed_docs += batch_report.embedding_failed_docs;
        if let Some(failure) = batch_report.last_embedding_failure {
            report.last_embedding_failed_file = Some(failure.document_path);
            report.last_embedding_failure = Some(failure.message);
        }
    }

    persist_managed_directory_snapshots(db, &reuse_decision.snapshot_persistence)?;
    refresh_unity_managed_retrieval_summary_cache(working_dir, app_knowledge_dir, db)?;

    Ok(report)
}

fn plan_managed_directory_reuse(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    backend_signature: &str,
    embedding_backfill_ready: bool,
    force_rebuild: bool,
) -> Result<ManagedDirectoryReuseDecision, String> {
    let unity_snapshot = unity_docs::current_unity_reference_managed_snapshot(working_dir)?;
    let mut decision = plan_unity_reference_reuse(
        working_dir,
        app_knowledge_dir,
        db,
        backend_signature,
        embedding_backfill_ready,
        force_rebuild,
        unity_snapshot.as_ref(),
    )?
    .unwrap_or_default();

    decision
        .snapshot_persistence
        .push(ManagedDirectorySnapshotPersistence {
            managed_path: unity_docs::UNITY_REFERENCE_MANAGED_PATH.to_string(),
            snapshot: managed_snapshot_row_from_snapshot(unity_snapshot.as_ref()),
        });
    Ok(decision)
}

fn plan_unity_reference_reuse(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    backend_signature: &str,
    embedding_backfill_ready: bool,
    force_rebuild: bool,
    current_snapshot: Option<&unity_docs::UnityReferenceManagedSnapshot>,
) -> Result<Option<ManagedDirectoryReuseDecision>, String> {
    if force_rebuild {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=force_rebuild",
            working_dir
        );
        return Ok(None);
    }

    let Some(current_snapshot) = current_snapshot else {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=no_current_snapshot",
            working_dir
        );
        return Ok(None);
    };
    if current_snapshot.document_count != current_snapshot.expected_document_count {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=snapshot_count_mismatch current={} expected={}",
            working_dir, current_snapshot.document_count, current_snapshot.expected_document_count
        );
        return Ok(None);
    }

    let Some(stored_snapshot) =
        db.get_managed_directory_snapshot(&current_snapshot.managed_path)?
    else {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=no_stored_snapshot managed_path={}",
            working_dir, current_snapshot.managed_path
        );
        return Ok(None);
    };
    if stored_snapshot.fingerprint != current_snapshot.fingerprint
        || stored_snapshot.document_count != current_snapshot.document_count
    {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=stored_snapshot_mismatch stored_docs={} current_docs={} stored_fingerprint={} current_fingerprint={}",
            working_dir,
            stored_snapshot.document_count,
            current_snapshot.document_count,
            stored_snapshot.fingerprint,
            current_snapshot.fingerprint
        );
        return Ok(None);
    }

    let catalog_rows = db.list_document_catalog_entries_with_prefix(
        KnowledgeType::Reference.as_str(),
        &current_snapshot.doc_path_prefix,
    )?;
    if catalog_rows.len() != current_snapshot.document_count {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=catalog_count_mismatch catalog_rows={} snapshot_docs={}",
            working_dir,
            catalog_rows.len(),
            current_snapshot.document_count
        );
        return Ok(None);
    }

    let state_rows = db.list_index_states_with_prefix(
        KnowledgeType::Reference.as_str(),
        &current_snapshot.doc_path_prefix,
    )?;
    if state_rows.len() != current_snapshot.document_count {
        eprintln!(
            "[KnowledgeIndex] unity reuse skipped workspace={} reason=index_state_count_mismatch state_rows={} snapshot_docs={}",
            working_dir,
            state_rows.len(),
            current_snapshot.document_count
        );
        return Ok(None);
    }
    let state_map: HashMap<String, DocIndexState> = state_rows
        .into_iter()
        .map(|state| (state.doc_id.clone(), state))
        .collect();
    let docs_missing_embeddings = if embedding_backfill_ready {
        db.list_docs_missing_embeddings_with_prefix(
            KnowledgeType::Reference.as_str(),
            &current_snapshot.doc_path_prefix,
        )?
    } else {
        HashSet::new()
    };

    let mut access_cache = HashMap::new();
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));
    let mut retained_doc_ids = HashSet::with_capacity(current_snapshot.document_count);
    for row in catalog_rows {
        let Some(state) = state_map.get(&row.doc_id) else {
            return Ok(None);
        };
        let access = apply_general_search_config(
            cached_document_search_access(
                working_dir,
                app_knowledge_dir,
                KnowledgeType::Reference,
                &row.doc_path,
                &mut access_cache,
            )?,
            &general_config,
        );
        let expected_backend = build_embedding_backend_state_marker(backend_signature, access);
        if state.stale != 0
            || state.index_version != INDEX_VERSION
            || state.doc_type != row.doc_type
            || state.doc_path != row.doc_path
            || state.embedding_backend != expected_backend
            || (embedding_backfill_ready
                && access.vector_enabled
                && docs_missing_embeddings.contains(&row.doc_id))
        {
            eprintln!(
                "[KnowledgeIndex] unity reuse skipped workspace={} reason=state_mismatch doc_id={} doc_path={} stale={} index_version={} state_doc_type={} state_doc_path={} expected_backend={} actual_backend={} missing_embedding={}",
                working_dir,
                row.doc_id,
                row.doc_path,
                state.stale,
                state.index_version,
                state.doc_type,
                state.doc_path,
                expected_backend,
                state.embedding_backend,
                embedding_backfill_ready
                    && access.vector_enabled
                    && docs_missing_embeddings.contains(&row.doc_id)
            );
            return Ok(None);
        }
        retained_doc_ids.insert(row.doc_id);
    }

    eprintln!(
        "[KnowledgeIndex] unity reuse accepted workspace={} docs={}",
        working_dir,
        retained_doc_ids.len()
    );
    Ok(Some(ManagedDirectoryReuseDecision {
        excluded_prefixes: vec![(
            KnowledgeType::Reference,
            current_snapshot.doc_path_prefix.clone(),
        )],
        retained_doc_ids,
        snapshot_persistence: Vec::new(),
    }))
}

fn managed_snapshot_row_from_snapshot(
    snapshot: Option<&unity_docs::UnityReferenceManagedSnapshot>,
) -> Option<ManagedDirectorySnapshotRow> {
    let Some(snapshot) = snapshot else {
        return None;
    };
    if snapshot.document_count != snapshot.expected_document_count {
        return None;
    }
    Some(ManagedDirectorySnapshotRow {
        managed_path: snapshot.managed_path.clone(),
        fingerprint: snapshot.fingerprint.clone(),
        document_count: snapshot.document_count,
    })
}

fn persist_managed_directory_snapshots(
    db: &KnowledgeDb,
    snapshots: &[ManagedDirectorySnapshotPersistence],
) -> Result<(), String> {
    for snapshot in snapshots {
        if let Some(row) = snapshot.snapshot.as_ref() {
            db.upsert_managed_directory_snapshot(row)?;
        } else {
            db.delete_managed_directory_snapshot(&snapshot.managed_path)?;
        }
    }
    Ok(())
}

fn is_directory_config_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.ends_with(".locus-meta") || value.ends_with(".meta"))
        .unwrap_or(false)
}

fn managed_reference_config_signature(
    working_dir: &str,
    managed_dir: &str,
) -> Result<String, String> {
    let type_root = Path::new(working_dir)
        .join("Locus")
        .join("knowledge")
        .join(KnowledgeType::Reference.as_str());
    let managed_root = type_root.join(managed_dir);
    let mut config_paths = Vec::<PathBuf>::new();

    for suffix in [".locus-meta", ".meta"] {
        let root_config = type_root.join(format!("{}{}", managed_dir, suffix));
        if root_config.is_file() {
            config_paths.push(root_config);
        }
    }

    if managed_root.is_dir() {
        for entry in WalkDir::new(&managed_root)
            .min_depth(1)
            .into_iter()
            .flatten()
        {
            if entry.file_type().is_file() && is_directory_config_file(entry.path()) {
                config_paths.push(entry.path().to_path_buf());
            }
        }
    }

    config_paths.sort();
    config_paths.dedup();

    let mut hasher = Sha256::new();
    hasher.update(managed_dir.as_bytes());
    for path in config_paths {
        let relative = path
            .strip_prefix(&type_root)
            .map_err(|e| format!("Failed to resolve managed config signature path: {}", e))?
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read(&path).map_err(|e| {
            format!(
                "Failed to read managed config signature source '{}': {}",
                path.display(),
                e
            )
        })?;
        hasher.update(relative.as_bytes());
        hasher.update((contents.len() as u64).to_le_bytes());
        hasher.update(contents);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn build_unity_managed_retrieval_summary_cache_row(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    snapshot: &unity_docs::UnityReferenceManagedSnapshot,
) -> Result<ManagedRetrievalSummaryCacheRow, String> {
    let config_signature =
        managed_reference_config_signature(working_dir, unity_docs::UNITY_REFERENCE_MANAGED_DIR)?;
    let catalog_rows = db.list_document_catalog_entries_with_prefix(
        KnowledgeType::Reference.as_str(),
        unity_docs::UNITY_REFERENCE_MANAGED_DIR,
    )?;
    let state_map = db
        .list_index_states_with_prefix(
            KnowledgeType::Reference.as_str(),
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
        )?
        .into_iter()
        .map(|state| (state.doc_id.clone(), state))
        .collect::<HashMap<_, _>>();

    let mut access_cache = HashMap::new();
    let mut lexical_enabled_docs = 0usize;
    let mut vector_enabled_docs = 0usize;
    let mut lexical_fresh_docs = 0usize;
    let mut lexical_stale_docs = 0usize;
    let mut fresh_enabled_docs = 0usize;

    for row in &catalog_rows {
        let access = cached_document_search_access(
            working_dir,
            app_knowledge_dir,
            KnowledgeType::Reference,
            &row.doc_path,
            &mut access_cache,
        )?;
        if access.lexical_enabled {
            lexical_enabled_docs += 1;
        }
        if access.vector_enabled {
            vector_enabled_docs += 1;
        }
        if let Some(state) = state_map.get(&row.doc_id) {
            if access.lexical_enabled {
                if state.stale == 0 {
                    lexical_fresh_docs += 1;
                } else {
                    lexical_stale_docs += 1;
                }
            }
            if state.stale == 0 && (access.lexical_enabled || access.vector_enabled) {
                fresh_enabled_docs += 1;
            }
        }
    }

    Ok(ManagedRetrievalSummaryCacheRow {
        managed_path: snapshot.managed_path.clone(),
        fingerprint: snapshot.fingerprint.clone(),
        config_signature,
        total_docs: catalog_rows.len(),
        lexical_enabled_docs,
        vector_enabled_docs,
        lexical_fresh_docs,
        lexical_stale_docs,
        fresh_enabled_docs,
        chunk_count: db.count_chunks_for_fresh_docs_with_prefix(
            KnowledgeType::Reference.as_str(),
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
        )?,
        embedded_chunk_count: db.count_embeddings_for_fresh_docs_with_prefix(
            KnowledgeType::Reference.as_str(),
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
        )?,
        embedded_doc_count: db.count_distinct_docs_with_embeddings_for_fresh_docs_with_prefix(
            KnowledgeType::Reference.as_str(),
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
        )?,
        updated_at: chrono::Utc::now().timestamp_millis(),
    })
}

fn refresh_unity_managed_retrieval_summary_cache(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
) -> Result<Option<ManagedRetrievalSummaryCacheRow>, String> {
    let Some(snapshot) = unity_docs::current_unity_reference_managed_snapshot(working_dir)? else {
        db.delete_managed_retrieval_summary_cache(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?;
        return Ok(None);
    };
    if snapshot.document_count != snapshot.expected_document_count {
        db.delete_managed_retrieval_summary_cache(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?;
        return Ok(None);
    }

    let next = build_unity_managed_retrieval_summary_cache_row(
        working_dir,
        app_knowledge_dir,
        db,
        &snapshot,
    )?;
    db.upsert_managed_retrieval_summary_cache(&next)?;
    Ok(Some(next))
}

fn get_or_build_unity_managed_retrieval_summary_cache(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
) -> Result<Option<ManagedRetrievalSummaryCacheRow>, String> {
    let Some(snapshot) = unity_docs::current_unity_reference_managed_snapshot(working_dir)? else {
        db.delete_managed_retrieval_summary_cache(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?;
        return Ok(None);
    };
    if snapshot.document_count != snapshot.expected_document_count {
        db.delete_managed_retrieval_summary_cache(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?;
        return Ok(None);
    }

    let config_signature =
        managed_reference_config_signature(working_dir, unity_docs::UNITY_REFERENCE_MANAGED_DIR)?;
    if let Some(cached) =
        db.get_managed_retrieval_summary_cache(unity_docs::UNITY_REFERENCE_MANAGED_PATH)?
    {
        if cached.fingerprint == snapshot.fingerprint && cached.config_signature == config_signature
        {
            return Ok(Some(cached));
        }
    }

    refresh_unity_managed_retrieval_summary_cache(working_dir, app_knowledge_dir, db)
}

async fn ensure_document_catalog_available(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
) -> Result<(), String> {
    let workspace_key = normalize_workspace_cache_key(working_dir);
    let mut bootstrapped = state.catalog_bootstrapped_workspaces.lock().await;
    if !bootstrapped.contains(&workspace_key) {
        let started_at = Instant::now();
        eprintln!(
            "[KnowledgeIndex] bootstrap start workspace={} app_root={}",
            working_dir,
            app_knowledge_dir
                .map(|value| value.display().to_string())
                .unwrap_or_else(|| "<none>".to_string())
        );
        reconcile_workspace(working_dir, app_knowledge_dir, state.clone()).await?;
        bootstrapped.insert(workspace_key);
        eprintln!(
            "[KnowledgeIndex] bootstrap finished workspace={} elapsed_ms={}",
            working_dir,
            started_at.elapsed().as_millis()
        );
        return Ok(());
    }

    let catalog_count = state.db().count_document_catalog_entries()?;
    if catalog_count == 0 {
        let started_at = Instant::now();
        eprintln!(
            "[KnowledgeIndex] bootstrap retry for empty catalog workspace={}",
            working_dir
        );
        reconcile_workspace(working_dir, app_knowledge_dir, state.clone()).await?;
        eprintln!(
            "[KnowledgeIndex] empty catalog bootstrap finished workspace={} elapsed_ms={}",
            working_dir,
            started_at.elapsed().as_millis()
        );
    }
    Ok(())
}

fn normalize_workspace_cache_key(working_dir: &str) -> String {
    working_dir
        .trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

pub async fn list_cached_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
    state: Arc<KnowledgeIndexState>,
) -> Result<Vec<KnowledgeListItem>, String> {
    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;
    let rows = state.db().list_document_catalog_entries_filtered(
        doc_type.map(|value| value.as_str()),
        path_prefix,
    )?;

    rows.into_iter()
        .map(catalog_row_to_cached_document)
        .map(|result| result.map(|entry| entry.item))
        .collect()
}

pub async fn list_cached_documents_page(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: Option<KnowledgeType>,
    path_prefix: Option<&str>,
    limit: usize,
    offset: usize,
    state: Arc<KnowledgeIndexState>,
) -> Result<CachedKnowledgeListPage, String> {
    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;
    let rows = state.db().list_document_catalog_entries_page(
        doc_type.map(|value| value.as_str()),
        path_prefix,
        limit,
        offset,
    )?;
    build_cached_document_page(rows, limit, offset)
}

pub async fn list_cached_directory_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: KnowledgeType,
    directory_path: Option<&str>,
    state: Arc<KnowledgeIndexState>,
) -> Result<Vec<KnowledgeListItem>, String> {
    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;
    let normalized_directory = directory_path
        .map(|value| value.trim().trim_matches('/').replace('\\', "/"))
        .filter(|value| !value.is_empty());
    let rows = state.db().list_document_catalog_directory_entries(
        doc_type.as_str(),
        normalized_directory.as_deref(),
    )?;

    rows.into_iter()
        .map(catalog_row_to_cached_document)
        .map(|result| result.map(|entry| entry.item))
        .collect()
}

pub async fn list_cached_directory_documents_page(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: KnowledgeType,
    directory_path: Option<&str>,
    limit: usize,
    offset: usize,
    state: Arc<KnowledgeIndexState>,
) -> Result<CachedKnowledgeListPage, String> {
    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;
    let normalized_directory = directory_path
        .map(|value| value.trim().trim_matches('/').replace('\\', "/"))
        .filter(|value| !value.is_empty());
    let rows = state.db().list_document_catalog_directory_entries_page(
        doc_type.as_str(),
        normalized_directory.as_deref(),
        limit,
        offset,
    )?;
    build_cached_document_page(rows, limit, offset)
}

pub async fn upsert_document(
    state: Arc<KnowledgeIndexState>,
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    document: KnowledgeDocument,
) -> Result<(), String> {
    let db = state.db();
    let tantivy = state.tantivy();
    let mgr_handle = state.embedding_mgr();
    let mgr = mgr_handle.lock().await;
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));
    let access = apply_general_search_config(
        knowledge_store::effective_document_search_access_with_app_root(
            working_dir,
            app_knowledge_dir,
            document.doc_type,
            &document.path,
        )?,
        &general_config,
    );
    let desired_state = build_index_state(&document, &mgr.backend_signature_json(), access);
    let catalog_row = build_document_catalog_row(&document, access)?;
    let prepared =
        prepare_document_update_sync(Some(&mgr), &document, desired_state, access, true)?;
    let _ = commit_prepared_updates_batch(&db, &tantivy, &[prepared])?;
    db.upsert_document_catalog_entries(&[catalog_row])?;
    Ok(())
}

pub fn remove_documents(state: Arc<KnowledgeIndexState>, doc_ids: &[String]) -> Result<(), String> {
    if doc_ids.is_empty() {
        return Ok(());
    }

    let db = state.db();
    let tantivy = state.tantivy();
    tantivy.remove_docs(doc_ids)?;
    db.delete_documents(doc_ids)
}

pub fn remove_shadowed_documents_for_path(
    state: Arc<KnowledgeIndexState>,
    doc_type: KnowledgeType,
    doc_path: &str,
    keep_doc_id: Option<&str>,
) -> Result<(), String> {
    let rows = state
        .db()
        .find_document_catalog_entries_by_path(doc_type.as_str(), doc_path)?;
    let doc_ids = rows
        .into_iter()
        .filter_map(|row| {
            if keep_doc_id
                .map(|expected| row.doc_id == expected)
                .unwrap_or(false)
            {
                None
            } else {
                Some(row.doc_id)
            }
        })
        .collect::<Vec<_>>();
    remove_documents(state, &doc_ids)
}

pub async fn build_overview(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    state: Arc<KnowledgeIndexState>,
    model_storage_dir: &Path,
) -> Result<KnowledgeOverview, String> {
    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;

    let db = state.db();
    let library_dir = library_dir_for_working_dir(working_dir);
    let general_config = load_general_config(&library_dir);
    let catalog_rows = db.list_document_catalog_entries(None)?;
    let all_states = db.list_all_index_states()?;
    let state_map: HashMap<String, DocIndexState> = all_states
        .into_iter()
        .map(|state| (state.doc_id.clone(), state))
        .collect();
    let unity_managed_cache =
        get_or_build_unity_managed_retrieval_summary_cache(working_dir, app_knowledge_dir, &db)?;
    let mut access_cache = HashMap::new();
    let mut total_document_count = unity_managed_cache
        .as_ref()
        .map(|value| value.total_docs)
        .unwrap_or(0);
    let mut lexical_indexable_count =
        if general_config.enabled && general_config.lexical_search_enabled {
            unity_managed_cache
                .as_ref()
                .map(|value| value.lexical_enabled_docs)
                .unwrap_or(0)
        } else {
            0
        };
    let mut lexical_fresh_count = if general_config.enabled && general_config.lexical_search_enabled
    {
        unity_managed_cache
            .as_ref()
            .map(|value| value.lexical_fresh_docs)
            .unwrap_or(0)
    } else {
        0
    };
    let mut lexical_stale_count = if general_config.enabled && general_config.lexical_search_enabled
    {
        unity_managed_cache
            .as_ref()
            .map(|value| value.lexical_stale_docs)
            .unwrap_or(0)
    } else {
        0
    };
    let mut vector_indexable_count =
        if general_config.enabled && general_config.semantic_search_enabled {
            unity_managed_cache
                .as_ref()
                .map(|value| value.vector_enabled_docs)
                .unwrap_or(0)
        } else {
            0
        };
    let mut fresh_enabled_count = if general_config.enabled {
        unity_managed_cache
            .as_ref()
            .map(|value| value.fresh_enabled_docs)
            .unwrap_or(0)
    } else {
        0
    };
    for row in catalog_rows {
        if unity_managed_cache.is_some()
            && row.doc_type == KnowledgeType::Reference.as_str()
            && unity_docs::is_unity_reference_managed_relative_path(&row.doc_path)
        {
            continue;
        }
        total_document_count += 1;
        let doc_type = match row.doc_type.as_str() {
            "design" => KnowledgeType::Design,
            "memory" => KnowledgeType::Memory,
            "skill" => KnowledgeType::Skill,
            "reference" => KnowledgeType::Reference,
            _ => continue,
        };
        let access = apply_general_search_config(
            cached_document_search_access(
                working_dir,
                app_knowledge_dir,
                doc_type,
                &row.doc_path,
                &mut access_cache,
            )?,
            &general_config,
        );
        if access.lexical_enabled {
            lexical_indexable_count += 1;
        }
        if access.vector_enabled {
            vector_indexable_count += 1;
        }
        if let Some(state) = state_map.get(&row.doc_id) {
            if access.lexical_enabled {
                if state.stale == 0 {
                    lexical_fresh_count += 1;
                } else {
                    lexical_stale_count += 1;
                }
            }
            if state.stale == 0 && (access.lexical_enabled || access.vector_enabled) {
                fresh_enabled_count += 1;
            }
        }
    }
    let chunk_count = db.count_chunks_for_fresh_docs()?;
    let embedded_chunk_count = db.count_embeddings_for_fresh_docs()?;
    let embedded_doc_count = db.count_distinct_docs_with_embeddings_for_fresh_docs()?;

    let (
        embedding_config,
        backend_signature,
        device_name,
        gpu_memory_bytes,
        gpu_dedicated_memory_bytes,
        managed_model_dir,
    ) = if let Ok(mgr) = state.embedding_mgr().try_lock() {
        (
            mgr.config().clone(),
            mgr.backend_signature(),
            mgr.device_name().unwrap_or_default(),
            mgr.gpu_memory_bytes().unwrap_or(0),
            mgr.gpu_dedicated_memory_bytes().unwrap_or(0),
            mgr.managed_model_root_path().to_path_buf(),
        )
    } else {
        let fallback_mgr =
            EmbeddingManager::new(embedding::load_config(&library_dir), model_storage_dir);
        (
            fallback_mgr.config().clone(),
            fallback_mgr.backend_signature(),
            fallback_mgr.device_name().unwrap_or_default(),
            0,
            fallback_mgr.gpu_dedicated_memory_bytes().unwrap_or(0),
            fallback_mgr.managed_model_root_path().to_path_buf(),
        )
    };
    let embedding_status = state.embedding_status_snapshot();

    let lexical_index_bytes = directory_size(&library_dir.join("knowledge_tantivy_index"));
    let db_bytes = file_size(&library_dir.join("knowledge_index.db"));
    let model_bytes = directory_size(&managed_model_dir);

    Ok(KnowledgeOverview {
        total_document_count,
        full_text: KnowledgeFullTextOverview {
            enabled: general_config.enabled && general_config.lexical_search_enabled,
            indexable_item_count: lexical_indexable_count,
            indexed_item_count: lexical_fresh_count.min(lexical_indexable_count),
            fresh_item_count: lexical_fresh_count,
            stale_item_count: lexical_stale_count,
            pending_item_count: lexical_indexable_count.saturating_sub(lexical_fresh_count),
            chunk_count,
            last_build_at: state.lexical_rebuild_status_snapshot().completed_at,
        },
        semantic: KnowledgeSemanticOverview {
            enabled: general_config.enabled
                && general_config.semantic_search_enabled
                && embedding_config.enabled,
            ready: embedding_status.ready,
            backend: embedding_backend_label(&embedding_config),
            model: embedding_model_label(&embedding_config),
            device_route: backend_signature.device_route,
            device_name,
            indexed_item_count: embedded_doc_count.min(vector_indexable_count),
            embedded_chunk_count,
            pending_item_count: vector_indexable_count.saturating_sub(embedded_doc_count),
            coverage_ratio: if vector_indexable_count > 0 {
                embedded_doc_count as f64 / vector_indexable_count as f64
            } else {
                0.0
            },
            stage: embedding_status.stage,
            error: embedding_status.error,
        },
        performance: KnowledgePerformanceOverview {
            db_bytes,
            lexical_index_bytes,
            local_model_bytes: model_bytes,
            gpu_memory_bytes,
            gpu_dedicated_memory_bytes,
            total_bytes: lexical_index_bytes
                .saturating_add(db_bytes)
                .saturating_add(model_bytes),
            avg_chunks_per_item: if fresh_enabled_count > 0 {
                chunk_count as f64 / fresh_enabled_count as f64
            } else {
                0.0
            },
        },
    })
}

pub async fn query_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    lexical_query: Option<&str>,
    semantic_query: Option<&str>,
    types: Option<&[KnowledgeType]>,
    path_prefix: Option<&str>,
    limit: usize,
    state: Arc<KnowledgeIndexState>,
) -> Result<Vec<KnowledgeSearchHit>, String> {
    let general_config = load_general_config(&library_dir_for_working_dir(working_dir));
    if !general_config.enabled {
        return Ok(Vec::new());
    }

    ensure_document_catalog_available(working_dir, app_knowledge_dir, state.clone()).await?;
    let db = state.db();
    let tantivy = state.tantivy();

    let query_limit = limit.max(1).min(50);
    let lexical_query = lexical_query.filter(|value| !value.trim().is_empty());
    let (lexical_hits, lexical_kind) = match lexical_query {
        Some(value) if general_config.lexical_search_enabled => (
            lexical_index_search_documents(
                working_dir,
                app_knowledge_dir,
                &db,
                &tantivy,
                value,
                types,
                path_prefix,
                query_limit * 6,
            )?,
            Some(KeywordSearchKind::LexicalIndex),
        ),
        Some(value) => (
            text_scan_search_documents(
                working_dir,
                app_knowledge_dir,
                value,
                types,
                path_prefix,
                query_limit * 6,
            )?,
            Some(KeywordSearchKind::TextScan),
        ),
        None => (Vec::new(), None),
    };
    let semantic_hits = if general_config.semantic_search_enabled
        && semantic_query
            .map(|value| should_use_semantic_recall(value))
            .unwrap_or(false)
    {
        if let Ok(mgr) = state.embedding_mgr().try_lock() {
            if mgr.config().enabled && mgr.is_ready() {
                semantic_recall(
                    &db,
                    &mgr,
                    semantic_query.unwrap_or_default(),
                    query_limit * 6,
                )?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let hit_doc_ids: Vec<String> = lexical_hits
        .iter()
        .map(|hit| hit.doc_id.clone())
        .chain(semantic_hits.iter().map(|hit| hit.doc_id.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let doc_map = load_cached_documents_by_ids(&db, &hit_doc_ids)?;
    let mut access_cache = HashMap::new();
    let mut filtered_lexical = Vec::new();
    for hit in lexical_hits {
        if !cached_doc_allowed(&doc_map, &hit.doc_id, types, path_prefix) {
            continue;
        }
        let Some(document) = doc_map.get(&hit.doc_id) else {
            continue;
        };
        let access = cached_document_entry_search_access(
            working_dir,
            app_knowledge_dir,
            document,
            &mut access_cache,
        )?;
        if access.lexical_enabled {
            filtered_lexical.push(hit);
        }
    }

    let mut filtered_semantic = Vec::new();
    for hit in semantic_hits {
        if !cached_doc_allowed(&doc_map, &hit.doc_id, types, path_prefix) {
            continue;
        }
        let Some(document) = doc_map.get(&hit.doc_id) else {
            continue;
        };
        let access = cached_document_entry_search_access(
            working_dir,
            app_knowledge_dir,
            document,
            &mut access_cache,
        )?;
        if access.vector_enabled {
            filtered_semantic.push(hit);
        }
    }

    let mut entries: HashMap<String, FusionEntry> = HashMap::new();
    for (rank, hit) in filtered_lexical.iter().enumerate() {
        let entry = entries
            .entry(hit.doc_id.clone())
            .or_insert_with(|| FusionEntry {
                doc_id: hit.doc_id.clone(),
                lexical_rank: None,
                lexical_kind: None,
                semantic_rank: None,
                semantic_score: None,
                section: hit.section.clone(),
                snippet: hit.snippet.clone(),
            });
        if entry.lexical_rank.is_none() {
            entry.lexical_rank = Some(rank);
            entry.lexical_kind = lexical_kind;
            entry.section = hit.section.clone();
            entry.snippet = hit.snippet.clone();
        }
    }
    for (rank, hit) in filtered_semantic.iter().enumerate() {
        let entry = entries
            .entry(hit.doc_id.clone())
            .or_insert_with(|| FusionEntry {
                doc_id: hit.doc_id.clone(),
                lexical_rank: None,
                lexical_kind: None,
                semantic_rank: None,
                semantic_score: None,
                section: hit.section.clone(),
                snippet: hit.snippet.clone(),
            });
        if entry.semantic_rank.is_none() {
            entry.semantic_rank = Some(rank);
            entry.semantic_score = Some(hit.score);
            if entry.lexical_rank.is_none() {
                entry.section = hit.section.clone();
                entry.snippet = hit.snippet.clone();
            }
        }
    }

    const K: f64 = 60.0;
    let title_match_query = lexical_query
        .filter(|value| !value.trim().is_empty())
        .or(semantic_query)
        .unwrap_or_default()
        .to_lowercase();
    let mut results = entries
        .into_values()
        .filter_map(|entry| {
            let document = doc_map.get(&entry.doc_id)?;
            let mut score = 0.0;
            if let Some(rank) = entry.lexical_rank {
                score += 1.0 / (K + rank as f64);
            }
            if let Some(rank) = entry.semantic_rank {
                score += 1.0 / (K + rank as f64);
            }
            if !title_match_query.is_empty()
                && document
                    .item
                    .title
                    .to_lowercase()
                    .contains(&title_match_query)
            {
                score += 0.3;
            }
            let match_kind = match (entry.lexical_rank.is_some(), entry.semantic_rank.is_some()) {
                (true, true) => match entry.lexical_kind {
                    Some(KeywordSearchKind::TextScan) => "grepHybrid",
                    _ => "hybrid",
                },
                (true, false) => match entry.lexical_kind {
                    Some(KeywordSearchKind::TextScan) => "grep",
                    _ => "lexical",
                },
                (false, true) => "semantic",
                _ => "lexical",
            };

            Some(KnowledgeSearchHit {
                id: document.item.id.clone(),
                doc_type: document.item.doc_type,
                path: document.item.path.clone(),
                title: document.item.title.clone(),
                storage_source: document.item.storage_source,
                inject_mode: document.item.inject_mode,
                ai_maintained: document.item.ai_maintained,
                score: score as f32,
                snippet: entry.snippet,
                matched_section: section_to_match_section(&entry.section),
                has_summary: document.item.has_summary,
                updated_at: document.item.updated_at,
                match_kind: match_kind.to_string(),
                semantic_score: entry.semantic_score,
                semantic_confidence: entry.semantic_score.map(semantic_confidence),
                estimated_tokens: Some(document.estimated_tokens),
            })
        })
        .collect::<Vec<_>>();

    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(query_limit);
    Ok(results)
}

type PendingRebuildDocument = (
    KnowledgeDocument,
    DocIndexState,
    DirectorySearchAccess,
    bool,
);
type PendingDocumentAnalysisInput = (KnowledgeDocument, DirectorySearchAccess);

fn should_parallelize_prepare_analysis(
    embedding_backfill_ready: bool,
    documents: &[PendingDocumentAnalysisInput],
) -> bool {
    if documents.len() < PREPARING_ANALYSIS_BATCH_DOCS {
        return false;
    }
    if std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
        <= 1
    {
        return false;
    }
    !(embedding_backfill_ready && documents.iter().any(|(_, access)| access.vector_enabled))
}

fn analyze_document_for_rebuild(
    db: &KnowledgeDb,
    document: &KnowledgeDocument,
    access: DirectorySearchAccess,
    state_map: &HashMap<String, DocIndexState>,
    backend_signature: &str,
    embedding_backfill_ready: bool,
    force_rebuild: bool,
    force_lexical_sync: bool,
) -> Result<PreparedDocumentAnalysis, String> {
    let catalog_row = build_document_catalog_row(document, access)?;
    let desired_state = build_index_state(document, backend_signature, access);
    let rebuild_plan = match state_map.get(&document.id) {
        Some(existing) => rebuild_plan_for_document(
            force_rebuild,
            force_lexical_sync,
            existing,
            &desired_state,
            if embedding_backfill_ready && access.vector_enabled {
                needs_embedding_backfill(db, &document.id)?
            } else {
                false
            },
        ),
        None => RebuildPlan {
            reason: Some(RebuildReason::Added),
            lexical_sync: access.lexical_enabled,
        },
    };

    Ok(PreparedDocumentAnalysis {
        catalog_row,
        desired_state,
        rebuild_reason: rebuild_plan.reason,
        lexical_sync: rebuild_plan.lexical_sync,
    })
}

fn analyze_prepare_batch(
    db: &KnowledgeDb,
    documents: &[PendingDocumentAnalysisInput],
    state_map: &HashMap<String, DocIndexState>,
    backend_signature: &str,
    embedding_backfill_ready: bool,
    force_rebuild: bool,
    force_lexical_sync: bool,
    parallelize: bool,
) -> Result<Vec<PreparedDocumentAnalysis>, String> {
    if parallelize {
        let analyzed = documents
            .par_iter()
            .map(|(document, access)| {
                analyze_document_for_rebuild(
                    db,
                    document,
                    *access,
                    state_map,
                    backend_signature,
                    embedding_backfill_ready,
                    force_rebuild,
                    force_lexical_sync,
                )
            })
            .collect::<Vec<_>>();
        analyzed.into_iter().collect()
    } else {
        documents
            .iter()
            .map(|(document, access)| {
                analyze_document_for_rebuild(
                    db,
                    document,
                    *access,
                    state_map,
                    backend_signature,
                    embedding_backfill_ready,
                    force_rebuild,
                    force_lexical_sync,
                )
            })
            .collect()
    }
}

fn should_parallelize_prepare_batch(
    embedding_mgr: Option<&EmbeddingManager>,
    batch: &[PendingRebuildDocument],
) -> bool {
    if batch.len() < PARALLEL_PREPARE_DOC_THRESHOLD {
        return false;
    }
    if std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
        <= 1
    {
        return false;
    }
    !embedding_mgr
        .map(|_| batch.iter().any(|(_, _, access, _)| access.vector_enabled))
        .unwrap_or(false)
}

fn prepare_rebuild_batch(
    embedding_mgr: Option<&EmbeddingManager>,
    batch: &[PendingRebuildDocument],
) -> Result<Vec<PreparedIndexUpdate>, String> {
    if should_parallelize_prepare_batch(embedding_mgr, batch) {
        let prepared = batch
            .par_iter()
            .map(|(document, state, access, lexical_sync)| {
                prepare_document_update_sync(
                    embedding_mgr,
                    document,
                    state.clone(),
                    *access,
                    *lexical_sync,
                )
            })
            .collect::<Vec<_>>();
        prepared.into_iter().collect()
    } else {
        batch
            .iter()
            .map(|(document, state, access, lexical_sync)| {
                prepare_document_update_sync(
                    embedding_mgr,
                    document,
                    state.clone(),
                    *access,
                    *lexical_sync,
                )
            })
            .collect()
    }
}

fn prepared_batch_commit_report(prepared: &[PreparedIndexUpdate]) -> PreparedBatchCommitReport {
    let mut report = PreparedBatchCommitReport {
        prepared_count: prepared.len(),
        chunk_count: prepared
            .iter()
            .map(|update| update.chunks.len())
            .sum::<usize>(),
        lexical_doc_count: prepared
            .iter()
            .filter(|update| update.lexical.is_some())
            .count(),
        embedding_vector_count: prepared
            .iter()
            .filter_map(|update| update.embeddings.as_ref())
            .map(|vectors| vectors.len())
            .sum::<usize>(),
        ..PreparedBatchCommitReport::default()
    };

    for update in prepared {
        if update.embedding_attempted {
            report.embedding_attempted_docs += 1;
        }
        if let Some(failure) = &update.embedding_failure {
            report.embedding_failed_docs += 1;
            report.last_embedding_failure = Some(failure.clone());
        }
    }

    report
}

fn commit_prepared_updates_batch(
    db: &KnowledgeDb,
    tantivy: &KnowledgeTantivyIndex,
    prepared: &[PreparedIndexUpdate],
) -> Result<BatchCommitTimings, String> {
    if prepared.is_empty() {
        return Ok(BatchCommitTimings::default());
    }

    let lexical_removals = prepared
        .iter()
        .filter_map(|update| update.lexical_remove_doc_id.clone())
        .collect::<Vec<_>>();
    let lexical_docs = prepared
        .iter()
        .filter_map(|update| update.lexical.clone())
        .collect::<Vec<_>>();
    let persist_updates = prepared
        .iter()
        .map(|update| DocumentPersistUpdate {
            state: &update.state,
            chunks: &update.chunks,
            embeddings: update.embeddings.as_deref(),
        })
        .collect::<Vec<_>>();

    std::thread::scope(|scope| {
        let tantivy_task = (!lexical_removals.is_empty() || !lexical_docs.is_empty()).then(|| {
            scope.spawn(|| {
                let started = Instant::now();
                tantivy.apply_batch(&lexical_removals, &lexical_docs)?;
                Ok::<u64, String>(started.elapsed().as_millis() as u64)
            })
        });
        let db_task = scope.spawn(|| {
            let started = Instant::now();
            db.apply_document_updates(&persist_updates)?;
            Ok::<u64, String>(started.elapsed().as_millis() as u64)
        });

        let lexical_elapsed_ms = if let Some(task) = tantivy_task {
            Some(
                task.join()
                    .map_err(|_| "Knowledge tantivy batch worker panicked".to_string())??,
            )
        } else {
            None
        };
        let db_elapsed_ms = db_task
            .join()
            .map_err(|_| "Knowledge db batch worker panicked".to_string())??;
        Ok(BatchCommitTimings {
            db_elapsed_ms,
            lexical_elapsed_ms,
        })
    })
}

fn prepare_and_commit_rebuild_batch(
    db: &KnowledgeDb,
    tantivy: &KnowledgeTantivyIndex,
    embedding_mgr: Option<&EmbeddingManager>,
    batch: &[PendingRebuildDocument],
) -> Result<PreparedBatchCommitReport, String> {
    let total_started = Instant::now();
    let prepare_started = Instant::now();
    let prepared = prepare_rebuild_batch(embedding_mgr, batch)?;
    let mut report = prepared_batch_commit_report(&prepared);
    report.prepare_elapsed_ms = prepare_started.elapsed().as_millis() as u64;
    tracing::info!(
        log_module = "knowledge_index",
        prepared_docs = report.prepared_count,
        lexical_docs = report.lexical_doc_count,
        lexical_chunks = report.chunk_count,
        embedding_attempted_docs = report.embedding_attempted_docs,
        embedding_failed_docs = report.embedding_failed_docs,
        embedding_vectors = report.embedding_vector_count,
        prepare_elapsed_ms = report.prepare_elapsed_ms,
        "knowledge rebuild batch commit start"
    );
    match commit_prepared_updates_batch(db, tantivy, &prepared) {
        Ok(timings) => {
            report.db_elapsed_ms = timings.db_elapsed_ms;
            report.lexical_elapsed_ms = timings.lexical_elapsed_ms;
            report.total_elapsed_ms = total_started.elapsed().as_millis() as u64;
            tracing::info!(
                log_module = "knowledge_index",
                prepared_docs = report.prepared_count,
                lexical_docs = report.lexical_doc_count,
                lexical_chunks = report.chunk_count,
                embedding_attempted_docs = report.embedding_attempted_docs,
                embedding_failed_docs = report.embedding_failed_docs,
                embedding_vectors = report.embedding_vector_count,
                prepare_elapsed_ms = report.prepare_elapsed_ms,
                db_elapsed_ms = report.db_elapsed_ms,
                lexical_elapsed_ms = report.lexical_elapsed_ms.unwrap_or_default(),
                total_elapsed_ms = report.total_elapsed_ms,
                "knowledge rebuild batch commit completed"
            );
        }
        Err(err) => {
            report.total_elapsed_ms = total_started.elapsed().as_millis() as u64;
            tracing::error!(
                log_module = "knowledge_index",
                prepared_docs = report.prepared_count,
                lexical_docs = report.lexical_doc_count,
                lexical_chunks = report.chunk_count,
                embedding_attempted_docs = report.embedding_attempted_docs,
                embedding_failed_docs = report.embedding_failed_docs,
                embedding_vectors = report.embedding_vector_count,
                prepare_elapsed_ms = report.prepare_elapsed_ms,
                total_elapsed_ms = report.total_elapsed_ms,
                error = %err,
                "knowledge rebuild batch commit failed"
            );
            return Err(err);
        }
    }
    Ok(report)
}

fn prepare_document_update_sync(
    embedding_mgr: Option<&EmbeddingManager>,
    document: &KnowledgeDocument,
    mut state: DocIndexState,
    access: DirectorySearchAccess,
    lexical_sync: bool,
) -> Result<PreparedIndexUpdate, String> {
    if !access.lexical_enabled && !access.vector_enabled {
        state.stale = 0;
        return Ok(PreparedIndexUpdate {
            state,
            lexical: None,
            lexical_remove_doc_id: lexical_sync.then_some(document.id.clone()),
            chunks: Vec::new(),
            embeddings: None,
            embedding_attempted: false,
            embedding_failure: None,
        });
    }

    let summary = knowledge_store::active_summary(document);
    let rules = knowledge_store::active_maintenance_rules(document);
    let chunks = chunker::chunk_document(&document.title, summary, rules, &document.body);
    let tantivy_chunks = chunks
        .iter()
        .map(|chunk| (chunk.section.clone(), chunk.seq, chunk.text.clone()))
        .collect::<Vec<_>>();

    let embedding_attempted =
        access.vector_enabled && embedding_mgr.map(|mgr| mgr.is_ready()).unwrap_or(false);
    let mut embedding_failure = None;
    let embeddings = if embedding_attempted {
        let texts = chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>();
        match embedding_mgr.and_then(|mgr| mgr.embed_documents(&texts)) {
            Some(Ok(vectors)) => Some(vectors),
            Some(Err(err)) => {
                tracing::error!(
                    log_module = "knowledge_index",
                    document_path = %document.path,
                    error = %err,
                    "embedding failed for document"
                );
                eprintln!(
                    "[knowledge_index] embedding failed for {}: {}",
                    document.path, err
                );
                embedding_failure = Some(EmbeddingDocumentFailure {
                    document_path: document.path.clone(),
                    message: err.clone(),
                });
                None
            }
            None => None,
        }
    } else {
        None
    };

    state.stale = 0;
    Ok(PreparedIndexUpdate {
        state,
        lexical: (lexical_sync && access.lexical_enabled).then(|| LexicalDocumentRecord {
            doc_id: document.id.clone(),
            title: document.title.clone(),
            path: document.path.clone(),
            keywords: summary.unwrap_or_default().to_string(),
            chunks: tantivy_chunks,
        }),
        lexical_remove_doc_id: (lexical_sync && !access.lexical_enabled)
            .then_some(document.id.clone()),
        chunks,
        embeddings,
        embedding_attempted,
        embedding_failure,
    })
}

fn load_all_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    excluded_prefixes: &[(KnowledgeType, String)],
) -> Result<Vec<KnowledgeDocument>, String> {
    knowledge_store::load_documents_with_app_root_excluding_prefixes(
        working_dir,
        app_knowledge_dir,
        None,
        None,
        excluded_prefixes,
    )
}

fn build_index_state(
    document: &KnowledgeDocument,
    embedding_backend: &str,
    access: DirectorySearchAccess,
) -> DocIndexState {
    DocIndexState {
        doc_id: document.id.clone(),
        doc_type: document.doc_type.as_str().to_string(),
        doc_path: document.path.clone(),
        title_hash: hash_bytes(&document.title),
        summary_hash: hash_bytes(knowledge_store::active_summary(document).unwrap_or_default()),
        body_hash: hash_bytes(&document.body),
        rules_hash: hash_bytes(
            knowledge_store::active_maintenance_rules(document).unwrap_or_default(),
        ),
        index_version: INDEX_VERSION,
        embedding_backend: build_embedding_backend_state_marker(embedding_backend, access),
        stale: 0,
    }
}

fn build_document_catalog_row(
    document: &KnowledgeDocument,
    access: DirectorySearchAccess,
) -> Result<DocumentCatalogRow, String> {
    let item = build_list_item(document, Some(access));
    let payload_json = serde_json::to_string(&item)
        .map_err(|e| format!("Failed to serialize knowledge catalog entry: {}", e))?;
    Ok(DocumentCatalogRow {
        doc_id: item.id.clone(),
        doc_type: item.doc_type.as_str().to_string(),
        doc_path: item.path.clone(),
        parent_path: document_parent_directory(&item.path),
        title: item.title.clone(),
        updated_at: item.updated_at,
        estimated_tokens: estimate_document_tokens(document),
        payload_json,
    })
}

fn build_list_item(
    document: &KnowledgeDocument,
    access: Option<DirectorySearchAccess>,
) -> KnowledgeListItem {
    KnowledgeListItem {
        id: document.id.clone(),
        doc_type: document.doc_type,
        path: document.path.clone(),
        title: document.title.clone(),
        inject_mode: document.inject_mode,
        summary_enabled: document.summary_enabled,
        command_enabled: document.command_enabled,
        read_only: document.read_only,
        ai_maintained: document.ai_maintained,
        explicit_maintenance_rules: document.explicit_maintenance_rules,
        storage_source: document.storage_source,
        external_source: document.external_source.clone(),
        skill_enabled: document.skill_enabled,
        skill_surface: document.skill_surface,
        command_trigger: document.command_trigger.clone(),
        argument_hint: document.argument_hint.clone(),
        created_at: document.created_at,
        updated_at: document.updated_at,
        has_summary: knowledge_store::active_summary(document).is_some(),
        has_body_content: !document.body.trim().is_empty(),
        byte_size: knowledge_store::rendered_document_size_bytes(document).ok(),
        lexical_search_enabled: access.map(|value| value.lexical_enabled),
        semantic_search_enabled: access.map(|value| value.vector_enabled),
        summary: document.summary.clone(),
    }
}

fn build_cached_document_page(
    mut rows: Vec<DocumentCatalogRow>,
    limit: usize,
    offset: usize,
) -> Result<CachedKnowledgeListPage, String> {
    let has_more = rows.len() > limit;
    if has_more {
        rows.truncate(limit);
    }
    let items = rows
        .into_iter()
        .map(catalog_row_to_cached_document)
        .map(|result| result.map(|entry| entry.item))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CachedKnowledgeListPage {
        items,
        next_offset: has_more.then_some(offset.saturating_add(limit)),
    })
}

fn load_cached_documents_by_ids(
    db: &KnowledgeDb,
    doc_ids: &[String],
) -> Result<HashMap<String, CachedDocumentEntry>, String> {
    db.get_document_catalog_entries(doc_ids)?
        .into_iter()
        .map(|row| {
            let doc_id = row.doc_id.clone();
            catalog_row_to_cached_document(row).map(|entry| (doc_id, entry))
        })
        .collect()
}

fn catalog_row_to_cached_document(row: DocumentCatalogRow) -> Result<CachedDocumentEntry, String> {
    let item = serde_json::from_str::<KnowledgeListItem>(&row.payload_json).map_err(|e| {
        format!(
            "Failed to decode knowledge catalog row '{}': {}",
            row.doc_id, e
        )
    })?;
    Ok(CachedDocumentEntry {
        item,
        estimated_tokens: row.estimated_tokens,
    })
}

fn document_parent_directory(path: &str) -> Option<String> {
    let parent = Path::new(path).parent()?;
    let normalized = parent.to_string_lossy().replace('\\', "/");
    let trimmed = normalized.trim_matches('/').trim();
    if trimmed.is_empty() || trimmed == "." {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn directory_access_cache_path(doc_type: KnowledgeType, doc_path: &str) -> Option<String> {
    if doc_type == KnowledgeType::Reference
        && unity_docs::is_unity_reference_managed_relative_path(doc_path)
    {
        return Some(unity_docs::UNITY_REFERENCE_MANAGED_DIR.to_string());
    }
    document_parent_directory(doc_path)
}

fn build_embedding_backend_state_marker(
    embedding_backend: &str,
    access: DirectorySearchAccess,
) -> String {
    if access.vector_enabled {
        format!(
            "{}|l={}|v=1",
            embedding_backend,
            if access.lexical_enabled { 1 } else { 0 }
        )
    } else {
        format!(
            "vector-disabled|l={}|v=0",
            if access.lexical_enabled { 1 } else { 0 }
        )
    }
}

fn knowledge_type_from_str(value: &str) -> Result<KnowledgeType, String> {
    match value {
        "design" => Ok(KnowledgeType::Design),
        "memory" => Ok(KnowledgeType::Memory),
        "skill" => Ok(KnowledgeType::Skill),
        "reference" => Ok(KnowledgeType::Reference),
        _ => Err(format!("Unknown knowledge type '{}'", value)),
    }
}

fn cached_document_search_access(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    doc_type: KnowledgeType,
    doc_path: &str,
    cache: &mut HashMap<DirectoryAccessCacheKey, DirectorySearchAccess>,
) -> Result<DirectorySearchAccess, String> {
    let Some(parent_path) = directory_access_cache_path(doc_type, doc_path) else {
        return Ok(DirectorySearchAccess {
            lexical_enabled: true,
            vector_enabled: true,
        });
    };
    let key = DirectoryAccessCacheKey {
        doc_type,
        path: parent_path.clone(),
    };
    if let Some(access) = cache.get(&key) {
        return Ok(*access);
    }
    let access = knowledge_store::effective_directory_search_access_with_app_root(
        working_dir,
        app_knowledge_dir,
        doc_type,
        &parent_path,
    )?;
    cache.insert(key, access);
    Ok(access)
}

fn should_parallelize_directory_access_resolution(unique_dirs: usize) -> bool {
    unique_dirs >= 16
        && std::thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1)
            > 1
}

fn populate_document_access_cache_for_batch(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    documents: &[KnowledgeDocument],
    cache: &mut HashMap<DirectoryAccessCacheKey, DirectorySearchAccess>,
) -> Result<(), String> {
    let mut seen_missing = HashSet::new();
    let mut missing_keys = Vec::new();

    for document in documents {
        let Some(parent_path) = directory_access_cache_path(document.doc_type, &document.path)
        else {
            continue;
        };
        let key = DirectoryAccessCacheKey {
            doc_type: document.doc_type,
            path: parent_path,
        };
        if cache.contains_key(&key) {
            continue;
        }
        if seen_missing.insert(key.clone()) {
            missing_keys.push(key);
        }
    }

    if missing_keys.is_empty() {
        return Ok(());
    }

    let resolved = if should_parallelize_directory_access_resolution(missing_keys.len()) {
        let resolved = missing_keys
            .par_iter()
            .map(|key| {
                let access = knowledge_store::effective_directory_search_access_with_app_root(
                    working_dir,
                    app_knowledge_dir,
                    key.doc_type,
                    &key.path,
                )?;
                Ok((key.clone(), access))
            })
            .collect::<Vec<_>>();
        resolved.into_iter().collect::<Result<Vec<_>, String>>()?
    } else {
        missing_keys
            .into_iter()
            .map(|key| {
                let access = knowledge_store::effective_directory_search_access_with_app_root(
                    working_dir,
                    app_knowledge_dir,
                    key.doc_type,
                    &key.path,
                )?;
                Ok((key, access))
            })
            .collect::<Result<Vec<_>, String>>()?
    };

    for (key, access) in resolved {
        cache.insert(key, access);
    }
    Ok(())
}

fn batch_document_search_inputs(
    documents: Vec<KnowledgeDocument>,
    cache: &HashMap<DirectoryAccessCacheKey, DirectorySearchAccess>,
    general_config: &KnowledgeGeneralConfig,
) -> Vec<PendingDocumentAnalysisInput> {
    documents
        .into_iter()
        .map(|document| {
            let access = directory_access_cache_path(document.doc_type, &document.path)
                .and_then(|parent_path| {
                    cache
                        .get(&DirectoryAccessCacheKey {
                            doc_type: document.doc_type,
                            path: parent_path,
                        })
                        .copied()
                })
                .unwrap_or(DirectorySearchAccess {
                    lexical_enabled: true,
                    vector_enabled: true,
                });
            (
                document,
                apply_general_search_config(access, general_config),
            )
        })
        .collect()
}

fn cached_document_entry_search_access(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    document: &CachedDocumentEntry,
    cache: &mut HashMap<DirectoryAccessCacheKey, DirectorySearchAccess>,
) -> Result<DirectorySearchAccess, String> {
    cached_document_search_access(
        working_dir,
        app_knowledge_dir,
        document.item.doc_type,
        &document.item.path,
        cache,
    )
}

fn index_state_core_eq(left: &DocIndexState, right: &DocIndexState) -> bool {
    left.doc_type == right.doc_type
        && left.doc_path == right.doc_path
        && left.title_hash == right.title_hash
        && left.summary_hash == right.summary_hash
        && left.body_hash == right.body_hash
        && left.rules_hash == right.rules_hash
        && left.index_version == right.index_version
}

fn parse_embedding_backend_state_marker(
    embedding_backend: &str,
) -> (String, DirectorySearchAccess) {
    let mut backend = embedding_backend.to_string();
    let mut lexical_enabled = None;
    let mut vector_enabled = None;

    for (index, part) in embedding_backend.split('|').enumerate() {
        if index == 0 {
            backend = part.to_string();
            continue;
        }
        if let Some(value) = part.strip_prefix("l=") {
            lexical_enabled = match value {
                "1" => Some(true),
                "0" => Some(false),
                _ => lexical_enabled,
            };
            continue;
        }
        if let Some(value) = part.strip_prefix("v=") {
            vector_enabled = match value {
                "1" => Some(true),
                "0" => Some(false),
                _ => vector_enabled,
            };
        }
    }

    (
        backend.clone(),
        DirectorySearchAccess {
            lexical_enabled: lexical_enabled.unwrap_or(true),
            vector_enabled: vector_enabled.unwrap_or(backend != "vector-disabled"),
        },
    )
}

fn rebuild_plan_for_document(
    force_rebuild: bool,
    force_lexical_sync: bool,
    existing: &DocIndexState,
    desired: &DocIndexState,
    embedding_backfill_missing: bool,
) -> RebuildPlan {
    let (existing_backend, existing_access) =
        parse_embedding_backend_state_marker(&existing.embedding_backend);
    let (desired_backend, desired_access) =
        parse_embedding_backend_state_marker(&desired.embedding_backend);
    let lexical_stale = existing.stale != 0
        || !index_state_core_eq(existing, desired)
        || existing_access.lexical_enabled != desired_access.lexical_enabled;
    let vector_stale = existing_access.vector_enabled != desired_access.vector_enabled
        || (existing_access.vector_enabled
            && desired_access.vector_enabled
            && existing_backend != desired_backend);

    if force_rebuild {
        return RebuildPlan {
            reason: Some(RebuildReason::Forced),
            lexical_sync: force_lexical_sync || lexical_stale,
        };
    }
    if lexical_stale {
        return RebuildPlan {
            reason: Some(RebuildReason::LexicalStale),
            lexical_sync: true,
        };
    }
    if vector_stale {
        return RebuildPlan {
            reason: Some(RebuildReason::VectorStale),
            lexical_sync: false,
        };
    }
    if embedding_backfill_missing {
        return RebuildPlan {
            reason: Some(RebuildReason::EmbeddingBackfill),
            lexical_sync: false,
        };
    }
    RebuildPlan {
        reason: None,
        lexical_sync: false,
    }
}

fn rebuild_reason_for_document(
    force_rebuild: bool,
    existing: &DocIndexState,
    desired: &DocIndexState,
    embedding_backfill_missing: bool,
) -> Option<RebuildReason> {
    rebuild_plan_for_document(
        force_rebuild,
        force_rebuild,
        existing,
        desired,
        embedding_backfill_missing,
    )
    .reason
}

fn needs_embedding_backfill(db: &KnowledgeDb, doc_id: &str) -> Result<bool, String> {
    let chunk_count = db.count_chunks_for_doc(doc_id)?;
    if chunk_count == 0 {
        return Ok(false);
    }
    let embedding_count = db.count_embeddings_for_doc(doc_id)?;
    Ok(embedding_count < chunk_count)
}

fn lexical_index_search_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    db: &KnowledgeDb,
    tantivy: &KnowledgeTantivyIndex,
    query: &str,
    types: Option<&[KnowledgeType]>,
    path_prefix: Option<&str>,
    limit: usize,
) -> Result<Vec<LexicalHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let target_limit = limit.max(1);
    let indexed_entry_count = tantivy.indexed_entry_count()?;
    if indexed_entry_count == 0 {
        return Ok(Vec::new());
    }

    let max_search_limit = indexed_entry_count.max(target_limit).min(50_000);
    let mut search_limit = target_limit.min(max_search_limit);

    loop {
        let hits = tantivy.search(query, search_limit, 2.0)?;
        let hit_doc_ids = hits
            .iter()
            .map(|hit| hit.doc_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let doc_map = load_cached_documents_by_ids(db, &hit_doc_ids)?;
        let mut access_cache = HashMap::new();
        let mut seen_doc_ids = HashSet::new();
        let mut filtered_hits = Vec::new();

        for hit in hits.iter() {
            if !cached_doc_allowed(&doc_map, &hit.doc_id, types, path_prefix) {
                continue;
            }
            let Some(document) = doc_map.get(&hit.doc_id) else {
                continue;
            };
            let access = cached_document_entry_search_access(
                working_dir,
                app_knowledge_dir,
                document,
                &mut access_cache,
            )?;
            if !access.lexical_enabled {
                continue;
            }
            if seen_doc_ids.insert(hit.doc_id.clone()) {
                filtered_hits.push(hit.clone());
                if filtered_hits.len() >= target_limit {
                    return Ok(filtered_hits);
                }
            }
        }

        if hits.len() < search_limit || search_limit >= max_search_limit {
            return Ok(filtered_hits);
        }

        let next_limit = search_limit.saturating_mul(2).min(max_search_limit);
        if next_limit == search_limit {
            return Ok(filtered_hits);
        }
        search_limit = next_limit;
    }
}

fn text_scan_search_documents(
    working_dir: &str,
    app_knowledge_dir: Option<&std::path::PathBuf>,
    query: &str,
    types: Option<&[KnowledgeType]>,
    path_prefix: Option<&str>,
    limit: usize,
) -> Result<Vec<LexicalHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let mut documents = Vec::new();
    if let Some(types) = types {
        for doc_type in types {
            documents.extend(knowledge_store::load_documents_with_app_root(
                working_dir,
                app_knowledge_dir,
                Some(*doc_type),
                path_prefix,
            )?);
        }
    } else {
        documents = knowledge_store::load_documents_with_app_root(
            working_dir,
            app_knowledge_dir,
            None,
            path_prefix,
        )?;
    }

    let mut hits = documents
        .into_iter()
        .filter_map(|document| {
            let access = knowledge_store::effective_document_search_access_with_app_root(
                working_dir,
                app_knowledge_dir,
                document.doc_type,
                &document.path,
            )
            .ok()?;
            if !access.lexical_enabled {
                return None;
            }
            let (score, snippet, matched_section) =
                knowledge_store::score_document_text_match(query, &document)?;
            Some(LexicalHit {
                doc_id: document.id,
                section: match_section_name(matched_section).to_string(),
                title: document.title,
                path: document.path,
                score,
                snippet: truncate_snippet(&snippet, 220),
            })
        })
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.title.cmp(&right.title))
    });
    hits.truncate(limit.max(1));
    Ok(hits)
}

fn semantic_recall(
    db: &KnowledgeDb,
    embedding_mgr: &EmbeddingManager,
    query: &str,
    top_k: usize,
) -> Result<Vec<SemanticHit>, String> {
    let query_vec = match embedding_mgr.embed_query(query) {
        Some(Ok(vectors)) if !vectors.is_empty() => vectors.into_iter().next().unwrap(),
        Some(Err(err)) => return Err(err),
        _ => return Ok(Vec::new()),
    };

    let all_embeddings = db.load_all_embeddings()?;
    if all_embeddings.is_empty() {
        return Ok(Vec::new());
    }

    let mut scored: Vec<(usize, f32)> = all_embeddings
        .par_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let score = cosine_similarity(&query_vec, &row.vector);
            passes_semantic_score_threshold(score).then_some((index, score))
        })
        .collect();
    scored.par_sort_unstable_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_k);

    let mut chunks_cache: HashMap<String, Vec<db::ChunkRow>> = HashMap::new();
    let hits = scored
        .into_iter()
        .map(|(index, score)| {
            let row: &EmbeddingRow = &all_embeddings[index];
            let chunks = chunks_cache
                .entry(row.doc_id.clone())
                .or_insert_with(|| db.get_chunks(&row.doc_id).unwrap_or_default());
            let snippet = chunks
                .iter()
                .find(|chunk| chunk.section == row.section && chunk.seq == row.seq)
                .map(|chunk| truncate_snippet(&chunk.text, 220))
                .unwrap_or_default();

            SemanticHit {
                doc_id: row.doc_id.clone(),
                section: row.section.clone(),
                score,
                snippet,
            }
        })
        .collect();

    Ok(hits)
}

fn cached_doc_allowed(
    docs: &HashMap<String, CachedDocumentEntry>,
    doc_id: &str,
    types: Option<&[KnowledgeType]>,
    path_prefix: Option<&str>,
) -> bool {
    let Some(document) = docs.get(doc_id) else {
        return false;
    };
    if let Some(types) = types {
        if !types.contains(&document.item.doc_type) {
            return false;
        }
    }
    if let Some(path_prefix) = path_prefix {
        if !document.item.path.starts_with(path_prefix) {
            return false;
        }
    }
    true
}

fn section_to_match_section(section: &str) -> Option<KnowledgeSearchMatchSection> {
    match section {
        "summary" => Some(KnowledgeSearchMatchSection::Summary),
        "maintenanceRules" => Some(KnowledgeSearchMatchSection::MaintenanceRules),
        "body" => Some(KnowledgeSearchMatchSection::Body),
        _ => None,
    }
}

fn match_section_name(section: Option<KnowledgeSearchMatchSection>) -> &'static str {
    match section {
        Some(KnowledgeSearchMatchSection::Summary) => "summary",
        Some(KnowledgeSearchMatchSection::MaintenanceRules) => "maintenanceRules",
        Some(KnowledgeSearchMatchSection::Body) => "body",
        None => "",
    }
}

fn estimate_document_tokens(document: &KnowledgeDocument) -> u64 {
    let mut text = String::new();
    if let Some(summary) = knowledge_store::active_summary(document) {
        text.push_str(summary);
        text.push('\n');
    }
    if let Some(rules) = knowledge_store::active_maintenance_rules(document) {
        text.push_str(rules);
        text.push('\n');
    }
    text.push_str(&document.body);
    estimate_tokens_from_text(&text)
}

fn estimate_tokens_from_text(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    ((text.as_bytes().len() as f64) / 3.5).ceil() as u64
}

fn hash_bytes<T: AsRef<[u8]>>(value: T) -> Vec<u8> {
    blake3::hash(value.as_ref()).as_bytes().to_vec()
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_left = 0.0f32;
    let mut norm_right = 0.0f32;
    for index in 0..left.len() {
        dot += left[index] * right[index];
        norm_left += left[index] * left[index];
        norm_right += right[index] * right[index];
    }
    let denom = norm_left.sqrt() * norm_right.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut end = max_chars;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn directory_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .metadata()
                .ok()
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len())
        })
        .sum()
}

fn embedding_backend_label(config: &EmbeddingConfig) -> String {
    if config.embedding_mode == "remote" {
        "Remote".to_string()
    } else {
        "fastembed".to_string()
    }
}

fn embedding_model_label(config: &EmbeddingConfig) -> String {
    if config.embedding_mode == "remote" {
        config.remote_model.clone()
    } else if !config.local_model.trim().is_empty() {
        config.local_model.clone()
    } else if !config.local_model_path.trim().is_empty() {
        std::path::Path::new(config.local_model_path.trim())
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(config.local_model_path.trim())
            .to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_embedding_backend_state_marker, build_overview, embedding_rebuild_detail,
        embedding_stage_progress, lexical_stage_progress, library_dir_for_working_dir,
        list_cached_documents, needs_embedding_backfill, plan_managed_directory_reuse,
        query_documents, rebuild_plan_for_document, rebuild_reason_for_document,
        reconcile_unity_reference_import, reconcile_workspace_internal, save_general_config,
        DirectorySearchAccess, KnowledgeGeneralConfig, KnowledgeIndexState, KnowledgeRuntime,
        RebuildReason, INDEX_VERSION, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
    };
    use crate::knowledge_index::db::{ChunkRecord, DocIndexState, KnowledgeDb};
    use crate::knowledge_store::{
        default_directory_config_for_type, save_document, update_directory_config,
        FolderIndexRuleSetting, KnowledgeConfigSource, KnowledgeConfigSourceKind,
        KnowledgeDocument, KnowledgeExternalSource, KnowledgeInjectMode,
        KnowledgeSearchMatchSection, KnowledgeSourceProvider, KnowledgeStorageSource,
        KnowledgeType,
    };
    use crate::unity_docs;
    use std::{path::Path, sync::Arc, time::Duration};
    use tempfile::tempdir;

    fn sample_state(doc_id: &str) -> DocIndexState {
        DocIndexState {
            doc_id: doc_id.to_string(),
            doc_type: "design".to_string(),
            doc_path: "combat/core-loop.md".to_string(),
            title_hash: vec![1],
            summary_hash: vec![2],
            body_hash: vec![3],
            rules_hash: vec![4],
            index_version: INDEX_VERSION,
            embedding_backend: "backend-a".to_string(),
            stale: 0,
        }
    }

    #[test]
    fn general_config_defaults_keep_lexical_search_disabled() {
        let config = KnowledgeGeneralConfig::default();

        assert!(config.enabled);
        assert!(!config.lexical_search_enabled);
        assert!(!config.semantic_search_enabled);
    }

    #[test]
    fn rebuild_reason_uses_embedding_backfill_for_otherwise_fresh_docs() {
        let existing = sample_state("doc-1");
        let desired = sample_state("doc-1");

        assert_eq!(
            rebuild_reason_for_document(false, &existing, &desired, true),
            Some(RebuildReason::EmbeddingBackfill)
        );
    }

    #[test]
    fn rebuild_reason_prioritizes_lexical_stale_before_embedding_backfill() {
        let mut existing = sample_state("doc-1");
        existing.stale = 1;
        let desired = sample_state("doc-1");

        assert_eq!(
            rebuild_reason_for_document(false, &existing, &desired, true),
            Some(RebuildReason::LexicalStale)
        );
    }

    #[test]
    fn rebuild_plan_marks_vector_only_changes_without_lexical_sync() {
        let existing = DocIndexState {
            embedding_backend: "backend-a|l=1|v=1".to_string(),
            ..sample_state("doc-1")
        };
        let desired = DocIndexState {
            embedding_backend: "vector-disabled|l=1|v=0".to_string(),
            ..sample_state("doc-1")
        };

        let plan = rebuild_plan_for_document(false, false, &existing, &desired, false);

        assert_eq!(plan.reason, Some(RebuildReason::VectorStale));
        assert!(!plan.lexical_sync);
    }

    #[test]
    fn embedding_backend_marker_tracks_folder_retrieval_capabilities() {
        assert_eq!(
            build_embedding_backend_state_marker(
                "backend-a",
                DirectorySearchAccess {
                    lexical_enabled: true,
                    vector_enabled: true,
                },
            ),
            "backend-a|l=1|v=1"
        );
        assert_eq!(
            build_embedding_backend_state_marker(
                "backend-a",
                DirectorySearchAccess {
                    lexical_enabled: false,
                    vector_enabled: true,
                },
            ),
            "backend-a|l=0|v=1"
        );
        assert_eq!(
            build_embedding_backend_state_marker(
                "backend-a",
                DirectorySearchAccess {
                    lexical_enabled: true,
                    vector_enabled: false,
                },
            ),
            "vector-disabled|l=1|v=0"
        );
    }

    #[test]
    fn needs_embedding_backfill_detects_missing_vectors() {
        let dir = tempdir().expect("temp dir");
        let db_path = dir.path().join("knowledge_index.db");
        let db = KnowledgeDb::open_or_recover(&db_path).expect("open db");
        let chunk_ids = db
            .replace_chunks(
                "doc-1",
                &[
                    ChunkRecord {
                        section: "body".to_string(),
                        seq: 0,
                        text: "chunk one".to_string(),
                        text_hash: vec![1],
                    },
                    ChunkRecord {
                        section: "body".to_string(),
                        seq: 1,
                        text: "chunk two".to_string(),
                        text_hash: vec![2],
                    },
                ],
            )
            .expect("insert chunks");

        assert!(needs_embedding_backfill(&db, "doc-1").expect("backfill check before embeddings"));

        db.upsert_embedding(chunk_ids[0], &[0.1_f32, 0.2_f32], 2)
            .expect("insert first embedding");
        assert!(needs_embedding_backfill(&db, "doc-1").expect("partial embeddings"));

        db.upsert_embedding(chunk_ids[1], &[0.3_f32, 0.4_f32], 2)
            .expect("insert second embedding");
        assert!(!needs_embedding_backfill(&db, "doc-1").expect("complete embeddings"));
    }

    fn save_design_document(
        working_dir: &str,
        id: &str,
        path: &str,
        title: &str,
        summary: Option<&str>,
        body: &str,
        updated_at: i64,
    ) {
        save_document(
            working_dir,
            KnowledgeDocument {
                id: id.to_string(),
                doc_type: KnowledgeType::Design,
                path: path.to_string(),
                title: title.to_string(),
                inject_mode: KnowledgeInjectMode::Path,
                inherit_inject_mode: false,
                inject_mode_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                summary_enabled: true,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: summary.map(str::to_string),
                body: body.to_string(),
                maintenance_rules: None,
                created_at: 1,
                updated_at,
            },
        )
        .expect("save knowledge document");
    }

    fn seed_design_document(working_dir: &str) {
        save_design_document(
            working_dir,
            "kd_test_design_doc",
            "combat/core-loop.md",
            "核心循环",
            Some("战斗核心循环"),
            "正文",
            1,
        );
    }

    fn memory_document(index: usize, body: String, updated_at: i64) -> KnowledgeDocument {
        KnowledgeDocument {
            id: format!("kd_test_memory_doc_{:03}", index),
            doc_type: KnowledgeType::Memory,
            path: format!("project/doc-{:03}.md", index),
            title: format!("Project Memory {:03}", index),
            inject_mode: KnowledgeInjectMode::Path,
            inherit_inject_mode: false,
            inject_mode_source: KnowledgeConfigSource {
                kind: KnowledgeConfigSourceKind::SelfValue,
                path: None,
            },
            summary_enabled: true,
            command_enabled: false,
            read_only: false,
            ai_maintained: false,
            storage_source: KnowledgeStorageSource::Project,
            inherit_ai_config: false,
            ai_config_source: KnowledgeConfigSource {
                kind: KnowledgeConfigSourceKind::SelfValue,
                path: None,
            },
            explicit_maintenance_rules: false,
            external_source: None,
            skill_enabled: None,
            skill_surface: None,
            command_trigger: None,
            argument_hint: None,
            summary: Some(format!("Project memory summary {:03}", index)),
            body,
            maintenance_rules: None,
            created_at: 1,
            updated_at,
        }
    }

    fn save_memory_document(working_dir: &str, index: usize, body: &str, updated_at: i64) {
        save_document(
            working_dir,
            memory_document(index, body.to_string(), updated_at),
        )
        .expect("save memory document");
    }

    fn seed_memory_documents(working_dir: &str, document_count: usize) {
        for index in 0..document_count {
            save_memory_document(
                working_dir,
                index,
                &format!("Project memory body {:03}", index),
                1,
            );
        }
    }

    fn seed_unity_reference_managed_document(working_dir: &str) {
        crate::unity_docs::seed_managed_documents_for_tests(
            working_dir,
            &[KnowledgeDocument {
                id: "kd_unity_execution_order".to_string(),
                doc_type: KnowledgeType::Reference,
                path: "unity-official-docs/manual/ExecutionOrder.md".to_string(),
                title: "Execution Order".to_string(),
                inject_mode: KnowledgeInjectMode::None,
                inherit_inject_mode: false,
                inject_mode_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                summary_enabled: true,
                command_enabled: false,
                read_only: true,
                ai_maintained: false,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                explicit_maintenance_rules: false,
                external_source: Some(KnowledgeExternalSource {
                    provider: KnowledgeSourceProvider::Unity,
                    locator: Some(
                        "https://docs.unity3d.com/2022.3/Documentation/Manual/ExecutionOrder.html"
                            .to_string(),
                    ),
                    source_id: Some("unity-2022.3".to_string()),
                    sync_enabled: true,
                }),
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: Some("Execution order".to_string()),
                body: "Execution order details".to_string(),
                maintenance_rules: None,
                created_at: 1,
                updated_at: 1,
            }],
        )
        .expect("save unity index doc");

        let manifest_path = std::path::Path::new(working_dir)
            .join("Library")
            .join("Locus")
            .join("unity_reference_docs_manifest.json");
        std::fs::create_dir_all(manifest_path.parent().expect("manifest dir"))
            .expect("create manifest dir");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "projectVersion": "2022.3.47f1",
                "docsVersion": "2022.3",
                "locale": "en",
                "importedAt": 1,
                "importedDocCount": 1,
                "sourceUrl": "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html"
            })
            .to_string(),
        )
        .expect("write unity manifest");
    }

    fn write_unity_reference_manifest(working_dir: &str, imported_doc_count: usize) {
        let manifest_path = std::path::Path::new(working_dir)
            .join("Library")
            .join("Locus")
            .join("unity_reference_docs_manifest.json");
        std::fs::create_dir_all(manifest_path.parent().expect("manifest dir"))
            .expect("create manifest dir");
        std::fs::write(
            &manifest_path,
            serde_json::json!({
                "projectVersion": "2022.3.47f1",
                "docsVersion": "2022.3",
                "locale": "en",
                "importedAt": 1,
                "importedDocCount": imported_doc_count,
                "sourceUrl": "https://docs.unity3d.com/2022.3/Documentation/Manual/index.html"
            })
            .to_string(),
        )
        .expect("write unity manifest");
    }

    fn seed_unity_reference_documents(working_dir: &str, document_count: usize) {
        let documents = (0..document_count)
            .map(|index| KnowledgeDocument {
                id: format!("kd_unity_ref_{:03}", index),
                doc_type: KnowledgeType::Reference,
                path: format!(
                    "{}/manual/doc-{:03}.md",
                    unity_docs::UNITY_REFERENCE_MANAGED_DIR,
                    index
                ),
                title: format!("Unity Manual {:03}", index),
                inject_mode: KnowledgeInjectMode::None,
                inherit_inject_mode: false,
                inject_mode_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                summary_enabled: true,
                command_enabled: false,
                read_only: true,
                ai_maintained: false,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                explicit_maintenance_rules: false,
                external_source: Some(KnowledgeExternalSource {
                    provider: KnowledgeSourceProvider::Unity,
                    locator: Some(format!(
                        "https://docs.unity3d.com/2022.3/Documentation/Manual/doc-{:03}.html",
                        index
                    )),
                    source_id: Some("unity-2022.3".to_string()),
                    sync_enabled: true,
                }),
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: Some(format!("Unity 文档摘要 {:03}", index)),
                body: format!("Unity 文档正文 {:03}", index),
                maintenance_rules: None,
                created_at: 1,
                updated_at: 1,
            })
            .collect::<Vec<_>>();
        unity_docs::seed_managed_documents_for_tests(working_dir, &documents)
            .expect("seed unity managed docs");
        write_unity_reference_manifest(working_dir, document_count);
    }

    fn create_state(working_dir: &str) -> Arc<KnowledgeIndexState> {
        let runtime = KnowledgeRuntime::open(
            &library_dir_for_working_dir(working_dir),
            Path::new(working_dir),
        )
        .expect("open runtime");
        Arc::new(KnowledgeIndexState::new(
            runtime.db,
            runtime.tantivy,
            runtime.embedding_mgr,
        ))
    }

    fn save_test_general_config(
        working_dir: &str,
        lexical_search_enabled: bool,
        semantic_search_enabled: bool,
    ) {
        save_general_config(
            &library_dir_for_working_dir(working_dir),
            &KnowledgeGeneralConfig {
                enabled: true,
                lexical_search_enabled,
                semantic_search_enabled,
            },
        )
        .expect("save knowledge general config");
    }

    #[tokio::test]
    async fn query_documents_uses_text_scan_when_lexical_index_disabled() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        save_test_general_config(&working_dir, false, false);
        let state = create_state(&working_dir);

        let hits = query_documents(
            &working_dir,
            None,
            Some("战斗核心"),
            None,
            None,
            None,
            5,
            state.clone(),
        )
        .await
        .expect("query documents");
        let indexed_hits = state
            .tantivy()
            .search("战斗核心", 5, 2.0)
            .expect("query tantivy directly");

        assert!(hits.iter().any(|hit| {
            hit.id == "kd_test_design_doc"
                && hit.match_kind == "grep"
                && hit.matched_section == Some(KnowledgeSearchMatchSection::Summary)
        }));
        assert!(indexed_hits.is_empty());
    }

    #[tokio::test]
    async fn query_documents_text_scan_matches_split_keyword_terms() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        save_test_general_config(&working_dir, false, false);
        let state = create_state(&working_dir);

        let hits = query_documents(
            &working_dir,
            None,
            Some("战斗 核心"),
            None,
            Some(&[KnowledgeType::Design]),
            Some("combat"),
            5,
            state,
        )
        .await
        .expect("query documents");

        assert!(hits.iter().any(|hit| {
            hit.id == "kd_test_design_doc"
                && hit.match_kind == "grep"
                && hit.matched_section == Some(KnowledgeSearchMatchSection::Summary)
        }));
    }

    #[tokio::test]
    async fn query_documents_text_scan_respects_directory_lexical_access() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        save_test_general_config(&working_dir, false, false);

        let mut directory_config = default_directory_config_for_type(KnowledgeType::Design);
        directory_config.lexical_search = FolderIndexRuleSetting::Disabled;
        update_directory_config(
            &working_dir,
            KnowledgeType::Design,
            "combat",
            directory_config,
        )
        .expect("disable directory lexical search");

        let state = create_state(&working_dir);
        let hits = query_documents(
            &working_dir,
            None,
            Some("战斗核心"),
            None,
            None,
            None,
            5,
            state,
        )
        .await
        .expect("query documents");

        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn query_documents_lexical_index_refills_after_path_filter() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        for index in 0..24 {
            save_design_document(
                &working_dir,
                &format!("kd_noise_doc_{:03}", index),
                &format!("noise/doc-{:03}.md", index),
                &format!("共享检索词 Noise {:03}", index),
                Some("共享检索词 noise summary"),
                "共享检索词 noise body",
                1,
            );
        }
        save_design_document(
            &working_dir,
            "kd_target_doc",
            "target/match.md",
            "Target",
            None,
            "共享检索词 target body",
            1,
        );
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);
        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            true,
            true,
            false,
            |_stage, _processed, _total, _path| {},
        )
        .await
        .expect("reconcile workspace");

        let hits = query_documents(
            &working_dir,
            None,
            Some("共享检索词"),
            None,
            Some(&[KnowledgeType::Design]),
            Some("target"),
            1,
            state,
        )
        .await
        .expect("query documents");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "kd_target_doc");
        assert_eq!(hits[0].match_kind, "lexical");
    }

    #[tokio::test]
    async fn rebuild_reopens_same_workspace_after_releasing_existing_tantivy_writer() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let state = create_state(&working_dir);
        let library_dir = library_dir_for_working_dir(&working_dir);

        state
            .rebuild(&library_dir, Path::new(&working_dir))
            .await
            .expect("rebuild same workspace");

        let expected_index_dir = library_dir.join("knowledge_tantivy_index");
        assert_eq!(state.tantivy().index_dir(), expected_index_dir.as_path());
        state
            .tantivy()
            .index_doc(
                "doc-after-rebuild",
                "After Rebuild",
                "design/after-rebuild.md",
                "",
                &[("body".to_string(), 0, "reopened writer works".to_string())],
            )
            .expect("index after rebuild");
    }

    #[tokio::test]
    async fn rebuild_clears_catalog_bootstrap_cache() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        let state = create_state(&working_dir);
        let library_dir = library_dir_for_working_dir(&working_dir);
        let workspace_key = super::normalize_workspace_cache_key(&working_dir);

        state
            .catalog_bootstrapped_workspaces
            .lock()
            .await
            .insert(workspace_key);

        state
            .rebuild(&library_dir, Path::new(&working_dir))
            .await
            .expect("rebuild clears bootstrap cache");

        assert!(state
            .catalog_bootstrapped_workspaces
            .lock()
            .await
            .is_empty());
    }

    #[test]
    fn retryable_tantivy_lock_errors_include_windows_file_in_use() {
        assert!(super::is_retryable_tantivy_lock_error(
            "Failed to open/create knowledge tantivy index: An IO error occurred: '另一个程序正在使用此文件，进程无法访问。 (os error 32)'"
        ));
    }

    #[tokio::test]
    async fn list_cached_documents_returns_while_embedding_manager_is_busy() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        let state = create_state(&working_dir);
        let guard = state.embedding_mgr().lock_owned().await;

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            list_cached_documents(&working_dir, None, None, None, state.clone()),
        )
        .await
        .expect("knowledge list should not block on embedding manager");

        drop(guard);

        let documents = result.expect("list cached documents");
        assert!(documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Design && doc.path == "combat/core-loop.md"
        }));
    }

    #[tokio::test]
    async fn build_overview_returns_while_embedding_manager_is_busy() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);
        let guard = state.embedding_mgr().lock_owned().await;

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            build_overview(&working_dir, None, state.clone(), Path::new(&working_dir)),
        )
        .await
        .expect("knowledge overview should not block on embedding manager");

        drop(guard);

        let overview = result.expect("build overview");
        assert!(overview.total_document_count >= 1);
        assert!(overview.full_text.indexable_item_count >= 1);
    }

    #[tokio::test]
    async fn list_cached_documents_reconciles_stale_catalog_on_first_read_after_restart() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);

        let initial_state = create_state(&working_dir);
        let initial_documents =
            list_cached_documents(&working_dir, None, None, None, initial_state.clone())
                .await
                .expect("initial list");
        assert!(initial_documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Design && doc.path == "combat/core-loop.md"
        }));
        drop(initial_documents);
        drop(initial_state);

        let doc_path = workspace
            .path()
            .join("Locus")
            .join("knowledge")
            .join("design")
            .join("combat")
            .join("core-loop.md");
        std::fs::remove_file(&doc_path).expect("remove knowledge doc");

        let restarted_state = create_state(&working_dir);
        let documents_after_restart =
            list_cached_documents(&working_dir, None, None, None, restarted_state.clone())
                .await
                .expect("list after restart");

        assert!(!documents_after_restart.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Design && doc.path == "combat/core-loop.md"
        }));
    }

    #[tokio::test]
    async fn plan_managed_directory_reuse_reuses_unity_reference_snapshot_when_fingerprint_matches()
    {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_unity_reference_managed_document(&working_dir);

        let initial_state = create_state(&working_dir);
        let initial_documents =
            list_cached_documents(&working_dir, None, None, None, initial_state.clone())
                .await
                .expect("initial list");
        assert!(initial_documents.iter().any(|doc| {
            doc.doc_type == KnowledgeType::Reference
                && doc.path == "unity-official-docs/manual/ExecutionOrder.md"
        }));
        drop(initial_documents);
        drop(initial_state);

        let restarted_state = create_state(&working_dir);
        let backend_signature = restarted_state
            .embedding_mgr()
            .lock()
            .await
            .backend_signature_json();
        let db = restarted_state.db();
        let decision = plan_managed_directory_reuse(
            &working_dir,
            None,
            db.as_ref(),
            &backend_signature,
            false,
            false,
        )
        .expect("plan reuse");

        assert_eq!(
            decision.excluded_prefixes,
            vec![(KnowledgeType::Reference, "unity-official-docs".to_string())]
        );
        assert!(decision
            .retained_doc_ids
            .contains("kd_unity_execution_order"));
        assert_eq!(
            db.get_managed_directory_snapshot(unity_docs::UNITY_REFERENCE_MANAGED_PATH)
                .expect("load managed snapshot")
                .expect("managed snapshot")
                .document_count,
            1
        );
    }

    #[tokio::test]
    async fn reconcile_unity_reference_import_bulk_indexes_large_managed_snapshot() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_unity_reference_documents(&working_dir, 129);
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);

        let mut events = Vec::new();
        let report = reconcile_unity_reference_import(
            &working_dir,
            None,
            state.clone(),
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("bulk reconcile unity reference import");

        let snapshot = state
            .db()
            .get_managed_directory_snapshot(unity_docs::UNITY_REFERENCE_MANAGED_PATH)
            .expect("load managed snapshot")
            .expect("managed snapshot");
        let hits = state
            .tantivy()
            .search("Manual 000", 5, 2.0)
            .expect("search indexed unity docs");

        assert_eq!(report.added, 129);
        assert_eq!(report.removed, 0);
        assert_eq!(report.rebuilt, 129);
        assert_eq!(snapshot.document_count, 129);
        assert!(hits.iter().any(|hit| hit.doc_id == "kd_unity_ref_000"));
        assert!(events.iter().any(|(stage, _, _, _)| stage == "preparing"));
        assert!(events.iter().any(|(stage, _, _, _)| stage == "indexing"));
        assert!(events.iter().any(|(stage, _, _, _)| stage == "committing"));
    }

    #[tokio::test]
    async fn reconcile_unity_reference_import_skips_tantivy_when_lexical_index_disabled() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_unity_reference_documents(&working_dir, 2);
        save_test_general_config(&working_dir, false, false);
        let state = create_state(&working_dir);

        reconcile_unity_reference_import(&working_dir, None, state.clone(), |_stage, _, _, _| {})
            .await
            .expect("reconcile unity reference import");

        let hits = state
            .tantivy()
            .search("Manual 000", 5, 2.0)
            .expect("search indexed unity docs");
        let sample_state = state
            .db()
            .get_index_state("kd_unity_ref_000")
            .expect("load index state")
            .expect("stored index state");

        assert!(hits.is_empty());
        assert!(sample_state.embedding_backend.ends_with("|l=0|v=0"));
    }

    #[tokio::test]
    async fn reconcile_workspace_reports_preparing_progress_before_indexing() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_design_document(&working_dir);
        save_document(
            &working_dir,
            KnowledgeDocument {
                id: "kd_test_design_doc_2".to_string(),
                doc_type: KnowledgeType::Design,
                path: "combat/hit-stop.md".to_string(),
                title: "Hit Stop".to_string(),
                inject_mode: KnowledgeInjectMode::Path,
                inherit_inject_mode: false,
                inject_mode_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                summary_enabled: true,
                command_enabled: false,
                read_only: false,
                ai_maintained: false,
                storage_source: KnowledgeStorageSource::Project,
                inherit_ai_config: false,
                ai_config_source: KnowledgeConfigSource {
                    kind: KnowledgeConfigSourceKind::SelfValue,
                    path: None,
                },
                explicit_maintenance_rules: false,
                external_source: None,
                skill_enabled: None,
                skill_surface: None,
                command_trigger: None,
                argument_hint: None,
                summary: Some("停顿反馈".to_string()),
                body: "命中停顿规则".to_string(),
                maintenance_rules: None,
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save second knowledge document");
        let state = create_state(&working_dir);

        let mut events = Vec::new();
        reconcile_workspace_internal(
            &working_dir,
            None,
            state,
            true,
            true,
            false,
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("reconcile workspace");

        let first_indexing = events
            .iter()
            .position(|(stage, _, _, _)| stage == "indexing")
            .expect("indexing progress");
        let first_preparing_with_progress = events
            .iter()
            .position(|(stage, processed, total, _)| {
                stage == "preparing" && *total >= 2 && *processed > 0
            })
            .expect("preparing progress");

        assert!(first_preparing_with_progress < first_indexing);
    }

    #[tokio::test]
    async fn reconcile_workspace_suppresses_progress_for_single_lexical_update_in_large_catalog() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_memory_documents(&working_dir, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD + 1);
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);

        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |_stage, _processed, _total, _path| {},
        )
        .await
        .expect("initial reconcile");

        save_memory_document(&working_dir, 42, "Updated single memory body", 2);

        let mut events = Vec::new();
        let report = reconcile_workspace_internal(
            &working_dir,
            None,
            state,
            false,
            false,
            false,
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("single document reconcile");

        assert_eq!(report.stale, 1);
        assert_eq!(report.rebuilt, 1);
        assert!(
            events.is_empty(),
            "single lexical update should stay below visible progress threshold: {:?}",
            events
        );
    }

    #[tokio::test]
    async fn reconcile_workspace_surfaces_progress_for_large_lexical_update_batch() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_memory_documents(&working_dir, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD + 1);
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);

        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |_stage, _processed, _total, _path| {},
        )
        .await
        .expect("initial reconcile");

        for index in 0..LARGE_LEXICAL_REBUILD_DOC_THRESHOLD {
            save_memory_document(
                &working_dir,
                index,
                &format!("Updated memory body {:03}", index),
                2,
            );
        }

        let mut events = Vec::new();
        let report = reconcile_workspace_internal(
            &working_dir,
            None,
            state,
            false,
            false,
            false,
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("large lexical reconcile");

        assert_eq!(report.stale, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD);
        assert_eq!(report.rebuilt, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD);
        assert!(events.iter().any(|(stage, processed, total, _)| {
            stage == "preparing"
                && *processed == LARGE_LEXICAL_REBUILD_DOC_THRESHOLD
                && *total == LARGE_LEXICAL_REBUILD_DOC_THRESHOLD
        }));
        assert!(events.iter().any(|(stage, _, total, _)| {
            stage == "indexing" && *total == LARGE_LEXICAL_REBUILD_DOC_THRESHOLD
        }));
        assert!(events
            .iter()
            .all(|(_, _, total, _)| *total == LARGE_LEXICAL_REBUILD_DOC_THRESHOLD));
    }

    #[tokio::test]
    async fn reconcile_workspace_suppresses_progress_for_large_lexical_disable_cleanup() {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_memory_documents(&working_dir, LARGE_LEXICAL_REBUILD_DOC_THRESHOLD + 1);
        save_test_general_config(&working_dir, true, false);
        let state = create_state(&working_dir);

        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |_stage, _processed, _total, _path| {},
        )
        .await
        .expect("initial lexical reconcile");
        let indexed_doc_count = state
            .db()
            .list_all_index_states()
            .expect("list initial index states")
            .len();

        save_test_general_config(&working_dir, false, false);

        let mut events = Vec::new();
        let report = reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("lexical disable reconcile");
        let sample_state = state
            .db()
            .get_index_state("kd_test_memory_doc_000")
            .expect("load index state")
            .expect("stored index state");
        let hits = state
            .tantivy()
            .search("Project memory body 000", 5, 2.0)
            .expect("search tantivy after cleanup");

        assert!(indexed_doc_count >= LARGE_LEXICAL_REBUILD_DOC_THRESHOLD);
        assert_eq!(report.stale, indexed_doc_count);
        assert_eq!(report.rebuilt, indexed_doc_count);
        assert!(
            events.is_empty(),
            "lexical disable cleanup should not surface progress: {:?}",
            events
        );
        assert!(sample_state.embedding_backend.ends_with("|l=0|v=0"));
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn reconcile_workspace_skips_lexical_progress_and_tantivy_commit_for_vector_only_change()
    {
        let workspace = tempdir().expect("workspace");
        let working_dir = workspace.path().to_string_lossy().to_string();
        seed_unity_reference_documents(&working_dir, 129);
        save_test_general_config(&working_dir, true, true);

        let mut enabled_config = default_directory_config_for_type(KnowledgeType::Reference);
        enabled_config.vector_search = FolderIndexRuleSetting::Enabled;
        update_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
            enabled_config,
        )
        .expect("enable vector search");

        let state = create_state(&working_dir);
        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |_stage, _processed, _total, _path| {},
        )
        .await
        .expect("initial reconcile");

        let meta_path = library_dir_for_working_dir(&working_dir)
            .join("knowledge_tantivy_index")
            .join("meta.json");
        let lexical_meta_before = std::fs::read_to_string(&meta_path).expect("read meta before");

        let mut disabled_config = default_directory_config_for_type(KnowledgeType::Reference);
        disabled_config.vector_search = FolderIndexRuleSetting::Disabled;
        update_directory_config(
            &working_dir,
            KnowledgeType::Reference,
            unity_docs::UNITY_REFERENCE_MANAGED_DIR,
            disabled_config,
        )
        .expect("disable vector search");

        let mut events = Vec::new();
        reconcile_workspace_internal(
            &working_dir,
            None,
            state.clone(),
            false,
            false,
            false,
            |stage, processed, total, path| {
                events.push((
                    stage.to_string(),
                    processed,
                    total,
                    path.map(|value| value.to_string()),
                ));
            },
        )
        .await
        .expect("vector-only reconcile");

        let lexical_meta_after = std::fs::read_to_string(&meta_path).expect("read meta after");
        let sample_state = state
            .db()
            .get_index_state("kd_unity_ref_000")
            .expect("load index state")
            .expect("stored index state");

        assert!(events.is_empty());
        assert_eq!(lexical_meta_after, lexical_meta_before);
        assert!(sample_state.embedding_backend.ends_with("|l=1|v=0"));
    }

    #[test]
    fn lexical_stage_progress_advances_across_internal_phases() {
        let preparing = lexical_stage_progress("preparing", 0, 100);
        let preparing_mid = lexical_stage_progress("preparing", 50, 100);
        let cleaning = lexical_stage_progress("cleaning", 10, 100);
        let indexing = lexical_stage_progress("indexing", 60, 100);
        let committing = lexical_stage_progress("committing", 100, 100);

        assert!(preparing > 0.0);
        assert!(preparing_mid > preparing);
        assert!(cleaning > preparing_mid);
        assert!(indexing > cleaning);
        assert_eq!(committing, 1.0);
    }

    #[test]
    fn embedding_stage_progress_reserves_tail_for_commit_and_ready() {
        let preparing = embedding_stage_progress("preparing", 0, 100);
        let indexing = embedding_stage_progress("indexing", 100, 100);
        let committing = embedding_stage_progress("committing", 100, 100);
        let ready = embedding_stage_progress("ready", 100, 100);

        assert!(preparing > 0.0);
        assert!(indexing < committing);
        assert!(committing < 1.0);
        assert_eq!(ready, 1.0);
    }

    #[test]
    fn embedding_rebuild_detail_tracks_commit_stage_without_stale_file() {
        let indexing = embedding_rebuild_detail("indexing", 7, 7, Some("unity-project-setup.md"));
        let committing = embedding_rebuild_detail("committing", 7, 7, None);

        assert!(indexing.contains("unity-project-setup.md"));
        assert!(committing.contains("Persisting vector batches"));
        assert!(!committing.contains("unity-project-setup.md"));
    }
}
